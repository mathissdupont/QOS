# ADR-0017: Modern UI — native-resolution compositor, TrueType fonts, widget toolkit

- Status: accepted
- Date: 2026-07-01
- Supersedes (UI portions of): ADR-0012 (desktop UX), ADR-0013 (VGA-then-VESA graphics path)
- Related: ADR-0014 (UEFI framebuffer), ADR-0015 (modern hardware & UI), WP-05

## Context

QOS boots to a UEFI linear framebuffer (e.g. 1280×800, 32-bit BGRX) but the existing desktop
(`gfxui`) draws onto a **320×200 logical canvas** that is integer-scaled and centered onto the real
screen (ADR-0013 / `qos-gfx::ScaleMap`). That was the right first step, but it produces a blocky,
low-resolution result: not a *modern* UI. The only font is an 8×8 bitmap; widgets are hand-painted
one-offs; there is no compositor, no double buffering in true color, no theming.

The product direction (ADR-0015) is a real, modern, general-purpose OS. The user has chosen:

- **Native-resolution, true-color rendering** (draw at the framebuffer's real resolution, 32-bit).
- **Visual style:** a macOS/GNOME hybrid — a top menu bar plus a bottom dock, soft shadows,
  rounded corners — with **both light and dark themes**, switchable at runtime.
- **Typography:** **TrueType, antialiased, runtime-scalable** fonts (not a fixed bitmap).
- **A branded animated boot splash** using the Heptapus Group logo.
- **A suite of built-in apps:** terminal, file manager, Quantum Lab, settings/system monitor, and
  the other apps a usable OS needs.

## Decision

Build a layered, mostly-portable UI stack, keeping the existing text-mode/`gfxui` path as a
fallback (fallback-first, per ADR-0015) until the new stack fully replaces it.

1. **Compositor core — new portable crate `qos-ui`** (`#![cfg_attr(not(test), no_std)]` + `alloc`,
   host-tested like `qos-gfx`/`qos-driver`/`qos-acpi`). Owns:
   - `Surface`: a true-color (`u32` `0x00RRGGBB`) pixel buffer at arbitrary size.
   - Primitives: `fill_rect`, `blend_rect` (alpha), `rounded_rect`, `blit`/`blit_blend`, `hline`/
     `vline`, vertical/linear **gradient**, and **drop-shadow** (blurred rounded rect).
   - `Rect` + `DirtyTracker`: accumulate changed regions so only damaged tiles are blitted.
   - `Theme`: light + dark palettes (bg, surface, text, accent, shadow, …) selected at runtime.
   All logic is pure and deterministic → unit-tested on the host.

2. **Kernel seam — `compositor` module.** Allocates a back `Surface` at the framebuffer's native
   resolution, composes the scene, and **blits damaged regions** to the UEFI framebuffer with a
   fast per-scanline path (convert `0x00RRGGBB` → the framebuffer's byte order in a tight loop),
   replacing the per-pixel/`ScaleMap` path for the modern desktop. Reads the real resolution/stride
   from the bootloader framebuffer info — nothing hardcoded, so it adapts to any screen.

3. **TrueType text — `qos-ui::text` (E-71).** Embed one TTF at build time; parse `head`/`cmap`/
   `loca`/`glyf`/`hmtx`; rasterize glyph outlines (quadratic Béziers) with scanline **coverage
   antialiasing**; cache rasterized glyphs per (char, px). Text layout (advance widths, runs) on
   top. No external font-engine dependency (keeps `no_std` clean and license-simple).

4. **Widget toolkit + window manager — `qos-ui::widget` (E-72).** A `Widget` trait
   (`measure`/`layout`/`on_event`/`paint`), a clip/scissor stack, z-ordered windows with rounded
   corners + shadows + title bars, and controls (button, label, text field, list, scrollbar). A
   `CompositorTask` drives it from the scheduler, consuming the existing `input` queue (keyboard +
   the new USB HID mouse from WP-04) — no input changes needed.

5. **Boot splash (branded).** The Heptapus logo ships as an **alpha coverage mask** generated from
   `heptapus_logo_primary_black.png` at build time (downscaled, one byte/pixel), embedded in the
   kernel. The splash tints the mask per theme and animates (fade-in + gentle scale) via the
   compositor before the desktop appears.

6. **Apps (E-73).** Each GUI capability ships with an app (per the "an app per GUI addition"
   working rule): terminal (wraps the shell), file manager (FAT16), Quantum Lab (modernizes the
   existing `QuantumApp`), settings/system monitor, and further built-ins as the toolkit matures.

## Consequences

- **Sharp, modern output** at native resolution with real typography; the retro 320×200 look is
  retired for the desktop (the `qos-gfx` ScaleMap remains for the legacy/VGA fallback path).
- **Portable, tested core:** rendering math, dirty tracking, font rasterization, and layout are
  host-unit-tested; only the thin framebuffer blit and scheduler wiring are kernel-only.
- **Memory:** a native-res true-color back buffer is ~4 MB at 1280×800 (`w*h*4`). The current 1 MiB
  kernel heap is too small; the heap grows (WP-05 step 0) to accommodate the back buffer + glyph
  cache + widget state. Sized from the detected resolution, not assumed.
- **Cost:** a TrueType rasterizer and a widget/WM layer are substantial. WP-05 delivers them in
  verified slices (compositor → splash → fonts → widgets/WM → apps), each screenshot-checked in
  QEMU, so the boot never regresses.
- **Licensing:** the embedded TTF must be a redistributable, permissively-licensed font (e.g. an
  OFL/Apache face); recorded in WP-05 and the crate. The Heptapus logo is first-party.
