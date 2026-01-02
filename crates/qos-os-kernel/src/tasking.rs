use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;
use x86_64::VirtAddr;
use x86_64::structures::paging::{Page, PhysFrame, Size4KiB};

use crate::{gdt, memory};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcState {
    Ready,
    Running,
    Exited,
}

pub struct Process {
    pub pid: u64,
    pub state: ProcState,
    pub exit_code: u64,

    // Saved kernel stack pointer (points to the saved-regs block used by our trap stubs).
    pub saved_rsp: u64,

    // Kernel stack backing storage (kept so the memory stays alive).
    pub kstack: Vec<u8>,
    pub kstack_top: VirtAddr,

    // User address space root.
    pub user_cr3: PhysFrame<Size4KiB>,

    // Per-process user mappings to tear down on exit.
    pub mapped_pages: Vec<Page<Size4KiB>>,
}

static NEXT_PID: AtomicU64 = AtomicU64::new(1);
static CURRENT_PID: AtomicU64 = AtomicU64::new(0);
static SHELL_SAVED_RSP: AtomicU64 = AtomicU64::new(0);
static LAST_IDX: AtomicU64 = AtomicU64::new(0);

// Foreground scheduled PID (0 means none). Used for Ctrl+C targeting and simple fg waits.
static FOREGROUND_PID: AtomicU64 = AtomicU64::new(0);

// If non-zero: request to terminate the current running scheduled process with this exit code.
static KILL_CURRENT_EXIT_CODE: AtomicU64 = AtomicU64::new(0);

// Monotonic counter incremented whenever any scheduled process exits.
// Used by `wait` loops to avoid scanning on every timer tick.
static EXIT_SEQ: AtomicU64 = AtomicU64::new(0);

static PROCS: Mutex<Vec<Process>> = Mutex::new(Vec::new());

const REGS_QWORDS: u64 = 15;

fn frame_cs(saved_rsp: u64) -> u64 {
    // Layout: [r15..rax] then hardware iret frame.
    // iret frame starts at +15*8, CS is the second qword.
    unsafe { *((saved_rsp + (REGS_QWORDS * 8) + 8) as *const u64) }
}

fn is_user_frame(saved_rsp: u64) -> bool {
    (frame_cs(saved_rsp) & 0x3) == 3
}

pub fn shell_saved_rsp() -> u64 {
    SHELL_SAVED_RSP.load(Ordering::Relaxed)
}

pub fn foreground_pid() -> u64 {
    FOREGROUND_PID.load(Ordering::Relaxed)
}

pub fn set_foreground(pid: u64) {
    FOREGROUND_PID.store(pid, Ordering::Relaxed);
}

pub fn clear_foreground(pid: u64) {
    // Only clear if it still matches (avoid races).
    let _ = FOREGROUND_PID.compare_exchange(pid, 0, Ordering::Relaxed, Ordering::Relaxed);
}

pub fn exit_seq() -> u64 {
    EXIT_SEQ.load(Ordering::Relaxed)
}

fn bump_exit_seq() {
    EXIT_SEQ.fetch_add(1, Ordering::Relaxed);
}

pub fn spawn_user_process(
    user_cr3: PhysFrame<Size4KiB>,
    entry: VirtAddr,
    user_stack_top: VirtAddr,
    mapped_pages: Vec<Page<Size4KiB>>,
) -> u64 {
    let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);

    // 32 KiB kernel stack per process.
    let mut kstack = alloc::vec![0u8; 4096 * 8];
    let base = kstack.as_ptr() as u64;
    let top = base + kstack.len() as u64;
    let top_aligned = top & !0xF;

    // Build initial stack layout expected by our trap stubs:
    // [r15..rax]=0 then iretq frame for Ring3: rip, cs, rflags, rsp, ss.
    let mut sp = top_aligned;

    unsafe fn push(sp: &mut u64, v: u64) {
        *sp -= 8;
        *(*sp as *mut u64) = v;
    }

    let sel = crate::gdt::selectors();
    // When manually building an iret frame, we must set RPL=3 in the selector values.
    // The GDT selectors themselves have RPL=0 by default.
    let user_cs = (sel.user_code.0 | 0b11) as u64;
    let user_ss = (sel.user_data.0 | 0b11) as u64;
    let rflags: u64 = 0x202;

    unsafe {
        push(&mut sp, user_ss);
        push(&mut sp, user_stack_top.as_u64());
        push(&mut sp, rflags);
        push(&mut sp, user_cs);
        push(&mut sp, entry.as_u64());

        // 15 GPRs (match push/pop order in trap stubs).
        for _ in 0..REGS_QWORDS {
            push(&mut sp, 0);
        }
    }

    let proc = Process {
        pid,
        state: ProcState::Ready,
        exit_code: 0,
        saved_rsp: sp,
        kstack,
        kstack_top: VirtAddr::new(top_aligned),
        user_cr3,
        mapped_pages,
    };

    PROCS.lock().push(proc);
    pid
}

pub fn list_processes() -> alloc::vec::Vec<(u64, ProcState, u64)> {
    let procs = PROCS.lock();
    procs.iter().map(|p| (p.pid, p.state, p.exit_code)).collect()
}

pub fn kill(pid: u64) -> bool {
    kill_with_exit(pid, 0x100 + 9)
}

