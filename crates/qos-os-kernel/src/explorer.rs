//! Text-Mode File Explorer for QaOS
//! 
//! A simple file browser with navigation and file operations

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use spin::Mutex;

use crate::vga::{self, Color};
use crate::keyboard;
use crate::fs;

/// Key actions for explorer
enum KeyAction {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Escape,
    Backspace,
    None,
}

/// Explorer state
static EXPLORER_ACTIVE: AtomicBool = AtomicBool::new(false);
static CURRENT_PATH: Mutex<String> = Mutex::new(String::new());
static FILE_LIST: Mutex<Vec<FileEntry>> = Mutex::new(Vec::new());
static SELECTION: AtomicUsize = AtomicUsize::new(0);
static SCROLL_OFFSET: AtomicUsize = AtomicUsize::new(0);
static SELECTED_FILE: Mutex<Option<String>> = Mutex::new(None);

/// Number of visible items in the list
const VISIBLE_ITEMS: usize = 16;
const EXPLORER_WIDTH: usize = 60;
const EXPLORER_HEIGHT: usize = 20;
const EXPLORER_X: usize = 10;
const EXPLORER_Y: usize = 2;

/// File entry in the list
#[derive(Clone)]
struct FileEntry {
    name: String,
    full_path: String,
    is_dir: bool,
    size: usize,
}

/// Check if explorer is active
pub fn is_active() -> bool {
    EXPLORER_ACTIVE.load(Ordering::Relaxed)
}

/// Get selected file (if any)
pub fn take_selected() -> Option<String> {
    SELECTED_FILE.lock().take()
}

/// Open file explorer and return selected file path
pub fn open() -> Option<Vec<u8>> {
    // Initialize
    {
        let mut current = CURRENT_PATH.lock();
        current.clear();
        current.push_str("/");
    }
    *SELECTED_FILE.lock() = None;
    
    refresh_file_list();
    SELECTION.store(0, Ordering::Relaxed);
    SCROLL_OFFSET.store(0, Ordering::Relaxed);
    EXPLORER_ACTIVE.store(true, Ordering::Relaxed);
    
    draw();
    
    // Main loop
    loop {
        match wait_for_key() {
            KeyAction::Escape => {
                // Cancel
                close();
                return None;
            }
            KeyAction::Enter => {
                // Select item
                if let Some(path) = select_item() {
                    close();
                    return Some(path.into_bytes());
                }
            }
            KeyAction::Up => {
                move_selection(-1);
                draw();
            }
            KeyAction::Down => {
                move_selection(1);
                draw();
            }
            KeyAction::PageUp => {
                move_selection(-(VISIBLE_ITEMS as isize));
                draw();
            }
            KeyAction::PageDown => {
                move_selection(VISIBLE_ITEMS as isize);
                draw();
            }
            KeyAction::Home => {
                SELECTION.store(0, Ordering::Relaxed);
                SCROLL_OFFSET.store(0, Ordering::Relaxed);
                draw();
            }
            KeyAction::End => {
                let count = FILE_LIST.lock().len();
                if count > 0 {
                    SELECTION.store(count - 1, Ordering::Relaxed);
                    if count > VISIBLE_ITEMS {
                        SCROLL_OFFSET.store(count - VISIBLE_ITEMS, Ordering::Relaxed);
                    }
                }
                draw();
            }
            KeyAction::Backspace => {
                // Go to parent directory
                go_parent();
                draw();
            }
            KeyAction::None => {}
        }
    }
}

/// Close the explorer
fn close() {
    EXPLORER_ACTIVE.store(false, Ordering::Relaxed);
    // Clear the explorer area
    for y in EXPLORER_Y..(EXPLORER_Y + EXPLORER_HEIGHT) {
        for x in EXPLORER_X..(EXPLORER_X + EXPLORER_WIDTH) {
            vga::write_at(y, x, " ", Color::White, Color::Black);
        }
    }
}

/// Refresh the file list from current path
fn refresh_file_list() {
    let path = CURRENT_PATH.lock().clone();
    let mut list = FILE_LIST.lock();
    list.clear();
    
    // Add parent directory entry if not at root
    if path != "/" {
        list.push(FileEntry {
            name: String::from(".."),
            full_path: get_parent_path(&path),
            is_dir: true,
            size: 0,
        });
    }
    
    // Handle different paths
    if path == "/" {
        // Root - show mount points
        list.push(FileEntry {
            name: String::from("ram"),
            full_path: String::from("/ram"),
            is_dir: true,
            size: 0,
        });
        list.push(FileEntry {
            name: String::from("disk"),
            full_path: String::from("/disk"),
            is_dir: true,
            size: 0,
        });
    } else if path == "/ram" || path.starts_with("/ram/") {
        // RAM filesystem
        let fs_path = if path == "/ram" {
            ""
        } else {
            &path[5..] // Skip "/ram/"
        };
        
        // Get entries from RAM filesystem
        let entries = fs::get_entries(fs_path.as_bytes());
        for (name, is_dir, size) in entries {
            let full = if path == "/ram" {
                format!("/ram/{}", name)
            } else {
                format!("{}/{}", path, name)
            };
            list.push(FileEntry {
                name,
                full_path: full,
                is_dir,
                size,
            });
        }
    } else if path == "/disk" || path.starts_with("/disk/") {
        // Disk filesystem
        let disk_path = if path == "/disk" {
            ""
        } else {
            &path[6..] // Skip "/disk/"
        };
        
        let entries = crate::diskfs::get_entries(disk_path.as_bytes());
        for (name, is_dir, size) in entries {
            let full = if path == "/disk" {
                format!("/disk/{}", name)
            } else {
                format!("{}/{}", path, name)
            };
            list.push(FileEntry {
                name,
                full_path: full,
                is_dir,
                size,
            });
        }
    }
    
    // Sort: directories first, then files (but keep ".." at top)
    let has_parent = list.first().map(|e| e.name == "..").unwrap_or(false);
    let start = if has_parent { 1 } else { 0 };
    
    if list.len() > start {
        list[start..].sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => core::cmp::Ordering::Less,
                (false, true) => core::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });
    }
}

