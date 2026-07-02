//! xHCI USB host-controller driver (WP-04, epic E-20).
//!
//! - Step 1 (done): detection via the device model — match the PCI xHCI class and read the
//!   capability registers.
//! - Step 2 (this): controller **bring-up** — halt, reset, set the enabled device-slot count, and
//!   hand the controller its three DMA structures (Device Context Base Address Array, Command
//!   Ring, Event Ring + Event Ring Segment Table), then set Run and confirm it is running.
//!
//! DMA structures must have known physical addresses (the controller DMAs to them), so we
//! allocate physical frames from the kernel frame allocator and access them through the
//! bootloader's physical-memory offset mapping. Register access goes through `DeviceIo` (which
//! maps the controller's MMIO). All controller waits are bounded, so a wedged controller degrades
//! to a logged timeout rather than hanging the boot.

use spin::Mutex;
use x86_64::structures::paging::FrameAllocator;

use qos_driver::{BusKind, Device, DeviceId, DeviceIo, Driver, DriverError};

/// PCI classification for an xHCI controller: base 0x0C (serial bus), sub 0x03 (USB),
/// prog-if 0x30 (xHCI), packed the way `device::pci_class` builds `DeviceId::class`.
const PCI_CLASS_XHCI: u32 = 0x0C_03_30;

// Capability registers (offset from the MMIO/capability base).
const CAP_CAPLENGTH_HCIVERSION: u64 = 0x00;
const CAP_HCSPARAMS1: u64 = 0x04;
const CAP_HCCPARAMS1: u64 = 0x10; // bit 2 = CSZ (Context Size: 1 → 64-byte contexts)
const CAP_DBOFF: u64 = 0x14;
const CAP_RTSOFF: u64 = 0x18;

// Operational registers (offset from op base = cap base + CAPLENGTH).
const OP_USBCMD: u64 = 0x00;
const OP_USBSTS: u64 = 0x04;
const OP_CRCR: u64 = 0x18; // Command Ring Control (64-bit)
const OP_DCBAAP: u64 = 0x30; // Device Context Base Address Array Pointer (64-bit)
const OP_CONFIG: u64 = 0x38; // MaxSlotsEn in bits [7:0]

const USBCMD_RS: u32 = 1 << 0; // Run/Stop
const USBCMD_HCRST: u32 = 1 << 1; // Host Controller Reset
const USBCMD_INTE: u32 = 1 << 2; // Interrupter Enable (global)
const USBSTS_HCH: u32 = 1 << 0; // HCHalted
const USBSTS_EINT: u32 = 1 << 3; // Event Interrupt (RW1C)
const USBSTS_CNR: u32 = 1 << 11; // Controller Not Ready

// Interrupter 0 registers (offset from IR0 = runtime base + 0x20).
const IR0: u64 = 0x20;
const IR_IMAN: u64 = 0x00; // Interrupter Management (bit0 IP RW1C, bit1 IE)
const IR_ERSTSZ: u64 = 0x08; // Event Ring Segment Table Size
const IR_ERSTBA: u64 = 0x10; // Event Ring Segment Table Base Address (64-bit)
const IR_ERDP: u64 = 0x18; // Event Ring Dequeue Pointer (64-bit)

const IMAN_IP: u32 = 1 << 0; // Interrupt Pending (RW1C)
const IMAN_IE: u32 = 1 << 1; // Interrupt Enable
const ERDP_EHB: u64 = 1 << 3; // Event Handler Busy (write 1 to clear when advancing ERDP)

/// Entries in the command / event rings (TRBs are 16 bytes → 256 fit in one 4 KiB page).
const RING_TRBS: u32 = 256;

/// Brought-up controller state, kept for the later enumeration/HID steps.
pub struct Xhci {
    pub cap_base: u64,
    pub op_base: u64,
    pub runtime_base: u64,
    pub doorbell_base: u64,
    pub max_slots: u32,
    pub cmd_ring_phys: u64,
    pub cmd_ring_virt: u64,
    pub event_ring_phys: u64,
    pub event_ring_virt: u64,
    pub dcbaa_phys: u64,
    pub dcbaa_virt: u64,
    /// Device context size in bytes (32 or 64), read from HCCPARAMS1.CSZ — hardware-specific, so
    /// we never assume it.
    pub context_size: u64,
    // Command-ring producer state and event-ring consumer state (with their cycle bits).
    pub cmd_enqueue: usize,
    pub cmd_cycle: u32,
    pub event_dequeue: usize,
    pub event_cycle: u32,
    // EP0 (default control endpoint) transfer ring of the device currently being enumerated. Each
    // device is enumerated sequentially, so a single scratch EP0 ring is reused; it is not needed
    // after a device's interrupt endpoint is configured.
    pub ep0_ring_phys: u64,
    pub ep0_ring_virt: u64,
    pub ep0_enqueue: usize,
    pub ep0_cycle: u32,
    pub dev_slot: u32,
    // Root-hub port + PORTSC speed of the device being enumerated (needed to build the slot context
    // for Address Device / Configure Endpoint).
    pub dev_port: u32,
    pub dev_speed: u32,
    // All configured HID interrupt-IN endpoints (keyboard, mouse, …), polled from the main loop.
    pub hid_devices: alloc::vec::Vec<HidEndpoint>,
}

/// A configured HID interrupt-IN endpoint on some device slot, with its own transfer ring and a
/// single outstanding Normal TRB (`pending`) carrying the next boot report. `kind`: 1 = keyboard,
/// 2 = mouse.
pub struct HidEndpoint {
    pub slot: u32,
    pub dci: u32,
    pub kind: u8,
    pub ring_phys: u64,
    pub ring_virt: u64,
    pub enqueue: usize,
    pub cycle: u32,
    pub buf_phys: u64,
    pub buf_virt: u64,
    pub max_packet: u16,
    pub pending: bool,
    pub prev_report: [u8; 8],
}

/// TRB types we use.
const TRB_NORMAL: u32 = 1;
const TRB_SETUP_STAGE: u32 = 2;
const TRB_DATA_STAGE: u32 = 3;
const TRB_STATUS_STAGE: u32 = 4;
const TRB_LINK: u32 = 6;
const TRB_ENABLE_SLOT: u32 = 9;
const TRB_ADDRESS_DEVICE: u32 = 11;
const TRB_CONFIGURE_ENDPOINT: u32 = 12;
const TRB_TRANSFER_EVENT: u32 = 32;
const TRB_COMMAND_COMPLETION: u32 = 33;

/// The 18-byte USB device descriptor, as returned by GET_DESCRIPTOR.
#[derive(Clone, Copy, Default)]
pub struct DeviceDescriptor {
    pub usb_class: u8,
    pub max_packet0: u8,
    pub vendor: u16,
    pub product: u16,
}

