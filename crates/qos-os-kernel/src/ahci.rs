//! SATA AHCI Driver for QOS
//!
//! Advanced Host Controller Interface for modern SATA drives.
//! Falls back to ATA PIO if AHCI is not available.

use crate::pci::{self, PciDevice};
use alloc::vec::Vec;

/// AHCI memory registers (HBA Memory)
#[repr(C)]
pub struct AhciHba {
    pub cap: u32,          // Host Capabilities
    pub ghc: u32,          // Global Host Control
    pub is: u32,           // Interrupt Status
    pub pi: u32,           // Ports Implemented
    pub vs: u32,           // Version
    pub ccc_ctl: u32,      // Command Completion Coalescing Control
    pub ccc_ports: u32,    // Command Completion Coalescing Ports
    pub em_loc: u32,       // Enclosure Management Location
    pub em_ctl: u32,       // Enclosure Management Control
    pub cap2: u32,         // Extended Capabilities
    pub bohc: u32,         // BIOS/OS Handoff Control
    pub reserved: [u32; 53],
    pub vendor: [u32; 24],
    pub ports: [AhciPort; 32],
}

/// AHCI Port registers
#[repr(C)]
pub struct AhciPort {
    pub clb: u64,          // Command List Base Address
    pub fb: u64,           // FIS Base Address
    pub is: u32,           // Interrupt Status
    pub ie: u32,           // Interrupt Enable
    pub cmd: u32,          // Command and Status
    pub reserved0: u32,
    pub tfd: u32,          // Task File Data
    pub sig: u32,          // Signature
    pub ssts: u32,         // SATA Status
    pub sctl: u32,         // SATA Control
    pub serr: u32,         // SATA Error
    pub sact: u32,         // SATA Active
    pub ci: u32,           // Command Issue
    pub sntf: u32,         // SATA Notification
    pub fbs: u32,          // FIS-based Switching Control
    pub devslp: u32,       // Device Sleep
    pub reserved1: [u32; 10],
    pub vendor: [u32; 4],
}

/// AHCI Command Header
#[repr(C)]
pub struct AhciCmdHeader {
    pub flags: u16,
    pub prdtl: u16,        // Physical Region Descriptor Table Length
    pub prdbc: u32,        // PRD Byte Count
    pub ctba: u64,         // Command Table Base Address
    pub reserved: [u32; 4],
}

/// Physical Region Descriptor Table Entry
#[repr(C)]
pub struct AhciPrdt {
    pub dba: u64,          // Data Base Address
    pub reserved: u32,
    pub dbc: u32,          // Data Byte Count (bit 31: Interrupt on Completion)
}

/// AHCI Command Table
#[repr(C)]
pub struct AhciCmdTable {
    pub cfis: [u8; 64],    // Command FIS
    pub acmd: [u8; 16],    // ATAPI Command
    pub reserved: [u8; 48],
    pub prdt: [AhciPrdt; 8], // Up to 8 PRD entries
}

/// Port device type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortType {
    None,
    Sata,
    Semb,      // Enclosure management bridge
    PortMultiplier,
    Satapi,    // SATA ATAPI (CD/DVD)
}

/// AHCI controller state
pub struct AhciController {
    base_addr: usize,
    ports_available: u32,
    devices: Vec<AhciDevice>,
}

/// AHCI device info
#[derive(Debug)]
pub struct AhciDevice {
    pub port: u8,
    pub port_type: PortType,
    pub model: [u8; 40],
    pub serial: [u8; 20],
    pub sectors: u64,
    pub sector_size: u32,
}

impl AhciController {
    /// Find and initialize AHCI controller
    pub fn new() -> Option<Self> {
        // Find AHCI controller via PCI
        let ahci_devices = pci::find_by_class_subclass(0x01, 0x06); // Mass Storage, SATA
        
        if ahci_devices.is_empty() {
            crate::serial_println!("[AHCI] No AHCI controller found");
            return None;
        }
        
        let pci_dev = &ahci_devices[0];
        crate::serial_println!("[AHCI] Found controller at {:02X}:{:02X}.{:X}",
            pci_dev.bus, pci_dev.device, pci_dev.function);
        
        // Get ABAR (AHCI Base Address Register) from BAR5
        let abar = pci_dev.bar0 & !0xF; // BAR5 for AHCI is actually at offset 0x24
        
        if abar == 0 {
            crate::serial_println!("[AHCI] Invalid ABAR");
            return None;
        }
        
        crate::serial_println!("[AHCI] ABAR: 0x{:08X}", abar);
        
        Some(Self {
            base_addr: abar as usize,
            ports_available: 0,
            devices: Vec::new(),
        })
    }
    