/// Get parent path
fn get_parent_path(path: &str) -> String {
    if path == "/" || path.is_empty() {
        return String::from("/");
    }
    
    if let Some(pos) = path.rfind('/') {
        if pos == 0 {
            String::from("/")
        } else {
            String::from(&path[..pos])
        }
    } else {
        String::from("/")
    }
}

/// Draw the file explorer
fn draw() {
    let path = CURRENT_PATH.lock().clone();
    let list = FILE_LIST.lock();
    let selection = SELECTION.load(Ordering::Relaxed);
    let scroll = SCROLL_OFFSET.load(Ordering::Relaxed);
    
    // Draw border
    draw_box(EXPLORER_X, EXPLORER_Y, EXPLORER_WIDTH, EXPLORER_HEIGHT);
    
    // Title bar
    let title = format!(" File Explorer - {} ", path);
    let title = if title.len() > EXPLORER_WIDTH - 2 {
        format!(" ...{} ", &path[path.len().saturating_sub(EXPLORER_WIDTH - 10)..])
    } else {
        title
    };
    
    // Title background
    for dx in 1..EXPLORER_WIDTH-1 {
        vga::write_at(EXPLORER_Y, EXPLORER_X + dx, " ", Color::White, Color::Cyan);
    }
    vga::write_at(EXPLORER_Y, EXPLORER_X + 1, &title, Color::White, Color::Cyan);
    
    // Column headers
    let header_y = EXPLORER_Y + 1;
    let header_line = format!("{:<36} {:>8} {:>8}", "Name", "Size", "Type");
    vga::write_at(header_y, EXPLORER_X + 2, &header_line, Color::Yellow, Color::Blue);
    // Fill rest of header
    for dx in (header_line.len() + 2)..EXPLORER_WIDTH-1 {
        vga::write_at(header_y, EXPLORER_X + dx, " ", Color::Yellow, Color::Blue);
    }
    
    // File list area
    let list_start_y = EXPLORER_Y + 2;
    let list_height = EXPLORER_HEIGHT - 4;
    
    // Clear list area
    for dy in 0..list_height {
        for dx in 1..EXPLORER_WIDTH-1 {
            vga::write_at(list_start_y + dy, EXPLORER_X + dx, " ", Color::White, Color::Black);
        }
    }
    
    // Draw file list
    for (i, entry) in list.iter().skip(scroll).take(list_height).enumerate() {
        let y = list_start_y + i;
        let idx = scroll + i;
        let is_selected = idx == selection;
        
        let (fg, bg) = if is_selected {
            (Color::Black, Color::Cyan)
        } else if entry.is_dir {
            (Color::Yellow, Color::Black)
        } else {
            (Color::White, Color::Black)
        };
        
        // Clear line background if selected
        if is_selected {
            for dx in 1..EXPLORER_WIDTH-1 {
                vga::write_at(y, EXPLORER_X + dx, " ", fg, bg);
            }
        }
        
        // File icon
        let icon = if entry.name == ".." {
            ".."
        } else if entry.is_dir {
            "+"
        } else {
            " "
        };
        vga::write_at(y, EXPLORER_X + 2, icon, fg, bg);
        
        // Name (truncate if needed)
        let name_max = 34;
        let display_name = if entry.name.len() > name_max {
            format!("{}...", &entry.name[..name_max-3])
        } else {
            entry.name.clone()
        };
        vga::write_at(y, EXPLORER_X + 4, &display_name, fg, bg);
        
        // Size
        let size_str = if entry.is_dir {
            String::from("<DIR>")
        } else if entry.size >= 1024 * 1024 {
            format!("{} MB", entry.size / (1024 * 1024))
        } else if entry.size >= 1024 {
            format!("{} KB", entry.size / 1024)
        } else {
            format!("{} B", entry.size)
        };
        vga::write_at(y, EXPLORER_X + 40, &size_str, fg, bg);
        
        // Type
        let type_str = if entry.is_dir {
            "Folder"
        } else {
            get_file_type(&entry.name)
        };
        vga::write_at(y, EXPLORER_X + 50, type_str, fg, bg);
    }
    
    // Status bar
    let status_y = EXPLORER_Y + EXPLORER_HEIGHT - 2;
    let total = list.len();
    let status = format!(" {} items | Up/Down: Navigate | Enter: Select | Esc: Cancel ", total);
    
    for dx in 1..EXPLORER_WIDTH-1 {
        vga::write_at(status_y, EXPLORER_X + dx, " ", Color::Black, Color::LightGray);
    }
    let status_display = if status.len() > EXPLORER_WIDTH - 2 {
        &status[..EXPLORER_WIDTH - 2]
    } else {
        &status
    };
    vga::write_at(status_y, EXPLORER_X + 1, status_display, Color::Black, Color::LightGray);
}

