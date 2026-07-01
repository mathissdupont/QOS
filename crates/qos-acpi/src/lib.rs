//! # qos-acpi
//!
//! Portable ACPI table parsing for QOS (epic E-10, ADR-0015). ACPI tables are just bytes in
//! physical memory; this crate parses them as byte slices, so the fiddly offset/length logic is
//! **host-tested**. The kernel is responsible for locating the tables (RSDP → XSDT) and mapping
//! their physical addresses, then hands the bytes here.
//!
//! First target: the **MADT** (a.k.a. APIC table) — it describes the local APIC address, the
//! processor-local APICs (→ CPU count for SMP), the IO-APIC(s) (→ interrupt routing), and the
//! interrupt source overrides (legacy IRQ → GSI remaps). That is exactly what E-10 needs to bring
//! up the APIC and reroute the timer/keyboard IRQs.
//!
//! `no_std` when built into the kernel; uses `alloc`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;

/// Length of the standard ACPI System Description Table header.
pub const SDT_HEADER_LEN: usize = 36;

fn read_u16(b: &[u8], off: usize) -> u16 {
    if off + 2 <= b.len() {
        u16::from_le_bytes([b[off], b[off + 1]])
    } else {
        0
    }
}

fn read_u32(b: &[u8], off: usize) -> u32 {
    if off + 4 <= b.len() {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    } else {
        0
    }
}

fn read_u64(b: &[u8], off: usize) -> u64 {
    if off + 8 <= b.len() {
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[off..off + 8]);
        u64::from_le_bytes(a)
    } else {
        0
    }
}

/// The parsed header common to every ACPI SDT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
}

impl SdtHeader {
    /// Parse the 36-byte header from the start of a table, if enough bytes are present.
    pub fn parse(b: &[u8]) -> Option<SdtHeader> {
        if b.len() < SDT_HEADER_LEN {
            return None;
        }
        Some(SdtHeader {
            signature: [b[0], b[1], b[2], b[3]],
            length: read_u32(b, 4),
            revision: b[8],
        })
    }

    pub fn has_signature(&self, sig: &[u8; 4]) -> bool {
        &self.signature == sig
    }
}

/// Sum of all bytes must be 0 for a valid ACPI table/RSDP (mod 256).
pub fn checksum_ok(b: &[u8]) -> bool {
    b.iter().fold(0u8, |a, &x| a.wrapping_add(x)) == 0
}

/// One entry of interest from the MADT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadtEntry {
    /// A processor's local APIC. `enabled` (flags bit0) tells whether the CPU is usable.
    LocalApic { acpi_processor_id: u8, apic_id: u8, enabled: bool },
    /// An IO-APIC: its id, MMIO address, and the global system interrupt base it handles.
    IoApic { id: u8, address: u32, gsi_base: u32 },
    /// A legacy ISA IRQ remapped to a different GSI (very common: IRQ0 → GSI2).
    InterruptSourceOverride { bus: u8, source_irq: u8, gsi: u32, flags: u16 },
    /// Overrides the 32-bit local-APIC address from the MADT header with a 64-bit one.
    LocalApicAddressOverride(u64),
    /// An entry type this parser does not model yet (kept so counts stay honest).
    Other(u8),
}

/// The parsed MADT: the local-APIC MMIO address and the enumerated entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Madt {
    /// Local APIC address (from the header, upgraded by a `LocalApicAddressOverride` if present).
    pub local_apic_address: u64,
    pub entries: Vec<MadtEntry>,
}

impl Madt {
    /// Number of enabled processor-local APICs — i.e. usable CPU cores (for SMP, E-11).
    pub fn enabled_cpu_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, MadtEntry::LocalApic { enabled: true, .. }))
            .count()
    }

    /// The first IO-APIC's `(id, address, gsi_base)`, if any.
    pub fn first_io_apic(&self) -> Option<(u8, u32, u32)> {
        self.entries.iter().find_map(|e| match *e {
            MadtEntry::IoApic { id, address, gsi_base } => Some((id, address, gsi_base)),
            _ => None,
        })
    }

    /// Resolve a legacy ISA IRQ to its global system interrupt, applying any source override
    /// (defaults to identity when none is present — the ACPI-specified behavior).
    pub fn irq_to_gsi(&self, irq: u8) -> u32 {
        self.entries
            .iter()
            .find_map(|e| match *e {
                MadtEntry::InterruptSourceOverride { source_irq, gsi, .. } if source_irq == irq => {
                    Some(gsi)
                }
                _ => None,
            })
            .unwrap_or(irq as u32)
    }
}

