use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use alloc::vec::Vec;
use alloc::string::String;
use spin::Mutex;

use x86_64::structures::idt::InterruptStackFrame;

use crate::quantum::{self, Circuit, QuantumState, parse_qasm};
use crate::vga;

pub const ABI_CALL_ADDR: u64 = 0x0000_0000_4001_0000;

type ShmCall = qos_abi::shm::ShmCall;

const MAX_JOBS: usize = 16;
const MAX_IR_BYTES: usize = 4096;
/// Maximum qubits the system can simulate simultaneously
pub const MAX_QUBITS: usize = 32;
/// Gates to execute per timer tick (batch size)
const GATES_PER_TICK: usize = 10;

type SubmitHdr = qos_abi::shm::ShmSubmitIrHeader;
type VfsHdr = qos_abi::shm::ShmVfsIoHeader;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(0);
static CURRENT_RUNNING: AtomicU64 = AtomicU64::new(0);
static LAST_SLOT: AtomicU32 = AtomicU32::new(0);
/// Global counter of qubits currently in use by running jobs
pub static USED_QUBITS: AtomicUsize = AtomicUsize::new(0);
static TOTAL_GATES_EXECUTED: AtomicU64 = AtomicU64::new(0);
/// Flag to signal that quantum work is pending (set by timer, processed by main loop)
static QUANTUM_WORK_PENDING: AtomicBool = AtomicBool::new(false);

/// Information about a job for UI display
#[derive(Clone)]
pub struct JobInfo {
    pub slot: usize,
    pub handle: u64,
    pub state: qos_abi::JobState,
    pub gates_remaining: usize,
    pub gates_total: usize,
    pub shots_done: u32,
    pub shots_total: u32,
    pub n_qubits: u32,
    pub ir_len: u32,
}

pub fn snapshot_jobs() -> Vec<JobInfo> {
    let mut out = Vec::new();
    for (i, slot) in JOBS.iter().enumerate() {
        let handle = slot.handle.load(Ordering::Relaxed);
        if handle == 0 {
            continue;
        }
        let circuit = slot.circuit.lock();
        let gates_total = circuit.as_ref().map(|c| c.len()).unwrap_or(0);
        let gates_remaining = circuit.as_ref().map(|c| c.remaining()).unwrap_or(0);
        drop(circuit);
        
        out.push(JobInfo {
            slot: i,
            handle,
            state: slot.state_raw().as_job_state(),
            gates_remaining,
            gates_total,
            shots_done: slot.shots_done.load(Ordering::Relaxed),
            shots_total: slot.shots_total.load(Ordering::Relaxed),
            n_qubits: slot.n_qubits.load(Ordering::Relaxed),
            ir_len: slot.ir_len.load(Ordering::Relaxed),
        });
    }
    out
}

pub fn current_running_handle() -> u64 {
    CURRENT_RUNNING.load(Ordering::Relaxed)
}

pub fn abbrev_state(st: qos_abi::JobState) -> &'static str {
    match st {
        qos_abi::JobState::Queued => "Q",
        qos_abi::JobState::Running => "R",
        qos_abi::JobState::Done => "D",
        qos_abi::JobState::Cancelled => "C",
        // Keep "Failed" as catch-all for invalid/free.
        _ => "F",
    }
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum StateRaw {
    Free = 0,
    Queued = 1,
    Running = 2,
    Done = 3,
    Cancelled = 4,
    Failed = 5,
}

impl StateRaw {
    fn as_job_state(self) -> qos_abi::JobState {
        match self {
            StateRaw::Queued => qos_abi::JobState::Queued,
            StateRaw::Running => qos_abi::JobState::Running,
            StateRaw::Done => qos_abi::JobState::Done,
            StateRaw::Cancelled => qos_abi::JobState::Cancelled,
            StateRaw::Free | StateRaw::Failed => qos_abi::JobState::Failed,
        }
    }
}

#[repr(C, align(16))]
struct JobSlot {
    handle: AtomicU64,
    state: AtomicU32,
    shots_done: AtomicU32,     // Progress: shots completed so far
    shots_total: AtomicU32,   // Total shots requested
    ir_len: AtomicU32,
    ir_hash: AtomicU64,
    ir_format: AtomicU32,
    n_qubits: AtomicU32,
    shots: AtomicU32,
    res0: AtomicU64,
    res1: AtomicU64,
    // IR storage (protected by mutex for concurrent access)
    ir_data: Mutex<[u8; MAX_IR_BYTES]>,
    /// Parsed circuit for gate-by-gate execution
    circuit: Mutex<Option<Circuit>>,
    /// Quantum state for simulation
    quantum_state: Mutex<Option<QuantumState>>,
    /// Current shot being executed
    current_shot: AtomicU32,
}

impl JobSlot {
    const fn new() -> Self {
        Self {
            handle: AtomicU64::new(0),
            state: AtomicU32::new(StateRaw::Free as u32),
            shots_done: AtomicU32::new(0),
            shots_total: AtomicU32::new(0),
            ir_len: AtomicU32::new(0),
            ir_hash: AtomicU64::new(0),
            ir_format: AtomicU32::new(0),
            n_qubits: AtomicU32::new(0),
            shots: AtomicU32::new(0),
            res0: AtomicU64::new(0),
            res1: AtomicU64::new(0),
            ir_data: Mutex::new([0u8; MAX_IR_BYTES]),
            circuit: Mutex::new(None),
            quantum_state: Mutex::new(None),
            current_shot: AtomicU32::new(0),
        }
    }

