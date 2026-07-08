//! True-color helpers. Colors are packed `0x00RRGGBB` (same layout as `qos-gfx::Rgb` and what the
//! kernel framebuffer backend expects), so the compositor and the existing backends agree.

/// A 24-bit color packed as `0x00RRGGBB`.
pub type Rgb = u32;

/// Pack red/green/blue into an [`Rgb`].
#[inline]
pub const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Unpack an [`Rgb`] into `(r, g, b)`.
#[inline]
pub const fn channels(c: Rgb) -> (u8, u8, u8) {
    (((c >> 16) & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, (c & 0xFF) as u8)
}

/// Alpha-blend `fg` over `bg` with `alpha` in `0..=255` (0 = keep `bg`, 255 = full `fg`).
/// Rounded integer blend per channel — no floats, so it is identical on host and kernel.
#[inline]
pub fn blend(bg: Rgb, fg: Rgb, alpha: u8) -> Rgb {
    if alpha == 0 {
        return bg;
    }
    if alpha == 255 {
        return fg;
    }
    let a = alpha as u32;
    let ia = 255 - a;
    let (br, bg_, bb) = channels(bg);
    let (fr, fg_, fb) = channels(fg);
    // +127 for round-to-nearest.
    let r = (fr as u32 * a + br as u32 * ia + 127) / 255;
    let g = (fg_ as u32 * a + bg_ as u32 * ia + 127) / 255;
    let b = (fb as u32 * a + bb as u32 * ia + 127) / 255;
    rgb(r as u8, g as u8, b as u8)
}

/// Linear interpolation between `a` and `b`, `t` in `0..=255` (0 = `a`, 255 = `b`). Same as
/// blending `b` over `a` with alpha `t`.
#[inline]
pub fn lerp(a: Rgb, b: Rgb, t: u8) -> Rgb {
    blend(a, b, t)
}

/// Scale a color's brightness by `factor` in `0..=255` over 128 (128 = unchanged, 255 ≈ 2×,
/// 64 = half). Saturates at 255. Handy for hover/pressed states and gradients from one base color.
#[inline]
pub fn scale_brightness(c: Rgb, factor: u8) -> Rgb {
    let (r, g, b) = channels(c);
    let s = |v: u8| -> u8 {
        let x = (v as u32 * factor as u32) / 128;
        if x > 255 {
            255
        } else {
            x as u8
        }
    };
    rgb(s(r), s(g), s(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let c = rgb(0x12, 0x34, 0x56);
        assert_eq!(c, 0x00123456);
        assert_eq!(channels(c), (0x12, 0x34, 0x56));
    }

    #[test]
    fn blend_endpoints_and_midpoint() {
        let black = rgb(0, 0, 0);
        let white = rgb(255, 255, 255);
        assert_eq!(blend(black, white, 0), black);
        assert_eq!(blend(black, white, 255), white);
        // 50% grey (rounded)
        assert_eq!(blend(black, white, 128), rgb(128, 128, 128));
    }

    #[test]
    fn blend_is_per_channel() {
        let bg = rgb(200, 0, 100);
        let fg = rgb(0, 200, 100);
        assert_eq!(blend(bg, fg, 128), rgb(100, 100, 100));
    }

    #[test]
    fn scale_brightness_bounds() {
        assert_eq!(scale_brightness(rgb(100, 100, 100), 128), rgb(100, 100, 100));
        assert_eq!(scale_brightness(rgb(100, 100, 100), 64), rgb(50, 50, 50));
        assert_eq!(scale_brightness(rgb(200, 200, 200), 255), rgb(255, 255, 255)); // saturates
    }
}
