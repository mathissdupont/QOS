# ADR-0018: Modern persistent storage via AHCI/SATA

- **Status:** Accepted
- **Date:** 2026-07-02
- **Deciders:** QOS core
- **Related ADRs:** ADR-0015 (modern hardware & UI, fallback-first), ADR-0016 (device driver model)

## Context

QOS needs **persistent** storage: today the user-facing filesystem (`fs`, the RAM fs backing the
Files GUI, the Text Editor and the Terminal shell) lives entirely in RAM and is lost on reboot. A
`diskfs` module (a tiny QOSFS block format) already exists, but it is hardwired to the legacy
**ATA PIO** driver (`ata::AtaPio`), which talks to the ISA-era task-file ports `0x1F0/0x3F6`.

Verified facts:

- QOS boots on **UEFI + q35** (ADR-0014). The q35 machine has **no legacy IDE** at `0x1F0`; it
  exposes an **ICH9 AHCI/SATA** controller (seen in PCI enumeration as `8086:2922`,
  class `01:06:01`). Real modern hardware is the same: SATA (AHCI) or NVMe, never ISA IDE.
- The legacy ATA PIO path therefore only works on the older i440fx `pc` machine — switching to it
  to get persistence would move QOS *backwards* on hardware modernity, contradicting the WP-05
  guidance to keep **modern hardware compatibility** in mind.
- The kernel already has the DMA/MMIO primitives a bus-master controller needs: a physical-frame
  allocator (`memory::with_ctx(|_, fa| fa.allocate_frame())`), the bootloader's full
  physical-memory offset map (`memory::phys_offset()` / `map_mmio`), and PCI config read/write
  (`pci::config_read32` / `config_write16`). The xHCI driver (WP-04) already uses this pattern.

## Decision

**Implement an AHCI (SATA) block driver and run `diskfs` over it, keeping the RAM `fs` as the
default and the AHCI disk as opt-in/attached storage (fallback-first).**

- A new `ahci` module discovers the AHCI HBA via PCI (class `0x01`, subclass `0x06`, prog-IF
  `0x01`), enables memory-space + bus-master, maps the ABAR (BAR5) through the phys-offset, resets
  the HBA, scans the implemented ports for an attached SATA device, and does IDENTIFY + DMA
  read/write of 512-byte sectors (command list + command table with a Register-H2D FIS + PRDT, all
  in DMA-allocated frames).
- `diskfs` is decoupled from `ata` behind a minimal block interface so it can sit on AHCI.
- QEMU attaches a dedicated **persistent SATA data disk** on its own AHCI HBA (a raw image under
  `dist/`), separate from the boot disk, so data survives reboots and never risks the boot volume.

## Rationale

- **Modernity:** AHCI/SATA is the actual modern standard and matches what q35 (and real machines)
  expose; no regression to legacy ISA IDE or the older `pc` machine.
- **Reuse:** the DMA/MMIO/PCI plumbing from xHCI applies directly, so the incremental cost is the
  AHCI protocol itself, not new infrastructure.
- **Safety (fallback-first, ADR-0015):** the RAM fs keeps working with zero disk present, so a
  missing/again-unformatted disk never breaks boot or the UI; persistence is additive.
- **Performance:** AHCI is DMA (bus-master), not PIO word-by-word, so large transfers don't burn
  CPU in `in`/`out` loops — better than the legacy ATA path we are replacing.

## Consequences

### Positive

- Real persistent storage on modern hardware; files created in Files / the Text Editor / the shell
  can be committed to a disk that survives reboots.
- One more real, driver-based subsystem (a genuine OS trait), reusing the WP-04 DMA model.

### Negative / Trade-offs

- AHCI bring-up (HBA reset, port start, FIS/command-list layout, PRDT) is intricate and needs
  careful QEMU verification per step.
- Two filesystems coexist (RAM `fs` + on-disk `diskfs`) until a unifying VFS mount is built; the UI
  must make clear which is which.

### Neutral / Follow-ups

- Later: a VFS mount so the disk appears as a directory under the same tree; write-back/caching;
  NVMe as an even more modern backend; crash-safety for QOSFS.
- Keep the legacy `ata` module as a compile-time fallback for the `pc` machine until AHCI is proven.

## Alternatives considered

1. **Legacy ATA PIO on the `pc` machine** — works today but is not modern hardware (no ISA IDE on
   q35 or real UEFI machines) and is slow PIO; rejected as a regression.
2. **NVMe** — more modern than AHCI but a larger driver (submission/completion queues, MSI-X); a
   good future backend, but AHCI is the smaller step that already matches the q35 controller.
3. **Stay RAM-only** — no persistence; fails the user's explicit "can it access the hard disk" need.
