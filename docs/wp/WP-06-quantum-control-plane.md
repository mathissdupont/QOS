# WP-06: Quantum control plane — serious engine + visual lab + QASM toolchain

- Status: 🟡 in progress
- Epic: E-80 (quantum control plane)
- ADRs: ADR-0019 (in-place statevector engine), ADR-0020 (CPU hardening — shipped together),
  ADR-0021 (in-OS QASM toolchain)
- Commits: d7f273c (engine + editor + hardening), a0f03e9 (QASM Studio + transpile + bridges)

## Goal

Make the quantum layer a **serious, first-class subsystem** of QOS — a performant simulation
engine with parametric gates, a real visual circuit editor with measurement histograms in the
modern UI, and (later) QHAL backends/transpilation — while every increment also holds the OS-wide
bars: security, performance, UX/UI quality (user directive 2026-07-02).

## Steps (verified slices)

- [x] **Engine v2 (ADR-0019).** In-place O(2^n) gate application (single, controlled, SWAP)
  replacing the O(4^n) expanded-operator path; parametric **RX/RY/RZ/P(θ)**; QASM2 parser support
  for `rx(pi/2)`-style angle expressions (`pi`, fractions, products, decimals, negatives);
  **MAX_QUBITS = 20** enforced at all entry points (a text file can no longer OOM the kernel);
  shots without mid-circuit measurement evolve once + sample N times (1000 shots ≈ 1 execution).
  `run_program()` lets the UI submit instruction lists directly (no QASM round-trip).
- [x] **Visual circuit editor + histogram (UI).** The Quantum Lab app is a real editor: gate
  palette (H/X/Y/Z/S/T/RX/RY/RZ/CX), Run/Clear/qubit±/angle-cycler controls, per-qubit wires with
  textbook gate rendering (labeled boxes; CX as control-dot + ⊕ target + connector), a cell
  cursor, **full keyboard editing** (arrows move, Space places/removes, letters pick gates,
  `r` cycles RX→RY→RZ, `a` cycles the angle in π/4 steps, Enter runs, Backspace clears, `w`
  closes), and a sorted **measurement histogram** (top 6 outcomes, accent bars + counts).
  A GHZ circuit ships pre-loaded so the first Run instantly demonstrates entanglement.
- [x] **Verification (QEMU, 0-fault).** GHZ run → `000: 529 / 111: 471`; placing RX(π/2) on q0
  via keyboard → the physically exact 4-way split (`000/100/011/111` ≈ 250 each). Serial shows
  `[SEC] hardening: NX on WP on ...` (ADR-0020 landed in the same slice).
- [x] **QASM toolchain (ADR-0021).** **QASM Studio** (10th app): source editor with keyword
  tinting, Compile (F4) → parse/validate + transpile stats or error, Run (F5) → optimized
  execution + inline histogram, Save (F2) → fs/`disk:`. **`quantum::transpile`**: self-inverse
  pair cancellation (overlap-aware, fixpoint) + circuit-depth analysis, shared by Studio and the
  Terminal's new **`qasm <file> [shots]`** command. **Bridges:** Quantum Lab exports its visual
  circuit as OpenQASM into the Studio (QASM button / `e`); Files opens `.qasm` in the Studio by
  extension. UX: **F10 closes the focused window from any app** (keyboard-escape invariant);
  number keys 1–9,0 open all ten apps. Verified in QEMU (0-fault, screenshots): F4/F5/F2 loop
  (Bell 505/495; `draft.qasm` 102 B saved), Lab→Studio GHZ export, Terminal
  `qasm quantum/bell.qasm` → "2 -> 2 gates (0 cancelled), depth 2" + 529/471.
- [ ] **Next (engine/back-end scope — this WP):** rotation-merge pass (RZ·RZ→RZ); controlled
  rotations (CRZ/CP); measurement/reset placement in the visual Lab; parser line numbers for
  compile errors (shared prerequisite with WP-07); noise models; QHAL backend abstraction
  (MASTERPLAN E-80).
- **Scope note:** the *editor/IDE* items formerly listed here (real text-editing core with a
  cursor, file sidebar, live circuit preview, problems panel) moved to **WP-07 (Quantum IDE)**,
  opened 2026-07-02 on the user's "VS Code-like environment" directive. WP-06 stays 🟡 and
  continues in parallel — it is done only when the engine roadmap above ships.

## Acceptance criteria

Each slice: QEMU screenshot shows the expected editor/histogram behavior, results are physically
correct (analytic distributions for known circuits), boot stays 0-fault, and docs (this WP + the
ADRs) record what shipped. Engine changes must keep the existing Terminal quantum commands
(`bell`/`ghz`/`qrng`) working.

## Notes & gaps

- Statevector memory is O(2^n) by nature — 20 qubits is the by-design ceiling (ADR-0019);
  beyond that needs stabilizer/tensor-network methods (future).
- The editor grid is 8 columns × ≤5 qubits (window-size bound); column count/scroll can grow
  when window resize lands.
- Cross-cutting (standing user directive): every new feature also considers **security**
  (validate inputs, bound allocations), **performance** (no O(4^n) shortcuts, damage-rect UI),
  **UX/UI** (keyboard + mouse parity, discoverable hints, real icons), and **docs** (WP + ADR).
