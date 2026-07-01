# WP-02: Device/driver model

- Status: ✅ done
- Epic: E-01
- ADRs: ADR-0016
- Commits: 9c2dc81, 7e5df17

## Goal

Replace hardcoded per-device `init()` calls with a real, Linux/Windows-style driver architecture:
devices are enumerated and *bound* to drivers that *probe* them.

## What was delivered

- `qos-driver` crate: `DeviceId`/`Resource`/`Device`, the `Driver` + `DeviceIo` traits, and a
  `DeviceManager` that matches enumerated devices to drivers and probes them. Hardware access is
  behind `DeviceIo`, so the matching/binding/resource logic is host-tested (7 tests).
- Kernel `device.rs`: `KernelIo` (real MMIO/port I/O), PCI enumeration → `Device`s, an `e1000`
  driver, `probe_all`, and an `lsdev`-style bind log. Additive (legacy `e1000::init` untouched).

## Acceptance criteria

✅ In QEMU: enumerates 6 PCI devices, binds `8086:100e` (Ethernet) to the `e1000` driver
(MMIO 0x81040000, IRQ 11), leaves bridges/VGA unmatched; boot unaffected.

## Notes & gaps

- Existing IRQ handlers not yet driver-model citizens (Gaps **G-03**).
- No `lsdev` shell command yet (Gaps **G-04**).
