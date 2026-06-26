use pc_keyboard::{
    layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1,
};

use spin::Mutex;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{ata, diskfs, interrupts, keyboard, mouse, syscall, vfs, vga};

const LINE_MAX: usize = 80;
const CWD_MAX: usize = 32;
const HISTORY_MAX: usize = 16;
const EDITOR_MAX_LINES: usize = 64;
const EDITOR_LINE_MAX: usize = 80;
const MAX_ENV_VARS: usize = 32;
const MAX_ALIASES: usize = 16;

// ==================== ENVIRONMENT VARIABLES ====================

lazy_static::lazy_static! {
    /// Global environment variables storage
    static ref ENV_VARS: Mutex<BTreeMap<String, String>> = {
        let mut map = BTreeMap::new();
        // Default environment variables
        map.insert(String::from("PATH"), String::from("/ram:/disk"));
        map.insert(String::from("HOME"), String::from("/ram"));
        map.insert(String::from("SHELL"), String::from("qsh"));
        map.insert(String::from("USER"), String::from("quantum"));
        map.insert(String::from("PS1"), String::from("QaOS:$PWD $ "));
        Mutex::new(map)
    };
    
    /// Global aliases storage
    static ref ALIASES: Mutex<BTreeMap<String, String>> = {
        let mut map = BTreeMap::new();
        // Default aliases
        map.insert(String::from("ll"), String::from("ls"));
        map.insert(String::from("la"), String::from("ls"));
        map.insert(String::from("c"), String::from("clear"));
        map.insert(String::from("q"), String::from("shutdown"));
        Mutex::new(map)
    };
}

/// Get environment variable value
pub fn env_get(name: &str) -> Option<String> {
    ENV_VARS.lock().get(name).cloned()
}

/// Set environment variable
pub fn env_set(name: &str, value: &str) {
    ENV_VARS.lock().insert(String::from(name), String::from(value));
}

/// Remove environment variable
pub fn env_unset(name: &str) -> bool {
    ENV_VARS.lock().remove(name).is_some()
}

