//! ACPI Power Management for QOS
//!
//! Basic ACPI support for shutdown, reboot, and power state management.
//! Searches for RSDP and parses FADT to find shutdown/reboot ports.

use crate::arch;

/// ACPI RSDP signature "RSD PTR "
const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";

/// RSDP (Root System Description Pointer) structure
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

/// RSDP Extended (ACPI 2.0+)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct RsdpExtended {
    base: Rsdp,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    reserved: [u8; 3],
}

/// ACPI SDT Header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct AcpiSdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

/// FADT (Fixed ACPI Description Table) partial structure
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct Fadt {
    header: AcpiSdtHeader,
    firmware_ctrl: u32,
    dsdt: u32,
    reserved: u8,
    preferred_pm_profile: u8,
    sci_int: u16,
    smi_cmd: u32,
    acpi_enable: u8,
    acpi_disable: u8,
    s4bios_req: u8,
    pstate_cnt: u8,
    pm1a_evt_blk: u32,
    pm1b_evt_blk: u32,
    pm1a_cnt_blk: u32,
    pm1b_cnt_blk: u32,
    pm2_cnt_blk: u32,
    pm_tmr_blk: u32,
    gpe0_blk: u32,
    gpe1_blk: u32,
    pm1_evt_len: u8,
    pm1_cnt_len: u8,
    pm2_cnt_len: u8,
    pm_tmr_len: u8,
    gpe0_blk_len: u8,
    gpe1_blk_len: u8,
    gpe1_base: u8,
    cst_cnt: u8,
    p_lvl2_lat: u16,
    p_lvl3_lat: u16,
    flush_size: u16,
    flush_stride: u16,
    duty_offset: u8,
    duty_width: u8,
    day_alrm: u8,
    mon_alrm: u8,
    century: u8,
    iapc_boot_arch: u16,
    reserved2: u8,
    flags: u32,
    reset_reg: [u8; 12], // Generic Address Structure
    reset_value: u8,
}

/// ACPI power state
static mut ACPI_STATE: AcpiState = AcpiState::new();

/// ACPI state holder
struct AcpiState {
    initialized: bool,
    pm1a_cnt: u16,
    pm1b_cnt: u16,
    slp_typa: u16,
    slp_typb: u16,
    reset_reg: u16,
    reset_value: u8,
    smi_cmd: u16,
    acpi_enable: u8,
}

impl AcpiState {
    const fn new() -> Self {
        Self {
            initialized: false,
            pm1a_cnt: 0,
            pm1b_cnt: 0,
            slp_typa: 0,
            slp_typb: 0,
            reset_reg: 0,
            reset_value: 0,
            smi_cmd: 0,
            acpi_enable: 0,
        }
    }
}

/// Calculate ACPI checksum
fn checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}

/// Search for RSDP in memory region
unsafe fn find_rsdp_in_region(start: usize, end: usize) -> Option<*const Rsdp> {
    let mut addr = start;
    while addr < end {
        let ptr = addr as *const u8;
        let signature = core::slice::from_raw_parts(ptr, 8);
        if signature == RSDP_SIGNATURE {
            // Verify checksum
            let rsdp_bytes = core::slice::from_raw_parts(ptr, core::mem::size_of::<Rsdp>());
            if checksum(rsdp_bytes) == 0 {
                return Some(ptr as *const Rsdp);
            }
        }
        addr += 16; // RSDP is always 16-byte aligned
    }
    None
}

/// Find RSDP
unsafe fn find_rsdp() -> Option<*const Rsdp> {
    // Search EBDA (Extended BIOS Data Area)
    // EBDA pointer is at 0x40E
    let ebda_ptr = *(0x40E as *const u16) as usize;
    let ebda_start = ebda_ptr << 4;
    if ebda_start > 0 && ebda_start < 0xA0000 {
        if let Some(rsdp) = find_rsdp_in_region(ebda_start, ebda_start + 1024) {
            return Some(rsdp);
        }
    }
    
    // Search BIOS ROM area (0xE0000 - 0xFFFFF)
    find_rsdp_in_region(0xE0000, 0x100000)
}

/// Find a table in RSDT
unsafe fn find_table(rsdt: *const u8, signature: &[u8; 4]) -> Option<*const AcpiSdtHeader> {
    let header = &*(rsdt as *const AcpiSdtHeader);
    let entries = (header.length as usize - core::mem::size_of::<AcpiSdtHeader>()) / 4;
    let entries_ptr = rsdt.add(core::mem::size_of::<AcpiSdtHeader>()) as *const u32;
    
    for i in 0..entries {
        let table_addr = *entries_ptr.add(i) as *const AcpiSdtHeader;
        if table_addr.is_null() {
            continue;
        }
        
        let table = &*table_addr;
        if &table.signature == signature {
            return Some(table_addr);
        }
    }
    None
}

