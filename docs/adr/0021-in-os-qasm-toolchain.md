# ADR-0021: In-OS QASM toolchain (Studio IDE + transpile passes + app routing)

- **Status:** Accepted
- **Date:** 2026-07-02
- **Deciders:** QOS core
- **Related ADRs:** ADR-0019 (quantum engine), ADR-0017 (modern UI)

## Context

The user's goal is to maximize the **quantum developer experience** inside QOS itself — "Qiskit
gibi kodlama dillerinin compiler ve editörlerini de yapalım". Pieces existed but were siloed:
the engine ran QASM byte-strings, the Quantum Lab edited circuits visually, the Text Editor
edited any text, and the Terminal had fixed demos. There was no way to *write a quantum program
as code, compile it with feedback, optimize it, run it, and save it* — the loop a real quantum
SDK provides.

## Decision

Build the toolchain as **native OS integration**, not a monolithic app:

1. **QASM Studio** — a dedicated IDE app: QASM source buffer with keyword tinting, `Compile`
   (F4) → parse + validate + transpile stats or error, `Run` (F5) → optimized execution +
   inline histogram, `Save` (F2) → the same fs/disk backends every app uses.
2. **`quantum::transpile`** — compiler passes as a kernel library, shared by every front end
   (Studio, Terminal): self-inverse pair cancellation (overlap-aware, to fixpoint) and circuit
   **depth** analysis. Front ends report `gates M → M' (R cancelled), depth D`.
3. **Bridges, not copies**: the Quantum Lab serializes its visual circuit to OpenQASM source and
   opens it in the Studio (one editing model feeding the other); Files routes `.qasm` sources to
   the Studio by extension (other files keep opening in the Text Editor); the Terminal gets
   `qasm <file> [shots]` running the same parse→optimize→run pipeline.
4. **UX invariant**: keyboard-first parity — F-keys for toolchain actions, and a universal
   **F10 = close focused window** escape hatch, because text-entry apps consume letter keys and
   previously trapped keyboard-only users.

## Rationale

- One engine + one transpile library + many surfaces (IDE, visual editor, shell) is the Qiskit
  architecture in miniature — and avoids three divergent implementations.
- Extension-based app routing and shared fs backends make quantum sources ordinary files:
  editable in any editor, persistable to the SATA disk, runnable from the shell.
- Depth/gate-count stats are the metrics real hardware queues price by; showing them at compile
  time teaches users what the optimizer did.

## Consequences

### Positive

- Full in-OS loop verified: template → F4 (stats) → F5 (Bell 505/495 histogram) → F2
  (`draft.qasm`); Lab GHZ → generated QASM in Studio; `qasm quantum/bell.qasm` in the shell.
- New passes (rotation merging, gate fusion, mapping) have one home with every surface
  benefiting.

### Negative / Trade-offs

- The Studio's editor is append/backspace-only (no cursor movement/selection yet) — fine for
  small programs, needs a real text-editing core later (shared with the Text Editor).
- `ParseError` lacks line numbers; compile errors name the kind but not the location yet.

### Neutral / Follow-ups

- Next: parser line numbers, rotation-merge pass (RZ·RZ → RZ), QASM import in the Lab
  (code → visual), QHAL backend selection in the Studio, syntax highlighting per-token.

## Alternatives considered

1. **One mega quantum app** (editor+circuit+runner in a single window) — poor separation, blocks
   reuse by the Terminal; rejected in favor of library + bridges.
2. **A new custom language instead of OpenQASM 2.0** — throws away ecosystem familiarity and the
   existing parser; QASM 2.0 is the lingua franca (QASM 3 later).
