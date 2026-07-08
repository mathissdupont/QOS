# QOS Master Plan — the road to a real, modern operating system

Status: living document, created 2026-07-01. This is the authoritative long-range plan (it
superseded the earlier `ROADMAP.md`/`VISION.md`, since removed). Short-range execution detail
lives in `docs/PLAN.md`; every structural decision is (or becomes) an ADR in `docs/adr/`.

## 1. Vision

QOS is a **real, modern, general-purpose operating system** for **UEFI x86-64** — booting and
usable on mainstream modern PCs and VMs — with a **first-class quantum control plane** as its
differentiator (ADR-0002). "Real and modern" concretely means:

- A **driver-based architecture** like Linux/Windows: a device/driver model with bus enumeration,
  driver binding, resource and interrupt management, and (eventually) hotplug — not hardcoded
  `init()` calls per device.
- Modern hardware: **APIC/SMP** (multi-core), **USB** (HID + storage), **NVMe/virtio** storage,
  modern **NIC**s, high-resolution **framebuffer** graphics (done, ADR-0014).
- A modern **user space**: isolated processes (done), a stable syscall ABI (done), a C library
  and POSIX-ish surface, users/permissions, and a real UI (compositor, scalable fonts, widgets).
- Quantum computing as a **built-in subsystem**: simulator (done), transpilation, error
  mitigation, calibration, multi-backend + cloud QPU, hybrid execution, and dev tools.
- **Quantum-safe by design**: every cryptographic surface (provider channels, update signing,
  secrets at rest) uses NIST post-quantum algorithms, hybridized with classical crypto during
  the transition — an OS built for a quantum world must not be broken by one (WP-13).

### Honest framing

Matching Linux/Windows *breadth* is thousands of person-years and is **not** the goal. The goal
is a **usable modern general-purpose OS on the mainstream modern-hardware/VM set**, built
incrementally, each step shippable and verified in QEMU before real hardware. Every pillar below
is real work; sizings are honest and relative (S/M/L/XL), not calendar promises.

## 2. Guiding principles

1. **Driver model first.** New hardware support is a *driver* registered with a device manager,
   not a bespoke boot-time call. This is the single biggest "real OS" architectural shift.
2. **Fallback-first probing.** Probe modern (xHCI, APIC, virtio) first; keep the legacy path
   (PS/2, PIC, E1000) as graceful fallback so we never regress QEMU/existing platforms.
3. **Everything verifiable.** Host unit tests for portable logic (`qos-core`, `qos-gfx`, parsers,
   the driver core), QEMU integration/boot tests for the kernel, CI on every push.
4. **ADR-backed.** Each structural decision gets an ADR; each large epic may get its own.
5. **Incremental & shippable.** Prefer a working narrow slice over a broad broken one.

## 3. Current baseline (done / verified)

- UEFI boot (bootloader 0.11) + linear framebuffer; graphical desktop renders (ADR-0014).
- Preemptive multitasking, Ring-3 isolation, per-process paging, W^X, fault-kill, pipe IPC.
- Two syscall ABIs; in-kernel statevector simulator + QHAL skeleton (ADR-0004).
- Legacy drivers: PS/2 kbd/mouse, E1000 NIC (TCP/IP), PIC/PIT, RTC, PCI enumerate, ATA.

## 4. Pillars & epics

Each epic lists its rough size and key dependencies. IDs (E-xx) are for cross-reference.

### P0 — Kernel core hardening (foundation for everyone)
- **E-01 Driver/device model** (L). A `Device`/`Driver` trait model, a device manager/registry,
  bus abstraction, resource (MMIO/port/IRQ) allocation, and an IRQ dispatch abstraction. Retrofit
  existing drivers (E1000, PS/2, ATA) onto it. *New ADR.* **The backbone of "like Linux/Windows".**
- **E-02 Memory management maturation** (L). Proper VMM: demand paging, copy-on-write fork,
  guard pages everywhere, mmap; a scalable allocator; SMP-safe locking primitives.
- **E-03 Time & sleep** (S). Monotonic clock abstraction, timer wheel, `nanosleep`-grade waits,
  decoupled from the PIT.

