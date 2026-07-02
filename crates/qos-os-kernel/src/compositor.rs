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
    Quantum,
    Settings,
}

const APPS: [AppKind; 4] = [AppKind::Terminal, AppKind::Files, AppKind::Quantum, AppKind::Settings];

fn app_title(k: AppKind) -> &'static str {
    match k {
        AppKind::Terminal => "Terminal",
        AppKind::Files => "Files",
        AppKind::Quantum => "Quantum Lab",
        AppKind::Settings => "Settings",
    }
}

fn app_tint(k: AppKind, theme: &Theme) -> qos_ui::Rgb {
    match k {
        AppKind::Terminal => theme.accent,
        AppKind::Files => qos_ui::rgb(0x30, 0xb0, 0x60),
        AppKind::Quantum => qos_ui::rgb(0x8a, 0x5c, 0xd8),
        AppKind::Settings => qos_ui::rgb(0xe0, 0x7a, 0x2a),
    }
}

/// A filled circle via a maximally-rounded square (used for the macOS-style window dots + dock).
fn circle(s: &mut Surface, cx: i32, cy: i32, d: i32, color: qos_ui::Rgb) {
    s.rounded_rect(Rect::new(cx - d / 2, cy - d / 2, d, d), d / 2, color);
}

// Per-app clickable geometry, shared by drawing + hit-testing so they stay in sync (`win` = window
// rect). Files entry rows, Quantum Lab run buttons, and the Settings theme toggle.
fn files_row_rect(win: Rect, i: usize) -> Rect {
    Rect::new(win.x + 16, win.y + HEADER_H + 42 + i as i32 * 28, win.w - 32, 24)
}
fn qlab_btn_rect(win: Rect, i: usize) -> Rect {
    Rect::new(win.x + 24 + i as i32 * 150, win.y + HEADER_H + 108, 132, 40)
}
fn settings_theme_rect(win: Rect) -> Rect {
    Rect::new(win.x + 24, win.y + HEADER_H + 30, 220, 36)
}
const FILES_MAX_ROWS: usize = 6;

/// An open window on the desktop.
struct Win {
    rect: Rect,
    kind: AppKind,
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
}

impl Terminal {
    fn new() -> Self {
        let mut t = Terminal { lines: Vec::new(), input: String::new() };
        t.push("QOS Terminal — type 'help'.".to_string());
        t
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

    fn enter(&mut self) {
        let cmd = core::mem::take(&mut self.input);
        self.push(format!("qos:\\> {}", cmd));
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
                    "  help clear echo ver mem",
                    "  bell            2-qubit Bell state, 1000 shots",
                    "  ghz             3-qubit GHZ state, 1000 shots",
                    "  qrng [n]        n quantum random bits (default 8)",
                ] {
                    self.push(l.to_string());
                }
            }
            "clear" => self.lines.clear(),
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
    /// Quantum Lab: last run's result lines.
    qlab: Vec<String>,
}

/// Margin around a window rect that its shadow extends into (for damage rects).
const SHADOW_MARGIN: i32 = 34;

impl Desktop {
    fn new(w: i32, h: i32) -> Self {
        // Start with two cascaded windows so the desktop looks alive.
        let wins = vec![
            Win { rect: Rect::new(w / 2 - 440, 84, 540, 400), kind: AppKind::Terminal },
            Win { rect: Rect::new(w / 2 - 40, 250, 520, 400), kind: AppKind::Files },
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
            qlab: Vec::new(),
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
                let list = self.files_list();
                for (i, (name, is_dir, _)) in list.iter().enumerate().take(FILES_MAX_ROWS) {
                    if files_row_rect(wr, i).contains(cx, cy) {
                        let (n, d) = (name.clone(), *is_dir);
                        self.files_click(&n, d);
                        return;
                    }
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
            AppKind::Terminal => {}
        }
    }

    /// Navigate the Files app into `name` (a subdir) or up (`..`), or preview a file.
    fn files_click(&mut self, name: &str, is_dir: bool) {
        if name == ".." {
            // Go up one path segment.
            if let Some(pos) = self.files_cwd.rfind('/') {
                self.files_cwd.truncate(pos);
            } else {
                self.files_cwd.clear();
            }
            self.files_preview = None;
        } else if is_dir {
            if !self.files_cwd.is_empty() {
                self.files_cwd.push('/');
            }
            self.files_cwd.push_str(name);
            self.files_preview = None;
        } else {
            // Preview the file's text.
            let path = if self.files_cwd.is_empty() {
                name.to_string()
            } else {
                format!("{}/{}", self.files_cwd, name)
            };
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
            let rect = Rect::new((self.w / 2 - 270 + n * 28).max(20), (96 + n * 28).min(self.h - 420), 540, 400);
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
                    let col = if line.starts_with("qos:\\>") { green } else { theme.text };
                    fr.draw_text(s, tx, ty, line, 14.0, col);
                    ty += line_h;
                }
                let prompt = format!("qos:\\> {}_", self.term.input);
                fr.draw_text(s, tx, ty, &prompt, 14.0, green);
            }
            AppKind::Files => {
                // Real listing of the current directory (the in-kernel filesystem).
                let path = if self.files_cwd.is_empty() { "/".to_string() } else { format!("/{}", self.files_cwd) };
                fr.draw_text(s, bx, r.y + HEADER_H + 26, &path, 14.0, theme.text_dim);
                let list = self.files_list();
                for (i, (name, is_dir, size)) in list.iter().take(FILES_MAX_ROWS).enumerate() {
                    let rr = files_row_rect(r, i);
                    s.rounded_rect(rr, 6, theme.surface_alt);
                    let icon = if *is_dir { "[D]" } else { "[F]" };
                    fr.draw_text(s, rr.x + 10, rr.y + 17, icon, 13.0, if *is_dir { theme.accent } else { theme.text_dim });
                    fr.draw_text(s, rr.x + 46, rr.y + 17, name, 14.0, theme.text);
                    if !*is_dir {
                        let sz = format!("{} B", size);
                        let sw = fr.text_width(&sz, 12.0);
                        fr.draw_text(s, rr.right() - sw - 12, rr.y + 17, &sz, 12.0, theme.text_dim);
                    }
                }
                if let Some(prev) = &self.files_preview {
                    let rows = list.len().min(FILES_MAX_ROWS);
                    let py = files_row_rect(r, rows).y + 6;
                    s.fill_rect(Rect::new(r.x + 16, py, r.w - 32, 1), theme.border);
                    let mut ly = py + 22;
                    for line in prev.split('\n').take(5) {
                        fr.draw_text(s, r.x + 20, ly, line, 13.0, theme.text_dim);
                        ly += 18;
                    }
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
            let initial = &app_title(kind)[..1];
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
                        crate::framebuffer::clear(0x000000);
                        crate::framebuffer::reset_cursor();
                        return; // Esc → shell (from anywhere)
                    }
                    if desk.top_is_terminal() {
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
                    } else {
                        // Desktop shortcuts when no terminal is focused.
                        match scancode {
                            0x14 => {
                                desk.theme = desk.theme.toggled();
                                desk.mark_full();
                            } // t
                            0x02 => desk.open_app(AppKind::Terminal), // 1
                            0x03 => desk.open_app(AppKind::Files),    // 2
                            0x04 => desk.open_app(AppKind::Quantum),  // 3
                            0x05 => desk.open_app(AppKind::Settings), // 4
                            0x11 => {
                                if desk.wins.pop().is_some() {
                                    desk.mark_full();
                                }
                            } // w → close focused
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
