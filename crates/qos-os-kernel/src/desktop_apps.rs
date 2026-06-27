//! GUI Applications for QOS Desktop
//!
//! Collection of desktop applications: calculator, notepad, file browser, etc.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use crate::desktop;
use crate::vfs;

/// Calculator Application
pub struct Calculator {
    window_id: u32,
    display: String,
    current_value: f64,
    operation: Option<char>,
    new_number: bool,
}

impl Calculator {
    pub fn new() -> Self {
        let window_id = desktop::create_window("Calculator", 20, 5, 35, 14);
        let mut calc = Self {
            window_id,
            display: String::from("0"),
            current_value: 0.0,
            operation: None,
            new_number: true,
        };
        calc.render();
        calc
    }

    fn render(&self) {
        desktop::window_clear(self.window_id);
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, &format!("  Display: {}", self.display));
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  ┌───┬───┬───┬───┐");
        desktop::window_add_line(self.window_id, "  │ 7 │ 8 │ 9 │ ÷ │");
        desktop::window_add_line(self.window_id, "  ├───┼───┼───┼───┤");
        desktop::window_add_line(self.window_id, "  │ 4 │ 5 │ 6 │ × │");
        desktop::window_add_line(self.window_id, "  ├───┼───┼───┼───┤");
        desktop::window_add_line(self.window_id, "  │ 1 │ 2 │ 3 │ - │");
        desktop::window_add_line(self.window_id, "  ├───┼───┼───┼───┤");
        desktop::window_add_line(self.window_id, "  │ 0 │ . │ = │ + │");
        desktop::window_add_line(self.window_id, "  └───┴───┴───┴───┘");
    }

    pub fn window_id(&self) -> u32 {
        self.window_id
    }
}

/// File Browser Application
pub struct FileBrowser {
    window_id: u32,
    current_path: String,
    files: Vec<String>,
}

impl FileBrowser {
    pub fn new() -> Self {
        let window_id = desktop::create_window("File Explorer", 5, 2, 70, 20);
        let mut browser = Self {
            window_id,
            current_path: String::from("/ram"),
            files: Vec::new(),
        };
        browser.refresh();
        browser
    }

    fn refresh(&mut self) {
        desktop::window_clear(self.window_id);
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, &format!("  Location: {}", self.current_path));
        desktop::window_add_line(self.window_id, "  ═══════════════════════════════════════════════════════════════");
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  Name                    Size        Type");
        desktop::window_add_line(self.window_id, "  ────────────────────────────────────────────────────────────");
        
        // List files from VFS - simplified for now
        desktop::window_add_line(self.window_id, "  ..                      <DIR>       DIR ");
        desktop::window_add_line(self.window_id, "  example.txt             1024 B      FILE");
        desktop::window_add_line(self.window_id, "  data                    <DIR>       DIR ");
        desktop::window_add_line(self.window_id, "  README.md               2048 B      FILE");
        
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  ═══════════════════════════════════════════════════════════════");
        desktop::window_add_line(self.window_id, "  [Open] [Copy] [Delete] [New Folder] [Refresh]");
    }

    pub fn window_id(&self) -> u32 {
        self.window_id
    }
}

/// Notepad Application
pub struct Notepad {
    window_id: u32,
    filename: Option<String>,
    content: Vec<String>,
}

impl Notepad {
    pub fn new() -> Self {
        let window_id = desktop::create_window("Notepad - Untitled", 10, 4, 60, 16);
        let mut notepad = Self {
            window_id,
            filename: None,
            content: Vec::new(),
        };
        notepad.render();
        notepad
    }

    pub fn open(filename: &str) -> Self {
        let window_id = desktop::create_window(&format!("Notepad - {}", filename), 10, 4, 60, 16);
        let mut notepad = Self {
            window_id,
            filename: Some(filename.to_string()),
            content: Vec::new(),
        };
        
        // Try to load file
        if let Ok(data) = vfs::read(filename.as_bytes()) {
            let text = String::from_utf8_lossy(&data);
            notepad.content = text.lines().map(|l| l.to_string()).collect();
        }
        
        notepad.render();
        notepad
    }

    fn render(&self) {
        desktop::window_clear(self.window_id);
        desktop::window_add_line(self.window_id, "  File  Edit  Format  View  Help");
        desktop::window_add_line(self.window_id, "  ──────────────────────────────────────────────────────────");
        
        for line in &self.content {
            desktop::window_add_line(self.window_id, &format!("  {}", line));
        }
        
        // Padding
        for _ in self.content.len()..10 {
            desktop::window_add_line(self.window_id, "");
        }
        
        desktop::window_add_line(self.window_id, "  ──────────────────────────────────────────────────────────");
        let status = format!("  Line: {}  |  Chars: {}", 
                            self.content.len(), 
                            self.content.iter().map(|l| l.len()).sum::<usize>());
        desktop::window_add_line(self.window_id, &status);
    }

