use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use lazy_static::lazy_static;
use spin::Mutex;

const SCROLLBACK_LINES: usize = 256;

static SCROLL_OFFSET: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    Pink = 13,
    Yellow = 14,
    White = 15,
}

#[derive(Clone, Copy)]
#[repr(transparent)]
struct ColorCode(u8);

impl ColorCode {
    const fn new(fg: Color, bg: Color) -> Self {
        Self((bg as u8) << 4 | (fg as u8))
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
struct ScreenChar {
    ascii_character: u8,
    color_code: ColorCode,
}

const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

// Number of top rows reserved for a UI overlay.
// These rows are not affected by scrolling in `Writer::new_line`.
static RESERVED_TOP_ROWS: AtomicU64 = AtomicU64::new(0);

// Number of bottom rows reserved (e.g. fixed input line).
// These rows are not affected by scrolling in `Writer::new_line`.
static RESERVED_BOTTOM_ROWS: AtomicU64 = AtomicU64::new(0);

#[repr(transparent)]
struct Buffer {
    chars: [[ScreenChar; BUFFER_WIDTH]; BUFFER_HEIGHT],
}

struct Scrollback {
    // Ring of full-width lines.
    lines: [[u8; BUFFER_WIDTH]; SCROLLBACK_LINES],
    next_seq: u64, // next line sequence to write
    col: usize,
}

impl Scrollback {
    const fn new() -> Self {
        Self {
            lines: [[b' '; BUFFER_WIDTH]; SCROLLBACK_LINES],
            next_seq: 0,
            col: 0,
        }
    }

    fn current_index(&self) -> usize {
        (self.next_seq as usize) % SCROLLBACK_LINES
    }

    fn clear_line_at(&mut self, idx: usize) {
        self.lines[idx] = [b' '; BUFFER_WIDTH];
    }

    fn newline(&mut self) {
        self.next_seq = self.next_seq.wrapping_add(1);
        self.col = 0;
        let idx = self.current_index();
        self.clear_line_at(idx);
    }

    fn push_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            0x08 => {
                if self.col == 0 {
                    return;
                }
                self.col -= 1;
                let idx = self.current_index();
                self.lines[idx][self.col] = b' ';
            }
            b => {
                let idx = self.current_index();
                let ch = if (0x20..=0x7e).contains(&b) { b } else { 0xfe };
                self.lines[idx][self.col] = ch;
                self.col += 1;
                if self.col >= BUFFER_WIDTH {
                    self.newline();
                }
            }
        }
    }

    fn write_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.push_byte(b);
        }
    }

    fn oldest_seq(&self) -> u64 {
        self.next_seq.saturating_sub(SCROLLBACK_LINES as u64)
    }

    fn line_for_seq(&self, seq: u64) -> Option<&[u8; BUFFER_WIDTH]> {
        if seq < self.oldest_seq() {
            return None;
        }
        if seq > self.next_seq {
            return None;
        }
        let idx = (seq as usize) % SCROLLBACK_LINES;
        Some(&self.lines[idx])
    }
}

pub struct Writer {
    column_position: usize,
    color_code: ColorCode,
    buffer: &'static mut Buffer,
}

