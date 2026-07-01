//! APIC bring-up — discovery slice (epic E-10, ADR-0015).
//!
//! UEFI-correct ACPI discovery: the bootloader hands us the RSDP physical address
//! (`boot_info.rsdp_addr`), so we do **not** scan BIOS memory (that legacy path in `acpi.rs`
//! does not work under UEFI). We walk RSDP → XSDT/RSDT → the MADT and parse it with the
//! host-tested `qos_acpi` crate, then log the APIC topology (local-APIC address, usable CPU
//! count, IO-APIC, legacy-IRQ→GSI remaps).
//!
//! This slice is **read-only**: it discovers and reports. Enabling the local APIC + IO-APIC and
//! moving the timer tick off the PIT are the next slices; they build on the addresses found here.

use alloc::vec::Vec;

use qos_acpi::{Madt, SdtHeader};

/// Read `len` bytes of physical memory through the bootloader's physical-memory offset mapping
/// (all physical RAM, including ACPI regions, is mapped there under `Mapping::Dynamic`).
fn phys_read(phys: u64, len: usize) -> Vec<u8> {
    let virt = (crate::memory::phys_offset().as_u64() + phys) as *const u8;
    let mut out = Vec::with_capacity(len);
    unsafe {
        let slice = core::slice::from_raw_parts(virt, len);
        out.extend_from_slice(slice);
    }
    out
}

/// Read an SDT's full bytes given its physical address: read the 36-byte header first to learn
/// the length, then read the whole table.
fn read_sdt(phys: u64) -> Option<Vec<u8>> {
    let head = phys_read(phys, qos_acpi::SDT_HEADER_LEN);
    let header = SdtHeader::parse(&head)?;
    let len = header.length as usize;
    if len < qos_acpi::SDT_HEADER_LEN || len > 0x10_0000 {
        return None; // implausible length — refuse rather than read garbage
    }
    Some(phys_read(phys, len))
}

// ── Local APIC (E-10 slices 2–3) ─────────────────────────────────────────────────────────────
// Register offsets within the local-APIC MMIO page.
mod lapic_reg {
    pub const ID: u64 = 0x20;
    pub const VERSION: u64 = 0x30;
    pub const TPR: u64 = 0x80; // Task Priority Register
    pub const EOI: u64 = 0xB0;
    pub const SVR: u64 = 0xF0; // Spurious Interrupt Vector Register
    pub const LVT_TIMER: u64 = 0x320;
    pub const TIMER_INIT_COUNT: u64 = 0x380;
    pub const TIMER_CUR_COUNT: u64 = 0x390;
    pub const TIMER_DIVIDE: u64 = 0x3E0;
}

/// IA32_APIC_BASE MSR: bit 11 = APIC global enable, bit 8 = BSP.
const IA32_APIC_BASE: u32 = 0x1B;

/// Local-APIC timer divide-configuration value for "divide by 16" (bits [3,1,0] = 0b011).
const TIMER_DIV_16: u32 = 0b0011;
/// LVT Timer periodic mode = bits 18:17 == 0b01.
const LVT_TIMER_PERIODIC: u32 = 1 << 17;
/// LVT mask bit.
const LVT_MASKED: u32 = 1 << 16;

/// Cached local-APIC MMIO virtual base (0 until enabled), so `eoi()` needs no argument from the
/// interrupt path.
static LOCAL_APIC_BASE_VIRT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

fn lapic_read(base_virt: u64, reg: u64) -> u32 {
    unsafe { core::ptr::read_volatile((base_virt + reg) as *const u32) }
}
fn lapic_write(base_virt: u64, reg: u64, val: u32) {
    unsafe { core::ptr::write_volatile((base_virt + reg) as *mut u32, val) }
}

pub fn is_enabled() -> bool {
    LOCAL_APIC_BASE_VIRT.load(core::sync::atomic::Ordering::Relaxed) != 0
}

