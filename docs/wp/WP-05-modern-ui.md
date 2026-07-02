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
- [x] **Step 2 — Branded boot splash.** `scripts/gen_logo_mask.py` decodes
  `heptapus_logo_primary_black.png` (pure-stdlib PNG decode) → a 400×400 alpha coverage mask
  (`assets/heptapus_logo_mask.bin`) embedded via `include_bytes!`. `compositor::run_splash` plays a
  ~1.5 s animation on the dark gradient: logo fades in + scales up, holds, fades out, with an accent
  loading bar. Runs after init (heap/timer/framebuffer ready) in place of the text
  `wait_for_continue`. Verified in QEMU (screenshot): crisp white octopus + "HEPTAPUS GROUP" on the
  themed background, 0 faults, boot reaches ready. Fix: drain stale boot input first so a queued key
  doesn't instantly skip the splash. New primitive: `Surface::blit_mask_scaled`.
- [x] **Step 3 — TrueType fonts (E-71).** Embedded **Roboto Regular** (Apache-2.0; redistributable,
  org-compliant — `assets/LICENSE-Roboto.txt`). New `qos_ui::font`: parse offset table +
  head/maxp/hhea/hmtx/loca/glyf, cmap **format 4** char→glyph, simple **and composite** glyph
  outlines, quadratic-Bézier flattening, and a **4× supersampled nonzero-winding rasterizer** →
  antialiased coverage bitmaps, with a `FontRenderer` glyph cache + text layout (`text_width`,
  `draw_text`). **26 host tests** (parse, cmap, ink, space-advance, layout). Wired into the `modern`
  desktop: crisp top-bar menu ("QOS File Edit View Window Help" + clock), window titles, button
  labels. Verified in QEMU (screenshot) — sharp antialiased text at native resolution.
- [x] **Step 4 — Window manager (E-72).** Interactive `Desktop` in `compositor`: z-ordered windows
  (rounded + shadow + title bar + traffic-light dots + focus accent), a rendered arrow **cursor**,
  a **dock** with app icons + open-indicators, and a top-bar **light/dark toggle** pill. Input from
  the `input` queue: **mouse** drags windows by the title bar, clicks the red dot to close, clicks
  a dock icon to open/raise an app, clicks the pill to toggle theme; **keyboard** `1`–`4` open apps,
  `w` closes the focused window, `t` toggles theme, `Esc` exits. Redraws only on change (dirty
  flag). `run_demo` pumps `xhci::poll()` so USB HID stays live while it owns the loop. Verified in
  QEMU (screenshots): apps open via keys with distinct content, focus ring + dock dots, cursor
  tracks mouse motion, and both light + dark themes render at native 1280×800. (Full generic
  `Widget` trait/toolkit deferred; the WM covers the needed interactions.)
