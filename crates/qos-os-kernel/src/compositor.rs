//! Modern desktop compositor (WP-05 / E-70) — kernel seam over the portable `qos-ui` core.
//!
//! Allocates a true-color back [`Surface`] at the framebuffer's **native** resolution, composes a
//! modern themed scene (gradient wallpaper, translucent top bar + dock, rounded windows with soft
//! drop shadows), and blits it to the UEFI framebuffer. This is the foundation the boot splash
//! (step 2), TrueType text (step 3), widgets/WM (step 4) and apps (step 5) build on.
//!
//! Opt-in for now via the `modern` shell command (fallback-first, ADR-0015): it does not replace
//! the legacy desktop until the toolkit is ready.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use qos_ui::font::{Font, FontRenderer};
use qos_ui::{Rect, Surface, Theme};

/// Heptapus boot-splash logo coverage mask (WP-05 step 2): the octopus + "HEPTAPUS GROUP" shape,
/// generated from `heptapus_logo_primary_black.png`'s alpha by `scripts/gen_logo_mask.py`. One byte
/// per pixel; tinted per theme at draw time.
static LOGO_MASK: &[u8] = include_bytes!("assets/heptapus_logo_mask.bin");
const LOGO_W: usize = 400;
const LOGO_H: usize = 400;

// Layout constants shared by drawing + hit-testing (WP-05 step 4).
const BAR_H: i32 = 30;
const HEADER_H: i32 = 42;
const DOCK_ICON: i32 = 48;
const DOCK_GAP: i32 = 16;

/// The built-in apps a dock icon / window represents.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AppKind {
    Terminal,
    Files,
    Editor,
    Quantum,
    Monitor,
    Settings,
}

const APPS: [AppKind; 6] = [
    AppKind::Terminal,
    AppKind::Files,
    AppKind::Editor,
    AppKind::Quantum,
    AppKind::Monitor,
    AppKind::Settings,
];

fn app_title(k: AppKind) -> &'static str {
    match k {
        AppKind::Terminal => "Terminal",
        AppKind::Files => "Files",
        AppKind::Editor => "Text Editor",
        AppKind::Quantum => "Quantum Lab",
        AppKind::Monitor => "System Monitor",
        AppKind::Settings => "Settings",
    }
}

/// A distinct single-letter glyph for the dock icon (title initials collide: both System Monitor
/// and Settings start with "S").
fn app_dock_letter(k: AppKind) -> &'static str {
    match k {
        AppKind::Terminal => "T",
        AppKind::Files => "F",
        AppKind::Editor => "E",
        AppKind::Quantum => "Q",
        AppKind::Monitor => "M",
        AppKind::Settings => "S",
    }
}

fn app_tint(k: AppKind, theme: &Theme) -> qos_ui::Rgb {
    match k {
        AppKind::Terminal => theme.accent,
        AppKind::Files => qos_ui::rgb(0x30, 0xb0, 0x60),
        AppKind::Editor => qos_ui::rgb(0xd0, 0x9a, 0x2a),
        AppKind::Quantum => qos_ui::rgb(0x8a, 0x5c, 0xd8),
        AppKind::Monitor => qos_ui::rgb(0x27, 0xa8, 0xc8),
        AppKind::Settings => qos_ui::rgb(0xe0, 0x7a, 0x2a),
    }
}

/// A filled circle via a maximally-rounded square (used for the macOS-style window dots + dock).
fn circle(s: &mut Surface, cx: i32, cy: i32, d: i32, color: qos_ui::Rgb) {
    s.rounded_rect(Rect::new(cx - d / 2, cy - d / 2, d, d), d / 2, color);
}

// Per-app clickable geometry, shared by drawing + hit-testing so they stay in sync (`win` = window
// rect). Files toolbar + entry rows, Quantum Lab run buttons, the Settings theme toggle, and the
// Text Editor action buttons.

/// Files toolbar buttons (real file-manager ops), laid across the top of the body.
const FILES_TOOLBAR: [&str; 5] = ["New File", "New Dir", "Rename", "Delete", "Edit"];
fn files_tool_rect(win: Rect, i: usize) -> Rect {
    let bw = 90;
    let gap = 5;
    Rect::new(win.x + 16 + i as i32 * (bw + gap), win.y + HEADER_H + 32, bw, 26)
}
fn files_row_rect(win: Rect, i: usize) -> Rect {
    Rect::new(win.x + 16, win.y + HEADER_H + 72 + i as i32 * 28, win.w - 32, 24)
}
/// Centered modal box for entering a name (New File / New Dir / Rename).
fn files_name_box(win: Rect) -> Rect {
    Rect::new(win.x + 36, win.y + HEADER_H + 96, win.w - 72, 96)
}
fn qlab_btn_rect(win: Rect, i: usize) -> Rect {
    Rect::new(win.x + 24 + i as i32 * 150, win.y + HEADER_H + 108, 132, 40)
}
fn settings_theme_rect(win: Rect) -> Rect {
    Rect::new(win.x + 24, win.y + HEADER_H + 30, 220, 36)
}
/// Text Editor action buttons (Save / New).
fn editor_btn_rect(win: Rect, i: usize) -> Rect {
    Rect::new(win.x + 16 + i as i32 * 96, win.y + HEADER_H + 10, 88, 26)
}
const FILES_MAX_ROWS: usize = 5;

/// An open window on the desktop.
struct Win {
    rect: Rect,
    kind: AppKind,
}

/// Which kind of name the Files naming modal is collecting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NameMode {
    NewFile,
    NewDir,
    Rename,
}

