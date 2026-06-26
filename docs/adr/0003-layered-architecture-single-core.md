# ADR-0003: Layered architecture and a single `no_std` core runtime

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0002 (framing), planned ADR-0004 (QHAL)

## Context

QOS must run the QPU control plane in two environments (per the embodiment chosen for the
ADR-0002 goal: **both**):

1. **Bare-metal x86_64 kernel** — a real-time embedded controller close to the QPU control
   electronics.
2. **Host daemon (`qosd`)** — a control plane on Linux for fast iteration and (eventually)
   connecting to cloud QPUs.

There is a critical architectural duplication today (verified):

- `crates/qos-core` contains a clean job model: `JobManager`, `Scheduler` (FIFO),
  `JobStore`, **journal/recovery**, and `EventLog`. But because this crate uses `std`, it
  is **disabled** in the workspace (commented out in `Cargo.toml`).
- The kernel re-implements the same job logic in
  `crates/qos-os-kernel/src/syscall.rs` using a **fixed 16-slot static array**, in a more
  primitive form.

So there are two separate, potentially conflicting job models, and the "good" one is not
even compiled.

## Decision

Consolidate the job logic into a **single portable core runtime** and embed it into both
embodiments. Layers:

```text
┌──────────────────────────────────────────────────────────────┐
│  L3 — ABI           qos-abi (program/kernel wire model)         │
├──────────────────────────────────────────────────────────────┤
│  L2 — Embodiments   qos-os-kernel (bare-metal) | qosd (host)    │
│                     both embed L1                               │
├──────────────────────────────────────────────────────────────┤
│  L1 — Core          qos-core  (no_std + alloc)                  │
│       runtime       job model, scheduler, store, eventlog,      │
│                     qubit resource manager, transpiler hook     │
├──────────────────────────────────────────────────────────────┤
│  L0 — QHAL          QuantumBackend trait (device/backend bound) │
│                     simulator = reference backend; remote = plugin │
└──────────────────────────────────────────────────────────────┘
```

Concrete decisions:

1. **`qos-core` becomes `no_std + alloc`.** Parts that depend on `std` (the journal's file
   I/O, tests) move behind `#[cfg(feature = "std")]`. Default is `no_std`.
2. **A single job model.** The kernel's 16-slot static job array in `syscall.rs` is retired;
   the kernel also uses `qos-core`'s `JobStore`/`JobManager`.
3. **QHAL (L0) is defined inside `qos-core`.** The existing
   `quantum::backend::QuantumBackend` trait evolves into this boundary (details in
   ADR-0004). The statevector simulator is the reference implementation of this trait.
4. **Embodiments embed L1** and keep their platform details (IDT/memory in the kernel,
   threads/IO on the host) outside L1.
5. `qos-core` and `qos-abi` become workspace members again; CI builds and tests both the
   `no_std` and `std` feature configurations.

## Rationale

- Single source of truth: job/scheduler semantics are written once and run in both
  environments.
- `no_std + alloc` is the common denominator for both the bare-metal kernel and the host
  daemon; the host enables extra conveniences (file journal) via the `std` feature.
- It reuses the existing good design (recovery, eventlog) instead of discarding it, and
  removes the primitive 16-slot copy.
- Separating the QHAL as L0 puts the simulator and the real/remote QPU behind the same
  boundary.

## Consequences

### Positive

- Job logic lives in one place; the two-conflicting-models problem ends.
- The simulator and a real QPU are interchangeable behind the same trait.
- Behavioral parity between the host daemon and the kernel.

### Negative / Trade-offs

- Porting `qos-core` to `no_std` is a real refactor (moving to `alloc`, feature-gating
  `std` APIs, abstracting journal I/O).
- During the transition the old kernel job path and the new path may coexist; a careful
  cutover is required.

### Neutral / Follow-ups

- The exact QHAL trait surface is finalized in ADR-0004.
- The journal I/O abstraction (a trait) gets an in-memory implementation on the `no_std`
  side and a file-based one on the `std` side.

## Alternatives considered

1. **Keep the kernel's own 16-slot job store, leave `qos-core` disabled** — rejected:
   perpetuates two divergent job models and wastes the better existing design.
2. **Make `qos-core` host-only and keep the kernel separate** — rejected: defeats the
   "both embodiments" decision and guarantees behavioral drift.
