//! A true-color drawing surface (the compositor back buffer) and its primitives.
//!
//! Pixels are `0x00RRGGBB`. All primitives clip to the surface bounds, so callers may pass
//! off-surface coordinates freely. Antialiasing (rounded corners, shadows) uses integer square
//! roots only — no floating point — so results are bit-identical on the host and in the `no_std`
//! kernel.

use alloc::vec;
use alloc::vec::Vec;

use crate::color::{self, Rgb};
use crate::geometry::Rect;

/// Integer square root (floor) via Newton's method — used for antialiased edge coverage.
fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// A resizable true-color pixel buffer.
pub struct Surface {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgb>,
    /// Optional clip rectangle: all drawing is confined to it (intersected with the bounds). Lets
    /// callers recompose only a damaged sub-region (e.g. during a window drag) instead of the whole
    /// surface. `None` = draw to the full surface.
    clip_rect: Option<Rect>,
}

impl Surface {
    /// A black surface of the given size.
    pub fn new(width: usize, height: usize) -> Self {
        Surface { width, height, pixels: vec![0; width * height], clip_rect: None }
    }

    /// A surface filled with `color`.
    pub fn filled(width: usize, height: usize, color: Rgb) -> Self {
        Surface { width, height, pixels: vec![color; width * height], clip_rect: None }
    }

    /// Confine subsequent drawing to `r` (intersected with the surface); `None` clears the clip.
    pub fn set_clip(&mut self, r: Option<Rect>) {
        self.clip_rect = r;
    }

    #[inline]
    fn in_clip(&self, x: i32, y: i32) -> bool {
        match self.clip_rect {
            Some(c) => c.contains(x, y),
            None => true,
        }
    }

