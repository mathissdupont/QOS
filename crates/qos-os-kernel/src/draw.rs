//! Resolution-agnostic drawing facade for the graphical desktop (ADR-0014 Stage 3).
//!
//! The desktop UI (`gfxui`) is authored against a fixed **logical** 320×200, 16-color canvas —
//! the VGA Mode 13h coordinate space (ADR-0013). This module is the single seam that lets that
//! same UI code render either way:
//!
//! - **Linear framebuffer active** (bootloader 0.11 / UEFI, the modern path): the logical canvas
//!   is drawn onto the real framebuffer **integer-scaled and centered** ([`qos_gfx::ScaleMap`]),
//!   in true color via the 16-entry palette. This is what makes the desktop appear under UEFI,
//!   where the legacy VGA hardware / `0xA0000` window does not exist.
//! - **No framebuffer** (legacy BIOS fallback): calls pass straight through to [`crate::vga13h`]
//!   at 1:1.
//!
//! A 320×200 palette-index **shadow buffer** mirrors the logical canvas so `get_pixel` (used by
//! the mouse cursor's save-under) is a cheap in-memory read regardless of backend. `gfxui` uses
//! this module's API exactly as it used `vga13h`, so colors stay palette indices (`color::*`).

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use spin::Mutex;

use crate::framebuffer;
use crate::vga13h;
use qos_gfx::ScaleMap;

/// Logical canvas size — the coordinate space every `gfxui` call speaks.
pub const W: usize = 320;
pub const H: usize = 200;

/// Palette color indices, re-exported from `vga13h` so there is one source of truth for the
/// index → name mapping (`qos_gfx::PALETTE` renders those same indices to true color).
pub use crate::vga13h::color;

/// 8×8 font, LSB-first per row (bit 0 = leftmost pixel) — the convention `vga13h` uses, which
/// the desktop layout was tuned against. (The framebuffer console's own `draw_char` is MSB-first
/// and is unrelated to this graphics path.)
const FONT_8X8: [[u8; 8]; 128] = include!("font_8x8.rs");

/// Logical-canvas shadow (palette indices). Backs `get_pixel` cheaply for both backends.
static SHADOW: Mutex<[u8; W * H]> = Mutex::new([0u8; W * H]);

/// True while the framebuffer backend is selected (captured at [`enter`]).
static FB_MODE: AtomicBool = AtomicBool::new(false);
static SCALE: AtomicUsize = AtomicUsize::new(1);
static OFF_X: AtomicUsize = AtomicUsize::new(0);
static OFF_Y: AtomicUsize = AtomicUsize::new(0);

fn scale_map() -> ScaleMap {
    ScaleMap {
        scale: SCALE.load(Ordering::Relaxed),
        off_x: OFF_X.load(Ordering::Relaxed),
        off_y: OFF_Y.load(Ordering::Relaxed),
    }
}

/// Enter graphics mode. Picks the backend: framebuffer if the bootloader gave us one (computing
/// the centering scale for the current resolution), otherwise VGA Mode 13h.
pub fn enter() {
    if framebuffer::active() {
        let map = match framebuffer::info() {
            Some(info) => ScaleMap::compute(info.width, info.height, W, H),
            None => ScaleMap::identity(),
        };
        SCALE.store(map.scale, Ordering::Relaxed);
        OFF_X.store(map.off_x, Ordering::Relaxed);
        OFF_Y.store(map.off_y, Ordering::Relaxed);
        FB_MODE.store(true, Ordering::Relaxed);
        framebuffer::clear(qos_gfx::palette_rgb(color::BLACK));
        crate::serial_println!(
            "[DRAW] framebuffer desktop: {}x{} scale={}x off=({},{})",
            framebuffer::info().map(|i| i.width).unwrap_or(0),
            framebuffer::info().map(|i| i.height).unwrap_or(0),
            map.scale,
            map.off_x,
            map.off_y
        );
    } else {
        FB_MODE.store(false, Ordering::Relaxed);
        vga13h::enter();
    }
}