/// Translate a PS/2 Set-1 make scancode to a character (honoring shift). `None` for non-text keys.
fn scancode_to_char(sc: u8, shift: bool) -> Option<char> {
    // Letters (Set-1) → lowercase; shift makes them uppercase.
    let letter = match sc {
        0x10 => 'q', 0x11 => 'w', 0x12 => 'e', 0x13 => 'r', 0x14 => 't', 0x15 => 'y', 0x16 => 'u',
        0x17 => 'i', 0x18 => 'o', 0x19 => 'p', 0x1E => 'a', 0x1F => 's', 0x20 => 'd', 0x21 => 'f',
        0x22 => 'g', 0x23 => 'h', 0x24 => 'j', 0x25 => 'k', 0x26 => 'l', 0x2C => 'z', 0x2D => 'x',
        0x2E => 'c', 0x2F => 'v', 0x30 => 'b', 0x31 => 'n', 0x32 => 'm', _ => '\0',
    };
    if letter != '\0' {
        return Some(if shift { letter.to_ascii_uppercase() } else { letter });
    }
    let c = match sc {
        0x02..=0x0B => {
            let base = [b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0'];
            let sh = [b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')'];
            let i = (sc - 0x02) as usize;
            (if shift { sh[i] } else { base[i] }) as char
        }
        0x39 => ' ',
        0x0C => if shift { '_' } else { '-' },
        0x0D => if shift { '+' } else { '=' },
        0x1A => if shift { '{' } else { '[' },
        0x1B => if shift { '}' } else { ']' },
        0x27 => if shift { ':' } else { ';' },
        0x28 => if shift { '"' } else { '\'' },
        0x29 => if shift { '~' } else { '`' },
        0x2B => if shift { '|' } else { '\\' },
        0x33 => if shift { '<' } else { ',' },
        0x34 => if shift { '>' } else { '.' },
        0x35 => if shift { '?' } else { '/' },
        _ => return None,
    };
    Some(c)
}

/// An in-window terminal: a scrollback buffer + an input line, with a small real command set that
/// reaches actual subsystems (the quantum simulator, heap/uptime). This is the flagship of the
/// "real, working apps" step (WP-05 step 5) and the first user-facing bridge to the quantum layer.
struct Terminal {
    lines: Vec<String>,
    input: String,
    /// Working directory for the fs commands (empty = root), shared model with the Files app.
    cwd: String,
}

impl Terminal {
    fn new() -> Self {
        let mut t = Terminal { lines: Vec::new(), input: String::new(), cwd: String::new() };
        t.push("QOS Terminal — type 'help'.".to_string());
        t
    }

    /// Resolve `arg` against the terminal cwd into an absolute fs path (root-relative, no leading
    /// '/'). `..` pops one segment; an empty/`.`/`/` arg is the cwd itself; a leading '/' is
    /// treated as absolute.
    fn resolve(&self, arg: &str) -> String {
        let arg = arg.trim();
        if arg.is_empty() || arg == "." {
            return self.cwd.clone();
        }
        if arg == ".." {
            return match self.cwd.rfind('/') {
                Some(p) => self.cwd[..p].to_string(),
                None => String::new(),
            };
        }
        if let Some(stripped) = arg.strip_prefix('/') {
            return stripped.trim_end_matches('/').to_string();
        }
        if self.cwd.is_empty() {
            arg.to_string()
        } else {
            format!("{}/{}", self.cwd, arg)
        }
    }

    /// Run a single-path fs op (mkdir/touch/rm): validate the arg, resolve it, call `f`, and return
    /// a result line to print. Always returns `Some` (kept as `Option` for a uniform call site).
    fn op(&self, rest: &str, usage: &str, f: impl Fn(&[u8]) -> Result<(), &'static str>) -> Option<String> {
        if rest.trim().is_empty() {
            return Some(usage.to_string());
        }
        let path = self.resolve(rest);
        Some(match f(path.as_bytes()) {
            Ok(()) => format!("ok: /{}", path),
            Err(e) => format!("error: {}", e),
        })
    }

    fn push(&mut self, s: String) {
        self.lines.push(s);
        if self.lines.len() > 256 {
            self.lines.remove(0);
        }
    }

    fn type_char(&mut self, c: char) {
        if self.input.len() < 100 {
            self.input.push(c);
        }
    }

    fn backspace(&mut self) {
        self.input.pop();
    }

    /// The shell prompt, reflecting the current working directory.
    fn prompt(&self) -> String {
        if self.cwd.is_empty() {
            "qos:/>".to_string()
        } else {
            format!("qos:/{}>", self.cwd)
        }
    }

    fn enter(&mut self) {
        let cmd = core::mem::take(&mut self.input);
        let p = self.prompt();
        self.push(format!("{} {}", p, cmd));
        self.run(&cmd);
    }

    /// Execute one command line against real subsystems.
    fn run(&mut self, line: &str) {
        let line = line.trim();
        let (name, rest) = match line.split_once(' ') {
            Some((a, b)) => (a, b.trim()),
            None => (line, ""),
        };
        match name {
            "" => {}
            "help" => {
                for l in [
                    "commands:",
                    "  help clear echo ver mem pwd",
                    "  ls [dir]        list a directory",
                    "  cd <dir>        change directory (.. = up)",
                    "  cat <file>      print a file",
                    "  mkdir <dir>     create a directory",
                    "  touch <file>    create an empty file",
                    "  write <f> <tx>  write text to a file",
                    "  rm <path>       remove a file or empty dir",
                    "  bell            2-qubit Bell state, 1000 shots",
                    "  ghz             3-qubit GHZ state, 1000 shots",
                    "  qrng [n]        n quantum random bits (default 8)",
                ] {
                    self.push(l.to_string());
                }
            }
            "clear" => self.lines.clear(),
            "pwd" => {
                let p = if self.cwd.is_empty() { "/".to_string() } else { format!("/{}", self.cwd) };
                self.push(p);
            }
            "ls" => {
                let dir = self.resolve(rest);
                if !dir.is_empty() && !crate::fs::is_dir(dir.as_bytes()) {
                    self.push(format!("ls: not a directory: /{}", dir));
                } else {
                    let mut entries = crate::fs::get_entries(dir.as_bytes());
                    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                    if entries.is_empty() {
                        self.push("(empty)".to_string());
                    }
                    for (name, is_dir, size) in entries {
                        if is_dir {
                            self.push(format!("  {}/", name));
                        } else {
                            self.push(format!("  {}   {} B", name, size));
                        }
                    }
                }
            }
            "cd" => {
                let target = self.resolve(rest);
                if target.is_empty() || crate::fs::is_dir(target.as_bytes()) {
                    self.cwd = target;
                } else {
                    self.push(format!("cd: no such directory: {}", rest));
                }
            }
            "cat" => {
                if rest.is_empty() {
                    self.push("usage: cat <file>".to_string());
                } else {
                    let path = self.resolve(rest);
                    match crate::fs::read(path.as_bytes()) {
                        Some(bytes) => {
                            let text = String::from_utf8_lossy(&bytes);
                            for line in text.split('\n') {
                                self.push(line.to_string());
                            }
                        }
                        None => self.push(format!("cat: cannot read /{}", path)),
                    }
                }
            }
            "mkdir" => match self.op(rest, "usage: mkdir <dir>", |p| crate::fs::mkdir(p)) {
                Some(msg) => self.push(msg),
                None => {}
            },
            "touch" => match self.op(rest, "usage: touch <file>", |p| crate::fs::touch(p)) {
                Some(msg) => self.push(msg),
                None => {}
            },
            "rm" => match self.op(rest, "usage: rm <path>", |p| crate::fs::remove(p)) {
                Some(msg) => self.push(msg),
                None => {}
            },
            "write" => {
                match rest.split_once(' ') {
                    Some((name, text)) if !name.is_empty() => {
                        let path = self.resolve(name);
                        match crate::fs::write(path.as_bytes(), text.as_bytes()) {
                            Ok(()) => self.push(format!("wrote {} B to /{}", text.len(), path)),
                            Err(e) => self.push(format!("write: {}", e)),
                        }
                    }
                    _ => self.push("usage: write <file> <text>".to_string()),
                }
            }
            "echo" => self.push(rest.to_string()),
            "ver" => self.push("QOS 0.1 — UEFI x86-64, native compositor UI, quantum control plane".to_string()),
            "mem" => {
                let ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
                self.push(format!("heap {} MiB   uptime {} ticks (~{} s)", crate::allocator::HEAP_SIZE / 1024 / 1024, ticks, ticks / 100));
            }
            "bell" => {
                let (z, o) = crate::quantum::sim::run_bell(1000);
                self.push(format!("Bell |Φ+> x1000:   00 -> {}    11 -> {}", z, o));
                self.push(format!("(entangled: {}% correlated)", (z + o) * 100 / 1000));
            }
            "ghz" => self.run_qasm(
                b"OPENQASM 2.0;\nqreg q[3];\ncreg c[3];\nh q[0];\ncx q[0],q[1];\ncx q[1],q[2];\nmeasure q[0]->c[0];\nmeasure q[1]->c[1];\nmeasure q[2]->c[2];\n",
                1000,
                "GHZ |000>+|111>",
            ),
            "qrng" => {
                let n = rest.parse::<usize>().unwrap_or(8).clamp(1, 16);
                let mut bits = String::new();
                for _ in 0..n {
                    let (_z, o) = {
                        let r = crate::quantum::sim::run_qasm2(b"OPENQASM 2.0;\nqreg q[1];\ncreg c[1];\nh q[0];\nmeasure q[0]->c[0];\n", 1);
                        match r {
                            Ok(res) => (res.count_zeros(), res.count_ones()),
                            Err(_) => (1, 0),
                        }
                    };
                    bits.push(if o > 0 { '1' } else { '0' });
                }
                self.push(format!("quantum random: {}", bits));
            }
            _ => self.push(format!("unknown command: {}   (try help)", name)),
        }
    }

    fn run_qasm(&mut self, qasm: &[u8], shots: u64, label: &str) {
        match crate::quantum::sim::run_qasm2(qasm, shots) {
            Ok(res) => {
                self.push(format!("{} x{}:", label, shots));
                for (k, v) in res.counts.iter() {
                    self.push(format!("  {} -> {}", k, v));
                }
            }
            Err(_) => self.push("quantum: parse error".to_string()),
        }
    }
}

/// 11×16 arrow-cursor bitmap: `#` = dark outline, `o` = white fill, `.` = transparent.
const CURSOR: [&str; 16] = [
    "#..........",
    "##.........",
    "#o#........",
    "#oo#.......",
    "#ooo#......",
    "#oooo#.....",
    "#ooooo#....",
    "#oooooo#...",
    "#ooooooo#..",
    "#oooooooo#.",
    "#oooo#####.",
    "#oo#oo#....",
    "#o#.#oo#...",
    "##..#oo#...",
    "#....#oo#..",
    ".....#oo#..",
];

/// The interactive modern desktop: window manager state + rendering + input handling (WP-05 step 4).
struct Desktop {
    w: i32,
    h: i32,
    theme: Theme,
    wins: Vec<Win>,                  // z-order: last element is topmost/focused
    cursor: (i32, i32),
    drag: Option<(usize, i32, i32)>, // (window index, grab offset x, grab offset y)
    dirty: bool,
    /// When dirty, `full` = blit the whole screen; otherwise `damage` = the sub-rect to blit.
    full: bool,
    damage: Rect,
    /// The working terminal (shared by the Terminal window).
    term: Terminal,
    /// Files app: current directory (empty = root) + optional file preview text.
    files_cwd: String,
    files_preview: Option<String>,
    /// Files: the currently selected entry name (for Rename/Delete/Edit).
    files_sel: Option<String>,
    /// Files: a status/error line under the toolbar (e.g. "deleted", "cannot delete non-empty dir").
    files_status: String,
    /// Files: active naming modal (kind + typed buffer), if any.
    files_naming: Option<NameMode>,
    files_name_buf: String,
    /// Quantum Lab: last run's result lines.
    qlab: Vec<String>,
    /// Text Editor: path of the open file (None = nothing open), buffer, and a status line.
    editor_path: Option<String>,
    editor_buf: String,
    editor_status: String,
}

/// Margin around a window rect that its shadow extends into (for damage rects).
const SHADOW_MARGIN: i32 = 34;

impl Desktop {
    fn new(w: i32, h: i32) -> Self {
        // Start with two cascaded windows so the desktop looks alive.
        let wins = vec![
            Win { rect: Rect::new(w / 2 - 440, 74, 540, 440), kind: AppKind::Terminal },
            Win { rect: Rect::new(w / 2 - 40, 230, 520, 440), kind: AppKind::Files },
        ];
        Desktop {
            w,
            h,
            theme: Theme::dark(),
            wins,
            cursor: (w / 2, h / 2),
            drag: None,
            dirty: true,
            full: true,
            damage: Rect::new(0, 0, 0, 0),
            term: Terminal::new(),
            files_cwd: String::new(),
            files_preview: None,
            files_sel: None,
            files_status: String::new(),
            files_naming: None,
            files_name_buf: String::new(),
            qlab: Vec::new(),
            editor_path: None,
            editor_buf: String::new(),
            editor_status: "no file open — open one from Files".to_string(),
        }
    }

    // ---- app actions (invoked by body clicks) ----
    /// The Files listing for the current directory: `..` (unless at root) then dirs, then files,
    /// each sorted by name. Used by both drawing and click hit-testing.
    fn files_list(&self) -> Vec<(String, bool, usize)> {
        let mut entries = crate::fs::get_entries(self.files_cwd.as_bytes());
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut list = Vec::new();
        if !self.files_cwd.is_empty() {
            list.push(("..".to_string(), true, 0));
        }
        list.extend(entries);
        list
    }

    /// Dispatch a click inside a focused window's body to the app.
    fn on_body_click(&mut self, kind: AppKind, wr: Rect, cx: i32, cy: i32) {
        match kind {
            AppKind::Files => {
                // A naming modal is open: swallow body clicks (commit is Enter, cancel is Esc).
                if self.files_naming.is_some() {
                    return;
                }
                // Toolbar buttons: real file-manager operations.
                for i in 0..FILES_TOOLBAR.len() {
                    if files_tool_rect(wr, i).contains(cx, cy) {
                        self.files_tool(i);
                        return;
                    }
                }
                let list = self.files_list();
                for (i, (name, is_dir, _)) in list.iter().enumerate().take(FILES_MAX_ROWS) {
                    if files_row_rect(wr, i).contains(cx, cy) {
                        let (n, d) = (name.clone(), *is_dir);
                        self.files_click(&n, d);
                        return;
                    }
                }
            }
            AppKind::Editor => {
                if editor_btn_rect(wr, 0).contains(cx, cy) {
                    self.editor_save();
                    return;
                }
                if editor_btn_rect(wr, 1).contains(cx, cy) {
                    self.editor_buf.clear();
                    self.editor_path = None;
                    self.editor_status = "new buffer — Save creates a file via New File in Files".to_string();
                    self.mark_full();
                    return;
                }
            }
            AppKind::Quantum => {
                for idx in 0..2 {
                    if qlab_btn_rect(wr, idx).contains(cx, cy) {
                        self.qlab_run(idx as u8);
                        return;
                    }
                }
            }
            AppKind::Settings => {
                if settings_theme_rect(wr).contains(cx, cy) {
                    self.theme = self.theme.toggled();
                    self.mark_full();
                }
            }
            AppKind::Terminal | AppKind::Monitor => {}
        }
    }

    /// Absolute path (relative to fs root) of `name` in the current directory.
    fn files_path(&self, name: &str) -> String {
        if self.files_cwd.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", self.files_cwd, name)
        }
    }

    /// Navigate the Files app into `name` (a subdir) or up (`..`), or select+preview a file.
    fn files_click(&mut self, name: &str, is_dir: bool) {
        self.files_status.clear();
        if name == ".." {
            // Go up one path segment.
            if let Some(pos) = self.files_cwd.rfind('/') {
                self.files_cwd.truncate(pos);
            } else {
                self.files_cwd.clear();
            }
            self.files_preview = None;
            self.files_sel = None;
        } else if is_dir {
            // Single click selects a dir; navigating in is via double-purpose: select then click
            // again enters. Keep it simple: clicking a dir enters it (mirrors the old behavior).
            if !self.files_cwd.is_empty() {
                self.files_cwd.push('/');
            }
            self.files_cwd.push_str(name);
            self.files_preview = None;
            self.files_sel = None;
        } else {
            // Select the file and preview its text.
            self.files_sel = Some(name.to_string());
            let path = self.files_path(name);
            self.files_preview = Some(match crate::fs::read(path.as_bytes()) {
                Some(bytes) => {
                    let mut s = String::new();
                    for &b in bytes.iter().take(400) {
                        s.push(if b == b'\n' || (0x20..0x7f).contains(&b) { b as char } else { '.' });
                    }
                    s
                }
                None => "(cannot read)".to_string(),
            });
        }
        self.mark_full();
    }

    /// A Files toolbar button was clicked (index into `FILES_TOOLBAR`).
    fn files_tool(&mut self, i: usize) {
        self.files_status.clear();
        match i {
            0 => self.files_begin_name(NameMode::NewFile), // New File
            1 => self.files_begin_name(NameMode::NewDir),  // New Dir
            2 => {
                // Rename: needs a selection.
                if self.files_sel.is_some() {
                    self.files_begin_name(NameMode::Rename);
                } else {
                    self.files_status = "select an item first".to_string();
                    self.mark_full();
                }
            }
            3 => self.files_delete(), // Delete
            4 => self.files_edit(),   // Edit (open in Text Editor)
            _ => {}
        }
    }

    /// Open the Files naming modal in `mode` (prefilled with the selection for Rename).
    fn files_begin_name(&mut self, mode: NameMode) {
        self.files_name_buf = match mode {
            NameMode::Rename => self.files_sel.clone().unwrap_or_default(),
            _ => String::new(),
        };
        self.files_naming = Some(mode);
        self.mark_full();
    }

    fn files_cancel_name(&mut self) {
        self.files_naming = None;
        self.files_name_buf.clear();
        self.mark_full();
    }

    /// Commit the naming modal → the corresponding real fs operation.
    fn files_commit_name(&mut self) {
        let Some(mode) = self.files_naming else { return };
        let name = self.files_name_buf.trim().to_string();
        self.files_naming = None;
        self.files_name_buf.clear();
        if name.is_empty() || name.contains('/') {
            self.files_status = "invalid name".to_string();
            self.mark_full();
            return;
        }
        match mode {
            NameMode::NewFile => {
                let path = self.files_path(&name);
                match crate::fs::write(path.as_bytes(), b"") {
                    Ok(()) => {
                        self.files_status = format!("created {}", name);
                        self.files_sel = Some(name);
                    }
                    Err(e) => self.files_status = format!("error: {}", e),
                }
            }
            NameMode::NewDir => {
                let path = self.files_path(&name);
                match crate::fs::mkdir(path.as_bytes()) {
                    Ok(()) => self.files_status = format!("created {}/", name),
                    Err(e) => self.files_status = format!("error: {}", e),
                }
            }
            NameMode::Rename => {
                if let Some(old) = self.files_sel.clone() {
                    let from = self.files_path(&old);
                    let to = self.files_path(&name);
                    match crate::fs::rename(from.as_bytes(), to.as_bytes()) {
                        Ok(()) => {
                            self.files_status = format!("renamed to {}", name);
                            self.files_sel = Some(name);
                            self.files_preview = None;
                        }
                        Err(e) => self.files_status = format!("error: {}", e),
                    }
                }
            }
        }
        self.mark_full();
    }

    /// Delete the selected entry (files, or empty dirs — the fs enforces non-empty).
    fn files_delete(&mut self) {
        let Some(name) = self.files_sel.clone() else {
            self.files_status = "select an item first".to_string();
            self.mark_full();
            return;
        };
        let path = self.files_path(&name);
        match crate::fs::remove(path.as_bytes()) {
            Ok(()) => {
                self.files_status = format!("deleted {}", name);
                self.files_sel = None;
                self.files_preview = None;
            }
            Err(e) => self.files_status = format!("cannot delete: {}", e),
        }
        self.mark_full();
    }

    /// Open the selected file in the Text Editor.
    fn files_edit(&mut self) {
        let Some(name) = self.files_sel.clone() else {
            self.files_status = "select a file first".to_string();
            self.mark_full();
            return;
        };
        if crate::fs::is_dir(self.files_path(&name).as_bytes()) {
            self.files_status = "cannot edit a directory".to_string();
            self.mark_full();
            return;
        }
        let path = self.files_path(&name);
        self.editor_open(&path);
        self.open_app(AppKind::Editor);
    }

    /// Load `path` into the Text Editor buffer.
    fn editor_open(&mut self, path: &str) {
        match crate::fs::read(path.as_bytes()) {
            Some(bytes) => {
                self.editor_buf = String::from_utf8_lossy(&bytes).into_owned();
                self.editor_path = Some(path.to_string());
                self.editor_status = format!("editing {}  ({} bytes)", path, bytes.len());
            }
            None => {
                self.editor_buf.clear();
                self.editor_path = Some(path.to_string());
                self.editor_status = format!("editing {}  (new)", path);
            }
        }
    }

    /// Save the Text Editor buffer back to its file via the real fs.
    fn editor_save(&mut self) {
        match self.editor_path.clone() {
            Some(path) => match crate::fs::write(path.as_bytes(), self.editor_buf.as_bytes()) {
                Ok(()) => self.editor_status = format!("saved {}  ({} bytes)", path, self.editor_buf.len()),
                Err(e) => self.editor_status = format!("save error: {}", e),
            },
            None => self.editor_status = "no file — create one from Files first".to_string(),
        }
        self.mark_full();
    }

    /// Run the given quantum program in the Quantum Lab and capture measurement counts.
    fn qlab_run(&mut self, kind: u8) {
        self.qlab.clear();
        match kind {
            0 => {
                let (z, o) = crate::quantum::sim::run_bell(1000);
                self.qlab.push(format!("Bell x1000:  00 -> {}   11 -> {}", z, o));
            }
            _ => match crate::quantum::sim::run_qasm2(
                b"OPENQASM 2.0;\nqreg q[3];\ncreg c[3];\nh q[0];\ncx q[0],q[1];\ncx q[1],q[2];\nmeasure q[0]->c[0];\nmeasure q[1]->c[1];\nmeasure q[2]->c[2];\n",
                1000,
            ) {
                Ok(res) => {
                    self.qlab.push("GHZ x1000:".to_string());
                    for (k, v) in res.counts.iter() {
                        self.qlab.push(format!("  {} -> {}", k, v));
                    }
                }
                Err(_) => self.qlab.push("parse error".to_string()),
            },
        }
        self.mark_full();
    }

    /// True if the focused (topmost) window is the Terminal — then typed keys go to it.
    fn top_is_terminal(&self) -> bool {
        self.wins.last().map_or(false, |w| w.kind == AppKind::Terminal)
    }

    /// True if the focused window is the System Monitor — used to refresh it live.
    fn top_is_monitor(&self) -> bool {
        self.wins.last().map_or(false, |w| w.kind == AppKind::Monitor)
    }

    /// True if the focused window is the Text Editor — then typed keys edit its buffer.
    fn top_is_editor(&self) -> bool {
        self.wins.last().map_or(false, |w| w.kind == AppKind::Editor)
    }

    /// True while the Files naming modal is open and the focused window is Files — then typed keys
    /// go to the name buffer.
    fn files_naming_active(&self) -> bool {
        self.files_naming.is_some() && self.wins.last().map_or(false, |w| w.kind == AppKind::Files)
    }

    /// Mark just the focused window's footprint (plus shadow) dirty — used for terminal typing so a
    /// keystroke doesn't repaint the whole screen.
    fn mark_top_window(&mut self) {
        if let Some(w) = self.wins.last() {
            let r = w.rect;
            self.mark_region(r.inflate(SHADOW_MARGIN));
        } else {
            self.mark_full();
        }
    }

    /// Mark the whole screen for redraw (z-order / theme / open / close changes).
    fn mark_full(&mut self) {
        self.dirty = true;
        self.full = true;
    }
    /// Mark just `r` for redraw (accumulated); a full mark wins.
    fn mark_region(&mut self, r: Rect) {
        self.dirty = true;
        if !self.full {
            self.damage = self.damage.union(&r);
        }
    }

    // ---- geometry / hit-testing ----
    fn dock_rect(&self) -> Rect {
        let n = APPS.len() as i32;
        let dw = n * DOCK_ICON + (n + 1) * DOCK_GAP;
        let dh = DOCK_ICON + 2 * DOCK_GAP;
        Rect::new(self.w / 2 - dw / 2, self.h - dh - 14, dw, dh)
    }
    fn dock_icon_rect(&self, i: i32) -> Rect {
        let d = self.dock_rect();
        Rect::new(d.x + DOCK_GAP + i * (DOCK_ICON + DOCK_GAP), d.y + DOCK_GAP, DOCK_ICON, DOCK_ICON)
    }
    /// The light/dark toggle pill in the top-right of the menu bar.
    fn theme_btn(&self) -> Rect {
        Rect::new(self.w - 150, 4, 64, 22)
    }
    fn close_dot(&self, r: &Rect) -> (i32, i32) {
        (r.x + 22, r.y + HEADER_H / 2)
    }

    // ---- app/window management ----
    fn open_app(&mut self, kind: AppKind) {
        if let Some(i) = self.wins.iter().position(|w| w.kind == kind) {
            let win = self.wins.remove(i);
            self.wins.push(win); // raise
        } else {
            let n = self.wins.len() as i32;
            let rect = Rect::new((self.w / 2 - 270 + n * 28).max(20), (80 + n * 24).min(self.h - 460), 540, 440);
            self.wins.push(Win { rect, kind });
        }
        self.mark_full();
    }

    // ---- input ----
    fn on_mouse_move(&mut self, dx: i16, dy: i16) {
        self.cursor.0 = (self.cursor.0 + dx as i32).clamp(0, self.w - 1);
        // InputEvent dy is +up (PS/2 convention); screen y grows downward.
        self.cursor.1 = (self.cursor.1 - dy as i32).clamp(0, self.h - 1);
        // Only a drag changes the scene; a bare cursor move is handled cheaply (save-under) by the
        // caller, so it must NOT dirty (and thus fully recompose) the whole desktop.
        if let Some((idx, ox, oy)) = self.drag {
            if idx < self.wins.len() {
                let old = self.wins[idx].rect;
                self.wins[idx].rect.x = self.cursor.0 - ox;
                self.wins[idx].rect.y = (self.cursor.1 - oy).max(BAR_H);
                let new = self.wins[idx].rect;
                // Only the old + new window footprints (plus shadow) changed — blit just that.
                self.mark_region(old.union(&new).inflate(SHADOW_MARGIN));
            }
        }
    }

    fn on_left_down(&mut self) {
        let (cx, cy) = self.cursor;
        // Top menu bar: theme toggle.
        if self.theme_btn().contains(cx, cy) {
            self.theme = self.theme.toggled();
            self.mark_full();
            return;
        }
        // Windows, top-most first.
        for i in (0..self.wins.len()).rev() {
            let r = self.wins[i].rect;
            let (dxc, dyc) = self.close_dot(&r);
            if (cx - dxc).abs() <= 9 && (cy - dyc).abs() <= 9 {
                self.wins.remove(i); // close
                self.mark_full();
                return;
            }
            if r.contains(cx, cy) {
                // Raise; then either start dragging (header) or dispatch a body click to the app.
                let win = self.wins.remove(i);
                let (wr, kind) = (win.rect, win.kind);
                let on_header = cy < wr.y + HEADER_H;
                let off = (cx - wr.x, cy - wr.y);
                self.wins.push(win);
                if on_header {
                    self.drag = Some((self.wins.len() - 1, off.0, off.1));
                } else {
                    self.on_body_click(kind, wr, cx, cy);
                }
                self.mark_full();
                return;
            }
        }
        // Dock icons.
        for (i, &kind) in APPS.iter().enumerate() {
            if self.dock_icon_rect(i as i32).contains(cx, cy) {
                self.open_app(kind);
                return;
            }
        }
    }

    fn on_left_up(&mut self) {
        if self.drag.take().is_some() {
            self.mark_full();
        }
    }

    // ---- rendering ----
    fn draw_window(&self, s: &mut Surface, fr: &mut FontRenderer, i: usize, focused: bool) {
        let theme = &self.theme;
        let r = self.wins[i].rect;
        let kind = self.wins[i].kind;
        let radius = 14;
        let shadow_a = if focused { if theme.is_dark { 160 } else { 100 } } else { if theme.is_dark { 90 } else { 50 } };
        s.drop_shadow(Rect::new(r.x, r.y + 6, r.w, r.h), radius, 22, theme.shadow, shadow_a);
        s.rounded_rect(r, radius, theme.surface);
        // Header.
        s.rounded_rect(Rect::new(r.x, r.y, r.w, HEADER_H), radius, theme.surface_alt);
        s.fill_rect(Rect::new(r.x, r.y + radius, r.w, HEADER_H - radius), theme.surface_alt);
        s.fill_rect(Rect::new(r.x, r.y + HEADER_H, r.w, 1), theme.border);
        // Focus accent line along the top of a focused window.
        if focused {
            s.fill_rect(Rect::new(r.x + radius, r.y, r.w - 2 * radius, 2), theme.accent);
        }
        let cy = r.y + HEADER_H / 2;
        circle(s, r.x + 22, cy, 14, qos_ui::rgb(0xff, 0x5f, 0x57));
        circle(s, r.x + 44, cy, 14, qos_ui::rgb(0xfe, 0xbc, 0x2e));
        circle(s, r.x + 66, cy, 14, qos_ui::rgb(0x28, 0xc8, 0x40));
        let title = app_title(kind);
        let tw = fr.text_width(title, 18.0);
        fr.draw_text(s, r.x + (r.w - tw) / 2, cy + 6, title, 18.0, theme.text);
        // Body content (light stubs; full apps are step 5).
        let bx = r.x + 22;
        let by = r.y + HEADER_H + 30;
        match kind {
            AppKind::Terminal => {
                let inner = Rect::new(r.x + 12, r.y + HEADER_H + 10, r.w - 24, r.h - HEADER_H - 22);
                s.rounded_rect(inner, 8, qos_ui::rgb(0x10, 0x12, 0x18));
                let green = qos_ui::rgb(0x6e, 0xe0, 0x7a);
                let line_h = 20;
                let tx = inner.x + 14;
                let rows = ((inner.h - 20) / line_h).max(1) as usize;
                // Show the last `rows-1` scrollback lines, then the live input line with a cursor.
                let total = self.term.lines.len();
                let start = total.saturating_sub(rows - 1);
                let mut ty = inner.y + 24;
                for line in &self.term.lines[start..] {
                    let col = if line.starts_with("qos:/") { green } else { theme.text };
                    fr.draw_text(s, tx, ty, line, 14.0, col);
                    ty += line_h;
                }
                let prompt = format!("{} {}_", self.term.prompt(), self.term.input);
                fr.draw_text(s, tx, ty, &prompt, 14.0, green);
            }
            AppKind::Files => {
                // Real listing of the current directory (the in-kernel filesystem).
                let path = if self.files_cwd.is_empty() { "/".to_string() } else { format!("/{}", self.files_cwd) };
                fr.draw_text(s, bx, r.y + HEADER_H + 22, &path, 14.0, theme.text_dim);
                // Keyboard-shortcut hint (right-aligned) — this is a real keyboard-driven file mgr.
                let hint = "keys  n·new  k·dir  r·ren  x·del  e·edit";
                let hw = fr.text_width(hint, 11.0);
                fr.draw_text(s, r.right() - hw - 16, r.y + HEADER_H + 22, hint, 11.0, theme.text_dim);
                // Toolbar (real file-manager ops).
                for (i, label) in FILES_TOOLBAR.iter().enumerate() {
                    let b = files_tool_rect(r, i);
                    s.rounded_rect(b, 7, theme.surface_alt);
                    let lw = fr.text_width(label, 12.0);
                    fr.draw_text(s, b.x + (b.w - lw) / 2, b.y + 17, label, 12.0, theme.text);
                }
                // Directory entries (selected row highlighted with the accent).
                let list = self.files_list();
                for (i, (name, is_dir, size)) in list.iter().take(FILES_MAX_ROWS).enumerate() {
                    let rr = files_row_rect(r, i);
                    let selected = self.files_sel.as_deref() == Some(name.as_str());
                    s.rounded_rect(rr, 6, if selected { theme.accent } else { theme.surface_alt });
                    let txt = if selected { theme.on_accent } else { theme.text };
                    let dim = if selected { theme.on_accent } else { theme.text_dim };
                    let icon = if *is_dir { "[D]" } else { "[F]" };
                    fr.draw_text(s, rr.x + 10, rr.y + 17, icon, 13.0, if *is_dir && !selected { theme.accent } else { dim });
                    fr.draw_text(s, rr.x + 46, rr.y + 17, name, 14.0, txt);
                    if !*is_dir {
                        let sz = format!("{} B", size);
                        let sw = fr.text_width(&sz, 12.0);
                        fr.draw_text(s, rr.right() - sw - 12, rr.y + 17, &sz, 12.0, dim);
                    }
                }
                // Status line + preview, below the rows.
                let rows = list.len().min(FILES_MAX_ROWS);
                let py = files_row_rect(r, rows).y + 6;
                if !self.files_status.is_empty() {
                    fr.draw_text(s, r.x + 16, py + 4, &self.files_status, 12.0, theme.text_dim);
                }
                if let Some(prev) = &self.files_preview {
                    let py = py + 18;
                    s.fill_rect(Rect::new(r.x + 16, py, r.w - 32, 1), theme.border);
                    let mut ly = py + 20;
                    for line in prev.split('\n').take(4) {
                        fr.draw_text(s, r.x + 20, ly, line, 13.0, theme.text_dim);
                        ly += 18;
                    }
                }
                // Naming modal overlay (New File / New Dir / Rename).
                if let Some(mode) = self.files_naming {
                    let box_r = files_name_box(r);
                    s.drop_shadow(box_r, 12, 18, theme.shadow, if theme.is_dark { 150 } else { 80 });
                    s.rounded_rect(box_r, 12, theme.surface_alt);
                    let title = match mode {
                        NameMode::NewFile => "New file name:",
                        NameMode::NewDir => "New directory name:",
                        NameMode::Rename => "Rename to:",
                    };
                    fr.draw_text(s, box_r.x + 16, box_r.y + 26, title, 14.0, theme.text);
                    let field = Rect::new(box_r.x + 16, box_r.y + 38, box_r.w - 32, 26);
                    s.rounded_rect(field, 6, theme.surface);
                    let shown = format!("{}_", self.files_name_buf);
                    fr.draw_text(s, field.x + 10, field.y + 18, &shown, 14.0, theme.text);
                    fr.draw_text(s, box_r.x + 16, box_r.bottom() - 8, "Enter = ok    Esc = cancel", 12.0, theme.text_dim);
                }
            }
            AppKind::Editor => {
                // Action buttons.
                for (i, label) in ["Save", "New"].iter().enumerate() {
                    let b = editor_btn_rect(r, i);
                    s.rounded_rect(b, 7, if i == 0 { theme.accent } else { theme.surface_alt });
                    let lw = fr.text_width(label, 13.0);
                    fr.draw_text(s, b.x + (b.w - lw) / 2, b.y + 18, label, 13.0, if i == 0 { theme.on_accent } else { theme.text });
                }
                // Status line.
                fr.draw_text(s, r.x + 16 + 2 * 96 + 8, r.y + HEADER_H + 28, &self.editor_status, 12.0, theme.text_dim);
                // Text area.
                let area = Rect::new(r.x + 12, r.y + HEADER_H + 46, r.w - 24, r.h - HEADER_H - 58);
                s.rounded_rect(area, 8, qos_ui::rgb(0x12, 0x14, 0x1a));
                let tx = area.x + 12;
                let line_h = 18;
                let max_rows = ((area.h - 16) / line_h).max(1) as usize;
                let mono = qos_ui::rgb(0xd8, 0xdc, 0xe4);
                // Render buffer lines with a block cursor at the end; scroll to keep the tail visible.
                let mut lines: Vec<&str> = self.editor_buf.split('\n').collect();
                // The trailing element after a final '\n' is "", which is the current (empty) line.
                let total = lines.len();
                let start = total.saturating_sub(max_rows);
                let mut ty = area.y + 22;
                for (li, line) in lines.drain(..).enumerate().skip(start) {
                    let is_last = li + 1 == total;
                    let shown = if is_last { format!("{}_", line) } else { line.to_string() };
                    fr.draw_text(s, tx, ty, &shown, 14.0, mono);
                    ty += line_h;
                }
            }
            AppKind::Quantum => {
                fr.draw_text(s, bx, by, "Circuit:", 14.0, theme.text_dim);
                for (j, line) in ["q0 |0>  H  *", "q1 |0>     X"].iter().enumerate() {
                    fr.draw_text(s, bx, by + 26 + j as i32 * 22, line, 15.0, theme.text);
                }
                for (idx, label) in [(0usize, "Run Bell"), (1usize, "Run GHZ")] {
                    let b = qlab_btn_rect(r, idx);
                    s.rounded_rect(b, 9, theme.accent);
                    let lw = fr.text_width(label, 15.0);
                    fr.draw_text(s, b.x + (b.w - lw) / 2, b.y + 26, label, 15.0, theme.on_accent);
                }
                let mut ry = qlab_btn_rect(r, 0).bottom() + 26;
                if self.qlab.is_empty() {
                    fr.draw_text(s, bx, ry, "click Run to simulate + measure", 13.0, theme.text_dim);
                }
                for line in self.qlab.iter().take(7) {
                    fr.draw_text(s, bx, ry, line, 14.0, theme.text);
                    ry += 20;
                }
            }
            AppKind::Monitor => {
                let mut y = by;
                // System: real RTC time + APIC-tick uptime.
                let dt = crate::rtc::read_datetime();
                let ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
                fr.draw_text(s, bx, y, "System", 15.0, theme.accent);
                y += 24;
                fr.draw_text(s, bx + 8, y, &format!("time    {:04}-{:02}-{:02} {:02}:{:02}:{:02}", dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second), 14.0, theme.text);
                y += 20;
                fr.draw_text(s, bx + 8, y, &format!("uptime  {} s   ({} ticks)", ticks / 100, ticks), 14.0, theme.text);
                y += 30;

                // Memory: real kernel-heap usage + a bar.
                let (used, total) = crate::allocator::heap_stats();
                fr.draw_text(s, bx, y, "Memory (kernel heap)", 15.0, theme.accent);
                y += 24;
                fr.draw_text(s, bx + 8, y, &format!("used {} KiB / {} MiB", used / 1024, total / 1024 / 1024), 14.0, theme.text);
                y += 20;
                let barw = r.w - 64;
                s.rounded_rect(Rect::new(bx + 8, y, barw, 10), 5, theme.surface_alt);
                let fillw = if total > 0 { (barw as u64 * used as u64 / total as u64) as i32 } else { 0 };
                if fillw > 0 {
                    s.rounded_rect(Rect::new(bx + 8, y, fillw.max(4), 10), 5, theme.accent);
                }
                y += 30;

                // Input: live USB HID device counts.
                let (kbd, mice) = crate::xhci::hid_device_counts();
                fr.draw_text(s, bx, y, "Input (USB HID)", 15.0, theme.accent);
                y += 24;
                fr.draw_text(s, bx + 8, y, &format!("{} keyboard(s), {} mouse/mice", kbd, mice), 14.0, theme.text);
                y += 30;

                // PCI devices (real enumeration) + storage status.
                let devs = crate::pci::devices();
                fr.draw_text(s, bx, y, &format!("PCI devices ({})", devs.len()), 15.0, theme.accent);
                y += 24;
                for d in devs.iter().take(4) {
                    fr.draw_text(s, bx + 8, y, &format!("{:04x}:{:04x}  {}  {}", d.vendor_id, d.device_id, crate::pci::vendor_name(d.vendor_id), d.class_name()), 13.0, theme.text_dim);
                    y += 19;
                }
                y += 12;
                fr.draw_text(s, bx, y, "Storage", 15.0, theme.accent);
                y += 24;
                let fat = crate::fat16::is_fat16();
                fr.draw_text(s, bx + 8, y, &format!("RAM fs active   -   FAT16 disk: {}", if fat { "present" } else { "none attached" }), 14.0, theme.text_dim);
            }
            AppKind::Settings => {
                fr.draw_text(s, bx, by, "Appearance", 16.0, theme.text);
                let tr = settings_theme_rect(r);
                s.rounded_rect(tr, 9, theme.surface_alt);
                let label = if theme.is_dark { "Theme:  Dark   (click)" } else { "Theme:  Light   (click)" };
                fr.draw_text(s, tr.x + 14, tr.y + 24, label, 14.0, theme.text);
                fr.draw_text(s, bx, tr.bottom() + 40, "System", 16.0, theme.text);
                let ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
                let info1 = format!("QOS 0.1    {}x{}    heap {} MiB", self.w, self.h, crate::allocator::HEAP_SIZE / 1024 / 1024);
                let info2 = format!("uptime {} s    USB keyboard + mouse", ticks / 100);
                fr.draw_text(s, bx, tr.bottom() + 72, &info1, 14.0, theme.text_dim);
                fr.draw_text(s, bx, tr.bottom() + 94, &info2, 14.0, theme.text_dim);
            }
        }
    }

    fn compose(&self, s: &mut Surface, fr: &mut FontRenderer) {
        let theme = &self.theme;
        let (w, _h) = (self.w, self.h);
        s.gradient_v(Rect::new(0, 0, self.w, self.h), theme.wallpaper_top, theme.wallpaper_bottom);

        // Top menu bar.
        s.blend_rect(Rect::new(0, 0, w, BAR_H), theme.bar, 215);
        s.rounded_rect(Rect::new(12, 7, 16, 16), 5, theme.accent);
        fr.draw_text(s, 36, 21, "QOS", 16.0, theme.text);
        let mut mx = 84;
        for item in ["File", "Edit", "View", "Window", "Help"] {
            mx = fr.draw_text(s, mx, 21, item, 15.0, theme.text_dim) + 20;
        }
        // Theme toggle pill + clock.
        let tb = self.theme_btn();
        s.rounded_rect(tb, 11, theme.surface_alt);
        let tlabel = if theme.is_dark { "Dark" } else { "Light" };
        let tw = fr.text_width(tlabel, 13.0);
        fr.draw_text(s, tb.x + (tb.w - tw) / 2, tb.y + 16, tlabel, 13.0, theme.text);
        let clock = "12:42";
        let cw = fr.text_width(clock, 15.0);
        fr.draw_text(s, w - cw - 16, 21, clock, 15.0, theme.text);

        // Windows in z-order (topmost last = focused).
        let top = self.wins.len().saturating_sub(1);
        for i in 0..self.wins.len() {
            self.draw_window(s, fr, i, i == top);
        }

        // Dock with app icons (open apps get an accent underline dot).
        let dock = self.dock_rect();
        s.drop_shadow(dock, 20, 18, theme.shadow, if theme.is_dark { 140 } else { 70 });
        s.rounded_rect_blend(dock, 20, theme.dock, 238);
        for (i, &kind) in APPS.iter().enumerate() {
            let ir = self.dock_icon_rect(i as i32);
            s.rounded_rect(ir, 12, app_tint(kind, theme));
            let initial = app_dock_letter(kind);
            let iw = fr.text_width(initial, 22.0);
            fr.draw_text(s, ir.x + (ir.w - iw) / 2, ir.y + ir.h / 2 + 8, initial, 22.0, qos_ui::rgb(0xff, 0xff, 0xff));
            if self.wins.iter().any(|win| win.kind == kind) {
                circle(s, ir.x + ir.w / 2, ir.y + ir.h + 6, 5, theme.accent);
            }
        }
    }
}

