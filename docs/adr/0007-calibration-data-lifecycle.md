# ADR-0007: Calibration / characterization data lifecycle

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0004 (fetch_calibration), ADR-0006 (allocator uses quality), ADR-0008 (routing cost), ADR-0010 (mitigation)

## Context

Real QPUs are not static: gate fidelities, readout errors, and coherence times (T1/T2) drift
and are re-measured periodically. Good placement (ADR-0006), routing (ADR-0008), and error
mitigation (ADR-0010) all depend on **current** calibration data. The kernel has an
`ErrorRates` struct but no store, no freshness tracking, and no refresh path.

## Decision

Introduce a **`CalibrationStore`** per backend in `qos-core`:

- Holds: per-qubit single-gate error, per-pair two-qubit error, readout error matrices,
  T1/T2, and a **timestamp** of when it was measured.
- Populated by the QHAL `fetch_calibration()` (ADR-0004): the simulator supplies an ideal
  (error-free) calibration; cloud backends fetch real calibration via the provider API
  (through the ADR-0011 proxy); a future local QPU measures it.
- **Freshness:** calibration carries an age; a backend whose calibration is older than a
  configured threshold reports `NeedsCalibration` and the scheduler may refresh before
  dispatch.
- **Consumers:** the `QubitAllocator` (quality-aware placement), the transpiler routing pass
  (edge cost), and the error mitigator (readout-error inversion).

## Rationale

- Calibration is the data that makes hardware-aware decisions correct; centralizing its
  lifecycle (fetch, store, age, consume) avoids each component inventing its own copy.
- An ideal calibration for the simulator keeps the same code path live in development.

## Consequences

### Positive

- One source of truth for hardware quality data, with explicit staleness.
- Mitigation and routing become data-driven rather than hard-coded.

### Negative / Trade-offs

- Another per-backend structure to populate; trivial for the simulator, real work for cloud
  backends (parsing provider calibration payloads).

### Neutral / Follow-ups

- The exact provider calibration payload parsing is per-backend and lands with ADR-0011 work.
