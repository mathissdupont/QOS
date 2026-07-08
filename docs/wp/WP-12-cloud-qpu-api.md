# WP-12: Cloud QPU connectivity — QHAL backends + external provider API

- Status: 🔴 not started (blocked on WP-10 slice 1-3: a working NIC + TCP/HTTP)
- Epic: E-80/E-81 (quantum control plane, cloud QPU access)
- ADRs: ADR-0011 (cloud proxy concept), ADR-0019/0021 (local engine/toolchain); new ADR for the
  QHAL + provider-protocol decisions
- Commits: (appended as delivered)

## Goal

Let QOS run circuits on **real external quantum providers** (IBM Quantum, IonQ, etc.) through a
first-class **QHAL (Quantum Hardware Abstraction Layer)**: one backend interface with the local
statevector simulator as backend #0 and remote providers as peers — selectable from the Quantum
IDE/Lab, with job submission, queue status and result retrieval inside the OS.

## Steps (planned slices)

- [ ] **Slice 1 — QHAL trait + local backend.** `QuantumBackend` interface (capabilities:
  qubits/gate set/shots; submit/poll/result as async-style jobs); the existing simulator becomes
  `LocalSimBackend`; the IDE/Lab/Terminal run through the QHAL instead of calling `sim` directly.
  (The legacy `backend.rs` scaffolding is reviewed/merged here.)
- [ ] **Slice 2 — provider protocol (ADR).** Decide the wire approach per ADR-0011: an in-OS
  HTTPS client vs a thin local proxy that terminates TLS (kernel TLS is a heavy dependency —
  honest trade-off documented). Define the provider adapter API (auth token, submit QASM,
  job id, poll, counts).
- [ ] **Slice 3 — first remote provider.** One concrete adapter (e.g. IBM Quantum's REST API)
  end-to-end against the real service (or its sandbox): submit from the IDE, show queue state in
  the status bar, render returned counts in the histogram. Depends on WP-10 TCP/HTTP (+TLS
  decision).
- [ ] **Slice 4 — UX.** Backend picker in the IDE/Lab ("Local sim · IBM · …"), per-backend
  capability display (max qubits, native gates), transpile-to-target using the WP-06 passes,
  token management in Settings (stored on the persistent disk via `sysconfig`).
- [ ] **Slice 5 — resilience.** Offline queueing of jobs, retry policy, result caching to disk;
  clear error surfaces when the network is down (fallback-first).

## Acceptance criteria

Per slice: verified in QEMU (mock server on the host for slices 1-2; real/sandbox provider for
slice 3+); the local backend keeps working with zero network; secrets never logged; WP/ADR
updated with evidence.

## Notes & gaps

- Hard dependency: WP-10 networking (NIC driver, DHCP, TCP, HTTP; TLS decision in its slice 4).
- Security: provider tokens are secrets — store on disk with restricted surface, never print;
  input validation on all provider responses (ADR-0020 principle).