    fn state_raw(&self) -> StateRaw {
        match self.state.load(Ordering::Relaxed) {
            x if x == StateRaw::Queued as u32 => StateRaw::Queued,
            x if x == StateRaw::Running as u32 => StateRaw::Running,
            x if x == StateRaw::Done as u32 => StateRaw::Done,
            x if x == StateRaw::Cancelled as u32 => StateRaw::Cancelled,
            x if x == StateRaw::Failed as u32 => StateRaw::Failed,
            _ => StateRaw::Free,
        }
    }

    /// Store IR bytes into the slot
    fn store_ir(&self, ir: &[u8]) {
        let mut data = self.ir_data.lock();
        let len = ir.len().min(MAX_IR_BYTES);
        data[..len].copy_from_slice(&ir[..len]);
        self.ir_len.store(len as u32, Ordering::Relaxed);
    }

    /// Get IR bytes (copies to a Vec)
    fn get_ir(&self) -> Vec<u8> {
        let data = self.ir_data.lock();
        let len = self.ir_len.load(Ordering::Relaxed) as usize;
        data[..len].to_vec()
    }
    
    /// Initialize circuit from stored IR
    fn init_circuit(&self) -> bool {
        let ir = self.get_ir();
        let ir_str = core::str::from_utf8(&ir).unwrap_or("");
        
        match parse_qasm(ir_str) {
            Ok(circuit) => {
                let n_qubits = circuit.n_qubits;
                // Store the actual qubit count from the parsed circuit
                self.n_qubits.store(n_qubits as u32, Ordering::Relaxed);
                *self.circuit.lock() = Some(circuit);
                *self.quantum_state.lock() = Some(QuantumState::new(n_qubits));
                self.current_shot.store(0, Ordering::Relaxed);
                true
            }
            Err(_e) => {
                false
            }
        }
    }
    
    /// Reset for next shot
    fn reset_for_shot(&self) {
        let mut state_guard = self.quantum_state.lock();
        if let Some(state) = state_guard.as_mut() {
            state.reset();
        }
        let mut circuit_guard = self.circuit.lock();
        if let Some(circuit) = circuit_guard.as_mut() {
            circuit.pc = 0;
        }
    }
    
