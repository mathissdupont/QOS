# QOS transfer and issue opening plan

This file is a handoff checklist for moving QOS under Heptapus Open Code Organization and then
opening GitHub issues without changing the existing WP/ADR documentation format.

## Repository transfer checklist

1. Transfer the GitHub repository to **Heptapus Open Code Organization** from GitHub:
   `Settings -> General -> Danger Zone -> Transfer ownership`.
2. Confirm the new repository URL. Expected shape:
   `https://github.com/Heptapus-Open-Code-Organization/QOS.git`
3. Update the local remote after transfer:

```powershell
git remote set-url origin https://github.com/Heptapus-Open-Code-Organization/QOS.git
git remote -v
```

4. Confirm branch protection and CI are still enabled for `main`.
5. Confirm issue labels exist or create the minimal label set below.

## Minimal label set

- `type:wp`
- `type:gap`
- `type:bug`
- `area:kernel`
- `area:drivers`
- `area:storage`
- `area:networking`
- `area:ui`
- `area:quantum`
- `area:installer`
- `area:docs`
- `priority:p0`
- `priority:p1`
- `priority:p2`
- `blocked`
- `good first issue`

## Opening order

Open these first because they unblock the "real OS" path:

1. WP-10 Networking: working NIC, DHCP, TCP/HTTP egress.
2. WP-09 VFS unification: one filesystem namespace.
3. WP-08 remaining kernel foundations: per-process resources and SMP.
4. Platform gap: PCIe ECAM + MSI/MSI-X.
5. Storage gap: block layer + NVMe/virtio-blk.
6. WP-11 Installer/OOBE.
7. WP-05 UI polish.
8. WP-12 Cloud QPU connectivity, after WP-10 is usable.

## Issue drafts

Copy each draft into GitHub after the transfer. The body follows the current feature-request
template sections.

### [feat] WP-09: unify RAM fs, QOSFS, and FAT under one VFS tree

Labels: `type:wp`, `area:storage`, `priority:p0`

## Problem / motivation

QOS currently exposes multiple filesystem paths and APIs: RAM fs, flat `disk:` QOSFS, and FAT16
support. Apps special-case storage, which makes QOS feel like a kernel demo instead of a real OS.

## Proposed solution

Implement WP-09:

- Add a `FileSystem` trait and mount table.
- Mount RAM fs at `/`, persistent QOSFS at `/disk`, and leave room for FAT.
- Move Files, Text Editor, QASM Studio, Terminal, and user-mode VFS syscalls to plain paths.
- Add directory support or equivalent parity for persistent storage.
- Add a `mount` shell command and storage UI rows.

Acceptance: the same read/write/list/create/remove/rename behavior works through one API in QEMU,
two-boot persistence still passes, and apps no longer require `disk:` special cases.

## Alternatives considered

Keep `disk:` prefixes as a compatibility layer only during migration.

## Which phase?

P3 Storage & filesystems; WP-09.

### [feat] WP-10: bring up a NIC and verified TCP/HTTP egress

Labels: `type:wp`, `area:networking`, `priority:p0`

## Problem / motivation

The current E1000 driver does not bind to the q35 e1000e device, and TLS/cloud access is blocked.
QOS needs verified networking for general OS usability and for cloud QPU integration.

## Proposed solution

Implement WP-10:

- Decide via ADR whether the first modern path is e1000e or virtio-net.
- Bring up link in QEMU q35.
- Add DHCP and ICMP verification.
- Verify TCP and HTTP GET against a local host test server.
- Decide the TLS path: ADR-0011 proxy or a vetted no_std TLS approach.

Acceptance: QOS obtains an IP, `ping` round-trips, `fetch <url>` works against a local test server,
and serial/UI state proves link, IP, and packet exchange.

## Alternatives considered

Support virtio-net first for VM portability, then e1000e/real NICs.

## Which phase?

P5 Networking; WP-10.

### [feat] WP-08: finish process resource isolation and SMP bring-up

Labels: `type:wp`, `area:kernel`, `priority:p0`

## Problem / motivation

