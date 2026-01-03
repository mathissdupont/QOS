//! Intel E1000 Network Interface Controller Driver
//!
//! Supports Intel 82540EM (QEMU default NIC) and compatible devices.
//! Uses MMIO for register access.

use crate::pci::{self, PciDevice};
use crate::net::{MacAddr, EthHeader, eth_type};
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use spin::Mutex;
use core::ptr::{read_volatile, write_volatile};

// E1000 Vendor/Device IDs
const INTEL_VENDOR: u16 = 0x8086;
const E1000_82540EM: u16 = 0x100E;   // QEMU default
const E1000_82545EM: u16 = 0x100F;
const E1000_82574L: u16 = 0x10D3;

// E1000 Register Offsets
mod reg {
    pub const CTRL: u32 = 0x0000;      // Device Control
    pub const STATUS: u32 = 0x0008;    // Device Status
    pub const EERD: u32 = 0x0014;      // EEPROM Read
    pub const ICR: u32 = 0x00C0;       // Interrupt Cause Read
    pub const IMS: u32 = 0x00D0;       // Interrupt Mask Set
    pub const IMC: u32 = 0x00D8;       // Interrupt Mask Clear
    pub const RCTL: u32 = 0x0100;      // Receive Control
    pub const TCTL: u32 = 0x0400;      // Transmit Control
    pub const RDBAL: u32 = 0x2800;     // RX Descriptor Base Low
    pub const RDBAH: u32 = 0x2804;     // RX Descriptor Base High
    pub const RDLEN: u32 = 0x2808;     // RX Descriptor Length
    pub const RDH: u32 = 0x2810;       // RX Descriptor Head
    pub const RDT: u32 = 0x2818;       // RX Descriptor Tail
    pub const TDBAL: u32 = 0x3800;     // TX Descriptor Base Low
    pub const TDBAH: u32 = 0x3804;     // TX Descriptor Base High
    pub const TDLEN: u32 = 0x3808;     // TX Descriptor Length
    pub const TDH: u32 = 0x3810;       // TX Descriptor Head
    pub const TDT: u32 = 0x3818;       // TX Descriptor Tail
    pub const RAL0: u32 = 0x5400;      // Receive Address Low
    pub const RAH0: u32 = 0x5404;      // Receive Address High
    pub const MTA: u32 = 0x5200;       // Multicast Table Array (128 entries)
}

// Control Register bits
mod ctrl {
    pub const SLU: u32 = 1 << 6;       // Set Link Up
    pub const RST: u32 = 1 << 26;      // Device Reset
}

// Status Register bits
mod status {
    pub const LU: u32 = 1 << 1;        // Link Up
}

// Receive Control bits
mod rctl {
    pub const EN: u32 = 1 << 1;        // Receiver Enable
    pub const SBP: u32 = 1 << 2;       // Store Bad Packets
    pub const UPE: u32 = 1 << 3;       // Unicast Promiscuous
    pub const MPE: u32 = 1 << 4;       // Multicast Promiscuous
    pub const LBM_NONE: u32 = 0 << 6;  // No Loopback
    pub const BAM: u32 = 1 << 15;      // Broadcast Accept
    pub const BSIZE_2048: u32 = 0 << 16;
    pub const BSIZE_1024: u32 = 1 << 16;
    pub const BSIZE_512: u32 = 2 << 16;
    pub const BSIZE_256: u32 = 3 << 16;
    pub const SECRC: u32 = 1 << 26;    // Strip Ethernet CRC
}

// Transmit Control bits
mod tctl {
    pub const EN: u32 = 1 << 1;        // Transmitter Enable
    pub const PSP: u32 = 1 << 3;       // Pad Short Packets
    pub const CT_SHIFT: u32 = 4;       // Collision Threshold
    pub const COLD_SHIFT: u32 = 12;    // Collision Distance
}

// Interrupt bits
mod int {
    pub const TXDW: u32 = 1 << 0;      // TX Descriptor Written Back
    pub const TXQE: u32 = 1 << 1;      // TX Queue Empty
    pub const LSC: u32 = 1 << 2;       // Link Status Change
    pub const RXDMT0: u32 = 1 << 4;    // RX Descriptor Min Threshold
    pub const RXO: u32 = 1 << 6;       // RX Overrun
    pub const RXT0: u32 = 1 << 7;      // RX Timer Interrupt
}

/// Number of descriptors in TX/RX rings
const NUM_RX_DESC: usize = 32;
const NUM_TX_DESC: usize = 32;
const RX_BUFFER_SIZE: usize = 2048;