impl Xhci {
    /// Enqueue a command TRB (4 dwords) and ring the command doorbell. `d3_extra` carries
    /// type-specific dword3 bits (e.g. the slot id in [31:24]); the TRB type and cycle bit are
    /// OR'd in. Single-command use — does not yet handle ring wrap / Link TRBs.
    fn submit_command(&mut self, d0: u32, d1: u32, d2: u32, d3_extra: u32, trb_type: u32, io: &mut dyn DeviceIo) {
        let trb = self.cmd_ring_virt + (self.cmd_enqueue * 16) as u64;
        unsafe {
            core::ptr::write_volatile(trb as *mut u32, d0);
            core::ptr::write_volatile((trb + 4) as *mut u32, d1);
            core::ptr::write_volatile((trb + 8) as *mut u32, d2);
            core::ptr::write_volatile((trb + 12) as *mut u32, d3_extra | (trb_type << 10) | self.cmd_cycle);
        }
        self.cmd_enqueue += 1;
        io.mmio_write32(self.doorbell_base, 0); // doorbell 0, DB target 0 = command ring
    }

    /// Wait for the next Command Completion Event, draining intervening events (e.g. port status
    /// changes). Returns the completion code and slot id, or `None` on timeout.
    fn wait_command_completion(&mut self, io: &mut dyn DeviceIo) -> Option<(u32, u32)> {
        for _ in 0..32 {
            match self.poll_event(io) {
                Some((TRB_COMMAND_COMPLETION, cc, slot)) => return Some((cc, slot)),
                Some(_) => continue,
                None => return None,
            }
        }
        None
    }

    /// Poll the event ring for the next event; return `(trb_type, completion_code, slot_id)` or
    /// `None` on timeout. Advances the dequeue pointer and updates ERDP.
    fn poll_event(&mut self, io: &mut dyn DeviceIo) -> Option<(u32, u32, u32)> {
        for _ in 0..200_000 {
            let evt = self.event_ring_virt + (self.event_dequeue * 16) as u64;
            let d3 = unsafe { core::ptr::read_volatile((evt + 12) as *const u32) };
            if (d3 & 1) == self.event_cycle {
                let d2 = unsafe { core::ptr::read_volatile((evt + 8) as *const u32) };
                let trb_type = (d3 >> 10) & 0x3F;
                let completion = (d2 >> 24) & 0xFF;
                let slot = (d3 >> 24) & 0xFF;
                self.event_dequeue += 1;
                if self.event_dequeue >= RING_TRBS as usize {
                    self.event_dequeue = 0;
                    self.event_cycle ^= 1;
                }
                let erdp = self.event_ring_phys + (self.event_dequeue * 16) as u64;
                read64_lo_hi_write(io, self.runtime_base + IR0 + IR_ERDP, erdp | ERDP_EHB);
                return Some((trb_type, completion, slot));
            }
            for _ in 0..200 {
                core::hint::spin_loop();
            }
        }
        None
    }

    /// Send an Enable Slot command and return the assigned slot id (WP-04 step 3b-2).
    fn enable_slot(&mut self, io: &mut dyn DeviceIo) -> Option<u32> {
        self.submit_command(0, 0, 0, 0, TRB_ENABLE_SLOT, io);
        match self.wait_command_completion(io) {
            Some((1, slot)) => Some(slot),
            Some((cc, _)) => {
                crate::serial_println!("[XHCI] enable-slot completion code {}", cc);
                None
            }
            None => {
                crate::serial_println!("[XHCI] enable-slot: timeout");
                None
            }
        }
    }

    /// Address a device on `slot` attached to root-hub `port` at `speed` (WP-04 step 3c-1). Builds
    /// the Input Context (control + slot + EP0 contexts) and the device's EP0 transfer ring,
    /// installs the Device Context in the DCBAA, and issues Address Device. On success the device
    /// responds to control transfers on EP0 (used by step 3c-2 to read descriptors).
    fn address_device(&mut self, slot: u32, port: u32, speed: u32, io: &mut dyn DeviceIo) -> bool {
        let cs = self.context_size;
        let (ic_phys, ic_virt) = match alloc_dma_page() {
            Some(v) => v,
            None => return false,
        };
        let (dc_phys, _) = match alloc_dma_page() {
            Some(v) => v,
            None => return false,
        };
        let (ep0_phys, ep0_virt) = match alloc_dma_page() {
            Some(v) => v,
            None => return false,
        };
        let wr = |off: u64, val: u32| unsafe { core::ptr::write_volatile((ic_virt + off) as *mut u32, val) };

        // Input Control Context: Add flags for slot (A0) and EP0 (A1).
        wr(0x04, 0b11);
        // Slot Context: speed [23:20], context entries=1 [31:27]; root-hub port [23:16].
        wr(cs, (speed << 20) | (1 << 27));
        wr(cs + 0x04, port << 16);
        // EP0 (control) context: CErr=3 [2:1], EPType=4 [5:3], MaxPacketSize [31:16]; TR dequeue
        // pointer + DCS; average TRB length.
        let mps: u32 = match speed {
            2 => 8,   // Low
            4 => 512, // SuperSpeed
            _ => 64,  // Full/High (Full starts at 8 but 64 works for QEMU; refined at descriptor)
        };
        wr(2 * cs + 0x04, (3 << 1) | (4 << 3) | (mps << 16));
        wr(2 * cs + 0x08, (ep0_phys as u32 & !0xF) | 1); // DCS=1
        wr(2 * cs + 0x0C, (ep0_phys >> 32) as u32);
        wr(2 * cs + 0x10, 8);

        // Install the (zeroed) Device Context pointer into DCBAA[slot].
        unsafe { core::ptr::write_volatile((self.dcbaa_virt + slot as u64 * 8) as *mut u64, dc_phys) };

        // Address Device command: input context pointer + slot id in dword3 [31:24].
        self.submit_command(ic_phys as u32, (ic_phys >> 32) as u32, 0, slot << 24, TRB_ADDRESS_DEVICE, io);
        match self.wait_command_completion(io) {
            Some((1, _)) => {
                self.ep0_ring_phys = ep0_phys;
                self.ep0_ring_virt = ep0_virt;
                self.ep0_enqueue = 0;
                self.ep0_cycle = 1;
                self.dev_slot = slot;
                self.dev_port = port;
                self.dev_speed = speed;
                true
            }
            Some((cc, _)) => {
                crate::serial_println!("[XHCI] address-device completion code {}", cc);
                false
            }
            None => {
                crate::serial_println!("[XHCI] address-device: timeout");
                false
            }
        }
    }

    /// Enqueue one TRB (4 dwords) onto the addressed device's EP0 transfer ring, OR-ing in the TRB
    /// type and the current cycle bit. Single-descriptor use — three TRBs fit well within one page,
    /// so no Link-TRB / wrap handling yet.
    fn ep0_enqueue_trb(&mut self, d0: u32, d1: u32, d2: u32, d3_extra: u32, trb_type: u32) {
        let trb = self.ep0_ring_virt + (self.ep0_enqueue * 16) as u64;
        unsafe {
            core::ptr::write_volatile(trb as *mut u32, d0);
            core::ptr::write_volatile((trb + 4) as *mut u32, d1);
            core::ptr::write_volatile((trb + 8) as *mut u32, d2);
            core::ptr::write_volatile((trb + 12) as *mut u32, d3_extra | (trb_type << 10) | self.ep0_cycle);
        }
        self.ep0_enqueue += 1;
    }

