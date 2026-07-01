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
        }
        None => crate::serial_println!("[APIC] MADT not found"),
    }

    AcpiInfo { madt }
}
