//! Background quantum job execution on a **preemptive kernel thread** (WP-08 slice 1).
//!
//! The modern desktop arms the kthread scheduler and spawns one job worker beside itself
//! (the desktop stays the main context, index 0). Quantum runs submitted from the UI execute
//! on the worker while the APIC timer preempts between it and the desktop — so a heavy
//! simulation no longer freezes input, window drags or the clock. This is timer-driven
//! preemption doing production work, not a demo.
//!
//! Concurrency model (single core): state hand-off via one atomic; job/result payloads behind
//! spin mutexes held only for the enqueue/dequeue instant. Spin locks here never disable
//! interrupts, so if either thread spins on a briefly-held lock the timer still fires and the
//! holder gets scheduled to release it — progress is guaranteed.

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU8, Ordering};

use spin::Mutex;

use crate::quantum::parser::Instruction;
use crate::quantum::sim::{self, SimResult};

pub const IDLE: u8 = 0;
pub const QUEUED: u8 = 1;
pub const RUNNING: u8 = 2;
pub const DONE: u8 = 3;

/// Which UI surface submitted the job (so the result lands back in the right window).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    Lab,
    Ide,
}

pub struct Job {
    pub origin: Origin,
    pub n_qubits: usize,
    pub n_cbits: usize,
    pub instrs: Vec<Instruction>,
    pub shots: u64,
}

static STATE: AtomicU8 = AtomicU8::new(IDLE);
static JOB: Mutex<Option<Job>> = Mutex::new(None);
static RESULT: Mutex<Option<(Origin, Option<SimResult>)>> = Mutex::new(None);

/// Current worker state (IDLE/QUEUED/RUNNING/DONE) — shown by the Processes app.
pub fn state() -> u8 {
    STATE.load(Ordering::SeqCst)
}

/// Submit a job to the background worker. Returns `false` (job refused) if one is in flight.
pub fn submit(job: Job) -> bool {
    if STATE
        .compare_exchange(IDLE, QUEUED, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return false;
    }
    *JOB.lock() = Some(job);
    true
}

/// Non-blocking: fetch a finished job's result exactly once, freeing the worker.
pub fn take_result() -> Option<(Origin, Option<SimResult>)> {
    if STATE.load(Ordering::SeqCst) != DONE {
        return None;
    }
    let r = RESULT.lock().take();
    STATE.store(IDLE, Ordering::SeqCst);
    r
}

extern "C" fn worker() -> ! {
    loop {
        if STATE.load(Ordering::SeqCst) == QUEUED {
            let job = JOB.lock().take();
            match job {
                Some(j) => {
                    STATE.store(RUNNING, Ordering::SeqCst);
                    crate::serial_println!(
                        "[QJOB] worker: running {} qubits x {} shots (preemptive)",
                        j.n_qubits,
                        j.shots
                    );
                    let res = sim::run_program(j.n_qubits, j.n_cbits, j.instrs, j.shots);
                    *RESULT.lock() = Some((j.origin, res));
                    STATE.store(DONE, Ordering::SeqCst);
                    crate::serial_println!("[QJOB] worker: done");
                }
                None => STATE.store(IDLE, Ordering::SeqCst),
            }
        }
        // Idle (or between jobs): sleep until the next timer tick reschedules us.
        x86_64::instructions::hlt();
    }
}

/// Arm preemption with the job worker beside the main (desktop) context. If a previous session
/// left a job mid-flight (desktop exited while RUNNING), the stale job is dropped — its heap
/// allocations leak once, which the 64 MiB heap absorbs; logged for honesty.
pub fn start() {
    if STATE.load(Ordering::SeqCst) != IDLE {
        crate::serial_println!("[QJOB] dropping stale job from a previous desktop session");
        *JOB.lock() = None;
        *RESULT.lock() = None;
        STATE.store(IDLE, Ordering::SeqCst);
    }
    crate::kthread::reset();
    crate::kthread::spawn(worker);
    crate::kthread::arm();
    crate::serial_println!("[QJOB] preemptive job worker armed");
}

/// Disarm preemption (desktop exiting to the shell). A RUNNING job is abandoned (see start()).
pub fn stop() {
    crate::kthread::disarm();
    crate::kthread::reset();
}
