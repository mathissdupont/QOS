//! Dialog Box System for QaOS
//! 
//! Provides modal dialogs: message box, confirm, input, file picker

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use spin::Mutex;

use crate::vga::{self, Color};
use crate::keyboard;

/// Dialog result
#[derive(Clone, Debug)]
pub enum DialogResult {
    Ok,
    Cancel,
    Yes,
    No,
    Input(String),
    Selected(usize),
}

/// Dialog state
static DIALOG_ACTIVE: AtomicBool = AtomicBool::new(false);
static DIALOG_RESULT: Mutex<Option<DialogResult>> = Mutex::new(None);
static DIALOG_SELECTION: AtomicUsize = AtomicUsize::new(0);

/// Check if a dialog is currently active
pub fn is_active() -> bool {
    DIALOG_ACTIVE.load(Ordering::Relaxed)
}

/// Box drawing characters (ASCII-safe)
const BOX_TL: char = '+';
const BOX_TR: char = '+';
const BOX_BL: char = '+';
const BOX_BR: char = '+';
const BOX_H: char = '-';
const BOX_V: char = '|';

/// Draw a box at position
fn draw_box(x: usize, y: usize, width: usize, height: usize, fg: Color, bg: Color) {
    // Top border
    vga::write_at(y, x, &alloc::format!("{}", BOX_TL), fg, bg);
    for dx in 1..width-1 {
        vga::write_at(y, x + dx, &alloc::format!("{}", BOX_H), fg, bg);
    }
    vga::write_at(y, x + width - 1, &alloc::format!("{}", BOX_TR), fg, bg);
    
    // Middle rows
    for dy in 1..height-1 {
        vga::write_at(y + dy, x, &alloc::format!("{}", BOX_V), fg, bg);
        for dx in 1..width-1 {
            vga::write_at(y + dy, x + dx, " ", fg, bg);
        }
        vga::write_at(y + dy, x + width - 1, &alloc::format!("{}", BOX_V), fg, bg);
    }
    
    // Bottom border
    vga::write_at(y + height - 1, x, &alloc::format!("{}", BOX_BL), fg, bg);
    for dx in 1..width-1 {
        vga::write_at(y + height - 1, x + dx, &alloc::format!("{}", BOX_H), fg, bg);
    }
    vga::write_at(y + height - 1, x + width - 1, &alloc::format!("{}", BOX_BR), fg, bg);
}

/// Draw centered text
fn draw_centered(y: usize, box_x: usize, box_width: usize, text: &str, fg: Color, bg: Color) {
    let text_len = text.len().min(box_width - 2);
    let x = box_x + (box_width - text_len) / 2;
    vga::write_at(y, x, &text[..text_len], fg, bg);
}

/// Draw a button
fn draw_button(x: usize, y: usize, label: &str, selected: bool) {
    let (fg, bg) = if selected {
        (Color::White, Color::Blue)
    } else {
        (Color::Black, Color::LightGray)
    };
    
    let btn = alloc::format!("[ {} ]", label);
    vga::write_at(y, x, &btn, fg, bg);
}

/// Show a message box (OK button only)
pub fn message_box(title: &str, message: &str) {
    let width = message.len().max(title.len()).max(10) + 6;
    let width = width.min(70);
    let height = 7;
    let x = (80 - width) / 2;
    let y = (25 - height) / 2;
    
    // Draw dialog
    draw_box(x, y, width, height, Color::White, Color::Blue);
    
    // Title bar
    for dx in 1..width-1 {
        vga::write_at(y, x + dx, " ", Color::White, Color::Cyan);
    }
    draw_centered(y, x, width, title, Color::White, Color::Cyan);
    
    // Message
    draw_centered(y + 2, x, width, message, Color::White, Color::Blue);
    
    // OK button
    let btn_x = x + (width - 6) / 2;
    draw_button(btn_x, y + 4, "OK", true);
    
    DIALOG_ACTIVE.store(true, Ordering::Relaxed);
    
    // Wait for Enter or Escape
    wait_for_key_simple();
    
    DIALOG_ACTIVE.store(false, Ordering::Relaxed);
    vga::clear_screen();
}

