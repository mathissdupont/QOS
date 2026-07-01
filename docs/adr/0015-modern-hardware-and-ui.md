# ADR-0015: Target modern hardware (USB HID, APIC/SMP, virtio/NVMe) and a modern UI

- Status: Accepted
- Date: 2026-07-01
- Related ADRs: ADR-0012 (desktop/UX layer), ADR-0013 (graphics path), ADR-0014 (UEFI + framebuffer)

## Context

QOS now boots via UEFI on a linear framebuffer (ADR-0014) and presents a graphical desktop.
But its device and CPU support are the *bring-up defaults* of a hobby kernel — the pieces that
are simplest to program and that every emulator provides — not what a modern PC actually
exposes:

- **Input: PS/2 keyboard/mouse only.** Most modern laptops have no PS/2 controller (or only a
  firmware emulation that disappears under a native USB stack), so on real hardware the desktop
  can appear with **no working keyboard or mouse**.
- **Interrupts/timing: legacy 8259 PIC + 8254 PIT.** Modern systems route interrupts through the
  local APIC + IO-APIC; the PIC is often emulated but is a dead end for SMP and for many modern
  devices (incl. USB controllers) whose interrupts are wired through the IO-APIC/MSI.
- **CPU: single core.** APs are never started; the local-APIC timer is unused. Modern machines
  are multi-core and the scheduler cannot use them.
- **NIC: Intel E1000 only.** Fine in QEMU/VirtualBox; absent on most real machines and on
  virtio-based VMs.
- **Storage: no USB mass storage / NVMe; AHCI disabled.** Real disks are unreachable.
- **UI:** the desktop is authored on a fixed 320×200 logical canvas with an 8×8 bitmap font,
  scaled up. It is legible but not a *modern* UI (no scalable/antialiased fonts, no
  double-buffered compositor, minimal widget set, no HiDPI awareness).

These were never recorded as decisions; this ADR makes the modernization an explicit,
architected direction.

## Decision

**Target modern UEFI x86-64 PCs and mainstream VMs (QEMU/KVM, VirtualBox, VMware, Hyper-V).**
Adopt a phased hardware- and UI-modernization program. Each phase is independently shippable and
QEMU-verifiable, and each **keeps the legacy driver as a graceful fallback** (probe modern first,
fall back to PS/2/E1000/PIC when the modern path is absent), so the system never regresses on the
platforms that work today.

### Phases (dependency-ordered)

- **A — APIC & ACPI foundation.** Parse ACPI (RSDP → MADT). Bring up the local APIC and IO-APIC;
  mask the PIC. Move the tick to the local-APIC timer (calibrated), keeping the PIT as fallback.
  Prerequisite for USB interrupts and SMP.
- **B — USB HID input.** An xHCI host-controller driver + minimal USB core (device enumeration,
  control/interrupt transfers) + the HID boot protocol for keyboard and mouse. Feeds the existing
  unified input event queue, so the shell/desktop consume it unchanged. **The single biggest
  unblocker for real laptops.**
- **C — SMP.** Start application processors (MADT + INIT/SIPI), per-CPU GDT/TSS/IDT, and make the
  scheduler multi-core aware.
- **D — Modern I/O.** virtio-net + virtio-blk (modern VMs) and NVMe (real SSDs); wire block
  devices into the VFS. Keep E1000 as fallback.
- **E — Modern UI.** Scalable, antialiased font rendering (an embedded TrueType/bitmap-vector
  font); a double-buffered compositor (draw off-screen, blit once — removes flicker and the
  save-under cursor hack); a real widget toolkit (buttons, text fields, lists, scrollbars) with a
  theme; HiDPI/native-resolution layout instead of integer-scaling a 320×200 canvas.

### Non-goals

- Exotic/rare devices, 32-bit, or non-x86-64 architectures.
- A driver ecosystem rivalling a production OS. The bar is "boots and is usable on a mainstream
  modern PC/VM with keyboard, mouse, display, and network."

## Rationale

- APIC is the true foundation: USB, SMP, and modern timing all depend on it, so it goes first.
- USB HID is the difference between "the desktop appears" and "the desktop is usable" on real
  hardware, so it is the top device priority after the APIC prerequisite.
- Fallback-first probing means each phase is additive and low-risk: QEMU's PS/2/E1000/PIC paths
  keep working while the modern paths are validated.
- A compositor + scalable fonts is what makes the UI read as *modern* rather than retro; it also
  simplifies the rendering model (one off-screen surface, blit once) that ADR-0014's facade
  already points toward.

## Consequences

- **Positive:** a realistic path to running usably on modern PCs and VMs; each phase is testable
  in QEMU (q35 exposes xHCI, IO-APIC, SMP, virtio, NVMe) before real-hardware validation
  (ADR-0014 Stage 4).
- **Negative / risks:** each phase is a substantial subsystem (USB and SMP especially). Scope is
  controlled by targeting only the modern-mainstream device set and keeping legacy fallbacks.
- **Follow-ups:** individual phases may warrant their own ADRs as designs firm up (e.g. an xHCI
  driver ADR, an SMP scheduling ADR, a UI-toolkit ADR). The quantum control-plane track
  (ADR-0008 transpilation, ADR-0010 mitigation) proceeds independently and is not blocked by this.
```
