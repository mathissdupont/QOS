//! Minimal TrueType font engine (WP-05 / E-71): parse the core tables, map characters to glyphs,
//! read glyph outlines (simple + composite), and rasterize them to antialiased coverage bitmaps
//! with a glyph cache. Enough to render crisp scalable Latin text; no hinting, no ligatures.
//!
//! Portable (`no_std` + `alloc`) and host-tested. Only basic float arithmetic is used (add/mul/div
//! + integer casts) so it works without `libm`.

use alloc::vec;
use alloc::vec::Vec;

use crate::color::Rgb;
use crate::surface::Surface;

/// The default UI typeface: Roboto Regular (Copyright 2011 Google Inc., Apache License 2.0),
/// embedded unmodified. Parse it with [`Font::parse`]. See `assets/LICENSE-Roboto.txt`.
pub const DEFAULT_FONT: &[u8] = include_bytes!("../assets/Roboto-Regular.ttf");

// ---- big-endian readers (bounds-checked: return 0 past the end, never panic) ----
fn u16be(d: &[u8], o: usize) -> u16 {
    if o + 2 <= d.len() {
        ((d[o] as u16) << 8) | d[o + 1] as u16
    } else {
        0
    }
}
fn i16be(d: &[u8], o: usize) -> i16 {
    u16be(d, o) as i16
}
fn u32be(d: &[u8], o: usize) -> u32 {
    if o + 4 <= d.len() {
        ((d[o] as u32) << 24) | ((d[o + 1] as u32) << 16) | ((d[o + 2] as u32) << 8) | d[o + 3] as u32
    } else {
        0
    }
}

/// A parsed TrueType font backed by borrowed data (e.g. an embedded `&'static [u8]`).
pub struct Font<'a> {
    data: &'a [u8],
    units_per_em: u16,
    num_glyphs: u16,
    num_h_metrics: u16,
    loc_long: bool,
    off_head: usize,
    off_hmtx: usize,
    off_loca: usize,
    off_glyf: usize,
    off_cmap4: usize,
}

/// One rasterized glyph: an antialiased coverage bitmap plus placement metrics, all in pixels.
pub struct GlyphBitmap {
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<u8>,
    /// Offset of the bitmap's top-left from the pen origin (baseline). `left` is +right, `top` is
    /// +up from the baseline (so a positive `top` means the bitmap starts above the baseline).
    pub left: i32,
    pub top: i32,
    /// Horizontal advance in pixels.
    pub advance: i32,
}

struct Outline {
    // Flattened contours as polylines of (x, y) points in font units (y up).
    contours: Vec<Vec<(f32, f32)>>,
}