/// Cursor bitmap dimensions (for save-under).
const CURSOR_W: usize = 11;
const CURSOR_H: usize = 16;

/// Draw the arrow cursor at `(cx, cy)` directly onto the framebuffer (only its ~90 opaque pixels),
/// over whatever scene pixels are already there. Cheap — used for save-under cursor tracking.
fn draw_cursor_fb(cx: i32, cy: i32) {
    for (row, line) in CURSOR.iter().enumerate() {
        for (col, ch) in line.bytes().enumerate() {
            let color = match ch {
                b'#' => 0x10_12_18u32,
                b'o' => 0xFF_FF_FFu32,
                _ => continue,
            };
            crate::framebuffer::put_pixel((cx + col as i32) as usize, (cy + row as i32) as usize, color);
        }
    }
}

/// Play the branded animated boot splash (WP-05 step 2): the Heptapus logo fades in, grows, holds,
/// then fades out over ~1.5 s, with a loading bar. The gradient background is drawn once; each
/// frame only the logo box and the bar are recomposed and blitted (region updates, not the whole
/// 1280×800 screen), so the animation stays smooth. A keypress skips it.
pub fn run_splash() {
    let info = match crate::framebuffer::info() {
        Some(i) => i,
        None => return,
    };
    let (w, h) = (info.width, info.height);
    let (wi, hi) = (w as i32, h as i32);
    let theme = Theme::dark();
    crate::serial_println!("[UI] boot splash: {}x{} Heptapus animation", w, h);

    // Static gradient background: composed once, blitted once, then kept as the source for the
    // per-frame region patches (so we never re-touch the whole framebuffer).
    let mut bg = Surface::new(w, h);
    bg.gradient_v(Rect::new(0, 0, wi, hi), theme.wallpaper_top, theme.wallpaper_bottom);
    crate::framebuffer::blit_region(&bg.pixels, w, 0, 0, w, h);

    // Fixed logo box (max size, centered) + bar region — the only areas that change per frame.
    let base = (hi * 44 / 100).min(wi * 44 / 100);
    let logo_box = Rect::new(wi / 2 - base / 2, hi / 2 - hi / 20 - base / 2, base, base);
    let bar_w = wi * 22 / 100;
    let bar_h = 6;
    let bar_box = Rect::new((wi - bar_w) / 2, hi * 82 / 100, bar_w, bar_h);
    let mut logo_patch = Surface::new(base as usize, base as usize);
    let mut bar_patch = Surface::new(bar_w as usize, bar_h as usize);

    const DURATION: i32 = 150; // ~1.5 s at the 100 Hz APIC tick
    let in_end = DURATION * 33 / 100;
    let hold_end = DURATION * 73 / 100;
    while crate::input::poll().is_some() {} // drop stale boot input
    let start = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64;
    let mut last_e = -1;
    loop {
        let now = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64;
        let e = (now - start) as i32;
        if e >= DURATION {
            break;
        }
        if let Some(crate::input::InputEvent::Key { pressed: true, .. }) = crate::input::poll() {
            break;
        }
        if e != last_e {
            let alpha = if e < in_end {
                e * 255 / in_end
            } else if e < hold_end {
                255
            } else {
                255 - (e - hold_end) * 255 / (DURATION - hold_end).max(1)
            }
            .clamp(0, 255) as u8;
            let scale_pct = if e < in_end { 82 + 18 * e / in_end } else { 100 };

            // Logo patch: restore gradient under the box, draw the scaled+faded logo, blit the box.
            logo_patch.blit(&bg, -logo_box.x, -logo_box.y);
            let size = base * scale_pct / 100;
            let off = (base - size) / 2;
            logo_patch.blit_mask_scaled(LOGO_MASK, LOGO_W, LOGO_H, Rect::new(off, off, size, size), theme.text, alpha);
            crate::framebuffer::blit_at(&logo_patch.pixels, logo_patch.width, logo_patch.height, logo_box.x as usize, logo_box.y as usize);

            // Bar patch: gradient + track + accent fill.
            bar_patch.blit(&bg, -bar_box.x, -bar_box.y);
            bar_patch.rounded_rect(Rect::new(0, 0, bar_w, bar_h), 3, theme.surface_alt);
            let fill = (bar_w * e / DURATION).clamp(0, bar_w);
            if fill > 0 {
                bar_patch.rounded_rect(Rect::new(0, 0, fill, bar_h), 3, theme.accent);
            }
            crate::framebuffer::blit_at(&bar_patch.pixels, bar_patch.width, bar_patch.height, bar_box.x as usize, bar_box.y as usize);
            last_e = e;
        }
        crate::arch::hlt();
    }
    crate::framebuffer::clear(0x000000);
    crate::framebuffer::reset_cursor();
}