    pub fn window_id(&self) -> u32 {
        self.window_id
    }
}

/// Task Manager Application
pub struct TaskManager {
    window_id: u32,
}

impl TaskManager {
    pub fn new() -> Self {
        let window_id = desktop::create_window("Task Manager", 8, 3, 64, 18);
        let mut tm = Self { window_id };
        tm.refresh();
        tm
    }

    fn refresh(&self) {
        desktop::window_clear(self.window_id);
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  QOS Task Manager");
        desktop::window_add_line(self.window_id, "  ═══════════════════════════════════════════════════════════");
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  PID   Status      Name                    Exit Code");
        desktop::window_add_line(self.window_id, "  ────────────────────────────────────────────────────────");
        
        // Get process list
        let procs = crate::tasking::list_processes();
        if procs.is_empty() {
            desktop::window_add_line(self.window_id, "  (No running processes)");
        } else {
            for (pid, state, code) in procs {
                let line = format!("  {:<5} {:?}      Process-{}           {}", 
                                  pid, state, pid, code);
                desktop::window_add_line(self.window_id, &line);
            }
        }
        
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  ═══════════════════════════════════════════════════════════");
        desktop::window_add_line(self.window_id, "  System Information:");
        desktop::window_add_line(self.window_id, &format!("    Uptime: {}", crate::timer::uptime_string()));
        desktop::window_add_line(self.window_id, &format!("    Time: {}", crate::rtc::time_string()));
        desktop::window_add_line(self.window_id, "    Memory: Available");
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  [End Task] [Refresh] [Close]");
    }

    pub fn window_id(&self) -> u32 {
        self.window_id
    }
}

/// System Info Application
pub struct SystemInfo {
    window_id: u32,
}

impl SystemInfo {
    pub fn new() -> Self {
        let window_id = desktop::create_window("System Information", 15, 6, 50, 15);
        let mut info = Self { window_id };
        info.render();
        info
    }

    fn render(&self) {
        desktop::window_clear(self.window_id);
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  ╔══════════════════════════════════════════╗");
        desktop::window_add_line(self.window_id, "  ║     QOS - Quantum Operating System      ║");
        desktop::window_add_line(self.window_id, "  ╚══════════════════════════════════════════╝");
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "    Version: 1.0.0");
        desktop::window_add_line(self.window_id, "    Architecture: x86_64");
        desktop::window_add_line(self.window_id, "    Kernel: QOS Kernel");
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, &format!("    System Time: {}", crate::rtc::time_string()));
        desktop::window_add_line(self.window_id, &format!("    Uptime: {}", crate::timer::uptime_string()));
        desktop::window_add_line(self.window_id, "    Memory: Available");
        desktop::window_add_line(self.window_id, "");
        desktop::window_add_line(self.window_id, "  Features:");
        desktop::window_add_line(self.window_id, "    • Desktop GUI Environment");
        desktop::window_add_line(self.window_id, "    • Quantum Job Scheduler");
        desktop::window_add_line(self.window_id, "    • Network Stack (E1000)");
        desktop::window_add_line(self.window_id, "    • Virtual File System");
    }

    pub fn window_id(&self) -> u32 {
        self.window_id
    }
}

// Application launcher functions

/// Launch Calculator app
pub fn launch_calculator() -> u32 {
    let calc = Calculator::new();
    desktop::render();
    crate::serial_println!("[APP] Calculator launched");
    calc.window_id()
}

/// Launch File Browser app
pub fn launch_file_browser() -> u32 {
    let browser = FileBrowser::new();
    desktop::render();
    crate::serial_println!("[APP] File Browser launched");
    browser.window_id()
}

/// Launch Notepad app
pub fn launch_notepad() -> u32 {
    let notepad = Notepad::new();
    desktop::render();
    crate::serial_println!("[APP] Notepad launched");
    notepad.window_id()
}

/// Launch Task Manager app
pub fn launch_task_manager() -> u32 {
    let tm = TaskManager::new();
    desktop::render();
    crate::serial_println!("[APP] Task Manager launched");
    tm.window_id()
}

/// Launch System Info app
pub fn launch_system_info() -> u32 {
    let info = SystemInfo::new();
    desktop::render();
    crate::serial_println!("[APP] System Info launched");
    info.window_id()
}
