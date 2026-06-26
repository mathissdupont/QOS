//! Desktop Manager - Windows-Style GUI System for QOS
//!
//! Provides a full desktop environment with windows, taskbar, and desktop icons.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::boxed::Box;
use spin::Mutex;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::gui::{self, DrawContext, ColorAttr, Color, Rect, SCREEN_WIDTH, SCREEN_HEIGHT};

// ==================== Constants ====================

const TASKBAR_HEIGHT: usize = 2;
const WINDOW_BORDER_SIZE: usize = 1;
const TITLE_BAR_HEIGHT: usize = 1;

// ==================== Window System ====================

static WINDOW_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    Normal,
    Minimized,
    Maximized,
}

/// A GUI Window
pub struct Window {
    pub id: u32,
    pub title: String,
    pub rect: Rect,
    pub state: WindowState,
    pub is_focused: bool,
    pub content: Vec<String>,
    pub bg_color: Color,
    pub border_color: Color,
}

impl Window {
    pub fn new(title: &str, x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            id: WINDOW_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
            title: title.to_string(),
            rect: Rect::new(x, y, width, height),
            state: WindowState::Normal,
            is_focused: false,
            content: Vec::new(),
            bg_color: Color::White,
            border_color: Color::Blue,
        }
    }

    /// Add content line to window
    pub fn add_line(&mut self, line: &str) {
        self.content.push(line.to_string());
    }

    /// Clear window content
    pub fn clear(&mut self) {
        self.content.clear();
    }

    /// Render this window
    pub fn render(&self, ctx: &DrawContext) {
        if self.state == WindowState::Minimized {
            return;
        }

        let rect = if self.state == WindowState::Maximized {
            Rect::new(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT - TASKBAR_HEIGHT)
        } else {
            self.rect
        };

        // Draw border
        let border_attr = if self.is_focused {
            ColorAttr::new(Color::White, self.border_color)
        } else {
            ColorAttr::new(Color::LightGray, Color::DarkGray)
        };

        // Draw window frame
        for y in rect.y..rect.y + rect.height {
            for x in rect.x..rect.x + rect.width {
                let is_border = x == rect.x || x == rect.x + rect.width - 1 ||
                                y == rect.y || y == rect.y + rect.height - 1;
                let is_titlebar = y == rect.y && x > rect.x && x < rect.x + rect.width - 1;

                if is_border {
                    ctx.put_char(x, y, '█', border_attr);
                } else if is_titlebar {
                    ctx.put_char(x, y, ' ', ColorAttr::new(Color::White, self.border_color));
                } else {
                    ctx.put_char(x, y, ' ', ColorAttr::new(Color::Black, self.bg_color));
                }
            }
        }

        // Draw title
        let title_x = rect.x + 2;
        let title_text = if self.title.len() > rect.width - 8 {
            &self.title[..rect.width - 11]
        } else {
            &self.title
        };
        ctx.put_str(title_x, rect.y, title_text, ColorAttr::new(Color::White, self.border_color));

        // Draw window buttons [_][□][X]
        let close_x = rect.x + rect.width - 3;
        ctx.put_str(close_x, rect.y, "X", ColorAttr::new(Color::Yellow, Color::Red));
        ctx.put_str(close_x - 3, rect.y, "□", ColorAttr::new(Color::White, self.border_color));
        ctx.put_str(close_x - 6, rect.y, "_", ColorAttr::new(Color::White, self.border_color));

        // Draw content
        let content_y = rect.y + TITLE_BAR_HEIGHT + WINDOW_BORDER_SIZE;
        let content_x = rect.x + WINDOW_BORDER_SIZE + 1;
        let max_content_lines = rect.height.saturating_sub(TITLE_BAR_HEIGHT + 2 * WINDOW_BORDER_SIZE + 1);
        
        for (i, line) in self.content.iter().take(max_content_lines).enumerate() {
            let max_len = rect.width.saturating_sub(2 * WINDOW_BORDER_SIZE + 2);
            let display_line = if line.len() > max_len {
                &line[..max_len]
            } else {
                line
            };
            ctx.put_str(content_x, content_y + i, display_line, 
                       ColorAttr::new(Color::Black, self.bg_color));
        }
    }
}