/// Boot chooser shown after the splash (WP-05): pick the **Modern Desktop** or the **Terminal
/// (shell)**. Keyboard `Enter`/`D`/`1` or a click on the left card → desktop (`true`); `S`/`2` or
/// the right card → shell (`false`); `Esc` → shell. If nothing is pressed it defaults to the
/// desktop after a short countdown, so the UI comes up on its own. No-op → `true` without a
/// framebuffer.
/// Draw the boot-chooser scene (no cursor) into `s`: gradient, logo, title, the two cards, and the
/// countdown. Factored out so the loop can redraw it only when the countdown ticks, using cheap
/// cursor save-under for mouse movement in between.
fn draw_chooser_scene(
    s: &mut Surface,
    fr: &mut FontRenderer,
    theme: &Theme,
    wi: i32,
    hi: i32,
    desktop_card: Rect,
    shell_card: Rect,
    card_w: i32,
    sec: i32,
) {
    s.gradient_v(Rect::new(0, 0, wi, hi), theme.wallpaper_top, theme.wallpaper_bottom);
    let logo = 150;
    s.blit_mask_scaled(LOGO_MASK, LOGO_W, LOGO_H, Rect::new(wi / 2 - logo / 2, hi / 6, logo, logo), theme.text, 255);
    let title = "Welcome to QOS";
    let tw = fr.text_width(title, 34.0);
    fr.draw_text(s, wi / 2 - tw / 2, hi / 6 + logo + 40, title, 34.0, theme.text);
    let sub = "Choose how to start";
    let sw = fr.text_width(sub, 17.0);
    fr.draw_text(s, wi / 2 - sw / 2, hi / 6 + logo + 72, sub, 17.0, theme.text_dim);

    s.drop_shadow(desktop_card, 16, 18, theme.shadow, 130);
    s.rounded_rect(desktop_card, 16, theme.accent);
    let d1 = "Modern Desktop";
    let d1w = fr.text_width(d1, 22.0);
    fr.draw_text(s, desktop_card.x + (card_w - d1w) / 2, desktop_card.y + 88, d1, 22.0, theme.on_accent);
    let d2 = "Enter  /  D";
    let d2w = fr.text_width(d2, 15.0);
    fr.draw_text(s, desktop_card.x + (card_w - d2w) / 2, desktop_card.y + 130, d2, 15.0, theme.on_accent);

    s.drop_shadow(shell_card, 16, 18, theme.shadow, 100);
    s.rounded_rect(shell_card, 16, theme.surface);
    let s1 = "Terminal";
    let s1w = fr.text_width(s1, 22.0);
    fr.draw_text(s, shell_card.x + (card_w - s1w) / 2, shell_card.y + 88, s1, 22.0, theme.text);
    let s2 = "S";
    let s2w = fr.text_width(s2, 15.0);
    fr.draw_text(s, shell_card.x + (card_w - s2w) / 2, shell_card.y + 130, s2, 15.0, theme.text_dim);

    let mut buf = [0u8; 48];
    let hint = fmt_countdown(&mut buf, sec.max(0));
    let hw = fr.text_width(hint, 14.0);
    fr.draw_text(s, wi / 2 - hw / 2, shell_card.bottom() + 44, hint, 14.0, theme.text_dim);
}

