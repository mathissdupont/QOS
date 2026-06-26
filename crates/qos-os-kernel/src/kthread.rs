//! Preemptive kernel threads (Phase 2.1 — see docs/PLAN.md).
//!
//! This is the keystone of the real-OS core: a timer-driven context switch. The raw timer
//! ISR (`asm_timer_isr` in `asm_stubs.rs`) saves all GPRs on top of the hardware iretq frame
//! and calls [`schedule`], which saves the interrupted thread's stack pointer and returns the
//! stack pointer of the thread to resume. Switching is therefore just "swap the stack pointer
//! and `iretq`" — the GPRs and the instruction pointer ride along on each thread's own stack.
//!
//! Threads run in Ring 0 (kernel mode). Index 0 of the thread table is the *main* context (the
//! kernel loop / shell); workers are spawned at index ≥ 1. Preemption is only active while
//! [`arm`]ed, so normal boot and the graphical desktop are completely unaffected.
//!
//! Single-core only. State is guarded by a spin lock that is taken with interrupts disabled
//! (the ISR runs on an interrupt gate); the ISR uses `try_lock` and simply skips a switch if
//! the lock is momentarily held by non-interrupt setup code — never blocking in interrupt
//! context.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use spin::Mutex;
use x86_64::{PhysAddr, VirtAddr};
use x86_64::structures::paging::{PhysFrame, Size4KiB};

use crate::{gdt, memory};

/// 32 KiB per kernel thread stack.
const KSTACK_SIZE: usize = 4096 * 8;
/// 15 general-purpose registers saved by the ISR, matching the asm push/pop order.
const REGS_QWORDS: usize = 15;

/// Address-space binding for a thread that runs (or drops into) Ring 3. On switch-in the
/// scheduler installs this CR3 and sets TSS.RSP0 so a Ring-3 interrupt lands on the right
/// kernel stack. Pure Ring-0 threads (main context, kernel workers) leave this `None` and run
/// in the kernel address space.
#[derive(Clone, Copy)]
struct UserCtx {
    cr3: PhysFrame<Size4KiB>,
    rsp0_top: VirtAddr,
}

struct KThread {
    id: u64,
    /// Saved kernel stack pointer (points at the r15 slot of the saved-regs block).
    saved_rsp: u64,
    /// Backing storage for the thread stack — kept alive for the thread's lifetime.
    /// Empty for the main context (index 0), which runs on the kernel-loop stack.
    #[allow(dead_code)]
    stack: Vec<u8>,
    /// Ring-3 address space, if this thread is a user process.
    user: Option<UserCtx>,
    finished: bool,
}

/// Physical address of the currently-installed CR3, so we only reload (and flush the TLB)
/// when the address space actually changes. 0 means "kernel CR3".
static ACTIVE_CR3: AtomicU64 = AtomicU64::new(0);

/// Kernel CR3 physical address, cached at `arm()` time (from the main context, never an
/// interrupt). The scheduler runs in interrupt context, so it must NOT take any lock that the
/// main thread could be holding when preempted — reading this atomic and writing CR3 directly
/// avoids `switch_to_kernel_cr3()`'s `KERNEL_CR3` spin lock entirely.
static KERNEL_CR3_PHYS: AtomicU64 = AtomicU64::new(0);

/// Install the address space for the thread we are about to resume, avoiding a needless TLB
/// flush when it is unchanged. Lock-free: safe to call from the timer ISR.
fn apply_address_space(user: Option<UserCtx>) {
    match user {
        Some(u) => {
            gdt::set_rsp0(u.rsp0_top);
            let phys = u.cr3.start_address().as_u64();
            if ACTIVE_CR3.swap(phys, Ordering::SeqCst) != phys {
                memory::switch_cr3(u.cr3);
            }
        }
        None => {
            if ACTIVE_CR3.swap(0, Ordering::SeqCst) != 0 {
                let kphys = KERNEL_CR3_PHYS.load(Ordering::SeqCst);
                if kphys != 0 {
                    let frame = PhysFrame::containing_address(PhysAddr::new(kphys));
                    memory::switch_cr3(frame);
                }
            }
        }
    }
}

static THREADS: Mutex<Vec<KThread>> = Mutex::new(Vec::new());
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Reset the thread table to just the main context. Call before spawning a batch of workers.
pub fn reset() {
    let mut g = THREADS.lock();
    g.clear();
    g.push(KThread { id: 0, saved_rsp: 0, stack: Vec::new(), user: None, finished: false });
    CURRENT.store(0, Ordering::SeqCst);
    ACTIVE_CR3.store(0, Ordering::SeqCst);
}