// ==================== Desktop Manager ====================

pub struct Desktop {
    windows: Vec<Window>,
    focused_window: Option<u32>,
    desktop_icons: Vec<DesktopIcon>,
    wallpaper_char: char,
    wallpaper_color: Color,
}

impl Desktop {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            focused_window: None,
            desktop_icons: Vec::new(),
            wallpaper_char: ' ',
            wallpaper_color: Color::Black,
        }
    }

    /// Create a new window and return its ID
    pub fn create_window(&mut self, title: &str, x: usize, y: usize, width: usize, height: usize) -> u32 {
        let mut window = Window::new(title, x, y, width, height);
        let id = window.id;
        
        // Focus this window
        for w in &mut self.windows {
            w.is_focused = false;
        }
        window.is_focused = true;
        self.focused_window = Some(id);
        
        self.windows.push(window);
        id
    }

    /// Get mutable reference to a window
    pub fn get_window_mut(&mut self, id: u32) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Close a window
    pub fn close_window(&mut self, id: u32) {
        self.windows.retain(|w| w.id != id);
        if self.focused_window == Some(id) {
            self.focused_window = self.windows.last().map(|w| w.id);
            if let Some(w) = self.windows.last_mut() {
                w.is_focused = true;
            }
        }
    }

    /// Focus a window
    pub fn focus_window(&mut self, id: u32) {
        for w in &mut self.windows {
            w.is_focused = w.id == id;
        }
        self.focused_window = Some(id);
    }

    /// Add desktop icon
    pub fn add_icon(&mut self, icon: DesktopIcon) {
        self.desktop_icons.push(icon);
    }

    /// Render the entire desktop
    pub fn render(&self) {
        let ctx = DrawContext::new();

        // Draw wallpaper (simple dark background)
        for y in 0..SCREEN_HEIGHT - TASKBAR_HEIGHT {
            for x in 0..SCREEN_WIDTH {
                ctx.put_char(x, y, ' ', 
                           ColorAttr::new(Color::LightGray, Color::Black));
            }
        }

        // Draw desktop icons
        for icon in &self.desktop_icons {
            icon.render(&ctx);
        }

        // Draw windows (back to front)
        for window in &self.windows {
            window.render(&ctx);
        }

        // Draw taskbar
        self.render_taskbar(&ctx);
    }

    /// Render taskbar
    fn render_taskbar(&self, ctx: &DrawContext) {
        let taskbar_y = SCREEN_HEIGHT - TASKBAR_HEIGHT;
        
        // Taskbar background
        for y in taskbar_y..SCREEN_HEIGHT {
            for x in 0..SCREEN_WIDTH {
                ctx.put_char(x, y, ' ', ColorAttr::new(Color::White, Color::DarkGray));
            }
        }

        // Start button
        ctx.put_str(1, taskbar_y, "[ QOS ]", ColorAttr::new(Color::Yellow, Color::Blue));

        // Window buttons
        let mut x = 10;
        for window in &self.windows {
            if window.state != WindowState::Minimized {
                let button_text = if window.title.len() > 12 {
                    alloc::format!("[{}...]", &window.title[..9])
                } else {
                    alloc::format!("[{}]", window.title)
                };
                
                let attr = if window.is_focused {
                    ColorAttr::new(Color::White, Color::Blue)
                } else {
                    ColorAttr::new(Color::Black, Color::LightGray)
                };
                
                ctx.put_str(x, taskbar_y, &button_text, attr);
                x += button_text.len() + 1;
            }
        }

        // System tray (right side)
        let time_str = "12:34";  // TODO: Get real time
        ctx.put_str(SCREEN_WIDTH - time_str.len() - 1, taskbar_y, time_str, 
                   ColorAttr::new(Color::White, Color::DarkGray));
    }
}

// ==================== Desktop Icons ====================

pub struct DesktopIcon {
    pub name: String,
    pub x: usize,
    pub y: usize,
    pub icon_char: char,
    pub color: Color,
}