    /// Ring the device's EP0 doorbell (DB target 1 = the default control endpoint, DCI 1).
    fn ring_ep0_doorbell(&self, io: &mut dyn DeviceIo) {
        io.mmio_write32(self.doorbell_base + self.dev_slot as u64 * 4, 1);
    }

    /// Wait for the next Transfer Event, draining intervening events. Returns the completion code,
    /// or `None` on timeout.
    fn wait_transfer_completion(&mut self, io: &mut dyn DeviceIo) -> Option<u32> {
        for _ in 0..32 {
            match self.poll_event(io) {
                Some((TRB_TRANSFER_EVENT, cc, _)) => return Some(cc),
                Some(_) => continue,
                None => return None,
            }
        }
        None
    }

    /// Enumerate one enabled root-hub port fully (WP-04 steps 3b-2 → 4b): Enable Slot, Address
    /// Device, read the device descriptor, and — if it exposes a HID boot interrupt-IN endpoint —
    /// SET_CONFIGURATION, SET_PROTOCOL(boot), and Configure Endpoint (appended to `hid_devices`).
    /// A failure at any stage is logged and skips just this port, so one bad device can't wedge the
    /// rest.
    fn enumerate_port(&mut self, port: u32, speed: u32, io: &mut dyn DeviceIo) {
        let slot = match self.enable_slot(io) {
            Some(s) => s,
            None => return,
        };
        crate::serial_println!("[XHCI] Enable Slot -> slot id {} (port {})", slot, port);
        if !self.address_device(slot, port, speed, io) {
            return;
        }
        crate::serial_println!("[XHCI] Address Device OK: slot {} on port {} is addressed", slot, port);

        if let Some(d) = self.get_device_descriptor(io) {
            crate::serial_println!(
                "[XHCI] device descriptor: vendor={:#06x} product={:#06x} class={:#04x} bMaxPacketSize0={}",
                d.vendor, d.product, d.usb_class, d.max_packet0
            );
        }

        let hid = match self.get_hid_interface(io) {
            Some(h) => h,
            None => {
                crate::serial_println!("[XHCI] slot {}: no HID interrupt-IN endpoint", slot);
                return;
            }
        };
        let kind = match hid.protocol {
            1 => "keyboard",
            2 => "mouse",
            _ => "other",
        };
        crate::serial_println!(
            "[XHCI] HID {}: iface={} ep={:#04x} maxpkt={} interval={} config={}",
            kind, hid.interface, hid.ep_address, hid.ep_max_packet, hid.ep_interval, hid.config_value
        );
        let cfg_ok = self.set_configuration(io, hid.config_value);
        let proto_ok = self.set_boot_protocol(io, hid.interface as u16);
        if cfg_ok && proto_ok && self.configure_endpoint(io, &hid) {
            crate::serial_println!("[XHCI] HID {} ready on slot {} (DCI {})", kind, slot, (hid.ep_address as u32 & 0x0F) * 2 + 1);
        } else {
            crate::serial_println!("[XHCI] slot {}: HID setup failed (cfg={} proto={})", slot, cfg_ok, proto_ok);
        }
    }

    /// Perform a USB control transfer on EP0: a Setup stage, an optional Data stage (`length` > 0,
    /// direction from `dir_in`), and a Status stage in the opposite direction (with IOC so a
    /// Transfer Event is raised). `data_phys` is the physical address of the data buffer (ignored
    /// when `length == 0`). Returns the completion code, or `None` on timeout.
    fn control_transfer(
        &mut self,
        io: &mut dyn DeviceIo,
        req_type: u8,
        request: u8,
        value: u16,
        index: u16,
        data_phys: u64,
        length: u16,
        dir_in: bool,
    ) -> Option<u32> {
        // TRT (Transfer Type) in setup dword3 [17:16]: 0 = no data, 2 = OUT data, 3 = IN data.
        let trt: u32 = if length == 0 {
            0
        } else if dir_in {
            3
        } else {
            2
        };
        let setup_d0 = req_type as u32 | ((request as u32) << 8) | ((value as u32) << 16);
        let setup_d1 = index as u32 | ((length as u32) << 16);
        // Setup Stage: 8-byte Setup packet is immediate data (IDT, bit 6); always 8-byte length.
        self.ep0_enqueue_trb(setup_d0, setup_d1, 8, (1 << 6) | (trt << 16), TRB_SETUP_STAGE);
        // Data Stage (optional): DIR (bit 16) = 1 for IN.
        if length != 0 {
            let dir = if dir_in { 1u32 << 16 } else { 0 };
            self.ep0_enqueue_trb(data_phys as u32, (data_phys >> 32) as u32, length as u32, dir, TRB_DATA_STAGE);
        }
        // Status Stage: opposite direction of data (IN if there was no data stage), IOC (bit 5).
        let status_dir = if length != 0 && dir_in { 0u32 } else { 1u32 << 16 };
        self.ep0_enqueue_trb(0, 0, 0, status_dir | (1 << 5), TRB_STATUS_STAGE);

        self.ring_ep0_doorbell(io);
        self.wait_transfer_completion(io)
    }

    /// Read the 18-byte USB device descriptor (WP-04 step 3c-2) via a GET_DESCRIPTOR control
    /// transfer. Returns vendor/product/class/bMaxPacketSize0 on success.
    fn get_device_descriptor(&mut self, io: &mut dyn DeviceIo) -> Option<DeviceDescriptor> {
        let (buf_phys, buf_virt) = alloc_dma_page()?;
        // bmRequestType=0x80 (device→host, standard, device), bRequest=6 (GET_DESCRIPTOR),
        // wValue=0x0100 (device descriptor, index 0), wIndex=0, wLength=18.
        match self.control_transfer(io, 0x80, 6, 0x0100, 0, buf_phys, 18, true) {
            Some(1) => {
                let rd = |off: u64| unsafe { core::ptr::read_volatile((buf_virt + off) as *const u8) };
                let rd16 = |off: u64| (rd(off) as u16) | ((rd(off + 1) as u16) << 8);
                Some(DeviceDescriptor {
                    usb_class: rd(4),
                    max_packet0: rd(7),
                    vendor: rd16(8),
                    product: rd16(10),
                })
            }
            Some(cc) => {
                crate::serial_println!("[XHCI] get-descriptor completion code {}", cc);
                None
            }
            None => {
                crate::serial_println!("[XHCI] get-descriptor: timeout");
                None
            }
        }
    }

    /// Read the configuration descriptor and parse out the first HID interface's boot interrupt-IN
    /// endpoint (WP-04 step 4a). Returns the parsed `HidInterface` (config value, interface number,
    /// protocol, endpoint address / max-packet / interval), or `None`.
    fn get_hid_interface(&mut self, io: &mut dyn DeviceIo) -> Option<HidInterface> {
        let (buf_phys, buf_virt) = alloc_dma_page()?;
        // First fetch the 9-byte config descriptor header to learn wTotalLength.
        match self.control_transfer(io, 0x80, 6, 0x0200, 0, buf_phys, 9, true) {
            Some(1) => {}
            other => {
                crate::serial_println!("[XHCI] get-config(header) code {:?}", other);
                return None;
            }
        }
        let rd = |off: u64| unsafe { core::ptr::read_volatile((buf_virt + off) as *const u8) };
        let total_len = (rd(2) as u16) | ((rd(3) as u16) << 8);
        // Re-read the full configuration (bounded to one DMA page).
        let want = total_len.min(4096);
        match self.control_transfer(io, 0x80, 6, 0x0200, 0, buf_phys, want, true) {
            Some(1) => {}
            other => {
                crate::serial_println!("[XHCI] get-config(full) code {:?}", other);
                return None;
            }
        }
        parse_hid_interface(buf_virt, want as u64)
    }

