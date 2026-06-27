# ADR-0010: Error mitigation now, QEC later

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0007 (calibration feeds mitigation), ADR-0009 (stepwise path enables QEC)

## Context

Near-term ("NISQ") quantum hardware is noisy. Two distinct approaches exist:

- **Error mitigation** — cheap, post-hoc statistical correction of results (no extra qubits).
- **Quantum error correction (QEC)** — encodes logical qubits across many physical qubits and
  runs real-time syndrome extraction + classical decoding; vastly more demanding.

The kernel has an `ErrorMitigator` stub but no working mitigation, and no QEC. We must decide
how much to build now without painting ourselves into a corner.

## Decision

- **Now: error mitigation.** Implement, behind `JobOptions.error_mitigation`:
  - **Measurement (readout) error mitigation** via the calibration readout matrices
    (ADR-0007) — invert/least-squares correct the measured counts.
  - **Zero-noise extrapolation (ZNE)** hooks (run at scaled noise, extrapolate to zero).
  Mitigation operates on results/metadata and does not change the execution model.
- **Later: QEC is explicitly out of near-term scope**, but the architecture must not preclude
  it. The enabling pieces are already decided: the stepwise executor with a low-latency
  classical path (ADR-0009) and live classical registers are exactly what a syndrome-decoding
  loop needs. No QEC-specific structures are built now.

## Rationale

- Mitigation gives real accuracy improvements on cloud/NISQ backends for modest effort and
  exercises the calibration pipeline.
- QEC is a large research-grade effort; committing now would be premature, but designing the
  execution model (ADR-0009) so it *could* host QEC later is nearly free.

## Consequences

### Positive

- Usable accuracy gains for cloud QPU jobs in the near term.
- Calibration data (ADR-0007) gets a concrete consumer.

### Negative / Trade-offs

- Mitigation can mislead if calibration is stale; tie it to calibration freshness (ADR-0007).

### Neutral / Follow-ups

- A QEC ADR will be written if/when logical qubits become a goal; this ADR only commits to
  *not blocking* it.