QOS has preemption, Ring-3 demos, per-process paging, and W^X, but the remaining kernel foundation
work is what turns the system into a scalable OS: process-owned resources, quotas, and multi-core
execution.

## Proposed solution

Finish WP-08 remaining slices:

- Add per-process handles and quotas for files, memory, IPC, and quantum jobs.
- Ensure runaway processes cannot exhaust kernel resources.
- Bring up application processors through APIC INIT/SIPI.
- Add per-CPU state, scheduler queues, and SMP-safe locking rules.

Acceptance: QEMU demos show a process hitting a quota without destabilizing the kernel, and a
multi-core QEMU boot shows APs online with timer/scheduler accounting.

## Alternatives considered

Keep SMP as a separate WP if the first resource-isolation slice becomes large.

## Which phase?

P0 Kernel core hardening and P1 Platform modernization; WP-08.

### [feat] Add PCIe ECAM and MSI/MSI-X support

Labels: `type:gap`, `area:drivers`, `area:kernel`, `priority:p0`

## Problem / motivation

Modern hardware and VM devices need PCIe config space and MSI/MSI-X. Without ECAM and capability
parsing, NVMe, virtio, xHCI, and modern NIC work stays fragile or blocked.

## Proposed solution

- Parse ACPI MCFG and expose PCIe ECAM access.
- Enumerate PCI capabilities.
- Implement MSI/MSI-X allocation and interrupt routing.
- Keep legacy PCI config I/O as fallback.

Acceptance: QEMU q35 devices enumerate through ECAM, MSI-capable devices expose capabilities, and
at least one driver can receive an MSI interrupt.

## Alternatives considered

Continue legacy INTx first, but document which devices remain blocked.

## Which phase?

E-12 PCIe + MSI/MSI-X; follows WP-03 and feeds USB, NVMe, virtio, networking.

### [feat] Add block layer plus NVMe and virtio-blk roadmap slice

Labels: `type:gap`, `area:storage`, `area:drivers`, `priority:p0`

## Problem / motivation

QOS has AHCI/QOSFS work, but a real OS needs a block layer and modern storage devices. NVMe and
virtio-blk are the practical path for real PCs and modern VMs.

## Proposed solution

- Add a uniform block-device trait and request queue.
- Add a small buffer/page cache.
- Port AHCI behind the block layer.
- Add virtio-blk for modern VMs.
- Add NVMe discovery/read/write as the real-hardware target.

Acceptance: filesystem code talks only to the block layer, AHCI still works, and at least one
modern VM storage backend boots with read/write verification.

## Alternatives considered

Keep AHCI direct for now and land the block trait first.

## Which phase?

E-40 Block layer, E-23 NVMe/AHCI, E-24 virtio.

### [feat] WP-11: complete installer, login, i18n, and installed-disk boot

Labels: `type:wp`, `area:installer`, `priority:p1`

## Problem / motivation

QOS needs a first-boot and installation flow so it behaves like an OS product, not only a bootable
demo image.

## Proposed solution

Implement WP-11:

- Finish first-boot persisted config.
- Add login and accounts with password hashing.
- Add desktop-wide TR/EN string tables.
- Implement real installation to a target disk: GPT, ESP, copy boot image, boot without live media.
- Add recovery/reset/update hooks.

Acceptance: two-boot QEMU proof for OOBE, keyboard-only completion, and installed-disk boot with
the live medium detached.

## Alternatives considered

Keep slice 1 passwordless, but do not present it as real authentication.

## Which phase?

E-90 product/installation experience; WP-11.

### [feat] WP-05: finish desktop polish, resize, notifications, wallpaper, and image viewer

Labels: `type:wp`, `area:ui`, `priority:p1`

## Problem / motivation

The modern desktop is usable, but key desktop behaviors are still missing: edge resizing,
notifications, wallpaper choices, and basic media viewing.

## Proposed solution

Implement the remaining WP-05 UI side:

- Edge-drag window resizing with stable layout constraints.
- Wallpaper selection and persisted appearance settings.
- Notification surface and notification history.
- Image viewer app for files opened from Files.
- Keep keyboard/mouse parity and dirty-rect redraw discipline.