    /// Issue SET_CONFIGURATION (standard, no data stage). Returns true on success.
    fn set_configuration(&mut self, io: &mut dyn DeviceIo, config: u8) -> bool {
        // bmRequestType=0x00 (host→device, standard, device), bRequest=9 (SET_CONFIGURATION).
        matches!(self.control_transfer(io, 0x00, 9, config as u16, 0, 0, 0, false), Some(1))
    }

    /// Issue the HID class request SET_PROTOCOL(boot=0) on `interface`. Returns true on success.
    fn set_boot_protocol(&mut self, io: &mut dyn DeviceIo, interface: u16) -> bool {
        // bmRequestType=0x21 (host→device, class, interface), bRequest=0x0B (SET_PROTOCOL),
        // wValue=0 (boot protocol).
        matches!(self.control_transfer(io, 0x21, 0x0B, 0, interface, 0, 0, false), Some(1))
    }

    /// Configure the HID interrupt-IN endpoint (WP-04 step 4b): build an Input Context that adds
    /// the slot context (updated Context Entries) and the interrupt endpoint, give the endpoint its
    /// own transfer ring (with a trailing Link TRB so it wraps), and issue Configure Endpoint. On
    /// success the endpoint is ready to be polled via [`poll_hid`].
    fn configure_endpoint(&mut self, io: &mut dyn DeviceIo, hid: &HidInterface) -> bool {
        let cs = self.context_size;
        // Device Context Index: EP number *2, +1 for an IN endpoint.
        let ep_num = (hid.ep_address & 0x0F) as u32;
        let dci = ep_num * 2 + 1;

        let (ic_phys, ic_virt) = match alloc_dma_page() {
            Some(v) => v,
            None => return false,
        };
        let (ep_ring_phys, ep_ring_virt) = match alloc_dma_page() {
            Some(v) => v,
            None => return false,
        };
        let (buf_phys, buf_virt) = match alloc_dma_page() {
            Some(v) => v,
            None => return false,
        };
        let wr = |off: u64, val: u32| unsafe { core::ptr::write_volatile((ic_virt + off) as *mut u32, val) };

        // Input Control Context: add the slot context (A0) and the endpoint context (A[dci]).
        wr(0x04, (1 << 0) | (1 << dci));
        // Slot Context (offset cs): keep speed + root-hub port, bump Context Entries to dci [31:27].
        wr(cs, (self.dev_speed << 20) | (dci << 27));
        wr(cs + 0x04, self.dev_port << 16);
        // Endpoint Context (offset (dci+1)*cs). EPType=7 (interrupt IN), CErr=3, MaxPacketSize; the
        // xHCI Interval field is bInterval-1 for high/super-speed interrupt endpoints.
        let ep_off = (dci as u64 + 1) * cs;
        let interval = hid.ep_interval.saturating_sub(1) as u32;
        wr(ep_off, interval << 16);
        wr(ep_off + 0x04, (3 << 1) | (7 << 3) | ((hid.ep_max_packet as u32) << 16));
        wr(ep_off + 0x08, (ep_ring_phys as u32 & !0xF) | 1); // TR dequeue ptr low + DCS=1
        wr(ep_off + 0x0C, (ep_ring_phys >> 32) as u32);
        wr(ep_off + 0x10, hid.ep_max_packet as u32); // Average TRB Length

        // Lay down a Link TRB in the last slot of the endpoint ring so the producer can wrap back to
        // the base (Toggle Cycle keeps the consumer cycle correct).
        let link = ep_ring_virt + ((RING_TRBS as u64 - 1) * 16);
        unsafe {
            core::ptr::write_volatile(link as *mut u32, ep_ring_phys as u32);
            core::ptr::write_volatile((link + 4) as *mut u32, (ep_ring_phys >> 32) as u32);
            core::ptr::write_volatile((link + 8) as *mut u32, 0);
            core::ptr::write_volatile((link + 12) as *mut u32, (TRB_LINK << 10) | (1 << 1) | 1); // TC=1, cycle=1
        }

        self.submit_command(ic_phys as u32, (ic_phys >> 32) as u32, 0, self.dev_slot << 24, TRB_CONFIGURE_ENDPOINT, io);
        match self.wait_command_completion(io) {
            Some((1, _)) => {
                self.hid_devices.push(HidEndpoint {
                    slot: self.dev_slot,
                    dci,
                    kind: hid.protocol,
                    ring_phys: ep_ring_phys,
                    ring_virt: ep_ring_virt,
                    enqueue: 0,
                    cycle: 1,
                    buf_phys,
                    buf_virt,
                    max_packet: hid.ep_max_packet,
                    pending: false,
                    prev_report: [0; 8],
                });
                true
            }
            Some((cc, _)) => {
                crate::serial_println!("[XHCI] configure-endpoint completion code {}", cc);
                false
            }
            None => {
                crate::serial_println!("[XHCI] configure-endpoint: timeout");
                false
            }
        }
    }

    /// Enable interrupt-driven event delivery (WP-04 step 5) via **MSI-X** — the universal path for
    /// PCIe devices (the message is a plain memory write to the local APIC, so it needs no ACPI
    /// interrupt-routing tables). Programs MSI-X table entry 0 to target this CPU's local APIC at
    /// `vector`, unmasks it, enables MSI-X in PCI config, then enables the controller's interrupter
    /// (IMAN.IE) and the global interrupt enable (USBCMD.INTE). Returns true on success; on any
    /// failure the driver keeps working via main-loop polling.
    fn enable_msix(&mut self, io: &mut dyn DeviceIo, vector: u8) -> bool {
        // Find our PCI device by matching its MMIO base to this controller's cap base.
        let pd = match crate::pci::find_by_class_subclass(0x0C, 0x03)
            .into_iter()
            .find(|pd| pci_bar_base(pd, 0) == self.cap_base)
        {
            Some(pd) => pd,
            None => {
                crate::serial_println!("[XHCI] MSI-X: PCI device not found");
                return false;
            }
        };
        let msix = match crate::pci::find_capability(&pd, 0x11) {
            Some(off) => off,
            None => {
                crate::serial_println!("[XHCI] MSI-X: capability absent");
                return false;
            }
        };
        let control = crate::pci::config_read16(&pd, msix + 2);
        let table_info = crate::pci::config_read32(&pd, msix + 4);
        let bir = (table_info & 0x7) as u8;
        let table_off = (table_info & !0x7) as u64;
        let table = pci_bar_base(&pd, bir) + table_off;

        // MSI-X table entry 0: message address = 0xFEE0_0000 | (apic_id << 12) (Fixed delivery to
        // this CPU), message data = vector, vector control bit 0 = 0 (unmasked).
        let apic_id = crate::apic::local_apic_id();
        io.mmio_write32(table, 0xFEE0_0000 | (apic_id << 12));
        io.mmio_write32(table + 4, 0);
        io.mmio_write32(table + 8, vector as u32);
        io.mmio_write32(table + 12, 0);

        // Enable MSI-X (control bit 15), clear the global function mask (bit 14).
        crate::pci::config_write16(&pd, msix + 2, (control | (1 << 15)) & !(1 << 14));

        // Clear any interrupt state left set by the events consumed during enumeration, so the next
        // event is a fresh 0→1 pending transition (MSI is edge-triggered). USBSTS.EINT and IMAN.IP
        // are both RW1C. Then enable interrupter 0 (IE) and the global interrupt enable.
        io.mmio_write32(self.op_base + OP_USBSTS, USBSTS_EINT);
        io.mmio_write32(self.runtime_base + IR0 + IR_IMAN, IMAN_IP | IMAN_IE);
        let cmd = io.mmio_read32(self.op_base + OP_USBCMD);
        io.mmio_write32(self.op_base + OP_USBCMD, cmd | USBCMD_INTE);

        // Publish the register bases so the bare ISR can acknowledge without taking the lock.
        XHCI_OP_BASE.store(self.op_base, core::sync::atomic::Ordering::SeqCst);
        XHCI_RUNTIME_BASE.store(self.runtime_base, core::sync::atomic::Ordering::SeqCst);
        crate::serial_println!(
            "[XHCI] MSI-X enabled: table @ {:#x} (BIR {}), vector {:#x}, apic {}",
            table, bir, vector, apic_id
        );
        true
    }

