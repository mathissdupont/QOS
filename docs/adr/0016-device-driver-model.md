# ADR-0016: A device/driver model (the "real OS" driver architecture)

- Status: Accepted
- Date: 2026-07-01
- Related ADRs: ADR-0015 (modern hardware program). Epic E-01 in `docs/MASTERPLAN.md`.

## Context

Today every device is wired up by a hardcoded, hand-ordered call in `kernel_main`
(`mouse::init()`, `pci::init()`, `e1000::init()`, …). Each driver pokes hardware directly and
owns its own globals. That is fine for a handful of fixed devices but it is not how a real OS
(Linux, Windows, Redox) works, and it does not scale to the modern-hardware program (ADR-0015):
USB devices appear on a bus and must be *enumerated and bound* to drivers at runtime; the same
class of device (e.g. a NIC) may be served by different drivers; interrupts must be routed and
shared; resources (MMIO windows, port ranges, IRQ lines) must be owned and freed.

We need a **device/driver model**: a common abstraction that separates *what a device is* from
*which driver drives it*, with a manager that matches and binds them.

## Decision

Introduce a small, portable **driver core** as a new `no_std` crate `qos-driver`, plus a kernel
integration layer. The core is deliberately hardware-independent so it is **unit-testable on the
host** (per the master-plan "everything verifiable" principle); all real hardware access is
behind a trait the kernel implements.

### Core concepts

- **`DeviceId`** — how a device is identified/matched: a `bus` kind plus class/vendor/device
  fields (e.g. PCI `class`/`vendor`/`device`, or a synthetic id for platform/USB devices).
- **`Resource`** — a claimed hardware resource: `Mmio { base, len }`, `Port { base, len }`, or
  `Irq(line)`. A device carries the resources it needns; the manager tracks ownership.
- **`Device`** — an enumerated device: its `DeviceId`, its resources, and a bind state
  (`Unbound` → `Bound { driver }` / `Failed`).
- **`Driver`** — a trait: `name()`, `matches(&DeviceId) -> bool`, and
  `probe(&self, &mut Device, &mut dyn DeviceIo) -> Result<(), DriverError>`. A driver claims a
  device it matches and initializes it during `probe`.
- **`DeviceIo`** — the kernel-provided hardware gateway trait (MMIO read/write, port in/out,
  IRQ registration). Tests pass a mock; the kernel passes the real thing. This is the seam that
  keeps the core portable and testable.
- **`DeviceManager`** — the registry: register drivers, add enumerated devices, and
  `probe_all()` which binds each unbound device to the first matching driver and calls `probe`.
  Also lists devices/drivers for a `lsdev`-style view.

### Kernel integration

- The kernel implements `DeviceIo` over real paging/port I/O and the interrupt layer.
- Enumerators (PCI now; USB and platform later) create `Device`s and hand them to the manager.
- Existing drivers migrate onto the model incrementally. The first retrofit proves the seam; the
  rest follow. Until migrated, a driver may keep its legacy `init()` (fallback-first, no regression).

## Rationale

- Separating device/driver/manager is the standard, well-understood structure and is exactly what
  USB/hotplug (ADR-0015) needs — devices that appear at runtime get bound by matching, not by a
  new hardcoded call.
- Putting the core in a host-tested `no_std` crate means the matching/binding/resource logic —
  the part most prone to subtle bugs — is verified off-hardware, while the kernel only supplies a
  thin `DeviceIo`.

## Consequences

- **Positive:** a scalable foundation for every future driver; testable core; a clear place for
  resource/IRQ ownership; enables `lsdev`, hotplug, and driver reuse across device instances.
- **Negative / trade-offs:** an indirection layer over today's direct calls; existing drivers
  must be migrated (done incrementally, legacy kept as fallback until then).
- **Follow-ups:** IRQ sharing and hotplug/removal are deferred to when USB (E-20) needs them; the
  first cut covers register → enumerate → match → probe.
```