/// Spawn a kernel thread that begins executing `entry` in Ring 0. `entry` must not return
/// normally — it should call [`exit`] when done. Returns the thread id.
pub fn spawn(entry: extern "C" fn() -> !) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

    let mut stack = alloc::vec![0u8; KSTACK_SIZE];
    let base = stack.as_ptr() as u64;
    let top = (base + stack.len() as u64) & !0xF;

    // Build the frame the ISR will restore on first run: a Ring-0 iretq frame followed by
    // 15 zeroed GPRs. iretq in long mode always pops ss:rsp, so we provide them.
    let sel = gdt::selectors();
    let cs = sel.kernel_code.0 as u64; // RPL 0
    let ss = 0u64; // null SS is valid when iret targets CPL 0 in 64-bit mode
    let rflags = 0x202u64; // IF set, reserved bit 1 set

    let mut sp = top;
    unsafe fn push(sp: &mut u64, v: u64) {
        *sp -= 8;
        *(*sp as *mut u64) = v;
    }
    unsafe {
        push(&mut sp, ss);
        push(&mut sp, top); // thread's own rsp once running
        push(&mut sp, rflags);
        push(&mut sp, cs);
        push(&mut sp, entry as u64); // rip
        for _ in 0..REGS_QWORDS {
            push(&mut sp, 0);
        }
    }

    THREADS.lock().push(KThread { id, saved_rsp: sp, stack, user: None, finished: false });
    id
}

/// Register an already-built Ring-3 process as a schedulable thread. `saved_rsp` must point at
/// a frame on the process's *kernel* stack that, when restored by the ISR and `iretq`-ed,
/// enters Ring 3 (exactly the layout produced by `tasking::spawn_user_process`). `cr3` is the
/// process page table and `rsp0_top` its kernel stack top (installed into TSS.RSP0 on
/// switch-in so a Ring-3 interrupt lands on this stack). Returns the kthread id.
pub fn adopt_user(
    saved_rsp: u64,
    cr3: PhysFrame<Size4KiB>,
    rsp0_top: VirtAddr,
) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    THREADS.lock().push(KThread {
        id,
        saved_rsp,
        stack: Vec::new(), // the process owns its kernel stack (in tasking::Process)
        user: Some(UserCtx { cr3, rsp0_top }),
        finished: false,
    });
    id
}

/// Enable preemptive switching among the spawned threads. Caches the kernel CR3 so the
/// (interrupt-context) scheduler can return to the kernel address space without locking.
pub fn arm() {
    let kphys = memory::kernel_cr3_frame().start_address().as_u64();
    KERNEL_CR3_PHYS.store(kphys, Ordering::SeqCst);
    ARMED.store(true, Ordering::SeqCst);
}

/// Disable preemptive switching (the main context keeps running).
pub fn disarm() {
    ARMED.store(false, Ordering::SeqCst);
}

#[inline]
pub fn armed() -> bool {
    ARMED.load(Ordering::SeqCst)
}

/// True once every worker (index ≥ 1) has called [`exit`].
pub fn all_finished() -> bool {
    let g = THREADS.lock();
    g.iter().skip(1).all(|t| t.finished)
}

/// True if preemption is armed and the currently-running thread is a Ring-3 user process.
/// Used by fault/syscall handlers to decide whether to reap just this process (preemptive
/// path) or fall back to the legacy whole-kernel restart.
pub fn is_user_current() -> bool {
    if !ARMED.load(Ordering::SeqCst) {
        return false;
    }
    if let Some(g) = THREADS.try_lock() {
        let cur = CURRENT.load(Ordering::SeqCst);
        return cur < g.len() && g[cur].user.is_some();
    }
    false
}

/// Mark whichever thread is currently running as finished (used from an interrupt/fault
/// handler running on that thread's kernel stack).
pub fn mark_current_finished() {
    loop {
        if let Some(mut g) = THREADS.try_lock() {
            let cur = CURRENT.load(Ordering::SeqCst);
            if cur < g.len() {
                g[cur].finished = true;
            }
            return;
        }
        core::hint::spin_loop();
    }
}

/// Kill the current Ring-3 process and wait to be scheduled away. Called from a fault handler
/// when a user process misbehaves: we mark it finished, then enable interrupts and halt so the
/// next timer tick reschedules to another thread (the shell / other processes). The faulting
/// instruction is never retried, and only this process dies. Never returns.
pub fn kill_current_and_park() -> ! {
    mark_current_finished();
    loop {
        x86_64::instructions::interrupts::enable();
        x86_64::instructions::hlt();
    }
}

