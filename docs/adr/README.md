# Architecture Decision Records (ADR)

This directory is the permanent record of QOS's architectural decisions.

## What is an ADR?

An **Architecture Decision Record** is a short document that captures a single
architectural decision together with its *context* and *consequences*. The goal is to
preserve the answer to "why did we do it this way?" in a readable form, independent of
the code and the git history.

## Rules

1. Each ADR is numbered `NNNN-kebab-case-title.md` (monotonic; numbers are never reused).
2. Template: [`template.md`](template.md).
3. Once an ADR is `Accepted` it is **immutable**. If the decision changes, write a new ADR
   and mark the old one `Superseded by ADR-XXXX`.
4. Statuses: `Proposed` → `Accepted` → (`Superseded` | `Deprecated`).
5. ADRs must not contradict the code. On a conflict, update either the code or the ADR
   (via a new ADR).

## Index

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-adopt-adr-process.md) | Adopt the ADR process | Accepted |
| [0002](0002-qpu-control-plane-os.md) | Frame QOS as a quantum-first OS (desktop + control plane + cloud) | Accepted |
| [0003](0003-layered-architecture-single-core.md) | Layered architecture and a single `no_std` core runtime | Accepted |
| [0004](0004-qhal-quantum-hardware-abstraction.md) | QHAL — Quantum Hardware Abstraction Layer | Accepted |
| [0005](0005-canonical-ir.md) | Canonical IR — OpenQASM 3 external, typed Circuit internal | Accepted |
| [0006](0006-qubit-resource-and-topology.md) | Qubit resource and topology model | Accepted |
| [0007](0007-calibration-data-lifecycle.md) | Calibration / characterization data lifecycle | Accepted |
| [0008](0008-transpilation-pipeline.md) | Transpilation pipeline | Accepted |
| [0009](0009-hybrid-dynamic-execution.md) | Hybrid / dynamic execution and real-time feedback | Accepted |
| [0010](0010-error-mitigation-now-qec-later.md) | Error mitigation now, QEC later | Accepted |
| [0011](0011-cloud-connectivity-tls-proxy.md) | Quantum cloud connectivity via a host TLS proxy | Accepted |
| [0012](0012-desktop-ux-layer.md) | Desktop / UX layer | Accepted |
| [0013](0013-graphics-path-vga-then-vesa.md) | Graphics path — VGA Mode 13h now, VESA later | Accepted |

## Implementation order

The ADRs are accepted as a set. Implementation proceeds bottom-up:

1. **ADR-0003 + ADR-0004** — port `qos-core` to `no_std` and define the QHAL (foundation).
2. **ADR-0005 + ADR-0006** — move the `Circuit` IR into the core; add topology/allocation.
3. **ADR-0008** — transpilation pipeline (identity for the simulator first).
4. **ADR-0009 + ADR-0010** — stepwise/dynamic execution; readout-error mitigation.
5. **ADR-0011** — re-enable `qosd`, add the TLS proxy and real cloud backends.
6. **ADR-0012** — refactor the desktop behind its feature flag and the `qos-abi` boundary.