/// Receive Descriptor
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct RxDesc {
    addr: u64,      // Buffer address
    length: u16,    // Packet length
    checksum: u16,  // Packet checksum
    status: u8,     // Status bits
    errors: u8,     // Error bits
    special: u16,   // VLAN tag
}

impl RxDesc {
    const fn zero() -> Self {
        Self {
            addr: 0,
            length: 0,
            checksum: 0,
            status: 0,
            errors: 0,
            special: 0,
        }
    }
}

// RX Status bits
const RXDESC_DD: u8 = 1 << 0;      // Descriptor Done
const RXDESC_EOP: u8 = 1 << 1;     // End of Packet

/// Transmit Descriptor
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct TxDesc {
    addr: u64,      // Buffer address
    length: u16,    // Data length
    cso: u8,        // Checksum offset
    cmd: u8,        // Command bits
    status: u8,     // Status bits
    css: u8,        // Checksum start
    special: u16,   // VLAN tag
}

impl TxDesc {
    const fn zero() -> Self {
        Self {
            addr: 0,
            length: 0,
            cso: 0,
            cmd: 0,
            status: 0,
            css: 0,
            special: 0,
        }
    }
}

// TX Command bits
const TXCMD_EOP: u8 = 1 << 0;      // End of Packet
const TXCMD_IFCS: u8 = 1 << 1;     // Insert FCS
const TXCMD_RS: u8 = 1 << 3;       // Report Status

// TX Status bits
const TXDESC_DD: u8 = 1 << 0;      // Descriptor Done

/// E1000 NIC driver
pub struct E1000 {
    pci_device: PciDevice,
    mmio_base: usize,
    mac_addr: MacAddr,
    rx_descs: &'static mut [RxDesc; NUM_RX_DESC],
    tx_descs: &'static mut [TxDesc; NUM_TX_DESC],
    rx_buffers: [[u8; RX_BUFFER_SIZE]; NUM_RX_DESC],
    tx_buffers: [[u8; RX_BUFFER_SIZE]; NUM_TX_DESC],
    rx_cur: usize,
    tx_cur: usize,
}

/// Global E1000 instance
static E1000_NIC: Mutex<Option<E1000>> = Mutex::new(None);

/// Received packets queue
static RX_QUEUE: Mutex<VecDeque<Vec<u8>>> = Mutex::new(VecDeque::new());

impl E1000 {
    /// Read a 32-bit register
    fn read_reg(&self, offset: u32) -> u32 {
        unsafe {
            read_volatile((self.mmio_base + offset as usize) as *const u32)
        }
    }

    /// Write a 32-bit register
    fn write_reg(&self, offset: u32, value: u32) {
        unsafe {
            write_volatile((self.mmio_base + offset as usize) as *mut u32, value);
        }
    }

    /// Read MAC address from EEPROM
    fn read_mac_from_eeprom(&self) -> MacAddr {
        let mut mac = [0u8; 6];
        
        for i in 0..3 {
            // Start EEPROM read
            self.write_reg(reg::EERD, 1 | ((i as u32) << 8));
            
            // Wait for completion (poll done bit)
            let mut val = 0u32;
            for _ in 0..1000 {
                val = self.read_reg(reg::EERD);
                if val & (1 << 4) != 0 {
                    break;
                }
            }
            
            let data = (val >> 16) as u16;
            mac[i * 2] = (data & 0xFF) as u8;
            mac[i * 2 + 1] = (data >> 8) as u8;
        }
        
        MacAddr(mac)
    }

    /// Read MAC from RAL/RAH registers (fallback)
    fn read_mac_from_ral(&self) -> MacAddr {
        let low = self.read_reg(reg::RAL0);
        let high = self.read_reg(reg::RAH0);
        
        MacAddr([
            (low & 0xFF) as u8,
            ((low >> 8) & 0xFF) as u8,
            ((low >> 16) & 0xFF) as u8,
            ((low >> 24) & 0xFF) as u8,
            (high & 0xFF) as u8,
            ((high >> 8) & 0xFF) as u8,
        ])
    }

    /// Initialize receive ring
    fn init_rx(&mut self) {
        // Allocate RX descriptors (already done in struct)
        let rx_desc_phys = self.rx_descs.as_ptr() as u64;
        
        // Set up each RX descriptor
        for i in 0..NUM_RX_DESC {
            self.rx_descs[i].addr = self.rx_buffers[i].as_ptr() as u64;
            self.rx_descs[i].status = 0;
        }
        
        // Set descriptor base address
        self.write_reg(reg::RDBAL, (rx_desc_phys & 0xFFFFFFFF) as u32);
        self.write_reg(reg::RDBAH, (rx_desc_phys >> 32) as u32);
        
        // Set descriptor ring length
        self.write_reg(reg::RDLEN, (NUM_RX_DESC * core::mem::size_of::<RxDesc>()) as u32);
        
        // Set head and tail
        self.write_reg(reg::RDH, 0);
        self.write_reg(reg::RDT, (NUM_RX_DESC - 1) as u32);
        
        // Enable receiver
        self.write_reg(reg::RCTL, 
            rctl::EN | rctl::BAM | rctl::BSIZE_2048 | rctl::SECRC
        );
    }