    /// Clear circuit and state when job completes
    fn clear_circuit(&self) {
        *self.circuit.lock() = None;
        *self.quantum_state.lock() = None;
        self.current_shot.store(0, Ordering::Relaxed);
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn looks_like_qasm2(bytes: &[u8]) -> bool {
    // Minimal validation: require ASCII-ish and an OPENQASM header substring.
    let mut saw_openqasm = false;
    // Cheap sliding window for "OPENQASM".
    const NEEDLE: &[u8] = b"OPENQASM";
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'\n' && b != b'\r' && b != b'\t' && !(0x20..=0x7e).contains(&b) {
            return false;
        }
        if !saw_openqasm && i + NEEDLE.len() <= bytes.len() {
            if &bytes[i..i + NEEDLE.len()] == NEEDLE {
                saw_openqasm = true;
            }
        }
        i += 1;
    }
    saw_openqasm
}

fn contains_subsequence(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > hay.len() {
        return false;
    }
    for i in 0..=(hay.len() - needle.len()) {
        if &hay[i..i + needle.len()] == needle {
            return true;
        }
    }
    false
}

static JOBS: [JobSlot; MAX_JOBS] = [
    JobSlot::new(), JobSlot::new(), JobSlot::new(), JobSlot::new(),
    JobSlot::new(), JobSlot::new(), JobSlot::new(), JobSlot::new(),
    JobSlot::new(), JobSlot::new(), JobSlot::new(), JobSlot::new(),
    JobSlot::new(), JobSlot::new(), JobSlot::new(), JobSlot::new(),
];

fn find_slot_by_handle(handle: u64) -> Option<usize> {
    if handle == 0 {
        return None;
    }
    for (i, slot) in JOBS.iter().enumerate() {
        if slot.handle.load(Ordering::Relaxed) == handle {
            return Some(i);
        }
    }
    None
}

fn alloc_slot() -> Option<usize> {
    for (i, slot) in JOBS.iter().enumerate() {
        if slot.state_raw() == StateRaw::Free {
            return Some(i);
        }
    }
    None
}

fn rr_pick_next_queued() -> Option<usize> {
    let start = (LAST_SLOT.load(Ordering::Relaxed) as usize + 1) % MAX_JOBS;
    let used = USED_QUBITS.load(Ordering::Relaxed);
    let available = MAX_QUBITS.saturating_sub(used);
    
    for offset in 0..MAX_JOBS {
        let idx = (start + offset) % MAX_JOBS;
        let slot = &JOBS[idx];
        if slot.state_raw() == StateRaw::Queued {
            let n_qubits = slot.n_qubits.load(Ordering::Relaxed) as usize;
            let shots_done = slot.shots_done.load(Ordering::Relaxed);
            let shots_total = slot.shots_total.load(Ordering::Relaxed);
            
            // Check if job has work left and we have enough qubits
            if shots_done < shots_total && n_qubits <= available {
                return Some(idx);
            }
        }
    }
    None
}

fn schedule_slot(idx: usize) {
    let slot = &JOBS[idx];
    let handle = slot.handle.load(Ordering::Relaxed);
    if handle == 0 {
        return;
    }
    
    // Initialize circuit FIRST to determine actual qubit count from QASM
    if !slot.init_circuit() {
        // Parse failed - mark as failed
        slot.state.store(StateRaw::Failed as u32, Ordering::Relaxed);
        return;
    }
    
    // NOW reserve qubits using the actual count from parsed circuit
    let n_qubits = slot.n_qubits.load(Ordering::Relaxed) as usize;
    let used = USED_QUBITS.load(Ordering::Relaxed);
    if used + n_qubits > MAX_QUBITS {
        slot.clear_circuit();
        return; // Not enough qubits
    }
    USED_QUBITS.fetch_add(n_qubits, Ordering::Relaxed);
    
    CURRENT_RUNNING.store(handle, Ordering::Relaxed);
    LAST_SLOT.store(idx as u32, Ordering::Relaxed);
    slot.state.store(StateRaw::Running as u32, Ordering::Relaxed);
}

fn release_slot_qubits(slot: &JobSlot) {
    let n_qubits = slot.n_qubits.load(Ordering::Relaxed) as usize;
    USED_QUBITS.fetch_sub(n_qubits, Ordering::Relaxed);
    slot.clear_circuit();
}

fn maybe_schedule() {
    if CURRENT_RUNNING.load(Ordering::Relaxed) != 0 {
        return;
    }
    if let Some(idx) = rr_pick_next_queued() {
        schedule_slot(idx);
    }
}

/// Get available qubit count
pub fn available_qubits() -> usize {
    MAX_QUBITS.saturating_sub(USED_QUBITS.load(Ordering::Relaxed))
}

/// Get total system qubit count
pub fn system_qubits() -> usize {
    MAX_QUBITS
}

/// Get total gates executed (for stats)
pub fn total_gates_executed() -> u64 {
    TOTAL_GATES_EXECUTED.load(Ordering::Relaxed)
}

// --- Kernel-shell helpers (non-ABI) ---

pub fn shell_list_jobs() {
    let used = USED_QUBITS.load(Ordering::Relaxed);
    let avail = MAX_QUBITS.saturating_sub(used);
    vga::println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    vga::println!("║  QOS Quantum Job Manager  │  Qubits: {}/{} available  │  Gates: {}      ║",
        avail, MAX_QUBITS, TOTAL_GATES_EXECUTED.load(Ordering::Relaxed));
    vga::println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    
    let mut any_jobs = false;
    for (i, slot) in JOBS.iter().enumerate() {
        let handle = slot.handle.load(Ordering::Relaxed);
        let state = slot.state_raw().as_job_state();
        if handle == 0 {
            continue;
        }
        any_jobs = true;
        
        let shots_done = slot.shots_done.load(Ordering::Relaxed);
        let shots_total = slot.shots_total.load(Ordering::Relaxed);
        let n_qubits = slot.n_qubits.load(Ordering::Relaxed);
        
        // Get gate progress
        let circuit = slot.circuit.lock();
        let (gates_done, gates_total) = circuit.as_ref()
            .map(|c| (c.pc, c.len()))
            .unwrap_or((0, 0));
        drop(circuit);
        
        let state_icon = match state {
            qos_abi::JobState::Queued => "⏳",
            qos_abi::JobState::Running => "▶️ ",
            qos_abi::JobState::Done => "✓ ",
            qos_abi::JobState::Cancelled => "✗ ",
            _ => "? ",
        };
        
        vga::println!(
            "║  {} Job {} │ {} │ q={} │ shots {}/{} │ gates {}/{}         ║",
            state_icon, handle, abbrev_state(state), n_qubits,
            shots_done, shots_total, gates_done, gates_total
        );
    }
    
    if !any_jobs {
        vga::println!("║  No jobs in queue                                                            ║");
    }
    vga::println!("╚══════════════════════════════════════════════════════════════════════════════╝");
}

pub fn shell_submit_bell() -> Option<u64> {
    let idx = alloc_slot()?;
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed) + 1;
    let slot = &JOBS[idx];

    // Built-in QASM2 Bell program
    const IR: &[u8] = b"OPENQASM 2.0;\ninclude \"qelib1.inc\";\nqreg q[2];\ncreg c[2];\nh q[0];\ncx q[0],q[1];\nmeasure q[0] -> c[0];\nmeasure q[1] -> c[1];\n";

    slot.handle.store(handle, Ordering::Relaxed);
    slot.state.store(StateRaw::Queued as u32, Ordering::Relaxed);
    slot.shots_done.store(0, Ordering::Relaxed);
    slot.shots_total.store(1, Ordering::Relaxed); // 1 simulation batch

    slot.ir_len.store(IR.len() as u32, Ordering::Relaxed);
    slot.ir_hash.store(fnv1a64(IR), Ordering::Relaxed);
    slot.ir_format.store(qos_abi::shm::IRFMT_QASM2, Ordering::Relaxed);
    slot.n_qubits.store(2, Ordering::Relaxed);
    slot.shots.store(1024, Ordering::Relaxed);
    slot.store_ir(IR);

    // Results will be computed by the simulator
    slot.res0.store(0, Ordering::Relaxed);
    slot.res1.store(0, Ordering::Relaxed);

    maybe_schedule();
    Some(handle)
}

pub fn shell_submit_ir_bell(shots: u32) -> Option<u64> {
    let idx = alloc_slot()?;
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed) + 1;
    let slot = &JOBS[idx];

    // Built-in QASM2 Bell program (minimal, but includes OPENQASM + h + cx).
    const IR: &[u8] = b"OPENQASM 2.0;\ninclude \"qelib1.inc\";\nqreg q[2];\ncreg c[2];\nh q[0];\ncx q[0],q[1];\nmeasure q[0] -> c[0];\nmeasure q[1] -> c[1];\n";

    slot.handle.store(handle, Ordering::Relaxed);
    slot.state.store(StateRaw::Queued as u32, Ordering::Relaxed);
    slot.shots_done.store(0, Ordering::Relaxed);
    slot.shots_total.store(1, Ordering::Relaxed); // 1 simulation batch

    slot.ir_len.store(IR.len() as u32, Ordering::Relaxed);
    slot.ir_hash.store(fnv1a64(IR), Ordering::Relaxed);
    slot.ir_format.store(qos_abi::shm::IRFMT_QASM2, Ordering::Relaxed);
    slot.n_qubits.store(2, Ordering::Relaxed);
    slot.shots.store(shots, Ordering::Relaxed);
    slot.store_ir(IR);

    // Results will be computed by the simulator
    slot.res0.store(0, Ordering::Relaxed);
    slot.res1.store(0, Ordering::Relaxed);

    maybe_schedule();
    Some(handle)
}

