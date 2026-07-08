//! PCI Bus Enumeration for QOS
//!
//! Scans the PCI bus to discover hardware devices.
//! Essential for finding network cards, storage controllers, etc.

use crate::arch;
use alloc::vec::Vec;
use spin::Mutex;

/// PCI Configuration Space ports
const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// PCI device database
static PCI_DEVICES: Mutex<Vec<PciDevice>> = Mutex::new(Vec::new());

/// PCI Device information
#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub header_type: u8,
    pub bar0: u32,
    pub bar1: u32,
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
}

impl PciDevice {
    /// Get human-readable class name
    pub fn class_name(&self) -> &'static str {
        match self.class_code {
            0x00 => "Unclassified",
            0x01 => "Mass Storage",
            0x02 => "Network",
            0x03 => "Display",
            0x04 => "Multimedia",
            0x05 => "Memory",
            0x06 => "Bridge",
            0x07 => "Communication",
            0x08 => "System Peripheral",
            0x09 => "Input Device",
            0x0A => "Docking Station",
            0x0B => "Processor",
            0x0C => "Serial Bus",
            0x0D => "Wireless",
            0x0E => "Intelligent I/O",
            0x0F => "Satellite",
            0x10 => "Encryption",
            0x11 => "Signal Processing",
            0xFF => "Unknown",
            _ => "Reserved",
        }
    }

    /// Get subclass name for storage controllers
    pub fn storage_subclass_name(&self) -> &'static str {
        if self.class_code != 0x01 {
            return "";
        }
        match self.subclass {
            0x00 => "SCSI",
            0x01 => "IDE",
            0x02 => "Floppy",
            0x03 => "IPI",
            0x04 => "RAID",
            0x05 => "ATA",
            0x06 => "SATA",
            0x07 => "SAS",
            0x08 => "NVMe",
            _ => "Other",
        }
    }

    /// Get subclass name for network controllers
    pub fn network_subclass_name(&self) -> &'static str {
        if self.class_code != 0x02 {
            return "";
        }
        match self.subclass {
            0x00 => "Ethernet",
            0x01 => "Token Ring",
            0x02 => "FDDI",
            0x03 => "ATM",
            0x04 => "ISDN",
            0x05 => "WorldFip",
            0x06 => "PICMG",
            0x07 => "InfiniBand",
            0x80 => "Other",
            _ => "Unknown",
        }
    }
}

/// Build PCI configuration address
fn pci_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let bus = bus as u32;
    let device = device as u32;
    let function = function as u32;
    let offset = (offset & 0xFC) as u32;
    
    0x8000_0000 | (bus << 16) | (device << 11) | (function << 8) | offset
}

/// Read 32-bit value from PCI configuration space
fn pci_read_config32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let address = pci_address(bus, device, function, offset);
    unsafe {
        arch::outl(PCI_CONFIG_ADDRESS, address);
        arch::inl(PCI_CONFIG_DATA)
    }
}

/// Read 16-bit value from PCI configuration space
fn pci_read_config16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let val = pci_read_config32(bus, device, function, offset & 0xFC);
    ((val >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

/// Read 8-bit value from PCI configuration space
fn pci_read_config8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let val = pci_read_config32(bus, device, function, offset & 0xFC);
    ((val >> ((offset & 3) * 8)) & 0xFF) as u8
}

/// Write a 32-bit value to PCI configuration space (dword-aligned offset).
fn pci_write_config32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    let address = pci_address(bus, device, function, offset);
    unsafe {
        arch::outl(PCI_CONFIG_ADDRESS, address);
        arch::outl(PCI_CONFIG_DATA, value);
    }
}

/// Write a 16-bit value to PCI configuration space (read-modify-write of the containing dword).
fn pci_write_config16(bus: u8, device: u8, function: u8, offset: u8, value: u16) {
    let dword = pci_read_config32(bus, device, function, offset & 0xFC);
    let shift = (offset & 2) * 8;
    let cleared = dword & !(0xFFFFu32 << shift);
    pci_write_config32(bus, device, function, offset & 0xFC, cleared | ((value as u32) << shift));
}

// ---- Public config-space accessors keyed by a discovered `PciDevice` (used by MSI setup, etc.) ----