pub fn kill_with_exit(pid: u64, exit_code: u64) -> bool {
    let current = CURRENT_PID.load(Ordering::Relaxed);
    if current == pid && current != 0 {
        KILL_CURRENT_EXIT_CODE.store(exit_code, Ordering::Relaxed);
        return true;
    }

    let mut procs = PROCS.lock();
    let Some(p) = procs.iter_mut().find(|p| p.pid == pid) else {
        return false;
    };

    if p.state == ProcState::Exited {
        return true;
    }

    p.state = ProcState::Exited;
    p.exit_code = exit_code;

    clear_foreground(pid);
    bump_exit_seq();

    // User module disabled - skip address space cleanup
    // crate::user::cleanup_spawned_user_process(p.user_cr3, &mut p.mapped_pages);
    true
}

pub fn request_kill_current(exit_code: u64) {
    KILL_CURRENT_EXIT_CODE.store(exit_code, Ordering::Relaxed);
}

pub fn find_process(pid: u64) -> Option<(ProcState, u64)> {
    let procs = PROCS.lock();
    procs
        .iter()
        .find(|p| p.pid == pid)
        .map(|p| (p.state, p.exit_code))
}

pub fn on_timer_trap(current_saved_rsp: u64) -> u64 {
    // Record the current context.
    if is_user_frame(current_saved_rsp) {
        let pid = CURRENT_PID.load(Ordering::Relaxed);
        if pid == 0 {
            // User mode is running outside the scheduler (e.g. current verify/userdemo path).
            // Don't attempt to preempt/switch until user execution is represented as a Process.
            return 0;
        }

        let mut procs = PROCS.lock();
        if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
            p.saved_rsp = current_saved_rsp;
            if p.state == ProcState::Running {
                p.state = ProcState::Ready;
            }
        }

        // If we were asked to kill the current process (e.g. Ctrl+C), do it now and return to shell.
        let kill_code = KILL_CURRENT_EXIT_CODE.swap(0, Ordering::Relaxed);
        if kill_code != 0 {
            return exit_current_and_switch_to_shell(kill_code);
        }
    } else {
        SHELL_SAVED_RSP.store(current_saved_rsp, Ordering::Relaxed);
    }

    // Round-robin pick next runnable context among: shell + READY user processes.
    // Indexing: 0 = shell, 1..=N = procs[0..N-1].
    let mut procs = PROCS.lock();
    let n = procs.len();
    let total = n + 1;
    if total == 0 {
        return 0;
    }

    // Prefer foreground process if it exists and is runnable.
    let fg = FOREGROUND_PID.load(Ordering::Relaxed);
    if fg != 0 {
        if let Some(p) = procs.iter_mut().find(|p| p.pid == fg) {
            if p.state == ProcState::Ready {
                CURRENT_PID.store(p.pid, Ordering::Relaxed);
                p.state = ProcState::Running;
                gdt::set_rsp0(p.kstack_top);
                memory::switch_cr3(p.user_cr3);
                return p.saved_rsp;
            }
        }
    }

    let start = (LAST_IDX.load(Ordering::Relaxed) as usize + 1) % total;
    let mut choice: Option<usize> = None;
    for off in 0..total {
        let idx = (start + off) % total;
        if idx == 0 {
            if SHELL_SAVED_RSP.load(Ordering::Relaxed) != 0 {
                choice = Some(0);
                break;
            }
        } else {
            let p = &procs[idx - 1];
            if p.state == ProcState::Ready {
                choice = Some(idx);
                break;
            }
        }
    }

    let Some(idx) = choice else {
        // No runnable contexts; keep running current.
        return 0;
    };

    LAST_IDX.store(idx as u64, Ordering::Relaxed);
    if idx == 0 {
        // Switch back to shell (kernel CR3).
        CURRENT_PID.store(0, Ordering::Relaxed);
        memory::switch_to_kernel_cr3();
        return SHELL_SAVED_RSP.load(Ordering::Relaxed);
    }

    let p = &mut procs[idx - 1];
    let next_pid = p.pid;
    CURRENT_PID.store(next_pid, Ordering::Relaxed);
    p.state = ProcState::Running;

    // Install per-process RSP0 and CR3 before returning to it.
    gdt::set_rsp0(p.kstack_top);
    memory::switch_cr3(p.user_cr3);

    p.saved_rsp
}

pub fn on_return_to_shell() {
    CURRENT_PID.store(0, Ordering::Relaxed);
    memory::switch_to_kernel_cr3();
}

pub fn exit_current(exit_code: u64) {
    let pid = CURRENT_PID.load(Ordering::Relaxed);
    if pid == 0 {
        return;
    }

    let mut procs = PROCS.lock();
    if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
        p.state = ProcState::Exited;
        p.exit_code = exit_code;
        clear_foreground(pid);
        bump_exit_seq();
    }
}

pub fn exit_current_and_switch_to_shell(exit_code: u64) -> u64 {
    let pid = CURRENT_PID.load(Ordering::Relaxed);
    if pid == 0 {
        return 0;
    }

    {
        let mut procs = PROCS.lock();
        if let Some(p) = procs.iter_mut().find(|p| p.pid == pid) {
            p.state = ProcState::Exited;
            p.exit_code = exit_code;

            // User module disabled - skip address space cleanup
            // crate::user::cleanup_spawned_user_process(p.user_cr3, &mut p.mapped_pages);
        }
    }

    CURRENT_PID.store(0, Ordering::Relaxed);
    clear_foreground(pid);
    bump_exit_seq();
    memory::switch_to_kernel_cr3();
    SHELL_SAVED_RSP.load(Ordering::Relaxed)
}

pub fn current_pid() -> u64 {
    CURRENT_PID.load(Ordering::Relaxed)
}

pub fn current_is_user(saved_rsp: u64) -> bool {
    is_user_frame(saved_rsp)
}
