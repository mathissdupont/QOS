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
- [x] **Step 4 — HID boot protocol.** (4a) parse config → HID interrupt-IN endpoint,
  SET_CONFIGURATION, SET_PROTOCOL(boot). (4b) Configure Endpoint (own transfer ring + Link TRB),
  poll the endpoint from the kernel main loop, translate boot reports: keyboard HID usages →
  PS/2 Set-1 scancodes via `keyboard::push_scancode` (feeds both the legacy buffer and the unified
  queue, so consumers are unchanged); mouse reports → `InputEvent::MouseMove`/`MouseButton`.
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
- **Step 4 done (keyboard).** 4a: parsed the config descriptor →
  `HID keyboard: iface=0 ep=0x81 maxpkt=8 interval=7 config=1`, `set-config=true
  set-boot-protocol=true`. 4b: Configure Endpoint → `HID keyboard ready: endpoint DCI 3`; the
  main loop polls the endpoint and translates reports. Verified by driving QEMU monitor `sendkey`
  (usb-kbd now handled by us): typing `hello\n` produced the exact HID→Set-1 sequence
  (`0x0b→0x23 h`, `0x08→0x12 e`, `0x0f→0x26 l`×2, `0x12→0x18 o`, `0x28→0x1c Enter`), pushed via
  `keyboard::push_scancode` — the same path PS/2 uses, so it reaches the shell unchanged. 0 faults.

## Continuation notes (technical — resume here)

State: `xhci.rs` has a `Xhci` in `CONTROLLER: Mutex<Option<Xhci>>`, brought up, with one device
**addressed**, its **descriptor read**, and its **HID interrupt-IN endpoint configured + polled**.
Helpers: command ring (`submit_command`, `wait_command_completion`), EP0 control transfers
(`control_transfer(io, req_type, request, value, index, data_phys, length, dir_in)`,
`get_device_descriptor`, `get_hid_interface`, `set_configuration`, `set_boot_protocol`), endpoint
config + polling (`configure_endpoint(io,&HidInterface)`, `hid_queue_report`, `try_event`,
`poll_hid`, `process_report` → `process_keyboard_report`/`process_mouse_report`, `hid_to_set1`).
Module-level `xhci::poll()` is called from `runtime.rs` main loop. MMIO via
`crate::device::kernel_io()`. Fields to reuse: `dev_slot/dev_port/dev_speed`, `hid_*`, `context_size`.

**Step 5 — interrupt-driven (resume here).** Replace main-loop polling with interrupt delivery:
enable the xHCI interrupter (IMAN.IE + USBCMD.INTE), and either (a) route the controller's PCI
`interrupt_line` through the IO-APIC to a vector whose ISR calls into `poll_hid`, or (b) implement
PCIe MSI (epic E-12) and use that. On the interrupt, drain the event ring and re-queue the report
TRB. Keep `poll()` as a fallback.

**Multi-device — done.** Enumeration now loops over **every** enabled port (`enumerate_port`),
giving each device its own slot + interrupt endpoint, stored in `hid_devices: Vec<HidEndpoint>`.
`poll_hid` drains the shared event ring and routes each Transfer Event to its device by
`(slot, endpoint id)`. Verified in QEMU: `2 HID device(s) ready` (slot 1 keyboard on port 5, slot 2
mouse on port 6); `sendkey` produces keystrokes and `mouse_move 15 8` produces
`MouseMove{dx:15,dy:8}` events. 0 faults. Port/device count is discovered from the hardware, never
assumed.

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