    /// Non-blocking check of the event ring: return the next event if its cycle bit is current, else
    /// `None`. Returns `(trb_type, completion_code, slot_id, endpoint_id)`. Advances the dequeue
    /// pointer + ERDP like [`poll_event`] but without spinning.
    fn try_event(&mut self, io: &mut dyn DeviceIo) -> Option<(u32, u32, u32, u32)> {
        let evt = self.event_ring_virt + (self.event_dequeue as u64 * 16);
        let d3 = unsafe { core::ptr::read_volatile((evt + 12) as *const u32) };
        if (d3 & 1) != self.event_cycle {
            return None;
        }
        let d2 = unsafe { core::ptr::read_volatile((evt + 8) as *const u32) };
        let trb_type = (d3 >> 10) & 0x3F;
        let completion = (d2 >> 24) & 0xFF;
        let slot = (d3 >> 24) & 0xFF;
        let endpoint = (d3 >> 16) & 0x1F; // Endpoint ID (= DCI) on a Transfer Event
        self.event_dequeue += 1;
        if self.event_dequeue >= RING_TRBS as usize {
            self.event_dequeue = 0;
            self.event_cycle ^= 1;
        }
        let erdp = self.event_ring_phys + (self.event_dequeue as u64 * 16);
        read64_lo_hi_write(io, self.runtime_base + IR0 + IR_ERDP, erdp | ERDP_EHB);
        Some((trb_type, completion, slot, endpoint))
    }

    /// Poll every configured HID endpoint (WP-04 step 4b), called from the kernel main loop. Drains
    /// completed transfer events (routing each to its device by slot+endpoint and translating the
    /// boot report into input), then keeps exactly one Normal TRB outstanding per endpoint.
    fn poll_hid(&mut self, io: &mut dyn DeviceIo) {
        if self.hid_devices.is_empty() {
            return;
        }
        let doorbell_base = self.doorbell_base;
        while let Some((trb_type, cc, slot, epid)) = self.try_event(io) {
            // cc 1 = Success, 13 = Short Packet (a full boot report is fine either way).
            if trb_type == TRB_TRANSFER_EVENT && (cc == 1 || cc == 13) {
                if let Some(dev) = self.hid_devices.iter_mut().find(|d| d.slot == slot && d.dci == epid) {
                    dev.process_report();
                    dev.pending = false;
                }
            }
        }
        for dev in self.hid_devices.iter_mut() {
            if !dev.pending {
                dev.queue_report(doorbell_base, io);
                dev.pending = true;
            }
        }
    }
}

impl HidEndpoint {
    /// Queue one Normal TRB (pointing at the report buffer) on this endpoint's ring and ring its
    /// doorbell. Handles wrapping via the trailing Link TRB. One TRB is kept outstanding at a time.
    fn queue_report(&mut self, doorbell_base: u64, io: &mut dyn DeviceIo) {
        let trb = self.ring_virt + (self.enqueue as u64 * 16);
        unsafe {
            core::ptr::write_volatile(trb as *mut u32, self.buf_phys as u32);
            core::ptr::write_volatile((trb + 4) as *mut u32, (self.buf_phys >> 32) as u32);
            core::ptr::write_volatile((trb + 8) as *mut u32, self.max_packet as u32);
            // Normal TRB: IOC (bit 5) + ISP (Interrupt on Short Packet, bit 2) + cycle.
            core::ptr::write_volatile((trb + 12) as *mut u32, (TRB_NORMAL << 10) | (1 << 5) | (1 << 2) | self.cycle);
        }
        self.enqueue += 1;
        // If the next slot is the Link TRB, refresh its cycle bit and wrap.
        if self.enqueue == RING_TRBS as usize - 1 {
            let link = self.ring_virt + ((RING_TRBS as u64 - 1) * 16);
            unsafe {
                core::ptr::write_volatile((link + 12) as *mut u32, (TRB_LINK << 10) | (1 << 1) | self.cycle);
            }
            self.enqueue = 0;
            self.cycle ^= 1;
        }
        // Ring the device doorbell with DB target = this endpoint's DCI.
        io.mmio_write32(doorbell_base + self.slot as u64 * 4, self.dci);
    }

    /// Translate the current boot report in this endpoint's buffer into input events and remember it
    /// for the next diff.
    fn process_report(&mut self) {
        let mut report = [0u8; 8];
        for (i, b) in report.iter_mut().enumerate() {
            *b = unsafe { core::ptr::read_volatile((self.buf_virt + i as u64) as *const u8) };
        }
        match self.kind {
            1 => self.process_keyboard_report(&report),
            2 => self.process_mouse_report(&report),
            _ => {}
        }
        self.prev_report = report;
    }

    /// Boot mouse report: byte 0 = button bitmap, byte 1 = dx, byte 2 = dy (signed). Emit relative
    /// motion and — by diffing byte 0 against the previous report — proper button **press and
    /// release** events (so clicks register and window drags end on release). HID +dy is "down"; the
    /// queue's convention is +dy = "up", so negate.
    fn process_mouse_report(&mut self, report: &[u8; 8]) {
        use crate::input::{self, InputEvent, MouseButton};
        let dx = report[1] as i8 as i16;
        let dy = report[2] as i8 as i16;
        if dx != 0 || dy != 0 {
            input::push(InputEvent::MouseMove { dx, dy: -dy });
        }
        let prev = self.prev_report[0];
        for (bit, button) in [(0, MouseButton::Left), (1, MouseButton::Right), (2, MouseButton::Middle)] {
            let now = report[0] & (1 << bit) != 0;
            let was = prev & (1 << bit) != 0;
            if now != was {
                input::push(InputEvent::MouseButton { button, pressed: now });
            }
        }
    }