impl<'a> Font<'a> {
    /// Parse the table directory and the tables we need. Returns `None` if the font is malformed or
    /// lacks a usable Unicode cmap / glyf outline data.
    pub fn parse(data: &'a [u8]) -> Option<Font<'a>> {
        let num_tables = u16be(data, 4) as usize;
        let mut tables = [(0usize, 0usize); 0]; // placeholder to satisfy borrow; use locals below
        let _ = tables;
        let (mut off_head, mut off_maxp, mut off_hhea, mut off_hmtx, mut off_loca, mut off_glyf, mut off_cmap) =
            (0, 0, 0, 0, 0, 0, 0);
        for i in 0..num_tables {
            let rec = 12 + i * 16;
            if rec + 16 > data.len() {
                break;
            }
            let tag = &data[rec..rec + 4];
            let off = u32be(data, rec + 8) as usize;
            match tag {
                b"head" => off_head = off,
                b"maxp" => off_maxp = off,
                b"hhea" => off_hhea = off,
                b"hmtx" => off_hmtx = off,
                b"loca" => off_loca = off,
                b"glyf" => off_glyf = off,
                b"cmap" => off_cmap = off,
                _ => {}
            }
        }
        if off_head == 0 || off_glyf == 0 || off_loca == 0 || off_cmap == 0 {
            return None;
        }
        let units_per_em = u16be(data, off_head + 18);
        let loc_long = i16be(data, off_head + 50) != 0;
        let num_glyphs = u16be(data, off_maxp + 4);
        let num_h_metrics = u16be(data, off_hhea + 34);
        let off_cmap4 = find_cmap4(data, off_cmap)?;
        if units_per_em == 0 {
            return None;
        }
        Some(Font {
            data,
            units_per_em,
            num_glyphs,
            num_h_metrics,
            loc_long,
            off_head,
            off_hmtx,
            off_loca,
            off_glyf,
            off_cmap4,
        })
    }

    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Map a Unicode BMP character to a glyph index via the format-4 cmap subtable (0 = missing).
    pub fn glyph_index(&self, ch: char) -> u16 {
        let c = ch as u32;
        if c > 0xFFFF {
            return 0;
        }
        let c = c as u16;
        let b = self.off_cmap4;
        let d = self.data;
        let seg_count = (u16be(d, b + 6) / 2) as usize;
        let end_base = b + 14;
        let start_base = end_base + seg_count * 2 + 2; // +2 reservedPad
        let delta_base = start_base + seg_count * 2;
        let range_base = delta_base + seg_count * 2;
        for i in 0..seg_count {
            let end = u16be(d, end_base + i * 2);
            if c <= end {
                let start = u16be(d, start_base + i * 2);
                if c < start {
                    return 0;
                }
                let id_delta = u16be(d, delta_base + i * 2);
                let id_range = u16be(d, range_base + i * 2);
                if id_range == 0 {
                    return c.wrapping_add(id_delta);
                }
                // glyph id from the idRangeOffset table (offset in bytes from that array slot).
                let addr = range_base + i * 2 + id_range as usize + (c - start) as usize * 2;
                let g = u16be(d, addr);
                if g == 0 {
                    return 0;
                }
                return g.wrapping_add(id_delta);
            }
        }
        0
    }

    /// Horizontal advance of a glyph in font units.
    fn advance_units(&self, glyph: u16) -> u16 {
        let n = self.num_h_metrics;
        let g = if glyph < n { glyph } else { n.saturating_sub(1) };
        u16be(self.data, self.off_hmtx + g as usize * 4)
    }

    fn loca(&self, glyph: u16) -> (usize, usize) {
        let g = glyph as usize;
        if self.loc_long {
            (u32be(self.data, self.off_loca + g * 4) as usize, u32be(self.data, self.off_loca + (g + 1) * 4) as usize)
        } else {
            (u16be(self.data, self.off_loca + g * 2) as usize * 2, u16be(self.data, self.off_loca + (g + 1) * 2) as usize * 2)
        }
    }

    /// Read + flatten a glyph outline into polyline contours (font units, y up). Handles simple and
    /// (XY-offset) composite glyphs; returns `None` for empty glyphs (e.g. space) or on overflow.
    fn outline(&self, glyph: u16, depth: u8) -> Option<Outline> {
        if glyph >= self.num_glyphs || depth > 5 {
            return None;
        }
        let (start, end) = self.loca(glyph);
        if end <= start {
            return None; // empty glyph
        }
        let g = self.off_glyf + start;
        let d = self.data;
        let n_contours = i16be(d, g);
        if n_contours < 0 {
            return self.composite(g, depth);
        }
        let n_contours = n_contours as usize;
        let mut p = g + 10;
        let mut end_pts = Vec::with_capacity(n_contours);
        for _ in 0..n_contours {
            end_pts.push(u16be(d, p) as usize);
            p += 2;
        }
        let n_points = end_pts.last().map(|e| e + 1).unwrap_or(0);
        let instr_len = u16be(d, p) as usize;
        p += 2 + instr_len;

        // Flags (with repeat).
        let mut flags = Vec::with_capacity(n_points);
        while flags.len() < n_points {
            let f = *d.get(p)?;
            p += 1;
            flags.push(f);
            if f & 0x08 != 0 {
                let mut r = *d.get(p)?;
                p += 1;
                while r > 0 && flags.len() < n_points {
                    flags.push(f);
                    r -= 1;
                }
            }
        }
        // X coordinates (delta-encoded).
        let mut xs = Vec::with_capacity(n_points);
        let mut x: i32 = 0;
        for &f in &flags {
            if f & 0x02 != 0 {
                let dx = *d.get(p)? as i32;
                p += 1;
                x += if f & 0x10 != 0 { dx } else { -dx };
            } else if f & 0x10 == 0 {
                x += i16be(d, p) as i32;
                p += 2;
            }
            xs.push(x);
        }
        // Y coordinates (delta-encoded).
        let mut ys = Vec::with_capacity(n_points);
        let mut y: i32 = 0;
        for &f in &flags {
            if f & 0x04 != 0 {
                let dy = *d.get(p)? as i32;
                p += 1;
                y += if f & 0x20 != 0 { dy } else { -dy };
            } else if f & 0x20 == 0 {
                y += i16be(d, p) as i32;
                p += 2;
            }
            ys.push(y);
        }

        // Build contours, converting quadratic beziers (off-curve points) to polylines.
        let mut contours = Vec::new();
        let mut s = 0;
        for &e in &end_pts {
            if e >= n_points {
                break;
            }
            let pts: Vec<(f32, f32, bool)> = (s..=e)
                .map(|i| (xs[i] as f32, ys[i] as f32, flags[i] & 0x01 != 0))
                .collect();
            contours.push(flatten_contour(&pts));
            s = e + 1;
        }
        Some(Outline { contours })
    }

    fn composite(&self, g: usize, depth: u8) -> Option<Outline> {
        let d = self.data;
        let mut p = g + 10;
        let mut contours = Vec::new();
        loop {
            let flags = u16be(d, p);
            let comp_glyph = u16be(d, p + 2);
            p += 4;
            let (dx, dy) = if flags & 0x0001 != 0 {
                // ARG_1_AND_2_ARE_WORDS
                let a = i16be(d, p) as f32;
                let b = i16be(d, p + 2) as f32;
                p += 4;
                (a, b)
            } else {
                let a = (d.get(p).copied().unwrap_or(0) as i8) as f32;
                let b = (d.get(p + 1).copied().unwrap_or(0) as i8) as f32;
                p += 2;
                (a, b)
            };
            // Skip any scale fields (we don't apply component scaling yet).
            if flags & 0x0008 != 0 {
                p += 2; // WE_HAVE_A_SCALE
            } else if flags & 0x0040 != 0 {
                p += 4; // X_AND_Y_SCALE
            } else if flags & 0x0080 != 0 {
                p += 8; // 2x2 transform
            }
            if flags & 0x0002 != 0 {
                // ARGS_ARE_XY_VALUES: offset the component's contours.
                if let Some(sub) = self.outline(comp_glyph, depth + 1) {
                    for mut c in sub.contours {
                        for pt in c.iter_mut() {
                            pt.0 += dx;
                            pt.1 += dy;
                        }
                        contours.push(c);
                    }
                }
            }
            if flags & 0x0020 == 0 {
                break; // no MORE_COMPONENTS
            }
        }
        if contours.is_empty() {
            None
        } else {
            Some(Outline { contours })
        }
    }

    /// Rasterize `glyph` at `px` pixels/em into an antialiased coverage bitmap (4× supersampled,
    /// nonzero winding). Returns `None` for empty glyphs (still advance the pen for those).
    pub fn rasterize(&self, glyph: u16, px: f32) -> Option<GlyphBitmap> {
        let scale = px / self.units_per_em as f32;
        let advance = (self.advance_units(glyph) as f32 * scale + 0.5) as i32;
        let outline = match self.outline(glyph, 0) {
            Some(o) => o,
            None => {
                return Some(GlyphBitmap { width: 0, height: 0, coverage: Vec::new(), left: 0, top: 0, advance });
            }
        };
        // Scale points to pixel space (y down for the bitmap).
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut cs: Vec<Vec<(f32, f32)>> = Vec::with_capacity(outline.contours.len());
        for c in &outline.contours {
            let mut pc = Vec::with_capacity(c.len());
            for &(x, y) in c {
                let px_ = x * scale;
                let py_ = -y * scale; // flip to y-down
                min_x = min_x.min(px_);
                min_y = min_y.min(py_);
                max_x = max_x.max(px_);
                max_y = max_y.max(py_);
                pc.push((px_, py_));
            }
            cs.push(pc);
        }
        if !(min_x < max_x && min_y < max_y) {
            return Some(GlyphBitmap { width: 0, height: 0, coverage: Vec::new(), left: 0, top: 0, advance });
        }
        let ox = floor_i(min_x);
        let oy = floor_i(min_y);
        let w = (ceil_i(max_x) - ox).max(1) as usize;
        let h = (ceil_i(max_y) - oy).max(1) as usize;
        // Translate contours so the bitmap origin is (0,0).
        for c in cs.iter_mut() {
            for pt in c.iter_mut() {
                pt.0 -= ox as f32;
                pt.1 -= oy as f32;
            }
        }
        let coverage = fill_coverage(&cs, w, h);
        Some(GlyphBitmap {
            width: w,
            height: h,
            coverage,
            left: ox,
            top: -oy, // baseline-relative: rows above baseline are positive
            advance,
        })
    }
}

/// Find a usable format-4 Unicode cmap subtable, returning its absolute offset.
fn find_cmap4(d: &[u8], cmap: usize) -> Option<usize> {
    let n = u16be(d, cmap + 2) as usize;
    let mut fallback = None;
    for i in 0..n {
        let rec = cmap + 4 + i * 8;
        let plat = u16be(d, rec);
        let enc = u16be(d, rec + 2);
        let off = cmap + u32be(d, rec + 4) as usize;
        if u16be(d, off) != 4 {
            continue; // only format 4 here
        }
        // Prefer Windows Unicode BMP (3,1); accept Unicode (0,*) as fallback.
        if plat == 3 && enc == 1 {
            return Some(off);
        }
        if plat == 0 {
            fallback = Some(off);
        }
    }
    fallback
}

/// Flatten one contour of (x, y, on_curve) control points into a closed polyline, subdividing
/// quadratic Béziers. Handles implied on-curve midpoints between consecutive off-curve points.
fn flatten_contour(pts: &[(f32, f32, bool)]) -> Vec<(f32, f32)> {
    let n = pts.len();
    if n == 0 {
        return Vec::new();
    }
    // Find a starting on-curve point (synthesize one if the contour is all off-curve).
    let start = match pts.iter().position(|p| p.2) {
        Some(i) => (pts[i].0, pts[i].1),
        None => ((pts[0].0 + pts[n - 1].0) * 0.5, (pts[0].1 + pts[n - 1].1) * 0.5),
    };
    let start_idx = pts.iter().position(|p| p.2).unwrap_or(0);
    let mut out = vec![start];
    let mut cur = start;
    let mut i = 0;
    while i < n {
        let idx = (start_idx + 1 + i) % n;
        let (px, py, on) = pts[idx];
        if on {
            out.push((px, py));
            cur = (px, py);
            i += 1;
        } else {
            // Off-curve control point; the end is the next on-curve point (or an implied midpoint).
            let next_idx = (start_idx + 2 + i) % n;
            let (nx, ny, non) = pts[next_idx];
            let end = if non { (nx, ny) } else { ((px + nx) * 0.5, (py + ny) * 0.5) };
            // Subdivide the quadratic curve cur -> (px,py control) -> end.
            const STEPS: usize = 8;
            for s in 1..=STEPS {
                let t = s as f32 / STEPS as f32;
                let mt = 1.0 - t;
                let bx = mt * mt * cur.0 + 2.0 * mt * t * px + t * t * end.0;
                let by = mt * mt * cur.1 + 2.0 * mt * t * py + t * t * end.1;
                out.push((bx, by));
            }
            cur = end;
            i += if non { 2 } else { 1 };
        }
    }
    out
}

fn floor_i(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 {
        i - 1
    } else {
        i
    }
}
fn ceil_i(v: f32) -> i32 {
    let i = v as i32;
    if v > i as f32 {
        i + 1
    } else {
        i
    }
}

/// Rasterize filled polygons (nonzero winding) into a `w×h` u8 coverage buffer, 4× supersampled in
/// both axes via a per-sub-scanline edge crossing fill.
fn fill_coverage(contours: &[Vec<(f32, f32)>], w: usize, h: usize) -> Vec<u8> {
    const SS: usize = 4;
    let mut counts = vec![0u16; w * h];
    let sub_h = h * SS;
    let mut xs: Vec<(f32, i32)> = Vec::new(); // (x crossing, winding direction)
    for sy in 0..sub_h {
        let y = (sy as f32 + 0.5) / SS as f32;
        xs.clear();
        for c in contours {
            let m = c.len();
            if m < 2 {
                continue;
            }
            for i in 0..m {
                let (x0, y0) = c[i];
                let (x1, y1) = c[(i + 1) % m];
                // Does edge (y0->y1) cross scanline y?
                if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
                    let t = (y - y0) / (y1 - y0);
                    let x = x0 + t * (x1 - x0);
                    let dir = if y1 > y0 { 1 } else { -1 };
                    xs.push((x, dir));
                }
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
        let oy = sy / SS;
        let mut winding = 0i32;
        for i in 0..xs.len() - 1 {
            winding += xs[i].1;
            if winding != 0 {
                // Fill sub-columns between this crossing and the next.
                let xa = xs[i].0.max(0.0);
                let xb = xs[i + 1].0.min(w as f32);
                let sa = (xa * SS as f32 + 0.5) as i32;
                let sb = (xb * SS as f32 + 0.5) as i32;
                for sx in sa..sb {
                    if sx < 0 {
                        continue;
                    }
                    let ox = sx as usize / SS;
                    if ox < w {
                        counts[oy * w + ox] += 1;
                    }
                }
            }
        }
    }
    let denom = (SS * SS) as u16;
    counts.iter().map(|&c| ((c.min(denom) as u32 * 255) / denom as u32) as u8).collect()
}

/// A glyph cache keyed by `(glyph_index, px)` so repeated text is rasterized once.
pub struct FontRenderer<'a> {
    font: Font<'a>,
    cache: Vec<((u16, u32), GlyphBitmap)>, // small linear cache; px stored as bits for Eq
}

impl<'a> FontRenderer<'a> {
    pub fn new(font: Font<'a>) -> Self {
        FontRenderer { font, cache: Vec::new() }
    }

