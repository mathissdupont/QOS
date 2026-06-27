//! VESA Framebuffer Graphics Support
//!
//! This module provides framebuffer access for graphical output.
//! Note: Bootloader 0.9.x doesn't expose framebuffer, so this is a placeholder for future upgrade.

use core::fmt;
use spin::Mutex;

static FRAMEBUFFER: Mutex<Option<FrameBufferWrapper>> = Mutex::new(None);

struct FrameBufferWrapper {
    buffer: &'static mut [u8],
    info: FrameBufferInfo,
}

#[derive(Debug, Clone, Copy)]
pub struct FrameBufferInfo {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bytes_per_pixel: usize,
}

/// Initialize the framebuffer (placeholder - bootloader 0.9.x doesn't support this)
pub fn init() {
    // Bootloader 0.9.x doesn't expose framebuffer
    // TODO: Upgrade to bootloader 0.11+ for framebuffer support
    crate::serial_println!("[FB] Framebuffer not available in bootloader 0.9.x");
    crate::serial_println!("[FB] Using VGA text mode instead");
}

/// Get framebuffer info
pub fn info() -> Option<FrameBufferInfo> {
    FRAMEBUFFER.lock().as_ref().map(|fb| fb.info)
}

/// Clear the screen with a color
pub fn clear(color: u32) {
    let mut fb = FRAMEBUFFER.lock();
    if let Some(ref mut fb) = *fb {
        for y in 0..fb.info.height {
            for x in 0..fb.info.width {
                put_pixel_internal(fb, x, y, color);
            }
        }
    }
}

/// Draw a pixel at (x, y) with color
pub fn put_pixel(x: usize, y: usize, color: u32) {
    let mut fb = FRAMEBUFFER.lock();
    if let Some(ref mut fb) = *fb {
        put_pixel_internal(fb, x, y, color);
    }
}

fn put_pixel_internal(fb: &mut FrameBufferWrapper, x: usize, y: usize, color: u32) {
    if x >= fb.info.width || y >= fb.info.height {
        return;
    }

    let offset = y * fb.info.stride + x * fb.info.bytes_per_pixel;
    
    // Assume BGR format (most common in VESA)
    if offset + 2 < fb.buffer.len() {
        fb.buffer[offset + 0] = (color & 0xFF) as u8;         // B
        fb.buffer[offset + 1] = ((color >> 8) & 0xFF) as u8;  // G
        fb.buffer[offset + 2] = ((color >> 16) & 0xFF) as u8; // R
    }
}

/// Draw a filled rectangle
pub fn fill_rect(x: usize, y: usize, width: usize, height: usize, color: u32) {
    for dy in 0..height {
        for dx in 0..width {
            put_pixel(x + dx, y + dy, color);
        }
    }
}

/// Draw a line using Bresenham's algorithm
pub fn draw_line(x0: usize, y0: usize, x1: usize, y1: usize, color: u32) {
    let dx = if x1 > x0 { x1 - x0 } else { x0 - x1 };
    let dy = if y1 > y0 { y1 - y0 } else { y0 - y1 };
    
    let sx = if x0 < x1 { 1isize } else { -1isize };
    let sy = if y0 < y1 { 1isize } else { -1isize };
    
    let mut err = (if dx > dy { dx as isize } else { -(dy as isize) }) / 2;
    let mut x = x0 as isize;
    let mut y = y0 as isize;
    
    loop {
        put_pixel(x as usize, y as usize, color);
        
        if x == x1 as isize && y == y1 as isize {
            break;
        }
        
        let e2 = err;
        if e2 > -(dx as isize) {
            err -= dy as isize;
            x += sx;
        }
        if e2 < dy as isize {
            err += dx as isize;
            y += sy;
        }
    }
}

/// Common colors
pub mod colors {
    pub const BLACK: u32 = 0x000000;
    pub const WHITE: u32 = 0xFFFFFF;
    pub const RED: u32 = 0xFF0000;
    pub const GREEN: u32 = 0x00FF00;
    pub const BLUE: u32 = 0x0000FF;
    pub const YELLOW: u32 = 0xFFFF00;
    pub const CYAN: u32 = 0x00FFFF;
    pub const MAGENTA: u32 = 0xFF00FF;
    pub const GRAY: u32 = 0x808080;
    pub const DARK_GRAY: u32 = 0x404040;
    pub const LIGHT_GRAY: u32 = 0xC0C0C0;
}

/// 8x8 bitmap font for basic text rendering
const FONT_8X8: [[u8; 8]; 128] = include!("font_8x8.rs");

/// Draw a character at (x, y) with foreground and background colors
pub fn draw_char(x: usize, y: usize, ch: char, fg: u32, bg: u32) {
    let ch = ch as usize;
    if ch >= 128 {
        return;
    }
    
    for row in 0..8 {
        let bitmap = FONT_8X8[ch][row];
        for col in 0..8 {
            let pixel = if (bitmap & (1 << (7 - col))) != 0 {
                fg
            } else {
                bg
            };
            put_pixel(x + col, y + row, pixel);
        }
    }
}

/// Draw a string at (x, y)
pub fn draw_string(x: usize, y: usize, s: &str, fg: u32, bg: u32) {
    let mut offset_x = 0;
    for ch in s.chars() {
        draw_char(x + offset_x, y, ch, fg, bg);
        offset_x += 8;
    }
}

/// Simple console writer for framebuffer
pub struct FrameBufferWriter {
    x: usize,
    y: usize,
    fg: u32,
    bg: u32,
}

impl FrameBufferWriter {
    pub const fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            fg: colors::WHITE,
            bg: colors::BLACK,
        }
    }
    
    pub fn write_string(&mut self, s: &str) {
        let info = match info() {
            Some(i) => i,
            None => return,
        };
        
        for ch in s.chars() {
            if ch == '\n' {
                self.x = 0;
                self.y += 8;
            } else {
                draw_char(self.x, self.y, ch, self.fg, self.bg);
                self.x += 8;
                if self.x + 8 > info.width {
                    self.x = 0;
                    self.y += 8;
                }
            }
            
            // Scroll if needed
            if self.y + 8 > info.height {
                self.y = 0;
                clear(self.bg);
            }
        }
    }
    
    pub fn set_colors(&mut self, fg: u32, bg: u32) {
        self.fg = fg;
        self.bg = bg;
    }
}

impl fmt::Write for FrameBufferWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

static FB_WRITER: Mutex<FrameBufferWriter> = Mutex::new(FrameBufferWriter::new());

pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    FB_WRITER.lock().write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! fb_print {
    ($($arg:tt)*) => ($crate::framebuffer::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! fb_println {
    () => ($crate::fb_print!("\n"));
    ($($arg:tt)*) => ($crate::fb_print!("{}\n", format_args!($($arg)*)));
}