    #[inline]
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        self.pixels[y * self.width + x]
    }

    /// Write one pixel, ignoring out-of-bounds / out-of-clip coordinates.
    #[inline]
    pub fn put(&mut self, x: i32, y: i32, color: Rgb) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height && self.in_clip(x, y) {
            self.pixels[y as usize * self.width + x as usize] = color;
        }
    }

    /// Alpha-blend one pixel (`alpha` 0..=255), ignoring out-of-bounds / out-of-clip coordinates.
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, color: Rgb, alpha: u8) {
        if alpha == 0 || x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height || !self.in_clip(x, y) {
            return;
        }
        let idx = y as usize * self.width + x as usize;
        self.pixels[idx] = color::blend(self.pixels[idx], color, alpha);
    }

    /// Fill the whole surface with one color.
    pub fn clear(&mut self, color: Rgb) {
        for p in self.pixels.iter_mut() {
            *p = color;
        }
    }

    /// Clip a rectangle to the surface (and the active clip rect), returning integer pixel ranges
    /// `(x0, y0, x1, y1)`.
    fn clip(&self, r: &Rect) -> Option<(usize, usize, usize, usize)> {
        let mut c = r.intersect(&self.bounds())?;
        if let Some(cr) = self.clip_rect {
            c = c.intersect(&cr)?;
        }
        Some((c.x as usize, c.y as usize, c.right() as usize, c.bottom() as usize))
    }

    /// Fill a rectangle with a solid color (clipped).
    pub fn fill_rect(&mut self, r: Rect, color: Rgb) {
        if let Some((x0, y0, x1, y1)) = self.clip(&r) {
            for y in y0..y1 {
                let row = y * self.width;
                for x in x0..x1 {
                    self.pixels[row + x] = color;
                }
            }
        }
    }

    /// Alpha-blend a solid rectangle over the surface (clipped).
    pub fn blend_rect(&mut self, r: Rect, color: Rgb, alpha: u8) {
        if alpha == 0 {
            return;
        }
        if let Some((x0, y0, x1, y1)) = self.clip(&r) {
            for y in y0..y1 {
                let row = y * self.width;
                for x in x0..x1 {
                    self.pixels[row + x] = color::blend(self.pixels[row + x], color, alpha);
                }
            }
        }
    }

    /// Vertical linear gradient from `top` to `bottom` across the rectangle's height (clipped).
    pub fn gradient_v(&mut self, r: Rect, top: Rgb, bottom: Rgb) {
        if r.h <= 0 {
            return;
        }
        let h = r.h;
        if let Some((x0, y0, x1, y1)) = self.clip(&r) {
            for y in y0..y1 {
                // t in 0..=255 across the *logical* rect height (so clipping doesn't skew it).
                let rel = (y as i32 - r.y).clamp(0, h - 1);
                let t = if h > 1 { (rel * 255 / (h - 1)) as u8 } else { 0 };
                let c = color::lerp(top, bottom, t);
                let row = y * self.width;
                for x in x0..x1 {
                    self.pixels[row + x] = c;
                }
            }
        }
    }

    /// Coverage (0..=255) of a pixel at signed distance handled by the caller. Given the squared
    /// distance `d2` from a corner center and the corner `radius`, returns how much of the pixel is
    /// inside the rounded corner (a 1px antialiased band at the boundary).
    fn corner_coverage(d2: u64, radius: i32) -> u8 {
        let r_fp = (radius as u64) << 8; // radius in 8.8 fixed point
        let dist_fp = isqrt(d2 << 16); // sqrt(d2) in 8.8 fixed point
        // coverage = clamp(0,256, radius + 0.5 - dist) mapped to 0..=255
        let edge = r_fp + 128; // +0.5 px
        if dist_fp >= edge {
            0
        } else if edge - dist_fp >= 256 {
            255
        } else {
            (edge - dist_fp) as u8
        }
    }

    /// Fill an antialiased rounded rectangle. The straight interior is filled solid; only the four
    /// corner squares get per-pixel coverage, so cost is dominated by the solid fill.
    pub fn rounded_rect(&mut self, r: Rect, radius: i32, color: Rgb) {
        self.rounded_rect_blend(r, radius, color, 255);
    }

    /// Alpha-blended antialiased rounded rectangle (`alpha` 0..=255).
    pub fn rounded_rect_blend(&mut self, r: Rect, radius: i32, color: Rgb, alpha: u8) {
        if r.is_empty() || alpha == 0 {
            return;
        }
        let rad = radius.clamp(0, r.w.min(r.h) / 2);
        if rad == 0 {
            self.blend_rect(r, color, alpha);
            return;
        }
        // Middle band (full width, between the corner rows): solid.
        self.blend_rect(Rect::new(r.x, r.y + rad, r.w, r.h - 2 * rad), color, alpha);
        // Top and bottom bands: solid center, antialiased corners.
        for dy in 0..rad {
            // distance of this row from the corner center (centers are `rad` in from each edge).
            let cy = rad - 1 - dy; // 0 at the row nearest the center circle line
            self.rounded_row(r, rad, color, alpha, r.y + dy, cy);
            self.rounded_row(r, rad, color, alpha, r.bottom() - 1 - dy, cy);
        }
    }

    /// Paint one corner row `y` of a rounded rect: solid across the middle, AA in the two corners.
    fn rounded_row(&mut self, r: Rect, rad: i32, color: Rgb, alpha: u8, y: i32, cy: i32) {
        // Solid middle span (between the left and right corner circles).
        self.blend_rect(Rect::new(r.x + rad, y, r.w - 2 * rad, 1), color, alpha);
        for dx in 0..rad {
            let cx = rad - 1 - dx;
            let d2 = (cx as u64) * (cx as u64) + (cy as u64) * (cy as u64);
            let cov = Self::corner_coverage(d2, rad);
            if cov > 0 {
                let a = ((cov as u32 * alpha as u32 + 127) / 255) as u8;
                self.blend(r.x + dx, y, color, a); // left corner
                self.blend(r.right() - 1 - dx, y, color, a); // right corner
            }
        }
    }

    /// A soft drop shadow: a rounded rectangle whose alpha falls off over `blur` pixels outside
    /// `r`. Draw this before the element it sits under. `alpha` is the peak (innermost) opacity.
    pub fn drop_shadow(&mut self, r: Rect, radius: i32, blur: i32, color: Rgb, alpha: u8) {
        if blur <= 0 {
            self.rounded_rect_blend(r, radius, color, alpha);
            return;
        }
        let outer = r.inflate(blur);
        let rad = radius + blur;
        if let Some((x0, y0, x1, y1)) = self.clip(&outer) {
            let inner = Rect::new(r.x + radius, r.y + radius, r.w - 2 * radius, r.h - 2 * radius);
            for y in y0..y1 {
                let row = y * self.width;
                for x in x0..x1 {
                    let (px, py) = (x as i32, y as i32);
                    // Skip the interior: it is fully opaque shadow that the element drawn on top
                    // (a rounded rect covering `r ⊇ inner`) will overwrite anyway. This avoids the
                    // per-pixel isqrt over the large central area — the dominant shadow cost.
                    if inner.contains(px, py) {
                        continue;
                    }
                    // Distance from the shadow's rounded core (the inner straight rect inflated by
                    // radius has rounded corners of `radius`; outside that, fall off over `blur`).
                    let dx = (inner.x - px).max(px - (inner.right() - 1)).max(0);
                    let dy = (inner.y - py).max(py - (inner.bottom() - 1)).max(0);
                    let dist = isqrt((dx as u64) * (dx as u64) + (dy as u64) * (dy as u64)) as i32;
                    let edge = dist - rad; // <=0 inside the core, grows outward through the blur
                    let a = if edge <= 0 {
                        alpha
                    } else if edge >= blur {
                        0
                    } else {
                        // Quadratic-ish falloff for a softer look.
                        let t = 255 - (edge * 255 / blur);
                        ((t * t / 255) as u32 * alpha as u32 / 255) as u8
                    };
                    if a > 0 {
                        self.pixels[row + x] = color::blend(self.pixels[row + x], color, a);
                    }
                }
            }
        }
    }

    /// Copy another surface onto this one at `(dx, dy)`, opaquely (clipped).
    pub fn blit(&mut self, src: &Surface, dx: i32, dy: i32) {
        let dst_rect = Rect::new(dx, dy, src.width as i32, src.height as i32);
        if let Some((x0, y0, x1, y1)) = self.clip(&dst_rect) {
            for y in y0..y1 {
                let sy = y as i32 - dy;
                let drow = y * self.width;
                let srow = sy as usize * src.width;
                for x in x0..x1 {
                    let sx = (x as i32 - dx) as usize;
                    self.pixels[drow + x] = src.pixels[srow + sx];
                }
            }
        }
    }

    /// Tint and blend an 8-bit coverage mask into an arbitrary destination rectangle, scaling it
    /// (nearest-neighbor) to fit, with an extra `global_alpha` multiplier (0..=255) over the whole
    /// mask. Used for the animated boot splash (fade via `global_alpha`, grow via `dst` size) and,
    /// later, scaled glyph blits. Clipped.
    pub fn blit_mask_scaled(&mut self, mask: &[u8], mw: usize, mh: usize, dst: Rect, color: Rgb, global_alpha: u8) {
        if dst.w <= 0 || dst.h <= 0 || mw == 0 || mh == 0 || global_alpha == 0 {
            return;
        }
        let (dw, dh) = (dst.w as usize, dst.h as usize);
        if let Some((x0, y0, x1, y1)) = self.clip(&dst) {
            for y in y0..y1 {
                let sy = (((y as i32 - dst.y) as usize) * mh / dh).min(mh - 1);
                let mrow = sy * mw;
                let drow = y * self.width;
                for x in x0..x1 {
                    let sx = (((x as i32 - dst.x) as usize) * mw / dw).min(mw - 1);
                    let cov = mask[mrow + sx];
                    if cov > 0 {
                        let a = ((cov as u32 * global_alpha as u32 + 127) / 255) as u8;
                        self.pixels[drow + x] = color::blend(self.pixels[drow + x], color, a);
                    }
                }
            }
        }
    }

    /// Tint and blend an 8-bit coverage/alpha mask (`mask[y*mw + x]`, 0..=255) with `color` at
    /// `(dx, dy)` — used for glyphs and the logo splash mask (clipped).
    pub fn blit_mask(&mut self, mask: &[u8], mw: usize, mh: usize, dx: i32, dy: i32, color: Rgb) {
        let dst_rect = Rect::new(dx, dy, mw as i32, mh as i32);
        if let Some((x0, y0, x1, y1)) = self.clip(&dst_rect) {
            for y in y0..y1 {
                let sy = (y as i32 - dy) as usize;
                let drow = y * self.width;
                let mrow = sy * mw;
                for x in x0..x1 {
                    let sx = (x as i32 - dx) as usize;
                    let a = mask[mrow + sx];
                    if a > 0 {
                        self.pixels[drow + x] = color::blend(self.pixels[drow + x], color, a);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{channels, rgb};

    #[test]
    fn isqrt_matches_floor() {
        for n in [0u64, 1, 2, 3, 4, 15, 16, 17, 100, 255, 256, 65535, 1_000_000] {
            let s = isqrt(n);
            assert!(s * s <= n && (s + 1) * (s + 1) > n, "isqrt({n})={s}");
        }
    }

    #[test]
    fn fill_rect_clips() {
        let mut s = Surface::new(10, 10);
        s.fill_rect(Rect::new(-2, -2, 5, 5), rgb(255, 0, 0));
        assert_eq!(s.get(0, 0), rgb(255, 0, 0));
        assert_eq!(s.get(2, 2), rgb(255, 0, 0));
        assert_eq!(s.get(3, 3), 0); // outside the 3×3 visible part
        // fully off-surface: no panic, no change
        s.fill_rect(Rect::new(100, 100, 5, 5), rgb(0, 255, 0));
    }

    #[test]
    fn gradient_endpoints() {
        let mut s = Surface::new(4, 4);
        let top = rgb(0, 0, 0);
        let bottom = rgb(255, 255, 255);
        s.gradient_v(Rect::new(0, 0, 4, 4), top, bottom);
        assert_eq!(s.get(0, 0), top);
        assert_eq!(s.get(0, 3), bottom);
        // monotonic increase down the column
        assert!(s.get(0, 1) < s.get(0, 2) || channels(s.get(0, 1)).0 <= channels(s.get(0, 2)).0);
    }

    #[test]
    fn rounded_rect_corner_is_transparent_center_is_solid() {
        let mut s = Surface::new(40, 40);
        s.clear(rgb(0, 0, 0));
        let fg = rgb(255, 255, 255);
        s.rounded_rect(Rect::new(0, 0, 40, 40), 10, fg);
        // Center is solid.
        assert_eq!(s.get(20, 20), fg);
        // Extreme corner pixel is (mostly) outside the rounding → stays dark.
        assert!(channels(s.get(0, 0)).0 < 128, "corner should be < half covered");
    }

    #[test]
    fn blit_mask_tints_by_coverage() {
        let mut s = Surface::filled(4, 1, rgb(0, 0, 0));
        let mask = [0u8, 128, 255, 0];
        s.blit_mask(&mask, 4, 1, 0, 0, rgb(255, 255, 255));
        assert_eq!(s.get(0, 0), rgb(0, 0, 0)); // alpha 0
        assert_eq!(s.get(1, 0), rgb(128, 128, 128)); // alpha 128
        assert_eq!(s.get(2, 0), rgb(255, 255, 255)); // alpha 255
    }

    #[test]
    fn blit_mask_scaled_upscales_and_fades() {
        // 2x2 mask, fully opaque, scaled into a 4x4 dst with 50% global alpha over black.
        let mut s = Surface::filled(4, 4, rgb(0, 0, 0));
        let mask = [255u8; 4];
        s.blit_mask_scaled(&mask, 2, 2, Rect::new(0, 0, 4, 4), rgb(255, 255, 255), 128);
        // Every pixel is covered (mask all 255) at ~50% → mid grey.
        assert_eq!(s.get(0, 0), rgb(128, 128, 128));
        assert_eq!(s.get(3, 3), rgb(128, 128, 128));
    }

    #[test]
    fn blit_copies_region() {
        let mut dst = Surface::new(8, 8);
        let src = Surface::filled(4, 4, rgb(10, 20, 30));
        dst.blit(&src, 2, 2);
        assert_eq!(dst.get(2, 2), rgb(10, 20, 30));
        assert_eq!(dst.get(5, 5), rgb(10, 20, 30));
        assert_eq!(dst.get(6, 6), 0);
        assert_eq!(dst.get(1, 1), 0);
    }
}
