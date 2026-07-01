//! Kernel integration of the portable device/driver model (ADR-0016, epic E-01).
//!
//! This is the seam between `qos-driver` (the portable, host-tested core) and real hardware:
//! - [`KernelIo`] implements `qos_driver::DeviceIo` over real MMIO (via the physical-memory
//!   offset mapping), port I/O, and — later — interrupt routing.
//! - [`init`] turns the PCI enumeration ([`crate::pci`]) into `qos_driver::Device`s, registers
//!   the available drivers, runs `probe_all`, and logs an `lsdev`-style bind table.
//!
//! This runs **alongside** the existing hardcoded driver `init()`s (fallback-first per ADR-0015):
//! it does not yet take over device bring-up, so it cannot regress the working boot. Drivers are
//! migrated onto the model incrementally; the goal here is to prove the seam end-to-end on real
//! hardware (enumerate → match → probe → bind).

use alloc::boxed::Box;
use core::ptr::{read_volatile, write_volatile};

use qos_driver::{
    BusKind, Device, DeviceId, DeviceManager, DeviceState, DeviceIo, Driver, DriverError, Resource,
};

use crate::arch;
use crate::pci::{self, PciDevice};

/// Real hardware gateway for the driver core. MMIO goes through the bootloader's physical-memory
/// offset mapping ([`crate::memory::mmio_virt_addr`]); port I/O through the arch helpers.
struct KernelIo;

impl DeviceIo for KernelIo {
    fn mmio_read32(&self, addr: u64) -> u32 {
        let virt = crate::memory::mmio_virt_addr(addr).as_u64() as *const u32;
        unsafe { read_volatile(virt) }
    }
    fn mmio_write32(&mut self, addr: u64, val: u32) {
        let virt = crate::memory::mmio_virt_addr(addr).as_u64() as *mut u32;
        unsafe { write_volatile(virt, val) }
    }
    fn port_in8(&self, port: u16) -> u8 {
        unsafe { arch::inb(port) }
    }
    fn port_out8(&mut self, port: u16, val: u8) {
        unsafe { arch::outb(port, val) }
    }
    // Interrupt routing is still the legacy PIC path; the driver model gains real IRQ hookup with
    // the APIC/IO-APIC work (E-10). For now, acknowledge the request without wiring an ISR so the
    // model's bind flow is exercised without changing interrupt behavior.
    fn register_irq(&mut self, line: u8) -> Result<u8, DriverError> {
        Ok(line)
    }
}

/// Pack a PCI class/subclass/prog-if triple into the 24-bit `DeviceId::class` field.
fn pci_class(dev: &PciDevice) -> u32 {
    ((dev.class_code as u32) << 16) | ((dev.subclass as u32) << 8) | (dev.prog_if as u32)
}

/// Derive the resource list for a PCI device from BAR0 and its interrupt line.
///
/// BAR0 encoding: bit0 = 0 → memory BAR (bits[2:1] give the type: 00 = 32-bit, 10 = 64-bit, where
/// the high 32 bits live in BAR1); bit0 = 1 → I/O BAR. Handling the **64-bit** case matters: many
/// modern controllers (e.g. the xHCI USB host controller) place their MMIO above 4 GiB, so the
/// real base is `(bar1 << 32) | (bar0 & !0xF)` — using bar0 alone yields base 0.
fn pci_resources(dev: &PciDevice) -> alloc::vec::Vec<Resource> {
    let mut res = alloc::vec::Vec::new();
    if dev.bar0 != 0 {
        if dev.bar0 & 1 == 0 {
            // Memory BAR. bits[2:1] == 0b10 → 64-bit: combine BAR1 as the high dword.
            let is_64bit = (dev.bar0 >> 1) & 0b11 == 0b10;
            let low = (dev.bar0 & !0xF) as u64;
            let base = if is_64bit { ((dev.bar1 as u64) << 32) | low } else { low };
            res.push(Resource::Mmio { base, len: 0 });
        } else {
            // I/O BAR: bit0=1. Mask the low 2 flag bits.
            res.push(Resource::Port { base: (dev.bar0 & !0x3) as u16, len: 0 });
        }
    }
    if dev.interrupt_line != 0 && dev.interrupt_line != 0xFF {
        res.push(Resource::Irq(dev.interrupt_line));
    }
    res
}

/// Driver for the Intel E1000 family (the NIC QOS already supports). On the model it recognizes
/// the Ethernet-controller class and claims its resources; actual packet handling stays in the
/// legacy `e1000` driver until that is migrated. Probe here validates the resource plumbing.
struct E1000Driver;

impl Driver for E1000Driver {
    fn name(&self) -> &str {
        "e1000"
    }
    fn matches(&self, id: &DeviceId) -> bool {
        // Intel (0x8086) network-class device, or specifically the emulated 82540EM (0x100e).
        id.bus == BusKind::Pci
            && ((id.vendor == 0x8086 && (id.class >> 16) == 0x02) || id.device == 0x100e)
    }
    fn probe(&self, dev: &mut Device, _io: &mut dyn DeviceIo) -> Result<(), DriverError> {
        let (base, _) = dev.mmio().ok_or(DriverError::MissingResource)?;
        let irq = dev.irq().ok_or(DriverError::MissingResource)?;
        crate::serial_println!(
            "[DEV] e1000 driver claims {} (MMIO 0x{:08x}, IRQ {})",
            dev.name, base, irq
        );
        Ok(())
    }
}

/// Enumerate via PCI, register drivers, and probe. Logs an `lsdev`-style bind table. Additive:
/// does not replace the legacy device `init()`s.
pub fn init() {
    let mut mgr = DeviceManager::new();
    mgr.register_driver(Box::new(E1000Driver));
    mgr.register_driver(Box::new(crate::xhci::XhciDriver));

    for pdev in pci::devices() {
        let id = DeviceId::pci(pdev.vendor_id, pdev.device_id, pci_class(&pdev));
        let name = pci::vendor_name(pdev.vendor_id);
        mgr.add_device(Device::new(name, id, pci_resources(&pdev)));
    }

    let mut io = KernelIo;
    let summary = mgr.probe_all(&mut io);

    crate::serial_println!(
        "[DEV] device model: {} devices, {} bound, {} unmatched, {} failed",
        mgr.devices().len(),
        summary.bound,
        summary.unmatched,
        summary.failed
    );
    for d in mgr.devices() {
        let state = match &d.state {
            DeviceState::Bound(drv) => drv.as_str(),
            DeviceState::Unbound => "(no driver)",
            DeviceState::Failed(_) => "(probe failed)",
        };
        crate::serial_println!(
            "[DEV]   {:04x}:{:04x} class {:06x}  {:<16} -> {}",
            d.id.vendor, d.id.device, d.id.class, d.name, state
        );
    }
}
