//! # qos-driver
//!
//! The portable core of the QOS device/driver model (ADR-0016, epic E-01). It separates *what a
//! device is* ([`Device`] / [`DeviceId`] / [`Resource`]) from *which driver drives it*
//! ([`Driver`]), and a [`DeviceManager`] matches and binds them. All real hardware access goes
//! through the [`DeviceIo`] trait, which the kernel implements and tests mock — so this crate,
//! including the fiddly matching/binding/resource logic, is unit-tested on the host.
//!
//! `no_std` when compiled into the kernel (`#![cfg_attr(not(test), no_std)]`); uses `alloc`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// The bus a device lives on. Extensible as new enumerators land (USB, platform ACPI, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusKind {
    /// PCI/PCIe.
    Pci,
    /// A fixed platform device (e.g. PS/2, PIT) with a synthetic id.
    Platform,
    /// A device on the USB bus (E-20).
    Usb,
}

/// How a device is identified and matched to a driver. `vendor`/`device` are for exact matches;
/// `class` enables class-based matching (e.g. "any USB HID" or "any Ethernet controller"). Fields
/// that don't apply to a bus are left zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceId {
    pub bus: BusKind,
    pub vendor: u16,
    pub device: u16,
    /// Class code (bus-specific encoding, e.g. PCI class/subclass/prog-if packed into 24 bits).
    pub class: u32,
}

impl DeviceId {
    pub const fn pci(vendor: u16, device: u16, class: u32) -> Self {
        Self { bus: BusKind::Pci, vendor, device, class }
    }
    pub const fn platform(device: u16) -> Self {
        Self { bus: BusKind::Platform, vendor: 0, device, class: 0 }
    }
}

/// A hardware resource a device exposes and a driver uses. The manager records which device owns
/// which resource so future work (IRQ sharing, hotplug removal) has a single source of truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// Memory-mapped I/O window.
    Mmio { base: u64, len: u64 },
    /// Legacy port-I/O range.
    Port { base: u16, len: u16 },
    /// An interrupt line (IRQ/GSI).
    Irq(u8),
}

/// Bind state of a device within the manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceState {
    /// No driver bound yet.
    Unbound,
    /// Bound to the named driver after a successful `probe`.
    Bound(String),
    /// A matching driver's `probe` failed.
    Failed(DriverError),
}

/// An enumerated device: its identity, the resources it exposes, a human name, and bind state.
#[derive(Clone, Debug)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub resources: Vec<Resource>,
    pub state: DeviceState,
}

impl Device {
    pub fn new(name: impl Into<String>, id: DeviceId, resources: Vec<Resource>) -> Self {
        Self { id, name: name.into(), resources, state: DeviceState::Unbound }
    }

    /// First MMIO window, if any — the common case for a driver claiming its registers.
    pub fn mmio(&self) -> Option<(u64, u64)> {
        self.resources.iter().find_map(|r| match *r {
            Resource::Mmio { base, len } => Some((base, len)),
            _ => None,
        })
    }

    /// First port-I/O range, if any.
    pub fn port(&self) -> Option<(u16, u16)> {
        self.resources.iter().find_map(|r| match *r {
            Resource::Port { base, len } => Some((base, len)),
            _ => None,
        })
    }

    /// First IRQ line, if any.
    pub fn irq(&self) -> Option<u8> {
        self.resources.iter().find_map(|r| match *r {
            Resource::Irq(n) => Some(n),
            _ => None,
        })
    }

    pub fn is_bound(&self) -> bool {
        matches!(self.state, DeviceState::Bound(_))
    }
}

/// Errors a driver's `probe` can report. `Clone`/`Eq` so it can live in [`DeviceState::Failed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverError {
    /// A required resource (MMIO/port/IRQ) was missing.
    MissingResource,
    /// The device is present but not in a usable state.
    Unsupported,
    /// Hardware access failed (bad read-back, timeout, …).
    Io,
    /// IRQ registration was refused.
    IrqUnavailable,
}

/// The kernel-provided hardware gateway. The portable core never touches hardware directly; it
/// asks through this trait, so tests supply a mock and the kernel supplies the real MMIO/port/IRQ
/// paths. Kept minimal on purpose; grow it as drivers need more (MSI, DMA mapping, …).
pub trait DeviceIo {
    fn mmio_read32(&self, addr: u64) -> u32;
    fn mmio_write32(&mut self, addr: u64, val: u32);
    fn port_in8(&self, port: u16) -> u8;
    fn port_out8(&mut self, port: u16, val: u8);
    /// Wire up `line` for this device. Returns the line on success. Default: refuse (a bus that
    /// has no IRQ routing yet). The kernel overrides this to hook the actual interrupt.
    fn register_irq(&mut self, _line: u8) -> Result<u8, DriverError> {
        Err(DriverError::IrqUnavailable)
    }
}