### P1 — Platform modernization (ADR-0015)
- **E-10 ACPI + APIC** (M). ✅ **Done.** RSDP (from bootloader) → XSDT → MADT parse (host-tested
  `qos-acpi`); local APIC enabled; scheduler tick moved to the local-APIC timer (calibrated);
  keyboard/mouse routed through the IO-APIC; the 8259 PIC/PIT are fully masked. Verified in
  QEMU+OVMF (q35) end-to-end. *Dependency root for USB, SMP, MSI — now unblocked.* (FADT-based
  shutdown/reboot from the legacy `acpi.rs` is a separate follow-up.)
- **E-11 SMP / multi-core** (L). AP bring-up (INIT/SIPI), per-CPU state, multi-core scheduler,
  IPIs, SMP-safe kernel. Depends on E-10, E-02.
- **E-12 PCIe + MSI/MSI-X** (M). ECAM config space, capability parsing, MSI interrupts. Depends
  on E-10, E-01.

### P2 — Buses & devices (drivers)
- **E-20 USB core + xHCI** (XL). xHCI host controller, USB core (enumeration, transfers). Depends
  on E-01, E-10, E-12. *New ADR.*
- **E-21 USB HID** (M). Keyboard + mouse (boot protocol → full HID) into the input queue. The
  real-laptop input unblocker. Depends on E-20.
- **E-22 USB mass storage** (M). BOT/UAS → block layer. Depends on E-20, E-40.
- **E-23 NVMe** (L) and **AHCI/SATA** (M). Real-disk block devices. Depends on E-01, E-12, E-40.
- **E-24 virtio** (M). virtio-net, virtio-blk (and later virtio-gpu) for modern VMs. Depends on
  E-01, E-12.
- **E-25 Modern NICs** (M, ongoing). RTL8169 / newer Intel; driver-model NIC interface.

### P3 — Storage & filesystems
- **E-40 Block layer** (M). Uniform block-device interface, request queue, buffer/page cache.
- **E-41 Real filesystems** (L). FAT32 read/write (removable), ext2/ext4 read (Linux media);
  mature the VFS (dirs, metadata, permissions, mounts). 
- **E-42 Persistence & fsck-lite** (M). Crash-consistent writes, a native FS or journaling.

### P4 — Process, user & runtime model
- **E-50 Full process model** (L). Robust ELF loader (dynamic), `fork`/`exec`/`wait`, threads,
  POSIX signals, scheduler classes.
- **E-51 C library + SDK** (XL). A `libc` (newlib/relibc-style) and a userland SDK so third-party
  apps can be built against QOS. *New ADR.*
- **E-52 Users, permissions, security** (L). Accounts/login, file permissions enforcement,
  privilege boundaries, basic MAC; secure-by-default posture.
- **E-53 Init & services** (M). PID 1, service/dependency manager, logging (dmesg/syslog).
- **E-54 Quantum-safe cryptography** (M/L). Kernel crypto module on NIST PQC — ML-KEM (FIPS 203)
  key establishment, ML-DSA/SLH-DSA (FIPS 204/205) signatures, hybridized with X25519/Ed25519 —
  plus an entropy-fed CSPRNG (RDSEED/RDRAND + jitter), KAT self-tests at boot, sealed secrets,
  and signed updates. Mandatory before WP-12 carries real credentials. *WP-13, new ADR.*

### P5 — Networking
- **E-60 Robust TCP/IP** (M). Harden the stack (retransmit, windows, timers), sockets syscalls,
  DNS, DHCP client, loopback.
- **E-61 Secure egress / TLS** (M). TLS via the `qosd` proxy (ADR-0011) and/or an in-kernel
  rustls-style path; HTTPS.

### P6 — Modern UI/UX (ADR-0015 Phase E; ADR-0012/0013)
- **E-70 Compositor + graphics** (L). Double-buffered off-screen surfaces, damage tracking, blit
  once (removes flicker + the cursor save-under hack); native-resolution rendering, HiDPI.
- **E-71 Scalable fonts** (M). Antialiased TrueType/vector glyph rendering; text layout.
- **E-72 Widget toolkit + WM** (L). Buttons, text fields, lists, scrollbars, menus; a real window
  manager (focus, z-order, resize, decorations); theming.
- **E-73 Apps & shell-of-the-GUI** (M, ongoing). File manager, text editor, terminal, settings,
  and the quantum apps (circuit editor / job monitor).

