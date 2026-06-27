# ADR-0005: Canonical IR — OpenQASM 3 external, typed Circuit internal

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0004 (QHAL consumes the IR), ADR-0008 (transpiler passes), ADR-0009 (dynamic circuits)

## Context

Quantum jobs enter QOS as source text and must flow through validation, transpilation, and
execution. Today there is a partial OpenQASM 2 parser and a typed `Circuit`/`Gate`
representation in the kernel. `qos-abi::IrFormat` already enumerates `OpenQasm2`,
`OpenQasm3`, and `JsonIrV1`. We need one canonical story so passes and backends agree.

## Decision

- **External / wire IR:** OpenQASM is the primary submission format. **OpenQASM 2** is
  supported now; **OpenQASM 3 is the target** because dynamic circuits (classical registers,
  control flow, mid-circuit measurement — ADR-0009) require it. `JsonIrV1` remains an
  alternate machine-friendly wire format (already in `qos-abi`).
- **Internal IR:** a **typed `Circuit`** (an ordered list of typed `Gate`s over qreg/creg)
  is the single in-memory representation that every pass and every QHAL backend operates on.
  QASM/JSON are only serializations at the edges.
- Parsing direction is always: source (QASM/JSON) → `Circuit`. Backends that need a provider
  format serialize `Circuit` → that format themselves (ADR-0004).

## Rationale

- A typed internal IR makes transpilation passes (ADR-0008) and validation tractable and
  testable; string-level manipulation does not scale.
- Committing to QASM3 as the target aligns with the dynamic-circuit execution model and the
  broader quantum ecosystem.

## Consequences

### Positive

- One representation for passes and backends; clear parse-once boundary.
- Ecosystem compatibility (QASM is the lingua franca).

### Negative / Trade-offs

- A full QASM3 parser is a non-trivial build-out; QASM2 remains the working subset until then.

### Neutral / Follow-ups

- The `Circuit`/`Gate` type currently in the kernel moves into `qos-core` (with the QHAL) so
  it is shared (ADR-0003).
- Classical-compute constructs needed for QASM3 dynamic circuits are specified alongside
  ADR-0009.