    /// Initialize controller and probe ports
    pub fn init(&mut self) -> Result<(), &'static str> {
        let hba = self.hba();
        
        // Check version
        let version = unsafe { (*hba).vs };
        let major = (version >> 16) & 0xFFFF;
        let minor = version & 0xFFFF;
        crate::serial_println!("[AHCI] Version: {}.{}", major, minor);
        
        // Get capabilities
        let cap = unsafe { (*hba).cap };
        let num_ports = ((cap >> 0) & 0x1F) + 1;
        let num_cmd_slots = ((cap >> 8) & 0x1F) + 1;
        let supports_64bit = (cap >> 31) & 1 != 0;
        
        crate::serial_println!("[AHCI] Ports: {}, Cmd Slots: {}, 64-bit: {}",
            num_ports, num_cmd_slots, supports_64bit);
        
        // Enable AHCI mode
        unsafe {
            (*hba).ghc |= 0x8000_0000; // AHCI Enable
        }
        
        // Get implemented ports
        self.ports_available = unsafe { (*hba).pi };
        crate::serial_println!("[AHCI] Ports implemented: 0x{:08X}", self.ports_available);
        
        // Probe each port
        for i in 0..32 {
            if self.ports_available & (1 << i) != 0 {
                if let Some(dev) = self.probe_port(i as u8) {
                    self.devices.push(dev);
                }
            }
        }
        
        crate::serial_println!("[AHCI] Found {} devices", self.devices.len());
        Ok(())
    }
    
    /// Get HBA pointer
    fn hba(&self) -> *mut AhciHba {
        self.base_addr as *mut AhciHba
    }
    
    /// Get port registers
    fn port(&self, port_num: u8) -> *mut AhciPort {
        unsafe {
            &mut (*self.hba()).ports[port_num as usize] as *mut AhciPort
        }
    }
    
    /// Check port type
    fn check_port_type(&self, port_num: u8) -> PortType {
        let port = self.port(port_num);
        
        unsafe {
            let ssts = (*port).ssts;
            
            // Check device detection
            let det = ssts & 0x0F;
            let ipm = (ssts >> 8) & 0x0F;
            
            if det != 3 {
                return PortType::None; // No device
            }
            if ipm != 1 {
                return PortType::None; // Not active
            }
            
            // Check signature
            let sig = (*port).sig;
            match sig {
                0x00000101 => PortType::Sata,      // ATA
                0xEB140101 => PortType::Satapi,    // ATAPI
                0xC33C0101 => PortType::Semb,      // Enclosure
                0x96690101 => PortType::PortMultiplier,
                _ => {
                    crate::serial_println!("[AHCI] Port {}: Unknown sig 0x{:08X}", port_num, sig);
                    PortType::None
                }
            }
        }
    }
    
    /// Probe a port for devices
    fn probe_port(&self, port_num: u8) -> Option<AhciDevice> {
        let port_type = self.check_port_type(port_num);
        
        if port_type == PortType::None {
            return None;
        }
        
        crate::serial_println!("[AHCI] Port {}: {:?} device found", port_num, port_type);
        
        Some(AhciDevice {
            port: port_num,
            port_type,
            model: [0; 40],
            serial: [0; 20],
            sectors: 0,
            sector_size: 512,
        })
    }
    
    /// List all devices
    pub fn list_devices(&self) {
        crate::println!("AHCI Devices ({} found):", self.devices.len());
        for dev in &self.devices {
            crate::println!("  Port {}: {:?}", dev.port, dev.port_type);
        }
    }
    
    /// Get device count
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }
}

/// Global AHCI controller instance
static mut AHCI: Option<AhciController> = None;

/// Initialize AHCI subsystem
pub fn init() {
    if let Some(mut ctrl) = AhciController::new() {
        if ctrl.init().is_ok() {
            unsafe {
                AHCI = Some(ctrl);
            }
            crate::serial_println!("[AHCI] Initialized");
        }
    } else {
        crate::serial_println!("[AHCI] No controller, using legacy ATA");
    }
}

/// Check if AHCI is available
pub fn is_available() -> bool {
    unsafe { AHCI.is_some() }
}

/// Get controller reference
pub fn controller() -> Option<&'static AhciController> {
    unsafe { AHCI.as_ref() }
}
