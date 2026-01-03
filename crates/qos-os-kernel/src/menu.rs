//! Text-Mode Menu System for QaOS
//! 
//! Provides interactive menus with mouse and keyboard support.

use alloc::string::String;
use alloc::vec::Vec;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use spin::Mutex;

use crate::vga::{self, Color};

/// Screen dimensions
const SCREEN_WIDTH: usize = 80;
const SCREEN_HEIGHT: usize = 25;

/// Menu bar height
const MENU_BAR_ROW: usize = 0;

/// Menu state
static MENU_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static ACTIVE_MENU_INDEX: AtomicUsize = AtomicUsize::new(0);
static DROPDOWN_OPEN: AtomicBool = AtomicBool::new(false);
static DROPDOWN_SELECTION: AtomicUsize = AtomicUsize::new(0);

/// Callback type for menu actions
pub type MenuCallback = fn();

/// A menu item (can be in menu bar or dropdown)
#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub callback: Option<fn()>,
}

impl MenuItem {
    pub fn new(label: &str) -> Self {
        Self {
            label: String::from(label),
            shortcut: None,
            enabled: true,
            callback: None,
        }
    }
    
    pub fn with_shortcut(mut self, shortcut: &str) -> Self {
        self.shortcut = Some(String::from(shortcut));
        self
    }
    
    pub fn with_callback(mut self, cb: fn()) -> Self {
        self.callback = Some(cb);
        self
    }
    
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// A dropdown menu attached to a menu bar item
#[derive(Clone)]
pub struct DropdownMenu {
    pub title: String,
    pub items: Vec<MenuItem>,
    pub x: usize,  // Screen position
    pub width: usize,
}

impl DropdownMenu {
    pub fn new(title: &str) -> Self {
        Self {
            title: String::from(title),
            items: Vec::new(),
            x: 0,
            width: 0,
        }
    }
    
    pub fn add_item(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }
    
    pub fn add_separator(mut self) -> Self {
        self.items.push(MenuItem {
            label: String::from("---"),
            shortcut: None,
            enabled: false,
            callback: None,
        });
        self
    }
    
    /// Calculate dropdown width based on items
    pub fn calc_width(&mut self) {
        let mut max_label = self.title.len();
        for item in &self.items {
            let mut item_width = item.label.len();
            if let Some(ref sc) = item.shortcut {
                item_width += 2 + sc.len();
            }
            if item_width > max_label {
                max_label = item_width;
            }
        }
        self.width = max_label + 4; // padding
    }
}

/// The main menu bar
pub struct MenuBar {
    pub menus: Vec<DropdownMenu>,
}

impl MenuBar {
    pub fn new() -> Self {
        Self { menus: Vec::new() }
    }
    
