//! VGA Mode 13h (320×200, 256 colors) — Phase 1 pixel graphics, with a clean return to text
//! mode (Phase 0.2). See ADR-0013 and docs/PLAN.md.
//!
//! Entering Mode 13h overwrites the text-mode font stored in VGA plane 2, so we **save the
//! font before entering and restore it on leave**, then re-apply the Mode 03h (80×25 text)
//! register set. The text framebuffer at `0xb8000` is outside the Mode 13h window (`0xA0000`
//! + 64000 bytes), so the shell's text content survives untouched.

use core::sync::atomic::Ordering;
use spin::Mutex;
use x86_64::instructions::port::Port;

const WIDTH: usize = 320;
const HEIGHT: usize = 200;
const FB: *mut u8 = 0xA0000 as *mut u8;

/// 8×8 bitmap font (shared with the framebuffer module). Used for graphics-mode text.
const FONT_8X8: [[u8; 8]; 128] = include!("font_8x8.rs");

/// Saved copy of the real text-mode font (256 glyphs × 16 rows) captured before mode switch.
static SAVED_FONT: Mutex<[u8; 256 * 16]> = Mutex::new([0u8; 256 * 16]);

pub mod color {
    pub const BLACK: u8 = 0;
    pub const BLUE: u8 = 1;
    pub const GREEN: u8 = 2;
    pub const TEAL: u8 = 3;
    pub const RED: u8 = 4;
    pub const MAGENTA: u8 = 5;
    pub const ORANGE: u8 = 6;
    pub const LTGRAY: u8 = 7;
    pub const DKGRAY: u8 = 8;
    pub const LTBLUE: u8 = 9;
    pub const LTGREEN: u8 = 10;
    pub const LTCYAN: u8 = 11;
    pub const LTRED: u8 = 12;
    pub const PINK: u8 = 13;
    pub const YELLOW: u8 = 14;
    pub const WHITE: u8 = 15;
}

// ── Register sets ────────────────────────────────────────────────────────────────────────

const MISC_13H: u8 = 0x63;
const SEQ_13H: [u8; 5] = [0x03, 0x01, 0x0F, 0x00, 0x0E];
const CRTC_13H: [u8; 25] = [
    0x5F, 0x4F, 0x50, 0x82, 0x54, 0x80, 0xBF, 0x1F, 0x00, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x9C, 0x0E, 0x8F, 0x28, 0x40, 0x96, 0xB9, 0xA3, 0xFF,
];
const GC_13H: [u8; 9] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x05, 0x0F, 0xFF];
const AC_13H: [u8; 21] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
    0x0F, 0x41, 0x00, 0x0F, 0x00, 0x00,
];

// Standard 80×25 text mode (Mode 03h), used to return cleanly.
const MISC_03H: u8 = 0x67;
const SEQ_03H: [u8; 5] = [0x03, 0x00, 0x03, 0x00, 0x02];
const CRTC_03H: [u8; 25] = [
    0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F, 0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00,
    0x50, 0x9C, 0x0E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3, 0xFF,
];
const GC_03H: [u8; 9] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF];
const AC_03H: [u8; 21] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E,
    0x3F, 0x0C, 0x00, 0x0F, 0x08, 0x00,
];

unsafe fn out16(port: u16, value: u16) {
    Port::<u16>::new(port).write(value);
}

unsafe fn set_mode(misc: u8, seq: &[u8; 5], crtc: &[u8; 25], gc: &[u8; 9], ac: &[u8; 21]) {
    let mut misc_p = Port::<u8>::new(0x3C2);
    let mut idx = Port::<u8>::new(0x3D4);
    let mut dat = Port::<u8>::new(0x3D5);
    let mut gci = Port::<u8>::new(0x3CE);
    let mut gcd = Port::<u8>::new(0x3CF);
    let mut si = Port::<u8>::new(0x3C4);
    let mut sd = Port::<u8>::new(0x3C5);
    let mut ac_port = Port::<u8>::new(0x3C0);
    let mut ac_reset = Port::<u8>::new(0x3DA);

    misc_p.write(misc);
    for (i, &v) in seq.iter().enumerate() {
        si.write(i as u8);
        sd.write(v);
    }
    // CRTC tables already carry the unlock bits (idx 3 |= 0x80, idx 0x11 &= ~0x80).
    for (i, &v) in crtc.iter().enumerate() {
        idx.write(i as u8);
        dat.write(v);
    }
    for (i, &v) in gc.iter().enumerate() {
        gci.write(i as u8);
        gcd.write(v);
    }
    for (i, &v) in ac.iter().enumerate() {
        let _ = ac_reset.read();
        ac_port.write(i as u8);
        ac_port.write(v);
    }
    let _ = ac_reset.read();
    ac_port.write(0x20); // lock palette, enable video
}

