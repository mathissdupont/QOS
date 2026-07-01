//! # qos-gfx
//!
//! Portable, resolution-agnostic drawing math shared by the QOS graphics backends. The kernel
//! keeps a fixed **logical** 320×200 desktop coordinate space (the same one the VGA Mode 13h UI
//! was authored against, see ADR-0013). When the system boots on a linear framebuffer instead
//! (bootloader 0.11 / UEFI, ADR-0014), that logical canvas is drawn onto the real framebuffer
//! **integer-scaled and centered** so the existing UI code runs unchanged at any resolution.
//!
//! This crate holds the pure pieces of that mapping — the 16-color palette → true-color
//! conversion and the [`ScaleMap`] geometry — so they can be unit-tested on the host, away from
//! the hardware. It is `no_std` when compiled into the kernel (`#![cfg_attr(not(test), no_std)]`)
//! and needs no allocator.

#![cfg_attr(not(test), no_std)]

/// A 24-bit true color packed as `0x00RRGGBB`. This layout matches what the kernel's framebuffer
/// backend expects (it extracts `B = c & 0xFF`, `G = (c >> 8) & 0xFF`, `R = (c >> 16) & 0xFF`).
pub type Rgb = u32;

/// The 16-entry QOS/VGA palette, as true-color `0xRRGGBB`. Index `i` is the RGB rendering of the
/// Mode 13h palette color `i` (the `vga13h::color` constants), derived from the 6-bit VGA DAC
/// values scaled to 8-bit. Index order **must** match `vga13h::color`:
///
/// `0 BLACK, 1 BLUE, 2 GREEN, 3 TEAL, 4 RED, 5 MAGENTA, 6 ORANGE, 7 LTGRAY, 8 DKGRAY, 9 LTBLUE,
///  10 LTGREEN, 11 LTCYAN, 12 LTRED, 13 PINK, 14 YELLOW, 15 WHITE`.
pub const PALETTE: [Rgb; 16] = [
    0x000000, // 0  BLACK
    0x0000AA, // 1  BLUE
    0x00AA00, // 2  GREEN
    0x008181, // 3  TEAL
    0xAA0000, // 4  RED
    0xAA00AA, // 5  MAGENTA
    0xAA5500, // 6  ORANGE
    0xAAAAAA, // 7  LTGRAY
    0x555555, // 8  DKGRAY
    0x5555FF, // 9  LTBLUE
    0x55FF55, // 10 LTGREEN
    0x55FFFF, // 11 LTCYAN
    0xFF5555, // 12 LTRED
    0xFF55FF, // 13 PINK
    0xFFFF55, // 14 YELLOW
    0xFFFFFF, // 15 WHITE
];

/// Convert a palette index to its true-color value. Out-of-range indices fall back to black,
/// which is a safe, visible default (never panics — this runs in the kernel).
#[inline]
pub fn palette_rgb(index: u8) -> Rgb {
    PALETTE.get(index as usize).copied().unwrap_or(0x000000)
}

/// Maps a fixed **logical** canvas onto a larger physical target by the largest integer scale
/// that fits, centering the result. Used to draw the 320×200 desktop onto an arbitrary
/// framebuffer resolution without changing any UI layout code.
///
/// The scale is clamped to at least 1: if the physical target is smaller than the logical canvas
/// the content is drawn 1:1 from the top-left and the backend clips the overflow. This keeps the
/// mapping total and panic-free for any input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScaleMap {
    /// Integer zoom factor (physical pixels per logical pixel), always ≥ 1.
    pub scale: usize,
    /// Left margin in physical pixels that centers the scaled canvas.
    pub off_x: usize,
    /// Top margin in physical pixels that centers the scaled canvas.
    pub off_y: usize,
}

impl ScaleMap {
    /// Compute the centering integer-scale mapping of a `logical_w × logical_h` canvas onto a
    /// `target_w × target_h` physical surface.
    pub fn compute(target_w: usize, target_h: usize, logical_w: usize, logical_h: usize) -> Self {
        debug_assert!(logical_w > 0 && logical_h > 0);
        let sx = target_w / logical_w;
        let sy = target_h / logical_h;
        let scale = core_min(sx, sy).max(1);
        let used_w = logical_w * scale;
        let used_h = logical_h * scale;
        let off_x = target_w.saturating_sub(used_w) / 2;
        let off_y = target_h.saturating_sub(used_h) / 2;
        Self { scale, off_x, off_y }
    }