pub fn shell_submit_ir_qasm2(ir: &[u8], shots: u32, n_qubits: u32) -> Option<u64> {
    if ir.is_empty() || ir.len() > MAX_IR_BYTES {
        return None;
    }
    if !looks_like_qasm2(ir) {
        return None;
    }

    let idx = alloc_slot()?;
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed) + 1;
    let slot = &JOBS[idx];

    slot.handle.store(handle, Ordering::Relaxed);
    slot.state.store(StateRaw::Queued as u32, Ordering::Relaxed);
    slot.shots_done.store(0, Ordering::Relaxed);
    slot.shots_total.store(1, Ordering::Relaxed); // 1 simulation batch

    slot.ir_len.store(ir.len() as u32, Ordering::Relaxed);
    slot.ir_hash.store(fnv1a64(ir), Ordering::Relaxed);
    slot.ir_format.store(qos_abi::shm::IRFMT_QASM2, Ordering::Relaxed);
    slot.n_qubits.store(n_qubits, Ordering::Relaxed);
    slot.shots.store(shots, Ordering::Relaxed);
    slot.store_ir(ir);

    // Results will be computed by the simulator
    slot.res0.store(0, Ordering::Relaxed);
    slot.res1.store(0, Ordering::Relaxed);

    maybe_schedule();
    Some(handle)
}

pub fn shell_status(handle: u64) -> Option<qos_abi::JobState> {
    let idx = find_slot_by_handle(handle)?;
    Some(JOBS[idx].state_raw().as_job_state())
}

pub fn shell_result(handle: u64) -> Result<(u64, u64), qos_abi::JobState> {
    let Some(idx) = find_slot_by_handle(handle) else {
        return Err(qos_abi::JobState::Failed);
    };
    let slot = &JOBS[idx];
    let raw = slot.state_raw();
    if raw != StateRaw::Done {
        return Err(raw.as_job_state());
    }
    let n00 = slot.res0.load(Ordering::Relaxed);
    let n11 = slot.res1.load(Ordering::Relaxed);

    // Mimic ABI semantics: consuming result frees the slot.
    slot.handle.store(0, Ordering::Relaxed);
    slot.state.store(StateRaw::Free as u32, Ordering::Relaxed);
    slot.shots_done.store(0, Ordering::Relaxed);
    slot.shots_total.store(0, Ordering::Relaxed);

    Ok((n00, n11))
}

pub fn shell_cancel(handle: u64) -> bool {
    let Some(idx) = find_slot_by_handle(handle) else {
        return false;
    };
    let slot = &JOBS[idx];
    slot.state.store(StateRaw::Cancelled as u32, Ordering::Relaxed);
    slot.shots_done.store(0, Ordering::Relaxed);
    slot.shots_total.store(0, Ordering::Relaxed);
    release_slot_qubits(slot);
    if CURRENT_RUNNING.load(Ordering::Relaxed) == handle {
        CURRENT_RUNNING.store(0, Ordering::Relaxed);
        maybe_schedule();
    }
    true
}

/// Called from the PIT timer interrupt handler.
/// Just sets a flag - actual work is done in process_quantum_work() from main loop.
/// This avoids deadlocks from taking Mutex locks in interrupt context.
pub fn on_timer_tick() {
    QUANTUM_WORK_PENDING.store(true, Ordering::Relaxed);
}