/// Initialize ACPI subsystem
pub fn init() {
    unsafe {
        let rsdp = match find_rsdp() {
            Some(r) => r,
            None => {
                crate::serial_println!("[ACPI] RSDP not found");
                return;
            }
        };
        
        let rsdp = &*rsdp;
        crate::serial_println!("[ACPI] RSDP found at {:p}", rsdp as *const _);
        
        // Get RSDT
        let rsdt = rsdp.rsdt_address as *const u8;
        if rsdt.is_null() {
            crate::serial_println!("[ACPI] RSDT is null");
            return;
        }
        
        // Find FADT
        let fadt_ptr = match find_table(rsdt, b"FACP") {
            Some(f) => f as *const Fadt,
            None => {
                crate::serial_println!("[ACPI] FADT not found");
                return;
            }
        };
        
        let fadt = &*fadt_ptr;
        
        // Store ACPI information
        ACPI_STATE.pm1a_cnt = fadt.pm1a_cnt_blk as u16;
        ACPI_STATE.pm1b_cnt = fadt.pm1b_cnt_blk as u16;
        ACPI_STATE.smi_cmd = fadt.smi_cmd as u16;
        ACPI_STATE.acpi_enable = fadt.acpi_enable;
        
        // Parse reset register if available
        if fadt.header.length >= 129 {
            // Reset register is at offset 116-127 (Generic Address Structure)
            let reset_type = fadt.reset_reg[0];
            if reset_type == 1 {
                // System I/O space
                let addr = u16::from_le_bytes([fadt.reset_reg[4], fadt.reset_reg[5]]);
                ACPI_STATE.reset_reg = addr;
                ACPI_STATE.reset_value = fadt.reset_value;
            }
        }
        
        // Try to get SLP_TYPa from DSDT (simplified - use default S5 values)
        // In real implementation, we would parse the DSDT/AML
        ACPI_STATE.slp_typa = 0x2000; // SLP_TYPa = 5, SLP_EN = 1
        ACPI_STATE.slp_typb = 0x2000;
        
        ACPI_STATE.initialized = true;
        
        crate::serial_println!("[ACPI] Initialized - PM1A_CNT: {:04X}", ACPI_STATE.pm1a_cnt);
    }
}

/// Check if ACPI is available
pub fn is_available() -> bool {
    unsafe { ACPI_STATE.initialized }
}

/// Enable ACPI mode (if not already enabled)
pub fn enable() {
    unsafe {
        if !ACPI_STATE.initialized || ACPI_STATE.smi_cmd == 0 {
            return;
        }
        
        // Send ACPI enable command to SMI port
        arch::outb(ACPI_STATE.smi_cmd, ACPI_STATE.acpi_enable);
        
        // Wait for ACPI to be enabled
        for _ in 0..1000 {
            let status = arch::inw(ACPI_STATE.pm1a_cnt);
            if status & 1 != 0 {
                break;
            }
        }
    }
}

/// Shutdown the system using ACPI S5 state
pub fn shutdown() -> ! {
    crate::serial_println!("[ACPI] Initiating shutdown...");
    
    unsafe {
        if !ACPI_STATE.initialized {
            // Fallback: Try QEMU shutdown port
            crate::serial_println!("[ACPI] Using QEMU shutdown...");
            arch::outw(0x604, 0x2000);
            
            // Try Bochs/older QEMU
            arch::outw(0xB004, 0x2000);
            
            // Try VirtualBox
            arch::outw(0x4004, 0x3400);
        } else {
            // Write to PM1a_CNT to enter S5 (shutdown)
            if ACPI_STATE.pm1a_cnt != 0 {
                arch::outw(ACPI_STATE.pm1a_cnt, ACPI_STATE.slp_typa | 0x2000);
            }
            if ACPI_STATE.pm1b_cnt != 0 {
                arch::outw(ACPI_STATE.pm1b_cnt, ACPI_STATE.slp_typb | 0x2000);
            }
        }
    }
    
    // If shutdown didn't work, halt
    crate::serial_println!("[ACPI] Shutdown failed, halting...");
    loop {
        x86_64::instructions::hlt();
    }
}

/// Reboot the system
pub fn reboot() -> ! {
    crate::serial_println!("[ACPI] Initiating reboot...");
    
    unsafe {
        // Method 1: ACPI reset register
        if ACPI_STATE.initialized && ACPI_STATE.reset_reg != 0 {
            arch::outb(ACPI_STATE.reset_reg, ACPI_STATE.reset_value);
            for _ in 0..1000000 { core::hint::spin_loop(); }
        }
        
        // Method 2: 8042 keyboard controller reset
        // Wait for keyboard controller
        for _ in 0..10000 {
            if arch::inb(0x64) & 2 == 0 {
                break;
            }
        }
        arch::outb(0x64, 0xFE); // Reset CPU command
        
        for _ in 0..1000000 { core::hint::spin_loop(); }
        
        // Method 3: Just hang if above methods failed
        crate::serial_println!("[ACPI] Reset methods failed, halting...");
        loop {
            arch::hlt();
        }
    }
}

/// Get power state information
pub fn power_info() {
    unsafe {
        if !ACPI_STATE.initialized {
            crate::println!("ACPI: Not initialized");
            crate::println!("Fallback: QEMU/Bochs ports available");
        } else {
            crate::println!("ACPI Status:");
            crate::println!("  PM1A_CNT: 0x{:04X}", ACPI_STATE.pm1a_cnt);
            crate::println!("  PM1B_CNT: 0x{:04X}", ACPI_STATE.pm1b_cnt);
            crate::println!("  Reset Reg: 0x{:04X}", ACPI_STATE.reset_reg);
            crate::println!("  Reset Value: 0x{:02X}", ACPI_STATE.reset_value);
        }
    }
}