unsafe fn set_palette() {
    const PAL: [(u8, u8, u8); 16] = [
        (0, 0, 0), (0, 0, 42), (0, 42, 0), (0, 32, 32), (42, 0, 0), (42, 0, 42), (42, 21, 0),
        (42, 42, 42), (21, 21, 21), (21, 21, 63), (21, 63, 21), (21, 63, 63), (63, 21, 21),
        (63, 21, 63), (63, 63, 21), (63, 63, 63),
    ];
    let mut idx = Port::<u8>::new(0x3C8);
    let mut dat = Port::<u8>::new(0x3C9);
    idx.write(0);
    for (r, g, b) in PAL.iter() {
        dat.write(*r);
        dat.write(*g);
        dat.write(*b);
    }
}

/// Configure SEQ/GC to access character-generator RAM (plane 2) linearly at 0xA0000.
unsafe fn begin_font_access() {
    out16(0x3C4, 0x0100); // synchronous reset
    out16(0x3C4, 0x0402); // map mask = plane 2
    out16(0x3C4, 0x0704); // sequential memory mode
    out16(0x3C4, 0x0300); // end reset
    out16(0x3CE, 0x0204); // read map select = plane 2
    out16(0x3CE, 0x0005); // graphics mode = 0 (no odd/even)
    out16(0x3CE, 0x0406); // misc: map at 0xA0000 (64K)
}

/// Restore SEQ/GC to normal text addressing (planes 0/1, odd/even, 0xB8000).
unsafe fn end_font_access() {
    out16(0x3C4, 0x0100); // reset
    out16(0x3C4, 0x0302); // map mask = planes 0 & 1
    out16(0x3C4, 0x0204); // even/odd memory mode
    out16(0x3C4, 0x0300); // end reset
    out16(0x3CE, 0x0004); // read map = 0
    out16(0x3CE, 0x1005); // odd/even mode
    out16(0x3CE, 0x0E06); // misc: text at 0xB8000
}

fn save_font() {
    let mut buf = SAVED_FONT.lock();
    unsafe {
        begin_font_access();
        for c in 0..256 {
            for r in 0..16 {
                buf[c * 16 + r] = core::ptr::read_volatile(FB.add(c * 32 + r));
            }
        }
        end_font_access();
    }
}

fn restore_font() {
    let buf = SAVED_FONT.lock();
    unsafe {
        begin_font_access();
        for c in 0..256 {
            for r in 0..16 {
                core::ptr::write_volatile(FB.add(c * 32 + r), buf[c * 16 + r]);
            }
        }
        end_font_access();
    }
}

/// Enter Mode 13h (saving the text font first) and set up the palette.
pub fn enter() {
    save_font();
    unsafe {
        set_mode(MISC_13H, &SEQ_13H, &CRTC_13H, &GC_13H, &AC_13H);
        set_palette();
    }
    clear(color::BLACK);
}

/// Return to 80×25 text mode and restore the saved font (clean return to the shell).
pub fn leave() {
    unsafe {
        set_mode(MISC_03H, &SEQ_03H, &CRTC_03H, &GC_03H, &AC_03H);
    }
    restore_font();
    crate::serial_println!("[VGA13H] restored text mode");
}

// ── Drawing primitives ──────────────────────────────────────────────────────────────────

#[inline]
pub fn put_pixel(x: usize, y: usize, c: u8) {
    if x < WIDTH && y < HEIGHT {
        unsafe { core::ptr::write_volatile(FB.add(y * WIDTH + x), c) }
    }
}

#[inline]
pub fn get_pixel(x: usize, y: usize) -> u8 {
    if x < WIDTH && y < HEIGHT {
        unsafe { core::ptr::read_volatile(FB.add(y * WIDTH + x)) }
    } else {
        0
    }
}

/// Screen dimensions, for consumers building UIs on top.
pub const fn width() -> usize {
    WIDTH
}
pub const fn height() -> usize {
    HEIGHT
}

