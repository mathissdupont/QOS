# ADR-0012: Desktop / UX layer

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0002 (usable desktop is in scope), ADR-0003 (layers), ADR-0004 (QHAL)

## Context

ADR-0002 makes a usable, Windows-like experience an explicit goal. The kernel already has
text-mode desktop pieces (`desktop.rs`, `desktop_apps.rs`, `gui.rs`, `explorer.rs`,
`menu.rs`, `dialog.rs`, `mouse.rs`). We must define how this layer is structured so it does
not entangle with — or block the building/testing of — the quantum control plane.

## Decision

Treat the desktop/UX as an **L2/L3 layer (ADR-0003) that is modular and feature-gated**, and
that reaches the quantum subsystem **only through `qos-abi`** (`QosRequest`/`QosResponse`),
never by calling control-plane internals.

- **Structure:** a window/compositor layer, a shell, file management, and a first-class
  **Quantum app** (circuit editor, job monitor, backend/calibration manager). The Quantum app
  is the showcase of "quantum as a first-class citizen."
- **Feature flag:** the entire desktop sits behind a cargo feature (e.g. `desktop`). The
  control-plane core (`qos-core`, QHAL, scheduler) **must build and test with the desktop
  feature off**. This keeps the headless host daemon and CI lean and prevents UI churn from
  breaking the core.
- **Rendering:** **text-mode now** (it works today). **Pixel/framebuffer is a later upgrade**,
  gated on resolving the bootloader/graphics path; the UX code is written against a small
  drawing abstraction so the backend can switch without rewriting apps.
- **Boundary enforcement:** the desktop links `qos-abi` only. If it needs a new capability
  from the core, that capability is added to `qos-abi` first.

## Rationale

- The `qos-abi`-only boundary keeps the system usable headless and makes the GUI swappable,
  and it is the same boundary a future remote client would use.
- Feature-gating protects the core's build/test from the largest, most volatile surface.
- Starting text-mode preserves working functionality; the drawing abstraction avoids a
  rewrite when pixels arrive.

## Consequences

### Positive

- The desktop becomes a clean client of the control plane, not a tangle inside the kernel.
- Core CI stays fast and the headless daemon stays minimal.
- A clear, demoable "Quantum app" anchors the product story.

### Negative / Trade-offs

- Some existing desktop code must be refactored to go through `qos-abi` instead of direct
  calls.
- Maintaining a UI layer is real ongoing cost (acknowledged in ADR-0002).

### Neutral / Follow-ups

- The drawing abstraction (text-mode vs framebuffer) is a small interface to be defined when
  the desktop is refactored behind its feature flag.
