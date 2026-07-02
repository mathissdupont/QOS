# WP-07: Quantum IDE — a VS Code-like environment for quantum circuits

- Status: 🟡 in progress
- Epic: E-80 (quantum control plane), E-73 (apps)
- ADRs: ADR-0021 (in-OS QASM toolchain)
- Commits: (appended as delivered)

## Goal

Evolve QASM Studio into a **VS Code-like quantum IDE**: a real text-editing core (cursor,
line-based editing, line numbers, current-line highlight), a **file sidebar** for `.qasm`
sources, a **live circuit preview** that renders the code as a diagram while you type, and a
status/problems bar — so writing quantum programs in QOS feels like a modern editor, not a
teletype.

## Steps (verified slices)

- [ ] **Slice 1 — editing core + layout.** Line-based buffer (`Vec<String>`) with a real cursor
  (arrows/Home-ish navigation, insert/delete/split at the cursor), line-number gutter,
  current-line highlight, scroll-to-cursor; VS Code-style layout: sidebar (workspace `.qasm`
  files, click to open), code pane, live circuit preview strip (reparsed on each edit; parse
  errors show a problem marker), status bar. Larger default window for the IDE.
- [ ] **Slice 2 — problems + navigation.** Parser line numbers (extend `ParseError`), a problems
  panel listing errors with their line, click-to-jump; click-to-position in the code pane.
- [ ] **Slice 3 — shared editing core.** Factor the editing core out (module or `qos-ui`) and
  adopt it in the Text Editor; selections + clipboard.
- [ ] **Slice 4 — IDE affordances.** Per-token syntax highlighting; autocompletion of gate
  names; snippets (Bell/GHZ/QFT templates); QASM import into the visual Lab (code → circuit).

## Acceptance criteria

Per slice: QEMU screenshots show the layout/behavior; typing/navigation is cursor-accurate;
the preview matches the code (verified against known circuits); compile/run keep working
(F4/F5/F2); boot stays 0-fault.

## Notes & gaps

- Proportional font ≈ approximate column metrics; acceptable for slice 1 (gutter + cursor line
  are line-accurate), refined when click-to-position lands.
- The preview renders up to the window's qubit/column budget; larger circuits clip with a note.
