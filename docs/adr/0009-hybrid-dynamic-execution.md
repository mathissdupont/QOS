# ADR-0009: Hybrid / dynamic execution and real-time feedback model

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0005 (QASM3), ADR-0004 (QHAL execution), ADR-0010 (QEC not precluded)

## Context

Modern quantum workloads are **hybrid**: a circuit may measure a qubit mid-execution, store
the bit in a classical register, and apply later gates **conditionally** on that bit
("dynamic circuits"). Variational algorithms (VQE/QAOA) wrap the QPU in a classical
optimization loop. On real hardware, the classical feedback inside a circuit must complete
within the qubits' coherence window — a **hard real-time** constraint. Today the simulator
computes a full statevector and samples at the end; it has no mid-circuit measurement,
classical registers as live data, or conditional execution.

## Decision

Adopt a **stepwise execution model** with a typed classical/quantum boundary:

- The executor walks the `Circuit` gate-by-gate (the simulator already does this internally),
  maintaining **live classical registers**. `Measure` writes a classical bit; conditional
  gates read classical bits and execute or skip accordingly; `Reset` is supported.
- Define an **execution interface** in the QHAL (ADR-0004) that supports this stepwise model
  plus an optional **classical callback** for the host-loop hybrid case (run circuit → return
  results → classical code computes new parameters → resubmit).
- **Real-time stance:** for the simulator and host daemon, feedback is *best-effort* (no
  coherence clock). For a future real-QPU controller embodiment it is *hard real-time*; the
  architecture therefore keeps the in-circuit classical path **minimal and bounded** (no
  heap-heavy work on the feedback path) so a real-time implementation is possible later.

## Rationale

- Stepwise + live classical registers is the minimum model that supports dynamic circuits and
  is the natural extension of the existing gate-by-gate simulator.
- Separating *in-circuit* feedback (latency-critical) from *host-loop* feedback (relaxed)
  lets us serve VQE/QAOA now while not designing out hard real-time later.

## Consequences

### Positive

- Dynamic circuits and variational loops become expressible and runnable on the simulator.
- The feedback-path discipline keeps a future real-time controller feasible.

### Negative / Trade-offs

- Stepwise execution with conditionals is more complex than end-of-circuit sampling and needs
  careful measurement/collapse semantics.

### Neutral / Follow-ups

- Requires QASM3 (ADR-0005) to express classical control flow at the source level.
- The classical-register data model is shared with the QEC path kept open by ADR-0010.
