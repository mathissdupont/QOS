use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use lazy_static::lazy_static;
use pic8259::ChainedPics;
use x86_64::PrivilegeLevel;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::registers::control::Cr2;

use crate::{arch, gdt, keyboard, serial, syscall, vga};

pub static TICKS: AtomicU64 = AtomicU64::new(0);
static CTRL_DOWN: AtomicBool = AtomicBool::new(false);

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

/// Local-APIC spurious interrupt vector (E-10). Any vector > 31; conventionally 0xFF.
pub const SPURIOUS_VECTOR: u8 = 0xFF;
/// Vector the local-APIC timer delivers on once we move the scheduler tick off the PIT (E-10).
pub const APIC_TIMER_VECTOR: u8 = 0x40;
/// Vector the xHCI controller's MSI-X interrupt delivers on (WP-04 step 5). Any free vector > 31.
pub const XHCI_VECTOR: u8 = 0x41;

/// True once the scheduler tick is delivered by the local-APIC timer (EOI goes to the APIC, not
/// the PIC). Set by `apic::start_apic_timer_100hz`.
pub static APIC_TIMER: AtomicBool = AtomicBool::new(false);

/// True once external IRQs (keyboard/mouse) are delivered through the IO-APIC and the 8259 PIC is
/// masked off. Their EOI then goes to the local APIC. Set by `apic::start_ioapic_routing`.
pub static IOAPIC_ACTIVE: AtomicBool = AtomicBool::new(false);

