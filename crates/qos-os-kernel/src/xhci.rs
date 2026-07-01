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
const USBSTS_HCH: u32 = 1 << 0; // HCHalted
const USBSTS_CNR: u32 = 1 << 11; // Controller Not Ready

// Interrupter 0 registers (offset from IR0 = runtime base + 0x20).
const IR0: u64 = 0x20;
const IR_ERSTSZ: u64 = 0x08; // Event Ring Segment Table Size
const IR_ERSTBA: u64 = 0x10; // Event Ring Segment Table Base Address (64-bit)
const IR_ERDP: u64 = 0x18; // Event Ring Dequeue Pointer (64-bit)

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
    // Command-ring producer state and event-ring consumer state (with their cycle bits).
    pub cmd_enqueue: usize,
    pub cmd_cycle: u32,
    pub event_dequeue: usize,
    pub event_cycle: u32,
}

/// TRB types we use.
const TRB_ENABLE_SLOT: u32 = 9;
const TRB_COMMAND_COMPLETION: u32 = 33;

impl Xhci {
    /// Enqueue a command TRB (4 dwords; the cycle bit is OR'd into dword3) and ring the command
    /// doorbell. Single-command use — does not yet handle ring wrap / Link TRBs.
    fn submit_command(&mut self, d0: u32, d1: u32, d2: u32, d3_type: u32, io: &mut dyn DeviceIo) {
        let trb = self.cmd_ring_virt + (self.cmd_enqueue * 16) as u64;
        unsafe {
            core::ptr::write_volatile(trb as *mut u32, d0);
            core::ptr::write_volatile((trb + 4) as *mut u32, d1);
            core::ptr::write_volatile((trb + 8) as *mut u32, d2);
            core::ptr::write_volatile((trb + 12) as *mut u32, (d3_type << 10) | self.cmd_cycle);
        }
        self.cmd_enqueue += 1;
        io.mmio_write32(self.doorbell_base, 0); // doorbell 0, DB target 0 = command ring
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
                read64_lo_hi_write(io, self.runtime_base + IR0 + IR_ERDP, erdp);
                return Some((trb_type, completion, slot));
            }
            for _ in 0..200 {
                core::hint::spin_loop();
            }
        }
        None
    }

    /// Send an Enable Slot command and return the assigned slot id (WP-04 step 3b-2). Proves the
    /// command ring, doorbell, and event ring all work end-to-end. The port resets done earlier
    /// leave Port Status Change Events (type 34) queued ahead of our Command Completion Event
    /// (type 33), so we drain intervening events until the completion arrives.
    fn enable_slot(&mut self, io: &mut dyn DeviceIo) -> Option<u32> {
        self.submit_command(0, 0, 0, TRB_ENABLE_SLOT, io);
        for _ in 0..32 {
            match self.poll_event(io) {
                Some((TRB_COMMAND_COMPLETION, cc, slot)) => {
                    if cc == 1 {
                        return Some(slot);
                    }
                    crate::serial_println!("[XHCI] enable-slot completion code {}", cc);
                    return None;
                }
                // Drain port-status-change (34) and any other non-completion events.
                Some(_) => continue,
                None => {
                    crate::serial_println!("[XHCI] enable-slot: no completion event (timeout)");
                    return None;
                }
            }
        }
        None
    }
}

/// The (single) xHCI controller, populated on successful bring-up.
pub static CONTROLLER: Mutex<Option<Xhci>> = Mutex::new(None);

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

    // Device Context Base Address Array (one page: 256 × 64-bit pointers, zeroed).
    let (dcbaa_phys, _) = alloc_dma_page().ok_or("no frame for DCBAA")?;
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
        cmd_enqueue: 0,
        cmd_cycle: 1,
        event_dequeue: 0,
        event_cycle: 1,
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
/// it becomes enabled (3b-1). Logs the result; returns the number of enabled ports.
fn scan_ports(op_base: u64, max_ports: u32, io: &mut dyn DeviceIo) -> u32 {
    let mut connected = 0;
    let mut enabled_count = 0;
    for port in 1..=max_ports {
        let portsc = io.mmio_read32(portsc_addr(op_base, port));
        if portsc & PORTSC_CCS != 0 {
            connected += 1;
            let speed = (portsc >> 10) & 0xF;
            let enabled = reset_port(op_base, port, io);
            if enabled {
                enabled_count += 1;
            }
            crate::serial_println!(
                "[XHCI] port {}: device connected (speed {} {}), after reset enabled={}",
                port, speed, speed_name(speed), enabled
            );
        }
    }
    crate::serial_println!(
        "[XHCI] ports: {} connected, {} enabled (of {})",
        connected, enabled_count, max_ports
    );
    enabled_count
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
                // Step 3a: reset connected ports so they enable.
                let enabled = scan_ports(ctrl.op_base, max_ports, io);
                // Step 3b-2: prove the command/event-ring machinery by requesting a device slot.
                if enabled > 0 {
                    match ctrl.enable_slot(io) {
                        Some(slot) => crate::serial_println!("[XHCI] Enable Slot -> slot id {}", slot),
                        None => crate::serial_println!("[XHCI] Enable Slot failed"),
                    }
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
