# WP-04: USB host controller + HID input

- Status: 🟡 in progress
- Epic: E-20 (USB core/xHCI), E-21 (USB HID)
- ADRs: ADR-0015 (modern hardware), ADR-0016 (driver model)
- Commits: (this WP, appended as delivered)

## Goal

Give QOS working **USB keyboard and mouse** — the single biggest unblocker for usability on real
modern laptops, which have no PS/2. Built on the driver model (WP-02) and the APIC (WP-03).

## Scope

- In: xHCI host controller (the modern USB controller), USB device enumeration for HID, the HID
  boot protocol for keyboard + mouse, feeding the existing unified input event queue.
- Out (defer): USB hubs beyond the root, mass storage (separate WP), full (non-boot) HID report
  descriptors, USB 1.x companion controllers (UHCI/EHCI) — xHCI presents all speeds.

## Steps

- [x] **Step 1 — Detect the xHCI controller.** Match PCI class 0x0C/0x03 via the driver model;
  read the capability registers (CAPLENGTH, HCIVERSION, HCSPARAMS1 → max ports/slots) and log.
- [x] **Step 2 — Controller bring-up.** Reset the controller; set up the Device Context Base
  Address Array, the Command Ring, the Event Ring, and the run/stop + interrupter registers; run.
- [x] **Step 3 — Port + device enumeration.** (3a) detect ports; (3b-1) reset to enable; (3b-2)
  command/event ring via Enable Slot → slot id; (3c-1) Input/Slot/EP0 contexts + EP0 ring +
  Device Context + **Address Device** → device addressed; (3c-2) **GET_DESCRIPTOR** via a control
  transfer on EP0 → vendor/product/class/bMaxPacketSize0. Context size read from HCCPARAMS1.CSZ
  (never assumed).
- [ ] **Step 4 — HID boot protocol.** Set boot protocol; poll/interrupt the keyboard and mouse
  endpoints; translate HID reports into unified `InputEvent`s.
- [ ] **Step 5 — Interrupt-driven.** Route the xHCI interrupt (MSI, or its IO-APIC GSI) so input
  is event-driven rather than polled. (Needs PCIe MSI, E-12, or the IO-APIC line.)

## Acceptance criteria

Ultimately: with `-device qemu-xhci -device usb-kbd -device usb-mouse` (and later real hardware),
typing on the USB keyboard reaches the shell/desktop and the USB mouse moves the cursor, with the
PS/2 path absent. Per-step: the boot log shows the controller detected and each bring-up stage
succeeding.

## Progress log

- **Step 1 done.** xHCI controller detected via the driver model and its caps read: in
  QEMU+OVMF (q35, `-device qemu-xhci -device usb-kbd -device usb-mouse`) →
  `HCIVERSION=0x0100 CAPLENGTH=64 slots=64 ports=8` at MMIO `0xc000000000`; bound `1b36:000d` →
  `xhci`. Found and fixed a real bug on the way (see below).
- **Steps 2, 3a, 3b done.** Controller brought up (`op=0xc000000040 runtime=0xc000001000
  doorbell=0xc000002000 slots_enabled=64`); ports scanned/reset (`port 5,6: High, enabled`);
  Enable Slot → `slot id 1`; Address Device OK (`slot 1 on port 5 is addressed`).
- **Step 3c-2 done.** GET_DESCRIPTOR control transfer on EP0 (Setup→Data-IN→Status-OUT + EP0
  doorbell, wait Transfer Event cc=1) →
  `device descriptor: vendor=0x0627 product=0x0001 class=0x00 bMaxPacketSize0=64`. Values read
  from the device, 0 faults, boot reaches `QaOS ready`.

## Continuation notes (technical — resume here)

State: `xhci.rs` has a `Xhci` controller in `CONTROLLER: Mutex<Option<Xhci>>`, brought up, with one
device **addressed** and its **device descriptor read**. Helpers now exist for both the command
ring (`submit_command`, `wait_command_completion`) and the EP0 transfer ring
(`ep0_enqueue_trb(d0,d1,d2,d3_extra,type)`, `ring_ep0_doorbell(io)`, `wait_transfer_completion(io)`,
and the full `get_device_descriptor(io) -> DeviceDescriptor`). `poll_event → (type,cc,slot)` drains
both Command Completion (33) and Transfer (32) events. MMIO via `io: &mut dyn DeviceIo`; DMA pages
via `alloc_dma_page() -> (phys, virt)`; `read64_lo_hi_write` for 64-bit regs. Fields to reuse:
`ep0_ring_virt/phys`, `ep0_enqueue`, `ep0_cycle`, `dev_slot`, `context_size`, `dcbaa_virt`.

**Step 4 — HID boot protocol (resume here).** Reuse the EP0 control-transfer path
(`ep0_enqueue_trb` + `ring_ep0_doorbell` + `wait_transfer_completion`) for the standard requests:
1. GET_DESCRIPTOR(configuration, wLength large enough) → parse for the HID **interrupt-IN**
   endpoint: bEndpointAddress, wMaxPacketSize, bInterval, and the interface number/bNumConfigs.
2. SET_CONFIGURATION(bConfigurationValue) (control transfer, no data stage).
3. SET_PROTOCOL(boot=0) on the HID interface (`bmRequestType=0x21, bRequest=0x0B, wValue=0`).
4. Configure that endpoint: build an Input Context adding the interrupt-IN EP (EPType=7 IN,
   its own transfer ring), issue a **Configure Endpoint** command (TRB type 12).
5. Queue a Normal TRB (type 1) with an 8-byte DMA buffer on that endpoint's ring, ring its doorbell
   (DB target = the endpoint's DCI), and on the Transfer Event read the boot report (byte0
   modifiers, bytes2–7 keycodes for keyboard; 3-byte for mouse). Translate to
   `crate::input::InputEvent` and push to the queue. Poll for now; interrupt-drive in Step 5.

**Step 5 — interrupt-driven.** Enable the xHCI interrupter (IMAN) and route its IRQ (the PCI
`interrupt_line` via the IO-APIC, or MSI once E-12 lands) instead of polling.

## Notes & gaps

- **Fixed: 64-bit PCI BARs.** `device::pci_resources` used BAR0 alone, so the xHCI's 64-bit BAR
  (real base `0xc000000000`, high dword in BAR1) resolved to base 0. Now combines BAR1 for
  64-bit memory BARs. (Also confirmed the bootloader's physical-memory offset mapping reaches
  high MMIO above 4 GiB, so we can access the controller.)
- **Gap:** BAR *size* probing isn't implemented (resource `len` is 0). Needed before we remap or
  bounds-check MMIO precisely; fine for fixed-offset register access now.
- xHCI is a large controller; this WP is intentionally multi-step. Each step is boot-verified.
- Interrupt wiring (Step 5) may pull in PCIe MSI (E-12); until then the driver can poll the event
  ring to prove function.