/// Leave graphics mode, returning to the text console cleanly.
pub fn leave() {
    if FB_MODE.load(Ordering::Relaxed) {
        framebuffer::clear(qos_gfx::palette_rgb(color::BLACK));
        framebuffer::reset_cursor();
    } else {
        vga13h::leave();
    }
}

/// Backend/geometry summary for the Display app: `(framebuffer_active, phys_w, phys_h, scale)`.
pub fn backend_info() -> (bool, usize, usize, usize) {
    if FB_MODE.load(Ordering::Relaxed) {
        let (w, h) = framebuffer::info().map(|i| (i.width, i.height)).unwrap_or((0, 0));
        (true, w, h, SCALE.load(Ordering::Relaxed))
    } else {
        (false, W, H, 1)
    }
}

#[inline]
pub fn put_pixel(x: usize, y: usize, c: u8) {
    if x >= W || y >= H {
        return;
    }
    SHADOW.lock()[y * W + x] = c;
    if FB_MODE.load(Ordering::Relaxed) {
        let m = scale_map();
        let (rx, ry, rw, rh) = m.rect(x, y, 1, 1);
        framebuffer::fill_rect(rx, ry, rw, rh, qos_gfx::palette_rgb(c));
    } else {
        vga13h::put_pixel(x, y, c);
    }
}

#[inline]
pub fn get_pixel(x: usize, y: usize) -> u8 {
    if x < W && y < H {
        SHADOW.lock()[y * W + x]
    } else {
        0
    }
}

pub const fn width() -> usize {
    W
}
pub const fn height() -> usize {
    H
}

pub fn clear(c: u8) {
    for b in SHADOW.lock().iter_mut() {
        *b = c;
    }
    if FB_MODE.load(Ordering::Relaxed) {
        // Clear the whole physical surface (borders included) to the background color.
        framebuffer::clear(qos_gfx::palette_rgb(c));
    } else {
        vga13h::clear(c);
    }
}

pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, c: u8) {
    // Update the shadow (clipped to the logical canvas) in one lock.
    {
        let mut s = SHADOW.lock();
        let y1 = core::cmp::min(y + h, H);
        let x1 = core::cmp::min(x + w, W);
        let mut yy = y;
        while yy < y1 {
            let mut xx = x;
            while xx < x1 {
                s[yy * W + xx] = c;
                xx += 1;
            }
            yy += 1;
        }
    }
    if FB_MODE.load(Ordering::Relaxed) {
        let m = scale_map();
        let (rx, ry, rw, rh) = m.rect(x, y, w, h);
        framebuffer::fill_rect(rx, ry, rw, rh, qos_gfx::palette_rgb(c));
    } else {
        vga13h::fill_rect(x, y, w, h, c);
    }
}

pub fn rect(x: usize, y: usize, w: usize, h: usize, c: u8) {
    if w == 0 || h == 0 {
        return;
    }
    // Four edges via fill_rect so scaling/clipping/shadow all stay consistent.
    fill_rect(x, y, w, 1, c); // top
    fill_rect(x, y + h - 1, w, 1, c); // bottom
    fill_rect(x, y, 1, h, c); // left
    fill_rect(x + w - 1, y, 1, h, c); // right
}

pub fn draw_char(x: usize, y: usize, ch: u8, fg: u8, bg: u8) {
    let c = ch as usize;
    if c >= 128 {
        return;
    }
    for row in 0..8 {
        let bits = FONT_8X8[c][row];
        for col in 0..8 {
            let on = (bits & (1 << col)) != 0; // LSB-first, matching vga13h
            put_pixel(x + col, y + row, if on { fg } else { bg });
        }
    }
}

pub fn draw_string(x: usize, y: usize, s: &str, fg: u8, bg: u8) {
    let mut cx = x;
    for &b in s.as_bytes() {
        draw_char(cx, y, b, fg, bg);
        cx += 8;
    }
}