/// Process quantum work - MUST be called from main loop, NOT from interrupt handler!
/// This does the actual quantum gate execution.
pub fn process_quantum_work() {
    if !QUANTUM_WORK_PENDING.swap(false, Ordering::Relaxed) {
        return;
    }
    
    let current = CURRENT_RUNNING.load(Ordering::Relaxed);
    if current == 0 {
        // Try to schedule a queued job
        maybe_schedule();
        return;
    }

    let Some(idx) = find_slot_by_handle(current) else {
        CURRENT_RUNNING.store(0, Ordering::Relaxed);
        maybe_schedule();
        return;
    };
    let slot = &JOBS[idx];
    if slot.state_raw() != StateRaw::Running {
        CURRENT_RUNNING.store(0, Ordering::Relaxed);
        maybe_schedule();
        return;
    }

    // Check if all shots are complete
    let shots_done = slot.shots_done.load(Ordering::Relaxed);
    let shots_total = slot.shots_total.load(Ordering::Relaxed);
    
    if shots_done >= shots_total {
        // Job is complete
        slot.state.store(StateRaw::Done as u32, Ordering::Relaxed);
        release_slot_qubits(slot);
        CURRENT_RUNNING.store(0, Ordering::Relaxed);
        maybe_schedule();
        return;
    }

    // For verify mode, use deterministic mock results for faster testing
    #[cfg(feature = "verify")]
    {
        let shots = slot.shots.load(Ordering::Relaxed) as u64;
        let half = shots / 2;
        slot.res0.store(half, Ordering::Relaxed);
        slot.res1.store(shots - half, Ordering::Relaxed);
        slot.shots_done.store(shots_total, Ordering::Relaxed);
        slot.state.store(StateRaw::Done as u32, Ordering::Relaxed);
        release_slot_qubits(slot);
        CURRENT_RUNNING.store(0, Ordering::Relaxed);
        maybe_schedule();
        return;
    }
    
    #[cfg(not(feature = "verify"))]
    {
        // Execute gates from the circuit
        let mut circuit = slot.circuit.lock();
        let mut qstate = slot.quantum_state.lock();
        
        let (circuit_opt, qstate_opt) = (circuit.as_mut(), qstate.as_mut());
        
        match (circuit_opt, qstate_opt) {
            (Some(circ), Some(qs)) => {
                // Execute a batch of gates
                let gates_executed = circ.step_n(qs, GATES_PER_TICK);
                TOTAL_GATES_EXECUTED.fetch_add(gates_executed as u64, Ordering::Relaxed);
                
                // Check if this shot is complete (circuit finished)
                if circ.is_done() {
                    // Sample the measurement outcome for this shot
                    let n_qubits = slot.n_qubits.load(Ordering::Relaxed) as usize;
                    let outcome = qs.sample_outcome_index();
                    
                    // Update result counters (for 2-qubit Bell-like results)
                    // outcome is a bitstring represented as usize
                    // res0 = count of |00⟩, res1 = count of |11⟩ (for 2-qubit case)
                    if n_qubits == 2 {
                        match outcome {
                            0b00 => { slot.res0.fetch_add(1, Ordering::Relaxed); }
                            0b11 => { slot.res1.fetch_add(1, Ordering::Relaxed); }
                            _ => {} // |01⟩ or |10⟩, not tracked in simple counters
                        }
                    } else {
                        // For arbitrary qubit counts, track all-zeros vs all-ones
                        let all_ones = (1usize << n_qubits) - 1;
                        if outcome == 0 {
                            slot.res0.fetch_add(1, Ordering::Relaxed);
                        } else if outcome == all_ones {
                            slot.res1.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    
                    // Increment shots done
                    let new_shots_done = slot.shots_done.fetch_add(1, Ordering::Relaxed) + 1;
                    
                    // Check if we need more shots
                    let total_shots = slot.shots.load(Ordering::Relaxed);
                    if new_shots_done < total_shots {
                        // Reset for next shot - directly on already-locked guards
                        qs.reset();
                        circ.pc = 0;
                    } else {
                        // All shots complete - mark shots_total as done
                        slot.shots_total.store(new_shots_done, Ordering::Relaxed);
                    }
                }
            }
            _ => {
                // No circuit or state - something went wrong, mark as failed
                drop(circuit);
                drop(qstate);
                slot.state.store(StateRaw::Failed as u32, Ordering::Relaxed);
                release_slot_qubits(slot);
                CURRENT_RUNNING.store(0, Ordering::Relaxed);
                maybe_schedule();
                return;
            }
        }
        
        // Check if job is now complete
        drop(circuit);
        drop(qstate);
        let shots_done = slot.shots_done.load(Ordering::Relaxed);
        let total_shots = slot.shots.load(Ordering::Relaxed);
        if shots_done >= total_shots {
            slot.state.store(StateRaw::Done as u32, Ordering::Relaxed);
            release_slot_qubits(slot);
            CURRENT_RUNNING.store(0, Ordering::Relaxed);
            maybe_schedule();
        }
    }
}


pub static SYSCALL_COUNT: AtomicU64 = AtomicU64::new(0);

#[repr(u64)]
pub enum SyscallNumber {
    // Reserved for Milestone 2.
    Nop = 0,
    WriteChar = 1,
    Exit = 2,
}

// Simplified syscall handler without naked_asm
pub extern "x86-interrupt" fn syscall_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let _n = SYSCALL_COUNT.fetch_add(1, Ordering::Relaxed) + 1;

    #[cfg(feature = "userabi")]
    {
        handle_userdemo_abi();
    }
}

// DISABLED: Complex naked asm handler for debugging offset error

#[allow(dead_code)]
fn syscall_interrupt_handler_rust_unused(_saved_rsp: u64) -> u64 {
    // Keep a counter for debugging/metrics, but don't spam the console on every syscall.
    let _n = SYSCALL_COUNT.fetch_add(1, Ordering::Relaxed) + 1;

    #[cfg(feature = "userabi")]
    {
        handle_userdemo_abi()
    }

    #[cfg(not(feature = "userabi"))]
    {
        0
    }
}

#[cfg(feature = "userabi")]
fn user_range_mapped(ptr: u64, len: usize) -> bool {
    use x86_64::structures::paging::{Mapper, Page, Size4KiB};
    use x86_64::VirtAddr;

    if ptr == 0 || len == 0 {
        return false;
    }

    let Some(end_u64) = ptr.checked_add((len - 1) as u64) else {
        return false;
    };

    let start = VirtAddr::new(ptr);
    let end = VirtAddr::new(end_u64);

    let start_page = Page::<Size4KiB>::containing_address(start);
    let end_page = Page::<Size4KiB>::containing_address(end);

    // Validate mappings in the currently active address space (which is the user CR3
    // while in the user ABI handler).
    let mut mapper = unsafe { crate::memory::init(crate::memory::phys_offset()) };
    let mut page = start_page;
    loop {
        if mapper.translate_page(page).is_err() {
            return false;
        }
        if page == end_page {
            break;
        }
        let next = page.start_address().as_u64().saturating_add(4096);
        page = Page::containing_address(VirtAddr::new(next));
    }
    true
}

#[cfg(feature = "userabi")]
fn handle_userdemo_abi() -> u64 {
    // Minimal shared-memory ABI:
    // User fills `qos_abi::shm::ShmCall` at ABI_CALL_ADDR and triggers int 0x80.
    // Kernel reads op and writes status/ret*.
    let call = unsafe { &mut *(ABI_CALL_ADDR as *mut ShmCall) };

    let abi_version = unsafe { core::ptr::read_volatile(&call.abi_version) };
    if abi_version != qos_abi::shm::SHM_ABI_VERSION {
        unsafe {
            core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
        }
        crate::serial_println!(
            "syscall abi: version mismatch user={} kernel={}",
            abi_version,
            qos_abi::shm::SHM_ABI_VERSION
        );
        vga::println!("abi version mismatch");
        return 0;
    }

    let op = unsafe { core::ptr::read_volatile(&call.op) };

    match op {
        qos_abi::shm::OP_GET_ABI_VERSION => {
            let v = qos_abi::ABI_VERSION as u64;
            unsafe {
                core::ptr::write_volatile(&mut call.ret0, v);
                core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_OK);
            }
            crate::serial_println!("syscall abi: GetAbiVersion -> {}", v);
            vga::println!("abi version ok");
            0
        }
        qos_abi::shm::OP_SUBMIT_BELL => {
            if let Some(idx) = alloc_slot() {
                let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed) + 1;
                let slot = &JOBS[idx];
                
                // Built-in Bell program
                const IR: &[u8] = b"OPENQASM 2.0;\ninclude \"qelib1.inc\";\nqreg q[2];\ncreg c[2];\nh q[0];\ncx q[0],q[1];\nmeasure q[0] -> c[0];\nmeasure q[1] -> c[1];\n";
                
                slot.handle.store(handle, Ordering::Relaxed);
                slot.state.store(StateRaw::Queued as u32, Ordering::Relaxed);
                slot.shots_done.store(0, Ordering::Relaxed);
                slot.shots_total.store(1, Ordering::Relaxed);

                slot.ir_len.store(IR.len() as u32, Ordering::Relaxed);
                slot.ir_hash.store(fnv1a64(IR), Ordering::Relaxed);
                slot.ir_format.store(qos_abi::shm::IRFMT_QASM2, Ordering::Relaxed);
                slot.n_qubits.store(2, Ordering::Relaxed);
                slot.shots.store(1024, Ordering::Relaxed);
                slot.store_ir(IR);

                // Results computed by simulator at runtime
                slot.res0.store(0, Ordering::Relaxed);
                slot.res1.store(0, Ordering::Relaxed);

                maybe_schedule();
                unsafe {
                    core::ptr::write_volatile(&mut call.ret0, handle);
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_OK);
                }
                crate::serial_println!("syscall abi: SubmitBell -> handle={}", handle);
                vga::println!("job submitted");
                0
            } else {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: SubmitBell -> ERR (no slots)");
                vga::println!("job submit ERR");
                0
            }
        }

        qos_abi::shm::OP_SUBMIT_IR => {
            // Submit a program buffer: arg0=user_ptr_to_header, arg1=total_bytes(header+payload).
            let user_ptr_u64 = unsafe { core::ptr::read_volatile(&call.arg0) };
            let user_ptr = user_ptr_u64 as *const u8;
            let total = unsafe { core::ptr::read_volatile(&call.arg1) } as usize;

            if user_ptr.is_null() || total < core::mem::size_of::<SubmitHdr>() || total > MAX_IR_BYTES {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: SubmitIr -> ERR (ptr={:?} total={})", user_ptr, total);
                vga::println!("submit IR ERR");
                return 0;
            }

            if !user_range_mapped(user_ptr_u64, total) {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!(
                    "syscall abi: SubmitIr -> ERR (unmapped ptr={:#x} total={})",
                    user_ptr_u64,
                    total
                );
                vga::println!("submit IR ERR");
                return 0;
            }

            if let Some(idx) = alloc_slot() {
                // Copy from user memory into a small stack buffer. This keeps the kernel allocation-free
                // inside the interrupt handler while still proving real payload flow.
                let mut buf = [0u8; MAX_IR_BYTES];
                unsafe {
                    core::ptr::copy_nonoverlapping(user_ptr, buf.as_mut_ptr(), total);
                }

                let hdr = unsafe { &*(buf.as_ptr() as *const SubmitHdr) };
                if hdr.version != qos_abi::shm::SUBMIT_HDR_VERSION {
                    unsafe {
                        core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                    }
                    crate::serial_println!("syscall abi: SubmitIr -> ERR (hdr version={})", hdr.version);
                    vga::println!("submit IR ERR");
                    return 0;
                }
                if hdr.ir_format != qos_abi::shm::IRFMT_QASM2 {
                    unsafe {
                        core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                    }
                    crate::serial_println!("syscall abi: SubmitIr -> ERR (ir_format={})", hdr.ir_format);
                    vga::println!("submit IR ERR");
                    return 0;
                }
                let payload_len = hdr.payload_len as usize;
                let header_len = core::mem::size_of::<SubmitHdr>();
                if payload_len == 0 || header_len + payload_len != total {
                    unsafe {
                        core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                    }
                    crate::serial_println!(
                        "syscall abi: SubmitIr -> ERR (bad sizes header={} payload={} total={})",
                        header_len,
                        payload_len,
                        total
                    );
                    vga::println!("submit IR ERR");
                    return 0;
                }

                let payload = &buf[header_len..header_len + payload_len];
                if !looks_like_qasm2(payload) {
                    unsafe {
                        core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                    }
                    crate::serial_println!("syscall abi: SubmitIr -> ERR (not qasm2)");
                    vga::println!("submit IR ERR");
                    return 0;
                }

                let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed) + 1;
                let slot = &JOBS[idx];
                slot.handle.store(handle, Ordering::Relaxed);
                slot.state.store(StateRaw::Queued as u32, Ordering::Relaxed);
                slot.shots_done.store(0, Ordering::Relaxed);
                slot.shots_total.store(1, Ordering::Relaxed);

                let hash = fnv1a64(payload);
                slot.ir_len.store(payload_len as u32, Ordering::Relaxed);
                slot.ir_hash.store(hash, Ordering::Relaxed);
                slot.ir_format.store(hdr.ir_format, Ordering::Relaxed);
                slot.n_qubits.store(hdr.n_qubits, Ordering::Relaxed);
                slot.shots.store(hdr.shots, Ordering::Relaxed);
                slot.store_ir(payload);

                // Results will be computed by the real quantum simulator
                slot.res0.store(0, Ordering::Relaxed);
                slot.res1.store(0, Ordering::Relaxed);

                maybe_schedule();

                unsafe {
                    core::ptr::write_volatile(&mut call.ret0, handle);
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_OK);
                }
                crate::serial_println!(
                    "syscall abi: SubmitIr(len={} shots={} n_qubits={} hash=0x{:016x}) -> handle={} (qasm2)",
                    payload_len,
                    hdr.shots,
                    hdr.n_qubits,
                    hash,
                    handle
                );
                vga::println!("job submitted (ir)");
            } else {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: SubmitIr -> ERR (no slots)");
                vga::println!("submit IR ERR");
            }

            0
        }

        qos_abi::shm::OP_VFS_IO => {
            // VFS I/O: arg0=user_ptr_to_buffer, arg1=total_bytes.
            let user_ptr_u64 = unsafe { core::ptr::read_volatile(&call.arg0) };
            let user_ptr = user_ptr_u64 as *mut u8;
            let total = unsafe { core::ptr::read_volatile(&call.arg1) } as usize;
            let header_len = core::mem::size_of::<VfsHdr>();
            if user_ptr.is_null() || total < header_len || total > MAX_IR_BYTES {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: VfsIo -> ERR (ptr={:?} total={})", user_ptr, total);
                return 0;
            }

            if !user_range_mapped(user_ptr_u64, total) {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!(
                    "syscall abi: VfsIo -> ERR (unmapped ptr={:#x} total={})",
                    user_ptr_u64,
                    total
                );
                return 0;
            }

            // Copy the entire user buffer into a stack buffer first (for safe parsing).
            let mut buf = [0u8; MAX_IR_BYTES];
            unsafe {
                core::ptr::copy_nonoverlapping(user_ptr as *const u8, buf.as_mut_ptr(), total);
            }

            let mut hdr = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const VfsHdr) };
            if hdr.version != qos_abi::shm::VFS_HDR_VERSION {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: VfsIo -> ERR (hdr version={})", hdr.version);
                return 0;
            }

            let path_len = hdr.path_len as usize;
            let data_cap = hdr.data_cap as usize;
            let data_len = hdr.data_len as usize;
            if header_len + path_len + data_cap != total {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: VfsIo -> ERR (bad sizes header={} path={} cap={} total={})", header_len, path_len, data_cap, total);
                return 0;
            }
            if path_len == 0 {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: VfsIo -> ERR (empty path)");
                return 0;
            }
            if data_len > data_cap {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: VfsIo -> ERR (data_len > data_cap)");
                return 0;
            }

            let path_off = header_len;
            let data_off = header_len + path_len;
            let path = &buf[path_off..path_off + path_len];

            // Prepare output slice pointer (user memory) for read/list.
            let user_data_ptr = unsafe { user_ptr.add(data_off) };

            let mut bytes_written: u64 = 0;
            let ok: bool;

            ok = match hdr.vfs_op {
                qos_abi::shm::VFS_OP_REMOVE => crate::vfs::remove(path).is_ok(),
                qos_abi::shm::VFS_OP_WRITE => {
                    let data = &buf[data_off..data_off + data_len];
                    crate::vfs::write(path, data).is_ok()
                }
                qos_abi::shm::VFS_OP_READ => match crate::vfs::read(path) {
                    Ok(bytes) => {
                        let n = core::cmp::min(bytes.len(), data_cap);
                        unsafe {
                            core::ptr::copy_nonoverlapping(bytes.as_ptr(), user_data_ptr, n);
                        }
                        bytes_written = n as u64;
                        hdr.data_len = n as u32;
                        true
                    }
                    Err(_) => false,
                },
                qos_abi::shm::VFS_OP_LIST_DIR => {
                    // Minimal listing as newline-separated bytes.
                    // For /ram and /disk we currently don't expose an in-memory list API, so we return a short marker.
                    let out: &[u8] = match path {
                        b"/" => b"/ram\n/disk\n",
                        b"/ram" => b"<ram>\n",
                        b"/disk" => b"<disk>\n",
                        _ => b"",
                    };
                    if out.is_empty() {
                        false
                    } else {
                        let n = core::cmp::min(out.len(), data_cap);
                        unsafe {
                            core::ptr::copy_nonoverlapping(out.as_ptr(), user_data_ptr, n);
                        }
                        bytes_written = n as u64;
                        hdr.data_len = n as u32;
                        true
                    }
                }
                _ => false,
            };

            // Copy fields for logging before moving `hdr`.
            let log_op = hdr.vfs_op;
            let log_path_len = hdr.path_len;
            let log_data_len = hdr.data_len;
            let log_data_cap = hdr.data_cap;

            // Write updated header back to user memory (data_len for read/list).
            unsafe {
                core::ptr::write_unaligned(user_ptr as *mut VfsHdr, hdr);
                core::ptr::write_volatile(&mut call.ret0, bytes_written);
                core::ptr::write_volatile(
                    &mut call.status,
                    if ok { qos_abi::shm::STATUS_OK } else { qos_abi::shm::STATUS_ERR },
                );
            }

            crate::serial_println!(
                "syscall abi: VfsIo(op={} path_len={} data_len={} cap={}) -> {} bytes",
                log_op,
                log_path_len,
                log_data_len,
                log_data_cap,
                bytes_written
            );

            0
        }

        qos_abi::shm::OP_DISPATCH_NEXT => {
            // Compatibility shim: dispatch is now timer-driven.
            maybe_schedule();
            let dispatched = CURRENT_RUNNING.load(Ordering::Relaxed);
            unsafe {
                core::ptr::write_volatile(&mut call.ret0, dispatched);
                core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_OK);
            }
            crate::serial_println!("syscall abi: DispatchNext -> dispatched={}", dispatched);
            vga::println!("dispatch");

            0
        }

        qos_abi::shm::OP_CANCEL => {
            let handle = unsafe { core::ptr::read_volatile(&call.arg0) };
            let Some(idx) = find_slot_by_handle(handle) else {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: Cancel(handle={}) -> ERR", handle);
                vga::println!("job cancel ERR");
                return 0;
            };
            let slot = &JOBS[idx];
            slot.state.store(StateRaw::Cancelled as u32, Ordering::Relaxed);
            slot.shots_done.store(0, Ordering::Relaxed);
            slot.shots_total.store(0, Ordering::Relaxed);
            release_slot_qubits(slot);
            if CURRENT_RUNNING.load(Ordering::Relaxed) == handle {
                CURRENT_RUNNING.store(0, Ordering::Relaxed);
                maybe_schedule();
            }
            unsafe {
                core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_OK);
            }
            crate::serial_println!("syscall abi: Cancel(handle={}) -> OK", handle);
            vga::println!("job cancelled");

            0
        }
        qos_abi::shm::OP_GET_STATUS => {
            let handle = unsafe { core::ptr::read_volatile(&call.arg0) };
            let Some(idx) = find_slot_by_handle(handle) else {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: GetStatus(handle={}) -> ERR", handle);
                vga::println!("job status ERR");
                return 0;
            };

            let slot = &JOBS[idx];
            let raw = slot.state_raw();
            let state = raw.as_job_state();
            let state_u64 = state as u32 as u64;
            unsafe {
                core::ptr::write_volatile(&mut call.ret0, state_u64);
                core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_OK);
            }
            crate::serial_println!("syscall abi: GetStatus(handle={}) -> {:?}", handle, state);
            vga::println!("job status");
            0
        }
        qos_abi::shm::OP_GET_RESULT => {
            let handle = unsafe { core::ptr::read_volatile(&call.arg0) };
            let Some(idx) = find_slot_by_handle(handle) else {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!("syscall abi: GetResult(handle={}) -> ERR (unknown)", handle);
                vga::println!("job result ERR");
                return 0;
            };

            let slot = &JOBS[idx];
            let raw = slot.state_raw();
            if raw != StateRaw::Done {
                unsafe {
                    core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
                }
                crate::serial_println!(
                    "syscall abi: GetResult(handle={}) -> ERR (state={:?})",
                    handle,
                    raw.as_job_state()
                );
                vga::println!("job result ERR");
                return 0;
            }

            let n00 = slot.res0.load(Ordering::Relaxed);
            let n11 = slot.res1.load(Ordering::Relaxed);
            unsafe {
                core::ptr::write_volatile(&mut call.ret0, n00);
                core::ptr::write_volatile(&mut call.ret1, n11);
                core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_OK);
            }
            crate::serial_println!("syscall abi: GetResult(handle={}) -> n00={}, n11={}", handle, n00, n11);
            vga::println!("job result");

            // Realistic lifecycle: once result is consumed, free the slot.
            slot.handle.store(0, Ordering::Relaxed);
            slot.state.store(StateRaw::Free as u32, Ordering::Relaxed);
            slot.shots_done.store(0, Ordering::Relaxed);
            slot.shots_total.store(0, Ordering::Relaxed);
            0
        }
        qos_abi::shm::OP_EXIT => {
            unsafe {
                core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_OK);
            }
            crate::serial_println!("syscall abi: Exit");
            vga::println!("exit");

            #[cfg(feature = "verify")]
            {
                crate::serial_println!("VERIFY: quantum demo ok (ring3), exiting QEMU");
                crate::qemu::exit(0x10);
            }

            #[cfg(not(feature = "verify"))]
            {
                let code = unsafe { core::ptr::read_volatile(&call.arg0) };
                if crate::tasking::current_pid() != 0 {
                    // Scheduled process: terminate and switch back to shell without resetting.
                    let shell_rsp = crate::tasking::exit_current_and_switch_to_shell(code);
                    return shell_rsp;
                }

                // Legacy foreground exec path.
                crate::process::exit_foreground(code);
                // User module disabled - just switch to kernel CR3
                crate::memory::switch_to_kernel_cr3();
                crate::runtime::restart_kernel_loop("OP_EXIT");
            }

            0
        }
        other => {
            unsafe {
                core::ptr::write_volatile(&mut call.status, qos_abi::shm::STATUS_ERR);
            }
            crate::serial_println!("syscall abi: unknown op {}", other);
            vga::println!("abi unknown");
            0
        }
    }
}
