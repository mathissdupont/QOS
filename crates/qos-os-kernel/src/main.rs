#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

mod arch;
mod asm_stubs;    // Low-level asm (global_asm!, no external cc required)
mod framebuffer;  // VESA framebuffer graphics
mod gdt;
mod interrupts;
mod input;       // Unified input event queue — Phase 0.1 (docs/PLAN.md)
mod kthread;     // Preemptive kernel threads — Phase 2.1 (docs/PLAN.md)
mod keyboard;
mod fs;
mod memory;
mod pit;
mod process;
mod quantum;
mod qviz;        // Quantum visualization
mod runtime;
mod scheduler;
mod qemu;
mod serial;
mod shell;
mod splash;     // Boot splash screen
mod syscall;
mod tasking;
mod user; // ✅ ENABLED: LLVM bug fixed with Docker Linux environment
mod ui;
mod vga;
mod vga13h;     // VGA Mode 13h (320x200x256) pixel graphics — ADR-0013 Phase 1

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
mod e1000;      // Intel E1000 NIC driver
mod http;       // HTTP/HTTPS client
mod gui;        // Window manager & GUI (text mode)
mod gfxui;      // Interactive graphical desktop (Mode 13h) — Phase 1 (docs/PLAN.md)
mod timer;      // Timer utilities
mod mouse;      // PS/2 Mouse driver
mod menu;       // Text-mode menu system
mod dialog;     // Dialog boxes
mod explorer;   // File explorer
mod desktop;    // Windows-style desktop environment
mod desktop_apps; // Desktop applications

use bootloader::{entry_point, BootInfo};

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    serial::init();

    serial::println!("QOS-OS boot OK (serial)");

    // Initialize framebuffer (placeholder for bootloader 0.9.x)
    framebuffer::init();

    // Show boot splash screen (skip delay in verify mode)
    splash::show_splash();
    splash::show_progress("Initializing memory...");

    let phys_mem_offset = x86_64::VirtAddr::new(boot_info.physical_memory_offset);
    unsafe { memory::init_global(phys_mem_offset, &boot_info.memory_map) };
    memory::with_ctx(|mapper, frame_allocator| {
        allocator::init_heap(mapper, frame_allocator).expect("heap init failed")
    });
    crate::serial_println!("heap initialized");

    let heap_test = alloc::boxed::Box::new(0xC0FF_EEu64);
    crate::serial_println!("heap test ok: 0x{:x}", *heap_test);

    splash::show_progress("Initializing CPU/GDT/IDT...");
    gdt::init();
    interrupts::init_idt();
    interrupts::init_pics();

    splash::show_progress("Initializing PIT timer...");
    pit::init_timer(100);
    mouse::init();        // PS/2 Mouse with scroll wheel

    // Initialize production-level subsystems
    splash::show_progress("Detecting hardware...");
    rtc::init();          // Real-Time Clock
    pci::init();          // PCI bus enumeration
    // acpi::init();      // ACPI - disabled (needs low memory mapping)
    // ahci::init();      // SATA/AHCI - disabled (needs ACPI)
    syscall_ext::init();  // Extended syscalls
    
    // Initialize network
    splash::show_progress("Initializing network...");
    if let Err(e) = e1000::init() {
        serial_println!("[NET] E1000 init skipped: {}", e);
    }
    
    gui::init();          // Window manager
    
    // Initialize menu system
    let menu_bar = menu::create_default_menu();
    menu::init(menu_bar);
    
    // Show system info
    let datetime = rtc::read_datetime();
    serial_println!("System Time: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        datetime.year, datetime.month, datetime.day,
        datetime.hour, datetime.minute, datetime.second);

    splash::show_progress("System ready!");

    // Wait for user to dismiss splash (skip in verify mode)
    splash::wait_for_continue();

    #[cfg(feature = "userdemo")]
    {
        // Milestone 2 demo: enter Ring 3 and trigger `int 0x80` from user mode.
        // User module disabled due to LLVM asm bug
        serial_println!("userdemo: user mode disabled (LLVM asm bug)");
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
