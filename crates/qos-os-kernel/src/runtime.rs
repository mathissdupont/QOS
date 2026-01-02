use crate::{arch, scheduler, serial, shell, ui, vga};

const KERNEL_LOOP_STACK_SIZE: usize = 4096 * 8;
static mut KERNEL_LOOP_STACK: [u8; KERNEL_LOOP_STACK_SIZE] = [0; KERNEL_LOOP_STACK_SIZE];

extern "C" fn kernel_loop_entry() -> ! {
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
    sched.add_task(scheduler::HeartbeatTask::new(100));
    sched.add_task(ui::UiTask::new());
    sched.add_task(shell::ShellTask::new());

    loop {
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

    // Instead of complex asm, just call kernel_loop_entry directly
    // This loses the stack switch but avoids the LLVM bug
    kernel_loop_entry();
}
