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
- [ ] **Step 3 — Port + device enumeration.** Detect connected ports, reset them, assign slots
  and addresses via control transfers; read device/config descriptors.
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