### P7 — Quantum control plane (the differentiator; ADR-0002/0004–0010)
- **E-80 Transpilation** (M). Layout, greedy SWAP routing, native-gate decomposition, opt passes
  (ADR-0008). *In progress — `Topology::shortest_path`/`is_all_to_all` landed.* Host-testable.
- **E-81 Noise + mitigation** (M). Noise models in the simulator; M3-style readout mitigation
  (ADR-0010). Matches the industry "noise mitigation" capability.
- **E-82 Calibration & monitoring** (M). Calibration fetch/aging, fidelity/health metrics,
  anomaly signals (ADR-0007).
- **E-83 Multi-backend + cloud QPU** (M). Real backends behind the QHAL via the TLS proxy
  (ADR-0011); job retry/trace; multi-user scheduling.
- **E-84 Hybrid execution** (M). Mid-circuit measurement + classical feedback (ADR-0009).
- **E-85 Quantum dev tools** (M, ongoing). Circuit editor, state inspector, algorithm demos.

### P8 — Toolchain, packaging, quality, distribution
- **E-90 Test & CI infra** (M, ongoing). Host unit tests + a QEMU integration harness (boot →
  drive → assert on serial) + coverage; CI on every push.
- **E-91 Package manager & app model** (L). Package format, dependency resolution, install/update.
- **E-92 Real-hardware & installer** (M). Bootable USB, hardware-support matrix, an installer;
  ADR-0014 Stage 4 validation across the major hypervisors and real machines.

## 5. Dependency-ordered critical path

```
E-01 driver model ─┬─> E-10 ACPI/APIC ─┬─> E-12 PCIe/MSI ─┬─> E-20 USB/xHCI ─> E-21 USB HID
                   │                    │                  │                 └─> E-22 USB storage ─┐
E-02 VMM ──────────┘                    ├─> E-11 SMP        ├─> E-24 virtio (net/blk)              ├─> E-40 block ─> E-41 FS
                                        │                   └─> E-23 NVMe / AHCI ──────────────────┘
E-70 compositor ─> E-71 fonts ─> E-72 widgets/WM ─> E-73 apps        (UI track — largely parallel)
E-80 transpile ─> E-81 noise/mitigation ─> E-82 calibration ─> E-83 cloud ─> E-84 hybrid  (quantum — parallel, host-testable)
```

Three tracks can advance in parallel: **hardware** (needs the driver model + APIC first), **UI**
(independent, immediately visible), **quantum** (independent, host-testable). The driver model
(E-01) and APIC (E-10) are the shared foundation for the hardware track.

## 6. Milestone bands

- **M1 — "Real OS foundation":** E-01 driver model + E-10 ACPI/APIC + E-02 VMM basics. Existing
  drivers retrofitted onto the model; boot unchanged; APIC timer live.
- **M2 — "Usable on modern PCs":** E-12 PCIe/MSI + E-20/E-21 USB HID + E-24 virtio-net. A real
  UEFI laptop/VM has working keyboard, mouse, display, network.
- **M3 — "Multi-core + storage":** E-11 SMP + E-23/E-24 block storage + E-40/E-41 filesystems.
- **M4 — "Modern desktop":** E-70/E-71/E-72/E-73 — compositor, scalable fonts, widgets, apps.
- **M5 — "Quantum OS":** E-80–E-85 + E-54 — transpile, mitigation, cloud QPU, hybrid, tools,
  and the quantum-safe crypto layer that secures the cloud path.
- **M6 — "Product":** E-51 SDK/libc, E-52 security, E-91 packaging, E-92 installer + real-HW.

## 7. Immediate next steps

1. Write the **driver-model ADR** (E-01) — the architectural centerpiece.
2. Implement **E-01** (device/driver registry + IRQ/resource abstraction) and retrofit one
   existing driver (e.g. PS/2 or E1000) as the proof.
3. Implement **E-10 ACPI/APIC** on top of it (dependency root for USB/SMP).
4. In parallel, continue **E-80 transpilation** (host-testable, already started) and/or begin the
   **E-70 compositor** for visible UI progress.

## 8. Risks

- **USB (E-20) and SMP (E-11) are XL/L** and the hardest items; sequence them after their
  prerequisites and keep legacy fallbacks so progress is never all-or-nothing.
- **Scope creep**: the driver model must stay minimal-but-real; resist rebuilding Linux's.
- **Real-hardware variance**: only closable via Stage 4 physical testing (ADR-0014).
```
