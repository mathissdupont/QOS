# ADR-0004: QHAL — Quantum Hardware Abstraction Layer

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0003 (layers, QHAL = L0), ADR-0011 (remote backends)

## Context

QOS must treat "the thing that executes a circuit" uniformly, whether it is the in-kernel
simulator, a cloud QPU, or (future) real local hardware. Today a `QuantumBackend` trait
exists in `qos-os-kernel/src/quantum/backend.rs`, but it lives in the kernel and is modeled
as a cloud-style "submit circuit → get counts" API; it has no device lifecycle and is not
shared with the host daemon.

Per ADR-0003, the device/backend boundary (L0) must live in the shared `qos-core` so both
embodiments use the same abstraction.

## Decision

Define a **QHAL** in `qos-core` as the single trait every quantum executor implements. It is
polling-based (no async), `no_std`-friendly, and covers device lifecycle, not just job
submission.

Trait surface (conceptual):

- **Identity & capability:** `name()`, `capabilities()` (max qubits, supported/native gates,
  connectivity topology, error rates, mid-circuit-measurement support).
- **Lifecycle:** `status()` (Available/Busy/Offline/NeedsCalibration/Maintenance),
  `fetch_calibration()` (refresh characterization data — see ADR-0007).
- **Validation:** `validate_circuit(circuit)` against capabilities/topology.
- **Execution (polling model):** `submit(job) -> BackendJobId`, `poll(id) -> JobState`,
  `result(id) -> Option<BackendResult>`, `cancel(id)`.

Rules:

1. The **local statevector simulator** is the reference implementation of QHAL.
2. **Cloud/remote QPUs** are QHAL implementations whose transport goes through the proxy in
   ADR-0011 (no in-backend TLS).
3. A `BackendManager` registers backends and selects one for a circuit
   (capability + topology aware).
4. The QHAL deals in the **internal `Circuit` IR** (ADR-0005), not in raw QASM strings;
   serialization to a provider's format is each backend's concern.

## Rationale

- A polling model avoids requiring an async runtime in `no_std`/bare-metal.
- Putting lifecycle (status, calibration) in the trait makes a real QPU a first-class target,
  not a special case bolted onto a cloud API.
- One boundary for simulator and hardware enables transparent substitution and testing.

## Consequences

### Positive

- Simulator and real/cloud QPU are interchangeable; tests run against the simulator.
- The host daemon and kernel share one backend abstraction.

### Negative / Trade-offs

- The existing kernel `QuantumBackend` must be migrated to the `qos-core` QHAL (one-time
  refactor); duplication exists during transition.

### Neutral / Follow-ups

- Pulse-level / instruction-stream control (below the gate level) is not in the first QHAL
  surface; it can be added as an optional sub-trait when a real hardware target appears.