    pub fn add_menu(mut self, mut menu: DropdownMenu) -> Self {
        // Calculate position
        let mut x = 1;
        for m in &self.menus {
            x += m.title.len() + 2;
        }
        menu.x = x;
        menu.calc_width();
        self.menus.push(menu);
        self
    }
}

/// Global menu bar
static MENU_BAR: Mutex<Option<MenuBar>> = Mutex::new(None);

/// Initialize the menu system with a menu bar
pub fn init(menu_bar: MenuBar) {
    *MENU_BAR.lock() = Some(menu_bar);
    crate::serial_println!("[Menu] Initialized");
}

/// Draw the menu bar
pub fn draw_menu_bar() {
    let guard = MENU_BAR.lock();
    let menu_bar = match guard.as_ref() {
        Some(mb) => mb,
        None => return,
    };
    
    let active_idx = ACTIVE_MENU_INDEX.load(Ordering::Relaxed);
    let is_active = MENU_ACTIVE.load(Ordering::Relaxed);
    
    // Clear menu bar row
    vga::clear_row(MENU_BAR_ROW, Color::Black, Color::LightGray);
    
    // Draw each menu title
    let mut x = 1;
    for (i, menu) in menu_bar.menus.iter().enumerate() {
        let (fg, bg) = if is_active && i == active_idx {
            (Color::White, Color::Blue)
        } else {
            (Color::Black, Color::LightGray)
        };
        
        // Draw with padding
        vga::write_at(MENU_BAR_ROW, x, " ", fg, bg);
        vga::write_at(MENU_BAR_ROW, x + 1, &menu.title, fg, bg);
        vga::write_at(MENU_BAR_ROW, x + 1 + menu.title.len(), " ", fg, bg);
        
        x += menu.title.len() + 2;
    }
}

/// Draw a dropdown menu
pub fn draw_dropdown(menu_idx: usize) {
    let guard = MENU_BAR.lock();
    let menu_bar = match guard.as_ref() {
        Some(mb) => mb,
        None => return,
    };
    
    if menu_idx >= menu_bar.menus.len() {
        return;
    }
    
    let menu = &menu_bar.menus[menu_idx];
    let selection = DROPDOWN_SELECTION.load(Ordering::Relaxed);
    
    let x = menu.x;
    let y = MENU_BAR_ROW + 1;
    let width = menu.width;
    
    // Draw box and items
    for (i, item) in menu.items.iter().enumerate() {
        let row = y + i;
        if row >= SCREEN_HEIGHT - 1 {
            break;
        }
        
        let is_separator = item.label == "---";
        let is_selected = i == selection && !is_separator;
        
        let (fg, bg) = if is_selected {
            (Color::White, Color::Blue)
        } else if !item.enabled {
            (Color::DarkGray, Color::White)
        } else {
            (Color::Black, Color::White)
        };
        
        // Clear line
        for dx in 0..width {
            if x + dx < SCREEN_WIDTH {
                vga::write_at(row, x + dx, " ", fg, bg);
            }
        }
        
        if is_separator {
            // Draw separator line
            let sep: String = (0..width).map(|_| '─').collect();
            vga::write_at(row, x, &sep, Color::DarkGray, Color::White);
        } else {
            // Draw item label
            vga::write_at(row, x + 1, &item.label, fg, bg);
            
            // Draw shortcut
            if let Some(ref shortcut) = item.shortcut {
                let sc_x = x + width - shortcut.len() - 1;
                vga::write_at(row, sc_x, shortcut, 
                    if is_selected { Color::LightCyan } else { Color::DarkGray },
                    bg);
            }
        }
    }
    
    // Draw shadow
    let bottom_row = y + menu.items.len();
    if bottom_row < SCREEN_HEIGHT {
        for dx in 1..width + 1 {
            if x + dx < SCREEN_WIDTH {
                vga::write_at(bottom_row, x + dx, "▄", Color::DarkGray, Color::Black);
            }
        }
    }
}

/// Open a menu by index
pub fn open_menu(idx: usize) {
    let guard = MENU_BAR.lock();
    let menu_bar = match guard.as_ref() {
        Some(mb) => mb,
        None => return,
    };
    
    if idx >= menu_bar.menus.len() {
        return;
    }
    drop(guard);
    
    MENU_ACTIVE.store(true, Ordering::Relaxed);
    ACTIVE_MENU_INDEX.store(idx, Ordering::Relaxed);
    DROPDOWN_OPEN.store(true, Ordering::Relaxed);
    DROPDOWN_SELECTION.store(0, Ordering::Relaxed);
    
    draw_menu_bar();
    draw_dropdown(idx);
}

/// Close any open menu
pub fn close_menu() {
    if !MENU_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    
    MENU_ACTIVE.store(false, Ordering::Relaxed);
    DROPDOWN_OPEN.store(false, Ordering::Relaxed);
    
    // Redraw menu bar (unhighlighted)
    draw_menu_bar();
    
    // Clear dropdown area (redraw will be handled by shell/UI)
    vga::clear_screen();
    draw_menu_bar();
}

/// Navigate menu selection up
pub fn menu_up() {
    if !DROPDOWN_OPEN.load(Ordering::Relaxed) {
        return;
    }
    
    let (menu_idx, item_count, separator_positions) = {
        let guard = MENU_BAR.lock();
        let menu_bar = match guard.as_ref() {
            Some(mb) => mb,
            None => return,
        };
        
        let idx = ACTIVE_MENU_INDEX.load(Ordering::Relaxed);
        if idx >= menu_bar.menus.len() {
            return;
        }
        
        let menu = &menu_bar.menus[idx];
        let count = menu.items.len();
        
        // Collect separator positions
        let mut seps = Vec::new();
        for (i, item) in menu.items.iter().enumerate() {
            if item.label == "---" {
                seps.push(i);
            }
        }
        (idx, count, seps)
    };
    
    let current = DROPDOWN_SELECTION.load(Ordering::Relaxed);
    
    // Find previous enabled item
    let mut new_sel = current;
    for _ in 0..item_count {
        if new_sel == 0 {
            new_sel = item_count - 1;
        } else {
            new_sel -= 1;
        }
        
        // Skip separators
        if !separator_positions.contains(&new_sel) {
            break;
        }
    }
    
    DROPDOWN_SELECTION.store(new_sel, Ordering::Relaxed);
    draw_dropdown(menu_idx);
}

/// Navigate menu selection down
pub fn menu_down() {
    if !DROPDOWN_OPEN.load(Ordering::Relaxed) {
        return;
    }
    
    let (menu_idx, item_count, separator_positions) = {
        let guard = MENU_BAR.lock();
        let menu_bar = match guard.as_ref() {
            Some(mb) => mb,
            None => return,
        };
        
        let idx = ACTIVE_MENU_INDEX.load(Ordering::Relaxed);
        if idx >= menu_bar.menus.len() {
            return;
        }
        
        let menu = &menu_bar.menus[idx];
        let count = menu.items.len();
        
        // Collect separator positions
        let mut seps = Vec::new();
        for (i, item) in menu.items.iter().enumerate() {
            if item.label == "---" {
                seps.push(i);
            }
        }
        (idx, count, seps)
    };
    
    let current = DROPDOWN_SELECTION.load(Ordering::Relaxed);
    
    // Find next enabled item
    let mut new_sel = current;
    for _ in 0..item_count {
        new_sel = (new_sel + 1) % item_count;
        
        // Skip separators
        if !separator_positions.contains(&new_sel) {
            break;
        }
    }
    
    DROPDOWN_SELECTION.store(new_sel, Ordering::Relaxed);
    draw_dropdown(menu_idx);
}

/// Navigate to next menu (right)
pub fn menu_right() {
    if !MENU_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    
    let guard = MENU_BAR.lock();
    let menu_bar = match guard.as_ref() {
        Some(mb) => mb,
        None => return,
    };
    
    let count = menu_bar.menus.len();
    drop(guard);
    
    if count == 0 {
        return;
    }
    
    let current = ACTIVE_MENU_INDEX.load(Ordering::Relaxed);
    let new_idx = (current + 1) % count;
    
    ACTIVE_MENU_INDEX.store(new_idx, Ordering::Relaxed);
    DROPDOWN_SELECTION.store(0, Ordering::Relaxed);
    
    // Redraw
    vga::clear_screen();
    draw_menu_bar();
    if DROPDOWN_OPEN.load(Ordering::Relaxed) {
        draw_dropdown(new_idx);
    }
}

/// Navigate to previous menu (left)
pub fn menu_left() {
    if !MENU_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    
    let guard = MENU_BAR.lock();
    let menu_bar = match guard.as_ref() {
        Some(mb) => mb,
        None => return,
    };
    
    let count = menu_bar.menus.len();
    drop(guard);
    
    if count == 0 {
        return;
    }
    
    let current = ACTIVE_MENU_INDEX.load(Ordering::Relaxed);
    let new_idx = if current == 0 { count - 1 } else { current - 1 };
    
    ACTIVE_MENU_INDEX.store(new_idx, Ordering::Relaxed);
    DROPDOWN_SELECTION.store(0, Ordering::Relaxed);
    
    // Redraw
    vga::clear_screen();
    draw_menu_bar();
    if DROPDOWN_OPEN.load(Ordering::Relaxed) {
        draw_dropdown(new_idx);
    }
}

/// Execute selected menu item
pub fn menu_select() -> Option<String> {
    if !DROPDOWN_OPEN.load(Ordering::Relaxed) {
        return None;
    }
    
    let guard = MENU_BAR.lock();
    let menu_bar = match guard.as_ref() {
        Some(mb) => mb,
        None => return None,
    };
    
    let menu_idx = ACTIVE_MENU_INDEX.load(Ordering::Relaxed);
    let sel_idx = DROPDOWN_SELECTION.load(Ordering::Relaxed);
    
    if menu_idx >= menu_bar.menus.len() {
        return None;
    }
    
    let menu = &menu_bar.menus[menu_idx];
    let item = menu.items.get(sel_idx)?;
    
    if !item.enabled || item.label == "---" {
        return None;
    }
    
    let label = item.label.clone();
    let callback = item.callback;
    drop(guard);
    
    // Close menu
    close_menu();
    
    // Execute callback if any
    if let Some(cb) = callback {
        cb();
    }
    
    Some(label)
}

/// Check if menu is currently active
pub fn is_active() -> bool {
    MENU_ACTIVE.load(Ordering::Relaxed)
}

/// Handle F10 key (toggle menu)
pub fn toggle() {
    if is_active() {
        close_menu();
    } else {
        open_menu(0);
    }
}

/// Handle a menu bar click at column x
pub fn handle_click(x: usize) {
    let guard = MENU_BAR.lock();
    let menu_bar = match guard.as_ref() {
        Some(mb) => mb,
        None => return,
    };
    
    // Find which menu was clicked
    let mut menu_x = 1;
    for (i, menu) in menu_bar.menus.iter().enumerate() {
        let menu_end = menu_x + menu.title.len() + 2;
        if x >= menu_x && x < menu_end {
            drop(guard);
            open_menu(i);
            return;
        }
        menu_x = menu_end;
    }
}

/// Create default QaOS menu bar
pub fn create_default_menu() -> MenuBar {
    MenuBar::new()
        .add_menu(
            DropdownMenu::new("File")
                .add_item(MenuItem::new("New").with_shortcut("Ctrl+N"))
                .add_item(MenuItem::new("Open...").with_shortcut("Ctrl+O"))
                .add_item(MenuItem::new("Save").with_shortcut("Ctrl+S"))
                .add_item(MenuItem::new("Save As..."))
                .add_separator()
                .add_item(MenuItem::new("Exit").with_shortcut("Alt+F4"))
        )
        .add_menu(
            DropdownMenu::new("Edit")
                .add_item(MenuItem::new("Cut").with_shortcut("Ctrl+X"))
                .add_item(MenuItem::new("Copy").with_shortcut("Ctrl+C"))
                .add_item(MenuItem::new("Paste").with_shortcut("Ctrl+V"))
                .add_separator()
                .add_item(MenuItem::new("Select All").with_shortcut("Ctrl+A"))
        )
        .add_menu(
            DropdownMenu::new("View")
                .add_item(MenuItem::new("UI Panel").with_shortcut("F12"))
                .add_item(MenuItem::new("Full Screen").with_shortcut("F11"))
                .add_separator()
                .add_item(MenuItem::new("Refresh").with_shortcut("F5"))
        )
        .add_menu(
            DropdownMenu::new("Quantum")
                .add_item(MenuItem::new("New Circuit"))
                .add_item(MenuItem::new("Submit Job"))
                .add_item(MenuItem::new("Job Status"))
                .add_separator()
                .add_item(MenuItem::new("Simulator Settings"))
        )
        .add_menu(
            DropdownMenu::new("Help")
                .add_item(MenuItem::new("Commands"))
                .add_item(MenuItem::new("About QaOS"))
        )
}
