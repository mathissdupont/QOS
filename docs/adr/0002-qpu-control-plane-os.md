# ADR-0002: Frame QOS as a quantum-first OS (usable desktop + QPU control plane + cloud)

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0003 (architecture layers), ADR-0011 (cloud connectivity), ADR-0012 (desktop/UX)

## Context

The goal of the project, as clarified by the team, has two parts that must hold together:

1. A **usable, Windows-like operating system** — a real desktop experience: shell, file
   management, applications, a window/UI layer.
2. A **first-class quantum capability** — the OS must run quantum circuits as managed jobs,
   connect to **quantum cloud services** today, and connect to a **real QPU** if/when one
   becomes available.

Verified technical fact: in the real world an OS does not run *on top of* a QPU. A QPU is a
**co-processor / accelerator**, like a GPU. So the quantum-hardware-facing part of QOS is a
**classical control plane** that drives the QPU; the user-facing part is an ordinary (if
modern) OS whose distinguishing feature is that quantum computing is a built-in, first-class
subsystem rather than a remote afterthought.

The current codebase already contains both seeds: a bare-metal x86_64 kernel with a
text-mode desktop (`desktop.rs`, `gui.rs`, `explorer.rs`, `menu.rs`, `dialog.rs`) and a real
in-kernel statevector simulator plus a backend abstraction.

## Decision

QOS is a **quantum-first operating system**: a usable, Windows-like OS whose defining
capability is a built-in quantum subsystem that acts as a **QPU control plane** and a
**quantum-cloud client**, able to target a real QPU in the future through a hardware
abstraction layer.

**In scope:**

- **User-facing OS (the "Windows-like" experience):** window/compositor layer, shell, file
  management, applications. Structured as a modular, feature-gated layer (see ADR-0012).
  Text-mode now; pixel/framebuffer later.
- **Quantum control plane:** job model, scheduler, persistent job store, event log.
- **QPU hardware abstraction layer (QHAL):** the device/backend boundary (see ADR-0004).
- **Qubit resource management:** topology, calibration, error rates (ADR-0006, ADR-0007).
- **Transpilation pipeline:** layout, routing, native gate decomposition (ADR-0008).
- **Hybrid/dynamic execution:** mid-circuit measurement, classical feedback (ADR-0009).
- **Quantum cloud connectivity:** submit/poll/fetch results from cloud QPU providers, via a
  vetted TLS path (ADR-0011).
- **The in-kernel statevector simulator** as the reference backend behind the QHAL.

**Out of scope / deferred:**

- General-purpose multi-user server features (not a goal in the near term).
- In-kernel from-scratch TLS — replaced by a proxy approach (ADR-0011).
- Quantum error correction (QEC) decoding — architecture must not preclude it, but it is not
  near-term work (ADR-0010).

**Key boundary:** the desktop/UX layer must reach the quantum subsystem **only** through the
`qos-abi` request/response interface, never by calling internals directly. This keeps the
control plane usable headless (host daemon) and from a future GUI alike.

## Rationale

- The two goals are compatible: a normal OS shell on top, a quantum control plane underneath,
  cleanly separated by `qos-abi`. Neither has to compromise the other.
- The accelerator model matches real QPU control architectures and prepares a future
  transition to real hardware without rework.
- The existing strongest asset (the statevector simulator) becomes the reference backend, so
  the system is fully usable and testable before any real QPU exists.

## Consequences

### Positive

- A clear product: "an OS where quantum computing is a first-class citizen."
- The desktop work already done is preserved and given a defined place (ADR-0012).
- Cloud QPU usage is reachable now (via ADR-0011) without waiting for hardware.

### Negative / Trade-offs

- Broader scope than a pure control plane; the desktop layer carries real maintenance cost.
  Mitigated by feature-gating it so the core builds and is testable without it.
- Two embodiments (kernel + host daemon) must stay in parity (handled by ADR-0003).

### Neutral / Follow-ups

- The QPU modality (superconducting / trapped-ion / photonic) is undecided, so the QHAL is
  designed **hardware-agnostic** (ADR-0004).
- The desktop layer's structure and its `qos-abi`-only boundary are specified in ADR-0012.

## Revision note

An earlier draft of this ADR (same day) scoped the desktop layer *out*. It was revised before
acceptance once the team clarified that a usable, Windows-like experience is an explicit goal
alongside the quantum control plane.