/// Parse a MADT (signature "APIC") from its full bytes (header included). Returns `None` if the
/// header is missing/short or the signature is wrong. Malformed/truncated entries stop iteration
/// rather than panicking (this runs in the kernel).
pub fn parse_madt(b: &[u8]) -> Option<Madt> {
    let header = SdtHeader::parse(b)?;
    if !header.has_signature(b"APIC") {
        return None;
    }
    let len = core::cmp::min(header.length as usize, b.len());
    // MADT-specific fields: local APIC address (u32) then flags (u32) right after the header.
    let mut madt = Madt {
        local_apic_address: read_u32(b, SDT_HEADER_LEN) as u64,
        entries: Vec::new(),
    };

    // Entries begin at offset 44 (header + apic_addr + flags). Each: type u8, length u8, payload.
    let mut off = SDT_HEADER_LEN + 8;
    while off + 2 <= len {
        let etype = b[off];
        let elen = b[off + 1] as usize;
        // A zero/short length would loop forever or overrun — stop defensively.
        if elen < 2 || off + elen > len {
            break;
        }
        let e = &b[off..off + elen];
        let entry = match etype {
            0 => MadtEntry::LocalApic {
                acpi_processor_id: e.get(2).copied().unwrap_or(0),
                apic_id: e.get(3).copied().unwrap_or(0),
                enabled: read_u32(e, 4) & 1 != 0,
            },
            1 => MadtEntry::IoApic {
                id: e.get(2).copied().unwrap_or(0),
                address: read_u32(e, 4),
                gsi_base: read_u32(e, 8),
            },
            2 => MadtEntry::InterruptSourceOverride {
                bus: e.get(2).copied().unwrap_or(0),
                source_irq: e.get(3).copied().unwrap_or(0),
                gsi: read_u32(e, 4),
                flags: read_u16(e, 8),
            },
            5 => MadtEntry::LocalApicAddressOverride(read_u64(e, 4)),
            other => MadtEntry::Other(other),
        };
        if let MadtEntry::LocalApicAddressOverride(addr) = entry {
            madt.local_apic_address = addr;
        }
        madt.entries.push(entry);
        off += elen;
    }
    Some(madt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but valid MADT byte buffer from entry byte-blocks.
    fn build_madt(local_apic_addr: u32, flags: u32, entries: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&local_apic_addr.to_le_bytes());
        body.extend_from_slice(&flags.to_le_bytes());
        for e in entries {
            body.extend_from_slice(e);
        }
        let total = SDT_HEADER_LEN + body.len();
        let mut b = Vec::with_capacity(total);
        b.extend_from_slice(b"APIC"); // signature
        b.extend_from_slice(&(total as u32).to_le_bytes()); // length
        b.push(4); // revision
        b.push(0); // checksum (not validated in these tests)
        b.extend_from_slice(&[0u8; 6]); // oem_id
        b.extend_from_slice(&[0u8; 8]); // oem_table_id
        b.extend_from_slice(&[0u8; 4]); // oem_revision
        b.extend_from_slice(&[0u8; 4]); // creator_id
        b.extend_from_slice(&[0u8; 4]); // creator_revision
        assert_eq!(b.len(), SDT_HEADER_LEN);
        b.extend_from_slice(&body);
        b
    }

    fn local_apic(pid: u8, apic_id: u8, enabled: bool) -> Vec<u8> {
        alloc::vec![0u8, 8, pid, apic_id, if enabled { 1 } else { 0 }, 0, 0, 0]
    }
    fn io_apic(id: u8, addr: u32, gsi: u32) -> Vec<u8> {
        let mut v = alloc::vec![1u8, 12, id, 0];
        v.extend_from_slice(&addr.to_le_bytes());
        v.extend_from_slice(&gsi.to_le_bytes());
        v
    }
    fn iso(bus: u8, irq: u8, gsi: u32, flags: u16) -> Vec<u8> {
        let mut v = alloc::vec![2u8, 10, bus, irq];
        v.extend_from_slice(&gsi.to_le_bytes());
        v.extend_from_slice(&flags.to_le_bytes());
        v
    }

    #[test]
    fn rejects_wrong_signature() {
        let mut b = build_madt(0xFEE0_0000, 1, &[]);
        b[0] = b'X';
        assert!(parse_madt(&b).is_none());
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(parse_madt(&[0u8; 10]).is_none());
    }

    #[test]
    fn parses_local_apic_address_and_cpus() {
        let b = build_madt(
            0xFEE0_0000,
            1,
            &[local_apic(0, 0, true), local_apic(1, 1, true), local_apic(2, 2, false)],
        );
        let m = parse_madt(&b).unwrap();
        assert_eq!(m.local_apic_address, 0xFEE0_0000);
        assert_eq!(m.enabled_cpu_count(), 2); // third is disabled
        assert_eq!(m.entries.len(), 3);
    }

    #[test]
    fn parses_io_apic() {
        let b = build_madt(0xFEE0_0000, 1, &[io_apic(0, 0xFEC0_0000, 0)]);
        let m = parse_madt(&b).unwrap();
        assert_eq!(m.first_io_apic(), Some((0, 0xFEC0_0000, 0)));
    }

    #[test]
    fn interrupt_source_override_remaps_irq() {
        // Classic PC: IRQ0 (PIT) is remapped to GSI 2.
        let b = build_madt(0xFEE0_0000, 1, &[iso(0, 0, 2, 0), io_apic(0, 0xFEC0_0000, 0)]);
        let m = parse_madt(&b).unwrap();
        assert_eq!(m.irq_to_gsi(0), 2); // overridden
        assert_eq!(m.irq_to_gsi(1), 1); // no override → identity
    }

    #[test]
    fn local_apic_address_override_wins() {
        let mut ov = alloc::vec![5u8, 12, 0, 0];
        ov.extend_from_slice(&0xFFFF_F000_FEE0_0000u64.to_le_bytes());
        let b = build_madt(0xFEE0_0000, 1, &[ov]);
        let m = parse_madt(&b).unwrap();
        assert_eq!(m.local_apic_address, 0xFFFF_F000_FEE0_0000);
    }

    #[test]
    fn zero_length_entry_does_not_hang() {
        // A malformed entry with length 0 must stop iteration, not spin forever.
        let mut b = build_madt(0xFEE0_0000, 1, &[]);
        b.extend_from_slice(&[0u8, 0]); // type 0, length 0
        // Fix the header length to include the bogus entry.
        let total = b.len() as u32;
        b[4..8].copy_from_slice(&total.to_le_bytes());
        let m = parse_madt(&b).unwrap();
        assert_eq!(m.entries.len(), 0);
    }

    #[test]
    fn sdt_header_parses() {
        let b = build_madt(0xFEE0_0000, 1, &[]);
        let h = SdtHeader::parse(&b).unwrap();
        assert!(h.has_signature(b"APIC"));
        assert_eq!(h.revision, 4);
    }
}