/// Software-enable the local APIC (E-10 slice 2). Does **not** reroute interrupts: the 8259 PIC
/// keeps delivering timer/keyboard/mouse. It brings the local APIC online (global-enable via
/// IA32_APIC_BASE, software-enable + spurious vector via the SVR) and drops TPR so it accepts
/// interrupts — groundwork for the APIC-timer slice.
pub fn enable_local_apic(local_apic_phys: u64) {
    unsafe {
        let mut msr = x86_64::registers::model_specific::Msr::new(IA32_APIC_BASE);
        let val = msr.read();
        msr.write(val | (1 << 11));
    }
    let base = crate::memory::mmio_virt_addr(local_apic_phys).as_u64();
    LOCAL_APIC_BASE_VIRT.store(base, core::sync::atomic::Ordering::Relaxed);
    let id = lapic_read(base, lapic_reg::ID) >> 24;
    let version = lapic_read(base, lapic_reg::VERSION) & 0xFF;
    lapic_write(base, lapic_reg::TPR, 0);
    lapic_write(base, lapic_reg::SVR, 0x100 | crate::interrupts::SPURIOUS_VECTOR as u32);
    crate::serial_println!(
        "[APIC] local APIC enabled: id={} version={:#x} svr={:#x}",
        id, version, lapic_read(base, lapic_reg::SVR)
    );
}

/// Signal end-of-interrupt to the local APIC (for APIC-delivered interrupts, e.g. the APIC timer).
pub fn eoi() {
    let base = LOCAL_APIC_BASE_VIRT.load(core::sync::atomic::Ordering::Relaxed);
    if base != 0 {
        lapic_write(base, lapic_reg::EOI, 0);
    }
}

/// Replace the PIT scheduler tick with the local-APIC timer at ~100 Hz (E-10 slice 3). The timer
/// is internal to the local APIC, so this needs **no** IO-APIC — external IRQs (keyboard/mouse)
/// stay on the PIC until the IO-APIC slice. Steps: calibrate the APIC timer frequency against a
/// PIT channel-2 busy-wait, mask the PIT IRQ0, switch `timer_dispatch` to EOI the APIC, then start
/// the periodic APIC timer on its own vector. Self-tests that ticks advance before returning.
pub fn start_apic_timer_100hz() {
    let base = LOCAL_APIC_BASE_VIRT.load(core::sync::atomic::Ordering::Relaxed);
    if base == 0 {
        crate::serial_println!("[APIC] timer: local APIC not enabled, staying on PIT");
        return;
    }

    // Calibrate: run the timer masked at max count for 10 ms and see how far it counted down.
    lapic_write(base, lapic_reg::TIMER_DIVIDE, TIMER_DIV_16);
    lapic_write(base, lapic_reg::LVT_TIMER, LVT_MASKED | crate::interrupts::APIC_TIMER_VECTOR as u32);
    lapic_write(base, lapic_reg::TIMER_INIT_COUNT, 0xFFFF_FFFF);
    crate::pit::busy_wait_us(10_000); // 10 ms
    let elapsed = 0xFFFF_FFFFu32.wrapping_sub(lapic_read(base, lapic_reg::TIMER_CUR_COUNT));
    lapic_write(base, lapic_reg::TIMER_INIT_COUNT, 0); // stop
    let ticks_per_10ms = elapsed.max(1); // 10 ms period → 100 Hz
    crate::serial_println!(
        "[APIC] timer calibrated: {} ticks/10ms (~{} MHz at div16)",
        ticks_per_10ms,
        (ticks_per_10ms as u64 * 100 * 16) / 1_000_000
    );

    // Mask PIT IRQ0 on the master PIC so channel 0 no longer delivers the tick.
    unsafe {
        let mask = crate::arch::inb(0x21);
        crate::arch::outb(0x21, mask | 0x01);
    }
    // From now on the tick is APIC-delivered: `timer_dispatch` must EOI the local APIC.
    crate::interrupts::APIC_TIMER.store(true, core::sync::atomic::Ordering::SeqCst);

    // Start the periodic APIC timer on its own vector (IDT entry installed at build time).
    lapic_write(base, lapic_reg::TIMER_DIVIDE, TIMER_DIV_16);
    lapic_write(
        base,
        lapic_reg::LVT_TIMER,
        LVT_TIMER_PERIODIC | crate::interrupts::APIC_TIMER_VECTOR as u32,
    );
    lapic_write(base, lapic_reg::TIMER_INIT_COUNT, ticks_per_10ms);

    // Self-test: with the PIT masked, ticks can only advance if the APIC timer is firing.
    use core::sync::atomic::Ordering::Relaxed;
    let before = crate::interrupts::TICKS.load(Relaxed);
    for _ in 0..10 {
        crate::pit::busy_wait_us(10_000); // ~100 ms total
    }
    let advanced = crate::interrupts::TICKS.load(Relaxed).wrapping_sub(before);
    crate::serial_println!("[APIC] timer active: {} ticks in ~100ms (PIT masked)", advanced);
}