impl Writer {
    fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            0x08 => {
                // Backspace: move cursor left and erase the character.
                if self.column_position == 0 {
                    return;
                }
                self.column_position -= 1;
                let row = output_bottom_row();
                let col = self.column_position;
                unsafe {
                    core::ptr::write_volatile(
                        &mut self.buffer.chars[row][col] as *mut ScreenChar,
                        ScreenChar {
                            ascii_character: b' ',
                            color_code: self.color_code,
                        },
                    );
                }
            }
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }

                let row = output_bottom_row();
                let col = self.column_position;

                unsafe {
                    core::ptr::write_volatile(
                        &mut self.buffer.chars[row][col] as *mut ScreenChar,
                        ScreenChar {
                    ascii_character: byte,
                    color_code: self.color_code,
                        },
                    );
                }

                self.column_position += 1;
            }
        }
    }

    fn new_line(&mut self) {
        let reserved = RESERVED_TOP_ROWS.load(Ordering::Relaxed) as usize;
        let bottom = output_bottom_row();
        let start = core::cmp::min(reserved + 1, bottom + 1);
        for row in start..=bottom {
            for col in 0..BUFFER_WIDTH {
                let c = unsafe {
                    core::ptr::read_volatile(&self.buffer.chars[row][col] as *const ScreenChar)
                };
                unsafe {
                    core::ptr::write_volatile(
                        &mut self.buffer.chars[row - 1][col] as *mut ScreenChar,
                        c,
                    );
                }
            }
        }
        self.clear_row(bottom);
        self.column_position = 0;
    }

    fn clear_row(&mut self, row: usize) {
        let blank = ScreenChar {
            ascii_character: b' ',
            color_code: self.color_code,
        };
        for col in 0..BUFFER_WIDTH {
            unsafe {
                core::ptr::write_volatile(
                    &mut self.buffer.chars[row][col] as *mut ScreenChar,
                    blank,
                );
            }
        }
    }

    fn write_string(&mut self, s: &str) {
        for byte in s.bytes() {
            match byte {
                0x20..=0x7e | b'\n' | 0x08 => self.write_byte(byte),
                _ => self.write_byte(0xfe),
            }
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

lazy_static! {
    static ref WRITER: Mutex<Writer> = Mutex::new(Writer {
        column_position: 0,
        color_code: ColorCode::new(Color::LightGreen, Color::Black),
        buffer: unsafe { &mut *(0xb8000 as *mut Buffer) },
    });

    static ref SCROLLBACK: Mutex<Scrollback> = Mutex::new(Scrollback::new());
}

pub fn clear_screen() {
    let mut w = WRITER.lock();
    for row in 0..BUFFER_HEIGHT {
        w.clear_row(row);
    }
    w.column_position = 0;

    // Reset scrollback as well.
    {
        let mut sb = SCROLLBACK.lock();
        *sb = Scrollback::new();
    }
    SCROLL_OFFSET.store(0, Ordering::Relaxed);
}

pub fn set_reserved_top_rows(rows: usize) {
    let rows = core::cmp::min(rows, BUFFER_HEIGHT);
    RESERVED_TOP_ROWS.store(rows as u64, Ordering::Relaxed);
}

pub fn reserved_top_rows() -> usize {
    RESERVED_TOP_ROWS.load(Ordering::Relaxed) as usize
}

pub fn set_reserved_bottom_rows(rows: usize) {
    let rows = core::cmp::min(rows, BUFFER_HEIGHT);
    RESERVED_BOTTOM_ROWS.store(rows as u64, Ordering::Relaxed);
}

pub fn reserved_bottom_rows() -> usize {
    RESERVED_BOTTOM_ROWS.load(Ordering::Relaxed) as usize
}

pub fn bottom_row() -> usize {
    BUFFER_HEIGHT - 1
}

fn output_bottom_row() -> usize {
    let rb = RESERVED_BOTTOM_ROWS.load(Ordering::Relaxed) as usize;
    BUFFER_HEIGHT.saturating_sub(1 + rb)
}

pub fn output_region_bounds() -> (usize, usize) {
    let top = RESERVED_TOP_ROWS.load(Ordering::Relaxed) as usize;
    let bottom = output_bottom_row();
    (core::cmp::min(top, BUFFER_HEIGHT - 1), bottom)
}

fn write_row_direct(row: usize, bytes: &[u8; BUFFER_WIDTH], fg: Color, bg: Color) {
    if row >= BUFFER_HEIGHT {
        return;
    }
    let code = ColorCode::new(fg, bg);
    let buffer = unsafe { &mut *(0xb8000 as *mut Buffer) };
    for col in 0..BUFFER_WIDTH {
        let b = bytes[col];
        let ch = if (0x20..=0x7e).contains(&b) { b } else { 0xfe };
        unsafe {
            core::ptr::write_volatile(
                &mut buffer.chars[row][col] as *mut ScreenChar,
                ScreenChar {
                    ascii_character: ch,
                    color_code: code,
                },
            );
        }
    }
}

fn render_viewport() {
    let (top, bottom) = output_region_bounds();
    if bottom < top {
        return;
    }
    let height = bottom - top + 1;

    let offset = SCROLL_OFFSET.load(Ordering::Relaxed);
    let sb = SCROLLBACK.lock();
    let newest = sb.next_seq;
    let oldest = sb.oldest_seq();

    // We want the bottom of the viewport to show `newest - offset`.
    let bottom_seq = newest.saturating_sub(offset);
    let top_seq = bottom_seq.saturating_sub((height - 1) as u64);

    for i in 0..height {
        let seq = top_seq.wrapping_add(i as u64);
        let row = top + i;
        let line = if seq < oldest {
            None
        } else {
            sb.line_for_seq(seq)
        };
        match line {
            Some(bytes) => write_row_direct(row, bytes, Color::LightGreen, Color::Black),
            None => write_row_direct(row, &[b' '; BUFFER_WIDTH], Color::LightGreen, Color::Black),
        }
    }
}

pub fn scroll_up(lines: usize) {
    if lines == 0 {
        return;
    }
    let mut off = SCROLL_OFFSET.load(Ordering::Relaxed);
    off = off.saturating_add(lines as u64);

    // Clamp to available scrollback.
    let sb = SCROLLBACK.lock();
    let max = sb.next_seq.saturating_sub(sb.oldest_seq());
    drop(sb);
    if off > max {
        off = max;
    }
    SCROLL_OFFSET.store(off, Ordering::Relaxed);
    render_viewport();
}

pub fn scroll_down(lines: usize) {
    if lines == 0 {
        return;
    }
    let off = SCROLL_OFFSET.load(Ordering::Relaxed);
    let off = off.saturating_sub(lines as u64);
    SCROLL_OFFSET.store(off, Ordering::Relaxed);
    render_viewport();
}

pub fn scroll_reset() {
    SCROLL_OFFSET.store(0, Ordering::Relaxed);
    render_viewport();
}

/// Alias for mouse scroll
pub fn scroll_up_lines(n: usize) {
    scroll_up(n);
}

/// Alias for mouse scroll  
pub fn scroll_down_lines(n: usize) {
    scroll_down(n);
}

pub fn clear_row(row: usize, fg: Color, bg: Color) {
    if row >= BUFFER_HEIGHT {
        return;
    }
    let mut w = WRITER.lock();
    let prev = w.color_code;
    w.color_code = ColorCode::new(fg, bg);
    w.clear_row(row);
    w.color_code = prev;
}

pub fn write_at(row: usize, col: usize, s: &str, fg: Color, bg: Color) {
    if row >= BUFFER_HEIGHT || col >= BUFFER_WIDTH {
        return;
    }
    let code = ColorCode::new(fg, bg);
    let mut ccol = col;
    let buffer = unsafe { &mut *(0xb8000 as *mut Buffer) };
    for b in s.bytes() {
        if ccol >= BUFFER_WIDTH {
            break;
        }
        let ch = if (0x20..=0x7e).contains(&b) { b } else { 0xfe };
        unsafe {
            core::ptr::write_volatile(
                &mut buffer.chars[row][ccol] as *mut ScreenChar,
                ScreenChar {
                    ascii_character: ch,
                    color_code: code,
                },
            );
        }
        ccol += 1;
    }
}

pub fn _print(args: fmt::Arguments) {
    use fmt::Write;

    // 1) Always update the real VGA writer.
    {
        let mut w = WRITER.lock();
        w.write_fmt(args).ok();
    }

    // 2) Capture into scrollback.
    {
        struct Sink<'a>(&'a mut Scrollback);
        impl<'a> fmt::Write for Sink<'a> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                self.0.write_str(s);
                Ok(())
            }
        }

        let mut sb = SCROLLBACK.lock();
        let mut sink = Sink(&mut sb);
        sink.write_fmt(args).ok();
    }

    // 3) If user is scrolled up, keep the viewport stable.
    if SCROLL_OFFSET.load(Ordering::Relaxed) != 0 {
        render_viewport();
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::vga::_print(core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($fmt:expr) => {
        $crate::print!(concat!($fmt, "\n"))
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::print!(concat!($fmt, "\n"), $($arg)*)
    };
}

pub use crate::println;
