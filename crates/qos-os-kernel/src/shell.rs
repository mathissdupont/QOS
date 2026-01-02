use pc_keyboard::{
    layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1,
};

use spin::Mutex;

use crate::{ata, diskfs, interrupts, keyboard, syscall, vfs, vga};

const LINE_MAX: usize = 80;
const CWD_MAX: usize = 32;
const HISTORY_MAX: usize = 16;
const EDITOR_MAX_LINES: usize = 64;
const EDITOR_LINE_MAX: usize = 80;

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
            crate::println!("commands:");
            crate::println!("  help            - show this help");
            crate::println!("  kbd [us|tr]      - set keyboard mapping for input");
            crate::println!("  clear           - clear VGA screen");
            crate::println!("  ticks           - show PIT tick counter");
            crate::println!("  ps              - show current process state");
            crate::println!("  pwd             - print current dir");
            crate::println!("  cd <dir>        - change dir: /, /ram, /disk\n");
            crate::println!("  ls [dir]        - list files (dir defaults to cwd)");
            crate::println!("  cat <path>      - print file contents");
            crate::println!("  rm <path>       - delete file");
            crate::println!("  mkbell <path>   - create built-in bell.qasm\n");
            crate::println!("  submit <path> [shots] - submit QASM2 file as job");
            crate::println!("  disk-id          - identify attached FS disk (IDE index=1)");
            crate::println!("  disk-read <lba>  - read one sector from FS disk");
            crate::println!("  mkfs             - format persistent disk FS (IDE index=1)");
            crate::println!("  dls              - list disk FS files");
            crate::println!("  dcat <file>      - print disk FS file");
            crate::println!("  drm <file>       - delete disk FS file");
            crate::println!("  dput <file>      - copy RAM file -> disk FS");
            crate::println!("  dget <file>      - copy disk FS -> RAM file");
            crate::println!("  dsubmit <file> [shots] - submit disk FS QASM2 as job");
            crate::println!("  vls <dir>        - list VFS dir: /, /ram, /disk");
            crate::println!("  vcat <path>      - cat by path: /ram/x or /disk/x");
            crate::println!("  vrm <path>       - remove by path");
            crate::println!("  vcp <src> <dst>  - copy between mounts");
            crate::println!("  vsubmit <path> [shots] - submit QASM2 from VFS path");
            crate::println!("  userdemo        - enter Ring3 demo (returns to shell on OP_EXIT)");
            crate::println!("  udemo           - run built-in Ring3 demo as scheduled foreground process");
            crate::println!("  udemo-bg        - run built-in Ring3 demo as scheduled background process");
            crate::println!("  exec <path>     - load + enter ELF64 from VFS path (returns to shell on OP_EXIT)");
            crate::println!("  procs           - list scheduled user processes");
            crate::println!("  spawn <path>    - spawn ELF64 as scheduled background process");
            crate::println!("  fg <pid>        - set scheduled process as foreground (Ctrl+C targets it)");
            crate::println!("  bg [pid]        - clear foreground (or set another pid to background)");
            crate::println!("  ui [on|off]     - toggle embedded UI overlay");
            crate::println!("  killp <pid>     - terminate a scheduled process");
            crate::println!("  waitp <pid>     - wait for scheduled process exit");
            crate::println!("  jobs            - list kernel job table");
            crate::println!("  submit-bell     - submit built-in Bell-ish job");
            crate::println!("  submit-ir-bell [shots] - submit built-in QASM2 Bell IR job");
            crate::println!("  status <handle> - show job status");
            crate::println!("  result <handle> - get result (frees slot when Done)");
            crate::println!("  cancel <handle> - cancel a job");
            crate::println!("");
            crate::println!("== System Commands ==");
            crate::println!("  time            - show current date/time");
            crate::println!("  uptime          - show system uptime");
            crate::println!("  pci             - list PCI devices");
            crate::println!("  net             - show network configuration");
            crate::println!("  shutdown        - shutdown the system");
            crate::println!("  reboot          - reboot the system");
            crate::println!("  powerinfo       - show ACPI power info");
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
        } else if Self::eq(cmd, b"clear") {
            crate::vga::clear_screen();
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
        } else if Self::eq(cmd, b"udemo") {
            crate::println!("udemo: user mode disabled (LLVM asm bug)");
        } else if Self::eq(cmd, b"udemo-bg") {
            crate::println!("udemo-bg: user mode disabled (LLVM asm bug)");
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
                Ok((n00, n11)) => crate::println!("result {} -> n00={} n11={}", h, n00, n11),
                Err(st) => crate::println!("result {} -> not ready (state={:?})", h, st),
            }
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
        } else {
            crate::println!("unknown command");
        }
    }
}

impl crate::scheduler::Task for ShellTask {
    fn step(&mut self) {
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
                            self.history_up();
                            self.redraw_input_line();
                        }
                        DecodedKey::RawKey(KeyCode::ArrowDown) => {
                            self.history_down();
                            self.redraw_input_line();
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