pub fn boot_choice() -> bool {
    let info = match crate::framebuffer::info() {
        Some(i) => i,
        None => return false,
    };
    let (w, h) = (info.width, info.height);
    let (wi, hi) = (w as i32, h as i32);
    let mut fr = match Font::parse(qos_ui::font::DEFAULT_FONT) {
        Some(f) => FontRenderer::new(f),
        None => return true,
    };
    let mut surface = Surface::new(w, h);
    let theme = Theme::dark();

    let card_w = 300;
    let card_h = 190;
    let gap = 44;
    let cy = hi / 2 + 95; // below the title + subtitle
    let desktop_card = Rect::new(wi / 2 - card_w - gap / 2, cy - card_h / 2, card_w, card_h);
    let shell_card = Rect::new(wi / 2 + gap / 2, cy - card_h / 2, card_w, card_h);

    const TIMEOUT: i32 = 800; // ~8 s at 100 Hz, then default to the desktop
    // flush stale input
    for _ in 0..20 {
        crate::xhci::poll();
        while crate::input::poll().is_some() {}
        crate::arch::hlt();
    }
    let start = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64;
    let mut cursor = (wi / 2, hi / 2);
    let mut last_cursor = cursor;

    // Compose + blit the scene once, then overlay the cursor.
    let first_sec = TIMEOUT / 100;
    draw_chooser_scene(&mut surface, &mut fr, &theme, wi, hi, desktop_card, shell_card, card_w, first_sec);
    crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);
    draw_cursor_fb(cursor.0, cursor.1);
    let mut last_sec = first_sec;

    loop {
        crate::xhci::poll();
        while let Some(ev) = crate::input::poll() {
            match ev {
                crate::input::InputEvent::Key { scancode, pressed: true } => match scancode {
                    0x1C | 0x20 | 0x02 => return true,  // Enter / D / 1 → desktop
                    0x1F | 0x03 | 0x01 => return false, // S / 2 / Esc → shell
                    _ => {}
                },
                crate::input::InputEvent::MouseMove { dx, dy } => {
                    cursor.0 = (cursor.0 + dx as i32).clamp(0, wi - 1);
                    cursor.1 = (cursor.1 - dy as i32).clamp(0, hi - 1);
                }
                crate::input::InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: true } => {
                    if desktop_card.contains(cursor.0, cursor.1) {
                        return true;
                    }
                    if shell_card.contains(cursor.0, cursor.1) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        let elapsed = (crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64 - start) as i32;
        if elapsed >= TIMEOUT {
            return true;
        }
        let sec = (TIMEOUT - elapsed) / 100;
        if sec != last_sec {
            // Countdown ticked: recompose the scene (full) and redraw the cursor on top.
            draw_chooser_scene(&mut surface, &mut fr, &theme, wi, hi, desktop_card, shell_card, card_w, sec);
            crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);
            last_cursor = cursor;
            draw_cursor_fb(last_cursor.0, last_cursor.1);
            last_sec = sec;
        } else if cursor != last_cursor {
            // Cursor-only move: cheap save-under, no full recompose.
            crate::framebuffer::blit_region(&surface.pixels, w, last_cursor.0 as usize, last_cursor.1 as usize, CURSOR_W, CURSOR_H);
            last_cursor = cursor;
            draw_cursor_fb(last_cursor.0, last_cursor.1);
        }
        crate::arch::hlt();
    }
}

