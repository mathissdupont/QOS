# ADR-0008: Transpilation pipeline

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0005 (Circuit IR), ADR-0006 (topology/layout), ADR-0007 (calibration cost)

## Context

A logical circuit almost never runs unchanged on real hardware: the device has a limited
**native gate set**, a fixed **connectivity**, and varying **qubit quality**. The circuit
must be rewritten to those constraints. The kernel has a `CircuitTranspiler` stub but no real
passes. This is mandatory infrastructure for any hardware (and useful even for the simulator
as a no-op/identity to keep the path live).

## Decision

Define a **staged, pluggable transpilation pipeline** in `qos-core` that maps a logical
`Circuit` (ADR-0005) to a backend-runnable `Circuit`:

1. **Optimize (pre-layout):** trivial simplifications (cancel adjacent inverses, merge
   rotations). Controlled by `optimization_level` (0–3, already in `qos-abi::JobOptions`).
2. **Layout:** choose a logical→physical qubit `Layout` (ADR-0006), using calibration quality
   (ADR-0007) when available.
3. **Routing:** insert SWAPs so every two-qubit gate acts on physically-connected qubits per
   the `Topology`.
4. **Native decomposition:** rewrite gates into the backend's native gate set
   (`capabilities().native_gates` from ADR-0004).
5. **Optimize (post):** clean up redundant gates introduced by routing/decomposition.

Each stage is a pass with a common interface; passes can be skipped per backend. For the
**simulator** (all-to-all, universal gate set) the pipeline is effectively identity, but runs
the same code so it is continuously exercised.

## Rationale

- Staged passes are the standard, well-understood structure (mirrors Qiskit/t|ket⟩) and keep
  each transformation testable in isolation.
- A pluggable pipeline lets a backend declare which stages it needs, so the simulator stays
  cheap while hardware gets the full treatment.

## Consequences

### Positive

- Hardware circuits become runnable; the simulator path validates the pipeline plumbing.
- `optimization_level` and `native_gates` gain concrete meaning.

### Negative / Trade-offs

- Routing/layout are genuinely hard problems; the first implementations will be simple
  (greedy SWAP routing, first-fit layout) and improved later.

### Neutral / Follow-ups

- Pass quality metrics (added gate count, estimated fidelity from calibration) are reported in
  job metadata for observability.