/// Read a 32-bit config-space dword from `dev` at `offset`.
pub fn config_read32(dev: &PciDevice, offset: u8) -> u32 {
    pci_read_config32(dev.bus, dev.device, dev.function, offset)
}

/// Read a 16-bit config-space word from `dev` at `offset`.
pub fn config_read16(dev: &PciDevice, offset: u8) -> u16 {
    pci_read_config16(dev.bus, dev.device, dev.function, offset)
}

/// Write a 32-bit config-space dword to `dev` at `offset`.
pub fn config_write32(dev: &PciDevice, offset: u8, value: u32) {
    pci_write_config32(dev.bus, dev.device, dev.function, offset, value);
}

/// Write a 16-bit config-space word to `dev` at `offset`.
pub fn config_write16(dev: &PciDevice, offset: u8, value: u16) {
    pci_write_config16(dev.bus, dev.device, dev.function, offset, value);
}

/// Find a PCI capability by id in `dev`'s capability list; returns its config-space offset.
///
/// Walks the standard linked list at 0x34 (only if the Capabilities List status bit is set). Common
/// ids: 0x05 = MSI, 0x11 = MSI-X, 0x10 = PCIe.
pub fn find_capability(dev: &PciDevice, cap_id: u8) -> Option<u8> {
    // Status register (0x06) bit 4 = Capabilities List present.
    let status = pci_read_config16(dev.bus, dev.device, dev.function, 0x06);
    if status & (1 << 4) == 0 {
        return None;
    }
    let mut ptr = pci_read_config8(dev.bus, dev.device, dev.function, 0x34) & 0xFC;
    // Bounded walk (config space is 256 bytes; guard against loops).
    for _ in 0..48 {
        if ptr == 0 {
            break;
        }
        let id = pci_read_config8(dev.bus, dev.device, dev.function, ptr);
        if id == cap_id {
            return Some(ptr);
        }
        ptr = pci_read_config8(dev.bus, dev.device, dev.function, ptr + 1) & 0xFC;
    }
    None
}

/// List all capability `(id, offset)` pairs in `dev`'s capability list (diagnostic).
pub fn capabilities(dev: &PciDevice) -> Vec<(u8, u8)> {
    let mut caps = Vec::new();
    let status = pci_read_config16(dev.bus, dev.device, dev.function, 0x06);
    if status & (1 << 4) == 0 {
        return caps;
    }
    let mut ptr = pci_read_config8(dev.bus, dev.device, dev.function, 0x34) & 0xFC;
    for _ in 0..48 {
        if ptr == 0 {
            break;
        }
        let id = pci_read_config8(dev.bus, dev.device, dev.function, ptr);
        caps.push((id, ptr));
        ptr = pci_read_config8(dev.bus, dev.device, dev.function, ptr + 1) & 0xFC;
    }
    caps
}

/// Check if a PCI device exists at the given location
fn device_exists(bus: u8, device: u8, function: u8) -> bool {
    pci_read_config16(bus, device, function, 0x00) != 0xFFFF
}

/// Read full device information
fn read_device(bus: u8, device: u8, function: u8) -> Option<PciDevice> {
    let vendor_id = pci_read_config16(bus, device, function, 0x00);
    if vendor_id == 0xFFFF {
        return None;
    }
    
    let device_id = pci_read_config16(bus, device, function, 0x02);
    let class_info = pci_read_config32(bus, device, function, 0x08);
    let header_type = pci_read_config8(bus, device, function, 0x0E);
    let bar0 = pci_read_config32(bus, device, function, 0x10);
    let bar1 = pci_read_config32(bus, device, function, 0x14);
    let interrupt = pci_read_config16(bus, device, function, 0x3C);
    
    Some(PciDevice {
        bus,
        device,
        function,
        vendor_id,
        device_id,
        revision: (class_info & 0xFF) as u8,
        prog_if: ((class_info >> 8) & 0xFF) as u8,
        subclass: ((class_info >> 16) & 0xFF) as u8,
        class_code: ((class_info >> 24) & 0xFF) as u8,
        header_type: header_type & 0x7F,
        bar0,
        bar1,
        interrupt_line: (interrupt & 0xFF) as u8,
        interrupt_pin: ((interrupt >> 8) & 0xFF) as u8,
    })
}