    /// Boot keyboard report: byte 0 = modifier bitmap, bytes 2..8 = currently-pressed HID usage IDs.
    /// We diff against the previous report to synthesize PS/2 Set-1 make/break scancodes and feed
    /// them through the existing keyboard path (so both the legacy buffer and the unified queue see
    /// them, unchanged for consumers).
    fn process_keyboard_report(&mut self, report: &[u8; 8]) {
        let prev = self.prev_report;
        // Modifier bits → Set-1 make codes: LCtrl, LShift, LAlt, RShift (RCtrl/RAlt/GUI folded onto
        // the left equivalents for the boot shell).
        const MODS: [(u8, u8); 5] = [(0, 0x1D), (1, 0x2A), (2, 0x38), (4, 0x1D), (5, 0x36)];
        for &(bit, set1) in MODS.iter() {
            let now = report[0] & (1 << bit) != 0;
            let was = prev[0] & (1 << bit) != 0;
            if now && !was {
                crate::keyboard::push_scancode(set1);
            } else if !now && was {
                crate::keyboard::push_scancode(set1 | 0x80);
            }
        }
        // Newly pressed keys (present now, absent before).
        for &k in &report[2..8] {
            if k >= 4 && !prev[2..8].contains(&k) {
                if let Some(sc) = hid_to_set1(k) {
                    crate::keyboard::push_scancode(sc);
                }
            }
        }
        // Released keys (present before, absent now).
        for &k in &prev[2..8] {
            if k >= 4 && !report[2..8].contains(&k) {
                if let Some(sc) = hid_to_set1(k) {
                    crate::keyboard::push_scancode(sc | 0x80);
                }
            }
        }
    }
}

/// Map a USB HID keyboard usage ID (usage page 0x07) to a PS/2 Set-1 make scancode. Returns `None`
/// for keys we don't translate. This is the standard, device-independent HID→Set-1 mapping.
fn hid_to_set1(usage: u8) -> Option<u8> {
    let sc: u8 = match usage {
        0x04 => 0x1E, // a
        0x05 => 0x30, // b
        0x06 => 0x2E, // c
        0x07 => 0x20, // d
        0x08 => 0x12, // e
        0x09 => 0x21, // f
        0x0A => 0x22, // g
        0x0B => 0x23, // h
        0x0C => 0x17, // i
        0x0D => 0x24, // j
        0x0E => 0x25, // k
        0x0F => 0x26, // l
        0x10 => 0x32, // m
        0x11 => 0x31, // n
        0x12 => 0x18, // o
        0x13 => 0x19, // p
        0x14 => 0x10, // q
        0x15 => 0x13, // r
        0x16 => 0x1F, // s
        0x17 => 0x14, // t
        0x18 => 0x16, // u
        0x19 => 0x2F, // v
        0x1A => 0x11, // w
        0x1B => 0x2D, // x
        0x1C => 0x15, // y
        0x1D => 0x2C, // z
        0x1E => 0x02, // 1
        0x1F => 0x03, // 2
        0x20 => 0x04, // 3
        0x21 => 0x05, // 4
        0x22 => 0x06, // 5
        0x23 => 0x07, // 6
        0x24 => 0x08, // 7
        0x25 => 0x09, // 8
        0x26 => 0x0A, // 9
        0x27 => 0x0B, // 0
        0x28 => 0x1C, // Enter
        0x29 => 0x01, // Esc
        0x2A => 0x0E, // Backspace
        0x2B => 0x0F, // Tab
        0x2C => 0x39, // Space
        0x2D => 0x0C, // - _
        0x2E => 0x0D, // = +
        0x2F => 0x1A, // [ {
        0x30 => 0x1B, // ] }
        0x31 => 0x2B, // \ |
        0x33 => 0x27, // ; :
        0x34 => 0x28, // ' "
        0x35 => 0x29, // ` ~
        0x36 => 0x33, // , <
        0x37 => 0x34, // . >
        0x38 => 0x35, // / ?
        0x39 => 0x3A, // Caps Lock
        0x3A => 0x3B, // F1
        0x3B => 0x3C, // F2
        0x3C => 0x3D, // F3
        0x3D => 0x3E, // F4
        0x3E => 0x3F, // F5
        0x3F => 0x40, // F6
        0x40 => 0x41, // F7
        0x41 => 0x42, // F8
        0x42 => 0x43, // F9
        0x43 => 0x44, // F10
        0x44 => 0x57, // F11
        0x45 => 0x58, // F12
        // Arrows (extended in Set-1; we emit the base code — best-effort for the boot shell).
        0x4F => 0x4D, // Right
        0x50 => 0x4B, // Left
        0x51 => 0x50, // Down
        0x52 => 0x48, // Up
        _ => return None,
    };
    Some(sc)
}

/// A HID interface's boot interrupt-IN endpoint, parsed from the configuration descriptor.
#[derive(Clone, Copy, Debug, Default)]
pub struct HidInterface {
    pub config_value: u8,
    pub interface: u8,
    /// bInterfaceProtocol: 1 = keyboard, 2 = mouse (HID boot subclass).
    pub protocol: u8,
    /// bEndpointAddress of the interrupt-IN endpoint (bit 7 set = IN; low nibble = endpoint number).
    pub ep_address: u8,
    pub ep_max_packet: u16,
    pub ep_interval: u8,
}

/// Walk a configuration descriptor (`base` for `len` bytes), returning the first HID interface that
/// has an interrupt-IN endpoint. Descriptors are a chain of `[bLength, bDescriptorType, ...]`.
fn parse_hid_interface(base: u64, len: u64) -> Option<HidInterface> {
    let rd = |off: u64| unsafe { core::ptr::read_volatile((base + off) as *const u8) };
    let mut hid = HidInterface::default();
    // Config descriptor: bConfigurationValue is at offset 5.
    hid.config_value = rd(5);
    let mut off = 0u64;
    let mut in_hid_iface = false;
    while off + 2 <= len {
        let b_length = rd(off) as u64;
        let b_type = rd(off + 1);
        if b_length == 0 {
            break; // malformed — avoid an infinite loop
        }
        match b_type {
            0x04 => {
                // Interface descriptor: class @5, subclass @6, protocol @7.
                let iface_class = rd(off + 5);
                in_hid_iface = iface_class == 0x03; // HID
                if in_hid_iface {
                    hid.interface = rd(off + 2);
                    hid.protocol = rd(off + 7);
                }
            }
            0x05 if in_hid_iface => {
                // Endpoint descriptor: address @2, attributes @3, wMaxPacketSize @4, interval @6.
                let addr = rd(off + 2);
                let attr = rd(off + 3);
                if addr & 0x80 != 0 && attr & 0x03 == 0x03 {
                    // Interrupt (attr bits [1:0]=3) IN (bit 7) endpoint — the boot report endpoint.
                    hid.ep_address = addr;
                    hid.ep_max_packet = (rd(off + 4) as u16) | ((rd(off + 5) as u16) << 8);
                    hid.ep_interval = rd(off + 6);
                    return Some(hid);
                }
            }
            _ => {}
        }
        off += b_length;
    }
    None
}

