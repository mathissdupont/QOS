#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

mod arch;
mod gdt;
mod interrupts;
mod keyboard;
mod fs;
mod memory;
mod pit;
mod process;
mod quantum;
mod runtime;
mod scheduler;
mod qemu;
mod serial;
mod shell;
mod syscall;
mod tasking;
// mod user; // Disabled due to LLVM asm bug "offset is not a multiple of 16"
mod ui;
mod vga;

mod allocator;
mod ata;
mod diskfs;
mod vfs;
mod elf;

// Production-level modules
mod rtc;        // Real-Time Clock
mod pci;        // PCI bus enumeration
mod acpi;       // ACPI power management
mod fat16;      // FAT16 file system
mod ahci;       // SATA AHCI driver
mod syscall_ext;// Extended syscalls (open/read/write/close)
mod net;        // Network stack
mod gui;        // Window manager & GUI
mod timer;      // Timer utilities
mod mouse;      // PS/2 Mouse driver

use bootloader::{entry_point, BootInfo};

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    serial::init();

    serial::println!("QaOS boot OK (serial)");

    vga::clear_screen();
    vga::println!("QaOS - Quantum Operating System v0.1.0");
    vga::println!("Initializing kernel...");

    let phys_mem_offset = x86_64::VirtAddr::new(boot_info.physical_memory_offset);
    unsafe { memory::init_global(phys_mem_offset, &boot_info.memory_map) };
    memory::with_ctx(|mapper, frame_allocator| {
        allocator::init_heap(mapper, frame_allocator).expect("heap init failed")
    });
    crate::serial_println!("heap initialized");

    let heap_test = alloc::boxed::Box::new(0xC0FF_EEu64);
    crate::serial_println!("heap test ok: 0x{:x}", *heap_test);
    vga::println!("[OK] Memory & heap initialized");

    gdt::init();
    interrupts::init_idt();
    interrupts::init_pics();
    pit::init_timer(100);
    mouse::init();        // PS/2 Mouse with scroll wheel

    // Initialize production-level subsystems
    vga::println!("[..] Detecting hardware...");
    rtc::init();          // Real-Time Clock
    pci::init();          // PCI bus enumeration
    // acpi::init();      // ACPI - disabled (needs low memory mapping)
    // ahci::init();      // SATA/AHCI - disabled (needs ACPI)
    syscall_ext::init();  // Extended syscalls
    // net::init();       // Network stack - disabled (needs working hw)
    gui::init();          // Window manager
    
    // Show system info
    let datetime = rtc::read_datetime();
    serial_println!("System Time: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        datetime.year, datetime.month, datetime.day,
        datetime.hour, datetime.minute, datetime.second);
    vga::println!("[OK] Hardware initialized - {}", rtc::time_string());
    vga::println!("");

    #[cfg(feature = "userdemo")]
    {
        // Milestone 2 demo: enter Ring 3 and trigger `int 0x80` from user mode.
        // User module disabled due to LLVM asm bug
        vga::println!("userdemo: user mode disabled (LLVM asm bug)");
    }

    #[cfg(all(not(feature = "userdemo"), not(feature = "userabi")))]
    {
        // Syscall gate is installed
    }

    crate::serial_println!("QaOS ready. Type on keyboard...");

    #[cfg(feature = "verify")]
    {
        crate::serial_println!("VERIFY: exiting QEMU");
        qemu::exit(0x10);
    }

    runtime::run_kernel_loop();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial::println!("PANIC: {}", info);
    vga::println!("PANIC: {}", info);

    loop {
        arch::hlt();
    }
}