pub fn clear(c: u8) {
    unsafe {
        for i in 0..(WIDTH * HEIGHT) {
            core::ptr::write_volatile(FB.add(i), c);
        }
    }
}

pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, c: u8) {
    for dy in 0..h {
        for dx in 0..w {
            put_pixel(x + dx, y + dy, c);
        }
    }
}

pub fn rect(x: usize, y: usize, w: usize, h: usize, c: u8) {
    if w == 0 || h == 0 {
        return;
    }
    for dx in 0..w {
        put_pixel(x + dx, y, c);
        put_pixel(x + dx, y + h - 1, c);
    }
    for dy in 0..h {
        put_pixel(x, y + dy, c);
        put_pixel(x + w - 1, y + dy, c);
    }
}

pub fn draw_char(x: usize, y: usize, ch: u8, fg: u8, bg: u8) {
    let c = ch as usize;
    if c >= 128 {
        return;
    }
    for row in 0..8 {
        let bits = FONT_8X8[c][row];
        for col in 0..8 {
            // font_8x8 stores bit 0 = leftmost pixel (LSB-first).
            let on = (bits & (1 << col)) != 0;
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

fn wait_ticks(n: u64) {
    let start = crate::interrupts::TICKS.load(Ordering::Relaxed);
    while crate::interrupts::TICKS.load(Ordering::Relaxed).wrapping_sub(start) < n {
        x86_64::instructions::hlt();
    }
}

/// Drain the input queue; return true if ESC (scancode 0x01) was pressed.
fn esc_pressed() -> bool {
    let mut esc = false;
    while let Some(ev) = crate::input::poll() {
        if let crate::input::InputEvent::Key { scancode: 0x01, pressed: true } = ev {
            esc = true;
        }
    }
    esc
}

/// Enter Mode 13h, draw a desktop-like scene, animate until ESC, then return to text mode.
pub fn demo() {
    use color::*;

    crate::serial_println!("[VGA13H] entering Mode 13h graphics demo");
    enter();

    clear(TEAL);
    fill_rect(0, 0, WIDTH, 12, BLUE);
    draw_string(2, 2, "QOS  VGA Mode 13h  320x200x256", WHITE, BLUE);

    let (wx, wy, ww, wh) = (40, 40, 240, 120);
    fill_rect(wx, wy, ww, wh, LTGRAY);
    rect(wx, wy, ww, wh, DKGRAY);
    fill_rect(wx + 2, wy + 2, ww - 4, 12, RED);
    draw_string(wx + 6, wy + 4, "Quantum Window", WHITE, RED);
    draw_string(wx + 8, wy + 24, "Pixels are alive!", BLACK, LTGRAY);
    draw_string(wx + 8, wy + 36, "Bell state: |00> + |11>", BLACK, LTGRAY);

    let base = wy + wh - 16;
    fill_rect(wx + 30, base - 40, 24, 40, GREEN);
    draw_string(wx + 30, base + 2, "00", BLACK, LTGRAY);
    fill_rect(wx + 90, base - 40, 24, 40, LTBLUE);
    draw_string(wx + 90, base + 2, "11", BLACK, LTGRAY);

    for i in 0..16usize {
        fill_rect(i * 20, HEIGHT - 10, 20, 10, i as u8);
    }
    draw_string(40, HEIGHT - 24, "press ESC to return to shell", WHITE, TEAL);

    // Animate a box in the desktop strip above the window until ESC.
    let (top, bottom) = (14usize, 36usize);
    let (bw, bh) = (10usize, 8usize);
    let mut bx: i32 = 50;
    let mut by: i32 = top as i32;
    let mut dx: i32 = 3;
    let mut dy: i32 = 2;
    loop {
        if esc_pressed() {
            break;
        }
        fill_rect(bx as usize, by as usize, bw, bh, TEAL);
        bx += dx;
        by += dy;
        if bx < 0 {
            bx = 0;
            dx = -dx;
        }
        if bx as usize + bw >= WIDTH {
            bx = (WIDTH - bw) as i32;
            dx = -dx;
        }
        if (by as usize) < top {
            by = top as i32;
            dy = -dy;
        }
        if by as usize + bh >= bottom {
            by = (bottom - bh) as i32;
            dy = -dy;
        }
        fill_rect(bx as usize, by as usize, bw, bh, YELLOW);
        wait_ticks(2);
    }

    leave();
}