/// The (single) xHCI controller, populated on successful bring-up.
pub static CONTROLLER: Mutex<Option<Xhci>> = Mutex::new(None);

/// Register bases published for the interrupt handler so it can acknowledge the controller without
/// taking the `CONTROLLER` lock (0 = interrupts not yet enabled).
static XHCI_OP_BASE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static XHCI_RUNTIME_BASE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Count of serviced xHCI interrupts (diagnostic / proof the IRQ path is live).
pub static IRQ_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Compute the base address of PCI BAR `index` for `pd`, combining the high dword for 64-bit memory
/// BARs (as [`crate::device::pci_resources`] does for BAR0).
fn pci_bar_base(pd: &crate::pci::PciDevice, index: u8) -> u64 {
    let off = 0x10 + index * 4;
    let lo = crate::pci::config_read32(pd, off);
    if lo & 1 == 0 {
        let base = (lo & !0xF) as u64;
        if (lo >> 1) & 0b11 == 0b10 {
            let hi = crate::pci::config_read32(pd, off + 4);
            return ((hi as u64) << 32) | base;
        }
        base
    } else {
        (lo & !0x3) as u64
    }
}

/// Poll the xHCI's configured HID endpoints for input (WP-04 step 4b). Called from the kernel main
/// loop; also the initial-queue path even when interrupts are enabled. No-op if no controller/HID
/// device is up. Uses `try_lock` so it never blocks the main loop.
pub fn poll() {
    if let Some(mut guard) = CONTROLLER.try_lock() {
        if let Some(ctrl) = guard.as_mut() {
            let mut io = crate::device::kernel_io();
            ctrl.poll_hid(&mut io);
        }
    }
}

/// xHCI interrupt service routine body (WP-04 step 5), called from the IDT stub. Acknowledges the
/// controller (clear USBSTS.EINT + IMAN.IP) using the published register bases — this happens
/// unconditionally so the interrupt always deasserts and can't storm — then drains the event ring
/// and re-queues reports if the controller lock is free (otherwise the main-loop `poll` will).
pub fn on_interrupt() {
    let op = XHCI_OP_BASE.load(core::sync::atomic::Ordering::SeqCst);
    let rt = XHCI_RUNTIME_BASE.load(core::sync::atomic::Ordering::SeqCst);
    if op == 0 {
        return;
    }
    IRQ_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let mut io = crate::device::kernel_io();
    // Acknowledge: clear the event-interrupt (USBSTS.EINT) and the interrupter pending bit
    // (IMAN.IP), keeping IMAN.IE set. Both are RW1C.
    io.mmio_write32(op + OP_USBSTS, USBSTS_EINT);
    io.mmio_write32(rt + IR0 + IR_IMAN, IMAN_IP | IMAN_IE);
    // Drain + re-queue if we can grab the lock without blocking the ISR.
    if let Some(mut guard) = CONTROLLER.try_lock() {
        if let Some(ctrl) = guard.as_mut() {
            ctrl.poll_hid(&mut io);
        }
    }
}

/// Count configured HID devices by kind: `(keyboards, mice)`. For the System Monitor app.
pub fn hid_device_counts() -> (usize, usize) {
    if let Some(guard) = CONTROLLER.try_lock() {
        if let Some(ctrl) = guard.as_ref() {
            let kbd = ctrl.hid_devices.iter().filter(|d| d.kind == 1).count();
            let mouse = ctrl.hid_devices.iter().filter(|d| d.kind == 2).count();
            return (kbd, mouse);
        }
    }
    (0, 0)
}

fn read64_lo_hi_write(io: &mut dyn DeviceIo, addr: u64, val: u64) {
    io.mmio_write32(addr, val as u32);
    io.mmio_write32(addr + 4, (val >> 32) as u32);
}

/// Poll `addr` until `(reg & mask) != 0` equals `want_set`, up to a bounded number of tries.
fn wait_bit(io: &mut dyn DeviceIo, addr: u64, mask: u32, want_set: bool) -> bool {
    for _ in 0..100_000 {
        if ((io.mmio_read32(addr) & mask) != 0) == want_set {
            return true;
        }
        for _ in 0..200 {
            core::hint::spin_loop();
        }
    }
    false
}

/// Allocate one zeroed 4 KiB physical frame for a DMA structure; return `(phys, virt)`.
fn alloc_dma_page() -> Option<(u64, u64)> {
    let frame = crate::memory::with_ctx(|_, fa| fa.allocate_frame())?;
    let phys = frame.start_address().as_u64();
    let virt = crate::memory::phys_offset().as_u64() + phys;
    unsafe {
        core::ptr::write_bytes(virt as *mut u8, 0, 4096);
    }
    Some((phys, virt))
}

/// Run the xHCI bring-up sequence. Returns the controller state or a static error string.
fn bring_up(cap_base: u64, io: &mut dyn DeviceIo) -> Result<Xhci, &'static str> {
    let cap0 = io.mmio_read32(cap_base + CAP_CAPLENGTH_HCIVERSION);
    let cap_length = (cap0 & 0xFF) as u64;
    let op_base = cap_base + cap_length;
    let runtime_base = cap_base + (io.mmio_read32(cap_base + CAP_RTSOFF) as u64 & !0x1F);
    let doorbell_base = cap_base + (io.mmio_read32(cap_base + CAP_DBOFF) as u64 & !0x3);
    let max_slots = io.mmio_read32(cap_base + CAP_HCSPARAMS1) & 0xFF;

    // Wait for the controller to be ready (CNR clear).
    if !wait_bit(io, op_base + OP_USBSTS, USBSTS_CNR, false) {
        return Err("controller not ready (CNR stuck)");
    }
    // Halt, then reset.
    let cmd = io.mmio_read32(op_base + OP_USBCMD);
    io.mmio_write32(op_base + OP_USBCMD, cmd & !USBCMD_RS);
    if !wait_bit(io, op_base + OP_USBSTS, USBSTS_HCH, true) {
        return Err("controller did not halt");
    }
    io.mmio_write32(op_base + OP_USBCMD, USBCMD_HCRST);
    if !wait_bit(io, op_base + OP_USBCMD, USBCMD_HCRST, false) {
        return Err("reset did not clear");
    }
    if !wait_bit(io, op_base + OP_USBSTS, USBSTS_CNR, false) {
        return Err("controller not ready after reset");
    }

    // Enable all device slots the controller supports.
    io.mmio_write32(op_base + OP_CONFIG, max_slots);

    // Context size (32 or 64 bytes) from HCCPARAMS1.CSZ — read, never assumed.
    let context_size = if io.mmio_read32(cap_base + CAP_HCCPARAMS1) & (1 << 2) != 0 { 64 } else { 32 };

    // Device Context Base Address Array (one page: 256 × 64-bit pointers, zeroed).
    let (dcbaa_phys, dcbaa_virt) = alloc_dma_page().ok_or("no frame for DCBAA")?;
    read64_lo_hi_write(io, op_base + OP_DCBAAP, dcbaa_phys);

    // Command Ring (one page of TRBs). Program CRCR with the ring base + Ring Cycle State = 1.
    let (cmd_ring_phys, cmd_ring_virt) = alloc_dma_page().ok_or("no frame for command ring")?;
    read64_lo_hi_write(io, op_base + OP_CRCR, cmd_ring_phys | 1);

    // Event Ring: one segment (a page of TRBs) described by a one-entry Event Ring Segment Table.
    let (event_ring_phys, event_ring_virt) = alloc_dma_page().ok_or("no frame for event ring")?;
    let (erst_phys, erst_virt) = alloc_dma_page().ok_or("no frame for ERST")?;
    unsafe {
        // ERST entry 0: ring segment base address (64-bit) then ring segment size (u16 in u32).
        core::ptr::write_volatile(erst_virt as *mut u64, event_ring_phys);
        core::ptr::write_volatile((erst_virt + 8) as *mut u32, RING_TRBS);
    }
    let ir0 = runtime_base + IR0;
    io.mmio_write32(ir0 + IR_ERSTSZ, 1);
    read64_lo_hi_write(io, ir0 + IR_ERDP, event_ring_phys);
    read64_lo_hi_write(io, ir0 + IR_ERSTBA, erst_phys);

    // Run.
    let cmd = io.mmio_read32(op_base + OP_USBCMD);
    io.mmio_write32(op_base + OP_USBCMD, cmd | USBCMD_RS);
    if !wait_bit(io, op_base + OP_USBSTS, USBSTS_HCH, false) {
        return Err("controller did not start running");
    }

    Ok(Xhci {
        cap_base,
        op_base,
        runtime_base,
        doorbell_base,
        max_slots,
        cmd_ring_phys,
        cmd_ring_virt,
        event_ring_phys,
        event_ring_virt,
        dcbaa_phys,
        dcbaa_virt,
        context_size,
        cmd_enqueue: 0,
        cmd_cycle: 1,
        event_dequeue: 0,
        event_cycle: 1,
        ep0_ring_phys: 0,
        ep0_ring_virt: 0,
        ep0_enqueue: 0,
        ep0_cycle: 1,
        dev_slot: 0,
        dev_port: 0,
        dev_speed: 0,
        hid_devices: alloc::vec::Vec::new(),
    })
}

