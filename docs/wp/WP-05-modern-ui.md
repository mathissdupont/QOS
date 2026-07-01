# WP-05: Modern UI — compositor, TrueType fonts, widgets, apps

- Status: 🟡 in progress
- Epic: E-70 (compositor), E-71 (scalable fonts), E-72 (widgets/WM), E-73 (apps)
- ADRs: ADR-0017 (modern UI), ADR-0015 (modern hardware & UI)
- Commits: (appended as delivered)

## Goal

Replace the retro 320×200 scaled desktop with a **modern**, native-resolution, true-color UI:
a compositor, antialiased TrueType text, a macOS/GNOME-hybrid shell (top bar + dock) with **light
and dark themes**, a branded **animated Heptapus boot splash**, and a suite of built-in apps.
Everything universal (reads resolution/stride from the framebuffer) and, where possible, portable +
host-tested.

## Design decisions (from the user)

- Native-resolution, true-color (32-bit) rendering — no more 320×200 upscaling for the desktop.
- Style: macOS + GNOME hybrid (top menu bar + bottom dock), **both light and dark**, runtime toggle.
- Fonts: TrueType, antialiased, runtime-scalable.
- Animated boot splash with the Heptapus Group logo (`heptapus_logo_primary_black.png` → alpha mask).
- Apps: terminal, file manager, Quantum Lab, settings/system monitor, + other built-ins.

## Steps (verified slices)

- [x] **Step 0 — Heap headroom.** Grew the kernel heap 1 MiB → **64 MiB** (fits a native-res
  back buffer + glyph cache + app surfaces). Growing it exposed an **O(n²)** frame allocator
  (`usable_frames().nth(next)` rebuilt+skipped the whole iterator each call), which made mapping
  16 384 heap pages take minutes; rewrote it as an **O(1)** region cursor. Boot reaches
  `QaOS ready` fast, 0-fault.
- [x] **Step 1 — Compositor core (E-70).** New portable crate `qos-ui`: `Surface`, `Rect`,
  `DirtyTracker`, primitives (fill/blend/rounded-rect with AA/blit/blit_mask/gradient/drop-shadow),
  `Theme` (light+dark). **19 host unit tests** pass. Kernel `compositor` module + fast per-scanline
  framebuffer blit (`framebuffer::blit_region`). Verified in QEMU (`modern` shell command):
  gradient wallpaper, translucent top bar + dock, two overlapping rounded windows with soft drop
  shadows and macOS traffic-light dots, colorful dock tiles — at native **1280×800 true-color**;
  `t` toggles light/dark live. 0 faults.
- [ ] **Step 2 — Branded boot splash.** Build-time logo→alpha-mask asset embedded in the kernel;
  animated fade-in + scale on a themed background before the desktop. Screenshot-verify.
- [ ] **Step 3 — TrueType fonts (E-71).** Embed a permissively-licensed TTF; parse
  head/cmap/loca/glyf/hmtx; rasterize outlines with coverage AA; glyph cache; text layout. Host
  tests for parsing + a known glyph's coverage. Verify crisp text on screen.
- [ ] **Step 4 — Widgets + window manager (E-72).** `Widget` trait, clip stack, z-ordered windows
  (rounded + shadow + title bar), controls (button/label/textfield/list/scrollbar), top bar + dock,
  light/dark toggle. `CompositorTask` drives it from the scheduler using the `input` queue
  (keyboard + USB HID mouse). Verify: draggable window, working buttons, theme switch.
- [ ] **Step 5 — Apps (E-73).** Terminal (wraps the shell), file manager (FAT16), Quantum Lab
  (modernizes `QuantumApp`), settings/system monitor, + further built-ins. Each app screenshot-
  verified.

## Acceptance criteria

Per slice: a QEMU+OVMF screenshot shows the expected native-resolution result and the boot stays
0-fault through `QaOS ready`. Ultimately: QOS boots to an animated Heptapus splash, then a modern
themed desktop with a top bar + dock, crisp antialiased text, draggable windows, a light/dark
toggle, and working built-in apps driven by USB keyboard + mouse.

## Progress log

- (pending first delivery)

## Notes & gaps

- Keep the legacy `gfxui`/VGA path as a fallback (ADR-0015 fallback-first) until the new stack fully
  replaces it; do not regress the current boot.
- Font license must be redistributable (OFL/Apache/etc.); record the exact face here when embedded.
- The Heptapus logo is first-party (Heptapus Group).