/// A driver: it recognizes a class of device and initializes each instance it is bound to.
pub trait Driver {
    fn name(&self) -> &str;
    /// True if this driver can drive a device with `id`.
    fn matches(&self, id: &DeviceId) -> bool;
    /// Initialize `dev`. Called once by the manager when the device is bound to this driver.
    fn probe(&self, dev: &mut Device, io: &mut dyn DeviceIo) -> Result<(), DriverError>;
}

/// Summary returned by [`DeviceManager::probe_all`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProbeSummary {
    /// Devices newly bound to a driver this pass.
    pub bound: usize,
    /// Devices whose matching driver's `probe` failed.
    pub failed: usize,
    /// Unbound devices for which no driver matched.
    pub unmatched: usize,
}

/// The registry: drivers register, enumerated devices are added, and `probe_all` matches and
/// binds them. One instance owns the device/driver tables for the system.
pub struct DeviceManager {
    drivers: Vec<Box<dyn Driver>>,
    devices: Vec<Device>,
}

impl DeviceManager {
    pub fn new() -> Self {
        Self { drivers: Vec::new(), devices: Vec::new() }
    }

    pub fn register_driver(&mut self, driver: Box<dyn Driver>) {
        self.drivers.push(driver);
    }

    pub fn add_device(&mut self, device: Device) {
        self.devices.push(device);
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn driver_names(&self) -> Vec<&str> {
        self.drivers.iter().map(|d| d.name()).collect()
    }

    pub fn bound_count(&self) -> usize {
        self.devices.iter().filter(|d| d.is_bound()).count()
    }

    /// Bind every currently-unbound device to the first matching driver and `probe` it, using
    /// `io` for hardware access. Idempotent: already-bound/failed devices are skipped, so it is
    /// safe to call again after new devices are enumerated (e.g. USB hotplug later).
    pub fn probe_all(&mut self, io: &mut dyn DeviceIo) -> ProbeSummary {
        let mut summary = ProbeSummary::default();
        for dev in self.devices.iter_mut() {
            if dev.state != DeviceState::Unbound {
                continue;
            }
            match self.drivers.iter().find(|drv| drv.matches(&dev.id)) {
                None => summary.unmatched += 1,
                Some(drv) => match drv.probe(dev, io) {
                    Ok(()) => {
                        dev.state = DeviceState::Bound(String::from(drv.name()));
                        summary.bound += 1;
                    }
                    Err(e) => {
                        dev.state = DeviceState::Failed(e);
                        summary.failed += 1;
                    }
                },
            }
        }
        summary
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    /// A mock DeviceIo that records writes and serves canned MMIO read-backs, so probe logic can
    /// be asserted without hardware.
    #[derive(Default)]
    struct MockIo {
        mmio: BTreeMap<u64, u32>,
        ports: BTreeMap<u16, u8>,
        irqs: Vec<u8>,
        allow_irq: bool,
    }
    impl DeviceIo for MockIo {
        fn mmio_read32(&self, addr: u64) -> u32 {
            *self.mmio.get(&addr).unwrap_or(&0)
        }
        fn mmio_write32(&mut self, addr: u64, val: u32) {
            self.mmio.insert(addr, val);
        }
        fn port_in8(&self, port: u16) -> u8 {
            *self.ports.get(&port).unwrap_or(&0)
        }
        fn port_out8(&mut self, port: u16, val: u8) {
            self.ports.insert(port, val);
        }
        fn register_irq(&mut self, line: u8) -> Result<u8, DriverError> {
            if self.allow_irq {
                self.irqs.push(line);
                Ok(line)
            } else {
                Err(DriverError::IrqUnavailable)
            }
        }
    }

    // A NIC-class driver that claims its MMIO base, writes a "reset" register, and hooks its IRQ.
    struct FakeNic;
    impl Driver for FakeNic {
        fn name(&self) -> &str {
            "fake-nic"
        }
        fn matches(&self, id: &DeviceId) -> bool {
            id.bus == BusKind::Pci && id.class == 0x02_00_00 // Ethernet controller
        }
        fn probe(&self, dev: &mut Device, io: &mut dyn DeviceIo) -> Result<(), DriverError> {
            let (base, _len) = dev.mmio().ok_or(DriverError::MissingResource)?;
            io.mmio_write32(base + 0x10, 0x1); // pretend: reset
            let line = dev.irq().ok_or(DriverError::MissingResource)?;
            io.register_irq(line)?;
            Ok(())
        }
    }

    // A driver that always fails probe, to exercise the Failed path.
    struct BrokenDriver;
    impl Driver for BrokenDriver {
        fn name(&self) -> &str {
            "broken"
        }
        fn matches(&self, id: &DeviceId) -> bool {
            id.bus == BusKind::Platform && id.device == 0xDEAD
        }
        fn probe(&self, _dev: &mut Device, _io: &mut dyn DeviceIo) -> Result<(), DriverError> {
            Err(DriverError::Unsupported)
        }
    }

    fn nic_device() -> Device {
        Device::new(
            "eth-test",
            DeviceId::pci(0x8086, 0x100e, 0x02_00_00),
            alloc::vec![Resource::Mmio { base: 0x8104_0000, len: 0x2_0000 }, Resource::Irq(11)],
        )
    }

    #[test]
    fn matching_driver_binds_and_probes() {
        let mut m = DeviceManager::new();
        m.register_driver(Box::new(FakeNic));
        m.add_device(nic_device());
        let mut io = MockIo { allow_irq: true, ..Default::default() };

        let s = m.probe_all(&mut io);
        assert_eq!(s, ProbeSummary { bound: 1, failed: 0, unmatched: 0 });
        assert_eq!(m.devices()[0].state, DeviceState::Bound("fake-nic".into()));
        assert_eq!(m.bound_count(), 1);
        // The driver's probe actually drove the (mock) hardware:
        assert_eq!(io.mmio.get(&0x8104_0010), Some(&0x1));
        assert_eq!(io.irqs, alloc::vec![11]);
    }

    #[test]
    fn unmatched_device_stays_unbound() {
        let mut m = DeviceManager::new();
        m.register_driver(Box::new(FakeNic));
        // A display-class PCI device: no registered driver matches.
        m.add_device(Device::new(
            "vga",
            DeviceId::pci(0x1234, 0x1111, 0x03_00_00),
            alloc::vec![],
        ));
        let mut io = MockIo::default();
        let s = m.probe_all(&mut io);
        assert_eq!(s, ProbeSummary { bound: 0, failed: 0, unmatched: 1 });
        assert_eq!(m.devices()[0].state, DeviceState::Unbound);
    }

    #[test]
    fn probe_failure_marks_failed_not_bound() {
        let mut m = DeviceManager::new();
        m.register_driver(Box::new(BrokenDriver));
        m.add_device(Device::new("bad", DeviceId::platform(0xDEAD), alloc::vec![]));
        let mut io = MockIo::default();
        let s = m.probe_all(&mut io);
        assert_eq!(s, ProbeSummary { bound: 0, failed: 1, unmatched: 0 });
        assert_eq!(m.devices()[0].state, DeviceState::Failed(DriverError::Unsupported));
        assert!(!m.devices()[0].is_bound());
    }

    #[test]
    fn missing_irq_hardware_makes_probe_fail() {
        // allow_irq=false → register_irq refuses → probe returns IrqUnavailable → Failed.
        let mut m = DeviceManager::new();
        m.register_driver(Box::new(FakeNic));
        m.add_device(nic_device());
        let mut io = MockIo { allow_irq: false, ..Default::default() };
        let s = m.probe_all(&mut io);
        assert_eq!(s.failed, 1);
        assert_eq!(m.devices()[0].state, DeviceState::Failed(DriverError::IrqUnavailable));
    }

    #[test]
    fn first_matching_driver_wins() {
        // Two drivers match the same NIC; the first registered should win.
        struct NicA;
        impl Driver for NicA {
            fn name(&self) -> &str { "nic-a" }
            fn matches(&self, id: &DeviceId) -> bool { id.class == 0x02_00_00 }
            fn probe(&self, _d: &mut Device, _io: &mut dyn DeviceIo) -> Result<(), DriverError> { Ok(()) }
        }
        struct NicB;
        impl Driver for NicB {
            fn name(&self) -> &str { "nic-b" }
            fn matches(&self, id: &DeviceId) -> bool { id.class == 0x02_00_00 }
            fn probe(&self, _d: &mut Device, _io: &mut dyn DeviceIo) -> Result<(), DriverError> { Ok(()) }
        }
        let mut m = DeviceManager::new();
        m.register_driver(Box::new(NicA));
        m.register_driver(Box::new(NicB));
        m.add_device(nic_device());
        let mut io = MockIo::default();
        m.probe_all(&mut io);
        assert_eq!(m.devices()[0].state, DeviceState::Bound("nic-a".into()));
    }

    #[test]
    fn probe_all_is_idempotent_and_skips_bound() {
        let mut m = DeviceManager::new();
        m.register_driver(Box::new(FakeNic));
        m.add_device(nic_device());
        let mut io = MockIo { allow_irq: true, ..Default::default() };
        let first = m.probe_all(&mut io);
        assert_eq!(first.bound, 1);
        // Second pass: the device is already bound, so nothing new happens.
        let second = m.probe_all(&mut io);
        assert_eq!(second, ProbeSummary { bound: 0, failed: 0, unmatched: 0 });
        // The reset register wasn't written twice (probe not called again): still exactly 0x1.
        assert_eq!(io.irqs.len(), 1);
    }

    #[test]
    fn resource_accessors() {
        let d = nic_device();
        assert_eq!(d.mmio(), Some((0x8104_0000, 0x2_0000)));
        assert_eq!(d.irq(), Some(11));
        assert_eq!(d.port(), None);
    }
}
