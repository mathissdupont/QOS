//! Simple GUI System for QOS
//!
//! Provides basic text-mode graphical interface with status bar.

use alloc::string::String;
use spin::Mutex;

/// Screen dimensions (VGA text mode)
pub const SCREEN_WIDTH: usize = 80;
pub const SCREEN_HEIGHT: usize = 25;

/// Color palette
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Color attribute
#[derive(Debug, Clone, Copy)]
pub struct ColorAttr {
    pub fg: Color,
    pub bg: Color,
}

impl ColorAttr {
    pub const fn new(fg: Color, bg: Color) -> Self {
        Self { fg, bg }
    }
    
    pub fn to_vga(&self) -> u8 {
        (self.bg as u8) << 4 | (self.fg as u8)
    }
}

/// Common color schemes
pub mod colors {
    use super::*;
    pub const NORMAL: ColorAttr = ColorAttr::new(Color::LightGray, Color::Black);
    pub const STATUS: ColorAttr = ColorAttr::new(Color::Black, Color::LightGray);
    pub const TITLE: ColorAttr = ColorAttr::new(Color::White, Color::Blue);
    pub const HIGHLIGHT: ColorAttr = ColorAttr::new(Color::Yellow, Color::Black);
    pub const ERROR: ColorAttr = ColorAttr::new(Color::White, Color::Red);
}

/// Box drawing characters
pub mod box_chars {
    pub const TOP_LEFT: char = '┌';
    pub const TOP_RIGHT: char = '┐';
    pub const BOTTOM_LEFT: char = '└';
    pub const BOTTOM_RIGHT: char = '┘';
    pub const HORIZONTAL: char = '─';
    pub const VERTICAL: char = '│';
}

/// Rectangle
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self { x, y, width, height }
    }
}

/// Draw context for VGA
pub struct DrawContext {
    buffer: *mut u16,
}

impl DrawContext {
    pub fn new() -> Self {
        Self {
            buffer: 0xB8000 as *mut u16,
        }
    }
    
    pub fn put_char(&self, x: usize, y: usize, c: char, attr: ColorAttr) {
        if x >= SCREEN_WIDTH || y >= SCREEN_HEIGHT {
            return;
        }
        let idx = y * SCREEN_WIDTH + x;
        unsafe {
            *self.buffer.add(idx) = (attr.to_vga() as u16) << 8 | (c as u16);
        }
    }
    
    pub fn put_str(&self, x: usize, y: usize, s: &str, attr: ColorAttr) {
        for (i, c) in s.chars().enumerate() {
            if x + i >= SCREEN_WIDTH {
                break;
            }
            self.put_char(x + i, y, c, attr);
        }
    }
    
    pub fn fill_line(&self, y: usize, c: char, attr: ColorAttr) {
        for x in 0..SCREEN_WIDTH {
            self.put_char(x, y, c, attr);
        }
    }
    
    pub fn draw_box(&self, rect: Rect, attr: ColorAttr) {
        // Corners
        self.put_char(rect.x, rect.y, box_chars::TOP_LEFT, attr);
        self.put_char(rect.x + rect.width - 1, rect.y, box_chars::TOP_RIGHT, attr);
        self.put_char(rect.x, rect.y + rect.height - 1, box_chars::BOTTOM_LEFT, attr);
        self.put_char(rect.x + rect.width - 1, rect.y + rect.height - 1, box_chars::BOTTOM_RIGHT, attr);
        
        // Horizontal lines
        for x in rect.x + 1..rect.x + rect.width - 1 {
            self.put_char(x, rect.y, box_chars::HORIZONTAL, attr);
            self.put_char(x, rect.y + rect.height - 1, box_chars::HORIZONTAL, attr);
        }
        
        // Vertical lines
        for y in rect.y + 1..rect.y + rect.height - 1 {
            self.put_char(rect.x, y, box_chars::VERTICAL, attr);
            self.put_char(rect.x + rect.width - 1, y, box_chars::VERTICAL, attr);
        }
    }
}

impl Default for DrawContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Status bar state
struct StatusBarState {
    left: String,
    center: String,
    right: String,
}

static STATUS_BAR: Mutex<StatusBarState> = Mutex::new(StatusBarState {
    left: String::new(),
    center: String::new(),
    right: String::new(),
});

/// Initialize GUI system
pub fn init() {
    crate::serial_println!("[GUI] Initialized");
}

/// Update status bar text
pub fn set_status(left: &str, center: &str, right: &str) {
    let mut state = STATUS_BAR.lock();
    state.left = String::from(left);
    state.center = String::from(center);
    state.right = String::from(right);
}

/// Draw status bar at bottom of screen
pub fn draw_status_bar() {
    let ctx = DrawContext::new();
    let state = STATUS_BAR.lock();
    let y = SCREEN_HEIGHT - 1;
    
    // Clear line
    ctx.fill_line(y, ' ', colors::STATUS);
    
    // Left
    ctx.put_str(1, y, &state.left, colors::STATUS);
    
    // Center
    let center_x = (SCREEN_WIDTH - state.center.len()) / 2;
    ctx.put_str(center_x, y, &state.center, colors::STATUS);
    
    // Right
    if !state.right.is_empty() {
        let right_x = SCREEN_WIDTH - state.right.len() - 1;
        ctx.put_str(right_x, y, &state.right, colors::STATUS);
    }
}

/// Draw a simple message box
pub fn message_box(title: &str, message: &str) {
    let ctx = DrawContext::new();
    
    let width = core::cmp::max(title.len(), message.len()) + 4;
    let width = core::cmp::min(width, 60);
    let x = (SCREEN_WIDTH - width) / 2;
    let y = 8;
    
    let rect = Rect::new(x, y, width, 5);
    
    // Draw border
    ctx.draw_box(rect, colors::TITLE);
    
    // Fill inside
    for dy in 1..4 {
        for dx in 1..width - 1 {
            ctx.put_char(x + dx, y + dy, ' ', colors::NORMAL);
        }
    }
    
    // Title
    let title_x = x + (width - title.len()) / 2;
    ctx.put_str(title_x, y, title, colors::TITLE);
    
    // Message
    let msg_x = x + (width - message.len()) / 2;
    ctx.put_str(msg_x, y + 2, message, colors::NORMAL);
}

/// Draw progress bar
pub fn progress_bar(x: usize, y: usize, width: usize, progress: f32) {
    let ctx = DrawContext::new();
    let filled = (progress * width as f32) as usize;
    
    for i in 0..width {
        let c = if i < filled { '█' } else { '░' };
        let attr = if i < filled {
            ColorAttr::new(Color::Green, Color::Black)
        } else {
            ColorAttr::new(Color::DarkGray, Color::Black)
        };
        ctx.put_char(x + i, y, c, attr);
    }
}