/// Format "Starting the desktop in Ns..." into `buf`, returning the &str (no_std, no alloc).
fn fmt_countdown(buf: &mut [u8; 48], sec: i32) -> &str {
    let prefix = b"Starting the desktop in ";
    let mut n = 0;
    for &b in prefix {
        buf[n] = b;
        n += 1;
    }
    let s = sec.clamp(0, 99);
    if s >= 10 {
        buf[n] = b'0' + (s / 10) as u8;
        n += 1;
    }
    buf[n] = b'0' + (s % 10) as u8;
    n += 1;
    for &b in b"s..." {
        buf[n] = b;
        n += 1;
    }
    core::str::from_utf8(&buf[..n]).unwrap_or("Starting...")
}

/// Run the interactive modern desktop (opt-in via the `modern` shell command, WP-05 step 4).
/// Mouse: drag windows by their title bar, click the red dot to close, click a dock icon to open an
/// app, click the top-right pill to toggle light/dark. Keyboard: `1`–`4` open apps, `w` closes the
/// focused window, `t` toggles the theme, `Esc` returns to the shell. Only redraws when something
/// changes (dirty flag) — idle frames just `hlt`.
pub fn run_demo() {
    use crate::input::{InputEvent, MouseButton};
    let info = match crate::framebuffer::info() {
        Some(i) => i,
        None => {
            crate::println!("modern: no linear framebuffer (UEFI only)");
            return;
        }
    };
    let (w, h) = (info.width, info.height);
    let mut fr = match Font::parse(qos_ui::font::DEFAULT_FONT) {
        Some(f) => FontRenderer::new(f),
        None => {
            crate::println!("modern: font parse failed");
            return;
        }
    };
    let mut surface = Surface::new(w, h);
    let mut desk = Desktop::new(w as i32, h as i32);
    crate::serial_println!("[UI] modern desktop: {}x{} interactive window manager", w, h);
    // Flush input left over from the launching command (e.g. the Enter key-up USB report still in
    // flight): pump USB and drain the queue over ~250 ms so nothing leaks into the interactive loop.
    for _ in 0..25 {
        crate::xhci::poll();
        while crate::input::poll().is_some() {}
        crate::arch::hlt();
    }

    // `surface` holds the composed desktop **without** the cursor; it mirrors what's on the
    // framebuffer minus the cursor overlay. `last_cursor` is where the cursor was last drawn.
    desk.compose(&mut surface, &mut fr);
    crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);
    let mut last_cursor = desk.cursor;
    draw_cursor_fb(last_cursor.0, last_cursor.1);
    desk.dirty = false;
    desk.full = false;
    desk.damage = Rect::new(0, 0, 0, 0);
    let mut shift = false;
    let mut last_refresh = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64;

    loop {
        // Pump USB HID here too: `run_demo` runs synchronously (it blocks the scheduler loop that
        // normally queues the interrupt-IN report TRB), so drive it directly to keep USB keyboard +
        // mouse alive on the desktop. Cheap (try_lock) and harmless alongside the scheduler.
        crate::xhci::poll();
        while let Some(ev) = crate::input::poll() {
            match ev {
                InputEvent::Key { scancode, pressed } => {
                    // Track Shift (either side) from both press and release.
                    if scancode & 0x7F == 0x2A || scancode & 0x7F == 0x36 {
                        shift = pressed;
                        continue;
                    }
                    if !pressed {
                        continue;
                    }
                    if scancode == 0x01 {
                        // Esc cancels an open naming modal first; otherwise drops to the shell.
                        if desk.files_naming_active() {
                            desk.files_cancel_name();
                            continue;
                        }
                        crate::framebuffer::clear(0x000000);
                        crate::framebuffer::reset_cursor();
                        return; // Esc → shell (from anywhere)
                    }
                    if desk.files_naming_active() {
                        // The Files naming modal captures typing.
                        match scancode {
                            0x0E => {
                                desk.files_name_buf.pop();
                                desk.mark_top_window();
                            }
                            0x1C => desk.files_commit_name(),
                            _ => {
                                if let Some(c) = scancode_to_char(scancode, shift) {
                                    if desk.files_name_buf.len() < 32 {
                                        desk.files_name_buf.push(c);
                                        desk.mark_top_window();
                                    }
                                }
                            }
                        }
                    } else if desk.top_is_terminal() {
                        // Focused Terminal captures typing.
                        match scancode {
                            0x0E => {
                                desk.term.backspace();
                                desk.mark_top_window();
                            } // Backspace
                            0x1C => {
                                desk.term.enter();
                                desk.mark_top_window();
                            } // Enter
                            _ => {
                                if let Some(c) = scancode_to_char(scancode, shift) {
                                    desk.term.type_char(c);
                                    desk.mark_top_window();
                                }
                            }
                        }
                    } else if desk.top_is_editor() {
                        // Focused Text Editor edits its buffer.
                        match scancode {
                            0x0E => {
                                desk.editor_buf.pop();
                                desk.mark_top_window();
                            } // Backspace
                            0x1C => {
                                desk.editor_buf.push('\n');
                                desk.mark_top_window();
                            } // Enter → newline
                            _ => {
                                if let Some(c) = scancode_to_char(scancode, shift) {
                                    if desk.editor_buf.len() < 32 * 1024 {
                                        desk.editor_buf.push(c);
                                        desk.mark_top_window();
                                    }
                                }
                            }
                        }
                    } else {
                        // Desktop shortcuts when no text-entry window is focused. When Files is the
                        // focused window, letter keys drive its file-manager operations (a real
                        // keyboard-driven file manager); app-switch numbers + t/w still work.
                        let files_focused =
                            desk.wins.last().map_or(false, |w| w.kind == AppKind::Files);
                        match scancode {
                            0x14 => {
                                desk.theme = desk.theme.toggled();
                                desk.mark_full();
                            } // t
                            s @ 0x02..=0x07 => {
                                // Number keys 1–6 open the dock apps in order.
                                let idx = (s - 0x02) as usize;
                                if idx < APPS.len() {
                                    desk.open_app(APPS[idx]);
                                }
                            }
                            0x11 => {
                                if desk.wins.pop().is_some() {
                                    desk.mark_full();
                                }
                            } // w → close focused
                            0x31 if files_focused => desk.files_tool(0), // n → New File
                            0x25 if files_focused => desk.files_tool(1), // k → New Dir
                            0x13 if files_focused => desk.files_tool(2), // r → Rename (selection)
                            0x2D if files_focused => desk.files_tool(3), // x → Delete (selection)
                            0x12 if files_focused => desk.files_tool(4), // e → Edit (selection)
                            _ => {}
                        }
                    }
                }
                InputEvent::MouseMove { dx, dy } => desk.on_mouse_move(dx, dy),
                InputEvent::MouseButton { button: MouseButton::Left, pressed } => {
                    if pressed {
                        desk.on_left_down();
                    } else {
                        desk.on_left_up();
                    }
                }
                _ => {}
            }
        }
        // Live refresh: if the System Monitor is focused, repaint its window ~once a second so its
        // clock / memory / uptime stay current.
        let now_ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64;
        if desk.top_is_monitor() && now_ticks - last_refresh >= 100 {
            last_refresh = now_ticks;
            desk.mark_top_window();
        }

        if desk.dirty {
            // Scene changed. For a drag, confine BOTH the recompose (RAM) and the blit (slow
            // framebuffer MMIO) to the damage rect — the union of old+new window footprints plus
            // the cursor's old+new spots. Full screen only for z-order/theme/open/close.
            if desk.full {
                surface.set_clip(None);
                desk.compose(&mut surface, &mut fr);
                crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);
            } else {
                let cur_old = Rect::new(last_cursor.0, last_cursor.1, CURSOR_W as i32, CURSOR_H as i32);
                let cur_new = Rect::new(desk.cursor.0, desk.cursor.1, CURSOR_W as i32, CURSOR_H as i32);
                let region = desk.damage.union(&cur_old).union(&cur_new);
                if let Some(r) = region.intersect(&Rect::new(0, 0, w as i32, h as i32)) {
                    surface.set_clip(Some(r));
                    desk.compose(&mut surface, &mut fr);
                    surface.set_clip(None);
                    crate::framebuffer::blit_region(&surface.pixels, w, r.x as usize, r.y as usize, r.w as usize, r.h as usize);
                }
            }
            last_cursor = desk.cursor;
            draw_cursor_fb(last_cursor.0, last_cursor.1);
            desk.dirty = false;
            desk.full = false;
            desk.damage = Rect::new(0, 0, 0, 0);
        } else if desk.cursor != last_cursor {
            // Cursor-only move: restore the scene under the old cursor (small blit) and draw the
            // cursor at the new spot. No full recompose — this is what keeps the pointer smooth.
            crate::framebuffer::blit_region(&surface.pixels, w, last_cursor.0 as usize, last_cursor.1 as usize, CURSOR_W, CURSOR_H);
            last_cursor = desk.cursor;
            draw_cursor_fb(last_cursor.0, last_cursor.1);
        }
        crate::arch::hlt();
    }
}
