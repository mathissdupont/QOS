# WP-07: Quantum IDE — a VS Code-like environment for quantum circuits

- Status: ✅ done (v1; deferred niceties tracked as gap G-17)
- Epic: E-80 (quantum control plane), E-73 (apps)
- ADRs: ADR-0021 (in-OS QASM toolchain)
- Commits: 78ee1ef (slice 1: editing core + sidebar + live preview), a0b3753 (slice 2:
  line-numbered errors + problems row + jump-to-problem), 9125d8f (slices 3+4: shared editor
  core + token highlighting + templates)

## Goal

Evolve QASM Studio into a **VS Code-like quantum IDE**: a real text-editing core (cursor,
line-based editing, line numbers, current-line highlight), a **file sidebar** for `.qasm`
sources, a **live circuit preview** that renders the code as a diagram while you type, and a
status/problems bar — so writing quantum programs in QOS feels like a modern editor, not a
teletype.

## Steps (verified slices)

- [x] **Slice 1 — editing core + layout.** Line-based buffer (`Vec<String>`) with a real cursor:
  arrow navigation (column clamping, line-end wrapping), insert/backspace/newline **at the
  cursor** (UTF-8-safe), 32 KiB cap; line-number gutter, current-line highlight, accent caret,
  scroll-to-cursor. VS Code layout in a 760×560 window: EXPLORER sidebar (workspace `.qasm`
  files, open-file highlight, click to open), code pane with keyword tinting, **live circuit
  preview** (reparse on every edit → wire diagram with gate boxes / CX dot+target / M markers;
  parse problems as an inline `(!)` row; last run counts right-aligned), status bar with
  compiler status + `Ln X, Col Y`. Verified in QEMU (0-fault): caret/status agree after
  mid-buffer edits; inserting `z q[0];` updates the preview instantly and F5 gives the
  physically correct unchanged Bell split (Z = phase only). A transient caret/status mismatch
  was diagnosed as a torn-frame screendump (screenshot raced the framebuffer blit), not a bug.
- [x] **Slice 2 — problems + navigation.** `ParseError` is now `{ line, kind }` (1-based source
  line at every error site, computed lazily; `.message()` renders `line N: …`, 0 = program-level);
  all consumers migrated (sim, IDE, Terminal `qasm`). IDE diagnostics: the problems row shows
  `(!) line N: message` with a jump hint, **click or F8** moves the cursor to the offending line,
  the problem line's **gutter number turns red**, and clicking the code pane positions the cursor
  (line-exact; column via average glyph advance — per-glyph metrics later). Verified in QEMU
  (0-fault): broken `cx q[0]` → `(!) line 8: syntax: expected ','` + red gutter 8; F8 after
  wandering → `Ln 8, Col 1`. Harness note: TCG queues keystrokes — settle before screendump.
- [x] **Slice 3 — shared editing core (9125d8f).** The editing core is factored into module-level
  `ed_*` helpers (byte-safe insert/delete/split, wrap-aware movement, set/get text) used by BOTH
  the IDE and the **Text Editor**, which now has the same cursor editing, current-line highlight,
  caret and `Ln, Col` indicator. *(Deferred → G-17: selections + clipboard.)*
- [x] **Slice 4 — IDE affordances (9125d8f).** **Per-token syntax highlighting** (keywords teal,
  gate names purple, numbers/`pi` amber, comments dim) and a **TEMPLATES** sidebar section
  (Bell, GHZ-3, CRZ demo — the CRZ demo carries two mergeable RZs so Compile shows the optimizer
  working). *(Deferred → G-17: autocompletion; QASM import into the visual Lab.)*

## Acceptance criteria

Per slice: QEMU screenshots show the layout/behavior; typing/navigation is cursor-accurate;
the preview matches the code (verified against known circuits); compile/run keep working
(F4/F5/F2); boot stays 0-fault.

## Notes & gaps

- Proportional font ≈ approximate column metrics; acceptable for slice 1 (gutter + cursor line
  are line-accurate), refined when click-to-position lands.
- The preview renders up to the window's qubit/column budget; larger circuits clip with a note.