    pub fn font(&self) -> &Font<'a> {
        &self.font
    }

    fn glyph(&mut self, glyph: u16, px: f32) -> Option<usize> {
        let key = (glyph, px.to_bits());
        if let Some(i) = self.cache.iter().position(|(k, _)| *k == key) {
            return Some(i);
        }
        let bmp = self.font.rasterize(glyph, px)?;
        self.cache.push((key, bmp));
        Some(self.cache.len() - 1)
    }

    /// Width in pixels that `text` would occupy at `px` (sum of advances).
    pub fn text_width(&mut self, text: &str, px: f32) -> i32 {
        let mut w = 0;
        for ch in text.chars() {
            let g = self.font.glyph_index(ch);
            if let Some(i) = self.glyph(g, px) {
                w += self.cache[i].1.advance;
            }
        }
        w
    }

    /// Ascent in pixels (top of text above the baseline) — approximated from em size.
    pub fn ascent(&self, px: f32) -> i32 {
        (px * 0.8) as i32
    }

    /// Draw `text` with its baseline at `(x, baseline_y)` in `color` on `surface`. Returns the pen
    /// x after the last glyph.
    pub fn draw_text(&mut self, surface: &mut Surface, x: i32, baseline_y: i32, text: &str, px: f32, color: Rgb) -> i32 {
        let mut pen = x;
        for ch in text.chars() {
            let g = self.font.glyph_index(ch);
            if let Some(i) = self.glyph(g, px) {
                let bmp = &self.cache[i].1;
                if bmp.width > 0 {
                    surface.blit_mask(&bmp.coverage, bmp.width, bmp.height, pen + bmp.left, baseline_y - bmp.top, color);
                }
                pen += bmp.advance;
            }
        }
        pen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ROBOTO: &[u8] = include_bytes!("../assets/Roboto-Regular.ttf");

    #[test]
    fn parses_roboto_tables() {
        let f = Font::parse(ROBOTO).expect("parse");
        assert!(f.units_per_em() >= 1000, "unitsPerEm={}", f.units_per_em());
        assert!(f.num_glyphs > 100);
    }

    #[test]
    fn maps_ascii_to_glyphs() {
        let f = Font::parse(ROBOTO).unwrap();
        assert_ne!(f.glyph_index('A'), 0);
        assert_ne!(f.glyph_index('a'), 0);
        assert_ne!(f.glyph_index('0'), 0);
        // distinct letters map to distinct glyphs
        assert_ne!(f.glyph_index('A'), f.glyph_index('B'));
    }

    #[test]
    fn rasterizes_letter_with_ink_and_advance() {
        let f = Font::parse(ROBOTO).unwrap();
        let g = f.glyph_index('H');
        let bmp = f.rasterize(g, 48.0).unwrap();
        assert!(bmp.width > 0 && bmp.height > 0, "H has a bitmap");
        assert!(bmp.advance > 0, "H advances");
        let ink: u32 = bmp.coverage.iter().map(|&c| c as u32).sum();
        assert!(ink > 0, "H has ink");
        // Some pixels should be fully/near covered (interior of the stems).
        assert!(bmp.coverage.iter().any(|&c| c > 200), "H has solid coverage somewhere");
    }

    #[test]
    fn space_has_advance_but_no_bitmap() {
        let f = Font::parse(ROBOTO).unwrap();
        let g = f.glyph_index(' ');
        let bmp = f.rasterize(g, 48.0).unwrap();
        assert_eq!(bmp.width, 0);
        assert!(bmp.advance > 0, "space still advances the pen");
    }

    #[test]
    fn text_width_grows_with_more_chars() {
        let f = Font::parse(ROBOTO).unwrap();
        let mut r = FontRenderer::new(f);
        let w1 = r.text_width("Hi", 32.0);
        let w2 = r.text_width("Hello", 32.0);
        assert!(w2 > w1 && w1 > 0);
    }

    #[test]
    fn draw_text_puts_ink_on_surface() {
        let f = Font::parse(ROBOTO).unwrap();
        let mut r = FontRenderer::new(f);
        let mut s = Surface::filled(200, 60, crate::color::rgb(0, 0, 0));
        r.draw_text(&mut s, 5, 45, "Ag", 40.0, crate::color::rgb(255, 255, 255));
        let lit = s.pixels.iter().filter(|&&p| p != 0).count();
        assert!(lit > 50, "expected rendered text pixels, got {lit}");
    }
}