/// Scan a single bus
fn scan_bus(bus: u8, devices: &mut Vec<PciDevice>) {
    for device in 0..32 {
        if !device_exists(bus, device, 0) {
            continue;
        }
        
        if let Some(dev) = read_device(bus, device, 0) {
            let is_multifunction = pci_read_config8(bus, device, 0, 0x0E) & 0x80 != 0;
            devices.push(dev);
            
            // If multifunction device, check other functions
            if is_multifunction {
                for function in 1..8 {
                    if device_exists(bus, device, function) {
                        if let Some(dev) = read_device(bus, device, function) {
                            devices.push(dev);
                        }
                    }
                }
            }
            
            // If this is a PCI-to-PCI bridge, scan the secondary bus
            if dev.class_code == 0x06 && dev.subclass == 0x04 {
                let secondary_bus = pci_read_config8(bus, device, 0, 0x19);
                if secondary_bus != 0 {
                    scan_bus(secondary_bus, devices);
                }
            }
        }
    }
}

/// Initialize PCI subsystem - scan all buses
pub fn init() {
    let mut devices = Vec::new();
    
    // Check if PCI exists by checking host bridge
    if pci_read_config16(0, 0, 0, 0x00) == 0xFFFF {
        crate::serial_println!("[PCI] No PCI bus detected");
        return;
    }
    
    // Check for multiple PCI host controllers
    let header_type = pci_read_config8(0, 0, 0, 0x0E);
    if header_type & 0x80 == 0 {
        // Single PCI host controller
        scan_bus(0, &mut devices);
    } else {
        // Multiple PCI host controllers
        for function in 0..8 {
            if pci_read_config16(0, 0, function, 0x00) != 0xFFFF {
                scan_bus(function, &mut devices);
            }
        }
    }
    
    let count = devices.len();
    *PCI_DEVICES.lock() = devices;
    
    crate::serial_println!("[PCI] Found {} devices", count);
}

/// Get list of all PCI devices
pub fn devices() -> Vec<PciDevice> {
    PCI_DEVICES.lock().clone()
}

/// Find devices by class code
pub fn find_by_class(class: u8) -> Vec<PciDevice> {
    PCI_DEVICES.lock()
        .iter()
        .filter(|d| d.class_code == class)
        .copied()
        .collect()
}

/// Find devices by class and subclass
pub fn find_by_class_subclass(class: u8, subclass: u8) -> Vec<PciDevice> {
    PCI_DEVICES.lock()
        .iter()
        .filter(|d| d.class_code == class && d.subclass == subclass)
        .copied()
        .collect()
}

/// Find devices by vendor and device ID
pub fn find_by_id(vendor: u16, device: u16) -> Vec<PciDevice> {
    PCI_DEVICES.lock()
        .iter()
        .filter(|d| d.vendor_id == vendor && d.device_id == device)
        .copied()
        .collect()
}

/// Get vendor name (common vendors)
pub fn vendor_name(vendor_id: u16) -> &'static str {
    match vendor_id {
        0x8086 => "Intel",
        0x1022 => "AMD",
        0x10DE => "NVIDIA",
        0x1002 => "AMD/ATI",
        0x14E4 => "Broadcom",
        0x10EC => "Realtek",
        0x1AF4 => "Red Hat (virtio)",
        0x1234 => "QEMU",
        0x1B36 => "Red Hat QEMU",
        0x15AD => "VMware",
        0x80EE => "VirtualBox",
        _ => "Unknown",
    }
}

/// List all devices to console
pub fn list_devices() {
    let devices = PCI_DEVICES.lock();
    crate::println!("PCI Devices ({} found):", devices.len());
    crate::println!("Bus:Dev.Fn  Vendor:Device  Class        Description");
    crate::println!("─────────────────────────────────────────────────────");
    
    for dev in devices.iter() {
        crate::println!(
            "{:02X}:{:02X}.{:X}   {:04X}:{:04X}     {:02X}:{:02X}        {} {}",
            dev.bus, dev.device, dev.function,
            dev.vendor_id, dev.device_id,
            dev.class_code, dev.subclass,
            vendor_name(dev.vendor_id),
            dev.class_name()
        );
    }
}