/// Draw a box
fn draw_box(x: usize, y: usize, width: usize, height: usize) {
    // Top border
    vga::write_at(y, x, "+", Color::White, Color::Black);
    for dx in 1..width-1 {
        vga::write_at(y, x + dx, "-", Color::White, Color::Black);
    }
    vga::write_at(y, x + width - 1, "+", Color::White, Color::Black);
    
    // Sides
    for dy in 1..height-1 {
        vga::write_at(y + dy, x, "|", Color::White, Color::Black);
        vga::write_at(y + dy, x + width - 1, "|", Color::White, Color::Black);
    }
    
    // Bottom border
    vga::write_at(y + height - 1, x, "+", Color::White, Color::Black);
    for dx in 1..width-1 {
        vga::write_at(y + height - 1, x + dx, "-", Color::White, Color::Black);
    }
    vga::write_at(y + height - 1, x + width - 1, "+", Color::White, Color::Black);
}

/// Move selection by delta
fn move_selection(delta: isize) {
    let count = FILE_LIST.lock().len();
    if count == 0 {
        return;
    }
    
    let current = SELECTION.load(Ordering::Relaxed);
    let new_sel = if delta < 0 {
        current.saturating_sub((-delta) as usize)
    } else {
        core::cmp::min(current + delta as usize, count - 1)
    };
    
    SELECTION.store(new_sel, Ordering::Relaxed);
    
    // Adjust scroll
    let scroll = SCROLL_OFFSET.load(Ordering::Relaxed);
    if new_sel < scroll {
        SCROLL_OFFSET.store(new_sel, Ordering::Relaxed);
    } else if new_sel >= scroll + VISIBLE_ITEMS {
        SCROLL_OFFSET.store(new_sel - VISIBLE_ITEMS + 1, Ordering::Relaxed);
    }
}

/// Select current item (navigate into dir or return file path)
fn select_item() -> Option<String> {
    let list = FILE_LIST.lock();
    let selection = SELECTION.load(Ordering::Relaxed);
    
    if let Some(entry) = list.get(selection) {
        if entry.is_dir {
            // Navigate into directory
            let new_path = entry.full_path.clone();
            drop(list);
            
            {
                let mut current = CURRENT_PATH.lock();
                current.clear();
                current.push_str(&new_path);
            }
            
            refresh_file_list();
            SELECTION.store(0, Ordering::Relaxed);
            SCROLL_OFFSET.store(0, Ordering::Relaxed);
            draw();
            None
        } else {
            // Return file path
            Some(entry.full_path.clone())
        }
    } else {
        None
    }
}

/// Go to parent directory
fn go_parent() {
    let path = CURRENT_PATH.lock().clone();
    if path == "/" {
        return;
    }
    
    let parent = get_parent_path(&path);
    
    {
        let mut current = CURRENT_PATH.lock();
        current.clear();
        current.push_str(&parent);
    }
    
    refresh_file_list();
    SELECTION.store(0, Ordering::Relaxed);
    SCROLL_OFFSET.store(0, Ordering::Relaxed);
}

/// Get file type based on extension
fn get_file_type(name: &str) -> &'static str {
    if let Some(pos) = name.rfind('.') {
        match &name[pos+1..] {
            "txt" => "Text",
            "qasm" => "QASM",
            "rs" => "Rust",
            "py" => "Python",
            "json" => "JSON",
            "log" => "Log",
            "cfg" | "conf" => "Config",
            "md" => "Markdown",
            _ => "File",
        }
    } else {
        "File"
    }
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
                        DecodedKey::RawKey(KeyCode::ArrowUp) => KeyAction::Up,
                        DecodedKey::RawKey(KeyCode::ArrowDown) => KeyAction::Down,
                        DecodedKey::RawKey(KeyCode::PageUp) => KeyAction::PageUp,
                        DecodedKey::RawKey(KeyCode::PageDown) => KeyAction::PageDown,
                        DecodedKey::RawKey(KeyCode::Home) => KeyAction::Home,
                        DecodedKey::RawKey(KeyCode::End) => KeyAction::End,
                        DecodedKey::RawKey(KeyCode::Escape) => KeyAction::Escape,
                        DecodedKey::RawKey(KeyCode::Backspace) => KeyAction::Backspace,
                        _ => KeyAction::None,
                    };
                }
            }
        }
        
        // Yield CPU
        crate::arch::hlt();
    }
}