/// Mark the current thread finished and wait to be preempted away. Never returns.
pub fn exit() -> ! {
    loop {
        if let Some(mut g) = THREADS.try_lock() {
            let cur = CURRENT.load(Ordering::SeqCst);
            if cur < g.len() {
                g[cur].finished = true;
            }
            break;
        }
        core::hint::spin_loop();
    }
    loop {
        x86_64::instructions::hlt();
    }
}

/// Called from the timer ISR with the interrupted thread's saved stack pointer. Returns the
/// stack pointer to resume on. If no switch is needed (or the lock is contended), returns the
/// input unchanged so the current thread simply continues.
pub fn schedule(prev_rsp: u64) -> u64 {
    let Some(mut g) = THREADS.try_lock() else {
        return prev_rsp;
    };
    let n = g.len();
    if n == 0 {
        return prev_rsp;
    }

    let cur = CURRENT.load(Ordering::SeqCst);
    if cur < n {
        g[cur].saved_rsp = prev_rsp;
    }

    // Round-robin to the next non-finished thread (the main context, index 0, never finishes).
    let mut next = cur;
    for off in 1..=n {
        let idx = (cur + off) % n;
        if !g[idx].finished {
            next = idx;
            break;
        }
    }
    if next == cur {
        return prev_rsp;
    }

    CURRENT.store(next, Ordering::SeqCst);
    // Install the next thread's address space (CR3 + TSS.RSP0) before handing back its stack.
    // Kernel mappings are present in every CR3, so switching here is safe — our code, the lock,
    // and every thread's kernel stack all live in the kernel half.
    apply_address_space(g[next].user);
    g[next].saved_rsp
}

// ── Demonstration: two preemptive kernel threads (`threadtest` shell command) ──────────────

fn worker(tag: u8) -> ! {
    for i in 0..12u32 {
        crate::serial_print!("{}{} ", tag as char, i);
        // Busy delay so several timer ticks elapse mid-loop and the scheduler preempts us —
        // proving the switch is involuntary (neither thread ever yields cooperatively).
        for _ in 0..600_000u64 {
            core::hint::spin_loop();
        }
    }
    crate::serial_println!("\n[kthread {} finished]", tag as char);
    exit();
}

extern "C" fn worker_a() -> ! {
    worker(b'A')
}

extern "C" fn worker_b() -> ! {
    worker(b'B')
}

// ── Background worker for the graphical desktop (Phase 2.1b → 3) ───────────────────────────

/// A monotonically increasing counter bumped by the desktop's background kernel thread. The
/// GUI reads it to display live, preemptive multitasking happening *behind* an interactive
/// desktop (the desktop is the main context; this worker runs preempted alongside it).
static BG_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn bg_counter() -> u64 {
    BG_COUNTER.load(Ordering::Relaxed)
}

extern "C" fn bg_worker() -> ! {
    loop {
        BG_COUNTER.fetch_add(1, Ordering::Relaxed);
        // Small delay so the count advances visibly rather than overflowing instantly.
        for _ in 0..150_000u64 {
            core::hint::spin_loop();
        }
    }
}

/// Start a preemptive background worker beside the current (main) context. Used by the
/// graphical desktop so a real task keeps running while the user interacts with the GUI.
pub fn start_background_worker() {
    BG_COUNTER.store(0, Ordering::SeqCst);
    reset();
    spawn(bg_worker);
    arm();
}

/// Stop the background worker. Must be called from the main context (so we keep control).
pub fn stop_background_worker() {
    disarm();
    reset();
}

/// Spawn two kernel threads and let the timer preempt between them until both finish. The
/// interleaved `A0 B0 A1 B1 …` serial output is the proof that preemption works.
pub fn demo() {
    crate::serial_println!("[KTHREAD] threadtest: spawning 2 preemptive kernel threads");
    crate::println!("threadtest: running 2 preemptive kernel threads (see serial for A/B interleave)");
    reset();
    spawn(worker_a);
    spawn(worker_b);
    arm();

    // Wait in the main context; the timer preempts us into the workers and back.
    while !all_finished() {
        x86_64::instructions::hlt();
    }

    disarm();
    crate::serial_println!("[KTHREAD] threadtest: both threads finished, preemption OK");
    crate::println!("threadtest: done (both threads preempted to completion)");
}