/// Human name for the xHCI PORTSC port-speed id (bits [13:10]).
fn speed_name(speed: u32) -> &'static str {
    match speed {
        1 => "Full",
        2 => "Low",
        3 => "High",
        4 => "SuperSpeed",
        5 => "SuperSpeedPlus",
        _ => "?",
    }
}

// PORTSC bits.
const PORTSC_CCS: u32 = 1 << 0; // Current Connect Status (RO)
const PORTSC_PED: u32 = 1 << 1; // Port Enabled/Disabled (RW1C to disable)
const PORTSC_PR: u32 = 1 << 4; // Port Reset
const PORTSC_PP: u32 = 1 << 9; // Port Power
const PORTSC_PRC: u32 = 1 << 21; // Port Reset Change (RW1C)

fn portsc_addr(op_base: u64, port: u32) -> u64 {
    op_base + 0x400 + (port as u64 - 1) * 0x10
}

/// Reset one root-hub port and return whether it ended up enabled (WP-04 step 3b-1). Writing only
/// `PP | PR` keeps the RW1C status-change bits (and PED, which is RW1C-to-disable) untouched, so
/// we don't accidentally clear/disable anything. USB3 ports auto-enable after reset; USB2 need it.
fn reset_port(op_base: u64, port: u32, io: &mut dyn DeviceIo) -> bool {
    let addr = portsc_addr(op_base, port);
    io.mmio_write32(addr, PORTSC_PP | PORTSC_PR);
    // Hardware clears PR when the reset completes.
    if !wait_bit(io, addr, PORTSC_PR, false) {
        return false;
    }
    let v = io.mmio_read32(addr);
    // Acknowledge the reset-change bit (RW1C), preserving power.
    io.mmio_write32(addr, PORTSC_PP | PORTSC_PRC);
    v & PORTSC_PED != 0
}

/// Scan + reset the root-hub ports (WP-04 step 3): detect connected ports (3a) and reset each so
/// it becomes enabled (3b-1). Logs the result; returns every enabled `(port, speed)` so all
/// attached devices (keyboard, mouse, …) can be enumerated — the count is discovered, not assumed.
fn scan_ports(op_base: u64, max_ports: u32, io: &mut dyn DeviceIo) -> alloc::vec::Vec<(u32, u32)> {
    let mut connected = 0;
    let mut enabled = alloc::vec::Vec::new();
    for port in 1..=max_ports {
        let portsc = io.mmio_read32(portsc_addr(op_base, port));
        if portsc & PORTSC_CCS != 0 {
            connected += 1;
            let speed = (portsc >> 10) & 0xF;
            let is_enabled = reset_port(op_base, port, io);
            if is_enabled {
                enabled.push((port, speed));
            }
            crate::serial_println!(
                "[XHCI] port {}: device connected (speed {} {}), after reset enabled={}",
                port, speed, speed_name(speed), is_enabled
            );
        }
    }
    crate::serial_println!(
        "[XHCI] ports: {} connected, {} enabled (of {})",
        connected, enabled.len(), max_ports
    );
    enabled
}

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
        let cap_length = cap0 & 0xFF;
        let hci_version = (cap0 >> 16) & 0xFFFF;
        let hcs1 = io.mmio_read32(base + CAP_HCSPARAMS1);
        let (max_slots, max_ports) = (hcs1 & 0xFF, (hcs1 >> 24) & 0xFF);
        if cap_length == 0 || max_ports == 0 {
            return Err(DriverError::Unsupported);
        }
        crate::serial_println!(
            "[XHCI] controller @ {:#x}: HCIVERSION={:#06x} CAPLENGTH={} slots={} ports={}",
            base, hci_version, cap_length, max_slots, max_ports
        );

        // Step 2: bring the controller up.
        match bring_up(base, io) {
            Ok(mut ctrl) => {
                crate::serial_println!(
                    "[XHCI] running: op={:#x} runtime={:#x} doorbell={:#x} slots_enabled={}",
                    ctrl.op_base, ctrl.runtime_base, ctrl.doorbell_base, ctrl.max_slots
                );
                // Step 3a/3b-1: reset connected ports so they enable, then enumerate every enabled
                // one (keyboard, mouse, …) — the device count is discovered from the hardware.
                for (port, speed) in scan_ports(ctrl.op_base, max_ports, io) {
                    ctrl.enumerate_port(port, speed, io);
                }
                crate::serial_println!("[XHCI] {} HID device(s) ready", ctrl.hid_devices.len());
                // Step 5: switch input from main-loop polling to interrupt-driven via MSI-X.
                if !ctrl.hid_devices.is_empty() {
                    ctrl.enable_msix(io, crate::interrupts::XHCI_VECTOR);
                }
                *CONTROLLER.lock() = Some(ctrl);
                Ok(())
            }
            Err(e) => {
                crate::serial_println!("[XHCI] bring-up failed: {}", e);
                Err(DriverError::Io)
            }
        }
    }
}