- [~] **Step 5 — Apps (E-73).** Five functional apps in the compositor WM:
  - **Terminal** — scrollback + input, Set-1 scancode→char with Shift, focus-based key routing,
    dirty-only-window redraw. `help/clear/echo/ver/mem` + `bell`/`ghz`/`qrng` run the real
    `quantum::sim` (verified `bell` → `00->495 11->505`).
  - **Files** — browses the real in-kernel fs (`fs::get_entries`); click dir to enter, `..` up,
    file to preview text. (Only the RAM fs so far — see "next".)
  - **Quantum Lab** — Run Bell / Run GHZ buttons → real simulator → measurement counts (verified
    click → `00->495 11->505`).
  - **System Monitor** — live RTC time + uptime, real kernel-heap used/total + bar, USB HID counts
    (`xhci::hid_device_counts`), real PCI device list (`pci::devices`), storage status; refreshes
    ~1 Hz while focused.
  - **Settings** — clickable light/dark toggle + live system info.
  Shared click geometry (`files_row_rect`/`qlab_btn_rect`/`settings_theme_rect`) drives draw +
  hit-test; `on_body_click` dispatches. **USB mouse fix:** `process_mouse_report` now diffs the
  button byte → real press AND release (fixed clicks + drag-release).
  - **Text Editor (6th app, dock 'E').** Open a file from Files → its bytes load into an editable
    buffer; keyboard edits it (chars / Backspace / Enter=newline); **Save** writes back via
    `fs::write`; **New** clears the buffer. Scrolling text area with a block cursor + status line.
  - **Files is now a real file manager.** A toolbar (**New File / New Dir / Rename / Delete /
    Edit**) + selection highlight + status line + a centered **naming modal**. Every op calls the
    real fs backend (`write`/`mkdir`/`rename`/`remove`) and refreshes the live listing. Also fully
    **keyboard-driven** (`n`/`k`/`r`/`x`/`e`); the naming modal captures typing (Enter commits, Esc
    cancels). Key routing precedence: naming modal → Terminal → Editor → desktop shortcuts; number
    keys `1`–`6` open the six dock apps. New geometry helpers `files_tool_rect`/`files_name_box`/
    `editor_btn_rect`. Verified in QEMU (0-fault, screenshots): keyboard New File → `hello.txt`,
    New Dir → `docs/`, both appear in the refreshed, correctly-sorted listing; Editor renders +
    accepts typing.
  - [ ] **Next (step 5 cont.):** attach a real **persistent data disk** to QEMU + make **Files
    browse real ATA/FAT16 or diskfs** (the "hard disk" — `ata`/`diskfs` exist; QEMU q35 needs an
    IDE/AHCI wiring decision since the PIO driver targets legacy ports 0x1F0); add **fs commands to
    the Terminal** (ls/cat/mkdir/rm/touch — makes it a real shell); more apps (calculator,
    devices/network panel, process viewer); deepen the quantum layer (visual circuit editor,
    RX/RY/RZ, histogram — MASTERPLAN E-80).

## Acceptance criteria

Per slice: a QEMU+OVMF screenshot shows the expected native-resolution result and the boot stays
0-fault through `QaOS ready`. Ultimately: QOS boots to an animated Heptapus splash, then a modern
themed desktop with a top bar + dock, crisp antialiased text, draggable windows, a light/dark
toggle, and working built-in apps driven by USB keyboard + mouse.

## Progress log

- **Boot flow.** After the splash a **boot chooser** (`compositor::boot_choice`) offers *Modern
  Desktop* vs *Terminal* — keyboard (Enter/D/1 vs S/2/Esc) or a mouse click on either card, with an
  ~8 s countdown that defaults to the desktop, so the UI comes up on its own. Choosing Desktop runs
  the interactive WM; `Esc` inside it drops to the shell (where `modern` relaunches it). Verified in
  QEMU: boot → Heptapus splash → welcome chooser → desktop.

## Notes & gaps

- Keep the legacy `gfxui`/VGA path as a fallback (ADR-0015 fallback-first) until the new stack fully
  replaces it; do not regress the current boot.
- Font license must be redistributable (OFL/Apache/etc.); record the exact face here when embedded.
- The Heptapus logo is first-party (Heptapus Group).
- **Bug (pre-existing, framebuffer text console):** `framebuffer::draw_char`/the VGA-text→framebuffer
  path renders text **horizontally mirrored** (visible in the shell/desktop text). The compositor
  (`blit_region`) is unaffected — the modern desktop + splash render correctly. Fix when the new UI
  replaces the text console, or sooner if it bothers the shell.
- **Cross-cutting concerns to carry through every UI slice** (per the user): **settings** (a real
  settings/preferences surface: theme, display, input, about), **security** (per-app boundaries,
  input isolation, no unchecked memory from untrusted data, least-privilege as user mode matures),
  **performance & efficiency** (dirty-rect blits instead of full-frame — `DirtyTracker` exists and
  should drive `blit_region` in the desktop/WM; glyph cache; avoid per-pixel locks; only redraw on
  change). Track these as acceptance checks in steps 4–5, not afterthoughts.