/// List all environment variables
pub fn env_list() -> alloc::vec::Vec<(String, String)> {
    ENV_VARS.lock().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Get alias value
pub fn alias_get(name: &str) -> Option<String> {
    ALIASES.lock().get(name).cloned()
}

/// Set alias
pub fn alias_set(name: &str, value: &str) {
    ALIASES.lock().insert(String::from(name), String::from(value));
}

/// Remove alias
pub fn alias_unset(name: &str) -> bool {
    ALIASES.lock().remove(name).is_some()
}

/// List all aliases
pub fn alias_list() -> alloc::vec::Vec<(String, String)> {
    ALIASES.lock().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

// ==================== END ENVIRONMENT VARIABLES ====================

/// List of all available shell commands for tab completion
static COMMANDS: &[&[u8]] = &[
    b"help", b"kbd", b"clear", b"gfx", b"gdesk", b"evtest", b"threadtest", b"proctest", b"faulttest", b"exittest", b"crash", b"ticks", b"ps", b"pwd", b"cd", b"ls", b"cat", b"rm",
    b"mkdir", b"mkbell", b"edit", b"touch", b"submit", b"write",
    b"disk-id", b"disk-read", b"mkfs", b"dls", b"dcat", b"drm", b"dput", b"dget", b"dsubmit",
    b"vls", b"vcat", b"vrm", b"vcp", b"vsubmit",
    b"userdemo", b"udemo", b"udemo-bg", b"exec", b"procs", b"spawn", b"fg", b"bg", b"killp", b"waitp",
    b"jobs", b"submit-bell", b"submit-ir-bell", b"status", b"result", b"viz", b"cancel",
    b"time", b"uptime", b"pci", b"net", b"shutdown", b"reboot", b"powerinfo", b"ui",
    b"qsubmit", b"qsim", b"qbackend", b"echo", b"grep", b"wc", b"head", b"tail", b"sort", b"uniq",
    b"env", b"export", b"unset", b"alias", b"unalias", b"source", b"run",
    b"df", b"du", b"stat", b"mv", b"cp", b"chmod", b"ll",
    b"desktop", b"window", b"gui",
    b"calc", b"notepad", b"explorer", b"taskmgr", b"sysinfo",
    b"ifconfig", b"ping", b"arp", b"netstat", b"dhcp",
    #[cfg(feature = "fat")]
    b"fatls", 
    #[cfg(feature = "fat")]
    b"fatcat", 
    #[cfg(feature = "fat")]
    b"fatwrite", 
    #[cfg(feature = "fat")]
    b"fatrm",
];

static NEXT_CWD: Mutex<Option<([u8; CWD_MAX], usize)>> = Mutex::new(None);

pub fn set_next_cwd(cwd: &[u8]) {
    let mut buf = [0u8; CWD_MAX];
    let n = core::cmp::min(cwd.len(), CWD_MAX);
    buf[..n].copy_from_slice(&cwd[..n]);
    *NEXT_CWD.lock() = Some((buf, n));
}

fn take_next_cwd() -> Option<([u8; CWD_MAX], usize)> {
    NEXT_CWD.lock().take()
}

pub struct ShellTask {
    kb: Keyboard<layouts::Us104Key, ScancodeSet1>,
    kbd_mode: KeyboardMode,
    line: [u8; LINE_MAX],
    len: usize,
    banner_printed: bool,
    prompt_shown: bool,
    cwd: [u8; CWD_MAX],
    cwd_len: usize,

    // Simple command history (newest-first browsing).
    history: [[u8; LINE_MAX]; HISTORY_MAX],
    history_len: [usize; HISTORY_MAX],
    history_count: usize,
    history_head: usize, // next insert slot
    history_pos: Option<usize>, // N from newest
    saved_line: [u8; LINE_MAX],
    saved_len: usize,
    
    // Editor mode
    editor_mode: bool,
    editor_path: [u8; 96],
    editor_path_len: usize,
    editor_lines: [[u8; EDITOR_LINE_MAX]; EDITOR_MAX_LINES],
    editor_line_lens: [usize; EDITOR_MAX_LINES],
    editor_line_count: usize,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum KeyboardMode {
    Us,
    Tr,
}

impl ShellTask {
    pub fn new() -> Self {
        let kb = Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::Ignore);
        let (cwd, cwd_len) = if let Some((buf, n)) = take_next_cwd() {
            (buf, n)
        } else {
            let mut buf = [0u8; CWD_MAX];
            let init = b"/ram";
            buf[..init.len()].copy_from_slice(init);
            (buf, init.len())
        };
        Self {
            kb,
            kbd_mode: KeyboardMode::Us,
            line: [0; LINE_MAX],
            len: 0,
            banner_printed: false,
            prompt_shown: false,
            cwd,
            cwd_len,

            history: [[0u8; LINE_MAX]; HISTORY_MAX],
            history_len: [0usize; HISTORY_MAX],
            history_count: 0,
            history_head: 0,
            history_pos: None,
            saved_line: [0u8; LINE_MAX],
            saved_len: 0,
            
            editor_mode: false,
            editor_path: [0u8; 96],
            editor_path_len: 0,
            editor_lines: [[0u8; EDITOR_LINE_MAX]; EDITOR_MAX_LINES],
            editor_line_lens: [0usize; EDITOR_MAX_LINES],
            editor_line_count: 0,
        }
    }

    fn kbd_name(&self) -> &'static str {
        match self.kbd_mode {
            KeyboardMode::Us => "us",
            KeyboardMode::Tr => "tr",
        }
    }

    fn set_kbd_mode(&mut self, mode: KeyboardMode) {
        self.kbd_mode = mode;
        crate::println!("keyboard: {}", self.kbd_name());
    }

    fn tr_ascii_map(c: char) -> Option<char> {
        // Türkçe Q klavye mapping - sadece özel Türkçe karakterler için
        // Köşeli parantez vs. olduğu gibi kalır (AltGr+8/9 ile yapılır)
        // Bu mapping US layout üzerinden TR karakterleri yazmak için:
        Some(match c {
            // Türkçe özel karakterler için kısayollar:
            // Alt+g -> ğ, Alt+u -> ü, Alt+s -> ş, Alt+i -> ı, Alt+o -> ö, Alt+c -> ç
            // Ama AltGr/Ctrl mapping karmaşık olduğundan,
            // Şimdilik sadece ; -> ş ve ' -> ı mapping yapalım (sık kullanılan)
            ';' => 's',  // noktalı virgül -> ş (ASCII s olarak)
            '\'' => 'i', // apostrof -> ı (ASCII i olarak)
            // Diğer karakterler olduğu gibi kalsın
            _ => return None,
        })
    }

    fn is_blank(line: &[u8]) -> bool {
        line.iter().all(|b| *b == b' ' || *b == b'\t')
    }

    fn history_push(&mut self, line: &[u8]) {
        if line.is_empty() || Self::is_blank(line) {
            return;
        }
        let n = core::cmp::min(line.len(), LINE_MAX);
        self.history[self.history_head][..n].copy_from_slice(&line[..n]);
        self.history_len[self.history_head] = n;
        self.history_head = (self.history_head + 1) % HISTORY_MAX;
        self.history_count = core::cmp::min(self.history_count + 1, HISTORY_MAX);
    }

    fn history_get_index(&self, n_from_newest: usize) -> Option<(usize, usize)> {
        if n_from_newest >= self.history_count {
            return None;
        }
        let idx = (self.history_head + HISTORY_MAX - 1 - n_from_newest) % HISTORY_MAX;
        Some((idx, self.history_len[idx]))
    }

    fn history_load(&mut self, n_from_newest: usize) {
        let Some((idx, len)) = self.history_get_index(n_from_newest) else {
            return;
        };
        let n = core::cmp::min(len, LINE_MAX);

        // Copy to a local array so we don't hold an immutable borrow of `self` while mutating.
        let src = self.history[idx];
        self.line[..n].copy_from_slice(&src[..n]);
        self.len = n;
    }

    fn history_up(&mut self) {
        if self.history_count == 0 {
            return;
        }

        match self.history_pos {
            None => {
                // Enter history mode: save current edit buffer.
                self.saved_line[..self.len].copy_from_slice(&self.line[..self.len]);
                self.saved_len = self.len;
                self.history_pos = Some(0);
                self.history_load(0);
            }
            Some(pos) => {
                let next = core::cmp::min(pos + 1, self.history_count - 1);
                self.history_pos = Some(next);
                self.history_load(next);
            }
        }
    }

    fn history_down(&mut self) {
        let Some(pos) = self.history_pos else {
            return;
        };

        if pos == 0 {
            // Exit history mode: restore saved line.
            self.line[..self.saved_len].copy_from_slice(&self.saved_line[..self.saved_len]);
            self.len = self.saved_len;
            self.history_pos = None;
            return;
        }

        let next = pos - 1;
        self.history_pos = Some(next);
        self.history_load(next);
    }

    fn prompt(&mut self) {
        if !self.banner_printed {
            self.print_banner();
            self.banner_printed = true;
        }
        self.redraw_input_line();
    }

    fn print_banner(&self) {
        crate::vga::clear_screen();
        crate::println!("");
        crate::println!("    ___           ___           ___           ___     ");
        crate::println!("   /\\  \\         /\\  \\         /\\  \\         /\\  \\    ");
        crate::println!("  /::\\  \\       /::\\  \\       /::\\  \\       /::\\  \\   ");
        crate::println!(" /:/\\:\\  \\     /:/\\:\\  \\     /:/\\:\\  \\     /:/\\ \\  \\  ");
        crate::println!("/::\\~\\:\\  \\   /::\\~\\:\\  \\   /:/  \\:\\  \\   _\\:\\~\\ \\  \\ ");
        crate::println!("/:/\\:\\ \\:\\__\\ /:/\\:\\ \\:\\__\\ /:/__/ \\:\\__\\ /\\ \\:\\ \\ \\__\\");
        crate::println!("\\/__\\:\\/:/  / \\/__\\:\\/:/  / \\:\\  \\ /:/  / \\:\\ \\:\\ \\/__/");
        crate::println!("     \\::/  /       \\::/  /   \\:\\  /:/  /   \\:\\ \\:\\__\\  ");
        crate::println!("      \\/__/        /:/  /     \\:\\/:/  /     \\:\\/:/  /  ");
        crate::println!("                  /:/  /       \\::/  /       \\::/  /   ");
        crate::println!("                  \\/__/         \\/__/         \\/__/    ");
        crate::println!("");
        crate::println!("              Quantum Operating System v0.1.0");
        crate::println!("");
        crate::println!("  +----------------------------------------------------------+");
        crate::println!("  |  Mouse Scroll: Navigate history    Arrows: Edit line    |");
        crate::println!("  |  Type 'help' for available commands                     |");
        crate::println!("  +----------------------------------------------------------+");
        crate::println!("");
    }

    fn redraw_input_line(&mut self) {
        // Clean prompt at bottom of screen
        let row = vga::bottom_row();
        vga::clear_row(row, vga::Color::LightGray, vga::Color::Black);

        let cwd = core::str::from_utf8(self.cwd_bytes()).unwrap_or("?");
        let prompt = alloc::format!("QaOS:{} $ ", cwd);

        // Render prompt + current line content
        let mut s = alloc::string::String::new();
        s.push_str(&prompt);

        let line = core::str::from_utf8(&self.line[..self.len]).unwrap_or("<non-utf8>");
        s.push_str(line);
        s.push('_'); // Cursor

        // Keep it to one row
        if s.len() > 80 {
            s.truncate(80);
        }
        
        // Green prompt, white text
        let prompt_len = prompt.len();
        vga::write_at(row, 0, &prompt, vga::Color::LightGreen, vga::Color::Black);
        vga::write_at(row, prompt_len, &s[prompt_len..], vga::Color::White, vga::Color::Black);

        self.prompt_shown = true;
    }

    fn cwd_bytes(&self) -> &[u8] {
        &self.cwd[..self.cwd_len]
    }

    fn set_cwd(&mut self, path: &[u8]) {
        let n = core::cmp::min(path.len(), CWD_MAX);
        self.cwd[..n].copy_from_slice(&path[..n]);
        self.cwd_len = n;
    }

    fn trim_trailing_slash(path: &[u8]) -> &[u8] {
        if path.len() > 1 && path[path.len() - 1] == b'/' {
            &path[..path.len() - 1]
        } else {
            path
        }
    }

    /// Format size with units (B, KB, MB, GB)
    fn format_size(bytes: usize) -> alloc::string::String {
        if bytes < 1024 {
            alloc::format!("{}B", bytes)
        } else if bytes < 1024 * 1024 {
            alloc::format!("{}KB", bytes / 1024)
        } else if bytes < 1024 * 1024 * 1024 {
            alloc::format!("{}MB", bytes / (1024 * 1024))
        } else {
            alloc::format!("{}GB", bytes / (1024 * 1024 * 1024))
        }
    }
    
    /// Format Unix timestamp to human-readable string
    fn format_timestamp(ts: u64) -> alloc::string::String {
        if ts == 0 {
            return alloc::string::String::from("-");
        }
        // Simple conversion - approximate
        let secs_per_min = 60u64;
        let secs_per_hour = 3600u64;
        let secs_per_day = 86400u64;
        let secs_per_year = 31536000u64; // 365 days
        
        let year = 1970 + (ts / secs_per_year) as u32;
        let remaining = ts % secs_per_year;
        let day_of_year = (remaining / secs_per_day) as u32;
        let remaining = remaining % secs_per_day;
        let hour = (remaining / secs_per_hour) as u32;
        let remaining = remaining % secs_per_hour;
        let minute = (remaining / secs_per_min) as u32;
        
        // Approximate month (30 days each)
        let month = (day_of_year / 30) + 1;
        let day = (day_of_year % 30) + 1;
        
        alloc::format!("{:04}-{:02}-{:02} {:02}:{:02}", year, month, day, hour, minute)
    }
    
    /// Format permissions as rwxrwxrwx
    fn format_perms(mode: u16) -> alloc::string::String {
        let mut s = alloc::string::String::with_capacity(9);
        
        // Owner
        s.push(if mode & 0o400 != 0 { 'r' } else { '-' });
        s.push(if mode & 0o200 != 0 { 'w' } else { '-' });
        s.push(if mode & 0o100 != 0 { 'x' } else { '-' });
        
        // Group
        s.push(if mode & 0o040 != 0 { 'r' } else { '-' });
        s.push(if mode & 0o020 != 0 { 'w' } else { '-' });
        s.push(if mode & 0o010 != 0 { 'x' } else { '-' });
        
        // Other
        s.push(if mode & 0o004 != 0 { 'r' } else { '-' });
        s.push(if mode & 0o002 != 0 { 'w' } else { '-' });
        s.push(if mode & 0o001 != 0 { 'x' } else { '-' });
        
        s
    }

    fn resolve_path(&self, input: &[u8], out: &mut [u8; 96]) -> Option<usize> {
        let input = Self::trim_trailing_slash(input);
        if input.is_empty() {
            return None;
        }

        if input == b"." {
            let cwd = self.cwd_bytes();
            if cwd.len() > out.len() {
                return None;
            }
            out[..cwd.len()].copy_from_slice(cwd);
            return Some(cwd.len());
        }
        if input == b".." {
            out[0] = b'/';
            return Some(1);
        }

        if input[0] == b'/' {
            if input.len() > out.len() {
                return None;
            }
            out[..input.len()].copy_from_slice(input);
            return Some(input.len());
        }

        let cwd = self.cwd_bytes();
        let need_slash = !cwd.ends_with(b"/");
        let total = cwd.len() + (need_slash as usize) + input.len();
        if total > out.len() {
            return None;
        }
        out[..cwd.len()].copy_from_slice(cwd);
        let mut off = cwd.len();
        if need_slash {
            out[off] = b'/';
            off += 1;
        }
        out[off..off + input.len()].copy_from_slice(input);
        Some(total)
    }

    fn push_byte(&mut self, b: u8) {
        if self.len >= LINE_MAX {
            return;
        }
        self.line[self.len] = b;
        self.len += 1;
    }

    fn backspace(&mut self) {
        if self.len == 0 {
            return;
        }
        self.len -= 1;
    }

    fn clear_line(&mut self) {
        self.len = 0;
        self.history_pos = None;
        self.prompt_shown = false;
    }

    /// Handle menu item selection
    fn handle_menu_action(&mut self, action: &str) {
        match action {
            // File menu
            "New" => {
                crate::println!("Creating new file...");
                self.start_editor(b"/ram/untitled.txt");
            }
            "Open..." => {
                // Open file explorer
                if let Some(path) = crate::explorer::open() {
                    crate::println!("Selected: {}", core::str::from_utf8(&path).unwrap_or("?"));
                    // Open in editor if it's a file
                    if !path.is_empty() && path[path.len() - 1] != b'/' {
                        self.start_editor(&path);
                    }
                }
            }
            "Save" => {
                if self.editor_mode {
                    crate::println!("Use :w in editor to save");
                }
            }
            "Exit" => {
                if crate::dialog::confirm("Exit QaOS", "Are you sure you want to shutdown?") {
                    crate::acpi::shutdown();
                }
            }
            
            // Edit menu
            "Cut" | "Copy" | "Paste" | "Select All" => {
                crate::println!("Clipboard not implemented yet");
            }
            
            // View menu
            "UI Panel" => {
                let enabled = crate::ui::enabled();
                crate::ui::set_enabled(!enabled);
            }
            "Full Screen" => {
                crate::println!("Already in full screen mode");
            }
            "Refresh" => {
                vga::clear_screen();
                self.banner_printed = false;
            }
            
            // Quantum menu
            "New Circuit" => {
                self.start_editor(b"/ram/circuit.qasm");
                let template = b"OPENQASM 2.0;\ninclude \"qelib1.inc\";\n\nqreg q[2];\ncreg c[2];\n\nh q[0];\ncx q[0],q[1];\nmeasure q -> c;\n";
                // Pre-fill with template
                for (i, line) in template.split(|&b| b == b'\n').enumerate() {
                    if i < EDITOR_MAX_LINES && !line.is_empty() {
                        let n = core::cmp::min(line.len(), EDITOR_LINE_MAX);
                        self.editor_lines[i][..n].copy_from_slice(&line[..n]);
                        self.editor_line_lens[i] = n;
                        self.editor_line_count = i + 1;
                    }
                }
            }
            "Submit Job" => {
                crate::println!("Use: qsubmit <file.qasm>");
            }
            "Job Status" => {
                self.run_command(b"qjobs");
            }
            "Simulator Settings" => {
                let max_qubits = crate::syscall::MAX_QUBITS;
                let avail = crate::syscall::available_qubits();
                crate::println!("╔════════════════════════════════════════╗");
                crate::println!("║   Quantum Simulator Settings          ║");
                crate::println!("╠════════════════════════════════════════╣");
                crate::println!("║ Max Qubits:        {}                ║", max_qubits);
                crate::println!("║ Available:         {}                ║", avail);
                crate::println!("║ State Vector Size: {} bytes          ║", (1 << max_qubits) * 16);
                crate::println!("║ Memory Model:      Sparse             ║");
                crate::println!("╚════════════════════════════════════════╝");
            }
            
            // Help menu
            "Commands" => {
                self.run_command(b"help");
            }
            "About QaOS" => {
                crate::dialog::message_box(
                    "About QaOS",
                    "Quantum Operating System v0.1.0 - Press F10 for menu"
                );
            }
            
            _ => {
                crate::println!("Menu: {}", action);
            }
        }
    }

    /// Handle mouse click events
    fn handle_mouse_click(&mut self, click: mouse::MouseClick) {
        use mouse::MouseButton;
        
        match click.button {
            MouseButton::Left => {
                // Row 0 = menu bar
                if click.y == 0 {
                    crate::menu::handle_click(click.x);
                } else if crate::menu::is_active() {
                    // Click inside dropdown area - check if valid selection
                    let menu_idx = crate::menu::ACTIVE_MENU_INDEX.load(core::sync::atomic::Ordering::Relaxed);
                    // For now, just select on click
                    if let Some(selected) = crate::menu::menu_select() {
                        self.handle_menu_action(&selected);
                    }
                    self.redraw_input_line();
                }
            }
            MouseButton::Right => {
                // Could show context menu in future
                if crate::menu::is_active() {
                    crate::menu::close_menu();
                    self.redraw_input_line();
                }
            }
            MouseButton::Middle => {
                // Toggle UI panel
                let enabled = crate::ui::enabled();
                crate::ui::set_enabled(!enabled);
            }
        }
    }

    fn eq(cmd: &[u8], s: &[u8]) -> bool {
        cmd == s
    }

    fn parse_u64(mut s: &[u8]) -> Option<u64> {
        while let Some((&b, rest)) = s.split_first() {
            if b == b' ' || b == b'\t' {
                s = rest;
            } else {
                break;
            }
        }
        if s.is_empty() {
            return None;
        }
        let mut v: u64 = 0;
        for &b in s {
            if !(b'0'..=b'9').contains(&b) {
                break;
            }
            v = v.saturating_mul(10).saturating_add((b - b'0') as u64);
        }
        Some(v)
    }

    fn parse_word(mut s: &[u8]) -> Option<(&[u8], &[u8])> {
        while let Some((&b, rest)) = s.split_first() {
            if b == b' ' || b == b'\t' {
                s = rest;
            } else {
                break;
            }
        }
        if s.is_empty() {
            return None;
        }
        let mut end = 0;
        while end < s.len() {
            let b = s[end];
            if b == b' ' || b == b'\t' {
                break;
            }
            end += 1;
        }
        Some((&s[..end], &s[end..]))
    }

    /// Trim leading and trailing whitespace from a byte slice
    fn trim_whitespace(mut s: &[u8]) -> &[u8] {
        // Trim leading
        while let Some((&b, rest)) = s.split_first() {
            if b == b' ' || b == b'\t' {
                s = rest;
            } else {
                break;
            }
        }
        // Trim trailing
        while let Some((&b, rest)) = s.split_last() {
            if b == b' ' || b == b'\t' {
                s = rest;
            } else {
                break;
            }
        }
        s
    }

    // ==================== TAB COMPLETION ====================
    
    /// Try to complete the current input with Tab
    fn tab_complete(&mut self) {
        // Copy input to avoid borrow issues
        let mut input_copy = [0u8; LINE_MAX];
        let input_len = self.len;
        input_copy[..input_len].copy_from_slice(&self.line[..input_len]);
        
        // Find if we're completing a command or a file argument
        let has_space = input_copy[..input_len].iter().any(|&b| b == b' ');
        
        if !has_space {
            // Complete command name
            self.complete_command(&input_copy[..input_len]);
        } else {
            // Complete file/path argument - copy cwd too
            let mut cwd_copy = [0u8; CWD_MAX];
            let cwd_len = self.cwd_len;
            cwd_copy[..cwd_len].copy_from_slice(&self.cwd[..cwd_len]);
            self.complete_path(&input_copy[..input_len], &cwd_copy[..cwd_len]);
        }
    }
    
    /// Complete command name
    fn complete_command(&mut self, prefix: &[u8]) {
        if prefix.is_empty() {
            return;
        }
        
        // Find matching commands
        let mut matches: [Option<&[u8]>; 16] = [None; 16];
        let mut match_count = 0;
        
        for &cmd in COMMANDS.iter() {
            if cmd.len() >= prefix.len() && &cmd[..prefix.len()] == prefix {
                if match_count < 16 {
                    matches[match_count] = Some(cmd);
                    match_count += 1;
                }
            }
        }
        
        if match_count == 0 {
            // No matches
            return;
        } else if match_count == 1 {
            // Single match - complete it
            if let Some(cmd) = matches[0] {
                self.len = 0;
                for &b in cmd.iter() {
                    if self.len < LINE_MAX {
                        self.line[self.len] = b;
                        self.len += 1;
                    }
                }
                // Add space after command
                if self.len < LINE_MAX {
                    self.line[self.len] = b' ';
                    self.len += 1;
                }
            }
        } else {
            // Multiple matches - show them and complete common prefix
            crate::println!();
            for i in 0..match_count {
                if let Some(cmd) = matches[i] {
                    let s = core::str::from_utf8(cmd).unwrap_or("?");
                    crate::print!("{}  ", s);
                }
            }
            crate::println!();
            
            // Find common prefix
            let common_len = Self::find_common_prefix(&matches[..match_count]);
            if common_len > prefix.len() {
                if let Some(first) = matches[0] {
                    self.len = 0;
                    for i in 0..common_len {
                        if self.len < LINE_MAX {
                            self.line[self.len] = first[i];
                            self.len += 1;
                        }
                    }
                }
            }
        }
    }
    
    /// Complete file/path argument
    fn complete_path(&mut self, input: &[u8], cwd: &[u8]) {
        // Find the last argument (after last space)
        let mut last_space = 0;
        for (i, &b) in input.iter().enumerate() {
            if b == b' ' {
                last_space = i + 1;
            }
        }
        
        let partial_path = &input[last_space..];
        if partial_path.is_empty() {
            return;
        }
        
        // Copy partial_path to owned
        let mut partial_copy = [0u8; LINE_MAX];
        partial_copy[..partial_path.len()].copy_from_slice(partial_path);
        let partial_len = partial_path.len();
        
        // Determine directory and prefix to match
        let slash_pos = partial_path.iter().rposition(|&b| b == b'/');
        
        let (dir_path, file_prefix): (&[u8], &[u8]) = if let Some(pos) = slash_pos {
            (&partial_path[..=pos], &partial_path[pos+1..])
        } else {
            // No slash - use current directory
            (cwd, partial_path)
        };
        
        // Get directory entries
        let dir_str = core::str::from_utf8(dir_path).unwrap_or("");
        let entries: alloc::vec::Vec<(alloc::string::String, bool, usize)> = if dir_str.is_empty() || dir_str == "/" {
            // Root - list mount points
            alloc::vec![
                (alloc::string::String::from("ram"), true, 0usize),
                (alloc::string::String::from("disk"), true, 0usize),
            ]
        } else if dir_str.starts_with("/ram") {
            let fs_path = if dir_str == "/ram" || dir_str == "/ram/" {
                ""
            } else {
                dir_str.trim_start_matches("/ram/")
            };
            crate::fs::get_entries(fs_path.as_bytes())
        } else if dir_str.starts_with("/disk") {
            let disk_path = if dir_str == "/disk" || dir_str == "/disk/" {
                ""
            } else {
                dir_str.trim_start_matches("/disk/")
            };
            crate::diskfs::get_entries(disk_path.as_bytes())
        } else {
            alloc::vec![]
        };
        
        // Find matching entries
        let mut matches: alloc::vec::Vec<(alloc::string::String, bool)> = alloc::vec::Vec::new();
        let prefix_str = core::str::from_utf8(file_prefix).unwrap_or("");
        
        for (name, is_dir, _size) in entries {
            if name.starts_with(prefix_str) {
                matches.push((name, is_dir));
            }
        }
        
        if matches.is_empty() {
            return;
        } else if matches.len() == 1 {
            // Single match - complete it
            let (ref name, is_dir) = matches[0];
            
            // Rebuild line: command + completed path
            self.len = 0;
            
            // Copy command part
            for i in 0..last_space {
                if self.len < LINE_MAX {
                    self.line[self.len] = input[i];
                    self.len += 1;
                }
            }
            
            // Add directory path if partial had one
            if slash_pos.is_some() {
                for &b in dir_path {
                    if self.len < LINE_MAX {
                        self.line[self.len] = b;
                        self.len += 1;
                    }
                }
            }
            
            // Add completed filename
            for b in name.bytes() {
                if self.len < LINE_MAX {
                    self.line[self.len] = b;
                    self.len += 1;
                }
            }
            
            // Add trailing slash for directories or space for files
            if is_dir {
                if self.len < LINE_MAX {
                    self.line[self.len] = b'/';
                    self.len += 1;
                }
            } else if self.len < LINE_MAX {
                self.line[self.len] = b' ';
                self.len += 1;
            }
        } else {
            // Multiple matches - show them
            crate::println!();
            for (name, is_dir) in &matches {
                if *is_dir {
                    crate::print!("{}/  ", name);
                } else {
                    crate::print!("{}  ", name);
                }
            }
            crate::println!();
            
            // Find and apply common prefix
            if let Some(ref common) = Self::find_common_string_prefix(&matches) {
                if common.len() > prefix_str.len() {
                    self.len = 0;
                    
                    for i in 0..last_space {
                        if self.len < LINE_MAX {
                            self.line[self.len] = input[i];
                            self.len += 1;
                        }
                    }
                    
                    if slash_pos.is_some() {
                        for &b in dir_path {
                            if self.len < LINE_MAX {
                                self.line[self.len] = b;
                                self.len += 1;
                            }
                        }
                    }
                    
                    for b in common.bytes() {
                        if self.len < LINE_MAX {
                            self.line[self.len] = b;
                            self.len += 1;
                        }
                    }
                }
            }
        }
    }
    
    /// Find common prefix length among command matches
    fn find_common_prefix(matches: &[Option<&[u8]>]) -> usize {
        let first = match matches.first() {
            Some(Some(s)) => *s,
            _ => return 0,
        };
        
        let mut common_len = first.len();
        
        for opt in matches.iter().skip(1) {
            if let Some(s) = opt {
                let mut i = 0;
                while i < common_len && i < s.len() && first[i] == s[i] {
                    i += 1;
                }
                common_len = i;
            }
        }
        
        common_len
    }
    
    /// Find common prefix among string matches
    fn find_common_string_prefix(matches: &[(alloc::string::String, bool)]) -> Option<alloc::string::String> {
        if matches.is_empty() {
            return None;
        }
        
        let first = &matches[0].0;
        let mut common_len = first.len();
        
        for (s, _) in matches.iter().skip(1) {
            let mut i = 0;
            let first_bytes = first.as_bytes();
            let s_bytes = s.as_bytes();
            while i < common_len && i < s_bytes.len() && first_bytes[i] == s_bytes[i] {
                i += 1;
            }
            common_len = i;
        }
        
        if common_len > 0 {
            Some(alloc::string::String::from(&first[..common_len]))
        } else {
            None
        }
    }

    // ==================== EDITOR ====================
    
    fn start_editor(&mut self, path: &[u8]) {
        // Copy path
        let n = core::cmp::min(path.len(), 96);
        self.editor_path[..n].copy_from_slice(&path[..n]);
        self.editor_path_len = n;
        
        // Clear editor buffer
        self.editor_line_count = 0;
        for i in 0..EDITOR_MAX_LINES {
            self.editor_line_lens[i] = 0;
        }
        
        // Try to load existing file
        if let Ok(data) = vfs::read(path) {
            let mut line_idx = 0;
            let mut col = 0;
            for &b in data.iter() {
                if b == b'\n' {
                    self.editor_line_lens[line_idx] = col;
                    line_idx += 1;
                    col = 0;
                    if line_idx >= EDITOR_MAX_LINES {
                        break;
                    }
                } else if col < EDITOR_LINE_MAX {
                    self.editor_lines[line_idx][col] = b;
                    col += 1;
                }
            }
            // Handle last line without newline
            if col > 0 && line_idx < EDITOR_MAX_LINES {
                self.editor_line_lens[line_idx] = col;
                line_idx += 1;
            }
            self.editor_line_count = line_idx;
        }
        
        self.editor_mode = true;
        self.len = 0; // Clear input line
        
        let path_str = core::str::from_utf8(path).unwrap_or("?");
        crate::println!("--- EDITOR: {} ---", path_str);
        crate::println!("Type lines. Commands: :w (save), :q (quit), :wq (save+quit)");
        crate::println!("Lines: {}", self.editor_line_count);
        crate::println!("---");
    }
    
    fn editor_handle_line(&mut self) {
        let input = &self.line[..self.len];
        
        // Check for commands
        if input == b":q" {
            self.editor_mode = false;
            crate::println!("(quit without saving)");
            return;
        }
        if input == b":w" {
            self.editor_save();
            crate::println!("(saved)");
            return;
        }
        if input == b":wq" {
            self.editor_save();
            self.editor_mode = false;
            crate::println!("(saved and quit)");
            return;
        }
        if input == b":l" {
            // List lines
            crate::println!("--- {} lines ---", self.editor_line_count);
            for i in 0..self.editor_line_count {
                let line = &self.editor_lines[i][..self.editor_line_lens[i]];
                let s = core::str::from_utf8(line).unwrap_or("?");
                crate::println!("{:2}: {}", i + 1, s);
            }
            crate::println!("---");
            return;
        }
        if input.starts_with(b":d") {
            // Delete line :d <num>
            let rest = &input[2..];
            let mut rest = rest;
            while let Some((&b, r)) = rest.split_first() {
                if b == b' ' { rest = r; } else { break; }
            }
            if let Some(num) = Self::parse_u64(rest) {
                let idx = num as usize;
                if idx >= 1 && idx <= self.editor_line_count {
                    // Shift lines up
                    for i in (idx - 1)..(self.editor_line_count - 1) {
                        self.editor_lines[i] = self.editor_lines[i + 1];
                        self.editor_line_lens[i] = self.editor_line_lens[i + 1];
                    }
                    self.editor_line_count -= 1;
                    crate::println!("(deleted line {})", idx);
                } else {
                    crate::println!("(invalid line number)");
                }
            }
            return;
        }
        
        // Add line to buffer
        if self.editor_line_count >= EDITOR_MAX_LINES {
            crate::println!("(buffer full, max {} lines)", EDITOR_MAX_LINES);
            return;
        }
        
        let n = core::cmp::min(input.len(), EDITOR_LINE_MAX);
        self.editor_lines[self.editor_line_count][..n].copy_from_slice(&input[..n]);
        self.editor_line_lens[self.editor_line_count] = n;
        self.editor_line_count += 1;
    }
    
    fn editor_save(&mut self) {
        // Build content with newlines
        let mut data = alloc::vec::Vec::new();
        for i in 0..self.editor_line_count {
            let line = &self.editor_lines[i][..self.editor_line_lens[i]];
            data.extend_from_slice(line);
            data.push(b'\n');
        }
        
        let path = &self.editor_path[..self.editor_path_len];
        match vfs::write(path, &data) {
            Ok(()) => {
                let path_str = core::str::from_utf8(path).unwrap_or("?");
                crate::println!("Wrote {} bytes to {}", data.len(), path_str);
            }
            Err(e) => crate::println!("Save error: {:?}", e),
        }
    }
    
    fn editor_prompt(&mut self) {
        let row = vga::bottom_row();
        vga::clear_row(row, vga::Color::LightGray, vga::Color::Black);
        
        let prompt = alloc::format!("[{}]> ", self.editor_line_count + 1);
        let line = core::str::from_utf8(&self.line[..self.len]).unwrap_or("");
        let display = alloc::format!("{}{}_ ", prompt, line);
        
        vga::write_at(row, 0, &prompt, vga::Color::Yellow, vga::Color::Black);
        vga::write_at(row, prompt.len(), &display[prompt.len()..], vga::Color::White, vga::Color::Black);
    }

    // ==================== END EDITOR ====================
    
    // ==================== PIPE & REDIRECTION SUPPORT ====================
    
    /// Run a command line with pipe and redirection support
    fn run_command(&mut self, line: &[u8]) {
        // trim leading spaces
        let mut line = line;
        while let Some((&b, rest)) = line.split_first() {
            if b == b' ' || b == b'\t' {
                line = rest;
            } else {
                break;
            }
        }
        if line.is_empty() {
            return;
        }
        
        // === VARIABLE EXPANSION ($VAR) ===
        let expanded = self.expand_variables(line);
        let line = if !expanded.is_empty() {
            expanded.as_slice()
        } else {
            line
        };
        
        // === ALIAS EXPANSION ===
        // Check if first word is an alias
        let expanded_alias = self.expand_alias(line);
        let line = if !expanded_alias.is_empty() {
            expanded_alias.as_slice()
        } else {
            line
        };
        
        // Check for pipe (|)
        if let Some(pipe_pos) = line.iter().position(|&b| b == b'|') {
            self.run_piped_command(line, pipe_pos);
            return;
        }
        
        // Check for output redirection (>> or >)
        if let Some(redir_pos) = line.iter().position(|&b| b == b'>') {
            let append = redir_pos + 1 < line.len() && line[redir_pos + 1] == b'>';
            self.run_redirected_command(line, redir_pos, append);
            return;
        }
        
        // No pipe or redirection - run single command
        self.run_single_command(line, None);
    }
    
    /// Expand environment variables in command line ($VAR or ${VAR})
    fn expand_variables(&self, line: &[u8]) -> alloc::vec::Vec<u8> {
        let mut result = alloc::vec::Vec::new();
        let mut i = 0;
        let mut had_expansion = false;
        
        while i < line.len() {
            if line[i] == b'$' && i + 1 < line.len() {
                had_expansion = true;
                i += 1;
                
                // Check for ${VAR} format
                if line[i] == b'{' {
                    i += 1;
                    let start = i;
                    while i < line.len() && line[i] != b'}' {
                        i += 1;
                    }
                    let var_name = &line[start..i];
                    if i < line.len() && line[i] == b'}' {
                        i += 1;
                    }
                    let name_str = core::str::from_utf8(var_name).unwrap_or("");
                    if let Some(val) = env_get(name_str) {
                        result.extend_from_slice(val.as_bytes());
                    }
                } else {
                    // $VAR format - read until non-alphanumeric/underscore
                    let start = i;
                    while i < line.len() && (line[i].is_ascii_alphanumeric() || line[i] == b'_') {
                        i += 1;
                    }
                    let var_name = &line[start..i];
                    let name_str = core::str::from_utf8(var_name).unwrap_or("");
                    if let Some(val) = env_get(name_str) {
                        result.extend_from_slice(val.as_bytes());
                    }
                }
            } else {
                result.push(line[i]);
                i += 1;
            }
        }
        
        if had_expansion {
            result
        } else {
            alloc::vec::Vec::new()  // Return empty if no expansion happened
        }
    }
    
    /// Expand alias if first word matches
    fn expand_alias(&self, line: &[u8]) -> alloc::vec::Vec<u8> {
        // Get first word
        let mut end = 0;
        while end < line.len() && line[end] != b' ' && line[end] != b'\t' {
            end += 1;
        }
        let first_word = &line[..end];
        let rest = &line[end..];
        
        let first_word_str = core::str::from_utf8(first_word).unwrap_or("");
        
        if let Some(alias_val) = alias_get(first_word_str) {
            let mut result = alloc::vec::Vec::new();
            result.extend_from_slice(alias_val.as_bytes());
            result.extend_from_slice(rest);
            result
        } else {
            alloc::vec::Vec::new()  // Return empty if no alias found
        }
    }
    
    /// Run piped commands: cmd1 | cmd2 | cmd3 ...
    fn run_piped_command(&mut self, line: &[u8], pipe_pos: usize) {
        // Split at first pipe
        let cmd1 = &line[..pipe_pos];
        let rest = &line[pipe_pos + 1..];
        
        // Trim cmd1
        let mut cmd1 = cmd1;
        while cmd1.ends_with(b" ") || cmd1.ends_with(b"\t") {
            cmd1 = &cmd1[..cmd1.len() - 1];
        }
        
        // Capture output of first command
        let output = self.capture_command_output(cmd1);
        
        // If there are more pipes, recurse
        if let Some(next_pipe) = rest.iter().position(|&b| b == b'|') {
            // For now, just pass stdin to next command
            // This is simplified - full implementation would need proper stdin handling
            self.run_piped_with_stdin(rest, &output);
        } else {
            // Last command in pipe chain - run with stdin
            self.run_single_command(rest, Some(&output));
        }
    }
    
    /// Run with stdin from previous pipe
    fn run_piped_with_stdin(&mut self, line: &[u8], stdin: &[u8]) {
        // Check for more pipes
        if let Some(pipe_pos) = line.iter().position(|&b| b == b'|') {
            let cmd = &line[..pipe_pos];
            let rest = &line[pipe_pos + 1..];
            
            let output = self.capture_command_with_stdin(cmd, stdin);
            self.run_piped_with_stdin(rest, &output);
        } else {
            self.run_single_command(line, Some(stdin));
        }
    }
    
    /// Run redirected command: cmd > file or cmd >> file
    fn run_redirected_command(&mut self, line: &[u8], redir_pos: usize, append: bool) {
        let cmd = &line[..redir_pos];
        let skip = if append { 2 } else { 1 };
        let file_part = &line[redir_pos + skip..];
        
        // Trim cmd and file
        let mut cmd = cmd;
        while cmd.ends_with(b" ") || cmd.ends_with(b"\t") {
            cmd = &cmd[..cmd.len() - 1];
        }
        
        let file = Self::trim_spaces(file_part);
        if file.is_empty() {
            crate::println!("error: no output file specified");
            return;
        }
        
        // Capture command output
        let output = self.capture_command_output(cmd);
        
        // Resolve file path
        let mut path_buf = [0u8; 96];
        let Some(path_len) = self.resolve_path(file, &mut path_buf) else {
            crate::println!("error: invalid path");
            return;
        };
        let path = &path_buf[..path_len];
        
        // Write or append to file
        if append {
            // Read existing content
            let existing = vfs::read(path).unwrap_or_default();
            let mut combined = existing;
            combined.extend_from_slice(&output);
            match vfs::write(path, &combined) {
                Ok(()) => {}
                Err(e) => crate::println!("error: {:?}", e),
            }
        } else {
            match vfs::write(path, &output) {
                Ok(()) => {}
                Err(e) => crate::println!("error: {:?}", e),
            }
        }
    }
    
    /// Trim leading and trailing spaces from a byte slice
    fn trim_spaces(mut s: &[u8]) -> &[u8] {
        while let Some((&b, rest)) = s.split_first() {
            if b == b' ' || b == b'\t' {
                s = rest;
            } else {
                break;
            }
        }
        while s.ends_with(b" ") || s.ends_with(b"\t") {
            s = &s[..s.len() - 1];
        }
        s
    }
    
    /// Capture command output to a buffer instead of printing
    fn capture_command_output(&mut self, cmd: &[u8]) -> alloc::vec::Vec<u8> {
        // Enable capture mode in VGA
        crate::vga::start_capture();
        
        // Run command
        self.run_single_command(cmd, None);
        
        // Get captured output
        crate::vga::stop_capture()
    }
    
    /// Capture command output with stdin
    fn capture_command_with_stdin(&mut self, cmd: &[u8], stdin: &[u8]) -> alloc::vec::Vec<u8> {
        crate::vga::start_capture();
        self.run_single_command(cmd, Some(stdin));
        crate::vga::stop_capture()
    }
    
    // ==================== END PIPE & REDIRECTION ====================

    /// Run a single command without pipe/redirection
    fn run_single_command(&mut self, line: &[u8], stdin: Option<&[u8]>) {
        // trim leading spaces
        let mut line = line;
        while let Some((&b, rest)) = line.split_first() {
            if b == b' ' || b == b'\t' {
                line = rest;
            } else {
                break;
            }
        }
        if line.is_empty() {
            return;
        }

        // split first token
        let mut end = 0;
        while end < line.len() {
            let b = line[end];
            if b == b' ' || b == b'\t' {
                break;
            }
            end += 1;
        }
        let cmd = &line[..end];
        let args = &line[end..];

        if Self::eq(cmd, b"help") {
            // Parse help category argument
            let category = if let Some((cat, _)) = Self::parse_word(args) {
                cat
            } else {
                b"" as &[u8]
            };
            
            if category.is_empty() {
                // Show help categories
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║         QOS Shell Help System            ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  help files    - File & directory cmds   ║");
                crate::println!("║  help disk     - Disk & VFS commands     ║");
                crate::println!("║  help process  - Process management      ║");
                crate::println!("║  help quantum  - Quantum job commands    ║");
                crate::println!("║  help network  - Network commands        ║");
                crate::println!("║  help system   - System commands         ║");
                crate::println!("║  help gui      - Desktop GUI commands    ║");
                crate::println!("║  help shell    - Shell features & pipes  ║");
                crate::println!("║  help env      - Environment & aliases   ║");
                crate::println!("║  help all      - Show all commands       ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"files") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║       File & Directory Commands          ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  pwd             print current directory ║");
                crate::println!("║  cd <dir>        change directory        ║");
                crate::println!("║  ls [dir]        list files              ║");
                crate::println!("║  cat <path>      print file contents     ║");
                crate::println!("║  rm <path>       delete file             ║");
                crate::println!("║  mkdir <name>    create directory        ║");
                crate::println!("║  touch <name>    create empty file       ║");
                crate::println!("║  write <f> <txt> write text to file      ║");
                crate::println!("║  edit <path>     open file in editor     ║");
                crate::println!("║  mkbell <path>   create bell.qasm sample ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"disk") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║         Disk & VFS Commands              ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  disk-id         identify FS disk        ║");
                crate::println!("║  disk-read <lba> read sector from disk   ║");
                crate::println!("║  mkfs            format disk filesystem  ║");
                crate::println!("║  dls             list disk files         ║");
                crate::println!("║  dcat <file>     print disk file         ║");
                crate::println!("║  drm <file>      delete disk file        ║");
                crate::println!("║  dput <file>     copy RAM -> disk        ║");
                crate::println!("║  dget <file>     copy disk -> RAM        ║");
                crate::println!("╠──────────────────────────────────────────╣");
                crate::println!("║  vls <dir>       list VFS directory      ║");
                crate::println!("║  vcat <path>     cat from VFS path       ║");
                crate::println!("║  vrm <path>      remove from VFS         ║");
                crate::println!("║  vcp <src> <dst> copy between mounts     ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"process") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║        Process Management                ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  ps              show current process    ║");
                crate::println!("║  procs           list all processes      ║");
                crate::println!("║  exec <path>     execute ELF64 binary    ║");
                crate::println!("║  spawn <path>    spawn background proc   ║");
                crate::println!("║  fg <pid>        bring to foreground     ║");
                crate::println!("║  bg [pid]        send to background      ║");
                crate::println!("║  killp <pid>     terminate process       ║");
                crate::println!("║  waitp <pid>     wait for process exit   ║");
                crate::println!("║  userdemo        enter Ring3 demo        ║");
                crate::println!("║  udemo           run demo foreground     ║");
                crate::println!("║  udemo-bg        run demo background     ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"quantum") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║        Quantum Job Commands              ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  submit <f> [n]  submit QASM2 job        ║");
                crate::println!("║  dsubmit <f> [n] submit from disk        ║");
                crate::println!("║  vsubmit <f> [n] submit from VFS path    ║");
                crate::println!("║  qsubmit <code>  submit inline QASM      ║");
                crate::println!("║  qsim            quantum simulator       ║");
                crate::println!("║  qbackend        manage QPU backends     ║");
                crate::println!("║  submit-bell     submit Bell circuit     ║");
                crate::println!("║  submit-ir-bell  submit IR Bell circuit  ║");
                crate::println!("║  jobs            list job table          ║");
                crate::println!("║  status <h>      show job status         ║");
                crate::println!("║  result <h>      get job result+viz      ║");
                crate::println!("║  viz <h>         visualize result        ║");
                crate::println!("║  cancel <h>      cancel job              ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"system") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║          System Commands                 ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  clear           clear screen            ║");
                crate::println!("║  kbd [us|tr]     set keyboard layout     ║");
                crate::println!("║  time            show date/time          ║");
                crate::println!("║  uptime          show system uptime      ║");
                crate::println!("║  ticks           show PIT tick count     ║");
                crate::println!("║  pci             list PCI devices        ║");
                crate::println!("║  powerinfo       ACPI power info         ║");
                crate::println!("║  ui [on|off]     toggle UI overlay       ║");
                crate::println!("║  shutdown        shutdown system         ║");
                crate::println!("║  reboot          reboot system           ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"gui") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║       Desktop GUI Commands               ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  desktop         start desktop demo      ║");
                crate::println!("║  window <title>  create new window       ║");
                crate::println!("║  gui             show GUI help           ║");
                crate::println!("╠──────────────────────────────────────────╣");
                crate::println!("║  Applications:                           ║");
                crate::println!("║  • calc          Calculator app          ║");
                crate::println!("║  • notepad       Text editor             ║");
                crate::println!("║  • explorer      File browser            ║");
                crate::println!("║  • taskmgr       Task manager            ║");
                crate::println!("║  • sysinfo       System information      ║");
                crate::println!("╠──────────────────────────────────────────╣");
                crate::println!("║  Features:                               ║");
                crate::println!("║  • Multiple overlapping windows          ║");
                crate::println!("║  • Taskbar with window buttons           ║");
                crate::println!("║  • Desktop icons (Computer, Files, etc)  ║");
                crate::println!("║  • Focus management                      ║");
                crate::println!("║  • Window controls (minimize/max/close)  ║");
                crate::println!("╠──────────────────────────────────────────╣");
                crate::println!("║  Try: desktop - to see demo              ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"network") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║         Network Commands                 ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  ifconfig        show network interfaces ║");
                crate::println!("║  ping <ip>       send ICMP ping          ║");
                crate::println!("║  arp             show ARP table          ║");
                crate::println!("║  netstat         network statistics      ║");
                crate::println!("╠──────────────────────────────────────────╣");
                crate::println!("║  Note: Run QEMU with -netdev to enable   ║");
                crate::println!("║  network: -netdev user,id=net0           ║");
                crate::println!("║           -device e1000,netdev=net0      ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"shell") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║        Shell Features & Pipes            ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  cmd1 | cmd2     pipe output to command  ║");
                crate::println!("║  cmd > file      redirect to file        ║");
                crate::println!("║  cmd >> file     append to file          ║");
                crate::println!("║  <TAB>           auto-complete command   ║");
                crate::println!("╠──────────────────────────────────────────╣");
                crate::println!("║  echo <text>     print text              ║");
                crate::println!("║  grep <pat> [f]  filter matching lines   ║");
                crate::println!("║  wc [file]       count lines/words/chars ║");
                crate::println!("║  head [-n N] [f] first N lines           ║");
                crate::println!("║  tail [-n N] [f] last N lines            ║");
                crate::println!("║  sort [file]     sort lines              ║");
                crate::println!("║  uniq [file]     remove duplicates       ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"env") {
                crate::println!("╔══════════════════════════════════════════╗");
                crate::println!("║      Environment & Aliases               ║");
                crate::println!("╠══════════════════════════════════════════╣");
                crate::println!("║  env             list all env vars       ║");
                crate::println!("║  export VAR=val  set environment var     ║");
                crate::println!("║  unset VAR       remove env variable     ║");
                crate::println!("║  $VAR            expand variable         ║");
                crate::println!("║  ${{VAR}}          expand variable         ║");
                crate::println!("╠──────────────────────────────────────────╣");
                crate::println!("║  alias           list all aliases        ║");
                crate::println!("║  alias n=cmd     create alias            ║");
                crate::println!("║  unalias name    remove alias            ║");
                crate::println!("╠──────────────────────────────────────────╣");
                crate::println!("║  source <file>   execute script (.qsh)   ║");
                crate::println!("║  run <file>      execute script (.qsh)   ║");
                crate::println!("╚══════════════════════════════════════════╝");
            } else if Self::eq(category, b"all") {
                // Page 1
                crate::println!("═══ FILE COMMANDS ═══");
                crate::println!("  pwd, cd, ls, cat, rm, mkdir, touch, write, edit, mkbell");
                crate::println!("═══ DISK/VFS COMMANDS ═══");
                crate::println!("  disk-id, disk-read, mkfs, dls, dcat, drm, dput, dget");
                crate::println!("  vls, vcat, vrm, vcp");
                crate::println!("═══ PROCESS COMMANDS ═══");
                crate::println!("  ps, procs, exec, spawn, fg, bg, killp, waitp");
                crate::println!("  userdemo, udemo, udemo-bg");
                crate::println!("═══ QUANTUM COMMANDS ═══");
                crate::println!("  submit, dsubmit, vsubmit, qsubmit, qsim");
                crate::println!("  submit-bell, submit-ir-bell, jobs, status, result, cancel");
                crate::println!("═══ NETWORK COMMANDS ═══");
                crate::println!("  ifconfig, ping, arp, netstat, dhcp");
                crate::println!("═══ SYSTEM COMMANDS ═══");
                crate::println!("  clear, kbd, time, uptime, ticks, pci");
                crate::println!("  powerinfo, ui, shutdown, reboot");
                crate::println!("═══ SHELL FEATURES ═══");
                crate::println!("  Pipes: cmd1 | cmd2    Redirect: > >>    Tab completion");
                crate::println!("  echo, grep, wc, head, tail, sort, uniq");
                crate::println!("═══ ENVIRONMENT ═══");
                crate::println!("  env, export, unset, alias, unalias, source, run");
                crate::println!("─────────────────────────────────────────────");
                crate::println!("Type 'help <category>' for details");
            } else {
                crate::println!("Unknown category. Type 'help' for categories.");
            }
        } else if Self::eq(cmd, b"kbd") {
            let Some((arg, _)) = Self::parse_word(args) else {
                crate::println!("keyboard: {}", self.kbd_name());
                crate::println!("usage: kbd us | kbd tr");
                return;
            };

            if Self::eq(arg, b"us") {
                self.set_kbd_mode(KeyboardMode::Us);
                return;
            }
            if Self::eq(arg, b"tr") {
                self.set_kbd_mode(KeyboardMode::Tr);
                return;
            }

            crate::println!("usage: kbd us | kbd tr");
        } else if Self::eq(cmd, b"time") {
            crate::println!("{}", crate::rtc::time_string());
        } else if Self::eq(cmd, b"uptime") {
            crate::println!("Uptime: {}", crate::timer::uptime_string());
        } else if Self::eq(cmd, b"pci") {
            crate::pci::list_devices();
        } else if Self::eq(cmd, b"net") {
            crate::net::show_info();
        } else if Self::eq(cmd, b"shutdown") {
            crate::println!("Shutting down...");
            crate::acpi::shutdown();
        } else if Self::eq(cmd, b"reboot") {
            crate::println!("Rebooting...");
            crate::acpi::reboot();
        } else if Self::eq(cmd, b"powerinfo") {
            crate::acpi::power_info();
        } else if Self::eq(cmd, b"desktop") {
            crate::println!("Starting Desktop Environment...");
            crate::desktop::init();
            crate::desktop::demo();
        } else if Self::eq(cmd, b"window") {
            if args.is_empty() {
                crate::println!("usage: window <title>");
                return;
            }
            let title = core::str::from_utf8(args).unwrap_or("Window");
            let win_id = crate::desktop::create_window(title, 5, 2, 50, 10);
            crate::desktop::window_add_line(win_id, "");
            crate::desktop::window_add_line(win_id, "  This is a demo window!");
            crate::desktop::window_add_line(win_id, "");
            crate::desktop::window_add_line(win_id, "  You can create multiple windows.");
            crate::desktop::render();
            crate::println!("Window created: {}", win_id);
        } else if Self::eq(cmd, b"gui") {
            crate::println!("=== QOS Desktop Environment ===");
            crate::println!("Commands:");
            crate::println!("  desktop - Start desktop with demo windows");
            crate::println!("  window <title> - Create a new window");
            crate::println!("");
            crate::println!("Features:");
            crate::println!("  - Multiple overlapping windows");
            crate::println!("  - Taskbar with window buttons");
            crate::println!("  - Desktop icons");
            crate::println!("  - Focus management");
            crate::println!("  - Window minimize/maximize/close");
        } else if Self::eq(cmd, b"calc") {
            crate::desktop_apps::launch_calculator();
            crate::println!("Calculator launched");
        } else if Self::eq(cmd, b"notepad") {
            crate::desktop_apps::launch_notepad();
            crate::println!("Notepad launched");
        } else if Self::eq(cmd, b"explorer") {
            crate::desktop_apps::launch_file_browser();
            crate::println!("File Explorer launched");
        } else if Self::eq(cmd, b"taskmgr") {
            crate::desktop_apps::launch_task_manager();
            crate::println!("Task Manager launched");
        } else if Self::eq(cmd, b"sysinfo") {
            crate::desktop_apps::launch_system_info();
            crate::println!("System Information launched");
        } else if Self::eq(cmd, b"clear") {
            crate::vga::clear_screen();
        } else if Self::eq(cmd, b"gfx") {
            crate::println!("Switching to VGA Mode 13h (320x200x256)... press ESC to return.");
            crate::vga13h::demo();
            crate::println!("Back in text mode.");
        } else if Self::eq(cmd, b"gdesk") {
            crate::println!("Launching graphical desktop (move mouse, drag/close window, ESC to exit)...");
            crate::gfxui::run();
            crate::println!("Back in text mode.");
        } else if Self::eq(cmd, b"crash") {
            // Diagnostic: deliberately dereference an unmapped address to exercise the
            // page-fault handler (Phase 0.3). Halts the kernel with a diagnostic banner.
            crate::println!("crash: triggering a page fault for diagnostics...");
            let bad = 0xdead_0000_beef_0000u64 as *const u8;
            let _v = unsafe { core::ptr::read_volatile(bad) };
            crate::println!("crash: unexpectedly survived: {}", _v);
        } else if Self::eq(cmd, b"threadtest") {
            // Phase 2.1: prove preemptive context switching with two kernel threads.
            crate::kthread::demo();
        } else if Self::eq(cmd, b"proctest") {
            // Phase 2.1b: prove preemptive Ring-3 multitasking. Spawn two runaway user
            // processes (infinite loops, no syscalls) and confirm the shell stays alive —
            // i.e. the timer preempts Ring 3 and a runaway program cannot freeze the OS.
            use core::sync::atomic::Ordering;
            crate::kthread::reset();
            crate::user::clear_ring3_test_stacks();
            let h1 = crate::user::spawn_ring3_spinner();
            let h2 = crate::user::spawn_ring3_spinner();
            crate::kthread::adopt_user(h1.saved_rsp, h1.cr3, h1.rsp0_top);
            crate::kthread::adopt_user(h2.saved_rsp, h2.cr3, h2.rsp0_top);
            crate::serial_println!("[PROCTEST] 2 ring3 spinners adopted; arming preemption");
            crate::println!("proctest: 2 runaway ring3 processes (infinite loops) running...");
            let start = interrupts::TICKS.load(Ordering::Relaxed);
            let mut beats = 0u64;
            crate::kthread::arm();
            loop {
                let elapsed = interrupts::TICKS.load(Ordering::Relaxed).wrapping_sub(start);
                if elapsed / 40 > beats {
                    beats = elapsed / 40;
                    crate::serial_println!(
                        "[PROCTEST] shell alive @tick+{} (both ring3 loops preempted)",
                        elapsed
                    );
                    crate::print!("*");
                    if beats >= 6 {
                        break;
                    }
                }
                x86_64::instructions::hlt();
            }
            // Disarm while the shell (main context) is current, so we keep control.
            crate::kthread::disarm();
            crate::kthread::reset();
            crate::memory::switch_to_kernel_cr3();
            crate::user::clear_ring3_test_stacks();
            crate::println!();
            crate::println!("proctest: shell survived while 2 ring3 loops ran -> Ring 3 preemption OK");
            crate::serial_println!("[PROCTEST] done; runaway ring3 loops did NOT freeze the OS");
        } else if Self::eq(cmd, b"faulttest") {
            // Phase 2.1b: prove fault isolation. One process spins forever, the other crashes
            // (page fault). The crash must kill ONLY the faulting process — the shell and the
            // spinner keep running.
            use core::sync::atomic::Ordering;
            crate::kthread::reset();
            crate::user::clear_ring3_test_stacks();
            let spinner = crate::user::spawn_ring3_spinner();
            let faulter = crate::user::spawn_ring3_faulter();
            crate::kthread::adopt_user(spinner.saved_rsp, spinner.cr3, spinner.rsp0_top);
            crate::kthread::adopt_user(faulter.saved_rsp, faulter.cr3, faulter.rsp0_top);
            crate::serial_println!("[FAULTTEST] spinner + crashing process adopted; arming");
            crate::println!("faulttest: 1 spinner + 1 crashing ring3 process; watch serial...");
            let start = interrupts::TICKS.load(Ordering::Relaxed);
            let mut beats = 0u64;
            crate::kthread::arm();
            loop {
                let elapsed = interrupts::TICKS.load(Ordering::Relaxed).wrapping_sub(start);
                if elapsed / 40 > beats {
                    beats = elapsed / 40;
                    crate::serial_println!("[FAULTTEST] shell alive @tick+{}", elapsed);
                    crate::print!("*");
                    if beats >= 8 {
                        break;
                    }
                }
                x86_64::instructions::hlt();
            }
            crate::kthread::disarm();
            crate::kthread::reset();
            crate::memory::switch_to_kernel_cr3();
            crate::user::clear_ring3_test_stacks();
            crate::println!();
            crate::println!("faulttest: a user crash killed only that process; kernel survived");
            crate::serial_println!("[FAULTTEST] done; crash was isolated to the faulting process");
        } else if Self::eq(cmd, b"exittest") {
            // Phase 2.1b: prove clean voluntary exit. One process spins forever, the other
            // busy-loops then calls the OP_EXIT syscall. The exiting process is reaped and the
            // shell + spinner keep running (no whole-kernel restart).
            use core::sync::atomic::Ordering;
            crate::kthread::reset();
            crate::user::clear_ring3_test_stacks();
            let spinner = crate::user::spawn_ring3_spinner();
            let exiter = crate::user::spawn_ring3_exiter();
            crate::kthread::adopt_user(spinner.saved_rsp, spinner.cr3, spinner.rsp0_top);
            crate::kthread::adopt_user(exiter.saved_rsp, exiter.cr3, exiter.rsp0_top);
            crate::serial_println!("[EXITTEST] spinner + self-exiting process adopted; arming");
            crate::println!("exittest: 1 spinner + 1 self-exiting ring3 process; watch serial...");
            let start = interrupts::TICKS.load(Ordering::Relaxed);
            let mut beats = 0u64;
            crate::kthread::arm();
            loop {
                let elapsed = interrupts::TICKS.load(Ordering::Relaxed).wrapping_sub(start);
                if elapsed / 40 > beats {
                    beats = elapsed / 40;
                    crate::serial_println!("[EXITTEST] shell alive @tick+{}", elapsed);
                    crate::print!("*");
                    if beats >= 8 {
                        break;
                    }
                }
                x86_64::instructions::hlt();
            }
            crate::kthread::disarm();
            crate::kthread::reset();
            crate::memory::switch_to_kernel_cr3();
            crate::user::clear_ring3_test_stacks();
            crate::println!();
            crate::println!("exittest: a process exited via syscall; kernel + spinner survived");
            crate::serial_println!("[EXITTEST] done; clean voluntary exit returned control");
        } else if Self::eq(cmd, b"regabitest") {
            // Phase 2.2: prove the register-based syscall ABI (int 0x81). A ring3 process
            // calls SYS_WRITE (print) then SYS_EXIT via registers; we wait for it to finish.
            use core::sync::atomic::Ordering;
            crate::kthread::reset();
            crate::user::clear_ring3_test_stacks();
            let h = crate::user::spawn_ring3_regabi();
            crate::kthread::adopt_user(h.saved_rsp, h.cr3, h.rsp0_top);
            crate::serial_println!("[REGABI] process adopted; arming");
            crate::println!("regabitest: ring3 program using int 0x81 register ABI; watch serial...");
            let start = interrupts::TICKS.load(Ordering::Relaxed);
            crate::kthread::arm();
            while !crate::kthread::all_finished() {
                if interrupts::TICKS.load(Ordering::Relaxed).wrapping_sub(start) > 400 {
                    crate::serial_println!("[REGABI] timeout waiting for process");
                    break;
                }
                x86_64::instructions::hlt();
            }
            crate::kthread::disarm();
            crate::kthread::reset();
            crate::memory::switch_to_kernel_cr3();
            crate::user::clear_ring3_test_stacks();
            crate::println!("regabitest: done (see serial for the ring3 message + exit)");
            crate::serial_println!("[REGABI] done");
        } else if Self::eq(cmd, b"evtest") {
            crate::println!("evtest: capturing input events ~3s (press keys / move mouse), see serial");
            let start = interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
            let mut n = 0u32;
            while interrupts::TICKS
                .load(core::sync::atomic::Ordering::Relaxed)
                .wrapping_sub(start)
                < 300
            {
                while let Some(ev) = crate::input::poll() {
                    crate::serial_println!("evt: {:?}", ev);
                    n += 1;
                }
                x86_64::instructions::hlt();
            }
            crate::println!("evtest: captured {} events", n);
        } else if Self::eq(cmd, b"ticks") {
            let t = interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
            crate::println!("ticks={}", t);
        } else if Self::eq(cmd, b"ps") {
            let p = crate::process::current();
            crate::println!(
                "pid={} state={:?} exit_code={} image_hash={:#x}",
                p.pid,
                p.state,
                p.exit_code,
                p.image_hash
            );
        } else if Self::eq(cmd, b"procs") {
            let procs = crate::tasking::list_processes();
            let fg = crate::tasking::foreground_pid();
            if procs.is_empty() {
                crate::println!("(no scheduled processes)");
                return;
            }
            crate::println!("pid  fg  state    exit_code");
            for (pid, st, code) in procs {
                let mark = if pid == fg { "*" } else { " " };
                crate::println!("{}  {}   {:?}  {}", pid, mark, st, code);
            }
        } else if Self::eq(cmd, b"spawn") {
            let Some((path, _)) = Self::parse_word(args) else {
                crate::println!("usage: spawn <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("spawn: bad path");
                return;
            };
            let bytes = match vfs::read(&buf[..n]) {
                Ok(b) => b,
                Err(e) => {
                    crate::println!("err: {:?}", e);
                    return;
                }
            };

            // User module disabled due to LLVM asm bug
            crate::println!("spawn: user mode disabled (LLVM asm bug)");
            crate::println!("ELF size: {} bytes", bytes.len());
            // Spawn functionality disabled
        } else if Self::eq(cmd, b"fg") {
            let Some(pid) = Self::parse_u64(args) else {
                crate::println!("usage: fg <pid>");
                return;
            };
            if let Some((st, _)) = crate::tasking::find_process(pid) {
                if st == crate::tasking::ProcState::Exited {
                    crate::println!("pid {} already exited", pid);
                    return;
                }
                crate::tasking::set_foreground(pid);
                crate::println!("foreground pid={}", pid);

                // Foreground wait: avoid polling every tick by sleeping until an exit event occurs.
                let mut seen = crate::tasking::exit_seq();
                loop {
                    if let Some((st, code)) = crate::tasking::find_process(pid) {
                        if st == crate::tasking::ProcState::Exited {
                            crate::println!("pid {} exited (code={})", pid, code);
                            crate::tasking::clear_foreground(pid);
                            break;
                        }
                    } else {
                        crate::println!("not found {}", pid);
                        crate::tasking::clear_foreground(pid);
                        break;
                    }

                    let now = crate::tasking::exit_seq();
                    if now == seen {
                        crate::arch::hlt();
                    } else {
                        seen = now;
                    }
                }
            } else {
                crate::println!("not found {}", pid);
            }
        } else if Self::eq(cmd, b"bg") {
            // Minimal bg: clear foreground so Ctrl+C no longer targets it.
            // If a pid is provided, it must exist; we just clear foreground regardless.
            if let Some(pid) = Self::parse_u64(args) {
                if crate::tasking::foreground_pid() == pid {
                    crate::tasking::clear_foreground(pid);
                }
                crate::println!("background pid={}", pid);
            } else {
                let fg = crate::tasking::foreground_pid();
                if fg != 0 {
                    crate::tasking::clear_foreground(fg);
                }
                crate::println!("background (no foreground)");
            }
        } else if Self::eq(cmd, b"ui") {
            let on = if let Some((w, _)) = Self::parse_word(args) {
                if Self::eq(w, b"on") {
                    Some(true)
                } else if Self::eq(w, b"off") {
                    Some(false)
                } else {
                    None
                }
            } else {
                None
            };

            match on {
                Some(v) => crate::ui::set_enabled(v),
                None => crate::ui::set_enabled(!crate::ui::enabled()),
            }
        } else if Self::eq(cmd, b"killp") {
            let Some(pid) = Self::parse_u64(args) else {
                crate::println!("usage: killp <pid>");
                return;
            };
            if crate::tasking::kill_with_exit(pid, 0x100 + 9) {
                crate::println!("killed {}", pid);
            } else {
                crate::println!("not found {}", pid);
            }
        } else if Self::eq(cmd, b"waitp") {
            let Some(pid) = Self::parse_u64(args) else {
                crate::println!("usage: waitp <pid>");
                return;
            };
            let mut seen = crate::tasking::exit_seq();
            loop {
                if let Some((st, code)) = crate::tasking::find_process(pid) {
                    if st == crate::tasking::ProcState::Exited {
                        crate::println!("pid {} exited (code={})", pid, code);
                        break;
                    }
                } else {
                    crate::println!("not found {}", pid);
                    break;
                }
                let now = crate::tasking::exit_seq();
                if now == seen {
                    crate::arch::hlt();
                } else {
                    seen = now;
                }
            }
        } else if Self::eq(cmd, b"pwd") {
            let cwd = core::str::from_utf8(self.cwd_bytes()).unwrap_or("?");
            crate::println!("{}", cwd);
        } else if Self::eq(cmd, b"cd") {
            let target = if let Some((dir, _)) = Self::parse_word(args) {
                dir
            } else {
                b"/ram"
            };
            // Handle special cases
            if target == b".." {
                // Go up one directory
                if let Some(pos) = self.cwd_bytes().iter().rposition(|&b| b == b'/') {
                    if pos == 0 {
                        self.set_cwd(b"/");
                    } else {
                        let parent = &self.cwd[..pos];
                        let mut tmp = [0u8; CWD_MAX];
                        tmp[..pos].copy_from_slice(parent);
                        self.cwd_len = pos;
                        self.cwd = tmp;
                    }
                }
                return;
            }
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(target, &mut buf) else {
                crate::println!("cd: bad path");
                return;
            };
            let resolved = &buf[..n];
            // Check if it's a valid directory
            if resolved == b"/" || resolved == b"/ram" || resolved == b"/disk" {
                self.set_cwd(resolved);
            } else if resolved.starts_with(b"/ram/") {
                // Check if it exists and is a directory in ramfs
                let subpath = &resolved[5..];
                if crate::fs::is_dir(subpath) {
                    self.set_cwd(resolved);
                } else if crate::fs::exists(subpath) {
                    crate::println!("cd: not a directory");
                } else {
                    crate::println!("cd: no such directory");
                }
            } else {
                crate::println!("cd: no such directory");
            }
        } else if Self::eq(cmd, b"ls") {
            let dir = if let Some((p, _)) = Self::parse_word(args) {
                p
            } else {
                self.cwd_bytes()
            };
            let mut buf = [0u8; 96];
            let resolved = if dir.starts_with(b"/") {
                Self::trim_trailing_slash(dir)
            } else {
                let Some(n) = self.resolve_path(dir, &mut buf) else {
                    crate::println!("ls: bad path");
                    return;
                };
                &buf[..n]
            };
            if let Err(e) = vfs::list_dir(resolved) {
                crate::println!("err: {:?}", e);
            }
        } else if Self::eq(cmd, b"cat") {
            let Some((name, _rest)) = Self::parse_word(args) else {
                crate::println!("usage: cat <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(name, &mut buf) else {
                crate::println!("cat: bad path");
                return;
            };
            match vfs::read(&buf[..n]) {
                Ok(bytes) => {
                    let s = core::str::from_utf8(&bytes).unwrap_or("<non-utf8>");
                    crate::println!("{}", s);
                }
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"write") {
            let Some((path, rest)) = Self::parse_word(args) else {
                crate::println!("usage: write <path> <data>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("write: bad path");
                return;
            };
            // Trim leading whitespace from rest (the data)
            let mut data = rest;
            while let Some((&b, r)) = data.split_first() {
                if b == b' ' || b == b'\t' {
                    data = r;
                } else {
                    break;
                }
            }
            if data.is_empty() {
                crate::println!("usage: write <path> <data>");
                return;
            }
            match vfs::write(&buf[..n], data) {
                Ok(()) => crate::println!("ok"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"rm") {
            let Some((name, _rest)) = Self::parse_word(args) else {
                crate::println!("usage: rm <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(name, &mut buf) else {
                crate::println!("rm: bad path");
                return;
            };
            match vfs::remove(&buf[..n]) {
                Ok(()) => crate::println!("removed"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"mkdir") {
            let Some((name, _rest)) = Self::parse_word(args) else {
                crate::println!("usage: mkdir <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(name, &mut buf) else {
                crate::println!("mkdir: bad path");
                return;
            };
            match vfs::mkdir(&buf[..n]) {
                Ok(()) => crate::println!("created"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"touch") {
            let Some((name, _rest)) = Self::parse_word(args) else {
                crate::println!("usage: touch <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(name, &mut buf) else {
                crate::println!("touch: bad path");
                return;
            };
            // Create empty file
            match vfs::write(&buf[..n], b"") {
                Ok(()) => crate::println!("created"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"mkbell") {
            let Some((name, _rest)) = Self::parse_word(args) else {
                crate::println!("usage: mkbell <path>");
                return;
            };
            const IR: &[u8] = b"OPENQASM 2.0;\ninclude \"qelib1.inc\";\nqreg q[2];\ncreg c[2];\nh q[0];\ncx q[0],q[1];\nmeasure q[0] -> c[0];\nmeasure q[1] -> c[1];\n";
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(name, &mut buf) else {
                crate::println!("mkbell: bad path");
                return;
            };
            match vfs::write(&buf[..n], IR) {
                Ok(()) => crate::println!("ok"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"submit") {
            let Some((name, rest)) = Self::parse_word(args) else {
                crate::println!("usage: submit <path> [shots]");
                return;
            };
            let shots = Self::parse_u64(rest).map(|v| v as u32).unwrap_or(1024);
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(name, &mut buf) else {
                crate::println!("submit: bad path");
                return;
            };
            let bytes = match vfs::read(&buf[..n]) {
                Ok(b) => b,
                Err(e) => {
                    crate::println!("err: {:?}", e);
                    return;
                }
            };
            // Parse QASM to get n_qubits
            let qasm_str = core::str::from_utf8(&bytes).unwrap_or("");
            let n_qubits = crate::quantum::count_qubits_from_qasm(qasm_str);
            match syscall::shell_submit_ir_qasm2(&bytes, shots, n_qubits) {
                Some(h) => crate::println!("submitted handle={} (shots={}, qubits={})", h, shots, n_qubits),
                None => crate::println!("submit failed"),
            }
        } else if Self::eq(cmd, b"disk-id") {
            let disk = ata::AtaPio::primary(ata::DriveSelect::Slave);
            let mut id = [0u16; 256];
            if !disk.identify(&mut id) {
                crate::println!("no fs disk (ide index=1)" );
                return;
            }
            let model = ata::parse_model(&id);
            let model = core::str::from_utf8(&model).unwrap_or("?");
            crate::println!("fs disk model: {}", model.trim());
        } else if Self::eq(cmd, b"disk-read") {
            let Some(lba) = Self::parse_u64(args) else {
                crate::println!("usage: disk-read <lba>");
                return;
            };
            let disk = ata::AtaPio::primary(ata::DriveSelect::Slave);
            let mut sec = [0u8; 512];
            if !disk.read_sector28(lba as u32, &mut sec) {
                crate::println!("read failed");
                return;
            }
            // Print first 32 bytes as hex.
            crate::print!("0x{:08x}: ", lba as u32);
            for b in &sec[..32] {
                crate::print!("{:02x} ", *b);
            }
            crate::println!("");
        } else if Self::eq(cmd, b"mkfs") {
            if diskfs::mkfs() {
                crate::println!("diskfs formatted");
            } else {
                crate::println!("mkfs failed (use cargo xtask run-fs?)");
            }
        } else if Self::eq(cmd, b"dls") {
            if !diskfs::is_formatted() {
                crate::println!("diskfs not formatted (run mkfs)");
                return;
            }
            if !diskfs::list() {
                crate::println!("dls failed");
            }
        } else if Self::eq(cmd, b"dcat") {
            let Some((name, _)) = Self::parse_word(args) else {
                crate::println!("usage: dcat <file>");
                return;
            };
            let Some(bytes) = diskfs::read(name) else {
                crate::println!("not found");
                return;
            };
            let s = core::str::from_utf8(&bytes).unwrap_or("<non-utf8>");
            crate::println!("{}", s);
        } else if Self::eq(cmd, b"drm") {
            let Some((name, _)) = Self::parse_word(args) else {
                crate::println!("usage: drm <file>");
                return;
            };
            if diskfs::remove(name) {
                crate::println!("removed");
            } else {
                crate::println!("not found");
            }
        } else if Self::eq(cmd, b"dput") {
            let Some((name, _)) = Self::parse_word(args) else {
                crate::println!("usage: dput <file>");
                return;
            };
            let mut src = [0u8; 96];
            let mut dst = [0u8; 96];
            let Some(nsrc) = self.resolve_path(name, &mut src) else {
                crate::println!("dput: bad src path");
                return;
            };
            // destination always disk/<basename>
            let dst_name = name;
            let dpath = b"/disk/";
            if dpath.len() + dst_name.len() > dst.len() {
                crate::println!("dput: name too long");
                return;
            }
            dst[..dpath.len()].copy_from_slice(dpath);
            dst[dpath.len()..dpath.len() + dst_name.len()].copy_from_slice(dst_name);
            let ndst = dpath.len() + dst_name.len();
            match vfs::copy(&src[..nsrc], &dst[..ndst]) {
                Ok(()) => crate::println!("ok"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"dget") {
            let Some((name, _)) = Self::parse_word(args) else {
                crate::println!("usage: dget <file>");
                return;
            };
            let mut src = [0u8; 96];
            let mut dst = [0u8; 96];
            let sp = b"/disk/";
            if sp.len() + name.len() > src.len() {
                crate::println!("dget: name too long");
                return;
            }
            src[..sp.len()].copy_from_slice(sp);
            src[sp.len()..sp.len() + name.len()].copy_from_slice(name);
            let nsrc = sp.len() + name.len();
            let Some(ndst) = self.resolve_path(name, &mut dst) else {
                crate::println!("dget: bad dst path");
                return;
            };
            match vfs::copy(&src[..nsrc], &dst[..ndst]) {
                Ok(()) => crate::println!("ok"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"dsubmit") {
            let Some((name, rest)) = Self::parse_word(args) else {
                crate::println!("usage: dsubmit <file> [shots]");
                return;
            };
            let shots = Self::parse_u64(rest).map(|v| v as u32).unwrap_or(1024);
            let mut p = [0u8; 96];
            let sp = b"/disk/";
            if sp.len() + name.len() > p.len() {
                crate::println!("dsubmit: name too long");
                return;
            }
            p[..sp.len()].copy_from_slice(sp);
            p[sp.len()..sp.len() + name.len()].copy_from_slice(name);
            let np = sp.len() + name.len();
            let bytes = match vfs::read(&p[..np]) {
                Ok(b) => b,
                Err(e) => {
                    crate::println!("err: {:?}", e);
                    return;
                }
            };
            match syscall::shell_submit_ir_qasm2(&bytes, shots, 0) {
                Some(h) => crate::println!("submitted handle={} (shots={})", h, shots),
                None => crate::println!("submit failed"),
            }
        } else if Self::eq(cmd, b"vls") {
            let dir = if let Some((d, _)) = Self::parse_word(args) {
                d
            } else {
                self.cwd_bytes()
            };
            let mut buf = [0u8; 96];
            let resolved = if dir.starts_with(b"/") {
                Self::trim_trailing_slash(dir)
            } else {
                let Some(n) = self.resolve_path(dir, &mut buf) else {
                    crate::println!("vls: bad path");
                    return;
                };
                &buf[..n]
            };
            match vfs::list_dir(resolved) {
                Ok(()) => {}
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"vcat") {
            let Some((path, _)) = Self::parse_word(args) else {
                crate::println!("usage: vcat <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("vcat: bad path");
                return;
            };
            match vfs::read(&buf[..n]) {
                Ok(bytes) => {
                    let s = core::str::from_utf8(&bytes).unwrap_or("<non-utf8>");
                    crate::println!("{}", s);
                }
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"vrm") {
            let Some((path, _)) = Self::parse_word(args) else {
                crate::println!("usage: vrm <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("vrm: bad path");
                return;
            };
            match vfs::remove(&buf[..n]) {
                Ok(()) => crate::println!("removed"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"vcp") {
            let Some((src, rest)) = Self::parse_word(args) else {
                crate::println!("usage: vcp <src> <dst>");
                return;
            };
            let Some((dst, _)) = Self::parse_word(rest) else {
                crate::println!("usage: vcp <src> <dst>");
                return;
            };
            let mut bsrc = [0u8; 96];
            let mut bdst = [0u8; 96];
            let Some(nsrc) = self.resolve_path(src, &mut bsrc) else {
                crate::println!("vcp: bad src");
                return;
            };
            let Some(ndst) = self.resolve_path(dst, &mut bdst) else {
                crate::println!("vcp: bad dst");
                return;
            };
            match vfs::copy(&bsrc[..nsrc], &bdst[..ndst]) {
                Ok(()) => crate::println!("ok"),
                Err(e) => crate::println!("err: {:?}", e),
            }
        } else if Self::eq(cmd, b"vsubmit") {
            let Some((path, rest)) = Self::parse_word(args) else {
                crate::println!("usage: vsubmit <path> [shots]");
                return;
            };
            let shots = Self::parse_u64(rest).map(|v| v as u32).unwrap_or(1024);
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("vsubmit: bad path");
                return;
            };
            let bytes = match vfs::read(&buf[..n]) {
                Ok(b) => b,
                Err(e) => {
                    crate::println!("err: {:?}", e);
                    return;
                }
            };
            match syscall::shell_submit_ir_qasm2(&bytes, shots, 0) {
                Some(h) => crate::println!("submitted handle={} (shots={})", h, shots),
                None => crate::println!("submit failed"),
            }
        } else if Self::eq(cmd, b"userdemo") {
            crate::println!("userdemo: user mode disabled (LLVM asm bug)");
            crate::println!("Use shell quantum commands instead:");
            crate::println!("  submit-bell, jobs, result, viz");
            // crate::user::exec_userdemo();
        } else if Self::eq(cmd, b"udemo") {
            crate::println!("udemo: user mode disabled (LLVM asm bug)");
            crate::println!("Use shell quantum commands instead");
            /* 
            match crate::memory::with_ctx(|_, fa| crate::user::spawn_userdemo_process(fa)) {
                Ok(proc) => {
                    let pid = crate::tasking::spawn_user_process(
                        proc.user_cr3, proc.entry, proc.user_stack_top, proc.mapped_pages
                    );
                    crate::println!("user demo started, PID={}", pid);
                }
                Err(e) => crate::println!("spawn failed: {}", e),
            }
            */
        } else if Self::eq(cmd, b"udemo-bg") {
            crate::println!("udemo-bg: user mode disabled (LLVM asm bug)");
            crate::println!("Use shell quantum commands instead");
            /*
            match crate::memory::with_ctx(|_, fa| crate::user::spawn_userdemo_process(fa)) {
                Ok(proc) => {
                    let pid = crate::tasking::spawn_user_process(
                        proc.user_cr3, proc.entry, proc.user_stack_top, proc.mapped_pages
                    );
                    crate::println!("user demo PID={} running in background", pid);
                }
                Err(e) => crate::println!("spawn failed: {}", e),
            }
            */
        } else if Self::eq(cmd, b"exec") {
            let Some((path, _)) = Self::parse_word(args) else {
                crate::println!("usage: exec <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("exec: bad path");
                return;
            };
            let bytes = match vfs::read(&buf[..n]) {
                Ok(b) => b,
                Err(e) => {
                    crate::println!("err: {:?}", e);
                    return;
                }
            };
            crate::println!("exec: user mode disabled (LLVM asm bug)");
            crate::println!("ELF size: {} bytes", bytes.len());
        } else if Self::eq(cmd, b"jobs") {
            syscall::shell_list_jobs();
        } else if Self::eq(cmd, b"submit-bell") {
            match syscall::shell_submit_bell() {
                Some(h) => crate::println!("submitted handle={}", h),
                None => crate::println!("submit failed (no slots)"),
            }
        } else if Self::eq(cmd, b"submit-ir-bell") {
            let shots = Self::parse_u64(args).map(|v| v as u32).unwrap_or(1024);
            match syscall::shell_submit_ir_bell(shots) {
                Some(h) => crate::println!("submitted handle={} (shots={})", h, shots),
                None => crate::println!("submit failed (no slots)"),
            }
        } else if Self::eq(cmd, b"status") {
            let Some(h) = Self::parse_u64(args) else {
                crate::println!("usage: status <handle>");
                return;
            };
            match syscall::shell_status(h) {
                Some(st) => crate::println!("status {} -> {:?}", h, st),
                None => crate::println!("status {} -> not found", h),
            }
        } else if Self::eq(cmd, b"result") {
            let Some(h) = Self::parse_u64(args) else {
                crate::println!("usage: result <handle>");
                return;
            };
            match syscall::shell_result(h) {
                Ok((n00, n11)) => {
                    crate::println!("Job #{} Results:", h);
                    crate::qviz::draw_bell_result(n00, n11);
                }
                Err(st) => crate::println!("result {} -> not ready (state={:?})", h, st),
            }
        } else if Self::eq(cmd, b"viz") {
            // Visualize quantum results
            let Some(h) = Self::parse_u64(args) else {
                crate::println!("usage: viz <handle>");
                return;
            };
            match syscall::shell_result(h) {
                Ok((n00, n11)) => {
                    crate::qviz::draw_bell_result(n00, n11);
                }
                Err(st) => crate::println!("viz {} -> not ready (state={:?})", h, st),
            }
        } else if Self::eq(cmd, b"jobs") {
            crate::qviz::list_jobs();
        } else if Self::eq(cmd, b"cancel") {
            let Some(h) = Self::parse_u64(args) else {
                crate::println!("usage: cancel <handle>");
                return;
            };
            if syscall::shell_cancel(h) {
                crate::println!("cancelled {}", h);
            } else {
                crate::println!("cancel failed {}", h);
            }
        } else if Self::eq(cmd, b"qbackend") {
            // Quantum backend management
            // Usage: qbackend [list|info|set|status]
            let parts: alloc::vec::Vec<&str> = core::str::from_utf8(args)
                .unwrap_or("")
                .split_whitespace()
                .collect();
            
            let subcmd = parts.get(0).copied().unwrap_or("list");
            
            match subcmd {
                "list" | "ls" => {
                    crate::println!("╔═════════════════════════════════════════════════════════╗");
                    crate::println!("║              Available Quantum Backends                 ║");
                    crate::println!("╠═════════════════════════════════════════════════════════╣");
                    crate::println!("║ Name               │ Type    │ Qubits │ Status          ║");
                    crate::println!("╠═════════════════════════════════════════════════════════╣");
                    crate::println!("║ local_simulator  * │ local   │   32   │ available       ║");
                    crate::println!("║ ibm_quantum        │ remote  │  127   │ offline         ║");
                    crate::println!("║ google_cirq        │ remote  │   72   │ offline         ║");
                    crate::println!("║ ionq_harmony       │ remote  │   11   │ offline         ║");
                    crate::println!("╚═════════════════════════════════════════════════════════╝");
                    crate::println!("(* = default backend)");
                    crate::println!("");
                    crate::println!("Note: Remote backends require network connectivity.");
                }
                "info" => {
                    let backend = parts.get(1).copied().unwrap_or("local_simulator");
                    match backend {
                        "local_simulator" | "local" => {
                            crate::println!("╔═══════════════════════════════════════╗");
                            crate::println!("║      Local Quantum Simulator          ║");
                            crate::println!("╠═══════════════════════════════════════╣");
                            crate::println!("║ Type:           CPU Simulation        ║");
                            crate::println!("║ Max Qubits:     32 (memory limited)   ║");
                            crate::println!("║ Supported Gates: All standard gates   ║");
                            crate::println!("║ Connectivity:   Full (all-to-all)     ║");
                            crate::println!("║ Error Rate:     0.0 (perfect sim)     ║");
                            crate::println!("║ Status:         Available             ║");
                            crate::println!("╚═══════════════════════════════════════╝");
                        }
                        "ibm_quantum" | "ibm" => {
                            crate::println!("╔═══════════════════════════════════════╗");
                            crate::println!("║       IBM Quantum Network             ║");
                            crate::println!("╠═══════════════════════════════════════╣");
                            crate::println!("║ Type:           Superconducting QPU   ║");
                            crate::println!("║ Max Qubits:     127 (Eagle r3)        ║");
                            crate::println!("║ Native Gates:   CX, ID, RZ, SX, X     ║");
                            crate::println!("║ Connectivity:   Heavy-hex lattice     ║");
                            crate::println!("║ T1 Time:        ~300 μs               ║");
                            crate::println!("║ T2 Time:        ~150 μs               ║");
                            crate::println!("║ Gate Error:     ~0.3%                 ║");
                            crate::println!("║ Status:         Offline (no network)  ║");
                            crate::println!("╚═══════════════════════════════════════╝");
                            crate::println!("Connect via: TCP/IP + IBM Quantum API");
                        }
                        "google_cirq" | "google" => {
                            crate::println!("╔═══════════════════════════════════════╗");
                            crate::println!("║       Google Quantum AI               ║");
                            crate::println!("╠═══════════════════════════════════════╣");
                            crate::println!("║ Type:           Superconducting QPU   ║");
                            crate::println!("║ Max Qubits:     72 (Bristlecone)      ║");
                            crate::println!("║ Native Gates:   PHASED_XZ, FSIM, CZ   ║");
                            crate::println!("║ Connectivity:   2D grid               ║");
                            crate::println!("║ Gate Error:     ~0.5%                 ║");
                            crate::println!("║ Status:         Offline (no network)  ║");
                            crate::println!("╚═══════════════════════════════════════╝");
                        }
                        "ionq_harmony" | "ionq" => {
                            crate::println!("╔═══════════════════════════════════════╗");
                            crate::println!("║       IonQ Harmony                    ║");
                            crate::println!("╠═══════════════════════════════════════╣");
                            crate::println!("║ Type:           Trapped-ion QPU       ║");
                            crate::println!("║ Max Qubits:     11                    ║");
                            crate::println!("║ Native Gates:   GPI, GPI2, MS         ║");
                            crate::println!("║ Connectivity:   Full (all-to-all)     ║");
                            crate::println!("║ Gate Fidelity:  >99.5%                ║");
                            crate::println!("║ Status:         Offline (no network)  ║");
                            crate::println!("╚═══════════════════════════════════════╝");
                        }
                        _ => {
                            crate::println!("Unknown backend: {}", backend);
                            crate::println!("Available: local_simulator, ibm_quantum, google_cirq, ionq_harmony");
                        }
                    }
                }
                "set" => {
                    let backend = parts.get(1).copied().unwrap_or("");
                    if backend.is_empty() {
                        crate::println!("usage: qbackend set <backend_name>");
                        crate::println!("Available: local_simulator, ibm_quantum, google_cirq, ionq_harmony");
                    } else {
                        match backend {
                            "local_simulator" | "local" => {
                                crate::println!("Default backend set to: local_simulator");
                            }
                            "ibm_quantum" | "google_cirq" | "ionq_harmony" => {
                                crate::println!("Backend '{}' requires network connectivity.", backend);
                                crate::println!("Use 'net status' to check network status.");
                                crate::println!("Remote backends will be available when TCP/IP stack is connected.");
                            }
                            _ => {
                                crate::println!("Unknown backend: {}", backend);
                            }
                        }
                    }
                }
                "status" => {
                    crate::println!("Backend Status:");
                    crate::println!("  local_simulator:  AVAILABLE (ready)");
                    crate::println!("  ibm_quantum:      OFFLINE (no network)");
                    crate::println!("  google_cirq:      OFFLINE (no network)");
                    crate::println!("  ionq_harmony:     OFFLINE (no network)");
                    crate::println!("");
                    crate::println!("Current default: local_simulator");
                }
                "caps" | "capabilities" => {
                    crate::println!("Local Simulator Capabilities:");
                    crate::println!("  ✓ Hadamard (H)");
                    crate::println!("  ✓ Pauli X, Y, Z");
                    crate::println!("  ✓ CNOT (CX)");
                    crate::println!("  ✓ Toffoli (CCX)");
                    crate::println!("  ✓ SWAP");
                    crate::println!("  ✓ Phase gates (S, T, Rz)");
                    crate::println!("  ✓ Rotation gates (Rx, Ry)");
                    crate::println!("  ✓ Measurement");
                    crate::println!("  ✓ Custom unitary (U3)");
                    crate::println!("");
                    crate::println!("Simulation Features:");
                    crate::println!("  ✓ State vector simulation");
                    crate::println!("  ✓ Multi-shot sampling");
                    crate::println!("  ✓ Noiseless operation");
                    crate::println!("  ✓ Up to 32 qubits");
                }
                "help" | "?" => {
                    crate::println!("qbackend - Quantum Backend Management");
                    crate::println!("");
                    crate::println!("Usage: qbackend <command>");
                    crate::println!("");
                    crate::println!("Commands:");
                    crate::println!("  list       List all available backends");
                    crate::println!("  info <n>   Show detailed info for backend");
                    crate::println!("  set <n>    Set default backend");
                    crate::println!("  status     Show status of all backends");
                    crate::println!("  caps       Show local simulator capabilities");
                    crate::println!("  help       Show this help");
                    crate::println!("");
                    crate::println!("Examples:");
                    crate::println!("  qbackend list");
                    crate::println!("  qbackend info ibm_quantum");
                    crate::println!("  qbackend set local_simulator");
                }
                _ => {
                    crate::println!("Unknown subcommand: {}", subcmd);
                    crate::println!("Use 'qbackend help' for usage");
                }
            }
        } else if Self::eq(cmd, b"qsim") {
            // Interactive quantum simulator
            // Usage: qsim [qubits]  - default 2 qubits
            let n_qubits = Self::parse_u64(args).unwrap_or(2) as usize;
            if n_qubits > 8 {
                crate::println!("qsim: max 8 qubits (memory limit)");
                return;
            }
            crate::println!("╔══════════════════════════════════════╗");
            crate::println!("║     Quantum Simulator ({} qubits)     ║", n_qubits);
            crate::println!("╚══════════════════════════════════════╝");
            crate::println!("Commands: h N, x N, cx C T, measure, reset, state, quit");
            
            use crate::quantum::{QuantumState, Circuit, Gate};
            let mut qstate = QuantumState::new(n_qubits);
            let mut qcircuit = Circuit::new(n_qubits, n_qubits);
            
            // Use a local keyboard decoder
            let mut qsim_kb = Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::Ignore);
            
            let mut qsim_line = [0u8; 64];
            let mut qsim_len = 0usize;
            
            'qsim_loop: loop {
                crate::print!("qsim> ");
                qsim_len = 0;
                
                // Read a line
                'read_line: loop {
                    if let Some(sc) = keyboard::pop_scancode() {
                        if let Ok(Some(event)) = qsim_kb.add_byte(sc) {
                            if let Some(key) = qsim_kb.process_keyevent(event) {
                                match key {
                                    DecodedKey::Unicode('\n') => {
                                        crate::println!("");
                                        break 'read_line;
                                    }
                                    DecodedKey::Unicode('\u{0008}') if qsim_len > 0 => {
                                        qsim_len -= 1;
                                        crate::print!("\x08 \x08");
                                    }
                                    DecodedKey::Unicode(c) if c.is_ascii() && qsim_len < 63 => {
                                        qsim_line[qsim_len] = c as u8;
                                        qsim_len += 1;
                                        crate::print!("{}", c);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    core::hint::spin_loop();
                }
                
                let qcmd = &qsim_line[..qsim_len];
                let qcmd_str = core::str::from_utf8(qcmd).unwrap_or("");
                let parts: alloc::vec::Vec<&str> = qcmd_str.split_whitespace().collect();
                
                if parts.is_empty() { continue; }
                
                match parts[0] {
                    "quit" | "exit" | "q" => break 'qsim_loop,
                    "h" => {
                        if let Some(q) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) {
                            if q < n_qubits {
                                qstate.apply_h(q);
                                qcircuit.add(Gate::H(q));
                                crate::println!("Applied H to q{}", q);
                            } else {
                                crate::println!("Invalid qubit {}", q);
                            }
                        } else {
                            crate::println!("usage: h <qubit>");
                        }
                    }
                    "x" => {
                        if let Some(q) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) {
                            if q < n_qubits {
                                qstate.apply_x(q);
                                qcircuit.add(Gate::X(q));
                                crate::println!("Applied X to q{}", q);
                            } else {
                                crate::println!("Invalid qubit {}", q);
                            }
                        } else {
                            crate::println!("usage: x <qubit>");
                        }
                    }
                    "y" => {
                        if let Some(q) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) {
                            if q < n_qubits {
                                qstate.apply_y(q);
                                qcircuit.add(Gate::Y(q));
                                crate::println!("Applied Y to q{}", q);
                            } else {
                                crate::println!("Invalid qubit {}", q);
                            }
                        } else {
                            crate::println!("usage: y <qubit>");
                        }
                    }
                    "z" => {
                        if let Some(q) = parts.get(1).and_then(|s| s.parse::<usize>().ok()) {
                            if q < n_qubits {
                                qstate.apply_z(q);
                                qcircuit.add(Gate::Z(q));
                                crate::println!("Applied Z to q{}", q);
                            } else {
                                crate::println!("Invalid qubit {}", q);
                            }
                        } else {
                            crate::println!("usage: z <qubit>");
                        }
                    }
                    "cx" | "cnot" => {
                        if let (Some(c), Some(t)) = (
                            parts.get(1).and_then(|s| s.parse::<usize>().ok()),
                            parts.get(2).and_then(|s| s.parse::<usize>().ok())
                        ) {
                            if c < n_qubits && t < n_qubits && c != t {
                                qstate.apply_cx(c, t);
                                qcircuit.add(Gate::Cx(c, t));
                                crate::println!("Applied CNOT q{} -> q{}", c, t);
                            } else {
                                crate::println!("Invalid qubits {} {}", c, t);
                            }
                        } else {
                            crate::println!("usage: cx <control> <target>");
                        }
                    }
                    "measure" | "m" => {
                        crate::println!("Measurement results:");
                        for q in 0..n_qubits {
                            let result = qstate.measure_qubit(q);
                            crate::println!("  q{} = {}", q, result);
                        }
                    }
                    "reset" | "r" => {
                        qstate.reset();
                        qcircuit = Circuit::new(n_qubits, n_qubits);
                        crate::println!("State reset to |0...0>");
                    }
                    "state" | "s" => {
                        crate::println!("Current state (non-zero amplitudes):");
                        for (i, amp) in qstate.amplitudes.iter().enumerate() {
                            let prob = amp.norm_sq();
                            if prob > 0.001 {
                                let bits: alloc::string::String = (0..n_qubits)
                                    .rev()
                                    .map(|b| if (i >> b) & 1 == 1 { '1' } else { '0' })
                                    .collect();
                                crate::println!("  |{}> : {:.4} + {:.4}i (prob={:.2}%)", 
                                    bits, amp.re, amp.im, prob * 100.0);
                            }
                        }
                    }
                    "bell" => {
                        if n_qubits >= 2 {
                            qstate.reset();
                            qstate.apply_h(0);
                            qstate.apply_cx(0, 1);
                            qcircuit = Circuit::new(n_qubits, n_qubits);
                            qcircuit.add(Gate::H(0));
                            qcircuit.add(Gate::Cx(0, 1));
                            crate::println!("Created Bell state |00> + |11>");
                        } else {
                            crate::println!("Need at least 2 qubits for Bell state");
                        }
                    }
                    "help" | "?" => {
                        crate::println!("Commands:");
                        crate::println!("  h N       - Hadamard gate on qubit N");
                        crate::println!("  x N       - Pauli-X gate on qubit N");
                        crate::println!("  y N       - Pauli-Y gate on qubit N");
                        crate::println!("  z N       - Pauli-Z gate on qubit N");
                        crate::println!("  cx C T    - CNOT with control C, target T");
                        crate::println!("  measure   - Measure all qubits");
                        crate::println!("  state     - Show state amplitudes");
                        crate::println!("  bell      - Create Bell state");
                        crate::println!("  reset     - Reset to |0...0>");
                        crate::println!("  quit      - Exit simulator");
                    }
                    _ => crate::println!("Unknown command. Type 'help' for commands."),
                }
            }
            crate::println!("Exited quantum simulator");
        } else if Self::eq(cmd, b"qsubmit") {
            // Submit inline QASM code
            let trimmed = Self::trim_whitespace(args);
            if trimmed.is_empty() {
                crate::println!("usage: qsubmit <inline-qasm>");
                crate::println!("  e.g.: qsubmit h q[0]; cx q[0],q[1]; measure q -> c;");
                return;
            }
            // Wrap in minimal QASM header
            let qasm_str = core::str::from_utf8(trimmed).unwrap_or("");
            let full_qasm = alloc::format!(
                "OPENQASM 2.0;\ninclude \"qelib1.inc\";\nqreg q[4];\ncreg c[4];\n{}\n",
                qasm_str
            );
            let n_qubits = crate::quantum::count_qubits_from_qasm(&full_qasm);
            match syscall::shell_submit_ir_qasm2(full_qasm.as_bytes(), 1024, n_qubits) {
                Some(h) => crate::println!("submitted handle={}", h),
                None => crate::println!("submit failed"),
            }
        } else if Self::eq(cmd, b"edit") {
            let Some((path, _rest)) = Self::parse_word(args) else {
                crate::println!("usage: edit <path>");
                return;
            };
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("edit: bad path");
                return;
            };
            self.start_editor(&buf[..n]);
        }
        // FAT16 commands (optional, behind 'fat' feature flag)
        #[cfg(feature = "fat")]
        {
            if Self::eq(cmd, b"fatls") {
                if !crate::fat16::is_fat16() {
                    crate::println!("error: FAT16 filesystem not available");
                    return;
                }
                crate::fat16::list();
                return;
            }
            if Self::eq(cmd, b"fatcat") {
                let Some((name, _)) = Self::parse_word(args) else {
                    crate::println!("usage: fatcat <file>");
                    return;
                };
                if !crate::fat16::is_fat16() {
                    crate::println!("error: FAT16 filesystem not available");
                    return;
                }
                match crate::fat16::read(name) {
                    Some(bytes) => {
                        let s = core::str::from_utf8(&bytes).unwrap_or("<non-utf8>");
                        crate::println!("{}", s);
                    }
                    None => crate::println!("error: file not found"),
                }
                return;
            }
            if Self::eq(cmd, b"fatwrite") {
                let Some((name, rest)) = Self::parse_word(args) else {
                    crate::println!("usage: fatwrite <file> <text>");
                    return;
                };
                let mut text = rest;
                while let Some((&b, r)) = text.split_first() {
                    if b == b' ' || b == b'\t' {
                        text = r;
                    } else {
                        break;
                    }
                }
                if text.is_empty() {
                    crate::println!("usage: fatwrite <file> <text>");
                    return;
                }
                if !crate::fat16::is_fat16() {
                    crate::println!("error: FAT16 filesystem not available");
                    return;
                }
                let name_str = core::str::from_utf8(name).unwrap_or("");
                match crate::fat16::write(name_str.as_bytes(), text) {
                    Ok(()) => crate::println!("wrote {} bytes to {}", text.len(), name_str),
                    Err(e) => crate::println!("error: {}", e),
                }
                return;
            }
            if Self::eq(cmd, b"fatrm") {
                let Some((name, _)) = Self::parse_word(args) else {
                    crate::println!("usage: fatrm <file>");
                    return;
                };
                if !crate::fat16::is_fat16() {
                    crate::println!("error: FAT16 filesystem not available");
                    return;
                }
                if crate::fat16::remove(name) {
                    crate::println!("removed");
                } else {
                    crate::println!("error: file not found");
                }
                return;
            }
        }
        
        // ==================== ADDITIONAL COMMANDS ====================
        
        // echo command - print arguments or stdin
        if Self::eq(cmd, b"echo") {
            // Trim leading whitespace from args
            let mut text = args;
            while let Some((&b, rest)) = text.split_first() {
                if b == b' ' || b == b'\t' {
                    text = rest;
                } else {
                    break;
                }
            }
            
            if text.is_empty() {
                // If stdin is provided (via pipe), print it
                if let Some(input) = stdin {
                    let s = core::str::from_utf8(input).unwrap_or("<non-utf8>");
                    crate::print!("{}", s);
                } else {
                    crate::println!();
                }
            } else {
                let s = core::str::from_utf8(text).unwrap_or("<non-utf8>");
                crate::println!("{}", s);
            }
            return;
        }
        
        // grep command - filter lines containing pattern
        if Self::eq(cmd, b"grep") {
            let Some((pattern, rest)) = Self::parse_word(args) else {
                crate::println!("usage: grep <pattern> [file]");
                return;
            };
            
            // Get input - either from file or stdin
            let input_data = if let Some((file, _)) = Self::parse_word(rest) {
                // Read from file
                let mut path_buf = [0u8; 96];
                let Some(path_len) = self.resolve_path(file, &mut path_buf) else {
                    crate::println!("grep: invalid path");
                    return;
                };
                match vfs::read(&path_buf[..path_len]) {
                    Ok(data) => data,
                    Err(e) => {
                        crate::println!("grep: {:?}", e);
                        return;
                    }
                }
            } else if let Some(input) = stdin {
                input.to_vec()
            } else {
                crate::println!("usage: grep <pattern> [file]");
                return;
            };
            
            let pattern_str = core::str::from_utf8(pattern).unwrap_or("");
            let input_str = core::str::from_utf8(&input_data).unwrap_or("");
            
            for line in input_str.lines() {
                if line.contains(pattern_str) {
                    crate::println!("{}", line);
                }
            }
            return;
        }
        
        // wc command - word/line/char count
        if Self::eq(cmd, b"wc") {
            let input_data = if let Some((file, _)) = Self::parse_word(args) {
                // Read from file
                let mut path_buf = [0u8; 96];
                let Some(path_len) = self.resolve_path(file, &mut path_buf) else {
                    crate::println!("wc: invalid path");
                    return;
                };
                match vfs::read(&path_buf[..path_len]) {
                    Ok(data) => data,
                    Err(e) => {
                        crate::println!("wc: {:?}", e);
                        return;
                    }
                }
            } else if let Some(input) = stdin {
                input.to_vec()
            } else {
                crate::println!("usage: wc [file]");
                return;
            };
            
            let input_str = core::str::from_utf8(&input_data).unwrap_or("");
            let lines = input_str.lines().count();
            let words = input_str.split_whitespace().count();
            let chars = input_data.len();
            
            crate::println!("{} {} {}", lines, words, chars);
            return;
        }
        
        // head command - first N lines
        if Self::eq(cmd, b"head") {
            let mut n_lines = 10usize;
            let mut file_arg: Option<&[u8]> = None;
            
            // Parse -n option
            if let Some((arg1, rest)) = Self::parse_word(args) {
                if arg1.starts_with(b"-n") {
                    if arg1.len() > 2 {
                        // -n10 format
                        if let Some(n) = Self::parse_u64(&arg1[2..]) {
                            n_lines = n as usize;
                        }
                    } else if let Some((num, rest2)) = Self::parse_word(rest) {
                        // -n 10 format
                        if let Some(n) = Self::parse_u64(num) {
                            n_lines = n as usize;
                        }
                        if let Some((f, _)) = Self::parse_word(rest2) {
                            file_arg = Some(f);
                        }
                    }
                } else if arg1 == b"-" {
                    // stdin
                } else {
                    file_arg = Some(arg1);
                }
            }
            
            let input_data = if let Some(file) = file_arg {
                let mut path_buf = [0u8; 96];
                let Some(path_len) = self.resolve_path(file, &mut path_buf) else {
                    crate::println!("head: invalid path");
                    return;
                };
                match vfs::read(&path_buf[..path_len]) {
                    Ok(data) => data,
                    Err(e) => {
                        crate::println!("head: {:?}", e);
                        return;
                    }
                }
            } else if let Some(input) = stdin {
                input.to_vec()
            } else {
                crate::println!("usage: head [-n N] [file]");
                return;
            };
            
            let input_str = core::str::from_utf8(&input_data).unwrap_or("");
            for (i, line) in input_str.lines().enumerate() {
                if i >= n_lines {
                    break;
                }
                crate::println!("{}", line);
            }
            return;
        }
        
        // tail command - last N lines
        if Self::eq(cmd, b"tail") {
            let mut n_lines = 10usize;
            let mut file_arg: Option<&[u8]> = None;
            
            // Parse -n option
            if let Some((arg1, rest)) = Self::parse_word(args) {
                if arg1.starts_with(b"-n") {
                    if arg1.len() > 2 {
                        if let Some(n) = Self::parse_u64(&arg1[2..]) {
                            n_lines = n as usize;
                        }
                    } else if let Some((num, rest2)) = Self::parse_word(rest) {
                        if let Some(n) = Self::parse_u64(num) {
                            n_lines = n as usize;
                        }
                        if let Some((f, _)) = Self::parse_word(rest2) {
                            file_arg = Some(f);
                        }
                    }
                } else {
                    file_arg = Some(arg1);
                }
            }
            
            let input_data = if let Some(file) = file_arg {
                let mut path_buf = [0u8; 96];
                let Some(path_len) = self.resolve_path(file, &mut path_buf) else {
                    crate::println!("tail: invalid path");
                    return;
                };
                match vfs::read(&path_buf[..path_len]) {
                    Ok(data) => data,
                    Err(e) => {
                        crate::println!("tail: {:?}", e);
                        return;
                    }
                }
            } else if let Some(input) = stdin {
                input.to_vec()
            } else {
                crate::println!("usage: tail [-n N] [file]");
                return;
            };
            
            let input_str = core::str::from_utf8(&input_data).unwrap_or("");
            let lines: alloc::vec::Vec<&str> = input_str.lines().collect();
            let start = lines.len().saturating_sub(n_lines);
            for line in &lines[start..] {
                crate::println!("{}", line);
            }
            return;
        }
        
        // sort command - sort lines
        if Self::eq(cmd, b"sort") {
            let input_data = if let Some((file, _)) = Self::parse_word(args) {
                let mut path_buf = [0u8; 96];
                let Some(path_len) = self.resolve_path(file, &mut path_buf) else {
                    crate::println!("sort: invalid path");
                    return;
                };
                match vfs::read(&path_buf[..path_len]) {
                    Ok(data) => data,
                    Err(e) => {
                        crate::println!("sort: {:?}", e);
                        return;
                    }
                }
            } else if let Some(input) = stdin {
                input.to_vec()
            } else {
                crate::println!("usage: sort [file]");
                return;
            };
            
            let input_str = core::str::from_utf8(&input_data).unwrap_or("");
            let mut lines: alloc::vec::Vec<&str> = input_str.lines().collect();
            lines.sort();
            for line in lines {
                crate::println!("{}", line);
            }
            return;
        }
        
        // uniq command - remove duplicate consecutive lines
        if Self::eq(cmd, b"uniq") {
            let input_data = if let Some((file, _)) = Self::parse_word(args) {
                let mut path_buf = [0u8; 96];
                let Some(path_len) = self.resolve_path(file, &mut path_buf) else {
                    crate::println!("uniq: invalid path");
                    return;
                };
                match vfs::read(&path_buf[..path_len]) {
                    Ok(data) => data,
                    Err(e) => {
                        crate::println!("uniq: {:?}", e);
                        return;
                    }
                }
            } else if let Some(input) = stdin {
                input.to_vec()
            } else {
                crate::println!("usage: uniq [file]");
                return;
            };
            
            let input_str = core::str::from_utf8(&input_data).unwrap_or("");
            let mut prev: Option<&str> = None;
            for line in input_str.lines() {
                if prev != Some(line) {
                    crate::println!("{}", line);
                    prev = Some(line);
                }
            }
            return;
        }
        
        // ==================== ENVIRONMENT & ALIAS COMMANDS ====================
        
        // env command - list all environment variables
        if Self::eq(cmd, b"env") {
            let vars = env_list();
            if vars.is_empty() {
                crate::println!("(no environment variables)");
            } else {
                for (k, v) in vars {
                    crate::println!("{}={}", k, v);
                }
            }
            return;
        }
        
        // export command - set environment variable
        // usage: export VAR=value or export VAR value
        if Self::eq(cmd, b"export") {
            let trimmed = Self::trim_whitespace(args);
            if trimmed.is_empty() {
                // List all like env
                let vars = env_list();
                for (k, v) in vars {
                    crate::println!("export {}={}", k, v);
                }
                return;
            }
            
            // Check for VAR=value format
            if let Some(eq_pos) = trimmed.iter().position(|&b| b == b'=') {
                let var_name = &trimmed[..eq_pos];
                let var_value = &trimmed[eq_pos + 1..];
                let name_str = core::str::from_utf8(var_name).unwrap_or("");
                let value_str = core::str::from_utf8(var_value).unwrap_or("");
                env_set(name_str, value_str);
                crate::println!("{}={}", name_str, value_str);
            } else {
                // export VAR value format
                if let Some((var_name, rest)) = Self::parse_word(trimmed) {
                    let value = Self::trim_whitespace(rest);
                    let name_str = core::str::from_utf8(var_name).unwrap_or("");
                    let value_str = core::str::from_utf8(value).unwrap_or("");
                    if value.is_empty() {
                        crate::println!("usage: export VAR=value or export VAR value");
                    } else {
                        env_set(name_str, value_str);
                        crate::println!("{}={}", name_str, value_str);
                    }
                }
            }
            return;
        }
        
        // unset command - remove environment variable
        if Self::eq(cmd, b"unset") {
            let Some((var_name, _)) = Self::parse_word(args) else {
                crate::println!("usage: unset VAR");
                return;
            };
            let name_str = core::str::from_utf8(var_name).unwrap_or("");
            if env_unset(name_str) {
                crate::println!("unset {}", name_str);
            } else {
                crate::println!("unset: {} not found", name_str);
            }
            return;
        }
        
        // alias command - list or set aliases
        // usage: alias (list all), alias name (show one), alias name=value (set)
        if Self::eq(cmd, b"alias") {
            let trimmed = Self::trim_whitespace(args);
            if trimmed.is_empty() {
                // List all aliases
                let aliases = alias_list();
                if aliases.is_empty() {
                    crate::println!("(no aliases defined)");
                } else {
                    for (k, v) in aliases {
                        crate::println!("alias {}='{}'", k, v);
                    }
                }
                return;
            }
            
            // Check for name=value format
            if let Some(eq_pos) = trimmed.iter().position(|&b| b == b'=') {
                let alias_name = &trimmed[..eq_pos];
                let alias_value = &trimmed[eq_pos + 1..];
                let name_str = core::str::from_utf8(alias_name).unwrap_or("");
                // Remove surrounding quotes from value if present
                let value_str = core::str::from_utf8(alias_value).unwrap_or("");
                let value_str = value_str.trim_matches('\'').trim_matches('"');
                alias_set(name_str, value_str);
                crate::println!("alias {}='{}'", name_str, value_str);
            } else {
                // Show specific alias
                let name_str = core::str::from_utf8(trimmed).unwrap_or("");
                if let Some(val) = alias_get(name_str) {
                    crate::println!("alias {}='{}'", name_str, val);
                } else {
                    crate::println!("alias: {} not found", name_str);
                }
            }
            return;
        }
        
        // unalias command - remove alias
        if Self::eq(cmd, b"unalias") {
            let Some((alias_name, _)) = Self::parse_word(args) else {
                crate::println!("usage: unalias name");
                return;
            };
            let name_str = core::str::from_utf8(alias_name).unwrap_or("");
            if alias_unset(name_str) {
                crate::println!("unalias {}", name_str);
            } else {
                crate::println!("unalias: {} not found", name_str);
            }
            return;
        }
        
        // source/run command - execute script file
        if Self::eq(cmd, b"source") || Self::eq(cmd, b"run") {
            let Some((script_path, _)) = Self::parse_word(args) else {
                crate::println!("usage: source <script.qsh>");
                return;
            };
            
            let mut path_buf = [0u8; 96];
            let Some(path_len) = self.resolve_path(script_path, &mut path_buf) else {
                crate::println!("source: invalid path");
                return;
            };
            
            match vfs::read(&path_buf[..path_len]) {
                Ok(data) => {
                    let script = core::str::from_utf8(&data).unwrap_or("");
                    crate::println!("Executing script...");
                    
                    // Execute each line as a command
                    for line in script.lines() {
                        let line = line.trim();
                        // Skip empty lines and comments
                        if line.is_empty() || line.starts_with('#') {
                            continue;
                        }
                        // Echo command being executed
                        crate::println!("> {}", line);
                        // Run the command
                        self.run_command(line.as_bytes());
                    }
                    crate::println!("Script completed.");
                }
                Err(e) => {
                    crate::println!("source: {:?}", e);
                }
            }
            return;
        }
        
        // ==================== FILESYSTEM COMMANDS ====================
        
        // df command - show filesystem usage
        if Self::eq(cmd, b"df") {
            let used = crate::fs::used_space();
            let total = crate::fs::total_capacity();
            let entries_used = crate::fs::used_entries();
            let entries_free = crate::fs::free_entries();
            
            crate::println!("Filesystem       Used      Avail   Use%  Entries");
            crate::println!("/ram       {:>10}  {:>10}   {:>3}%   {}/{}", 
                Self::format_size(used),
                Self::format_size(total - used),
                if total > 0 { (used * 100) / total } else { 0 },
                entries_used,
                entries_used + entries_free
            );
            return;
        }
        
        // du command - show directory size
        if Self::eq(cmd, b"du") {
            let path = if let Some((p, _)) = Self::parse_word(args) {
                let mut buf = [0u8; 96];
                if let Some(n) = self.resolve_path(p, &mut buf) {
                    let path_str = core::str::from_utf8(&buf[..n]).unwrap_or("");
                    // Get size for VFS path
                    if path_str.starts_with("/ram") {
                        let ram_path = if path_str == "/ram" { b"" as &[u8] } else { &buf[5..n] };
                        let size = crate::fs::dir_size(ram_path);
                        crate::println!("{}  {}", Self::format_size(size), path_str);
                    } else {
                        crate::println!("du: only /ram supported");
                    }
                } else {
                    crate::println!("du: invalid path");
                }
            } else {
                // Current directory
                let cwd = self.cwd_bytes();
                let ram_path = if cwd == b"/ram" { b"" as &[u8] } else if cwd.starts_with(b"/ram/") { &cwd[5..] } else { b"" };
                let size = crate::fs::dir_size(ram_path);
                crate::println!("{}  .", Self::format_size(size));
            };
            return;
        }
        
        // stat command - show file metadata
        if Self::eq(cmd, b"stat") {
            let Some((path, _)) = Self::parse_word(args) else {
                crate::println!("usage: stat <path>");
                return;
            };
            
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("stat: invalid path");
                return;
            };
            
            let path_str = core::str::from_utf8(&buf[..n]).unwrap_or("?");
            
            // Handle VFS path
            if path_str.starts_with("/ram") {
                let ram_path = if path_str == "/ram" { b"" as &[u8] } else { &buf[5..n] };
                if let Some((etype, size, meta)) = crate::fs::get_metadata(ram_path) {
                    crate::println!("  File: {}", path_str);
                    crate::println!("  Type: {:?}", etype);
                    crate::println!("  Size: {} bytes", size);
                    crate::println!("  Mode: {:04o}", meta.permissions);
                    crate::println!("  Created: {}", Self::format_timestamp(meta.created));
                    crate::println!("  Modified: {}", Self::format_timestamp(meta.modified));
                    crate::println!("  Accessed: {}", Self::format_timestamp(meta.accessed));
                } else {
                    crate::println!("stat: not found");
                }
            } else {
                crate::println!("stat: only /ram paths supported");
            }
            return;
        }
        
        // mv command - move/rename file
        if Self::eq(cmd, b"mv") {
            let Some((src, rest)) = Self::parse_word(args) else {
                crate::println!("usage: mv <src> <dst>");
                return;
            };
            let Some((dst, _)) = Self::parse_word(rest) else {
                crate::println!("usage: mv <src> <dst>");
                return;
            };
            
            let mut src_buf = [0u8; 96];
            let mut dst_buf = [0u8; 96];
            
            let Some(src_n) = self.resolve_path(src, &mut src_buf) else {
                crate::println!("mv: invalid source path");
                return;
            };
            let Some(dst_n) = self.resolve_path(dst, &mut dst_buf) else {
                crate::println!("mv: invalid dest path");
                return;
            };
            
            // Only support /ram paths for now
            let src_str = core::str::from_utf8(&src_buf[..src_n]).unwrap_or("");
            let dst_str = core::str::from_utf8(&dst_buf[..dst_n]).unwrap_or("");
            
            if !src_str.starts_with("/ram/") || !dst_str.starts_with("/ram/") {
                crate::println!("mv: only /ram paths supported");
                return;
            }
            
            let src_ram = &src_buf[5..src_n];
            let dst_ram = &dst_buf[5..dst_n];
            
            match crate::fs::rename(src_ram, dst_ram) {
                Ok(()) => crate::println!("ok"),
                Err(e) => crate::println!("mv: {}", e),
            }
            return;
        }
        
        // cp command - copy file  
        if Self::eq(cmd, b"cp") {
            let Some((src, rest)) = Self::parse_word(args) else {
                crate::println!("usage: cp <src> <dst>");
                return;
            };
            let Some((dst, _)) = Self::parse_word(rest) else {
                crate::println!("usage: cp <src> <dst>");
                return;
            };
            
            let mut src_buf = [0u8; 96];
            let mut dst_buf = [0u8; 96];
            
            let Some(src_n) = self.resolve_path(src, &mut src_buf) else {
                crate::println!("cp: invalid source path");
                return;
            };
            let Some(dst_n) = self.resolve_path(dst, &mut dst_buf) else {
                crate::println!("cp: invalid dest path");
                return;
            };
            
            let src_str = core::str::from_utf8(&src_buf[..src_n]).unwrap_or("");
            let dst_str = core::str::from_utf8(&dst_buf[..dst_n]).unwrap_or("");
            
            if !src_str.starts_with("/ram/") || !dst_str.starts_with("/ram/") {
                crate::println!("cp: only /ram paths supported");
                return;
            }
            
            let src_ram = &src_buf[5..src_n];
            let dst_ram = &dst_buf[5..dst_n];
            
            match crate::fs::copy(src_ram, dst_ram) {
                Ok(()) => crate::println!("ok"),
                Err(e) => crate::println!("cp: {}", e),
            }
            return;
        }
        
        // chmod command - change file permissions
        if Self::eq(cmd, b"chmod") {
            let Some((mode_str, rest)) = Self::parse_word(args) else {
                crate::println!("usage: chmod <mode> <path>");
                crate::println!("  mode: octal (e.g., 755, 644)");
                return;
            };
            let Some((path, _)) = Self::parse_word(rest) else {
                crate::println!("usage: chmod <mode> <path>");
                return;
            };
            
            // Parse octal mode
            let mode_s = core::str::from_utf8(mode_str).unwrap_or("0");
            let mode = u16::from_str_radix(mode_s, 8).unwrap_or(0o644);
            
            let mut buf = [0u8; 96];
            let Some(n) = self.resolve_path(path, &mut buf) else {
                crate::println!("chmod: invalid path");
                return;
            };
            
            let path_str = core::str::from_utf8(&buf[..n]).unwrap_or("");
            if !path_str.starts_with("/ram/") {
                crate::println!("chmod: only /ram paths supported");
                return;
            }
            
            let ram_path = &buf[5..n];
            match crate::fs::chmod(ram_path, mode) {
                Ok(()) => crate::println!("ok"),
                Err(e) => crate::println!("chmod: {}", e),
            }
            return;
        }
        
        // ll command - detailed ls
        if Self::eq(cmd, b"ll") {
            let dir = if let Some((p, _)) = Self::parse_word(args) {
                p
            } else {
                self.cwd_bytes()
            };
            
            let mut buf = [0u8; 96];
            let resolved = if dir.starts_with(b"/") {
                Self::trim_trailing_slash(dir)
            } else {
                let Some(n) = self.resolve_path(dir, &mut buf) else {
                    crate::println!("ll: bad path");
                    return;
                };
                &buf[..n]
            };
            
            // Handle /ram paths
            let path_str = core::str::from_utf8(resolved).unwrap_or("");
            if path_str.starts_with("/ram") {
                let ram_path = if path_str == "/ram" { b"" as &[u8] } else { &resolved[5..] };
                let entries = crate::fs::get_entries_detailed(ram_path);
                
                crate::println!("Mode    Size      Modified            Name");
                crate::println!("──────────────────────────────────────────────");
                
                for (name, etype, size, meta) in entries {
                    let type_char = match etype {
                        crate::fs::EntryType::Directory => 'd',
                        crate::fs::EntryType::File => '-',
                    };
                    let perms = Self::format_perms(meta.permissions);
                    let time_str = Self::format_timestamp(meta.modified);
                    
                    if etype == crate::fs::EntryType::Directory {
                        crate::println!("{}{} {:>8}  {}  {}/", type_char, perms, "-", time_str, name);
                    } else {
                        crate::println!("{}{} {:>8}  {}  {}", type_char, perms, size, time_str, name);
                    }
                }
            } else if path_str == "/" {
                crate::println!("drwxr-xr-x        -  -                   ram/");
                crate::println!("drwxr-xr-x        -  -                   disk/");
            } else {
                crate::println!("ll: path not supported");
            }
            return;
        }
        
        // ==================== END FILESYSTEM COMMANDS ====================
        
        // ==================== NETWORK COMMANDS ====================
        
        // ifconfig command - show network interface config
        if Self::eq(cmd, b"ifconfig") {
            crate::println!("Network Interfaces:");
            crate::println!("───────────────────────────────────────");
            
            if crate::e1000::is_available() {
                let mac = crate::e1000::mac_addr().unwrap_or(crate::net::MacAddr::ZERO);
                let link = if crate::e1000::is_link_up() { "UP" } else { "DOWN" };
                
                crate::println!("eth0:");
                crate::println!("  HWaddr: {}", mac);
                crate::println!("  Link: {}", link);
                
                // Get IP from net module
                let cfg = crate::net::config();
                crate::println!("  IPv4: {}", cfg.ip);
                crate::println!("  Gateway: {}", cfg.gateway);
                crate::println!("  RX pending: {} packets", crate::e1000::rx_pending());
            } else {
                crate::println!("eth0: No network adapter found");
                crate::println!("  (run QEMU with -netdev and -device e1000)");
            }
            return;
        }
        
        // ping command - send ICMP echo
        if Self::eq(cmd, b"ping") {
            let Some((target, _)) = Self::parse_word(args) else {
                crate::println!("usage: ping <ip>");
                crate::println!("  e.g., ping 10.0.2.2");
                return;
            };
            
            // Parse IP address
            let target_str = core::str::from_utf8(target).unwrap_or("0.0.0.0");
            let parts: Vec<&str> = target_str.split('.').collect();
            if parts.len() != 4 {
                crate::println!("ping: invalid IP format");
                return;
            }
            
            let ip = crate::net::Ipv4Addr::new(
                parts[0].parse().unwrap_or(0),
                parts[1].parse().unwrap_or(0),
                parts[2].parse().unwrap_or(0),
                parts[3].parse().unwrap_or(0),
            );
            
            if !crate::e1000::is_available() {
                crate::println!("ping: no network interface");
                return;
            }
            
            crate::println!("PING {} - sending ICMP echo request", ip);
            
            // Create and send ping packet
            let packet = crate::net::create_ping(ip, 1);
            match crate::e1000::send(&packet) {
                Ok(()) => crate::println!("Sent {} bytes", packet.len()),
                Err(e) => crate::println!("ping: send failed: {}", e),
            }
            
            // Poll for response (simple blocking wait)
            crate::println!("Waiting for reply...");
            for _ in 0..100000 {
                if let Some(resp) = crate::e1000::recv() {
                    crate::println!("Received {} bytes", resp.len());
                    // Parse ICMP reply (simplified)
                    if resp.len() >= 34 {
                        let icmp_type = resp[34];
                        if icmp_type == 0 {
                            crate::println!("Reply from {}: ICMP echo reply", ip);
                        }
                    }
                    break;
                }
            }
            return;
        }
        
        // arp command - show ARP table
        if Self::eq(cmd, b"arp") {
            crate::println!("ARP Table:");
            crate::println!("IP Address       MAC Address");
            crate::println!("──────────────────────────────────────");
            // ARP table'ı göster (şimdilik boş)
            crate::println!("(empty)");
            return;
        }
        
        // netstat command - network statistics
        if Self::eq(cmd, b"netstat") {
            crate::println!("Network Statistics:");
            crate::println!("───────────────────────────────────────");
            if crate::e1000::is_available() {
                crate::println!("  Adapter: Intel E1000");
                crate::println!("  Link: {}", if crate::e1000::is_link_up() { "UP" } else { "DOWN" });
                crate::println!("  RX Queue: {} packets", crate::e1000::rx_pending());
                
                // Show UDP sockets
                let udp_ports = crate::net::udp_ports();
                if !udp_ports.is_empty() {
                    crate::println!("\nUDP Sockets:");
                    for port in udp_ports {
                        crate::println!("  0.0.0.0:{}", port);
                    }
                }
                
                // Show TCP sockets
                let tcp_sockets = crate::net::tcp_sockets();
                if !tcp_sockets.is_empty() {
                    crate::println!("\nTCP Sockets:");
                    for (port, state, remote) in tcp_sockets {
                        if let Some((ip, rport)) = remote {
                            crate::println!("  :{} -> {}:{} [{:?}]", port, ip, rport, state);
                        } else {
                            crate::println!("  :{} [{:?}]", port, state);
                        }
                    }
                }
            } else {
                crate::println!("  No network adapter");
            }
            return;
        }
        
        // dhcp command - DHCP client
        if Self::eq(cmd, b"dhcp") {
            if !crate::e1000::is_available() {
                crate::println!("Error: No network adapter");
                return;
            }
            
            let state = crate::net::dhcp_state();
            if state == crate::net::DhcpState::Bound {
                crate::println!("DHCP: Already configured");
                crate::net::show_info();
                return;
            }
            
            crate::println!("Starting DHCP discovery...");
            match crate::net::dhcp_discover() {
                Ok(()) => {
                    // Wait for response with visible progress
                    crate::println!("Waiting for DHCP response (max 10s)...");
                    let mut ticks = 0;
                    for i in 0..100 {
                        // Show progress every 10 iterations
                        if i % 10 == 0 {
                            crate::print!(".");
                        }
                        
                        crate::e1000::poll();
                        // Process received frames
                        let mut processed = 0;
                        while let Some(frame) = crate::e1000::recv() {
                            crate::net::process_frame(&frame);
                            processed += 1;
                            if processed > 10 {
                                break; // Don't process too many in one go
                            }
                        }
                        
                        if crate::net::dhcp_state() == crate::net::DhcpState::Bound {
                            crate::println!("\nDHCP: Configuration successful!");
                            crate::net::show_info();
                            return;
                        }
                        
                        // Small delay (~100ms)
                        for _ in 0..100000 {
                            core::hint::spin_loop();
                        }
                        ticks += 1;
                    }
                    
                    crate::println!("\nDHCP: Timeout - no response from DHCP server");
                    crate::println!("Check network connection or try manual IP config");
                }
                Err(e) => crate::println!("DHCP error: {}", e),
            }
            return;
        }
        
        // ==================== END NETWORK COMMANDS ====================
        
        // ==================== END ADDITIONAL COMMANDS ====================
    }
}

impl crate::scheduler::Task for ShellTask {
    fn step(&mut self) {
        static STEP_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
        let count = STEP_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if count < 3 {
            crate::serial::println!("[SHELL] step() called, count={}", count);
        }
        
        // Check for mouse clicks
        if let Some(click) = mouse::take_click() {
            self.handle_mouse_click(click);
        }
        
        // Editor mode has different prompt
        if self.editor_mode {
            self.editor_prompt();
        } else {
            self.prompt();
        }

        // Limit work so keyboard bursts don't starve.
        for _ in 0..32 {
            let Some(sc) = keyboard::pop_scancode() else {
                return;
            };

            if let Ok(Some(event)) = self.kb.add_byte(sc) {
                if let Some(key) = self.kb.process_keyevent(event) {
                    match key {
                        DecodedKey::Unicode('\n') => {
                            // Handle menu selection first
                            if crate::menu::is_active() {
                                if let Some(selected) = crate::menu::menu_select() {
                                    self.handle_menu_action(&selected);
                                }
                                self.redraw_input_line();
                                return;
                            }
                            
                            // If the user was viewing scrollback, jump back to the live bottom
                            // before running the command so new output is visible.
                            crate::vga::scroll_reset();

                            let len = self.len;
                            let mut tmp = [0u8; LINE_MAX];
                            tmp[..len].copy_from_slice(&self.line[..len]);

                            if self.editor_mode {
                                // Handle editor input
                                self.editor_handle_line();
                                self.clear_line();
                                self.editor_prompt();
                            } else {
                                // Save into history before executing (so arrow-up works immediately).
                                self.history_push(&tmp[..len]);

                                // Echo command into the scrolling output region for history.
                                let cwd = core::str::from_utf8(self.cwd_bytes()).unwrap_or("?");
                                let cmd_s = core::str::from_utf8(&tmp[..len]).unwrap_or("<non-utf8>");
                                crate::println!("qos:{}> {}", cwd, cmd_s);

                                self.run_command(&tmp[..len]);
                                self.clear_line();
                                self.redraw_input_line();
                            }
                            return;
                        }
                        DecodedKey::Unicode('\r') => {
                            // ignore
                        }
                        DecodedKey::Unicode('\t') => {
                            // Tab completion (only in normal mode, not editor)
                            if !self.editor_mode && !crate::menu::is_active() {
                                self.tab_complete();
                                self.redraw_input_line();
                            }
                        }
                        DecodedKey::Unicode('\u{0008}') => {
                            self.backspace();
                            self.redraw_input_line();
                        }
                        DecodedKey::Unicode(c) => {
                            if c.is_ascii() {
                                let c = if self.kbd_mode == KeyboardMode::Tr {
                                    Self::tr_ascii_map(c).unwrap_or(c)
                                } else {
                                    c
                                };
                                self.push_byte(c as u8);
                                self.redraw_input_line();
                            }
                        }
                        DecodedKey::RawKey(KeyCode::ArrowUp) => {
                            // If menu is active, navigate menu
                            if crate::menu::is_active() {
                                crate::menu::menu_up();
                            } else {
                                self.history_up();
                                self.redraw_input_line();
                            }
                        }
                        DecodedKey::RawKey(KeyCode::ArrowDown) => {
                            if crate::menu::is_active() {
                                crate::menu::menu_down();
                            } else {
                                self.history_down();
                                self.redraw_input_line();
                            }
                        }
                        DecodedKey::RawKey(KeyCode::ArrowLeft) => {
                            if crate::menu::is_active() {
                                crate::menu::menu_left();
                            }
                        }
                        DecodedKey::RawKey(KeyCode::ArrowRight) => {
                            if crate::menu::is_active() {
                                crate::menu::menu_right();
                            }
                        }
                        DecodedKey::RawKey(KeyCode::Escape) => {
                            if crate::menu::is_active() {
                                crate::menu::close_menu();
                                self.redraw_input_line();
                            }
                        }
                        DecodedKey::RawKey(KeyCode::F10) => {
                            crate::menu::toggle();
                            if !crate::menu::is_active() {
                                self.redraw_input_line();
                            }
                        }
                        DecodedKey::RawKey(KeyCode::PageUp) => {
                            let (top, bottom) = crate::vga::output_region_bounds();
                            let page = bottom.saturating_sub(top).saturating_add(1);
                            crate::vga::scroll_up(core::cmp::max(1, page.saturating_sub(1)));
                            self.redraw_input_line();
                        }
                        DecodedKey::RawKey(KeyCode::PageDown) => {
                            let (top, bottom) = crate::vga::output_region_bounds();
                            let page = bottom.saturating_sub(top).saturating_add(1);
                            crate::vga::scroll_down(core::cmp::max(1, page.saturating_sub(1)));
                            self.redraw_input_line();
                        }
                        DecodedKey::RawKey(_k) => {}
                    }
                }
            }
        }
    }
}
