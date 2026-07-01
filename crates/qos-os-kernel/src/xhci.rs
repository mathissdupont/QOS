//! xHCI USB host-controller driver (WP-04, epic E-20).
//!
//! Step 1: detection. Registered with the device model (ADR-0016), this driver matches the PCI
//! xHCI class (0x0C/0x03/0x30) and reads the controller's capability registers — CAPLENGTH,
//! HCIVERSION, and HCSPARAMS1 (max ports / device slots). That proves QOS can find and talk to a
//! modern USB host controller, the foundation for controller bring-up, port enumeration, and HID
//! (the later WP-04 steps).

use qos_driver::{BusKind, Device, DeviceId, DeviceIo, Driver, DriverError};

/// PCI classification for an xHCI controller: base 0x0C (serial bus), sub 0x03 (USB),
/// prog-if 0x30 (xHCI), packed the way `device::pci_class` builds `DeviceId::class`.
const PCI_CLASS_XHCI: u32 = 0x0C_03_30;

// Capability-register offsets (from the MMIO base = CapBase).
const CAP_CAPLENGTH_HCIVERSION: u64 = 0x00; // u8 CAPLENGTH | u16 HCIVERSION at [31:16]
const CAP_HCSPARAMS1: u64 = 0x04; // maxSlots[7:0], maxIntrs[18:8], maxPorts[31:24]

pub struct XhciDriver;

impl Driver for XhciDriver {
    fn name(&self) -> &str {
        "xhci"
    }

    fn matches(&self, id: &DeviceId) -> bool {
        id.bus == BusKind::Pci && id.class == PCI_CLASS_XHCI
    }

    fn probe(&self, dev: &mut Device, io: &mut dyn DeviceIo) -> Result<(), DriverError> {
        let (base, _len) = dev.mmio().ok_or(DriverError::MissingResource)?;

        let cap0 = io.mmio_read32(base + CAP_CAPLENGTH_HCIVERSION);
        let cap_length = cap0 & 0xFF; // operational registers begin at base + CAPLENGTH
        let hci_version = (cap0 >> 16) & 0xFFFF; // BCD, e.g. 0x0100 = USB 3.0 xHCI 1.0

        let hcs1 = io.mmio_read32(base + CAP_HCSPARAMS1);
        let max_slots = hcs1 & 0xFF;
        let max_ports = (hcs1 >> 24) & 0xFF;

        // Sanity: a real xHCI reports a nonzero cap length and at least one port.
        if cap_length == 0 || max_ports == 0 {
            return Err(DriverError::Unsupported);
        }

        crate::serial_println!(
            "[XHCI] controller @ {:#x}: HCIVERSION={:#06x} CAPLENGTH={} slots={} ports={}",
            base, hci_version, cap_length, max_slots, max_ports
        );
        // Steps 2–5 (reset, rings, enumeration, HID) build from here.
        Ok(())
    }
}