    /// Initialize transmit ring
    fn init_tx(&mut self) {
        let tx_desc_phys = self.tx_descs.as_ptr() as u64;
        
        // Clear TX descriptors
        for i in 0..NUM_TX_DESC {
            self.tx_descs[i] = TxDesc::zero();
        }
        
        // Set descriptor base address
        self.write_reg(reg::TDBAL, (tx_desc_phys & 0xFFFFFFFF) as u32);
        self.write_reg(reg::TDBAH, (tx_desc_phys >> 32) as u32);
        
        // Set descriptor ring length
        self.write_reg(reg::TDLEN, (NUM_TX_DESC * core::mem::size_of::<TxDesc>()) as u32);
        
        // Set head and tail
        self.write_reg(reg::TDH, 0);
        self.write_reg(reg::TDT, 0);
        
        // Enable transmitter
        self.write_reg(reg::TCTL, 
            tctl::EN | tctl::PSP | 
            (15 << tctl::CT_SHIFT) |   // Collision threshold
            (64 << tctl::COLD_SHIFT)   // Collision distance
        );
    }

    /// Send a packet
    pub fn send(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if data.len() > RX_BUFFER_SIZE {
            return Err("packet too large");
        }
        
        let cur = self.tx_cur;
        
        // Wait for descriptor to be available
        if self.tx_descs[cur].status & TXDESC_DD == 0 && self.tx_descs[cur].cmd != 0 {
            // Descriptor still in use
            return Err("tx ring full");
        }
        
        // Copy data to buffer
        self.tx_buffers[cur][..data.len()].copy_from_slice(data);
        
        // Set up descriptor
        self.tx_descs[cur].addr = self.tx_buffers[cur].as_ptr() as u64;
        self.tx_descs[cur].length = data.len() as u16;
        self.tx_descs[cur].cmd = TXCMD_EOP | TXCMD_IFCS | TXCMD_RS;
        self.tx_descs[cur].status = 0;
        
        // Update tail
        self.tx_cur = (cur + 1) % NUM_TX_DESC;
        self.write_reg(reg::TDT, self.tx_cur as u32);
        
        Ok(())
    }

    /// Poll for received packets
    pub fn poll_rx(&mut self) {
        loop {
            let cur = self.rx_cur;
            
            // Check if descriptor has data
            if self.rx_descs[cur].status & RXDESC_DD == 0 {
                break;
            }
            
            // Get packet length
            let length = self.rx_descs[cur].length as usize;
            
            if length > 0 && length <= RX_BUFFER_SIZE {
                // Copy packet to queue
                let packet = self.rx_buffers[cur][..length].to_vec();
                RX_QUEUE.lock().push_back(packet);
            }
            
            // Reset descriptor
            self.rx_descs[cur].status = 0;
            
            // Update tail (give descriptor back to hardware)
            let old_tail = self.read_reg(reg::RDT);
            self.write_reg(reg::RDT, cur as u32);
            
            // Move to next descriptor
            self.rx_cur = (cur + 1) % NUM_RX_DESC;
        }
    }

    /// Check link status
    pub fn is_link_up(&self) -> bool {
        self.read_reg(reg::STATUS) & status::LU != 0
    }

    /// Get MAC address
    pub fn mac_addr(&self) -> MacAddr {
        self.mac_addr
    }
}