/// Show a confirm dialog (Yes/No)
pub fn confirm(title: &str, message: &str) -> bool {
    let width = message.len().max(title.len()).max(20) + 6;
    let width = width.min(70);
    let height = 7;
    let x = (80 - width) / 2;
    let y = (25 - height) / 2;
    
    DIALOG_ACTIVE.store(true, Ordering::Relaxed);
    DIALOG_SELECTION.store(0, Ordering::Relaxed);
    
    loop {
        let selection = DIALOG_SELECTION.load(Ordering::Relaxed);
        
        // Draw dialog
        draw_box(x, y, width, height, Color::White, Color::Blue);
        
        // Title bar
        for dx in 1..width-1 {
            vga::write_at(y, x + dx, " ", Color::White, Color::Cyan);
        }
        draw_centered(y, x, width, title, Color::White, Color::Cyan);
        
        // Message
        draw_centered(y + 2, x, width, message, Color::White, Color::Blue);
        
        // Buttons
        let btn_y = y + 4;
        let yes_x = x + width / 2 - 10;
        let no_x = x + width / 2 + 3;
        
        draw_button(yes_x, btn_y, "Yes", selection == 0);
        draw_button(no_x, btn_y, "No", selection == 1);
        
        // Handle input
        match wait_for_key() {
            KeyAction::Left => {
                if selection > 0 {
                    DIALOG_SELECTION.store(0, Ordering::Relaxed);
                }
            }
            KeyAction::Right => {
                if selection < 1 {
                    DIALOG_SELECTION.store(1, Ordering::Relaxed);
                }
            }
            KeyAction::Enter => {
                DIALOG_ACTIVE.store(false, Ordering::Relaxed);
                vga::clear_screen();
                return selection == 0;
            }
            KeyAction::Escape => {
                DIALOG_ACTIVE.store(false, Ordering::Relaxed);
                vga::clear_screen();
                return false;
            }
            _ => {}
        }
    }
}

/// Show an input dialog
pub fn input_box(title: &str, prompt: &str, default: &str) -> Option<String> {
    let width = 50;
    let height = 8;
    let x = (80 - width) / 2;
    let y = (25 - height) / 2;
    
    let mut input = String::from(default);
    
    DIALOG_ACTIVE.store(true, Ordering::Relaxed);
    
    loop {
        // Draw dialog
        draw_box(x, y, width, height, Color::White, Color::Blue);
        
        // Title bar
        for dx in 1..width-1 {
            vga::write_at(y, x + dx, " ", Color::White, Color::Cyan);
        }
        draw_centered(y, x, width, title, Color::White, Color::Cyan);
        
        // Prompt
        vga::write_at(y + 2, x + 2, prompt, Color::White, Color::Blue);
        
        // Input field
        let field_y = y + 3;
        let field_x = x + 2;
        let field_width = width - 4;
        
        // Draw input background
        for dx in 0..field_width {
            vga::write_at(field_y, field_x + dx, " ", Color::Black, Color::White);
        }
        
        // Draw input text with cursor
        let display: String = if input.len() < field_width - 1 {
            alloc::format!("{}_", input)
        } else {
            let start = input.len() - (field_width - 2);
            alloc::format!("{}_", &input[start..])
        };
        vga::write_at(field_y, field_x, &display, Color::Black, Color::White);
        
        // Buttons
        let btn_y = y + 5;
        draw_button(x + width/2 - 12, btn_y, "OK", true);
        draw_button(x + width/2 + 2, btn_y, "Cancel", false);
        
        // Handle input
        match wait_for_key() {
            KeyAction::Char(c) => {
                if input.len() < 200 {
                    input.push(c);
                }
            }
            KeyAction::Backspace => {
                input.pop();
            }
            KeyAction::Enter => {
                DIALOG_ACTIVE.store(false, Ordering::Relaxed);
                vga::clear_screen();
                return Some(input);
            }
            KeyAction::Escape => {
                DIALOG_ACTIVE.store(false, Ordering::Relaxed);
                vga::clear_screen();
                return None;
            }
            _ => {}
        }
    }
}