Acceptance: QEMU screenshots show resize, notifications, wallpaper selection, and image viewer
without layout overlap or input regressions.

## Alternatives considered

Split notifications into a later system-services WP if it needs an event bus first.

## Which phase?

P6 Modern UI/UX; WP-05.

### [feat] WP-12: QHAL backend abstraction and cloud QPU provider integration

Labels: `type:wp`, `area:quantum`, `area:networking`, `blocked`, `priority:p1`

## Problem / motivation

QOS has a strong local quantum engine and QASM tooling, but the QPU control-plane promise requires
remote provider backends. This is blocked until WP-10 supplies TCP/HTTP and a TLS decision.

## Proposed solution

Implement WP-12:

- Define a `QuantumBackend` trait and migrate the local simulator behind it.
- Write the provider-protocol ADR.
- Add one mock provider path for QEMU tests.
- Add one real/sandbox provider adapter.
- Add backend picker, capability display, token storage, retry, offline queue, and result cache.

Acceptance: local backend still works offline, mock provider works in QEMU, and provider secrets
are never logged.

## Alternatives considered

Use ADR-0011 local proxy first, then revisit in-kernel TLS later.

## Which phase?

P7 Quantum control plane; WP-12, blocked by WP-10.

### [feat] Add power management: UI shutdown/reboot and ACPI FADT path

Labels: `type:gap`, `area:kernel`, `area:ui`, `priority:p1`

## Problem / motivation

QOS has no clean shutdown/reboot path surfaced in the UI. The legacy ACPI shutdown/reboot logic is
not integrated with the modern ACPI path.

## Proposed solution

- Rework ACPI poweroff/reboot through bootloader-provided ACPI tables and `qos-acpi`.
- Add shell commands for `poweroff` and `reboot`.
- Add UI power menu actions.
- Verify QEMU exits or reboots cleanly.

## Alternatives considered

Keep keyboard-controller reset as a fallback only.

## Which phase?

Platform modernization and product polish; gap G-01/G-16.

### [feat] Real-hardware validation matrix and bootable USB release flow

Labels: `type:gap`, `area:docs`, `area:drivers`, `priority:p1`

## Problem / motivation

QOS is primarily verified under QEMU+OVMF. A real OS needs a tracked hardware matrix and release
flow for bootable USB images.

## Proposed solution

- Define the hardware/VM support matrix.
- Validate QEMU q35/pc, VirtualBox, VMware, Hyper-V, and at least one physical UEFI machine.
- Record display/input/storage/network status.
- Add release checklist for `dist/qos-uefi.img`.

## Alternatives considered

Use VM-only support as an explicit milestone, but do not describe it as real-hardware ready.

## Which phase?

E-92 Real hardware & installer; ADR-0014 Stage 4.

### [feat] Add libc/SDK and a first external userland app path

Labels: `type:gap`, `area:kernel`, `priority:p2`

## Problem / motivation

Ring-3 exists, but third-party programs need a libc/SDK, stable syscall wrappers, and a documented
build path.

## Proposed solution

- Define the initial stable userspace ABI surface.
- Provide syscall wrappers and a tiny libc/relibc/newlib-compatible layer.
- Add a sample external userland app.
- Document build, link, package, and run flow.

## Alternatives considered

Keep raw assembly demos only for kernel verification.

## Which phase?

E-51 C library + SDK and E-50 process model.

### [feat] Add init/services/logging foundation

Labels: `type:gap`, `area:kernel`, `priority:p2`

## Problem / motivation

A general-purpose OS needs PID 1, services, dependency startup, and logs beyond ad hoc serial
output.

## Proposed solution

- Add an init/service manager MVP.
- Add kernel/user logging surfaces: dmesg/syslog-style ring buffer.
- Move long-running background services behind the service model.
- Expose service status in shell and UI.

## Alternatives considered

Keep services as kernel threads until userland is ready, with the public model documented.

## Which phase?

E-53 Init & services.

