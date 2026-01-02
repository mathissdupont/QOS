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

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
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

        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);

        // Syscall entry point (Milestone 2).
        idt[0x80]
            .set_handler_fn(syscall::syscall_interrupt_handler)
            .set_privilege_level(PrivilegeLevel::Ring3);

        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

pub fn init_pics() {
    unsafe { PICS.lock().initialize() };

    // Deterministic IRQ masks:
    // - Unmask IRQ0 (timer) + IRQ1 (keyboard) on master PIC.
    // - Keep all IRQs masked on slave PIC.
    unsafe {
        arch::outb(0x21, 0b1111_1100);
        arch::outb(0xA1, 0b1111_1111);
    }

    arch::enable_interrupts();
}

fn is_user_mode(stack_frame: &InterruptStackFrame) -> bool {
    stack_frame.code_segment.rpl() == PrivilegeLevel::Ring3
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
        terminate_user_and_restart("USER_UD", 0x100 + 6)
    }

    loop { arch::hlt(); }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    serial::println!("EXCEPTION: GENERAL PROTECTION FAULT (code={:#x})\n{:#?}", error_code, stack_frame);
    vga::println!("EXCEPTION: GPF");

    if is_user_mode(&stack_frame) {
        terminate_user_and_restart("USER_GPF", 0x100 + 13)
    }

    loop { arch::hlt(); }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let addr = Cr2::read();
    serial::println!(
        "EXCEPTION: PAGE FAULT (addr={:?}, err={:?})\n{:#?}",
        addr,
        error_code,
        stack_frame
    );
    vga::println!("EXCEPTION: PAGE FAULT");

    if is_user_mode(&stack_frame) {
        // Conventional-ish exit code: 0x100 + 14 (page fault vector)
        terminate_user_and_restart("USER_PF", 0x100 + 14)
    }

    loop { arch::hlt(); }
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial::println!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
    vga::println!("EXCEPTION: DOUBLE FAULT");

    loop {
        arch::hlt();
    }
}

// Simple timer interrupt handler without naked_asm complexity
extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // Update tick counter
    TICKS.fetch_add(1, Ordering::Relaxed);
    
    // Notify syscall module
    crate::syscall::on_timer_tick();
    
    // Send EOI
    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
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
                unsafe {
                    PICS.lock()
                        .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
                }

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

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}
