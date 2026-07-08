# QOS Work Packages (WP)

A **work package** is a concrete, shippable unit of work: a scope, a task list, acceptance
criteria, and a status. WPs are how we execute the epics in [`../MASTERPLAN.md`] step by step.

- **ADRs** (`../adr/`) record *decisions* — why we chose an approach.
- **WPs** (this directory) record *work* — what we're building, the steps, and whether it's done.
- Each WP links to its epic (E-xx in the master plan), any ADRs, and the commits that delivered it.

Use [`template.md`](template.md) for new WPs. Statuses: 🔴 not started · 🟡 in progress · ✅ done.

## Register

| WP | Title | Epic | ADR | Issue | Status |
| ---- | ------- | ------ | ----- | ------- | -------- |
| [WP-01](WP-01-uefi-boot-and-desktop.md) | UEFI boot repair + framebuffer desktop | E-70 (seed) | ADR-0014 | — | ✅ done |
| [WP-02](WP-02-driver-model.md) | Device/driver model | E-01 | ADR-0016 | — | ✅ done |
| [WP-03](WP-03-acpi-apic.md) | ACPI + modern APIC interrupts | E-10 | ADR-0015 | — | ✅ done |
| [WP-04](WP-04-usb-input.md) | USB host controller + HID input | E-20, E-21 | ADR-0015 | — | ✅ done |
| [WP-05](WP-05-modern-ui.md) | Modern UI: compositor, fonts, WM, 10 apps, storage UX | E-70..73 | ADR-0017/0018 | [#11](https://github.com/Heptapus-Open-Code-Organization/QOS/issues/11) | 🟡 in progress |
| [WP-06](WP-06-quantum-control-plane.md) | Quantum control plane: engine, visual lab, QASM toolchain | E-80 | ADR-0019/0021 | — | ✅ done |
| [WP-07](WP-07-quantum-ide.md) | Quantum IDE (VS Code-like environment for circuits) | E-80, E-73 | ADR-0021 | — | ✅ done |
| [WP-08](WP-08-kernel-foundations.md) | Kernel foundations: preemption, user mode, W^X | E-30/31/11 | ADR-0020+ | [#7](https://github.com/Heptapus-Open-Code-Organization/QOS/issues/7) | 🟡 in progress |
| [WP-09](WP-09-vfs-unification.md) | VFS unification: one tree over RAM fs/QOSFS/FAT | E-40/41 | ADR-0018+ | [#6](https://github.com/Heptapus-Open-Code-Organization/QOS/issues/6) | 🔴 not started |
| [WP-10](WP-10-networking.md) | Networking: working NIC + TCP/IP + egress | E-50 | ADR-0011+ | [#5](https://github.com/Heptapus-Open-Code-Organization/QOS/issues/5) | 🔴 not started |
| [WP-11](WP-11-installer-oobe.md) | Installer & first-boot setup: language, user, disk, login | E-90 | — | [#10](https://github.com/Heptapus-Open-Code-Organization/QOS/issues/10) | 🟡 in progress |
| [WP-12](WP-12-cloud-qpu-api.md) | Cloud QPU connectivity: QHAL backends + provider API | E-80/81 | ADR-0011+ | [#12](https://github.com/Heptapus-Open-Code-Organization/QOS/issues/12) | 🔴 blocked on WP-10 |
| [WP-13](WP-13-quantum-safe-security.md) | Quantum-safe security: PQC, kernel crypto, secure channels | E-54 | (new ADR) | [#19](https://github.com/Heptapus-Open-Code-Organization/QOS/issues/19) | 🔴 not started |

All remaining epics are tracked as detailed [GitHub issues](https://github.com/Heptapus-Open-Code-Organization/QOS/issues)
with milestones M1–M4: PCIe ECAM + MSI (#8) · block layer + NVMe/virtio-blk (#9) · VMM
maturation (#20) · time & sleep (#21) · USB mass storage (#22) · quantum engine depth:
noise/mitigation/calibration/hybrid (#23) · users/permissions (#24) · CI/QEMU test harness
(#25) · packaging (#26) · crash-consistent QOSFS (#27) · audio (#28) · power management (#13) ·
real-hardware matrix (#14) · libc/SDK (#15) · init/services/logging (#16).

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
- **G-09** ~~The desktop runs as one cooperative kernel loop~~ → **resolved** by WP-08 slice 1
  (43d10e0): the desktop runs with the preemptive scheduler armed; heavy quantum jobs execute on
  a background kthread while the UI stays live. *(closed 2026-07-02)*
- **G-10** No process isolation on the desktop path: every "app" shares kernel memory and can,
  by bug, corrupt any other. → WP-08 slice 3 (user-mode processes). *(open)*
- **G-11** Windows cannot be resized by edge drag (only min/max); no notifications; no wallpaper
  options. → WP-05 next slices. *(open)*
- **G-12** ~~`ParseError` carries no line~~ → **resolved** by WP-07 slice 2 (a0b3753): errors
  carry their 1-based line; IDE problems row + red gutter + F8 jump. *(closed 2026-07-02)*
- **G-13** ~~Editors were append/backspace-only~~ → **resolved** by WP-07 slices 1+3 (9125d8f):
  shared `ed_*` cursor-editing core in both the IDE and the Text Editor. *(closed 2026-07-02)*
- **G-14** Three disjoint filesystems + `disk:` special-casing in apps; no mount tree. → WP-09.
  *(open)*
- **G-15** The NIC driver targets E1000 (8086:100e) but q35 exposes e1000e (8086:10d3) — no
  network at all today. → WP-10 slice 1. *(open)*
- **G-16** No clean shutdown/reboot path surfaced in the UI (ACPI poweroff exists only as legacy
  code, see G-01). Fold into a power-management WP. *(open)*
- **G-17** Editor niceties deferred from WP-07 v1: text selections + clipboard, gate-name
  autocompletion, QASM import into the visual Lab (code → circuit), per-glyph click-to-position
  metrics. *(open)*
- **G-18** PCIe ECAM + MSI/MSI-X are not implemented yet. This blocks the clean modern-device path
  for NVMe, virtio, newer NICs and MSI-capable USB/storage devices. *(open; issue #8)*
- **G-19** No uniform block layer/request queue/cache yet; AHCI/QOSFS exists, but filesystems and
  modern storage drivers need a shared block-device contract before NVMe/virtio-blk can mature.
  *(open; issue #9)*
- **G-20** Ring-3 exists, but there is no libc/SDK path for third-party userland apps. Raw syscall
  demos prove the kernel path; a real OS needs documented build/link/run support. *(open; issue
  #15)*
- **G-21** No init/service manager or dmesg/syslog-style service logging yet; background work is
  still exposed as bespoke kernel/UI state. *(open; issue #16)*
- **G-22** ~~Transfer-to-organization follow-up~~ → **resolved**: repository transferred to
  Heptapus Open Code Organization, `origin` updated, and the full backlog opened as 24 detailed
  issues with labels (`type:*`, `area:*`, `priority:*`) and milestones M1–M4. *(closed 2026-07-08)*
- **G-23** Cryptography today is classical-only or absent: the TLS path is a stub (G-07) and the
  kernel offers no PQC, no vetted hash/AEAD surface. Quantum-safe policy adopted → WP-13. *(open;
  issue #19)*
- **G-24** No kernel entropy source or CSPRNG API: RDSEED/RDRAND are unused and quantum shot
  sampling runs on an ad-hoc PRNG. One entropy-fed CSPRNG should serve both → WP-13 slice 2.
  *(open; issue #19)*
- **G-25** No CI-run QEMU integration harness: boot smoke tests, serial assertions, and
  screenshot checks exist only as ad-hoc local scripts. → E-90 testing issue. *(open; issue #25)*