/// Show a selection list dialog
pub fn select_list(title: &str, items: &[&str]) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    
    let max_item_len = items.iter().map(|s| s.len()).max().unwrap_or(10);
    let width = max_item_len.max(title.len()) + 6;
    let width = width.min(70);
    let visible_items = items.len().min(10);
    let height = visible_items + 4;
    let x = (80 - width) / 2;
    let y = (25 - height) / 2;
    
    DIALOG_ACTIVE.store(true, Ordering::Relaxed);
    DIALOG_SELECTION.store(0, Ordering::Relaxed);
    
    let mut scroll_offset = 0usize;
    
    loop {
        let selection = DIALOG_SELECTION.load(Ordering::Relaxed);
        
        // Adjust scroll
        if selection < scroll_offset {
            scroll_offset = selection;
        } else if selection >= scroll_offset + visible_items {
            scroll_offset = selection - visible_items + 1;
        }
        
        // Draw dialog
        draw_box(x, y, width, height, Color::White, Color::Blue);
        
        // Title bar
        for dx in 1..width-1 {
            vga::write_at(y, x + dx, " ", Color::White, Color::Cyan);
        }
        draw_centered(y, x, width, title, Color::White, Color::Cyan);
        
        // Items
        for (i, idx) in (scroll_offset..scroll_offset + visible_items).enumerate() {
            if idx >= items.len() {
                break;
            }
            
            let item_y = y + 2 + i;
            let (fg, bg) = if idx == selection {
                (Color::White, Color::DarkGray)
            } else {
                (Color::White, Color::Blue)
            };
            
            // Clear line
            for dx in 1..width-1 {
                vga::write_at(item_y, x + dx, " ", fg, bg);
            }
            
            let item_text = if items[idx].len() > width - 4 {
                &items[idx][..width-4]
            } else {
                items[idx]
            };
            vga::write_at(item_y, x + 2, item_text, fg, bg);
        }
        
        // Scroll indicator
        if items.len() > visible_items {
            let indicator = alloc::format!("{}/{}", selection + 1, items.len());
            vga::write_at(y + height - 1, x + width - indicator.len() - 2, 
                &indicator, Color::Yellow, Color::Blue);
        }
        
        // Handle input
        match wait_for_key() {
            KeyAction::Up => {
                if selection > 0 {
                    DIALOG_SELECTION.store(selection - 1, Ordering::Relaxed);
                }
            }
            KeyAction::Down => {
                if selection < items.len() - 1 {
                    DIALOG_SELECTION.store(selection + 1, Ordering::Relaxed);
                }
            }
            KeyAction::Enter => {
                DIALOG_ACTIVE.store(false, Ordering::Relaxed);
                vga::clear_screen();
                return Some(selection);
            }
            KeyAction::Escape => {
                DIALOG_ACTIVE.store(false, Ordering::Relaxed);
                vga::clear_screen();
                return None;
            }
            _ => {}
        }
    }
}

/// Key action for dialog handling
enum KeyAction {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Backspace,
    Char(char),
    None,
}

/// Wait for a key and return action
fn wait_for_key() -> KeyAction {
    use pc_keyboard::{layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1};
    
    let mut kb = Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::Ignore);
    
    loop {
        if let Some(sc) = keyboard::pop_scancode() {
            if let Ok(Some(event)) = kb.add_byte(sc) {
                if let Some(key) = kb.process_keyevent(event) {
                    return match key {
                        DecodedKey::Unicode('\n') => KeyAction::Enter,
                        DecodedKey::Unicode('\x1b') => KeyAction::Escape,
                        DecodedKey::Unicode('\u{0008}') => KeyAction::Backspace,
                        DecodedKey::Unicode(c) if c.is_ascii_graphic() || c == ' ' => KeyAction::Char(c),
                        DecodedKey::RawKey(KeyCode::ArrowUp) => KeyAction::Up,
                        DecodedKey::RawKey(KeyCode::ArrowDown) => KeyAction::Down,
                        DecodedKey::RawKey(KeyCode::ArrowLeft) => KeyAction::Left,
                        DecodedKey::RawKey(KeyCode::ArrowRight) => KeyAction::Right,
                        DecodedKey::RawKey(KeyCode::Escape) => KeyAction::Escape,
                        _ => KeyAction::None,
                    };
                }
            }
        }
        
        // Yield CPU
        crate::arch::hlt();
    }
}

/// Simple wait for Enter or Escape
fn wait_for_key_simple() {
    use pc_keyboard::{layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1};
    
    let mut kb = Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::Ignore);
    
    loop {
        if let Some(sc) = keyboard::pop_scancode() {
            if let Ok(Some(event)) = kb.add_byte(sc) {
                if let Some(key) = kb.process_keyevent(event) {
                    match key {
                        DecodedKey::Unicode('\n') => return,
                        DecodedKey::RawKey(KeyCode::Escape) => return,
                        _ => {}
                    }
                }
            }
        }
        crate::arch::hlt();
    }
}
