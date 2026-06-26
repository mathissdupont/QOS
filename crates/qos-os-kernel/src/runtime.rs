use crate::{arch, scheduler, serial, shell, syscall, ui, vga};

const KERNEL_LOOP_STACK_SIZE: usize = 4096 * 32; // 128KB stack
#[repr(align(16))]
struct AlignedStack([u8; KERNEL_LOOP_STACK_SIZE]);
static mut KERNEL_LOOP_STACK: AlignedStack = AlignedStack([0; KERNEL_LOOP_STACK_SIZE]);

extern "C" fn kernel_loop_entry() -> ! {
    serial::println!("[KERNEL_LOOP] Entry! Starting new scheduler...");
    vga::println!("kernel restarting...");
    
    // We may arrive here from an interrupt handler path with interrupts disabled.
    arch::enable_interrupts();

    // Default to UI on for interactive runs; keep it off for `os-verify`.
    if !cfg!(feature = "verify") {
        // Fixed bottom input line.
        vga::set_reserved_bottom_rows(1);
        ui::set_enabled(true);
    } else {
        vga::set_reserved_bottom_rows(0);
    }

    let mut sched = scheduler::Scheduler::new();
    serial::println!("[KERNEL_LOOP] Adding tasks...");
    sched.add_task(scheduler::HeartbeatTask::new(100));
    sched.add_task(ui::UiTask::new());
    sched.add_task(shell::ShellTask::new());
    serial::println!("[KERNEL_LOOP] Tasks added, entering main loop");

    loop {
        // Process quantum work from main loop (NOT interrupt) to avoid deadlocks
        syscall::process_quantum_work();
        sched.step();
        arch::hlt();
    }
}

pub fn run_kernel_loop() -> ! {
    kernel_loop_entry()
}

/// Restart the scheduler + shell after returning from user mode.
///
/// This intentionally discards any previous scheduler state. It also switches to a dedicated
/// kernel-loop stack to avoid unbounded stack growth if `exec` is used repeatedly.
pub fn restart_kernel_loop(reason: &'static str) -> ! {
    let p = crate::process::current();
    serial::println!(
        "user exit: {} (pid={} state={:?} code={}) -> restarting shell",
        reason,
        p.pid,
        p.state,
        p.exit_code
    );
    vga::println!("user exited -> shell");

    // Switch to dedicated kernel loop stack and restart
    unsafe {
        let stack_top = KERNEL_LOOP_STACK.0.as_ptr().add(KERNEL_LOOP_STACK_SIZE) as u64;
        serial::println!("[RESTART] Switching to new stack at {:#x}", stack_top);
        core::arch::asm!(
            "mov rsp, {stack}",
            "jmp {entry}",
            stack = in(reg) stack_top,
            entry = in(reg) kernel_loop_entry as usize,
            options(noreturn)
        );
    }
}