/// Initialize the E1000 driver
pub fn init() -> Result<(), &'static str> {
    // Find E1000 device
    let devices = pci::find_by_id(INTEL_VENDOR, E1000_82540EM);
    
    let device = if !devices.is_empty() {
        devices[0]
    } else {
        // Try other E1000 variants
        let alt = pci::find_by_id(INTEL_VENDOR, E1000_82545EM);
        if !alt.is_empty() {
            alt[0]
        } else {
            // Check for any network device
            let net_devs = pci::find_by_class(0x02);
            if net_devs.is_empty() {
                crate::serial_println!("[E1000] No network device found");
                return Err("no network device");
            }
            crate::serial_println!("[E1000] Found network device {:04x}:{:04x}, not E1000",
                net_devs[0].vendor_id, net_devs[0].device_id);
            return Err("no E1000 device");
        }
    };
    
    crate::serial_println!("[E1000] Found device at {:02x}:{:02x}.{:x}",
        device.bus, device.device, device.function);
    
    // Get MMIO base from BAR0
    let bar0 = device.bar0;
    if bar0 & 1 != 0 {
        return Err("E1000 uses I/O ports, not MMIO");
    }
    
    let mmio_base = (bar0 & !0xF) as usize;
    crate::serial_println!("[E1000] MMIO base: 0x{:08x}", mmio_base);
    
    // Map MMIO region using bootloader's physical memory offset
    // The MMIO BAR0 is a physical address - we use the offset mapping
    let mmio_virt = crate::memory::mmio_virt_addr(mmio_base as u64);
    let mmio_base_virt = mmio_virt.as_u64() as usize;
    crate::serial_println!("[E1000] MMIO virtual: 0x{:016x}", mmio_base_virt);
    
    // Allocate descriptor arrays (simplified - using static allocation)
    // In a real OS, these would be allocated from DMA-capable memory
    static mut RX_DESCS: [RxDesc; NUM_RX_DESC] = [RxDesc::zero(); NUM_RX_DESC];
    static mut TX_DESCS: [TxDesc; NUM_TX_DESC] = [TxDesc::zero(); NUM_TX_DESC];
    static mut RX_BUFS: [[u8; RX_BUFFER_SIZE]; NUM_RX_DESC] = [[0; RX_BUFFER_SIZE]; NUM_RX_DESC];
    static mut TX_BUFS: [[u8; RX_BUFFER_SIZE]; NUM_TX_DESC] = [[0; RX_BUFFER_SIZE]; NUM_TX_DESC];
    
    let mut nic = E1000 {
        pci_device: device,
        mmio_base: mmio_base_virt,
        mac_addr: MacAddr::ZERO,
        rx_descs: unsafe { &mut RX_DESCS },
        tx_descs: unsafe { &mut TX_DESCS },
        rx_buffers: unsafe { RX_BUFS },
        tx_buffers: unsafe { TX_BUFS },
        rx_cur: 0,
        tx_cur: 0,
    };
    
    // Reset device
    nic.write_reg(reg::CTRL, ctrl::RST);
    for _ in 0..10000 {
        if nic.read_reg(reg::CTRL) & ctrl::RST == 0 {
            break;
        }
    }
    
    // Set link up
    let ctrl_val = nic.read_reg(reg::CTRL);
    nic.write_reg(reg::CTRL, ctrl_val | ctrl::SLU);
    
    // Disable interrupts (we'll use polling)
    nic.write_reg(reg::IMC, 0xFFFFFFFF);
    
    // Clear multicast table
    for i in 0..128 {
        nic.write_reg(reg::MTA + i * 4, 0);
    }
    
    // Read MAC address
    nic.mac_addr = nic.read_mac_from_eeprom();
    if nic.mac_addr == MacAddr::ZERO || nic.mac_addr.0[0] == 0xFF {
        // Fallback to RAL/RAH
        nic.mac_addr = nic.read_mac_from_ral();
    }
    
    crate::serial_println!("[E1000] MAC address: {}", nic.mac_addr);
    
    // Initialize TX/RX rings
    nic.init_rx();
    nic.init_tx();
    
    // Check link status
    if nic.is_link_up() {
        crate::serial_println!("[E1000] Link is UP");
    } else {
        crate::serial_println!("[E1000] Link is DOWN");
    }
    
    *E1000_NIC.lock() = Some(nic);
    crate::serial_println!("[E1000] Initialized");
    
    Ok(())
}

/// Check if E1000 is available
pub fn is_available() -> bool {
    E1000_NIC.lock().is_some()
}

/// Get MAC address
pub fn mac_addr() -> Option<MacAddr> {
    E1000_NIC.lock().as_ref().map(|n| n.mac_addr)
}

/// Check link status
pub fn is_link_up() -> bool {
    E1000_NIC.lock().as_ref().map(|n| n.is_link_up()).unwrap_or(false)
}

/// Send a raw Ethernet frame
pub fn send(data: &[u8]) -> Result<(), &'static str> {
    E1000_NIC.lock().as_mut().ok_or("E1000 not initialized")?.send(data)
}

/// Poll for received packets
pub fn poll() {
    if let Some(ref mut nic) = *E1000_NIC.lock() {
        nic.poll_rx();
    }
}

/// Get a received packet
pub fn recv() -> Option<Vec<u8>> {
    poll();
    RX_QUEUE.lock().pop_front()
}

/// Get number of received packets waiting
pub fn rx_pending() -> usize {
    RX_QUEUE.lock().len()
}
