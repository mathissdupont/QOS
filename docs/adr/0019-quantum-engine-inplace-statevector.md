# ADR-0019: Quantum engine — in-place statevector, parametric gates, bounded resources

- **Status:** Accepted
- **Date:** 2026-07-02
- **Deciders:** QOS core
- **Related ADRs:** ADR-0017 (modern UI hosts the Quantum Lab), MASTERPLAN epic E-80

## Context

QOS's quantum control plane is a first-class subsystem, not a demo. The original simulator was
correct but architecturally limited:

- Every gate materialized the **full 2^n × 2^n operator** (`expand_single_gate` +
  matrix-times-vector): O(4^n) time and memory per gate. At 10 qubits that is a 1M×1M-entry
  matrix — the design capped practical circuits at ~8 qubits and wasted the kernel heap.
- Only fixed Clifford+T gates (H/X/Y/Z/S/T/CX/CZ/SWAP): no **parametric rotations**, which real
  algorithms (VQE, QAOA, tomography) require.
- `shots` re-executed the whole circuit per shot even when nothing measured mid-circuit.
- No input validation: a QASM file declaring `qreg q[64]` would attempt a 2^64-amplitude
  allocation → heap exhaustion → kernel panic (a denial-of-service on the OS from a text file).

## Decision

1. **Gates apply in place on the statevector**: a 2×2 unitary walks the amplitude pairs that
   differ in the target bit (O(2^n) time, O(1) extra memory); controlled gates restrict to
   indices with the control bit set; SWAP exchanges amplitudes directly. No expanded operators.
2. **Parametric gates are first-class**: RX(θ), RY(θ), RZ(θ), P(θ) in the engine, the QASM2
   parser (`rx(pi/2) q[0];` with angle expressions `pi/2`, `-pi/4`, `3*pi/2`, decimals), and the
   Quantum Lab UI.
3. **Resources are bounded**: `MAX_QUBITS = 20` (16 MiB of amplitudes) enforced at every public
   entry point (`run_qasm2`, `run_program`); out-of-range programs fail with a parse error.
4. **Shot execution is amortized**: circuits without mid-circuit measurement evolve once and
   sample the final distribution N times; only measuring circuits re-execute per shot (collapse).

## Rationale

- **Performance:** O(2^n) per gate is the textbook statevector algorithm; it takes the practical
  ceiling from ~8 qubits to the memory-bound 20 and makes 1000-shot runs interactive in the UI.
- **Security/robustness:** the qubit cap turns an OOM panic into a clean error — kernel
  subsystems must never let user input size an allocation unchecked.
- **Capability:** parametric rotations unlock real variational/rotation circuits, matching the
  "serious quantum OS" goal rather than fixed Bell/GHZ demos.

## Consequences

### Positive

- >10-qubit circuits now feasible; UI runs are instant; heap usage per gate is zero.
- The engine API (`run_program`) lets the visual circuit editor skip the QASM round-trip.

### Negative / Trade-offs

- `linalg`'s expanded-operator helpers become legacy (kept for tests/reference).
- Statevector still O(2^n) memory by nature; >20 qubits needs different methods (tensor networks,
  stabilizer) — out of scope.

### Neutral / Follow-ups

- Next: controlled-phase/controlled-rotation gates, QASM export from the editor, noise models,
  QHAL backends (MASTERPLAN E-80).

## Alternatives considered

1. **Keep expanded operators, just cap qubits lower** — wastes memory/time for no benefit;
   rejected.
2. **Stabilizer (Clifford-only) simulator** — scales to hundreds of qubits but cannot express
   parametric rotations; wrong trade-off for a general lab. May come later alongside.