/// End-of-interrupt for an external device IRQ: to the local APIC once the IO-APIC drives it,
/// otherwise to the 8259 PIC. `pic_vector` is the device's PIC vector (used only in PIC mode).
pub fn eoi_external(pic_vector: u8) {
    if IOAPIC_ACTIVE.load(Ordering::SeqCst) {
        crate::apic::eoi();
    } else {
        unsafe {
            PICS.lock().notify_end_of_interrupt(pic_vector);
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
    // IRQ 12 is on slave PIC (PIC_2_OFFSET + 4)
    Mouse = PIC_2_OFFSET + 4,
}

impl InterruptIndex {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

pub static PICS: spin::Mutex<ChainedPics> = spin::Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();

        idt.breakpoint.set_handler_fn(breakpoint_handler);

        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_fault_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);

        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }

        // Preemptive timer ISR (Phase 2.1): a raw global_asm entry that saves the full
        // register context so we can context-switch on the tick. `timer_dispatch` is its C
        // callback. Installed via set_handler_addr because the entry is not an
        // `extern "x86-interrupt"` fn.
        unsafe {
            idt[InterruptIndex::Timer.as_u8()]
                .set_handler_addr(x86_64::VirtAddr::new(crate::asm_stubs::asm_timer_isr as usize as u64));
        }
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt[InterruptIndex::Mouse.as_u8()].set_handler_fn(mouse_interrupt_handler);

        // Local-APIC timer (E-10): same preemptive ISR as the PIT timer, on its own vector. The
        // scheduler tick moves here once `apic::start_apic_timer_100hz` runs; `timer_dispatch`
        // then EOIs the local APIC instead of the PIC.
        unsafe {
            idt[APIC_TIMER_VECTOR]
                .set_handler_addr(x86_64::VirtAddr::new(crate::asm_stubs::asm_timer_isr as usize as u64));
        }

        // Local-APIC spurious vector (E-10): fired by the APIC on rare conditions; needs a
        // present IDT entry so it doesn't #GP, and requires NO end-of-interrupt.
        idt[SPURIOUS_VECTOR].set_handler_fn(spurious_interrupt_handler);

        // xHCI USB controller MSI-X interrupt (WP-04 step 5): delivered straight to the local APIC.
        idt[XHCI_VECTOR].set_handler_fn(xhci_interrupt_handler);

        // Syscall entry point (Milestone 2): shared-memory ABI.
        idt[0x80]
            .set_handler_fn(syscall::syscall_interrupt_handler)
            .set_privilege_level(PrivilegeLevel::Ring3);

        // Register-based syscall ABI (Phase 2.2): rax=number, rdi/rsi/...=args, return in rax.
        unsafe {
            idt[0x81]
                .set_handler_addr(x86_64::VirtAddr::new(crate::asm_stubs::asm_syscall_isr as usize as u64))
                .set_privilege_level(PrivilegeLevel::Ring3);
        }

        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

pub fn init_pics() {
    unsafe { PICS.lock().initialize() };

    // Deterministic IRQ masks:
    // - Unmask IRQ0 (timer) + IRQ1 (keyboard) + IRQ2 (cascade) on master PIC.
    // - Unmask IRQ12 (mouse) on slave PIC.
    unsafe {
        arch::outb(0x21, 0b1111_1000); // Master: Timer, Keyboard, Cascade
        arch::outb(0xA1, 0b1110_1111); // Slave: Mouse (IRQ12 = bit 4)
    }

    arch::enable_interrupts();
}

/// Local-APIC spurious interrupt (E-10). Per the SDM, no end-of-interrupt is sent for the
/// spurious vector — just return.
extern "x86-interrupt" fn spurious_interrupt_handler(_stack_frame: InterruptStackFrame) {}

fn is_user_mode(stack_frame: &InterruptStackFrame) -> bool {
    stack_frame.code_segment.rpl() == PrivilegeLevel::Ring3
}

/// Print a clear halt banner to both serial and screen, then stop. Used when a fault happens
/// in kernel context (no process to kill). Phase 0.3 (docs/PLAN.md).
fn kernel_halt(reason: &str) -> ! {
    serial::println!("*** KERNEL HALTED: {} *** (reboot to recover)", reason);
    vga::println!("");
    vga::println!("*** KERNEL HALTED: {} ***", reason);
    vga::println!("Reboot (restart QEMU) to recover.");
    loop {
        arch::hlt();
    }
}

fn terminate_user_and_restart(reason: &'static str, exit_code: u64) -> ! {
    crate::process::exit_foreground(exit_code);

    // User module disabled due to LLVM asm bug
    // Just switch back to kernel CR3 and restart
    crate::memory::switch_to_kernel_cr3();

    crate::runtime::restart_kernel_loop(reason)
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    serial::println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
    vga::println!("EXCEPTION: BREAKPOINT");
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    serial::println!("EXCEPTION: INVALID OPCODE\n{:#?}", stack_frame);
    vga::println!("EXCEPTION: INVALID OPCODE");

    if is_user_mode(&stack_frame) {
        if crate::kthread::is_user_current() {
            serial::println!("[user] #UD -> killing process; kernel survives");
            vga::println!("[user] invalid opcode -> process killed (others survive)");
            crate::kthread::kill_current_and_park();
        }
        terminate_user_and_restart("USER_UD", 0x100 + 6)
    }

    loop { arch::hlt(); }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    let rip = stack_frame.instruction_pointer.as_u64();
    serial::println!("EXCEPTION: GENERAL PROTECTION FAULT (code={:#x}, rip={:#x})\n{:#?}", error_code, rip, stack_frame);

    if is_user_mode(&stack_frame) {
        if crate::kthread::is_user_current() {
            serial::println!("[user] GPF (code={:#x}) -> killing process; kernel survives", error_code);
            vga::println!("[user] GPF -> process killed (others survive)");
            crate::kthread::kill_current_and_park();
        }
        vga::println!("[user] GPF (code={:#x}) -> process killed", error_code);
        terminate_user_and_restart("USER_GPF", 0x100 + 13)
    }

    vga::println!("EXCEPTION: GPF code={:#x} rip={:#x}", error_code, rip);
    kernel_halt("general protection fault");
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let addr = Cr2::read();
    let rip = stack_frame.instruction_pointer.as_u64();
    serial::println!(
        "EXCEPTION: PAGE FAULT (addr={:?}, err={:?}, rip={:#x})\n{:#?}",
        addr,
        error_code,
        rip,
        stack_frame
    );

    if is_user_mode(&stack_frame) {
        // Preemptive path: kill only this process, leave the kernel and other processes alive.
        if crate::kthread::is_user_current() {
            serial::println!("[user] PAGE FAULT @ {:?} -> killing process; kernel survives", addr);
            vga::println!("[user] page fault -> process killed (others survive)");
            crate::kthread::kill_current_and_park();
        }
        vga::println!("[user] PAGE FAULT @ {:?} -> process killed", addr);
        // Conventional-ish exit code: 0x100 + 14 (page fault vector)
        terminate_user_and_restart("USER_PF", 0x100 + 14)
    }

    vga::println!("EXCEPTION: PAGE FAULT @ {:?}", addr);
    vga::println!("  rip={:#x}  cause={:?}", rip, error_code);
    kernel_halt("page fault");
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial::println!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
    vga::println!("EXCEPTION: DOUBLE FAULT rip={:#x}", stack_frame.instruction_pointer.as_u64());
    kernel_halt("double fault");
}

/// C callback for the preemptive timer ISR (`asm_timer_isr`). Runs with interrupts disabled.
/// `saved_rsp` points at the saved register block of the interrupted thread; the return value
/// is the stack pointer to resume on (same thread, or another when preemption is armed).
#[no_mangle]
pub extern "C" fn timer_dispatch(saved_rsp: u64) -> u64 {
    TICKS.fetch_add(1, Ordering::Relaxed);
    crate::syscall::on_timer_tick();

    // Acknowledge the interrupt before any potential context switch. Once the tick is delivered
    // by the local-APIC timer, EOI goes to the APIC; until then it goes to the 8259 PIC.
    if APIC_TIMER.load(Ordering::SeqCst) {
        crate::apic::eoi();
    } else {
        unsafe {
            PICS.lock().notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
        }
    }

    if crate::kthread::armed() {
        crate::kthread::schedule(saved_rsp)
    } else {
        saved_rsp
    }
}

// DISABLED: Complex naked asm handler for debugging offset error
// (Context switching disabled for now to fix LLVM bug)

#[allow(dead_code)]
fn _timer_interrupt_handler_rust_unused(_saved_rsp: u64) -> u64 {
    TICKS.fetch_add(1, Ordering::Relaxed);

    crate::syscall::on_timer_tick();

    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }

    0 // No context switch in simplified version
}

