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
| [WP-04](WP-04-usb-input.md) | USB host controller + HID input | E-20, E-21 | ADR-0015 | ✅ done |
| [WP-05](WP-05-modern-ui.md) | Modern UI: compositor, fonts, WM, 10 apps, storage UX | E-70..73 | ADR-0017/0018 | 🟡 in progress |
| [WP-06](WP-06-quantum-control-plane.md) | Quantum control plane: engine, visual lab, QASM toolchain | E-80 | ADR-0019/0021 | 🟡 in progress |
| [WP-07](WP-07-quantum-ide.md) | Quantum IDE (VS Code-like environment for circuits) | E-80, E-73 | ADR-0021 | 🟡 in progress |
| [WP-08](WP-08-kernel-foundations.md) | Kernel foundations: preemption, user mode, W^X | E-30/31/11 | ADR-0020+ | 🔴 not started |
| [WP-09](WP-09-vfs-unification.md) | VFS unification: one tree over RAM fs/QOSFS/FAT | E-40/41 | ADR-0018+ | 🔴 not started |
| [WP-10](WP-10-networking.md) | Networking: working NIC + TCP/IP + egress | E-50 | ADR-0011+ | 🔴 not started |

Upcoming (not yet opened as WP files; see the master-plan critical path):
PCIe ECAM + MSI (E-12) · SMP (E-11, folded into WP-08 slice 5) · NVMe (E-23) · sound · power
management (clean ACPI shutdown/reboot).

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
- **G-09** The desktop runs as one **cooperative** kernel loop — a busy computation freezes
  input/UI; no preemption. → WP-08 slice 1. *(open)*
- **G-10** No process isolation on the desktop path: every "app" shares kernel memory and can,
  by bug, corrupt any other. → WP-08 slice 3 (user-mode processes). *(open)*
- **G-11** Windows cannot be resized by edge drag (only min/max); no notifications; no wallpaper
  options. → WP-05 next slices. *(open)*
- **G-12** `ParseError` (QASM) carries no line/column — compile errors name the kind, not the
  place. → WP-07 slice 2. *(open)*
- **G-13** Editors were append/backspace-only (no cursor) — being fixed by WP-07 slice 1 for the
  IDE; Text Editor adopts the shared core in WP-07 slice 3. *(in progress)*
- **G-14** Three disjoint filesystems + `disk:` special-casing in apps; no mount tree. → WP-09.
  *(open)*
- **G-15** The NIC driver targets E1000 (8086:100e) but q35 exposes e1000e (8086:10d3) — no
  network at all today. → WP-10 slice 1. *(open)*
- **G-16** No clean shutdown/reboot path surfaced in the UI (ACPI poweroff exists only as legacy
  code, see G-01). Fold into a power-management WP. *(open)*