/// Result of discovery, cached for later slices (APIC enable, IRQ routing, SMP).
#[derive(Clone, Debug, Default)]
pub struct AcpiInfo {
    pub madt: Option<Madt>,
}

/// Discover ACPI tables from the bootloader-provided RSDP and log the APIC topology.
/// Returns the parsed info for later APIC/SMP slices. Never faults on malformed tables.
pub fn init(rsdp_phys: u64) -> AcpiInfo {
    // RSDP: signature[8], checksum u8, oem[6], revision u8, rsdt_address u32, [len u32,
    // xsdt_address u64, ...] for revision >= 2.
    let rsdp = phys_read(rsdp_phys, 36);
    if rsdp.len() < 20 || &rsdp[0..8] != b"RSD PTR " {
        crate::serial_println!("[APIC] RSDP signature invalid at {:#x}", rsdp_phys);
        return AcpiInfo::default();
    }
    let revision = rsdp[15];
    let rsdt_addr = u32::from_le_bytes([rsdp[16], rsdp[17], rsdp[18], rsdp[19]]) as u64;
    let xsdt_addr = if revision >= 2 && rsdp.len() >= 32 {
        u64::from_le_bytes([
            rsdp[24], rsdp[25], rsdp[26], rsdp[27], rsdp[28], rsdp[29], rsdp[30], rsdp[31],
        ])
    } else {
        0
    };
    crate::serial_println!(
        "[APIC] RSDP @ {:#x} rev {} rsdt={:#x} xsdt={:#x}",
        rsdp_phys, revision, rsdt_addr, xsdt_addr
    );

    // Walk the XSDT (64-bit entries) if present, else the RSDT (32-bit entries), to find "APIC".
    let (root_phys, entry_size) = if xsdt_addr != 0 { (xsdt_addr, 8usize) } else { (rsdt_addr, 4) };
    let root = match read_sdt(root_phys) {
        Some(b) => b,
        None => {
            crate::serial_println!("[APIC] root SDT unreadable");
            return AcpiInfo::default();
        }
    };
    let root_len = root.len();
    let mut madt: Option<Madt> = None;
    let mut off = qos_acpi::SDT_HEADER_LEN;
    while off + entry_size <= root_len {
        let entry_phys = if entry_size == 8 {
            u64::from_le_bytes([
                root[off], root[off + 1], root[off + 2], root[off + 3],
                root[off + 4], root[off + 5], root[off + 6], root[off + 7],
            ])
        } else {
            u32::from_le_bytes([root[off], root[off + 1], root[off + 2], root[off + 3]]) as u64
        };
        off += entry_size;
        // Peek the signature cheaply before reading the whole table.
        let sig = phys_read(entry_phys, 4);
        if sig == b"APIC" {
            if let Some(bytes) = read_sdt(entry_phys) {
                madt = qos_acpi::parse_madt(&bytes);
            }
            break;
        }
    }

    match &madt {
        Some(m) => {
            crate::serial_println!(
                "[APIC] MADT: local_apic={:#x} cpus(enabled)={} entries={}",
                m.local_apic_address,
                m.enabled_cpu_count(),
                m.entries.len()
            );
            if let Some((id, addr, gsi)) = m.first_io_apic() {
                crate::serial_println!("[APIC] IO-APIC id={} addr={:#x} gsi_base={}", id, addr, gsi);
            }
            crate::serial_println!(
                "[APIC] legacy IRQ routing: IRQ0->GSI{}, IRQ1->GSI{}",
                m.irq_to_gsi(0),
                m.irq_to_gsi(1)
            );
            // Slice 2: bring the local APIC online (does not reroute IRQs; PIC still drives them).
            enable_local_apic(m.local_apic_address);
            // Slice 3: move the scheduler tick from the PIT to the local-APIC timer.
            start_apic_timer_100hz();
        }
        None => crate::serial_println!("[APIC] MADT not found"),
    }

    AcpiInfo { madt }
}