    /// The 1:1 identity mapping (physical == logical), for the native VGA Mode 13h path.
    pub const fn identity() -> Self {
        Self { scale: 1, off_x: 0, off_y: 0 }
    }

    /// Map a logical point to its top-left physical pixel.
    #[inline]
    pub fn point(&self, x: usize, y: usize) -> (usize, usize) {
        (self.off_x + x * self.scale, self.off_y + y * self.scale)
    }

    /// Map a logical rectangle to a physical rectangle `(x, y, w, h)`.
    #[inline]
    pub fn rect(&self, x: usize, y: usize, w: usize, h: usize) -> (usize, usize, usize, usize) {
        (
            self.off_x + x * self.scale,
            self.off_y + y * self.scale,
            w * self.scale,
            h * self.scale,
        )
    }
}

/// `core::cmp::min` as a plain fn so the const/`no_std` path stays trivially readable.
#[inline]
fn core_min(a: usize, b: usize) -> usize {
    if a < b {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_endpoints_are_black_and_white() {
        assert_eq!(palette_rgb(0), 0x000000);
        assert_eq!(palette_rgb(15), 0xFFFFFF);
    }

    #[test]
    fn palette_out_of_range_is_black_not_panic() {
        assert_eq!(palette_rgb(16), 0x000000);
        assert_eq!(palette_rgb(255), 0x000000);
    }

    #[test]
    fn palette_channel_layout_is_rrggbb() {
        // BLUE (index 1) must have its blue channel set and red/green clear.
        let blue = palette_rgb(1);
        assert_eq!(blue & 0xFF, 0xAA, "blue channel");
        assert_eq!((blue >> 8) & 0xFF, 0x00, "green channel");
        assert_eq!((blue >> 16) & 0xFF, 0x00, "red channel");
    }

    #[test]
    fn scale_640x480_is_2x_centered_vertically() {
        // 320*2 = 640 fits exactly in width; 200*2 = 400 leaves 80px -> 40px top/bottom.
        let m = ScaleMap::compute(640, 480, 320, 200);
        assert_eq!(m, ScaleMap { scale: 2, off_x: 0, off_y: 40 });
    }

    #[test]
    fn scale_800x600_limited_by_width() {
        // min(800/320=2, 600/200=3) = 2. used = 640x400 -> margins (80, 100).
        let m = ScaleMap::compute(800, 600, 320, 200);
        assert_eq!(m, ScaleMap { scale: 2, off_x: 80, off_y: 100 });
    }

    #[test]
    fn scale_1024x768_is_3x() {
        // min(1024/320=3, 768/200=3) = 3. used = 960x600 -> margins (32, 84).
        let m = ScaleMap::compute(1024, 768, 320, 200);
        assert_eq!(m, ScaleMap { scale: 3, off_x: 32, off_y: 84 });
    }

    #[test]
    fn scale_exact_logical_is_identity() {
        let m = ScaleMap::compute(320, 200, 320, 200);
        assert_eq!(m, ScaleMap { scale: 1, off_x: 0, off_y: 0 });
    }

    #[test]
    fn scale_smaller_than_logical_clamps_to_1x() {
        // Physical smaller than logical: clamp to 1x, no negative offsets, backend clips.
        let m = ScaleMap::compute(300, 150, 320, 200);
        assert_eq!(m, ScaleMap { scale: 1, off_x: 0, off_y: 0 });
    }

    #[test]
    fn point_and_rect_apply_scale_and_offset() {
        let m = ScaleMap { scale: 2, off_x: 80, off_y: 100 };
        assert_eq!(m.point(10, 5), (100, 110));
        assert_eq!(m.rect(10, 5, 4, 3), (100, 110, 8, 6));
    }

    #[test]
    fn identity_maps_1to1() {
        let m = ScaleMap::identity();
        assert_eq!(m.point(17, 23), (17, 23));
        assert_eq!(m.rect(1, 2, 3, 4), (1, 2, 3, 4));
    }
}
