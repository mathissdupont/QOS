# ADR-0006: Qubit resource and topology model

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0007 (calibration), ADR-0008 (routing uses topology), ADR-0004 (capabilities expose topology)

## Context

The unique scheduling problem of a QPU OS is not CPU time — it is **physical qubits**, which
are few, topology-constrained (not all pairs can interact), and heterogeneous in quality
(different error rates, T1/T2). A classical OS allocates memory/CPU; a QPU OS must allocate
**qubits** to jobs subject to connectivity and live calibration. Today this concept does not
exist in the code; circuits assume an abstract, fully connected register.

## Decision

Introduce a first-class **qubit resource model** in `qos-core`:

- **`Topology`** — a connectivity graph of physical qubits (adjacency). Standard
  constructors: `all_to_all` (simulator default), `linear`, `grid`, and arbitrary adjacency
  for real hardware. (A `ConnectivityMap` already exists in the kernel backend and is folded
  into this.)
- **`QubitAllocator`** — assigns a job's *logical* qubits to *physical* qubits, respecting
  the topology and (when available) calibration quality (ADR-0007). Produces a `Layout`
  (logical→physical mapping) consumed by the transpiler's routing stage (ADR-0008).
- Allocation is the **scheduling primitive**: the scheduler (ADR-0003) cannot dispatch a job
  until the allocator can satisfy its qubit/connectivity requirement on the chosen backend.

For the local simulator backend, `Topology = all_to_all` and allocation is trivial (identity
layout), so this model is inert there but exercised by the same code path as real hardware.

## Rationale

- Modeling qubits as a constrained, quality-aware resource is the defining responsibility of
  a QPU OS; building it now (even if trivial for the simulator) keeps the hardware path
  first-class rather than retrofitted.
- Sharing the same allocation path for simulator and hardware means the logic is continuously
  tested.

## Consequences

### Positive

- A clear place for connectivity-aware scheduling and quality-aware placement.
- Real hardware plugs in by supplying a `Topology` + calibration; no new control flow.

### Negative / Trade-offs

- Adds a stage the simulator does not strictly need; justified by parity and future-proofing.

### Neutral / Follow-ups

- Optimal layout selection (minimizing routing cost using calibration) is an optimization
  pass (ADR-0008); the initial allocator can be greedy/first-fit.