extern "x86-interrupt" fn keyboard_interrupt_handler(stack_frame: InterruptStackFrame) {
    let scancode = unsafe { arch::inb(0x60) };

    // Minimal SIGINT-like behavior: Ctrl+C while in Ring3 aborts the foreground user process.
    match scancode {
        0x1D => {
            // Left Ctrl pressed
            CTRL_DOWN.store(true, Ordering::Relaxed);
        }
        0x9D => {
            // Left Ctrl released
            CTRL_DOWN.store(false, Ordering::Relaxed);
        }
        0x2E => {
            // 'C' pressed
            if CTRL_DOWN.load(Ordering::Relaxed) {
                // A foreground scheduled process can be interrupted even if we're currently
                // running the shell (kernel mode).
                eoi_external(InterruptIndex::Keyboard.as_u8());

                let fg = crate::tasking::foreground_pid();
                if fg != 0 {
                    crate::tasking::kill_with_exit(fg, 0x100 + 2);
                    return;
                }

                // Legacy: Ctrl+C only affects Ring3 when running outside the scheduler.
                if is_user_mode(&stack_frame) {
                    if crate::tasking::current_pid() != 0 {
                        crate::tasking::request_kill_current(0x100 + 2);
                        return;
                    }
                    terminate_user_and_restart("SIGINT", 0x100 + 2)
                }
            }
        }
        _ => {}
    }

    keyboard::push_scancode(scancode);

    eoi_external(InterruptIndex::Keyboard.as_u8());
}

extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    crate::mouse::handle_interrupt();

    eoi_external(InterruptIndex::Mouse.as_u8());
}

/// xHCI MSI-X interrupt (WP-04 step 5). The controller acknowledges itself inside `on_interrupt`
/// (clears EINT/IP and drains the event ring); the message was delivered by the local APIC, so the
/// end-of-interrupt always goes to the local APIC.
extern "x86-interrupt" fn xhci_interrupt_handler(_stack_frame: InterruptStackFrame) {
    crate::xhci::on_interrupt();
    crate::apic::eoi();
}
