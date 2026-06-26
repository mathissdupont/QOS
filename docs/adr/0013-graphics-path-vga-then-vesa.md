# ADR-0013: Graphics path — VGA Mode 13h now, VESA framebuffer (bootloader 0.11+) later

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** QOS team
- **Related ADRs:** ADR-0012 (desktop/UX layer), ADR-0002 (Windows-like OS)

## Context

ADR-0012 wants a real pixel/framebuffer GUI eventually. Verified state of the code:

- The kernel uses `bootloader = "=0.9.29"`, which **does not expose a framebuffer**.
- `crates/qos-os-kernel/src/framebuffer.rs` already implements a full software drawing
  library — `put_pixel`, `fill_rect`, `draw_line`, an 8×8 font (`draw_char`/`draw_string`),
  and a `FrameBufferWriter`. But `framebuffer::init()` is a no-op: it never installs a real
  buffer, so `FRAMEBUFFER` stays `None` and every draw call silently does nothing. The
  current visible UI is therefore VGA **text mode** only.

So the only missing piece for pixels is *a real framebuffer to point the existing drawing code
at*. There are two ways to obtain one:

- **(A) VGA Mode 13h** — switch to 320×200×256 by programming the VGA registers directly from
  the kernel (port I/O), giving a linear 8-bpp framebuffer at physical `0xA0000`. No bootloader
  change, no real-mode BIOS calls, kernel-only.
- **(B) VESA/GOP framebuffer via bootloader 0.11+** — the modern `bootloader` crate sets a
  high-resolution true-color mode and hands the kernel a framebuffer in `BootInfo`. This
  requires migrating the boot/build pipeline (the `entry_point!`/`BootInfo` API changed
  completely, `cargo bootimage` is replaced by the `bootloader` build crate, and the memory
  init that reads `physical_memory_offset`/`memory_map` must be rewritten).

## Decision

Pursue graphics in **two phases**:

- **Phase 1 (now): VGA Mode 13h.** Add a mode-set routine (VGA register programming) and point
  the existing `framebuffer.rs` primitives at the `0xA0000` linear buffer with an 8-bpp
  palette format. This yields **real pixels and a real graphical desktop demo** without
  touching the working boot or build pipeline. It is contained and low-risk.
- **Phase 2 (later): bootloader 0.11+ / VESA framebuffer.** Migrate the boot pipeline to get a
  high-resolution, true-color framebuffer. Because `framebuffer.rs` abstracts drawing behind
  `put_pixel`/`draw_*`, the desktop and apps switch backends with minimal change. This
  migration is done on a **branch**, since it changes the build commands and risks regressing
  the boot we just got working.

The drawing API in `framebuffer.rs` is the stable seam between both phases; UI code (ADR-0012)
targets that API and is agnostic to which backend (Mode 13h vs VESA) is active.

## Rationale

- Phase 1 delivers a visible pixel GUI **soon** and reuses the already-written drawing code,
  maximizing motivation/feedback at minimal risk.
- Phase 2's heavy, pipeline-changing migration is deferred until it can be done carefully,
  rather than gating all pixel graphics behind it.
- Keeping the `framebuffer.rs` API stable means Phase 1 work is not thrown away in Phase 2.

## Consequences

### Positive

- Real pixels without disturbing the working boot; fast visual progress.
- The desktop/UX track (ADR-0012) can start against a live framebuffer immediately.

### Negative / Trade-offs

- Mode 13h is low resolution (320×200) and 256-color — a stepping stone, not the final look.
- Two backends exist until Phase 2 lands; mitigated by the shared drawing API.

### Neutral / Follow-ups

- Phase 1 needs a palette setup for Mode 13h (the `put_pixel` BGR path is for Phase 2's
  true-color buffer; Mode 13h writes 8-bit palette indices, so `framebuffer.rs` gains a
  format switch).
- Phase 2 will get its own implementation notes when the bootloader migration begins.
