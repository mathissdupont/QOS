# QOS Work Packages (WP)

A **work package** is a concrete, shippable unit of work: a scope, a task list, acceptance
criteria, and a status. WPs are how we execute the epics in [`../MASTERPLAN.md`] step by step.

- **ADRs** (`../adr/`) record *decisions* — why we chose an approach.
- **WPs** (this directory) record *work* — what we're building, the steps, and whether it's done.
- Each WP links to its epic (E-xx in the master plan), any ADRs, and the commits that delivered it.

Use [`template.md`](template.md) for new WPs. Statuses: 🔴 not started · 🟡 in progress · ✅ done.

## Register

| WP | Title | Epic | ADR | Status |
|----|-------|------|-----|--------|
| [WP-01](WP-01-uefi-boot-and-desktop.md) | UEFI boot repair + framebuffer desktop | E-70 (seed) | ADR-0014 | ✅ done |
| [WP-02](WP-02-driver-model.md) | Device/driver model | E-01 | ADR-0016 | ✅ done |
| [WP-03](WP-03-acpi-apic.md) | ACPI + modern APIC interrupts | E-10 | ADR-0015 | ✅ done |
| [WP-04](WP-04-usb-input.md) | USB host controller + HID input | E-20, E-21 | ADR-0015 | 🟡 in progress |

Upcoming (not yet opened as WP files; see the master-plan critical path):
PCIe ECAM + MSI (E-12) · SMP (E-11) · virtio/NVMe + block/FS (E-24/23/40/41) · modern UI
compositor (E-70) · quantum transpilation (E-80).

## Gaps & correctness backlog

Things noticed as missing, fragile, or wrong while building — captured here so a "massive, proper
OS" doesn't accrete silent debt. Each should become (or fold into) a WP.

- **G-01** Legacy `acpi.rs` still scans BIOS memory for the RSDP and uses raw physical pointers —
  broken under UEFI; only its shutdown/reboot port logic is still wanted. Should be reworked to
  use the bootloader RSDP + the `qos-acpi` walk (partially superseded by WP-03). *(open)*
- **G-02** `git add -A` on WP-03 swept the transpiler's `Topology::shortest_path`/`is_all_to_all`
  (quantum E-80) into an unrelated commit. Harmless (complete, will be used), but note the E-80
  transpiler work is started and uncommitted-in-spirit. *(open)*
- **G-03** Interrupt handlers (keyboard/mouse/timer) are not yet driver-model citizens; they are
  still hand-wired in `interrupts.rs`. Once the driver model owns IRQs (post-USB), migrate them.
  *(open)*
- **G-04** No `lsdev`/`lspci`-style shell command yet surfaces the device model to the user; it
  only logs at boot. Small, user-visible; fold into a UI/shell WP. *(open)*
- **G-05** Single BSP only; the APIC timer is per-CPU but we start no APs. Blocks true multi-core
  (E-11 SMP). *(open)*
- **G-06** BIOS boot image is disabled; UEFI-only. Fine for modern targets but note it. *(open)*
- **G-07** TLS is a non-functional stub → no real HTTPS/cloud reachability (ADR-0011 proxy).
  Blocks cloud-QPU and secure egress. *(open)*
- **G-08** No real-hardware validation yet (ADR-0014 Stage 4); everything is QEMU+OVMF verified.
  *(open)*