impl DesktopIcon {
    pub fn new(name: &str, x: usize, y: usize, icon_char: char) -> Self {
        Self {
            name: name.to_string(),
            x,
            y,
            icon_char,
            color: Color::White,
        }
    }

    fn render(&self, ctx: &DrawContext) {
        // Icon box
        ctx.put_char(self.x, self.y, '[', ColorAttr::new(Color::Yellow, Color::Black));
        ctx.put_char(self.x + 1, self.y, self.icon_char, ColorAttr::new(Color::White, Color::Black));
        ctx.put_char(self.x + 2, self.y, ']', ColorAttr::new(Color::Yellow, Color::Black));
        
        // Label (below icon)
        let label_y = self.y + 1;
        let max_len = 10;
        let label = if self.name.len() > max_len {
            &self.name[..max_len]
        } else {
            &self.name
        };
        
        let start_x = self.x;
        
        ctx.put_str(start_x, label_y, label, 
                   ColorAttr::new(Color::White, Color::Black));
    }
}

// ==================== Global Desktop Instance ====================

static DESKTOP: Mutex<Option<Desktop>> = Mutex::new(None);

/// Initialize desktop environment
pub fn init() {
    let mut desktop = Desktop::new();
    
    // Add default desktop icons (ASCII only for VGA text mode)
    desktop.add_icon(DesktopIcon::new("Computer", 2, 1, 'C'));
    desktop.add_icon(DesktopIcon::new("Files", 2, 4, 'F'));
    desktop.add_icon(DesktopIcon::new("Terminal", 2, 7, 'T'));
    desktop.add_icon(DesktopIcon::new("Settings", 2, 10, 'S'));
    
    *DESKTOP.lock() = Some(desktop);
    
    crate::serial_println!("[Desktop] Initialized");
}

/// Create a new window
pub fn create_window(title: &str, x: usize, y: usize, width: usize, height: usize) -> u32 {
    DESKTOP.lock().as_mut()
        .map(|d| d.create_window(title, x, y, width, height))
        .unwrap_or(0)
}

/// Add content to window
pub fn window_add_line(window_id: u32, line: &str) {
    if let Some(ref mut desktop) = *DESKTOP.lock() {
        if let Some(window) = desktop.get_window_mut(window_id) {
            window.add_line(line);
        }
    }
}

/// Clear window content
pub fn window_clear(window_id: u32) {
    if let Some(ref mut desktop) = *DESKTOP.lock() {
        if let Some(window) = desktop.get_window_mut(window_id) {
            window.clear();
        }
    }
}

/// Close a window
pub fn close_window(window_id: u32) {
    if let Some(ref mut desktop) = *DESKTOP.lock() {
        desktop.close_window(window_id);
    }
}

/// Render desktop
pub fn render() {
    if let Some(ref desktop) = *DESKTOP.lock() {
        desktop.render();
    }
}

/// Show a demo of the desktop system
pub fn demo() {
    // Create welcome window
    let welcome_win = create_window("Welcome to QOS", 10, 3, 60, 12);
    window_add_line(welcome_win, "");
    window_add_line(welcome_win, "  Welcome to QOS - Quantum Operating System");
    window_add_line(welcome_win, "");
    window_add_line(welcome_win, "  This is a desktop environment!");
    window_add_line(welcome_win, "");
    window_add_line(welcome_win, "  Features:");
    window_add_line(welcome_win, "  - Multiple windows");
    window_add_line(welcome_win, "  - Taskbar with window buttons");
    window_add_line(welcome_win, "  - Desktop icons");
    window_add_line(welcome_win, "  - Window focus management");
    
    // Create another window
    let info_win = create_window("System Info", 15, 8, 50, 10);
    window_add_line(info_win, "");
    window_add_line(info_win, "  QOS Desktop Environment v1.0");
    window_add_line(info_win, "");
    window_add_line(info_win, "  Resolution: 80x25 (text mode)");
    window_add_line(info_win, "  Color depth: 16 colors");
    window_add_line(info_win, "  Memory: Available");
    window_add_line(info_win, "  Status: Running");
    
    // Render
    render();
}
