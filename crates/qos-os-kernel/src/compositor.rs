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
    Qasm,
    Monitor,
    Settings,
    Calculator,
    Devices,
    Processes,
}

const APPS: [AppKind; 10] = [
    AppKind::Terminal,
    AppKind::Files,
    AppKind::Editor,
    AppKind::Quantum,
    AppKind::Qasm,
    AppKind::Monitor,
    AppKind::Settings,
    AppKind::Calculator,
    AppKind::Devices,
    AppKind::Processes,
];

fn app_title(k: AppKind) -> &'static str {
    match k {
        AppKind::Terminal => "Terminal",
        AppKind::Files => "Files",
        AppKind::Editor => "Text Editor",
        AppKind::Quantum => "Quantum Lab",
        AppKind::Qasm => "QASM Studio",
        AppKind::Monitor => "System Monitor",
        AppKind::Settings => "Settings",
        AppKind::Calculator => "Calculator",
        AppKind::Devices => "Devices",
        AppKind::Processes => "Processes",
    }
}

fn app_tint(k: AppKind, theme: &Theme) -> qos_ui::Rgb {
    match k {
        AppKind::Terminal => theme.accent,
        AppKind::Files => qos_ui::rgb(0x30, 0xb0, 0x60),
        AppKind::Editor => qos_ui::rgb(0xd0, 0x9a, 0x2a),
        AppKind::Quantum => qos_ui::rgb(0x8a, 0x5c, 0xd8),
        AppKind::Qasm => qos_ui::rgb(0x20, 0xa0, 0x8a),
        AppKind::Monitor => qos_ui::rgb(0x27, 0xa8, 0xc8),
        AppKind::Settings => qos_ui::rgb(0xe0, 0x7a, 0x2a),
        AppKind::Calculator => qos_ui::rgb(0x50, 0x60, 0xd8),
        AppKind::Devices => qos_ui::rgb(0x9a, 0x8a, 0x40),
        AppKind::Processes => qos_ui::rgb(0xc8, 0x40, 0x70),
    }
}

/// A filled circle via a maximally-rounded square (used for the macOS-style window dots + dock).
fn circle(s: &mut Surface, cx: i32, cy: i32, d: i32, color: qos_ui::Rgb) {
    s.rounded_rect(Rect::new(cx - d / 2, cy - d / 2, d, d), d / 2, color);
}

/// Draw a real (vector-drawn) app icon glyph inside `r` (the tinted tile). All glyphs are built
/// from the AA primitives (rects + circles) so they stay crisp at any tile size — no bitmaps.
fn draw_app_icon(s: &mut Surface, kind: AppKind, r: Rect, tint: qos_ui::Rgb) {
    let white = qos_ui::rgb(0xff, 0xff, 0xff);
    let faint = qos_ui::rgb(0xe8, 0xec, 0xf4);
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    let u = r.w / 12; // icon unit
    match kind {
        AppKind::Terminal => {
            // Dark screen + green prompt line and block cursor.
            s.rounded_rect(Rect::new(r.x + 2 * u, r.y + 3 * u, 8 * u, 6 * u), u, qos_ui::rgb(0x10, 0x12, 0x18));
            let green = qos_ui::rgb(0x6e, 0xe0, 0x7a);
            s.fill_rect(Rect::new(r.x + 3 * u, cy - u / 4, 2 * u, u / 2), green);
            s.fill_rect(Rect::new(r.x + 6 * u, cy - u / 4, u, u), green);
        }
        AppKind::Files => {
            // Folder: tab + body.
            s.rounded_rect(Rect::new(r.x + 2 * u, r.y + 3 * u, 4 * u, 2 * u), u / 2, faint);
            s.rounded_rect(Rect::new(r.x + 2 * u, r.y + 4 * u, 8 * u, 5 * u), u / 2, white);
        }
        AppKind::Editor => {
            // Document sheet + text lines.
            s.rounded_rect(Rect::new(cx - 3 * u, r.y + 2 * u, 6 * u, 8 * u), u / 2, white);
            for i in 0..3 {
                s.fill_rect(Rect::new(cx - 2 * u, r.y + 4 * u + i * u + i * u / 2, 4 * u, u / 2), tint);
            }
        }
        AppKind::Quantum => {
            // Atom: ring (white circle with a tint punch-out) + nucleus + electron.
            circle(s, cx, cy, 9 * u, white);
            circle(s, cx, cy, 7 * u, tint);
            circle(s, cx, cy, 3 * u, white);
            circle(s, cx + 4 * u, cy - 3 * u, u * 2, white);
        }
        AppKind::Monitor => {
            // Bar chart.
            let heights = [3, 6, 4, 7];
            for (i, h) in heights.iter().enumerate() {
                let bh = *h * u;
                s.rounded_rect(Rect::new(r.x + 2 * u + i as i32 * 2 * u, r.y + 9 * u - bh, u + u / 2, bh), u / 3, white);
            }
        }
        AppKind::Settings => {
            // GNOME-style sliders: three tracks with offset knobs.
            for (i, kx) in [3, 7, 5].iter().enumerate() {
                let ly = r.y + (3 + i as i32 * 2 + i as i32 / 2) * u + u / 2;
                s.rounded_rect(Rect::new(r.x + 2 * u, ly, 8 * u, u / 2), u / 4, faint);
                circle(s, r.x + *kx * u, ly + u / 4, u * 2, white);
            }
        }
        AppKind::Calculator => {
            // Display bar + key dots.
            s.rounded_rect(Rect::new(r.x + 2 * u, r.y + 2 * u, 8 * u, 2 * u), u / 2, white);
            for row in 0..2 {
                for col in 0..3 {
                    circle(s, r.x + 3 * u + col * 3 * u, r.y + 6 * u + row * 3 * u, u + u / 2, faint);
                }
            }
        }
        AppKind::Devices => {
            // Chip: outlined die + pins.
            s.rounded_rect(Rect::new(cx - 3 * u, cy - 3 * u, 6 * u, 6 * u), u / 2, white);
            s.rounded_rect(Rect::new(cx - 2 * u, cy - 2 * u, 4 * u, 4 * u), u / 3, tint);
            for i in 0..3 {
                let px = cx - 2 * u + i * 2 * u;
                s.fill_rect(Rect::new(px, cy - 4 * u - u / 2, u / 2, u + u / 2), white);
                s.fill_rect(Rect::new(px, cy + 3 * u, u / 2, u + u / 2), white);
            }
        }
        AppKind::Processes => {
            // Task list: bullet + line rows.
            for i in 0..3 {
                let ly = r.y + 3 * u + i * 2 * u + i * u / 2;
                circle(s, r.x + 3 * u, ly + u / 2, u + u / 4, white);
                s.rounded_rect(Rect::new(r.x + 5 * u, ly + u / 4, 5 * u, u / 2), u / 4, faint);
            }
        }
        AppKind::Qasm => {
            // Code block: dark editor pane + colored code lines (a quantum program).
            s.rounded_rect(Rect::new(r.x + 2 * u, r.y + 2 * u, 8 * u, 8 * u), u, qos_ui::rgb(0x10, 0x12, 0x18));
            let colors = [white, qos_ui::rgb(0x8a, 0x5c, 0xd8), faint, qos_ui::rgb(0x6e, 0xe0, 0x7a)];
            let widths = [5, 4, 6, 3];
            for i in 0..4 {
                s.fill_rect(
                    Rect::new(r.x + 3 * u, r.y + 3 * u + i as i32 * u + i as i32 * u / 2, widths[i] * u, u / 2),
                    colors[i],
                );
            }
        }
    }
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
/// The two Appearance theme cards (0 = Dark, 1 = Light) in the Settings window.
fn settings_card_rect(win: Rect, i: usize) -> Rect {
    let cw = (win.w - 48 - 16) / 2;
    Rect::new(win.x + 24 + i as i32 * (cw + 16), win.y + HEADER_H + 42, cw, 86)
}
/// Text Editor action buttons (Save / New).
fn editor_btn_rect(win: Rect, i: usize) -> Rect {
    Rect::new(win.x + 16 + i as i32 * 96, win.y + HEADER_H + 10, 88, 26)
}
/// Quantum IDE action buttons (0 = Compile, 1 = Run, 2 = Save).
fn qasm_btn_rect(win: Rect, i: usize) -> Rect {
    Rect::new(win.x + 16 + i as i32 * 96, win.y + HEADER_H + 10, 88, 26)
}
// Quantum IDE layout (VS Code-like): sidebar | code pane, preview strip + status bar below.
const QASM_PREVIEW_H: i32 = 96;
const QASM_STATUS_H: i32 = 20;
fn qasm_side_rect(win: Rect) -> Rect {
    let top = win.y + HEADER_H + 44;
    Rect::new(win.x + 10, top, 150, win.bottom() - top - QASM_PREVIEW_H - QASM_STATUS_H - 18)
}
fn qasm_side_row_rect(win: Rect, i: usize) -> Rect {
    let s = qasm_side_rect(win);
    Rect::new(s.x + 4, s.y + 26 + i as i32 * 22, s.w - 8, 20)
}
fn qasm_code_rect(win: Rect) -> Rect {
    let s = qasm_side_rect(win);
    Rect::new(s.right() + 6, s.y, win.right() - s.right() - 16, s.h)
}
fn qasm_preview_rect(win: Rect) -> Rect {
    let s = qasm_side_rect(win);
    Rect::new(win.x + 10, s.bottom() + 6, win.w - 20, QASM_PREVIEW_H)
}
const FILES_MAX_ROWS: usize = 5;

/// Calculator display field + button grid + Clear (shared by drawing and hit-testing).
fn calc_display_rect(win: Rect) -> Rect {
    Rect::new(win.x + 24, win.y + HEADER_H + 16, win.w - 48, 44)
}
fn calc_btn_rect(win: Rect, i: usize) -> Rect {
    let (row, col) = (i as i32 / 4, i as i32 % 4);
    let gap = 10;
    let bw = (win.w - 48 - 3 * gap) / 4;
    let bh = 52;
    Rect::new(win.x + 24 + col * (bw + gap), win.y + HEADER_H + 76 + row * (bh + gap), bw, bh)
}
fn calc_clear_rect(win: Rect) -> Rect {
    let g = calc_btn_rect(win, 12); // row 3 for vertical placement
    Rect::new(win.x + 24, g.bottom() + 10, win.w - 48, 34)
}

/// An open window on the desktop.
struct Win {
    rect: Rect,
    kind: AppKind,
    /// Hidden from the desktop (yellow dot); restored via its dock icon.
    minimized: bool,
    /// The pre-maximize rect while maximized (green dot toggles).
    saved: Option<Rect>,
}

impl Win {
    fn new(rect: Rect, kind: AppKind) -> Self {
        Win { rect, kind, minimized: false, saved: None }
    }
}

/// Which kind of name the Files naming modal is collecting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NameMode {
    NewFile,
    NewDir,
    Rename,
}

/// Quantum Lab circuit-editor gate kinds (palette order).
#[derive(Clone, Copy, PartialEq, Eq)]
enum QG {
    H,
    X,
    Y,
    Z,
    S,
    T,
    Rx,
    Ry,
    Rz,
    Cx,
}

const QLAB_PALETTE: [(QG, &str); 10] = [
    (QG::H, "H"),
    (QG::X, "X"),
    (QG::Y, "Y"),
    (QG::Z, "Z"),
    (QG::S, "S"),
    (QG::T, "T"),
    (QG::Rx, "RX"),
    (QG::Ry, "RY"),
    (QG::Rz, "RZ"),
    (QG::Cx, "CX"),
];

/// A placed gate: `q` is the (control) wire, `q2` the CX target (== q for single-qubit gates),
/// `angle` is the R-gate angle in units of π/4 (1..=8).
#[derive(Clone, Copy)]
struct QGate {
    kind: QG,
    q: usize,
    q2: usize,
    col: usize,
    angle: u8,
}

const QLAB_COLS: usize = 8;
const QLAB_MAX_Q: usize = 5;
const QLAB_SHOTS: u64 = 1000;

// Quantum Lab geometry (shared by draw + hit-test).
fn qlab_pal_rect(win: Rect, i: usize) -> Rect {
    Rect::new(win.x + 14 + i as i32 * 37, win.y + HEADER_H + 8, 33, 24)
}
/// Control buttons: 0 = Run, 1 = Clear, 2 = Q-, 3 = Q+, 4 = angle cycler, 5 = export to QASM.
fn qlab_ctl_rect(win: Rect, i: usize) -> Rect {
    let (x, w) = match i {
        0 => (14, 64),
        1 => (84, 56),
        2 => (146, 28),
        3 => (180, 28),
        4 => (214, 96),
        _ => (316, 62),
    };
    Rect::new(win.x + x, win.y + HEADER_H + 38, w, 24)
}
fn qlab_cell_rect(win: Rect, q: usize, col: usize) -> Rect {
    Rect::new(win.x + 44 + col as i32 * 44 + 3, win.y + HEADER_H + 72 + q as i32 * 34, 38, 28)
}
/// Angle label for `steps` × π/4 (indices 1..=8).
fn qlab_angle_label(steps: u8) -> &'static str {
    match steps {
        1 => "pi/4",
        2 => "pi/2",
        3 => "3pi/4",
        4 => "pi",
        5 => "5pi/4",
        6 => "3pi/2",
        7 => "7pi/4",
        _ => "2pi",
    }
}

/// Calculator button grid (4×4) + a wide Clear; shared by drawing and hit-testing.
const CALC_BTNS: [&str; 16] = [
    "7", "8", "9", "/", "4", "5", "6", "*", "1", "2", "3", "-", "0", ".", "=", "+",
];

/// A standard immediate-execution calculator.
struct Calc {
    display: String,
    acc: f64,
    pending: Option<char>,
    /// The next digit starts a fresh number (after an operator or `=`).
    reset_next: bool,
}

impl Calc {
    fn new() -> Self {
        Calc { display: "0".to_string(), acc: 0.0, pending: None, reset_next: true }
    }

    fn value(&self) -> f64 {
        self.display.parse::<f64>().unwrap_or(0.0)
    }

    /// Format a result: integers without a fraction, everything else trimmed to 6 decimals.
    fn fmt(v: f64) -> String {
        if v.is_nan() || v.is_infinite() {
            return "error".to_string();
        }
        if v == (v as i64) as f64 && v.abs() < 1e15 {
            return format!("{}", v as i64);
        }
        let s = format!("{:.6}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }

    fn apply_pending(&mut self) {
        let v = self.value();
        if let Some(op) = self.pending {
            self.acc = match op {
                '+' => self.acc + v,
                '-' => self.acc - v,
                '*' => self.acc * v,
                '/' => self.acc / v,
                _ => v,
            };
        } else {
            self.acc = v;
        }
        self.display = Self::fmt(self.acc);
    }

    /// Feed one input character (digit, '.', operator, '=' or 'C').
    fn input(&mut self, c: char) {
        match c {
            '0'..='9' => {
                if self.reset_next || self.display == "0" {
                    self.display.clear();
                    self.reset_next = false;
                }
                if self.display.len() < 15 {
                    self.display.push(c);
                }
            }
            '.' => {
                if self.reset_next {
                    self.display = "0".to_string();
                    self.reset_next = false;
                }
                if !self.display.contains('.') {
                    self.display.push('.');
                }
            }
            '+' | '-' | '*' | '/' => {
                self.apply_pending();
                self.pending = Some(c);
                self.reset_next = true;
            }
            '=' => {
                self.apply_pending();
                self.pending = None;
                self.reset_next = true;
            }
            'C' | 'c' => *self = Calc::new(),
            _ => {}
        }
    }
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
                    "  disk / dformat  SATA disk status / format (QOSFS)",
                    "  dls / dcat <n>  list / print a disk file",
                    "  dsave <f>       copy a fs file onto the disk (persists)",
                    "  dload <n>       copy a disk file into the fs",
                    "  qasm <f> [n]    compile + run a .qasm file (n shots)",
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
            "disk" => {
                if !crate::ahci::present() {
                    self.push("disk: no SATA disk attached".to_string());
                } else {
                    let sec = crate::ahci::capacity_sectors();
                    let fmt = crate::diskfs::is_formatted();
                    self.push(format!("disk: SATA, {} sectors (~{} MiB), {}",
                        sec, sec * 512 / 1024 / 1024, if fmt { "QOSFS formatted" } else { "unformatted (run dformat)" }));
                }
            }
            "dformat" => {
                if !crate::ahci::present() {
                    self.push("dformat: no disk".to_string());
                } else if crate::diskfs::mkfs() {
                    self.push("disk formatted (QOSFS)".to_string());
                } else {
                    self.push("dformat: failed".to_string());
                }
            }
            "dls" => {
                if !crate::diskfs::is_formatted() {
                    self.push("dls: disk not formatted (run dformat)".to_string());
                } else {
                    let entries = crate::diskfs::get_entries(b"");
                    if entries.is_empty() {
                        self.push("(disk empty)".to_string());
                    }
                    for (name, _is_dir, size) in entries {
                        self.push(format!("  {}   {} B", name, size));
                    }
                }
            }
            "dcat" => {
                if rest.is_empty() {
                    self.push("usage: dcat <name>".to_string());
                } else {
                    match crate::diskfs::read(rest.as_bytes()) {
                        Some(bytes) => {
                            let text = String::from_utf8_lossy(&bytes);
                            for line in text.split('\n') {
                                self.push(line.to_string());
                            }
                        }
                        None => self.push(format!("dcat: not found on disk: {}", rest)),
                    }
                }
            }
            "dsave" => {
                // Copy a RAM-fs file (resolved against cwd) onto the persistent disk (flat name).
                if rest.is_empty() {
                    self.push("usage: dsave <file>   (RAM fs -> disk)".to_string());
                } else {
                    let path = if self.cwd.is_empty() { rest.to_string() } else { format!("{}/{}", self.cwd, rest) };
                    let base = rest.rsplit('/').next().unwrap_or(rest);
                    match crate::fs::read(path.as_bytes()) {
                        Some(bytes) => match crate::diskfs::write(base.as_bytes(), &bytes) {
                            Ok(()) => self.push(format!("saved {} ({} B) to disk", base, bytes.len())),
                            Err(e) => self.push(format!("dsave: {}", e)),
                        },
                        None => self.push(format!("dsave: no such fs file: {}", rest)),
                    }
                }
            }
            "dload" => {
                // Copy a disk file into the RAM fs (into the current dir).
                if rest.is_empty() {
                    self.push("usage: dload <name>   (disk -> RAM fs)".to_string());
                } else {
                    match crate::diskfs::read(rest.as_bytes()) {
                        Some(bytes) => {
                            let path = if self.cwd.is_empty() { rest.to_string() } else { format!("{}/{}", self.cwd, rest) };
                            match crate::fs::write(path.as_bytes(), &bytes) {
                                Ok(()) => self.push(format!("loaded {} ({} B) into /{}", rest, bytes.len(), path)),
                                Err(e) => self.push(format!("dload: {}", e)),
                            }
                        }
                        None => self.push(format!("dload: not found on disk: {}", rest)),
                    }
                }
            }
            "echo" => self.push(rest.to_string()),
            "ver" => self.push("QOS 0.1 — UEFI x86-64, native compositor UI, quantum control plane".to_string()),
            "mem" => {
                let ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
                self.push(format!("heap {} MiB   uptime {} ticks (~{} s)", crate::allocator::HEAP_SIZE / 1024 / 1024, ticks, ticks / 100));
            }
            "qasm" => {
                // qasm <file> [shots] — compile (with transpile stats) + run a QASM source file.
                let (fname, shots_s) = match rest.split_once(' ') {
                    Some((a, b)) => (a.trim(), b.trim()),
                    None => (rest, ""),
                };
                if fname.is_empty() {
                    self.push("usage: qasm <file> [shots]".to_string());
                } else {
                    let shots = shots_s.parse::<u64>().unwrap_or(1000).clamp(1, 100_000);
                    let path = self.resolve(fname);
                    match crate::fs::read(path.as_bytes()) {
                        None => self.push(format!("qasm: cannot read /{}", path)),
                        Some(bytes) => match crate::quantum::parser::parse_qasm2(&bytes) {
                            Err(e) => self.push(format!("qasm: {}", e.message())),
                            Ok(prog) => {
                                let before = prog.instructions.len();
                                let (opt, removed) =
                                    crate::quantum::transpile::cancel_pairs(prog.instructions);
                                let d = crate::quantum::transpile::depth(&opt, prog.n_qubits);
                                self.push(format!(
                                    "compiled: {} qubits, {} -> {} gates ({} cancelled), depth {}",
                                    prog.n_qubits, before, opt.len(), removed, d
                                ));
                                match crate::quantum::sim::run_program(
                                    prog.n_qubits,
                                    prog.n_cbits.max(prog.n_qubits),
                                    opt,
                                    shots,
                                ) {
                                    None => self.push("qasm: qubit count out of range".to_string()),
                                    Some(res) => {
                                        let mut pairs: Vec<(String, u64)> =
                                            res.counts.iter().map(|(k, v)| (k.clone(), *v)).collect();
                                        pairs.sort_by(|a, b| b.1.cmp(&a.1));
                                        self.push(format!("{} shots:", shots));
                                        for (k, v) in pairs.iter().take(8) {
                                            self.push(format!("  {} -> {}", k, v));
                                        }
                                    }
                                }
                            }
                        },
                    }
                }
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
    /// Files: browsing the persistent SATA disk (QOSFS) instead of the RAM fs.
    files_on_disk: bool,
    /// Files: the currently selected entry name (for Rename/Delete/Edit).
    files_sel: Option<String>,
    /// Files: a status/error line under the toolbar (e.g. "deleted", "cannot delete non-empty dir").
    files_status: String,
    /// Files: active naming modal (kind + typed buffer), if any.
    files_naming: Option<NameMode>,
    files_name_buf: String,
    /// Quantum Lab circuit editor: qubit count, placed gates, palette selection, grid cursor,
    /// pending CX control cell, R-gate angle (×π/4), last run's histogram + a status line.
    qlab_qubits: usize,
    qlab_gates: Vec<QGate>,
    qlab_sel: usize,
    qlab_cursor: (usize, usize),
    qlab_pending: Option<(usize, usize)>,
    qlab_angle: u8,
    qlab_result: Vec<(String, u64)>,
    qlab_status: String,
    /// Text Editor: path of the open file (None = nothing open), buffer, and a status line.
    editor_path: Option<String>,
    editor_buf: String,
    editor_status: String,
    /// Calculator state.
    calc: Calc,
    /// Files: scroll offset into the listing (rows above it are hidden).
    files_scroll: usize,
    /// Quantum IDE (QASM Studio, WP-07): line-based source buffer with a real cursor
    /// (line, column in chars), backing file, status, problem marker, live-parsed circuit
    /// preview, and the last run's histogram.
    qasm_lines: Vec<String>,
    qasm_cur: (usize, usize),
    qasm_path: Option<String>,
    qasm_status: String,
    /// Active problem: (1-based source line or 0 for program-level, message). Drives the (!)
    /// problems row, the red gutter marker, and click-to-jump.
    qasm_problem: Option<(usize, String)>,
    qasm_preview: Option<(usize, Vec<crate::quantum::parser::Instruction>)>,
    qasm_result: Vec<(String, u64)>,
}

/// Margin around a window rect that its shadow extends into (for damage rects).
const SHADOW_MARGIN: i32 = 34;

impl Desktop {
    fn new(w: i32, h: i32) -> Self {
        // Start with two cascaded windows so the desktop looks alive.
        let wins = vec![
            Win::new(Rect::new(w / 2 - 440, 74, 540, 440), AppKind::Terminal),
            Win::new(Rect::new(w / 2 - 40, 230, 520, 440), AppKind::Files),
        ];
        let mut desk = Desktop {
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
            files_on_disk: false,
            files_sel: None,
            files_status: String::new(),
            files_naming: None,
            files_name_buf: String::new(),
            qlab_qubits: 3,
            // A GHZ circuit is pre-loaded so the first Run immediately shows entanglement.
            qlab_gates: vec![
                QGate { kind: QG::H, q: 0, q2: 0, col: 0, angle: 2 },
                QGate { kind: QG::Cx, q: 0, q2: 1, col: 1, angle: 2 },
                QGate { kind: QG::Cx, q: 1, q2: 2, col: 2, angle: 2 },
            ],
            qlab_sel: 0,
            qlab_cursor: (0, 0),
            qlab_pending: None,
            qlab_angle: 2, // π/2
            qlab_result: Vec::new(),
            qlab_status: "Space places · Enter runs · arrows move".to_string(),
            editor_path: None,
            editor_buf: String::new(),
            editor_status: "no file open — open one from Files".to_string(),
            calc: Calc::new(),
            files_scroll: 0,
            qasm_lines: Vec::new(),
            qasm_cur: (0, 0),
            qasm_path: None,
            qasm_status: "Bell template · F4 compile · F5 run · F2 save".to_string(),
            qasm_problem: None,
            qasm_preview: None,
            qasm_result: Vec::new(),
        };
        desk.qasm_set_text(
            "OPENQASM 2.0;\nqreg q[2];\ncreg c[2];\nh q[0];\ncx q[0],q[1];\nmeasure q[0] -> c[0];\nmeasure q[1] -> c[1];",
        );
        desk
    }

    // ---- app actions (invoked by body clicks) ----
    /// Display name of the persistent-disk location shown at the RAM-fs root.
    const DISK_ENTRY: &'static str = "Disk (SATA)";

    /// The Files listing for the current location. RAM fs: `..` (unless at root) then dirs, then
    /// files, each sorted by name — with a "Disk (SATA)" location at the root when a formatted
    /// persistent disk is present. Disk: `..` (back to the RAM root) + the flat QOSFS listing.
    /// Used by both drawing and click hit-testing.
    fn files_list(&self) -> Vec<(String, bool, usize)> {
        let mut list = Vec::new();
        if self.files_on_disk {
            list.push(("..".to_string(), true, 0));
            let mut entries = crate::diskfs::get_entries(b"");
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            list.extend(entries);
            return list;
        }
        let mut entries = crate::fs::get_entries(self.files_cwd.as_bytes());
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        if !self.files_cwd.is_empty() {
            list.push(("..".to_string(), true, 0));
        } else if crate::ahci::present() {
            // The persistent disk appears as a location at the top of the root listing.
            list.push((Self::DISK_ENTRY.to_string(), true, 0));
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
                let scroll = self.files_scroll.min(list.len());
                for (v, (name, is_dir, _)) in list.iter().skip(scroll).take(FILES_MAX_ROWS).enumerate() {
                    if files_row_rect(wr, v).contains(cx, cy) {
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
                // Palette.
                for i in 0..QLAB_PALETTE.len() {
                    if qlab_pal_rect(wr, i).contains(cx, cy) {
                        self.qlab_sel = i;
                        self.qlab_pending = None;
                        self.mark_top_window();
                        return;
                    }
                }
                // Controls: Run / Clear / Q- / Q+ / angle / export-to-QASM.
                for i in 0..6 {
                    if qlab_ctl_rect(wr, i).contains(cx, cy) {
                        match i {
                            0 => self.qlab_run(),
                            1 => {
                                self.qlab_gates.clear();
                                self.qlab_result.clear();
                                self.qlab_pending = None;
                                self.mark_top_window();
                            }
                            2 => {
                                if self.qlab_qubits > 2 {
                                    self.qlab_qubits -= 1;
                                    self.qlab_gates.retain(|g| g.q < self.qlab_qubits && g.q2 < self.qlab_qubits);
                                    self.qlab_cursor.0 = self.qlab_cursor.0.min(self.qlab_qubits - 1);
                                    self.mark_top_window();
                                }
                            }
                            3 => {
                                if self.qlab_qubits < QLAB_MAX_Q {
                                    self.qlab_qubits += 1;
                                    self.mark_top_window();
                                }
                            }
                            4 => {
                                self.qlab_angle = if self.qlab_angle >= 8 { 1 } else { self.qlab_angle + 1 };
                                self.mark_top_window();
                            }
                            _ => {
                                // Export the circuit as OpenQASM source and open it in the Studio.
                                let src = self.qlab_to_qasm();
                                self.qasm_open(None, src);
                            }
                        }
                        return;
                    }
                }
                // Grid cells.
                for q in 0..self.qlab_qubits {
                    for col in 0..QLAB_COLS {
                        if qlab_cell_rect(wr, q, col).contains(cx, cy) {
                            self.qlab_cursor = (q, col);
                            self.qlab_place(q, col);
                            return;
                        }
                    }
                }
            }
            AppKind::Settings => {
                for i in 0..2 {
                    if settings_card_rect(wr, i).contains(cx, cy) {
                        let want_dark = i == 0;
                        if self.theme.is_dark != want_dark {
                            self.theme = self.theme.toggled();
                            self.mark_full();
                        }
                        return;
                    }
                }
            }
            AppKind::Calculator => {
                for (i, label) in CALC_BTNS.iter().enumerate() {
                    if calc_btn_rect(wr, i).contains(cx, cy) {
                        let c = label.chars().next().unwrap_or(' ');
                        self.calc.input(c);
                        self.mark_top_window();
                        return;
                    }
                }
                if calc_clear_rect(wr).contains(cx, cy) {
                    self.calc.input('C');
                    self.mark_top_window();
                }
            }
            AppKind::Qasm => {
                if qasm_btn_rect(wr, 0).contains(cx, cy) {
                    self.qasm_compile();
                    return;
                }
                if qasm_btn_rect(wr, 1).contains(cx, cy) {
                    self.qasm_run_buf();
                    return;
                }
                if qasm_btn_rect(wr, 2).contains(cx, cy) {
                    self.qasm_save();
                    return;
                }
                // Sidebar: click a workspace file to open it in the editor.
                let files = self.qasm_workspace_files();
                for (i, f) in files.iter().enumerate() {
                    if qasm_side_row_rect(wr, i).contains(cx, cy) {
                        let content = crate::fs::read(f.as_bytes())
                            .map(|b| String::from_utf8_lossy(&b).into_owned())
                            .unwrap_or_default();
                        let f = f.clone();
                        self.qasm_open(Some(f), content);
                        return;
                    }
                }
                // Problems row: click jumps to the offending line.
                if self.qasm_problem.is_some() && qasm_preview_rect(wr).contains(cx, cy) {
                    self.qasm_goto_problem();
                    return;
                }
                // Code pane: click-to-position (line exact; column from the average glyph
                // advance ≈7 px at 13 pt — refined once per-glyph metrics are exposed here).
                let code = qasm_code_rect(wr);
                if code.contains(cx, cy) {
                    let line_h = 18;
                    let max_rows = ((code.h - 12) / line_h).max(1) as usize;
                    let (cl, _) = self.qasm_cur;
                    let first = if cl >= max_rows { cl + 1 - max_rows } else { 0 };
                    let row = ((cy - code.y - 6).max(0) / line_h) as usize;
                    let l = (first + row).min(self.qasm_lines.len().saturating_sub(1));
                    let approx_col = ((cx - code.x - 34).max(0) / 7) as usize;
                    let col = approx_col.min(self.qasm_lines[l].chars().count());
                    self.qasm_cur = (l, col);
                    self.mark_top_window();
                }
            }
            AppKind::Terminal | AppKind::Monitor | AppKind::Devices | AppKind::Processes => {}
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
        if self.files_on_disk {
            if name == ".." {
                // Leave the disk location, back to the RAM-fs root.
                self.files_on_disk = false;
                self.files_preview = None;
                self.files_sel = None;
                self.files_scroll = 0;
            } else {
                // Select + preview a disk file.
                self.files_sel = Some(name.to_string());
                self.files_preview = Some(match crate::diskfs::read(name.as_bytes()) {
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
            return;
        }
        if name == ".." {
            // Go up one path segment.
            if let Some(pos) = self.files_cwd.rfind('/') {
                self.files_cwd.truncate(pos);
            } else {
                self.files_cwd.clear();
            }
            self.files_preview = None;
            self.files_sel = None;
            self.files_scroll = 0;
        } else if name == Self::DISK_ENTRY && self.files_cwd.is_empty() {
            // Enter the persistent-disk location.
            self.files_on_disk = true;
            self.files_preview = None;
            self.files_sel = None;
            self.files_scroll = 0;
            if !crate::diskfs::is_formatted() {
                self.files_status = "disk unformatted — run dformat in the Terminal".to_string();
            } else {
                let mib = crate::ahci::capacity_sectors() * 512 / 1024 / 1024;
                self.files_status = format!("SATA disk, {} MiB (persistent)", mib);
            }
        } else if is_dir {
            // Single click selects a dir; navigating in is via double-purpose: select then click
            // again enters. Keep it simple: clicking a dir enters it (mirrors the old behavior).
            if !self.files_cwd.is_empty() {
                self.files_cwd.push('/');
            }
            self.files_cwd.push_str(name);
            self.files_preview = None;
            self.files_sel = None;
            self.files_scroll = 0;
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

    /// Keyboard navigation: move the Files selection up/down through the whole listing, scrolling
    /// the visible window (`files_scroll`) to keep the selection on screen.
    fn files_nav(&mut self, down: bool) {
        let list = self.files_list();
        if list.is_empty() {
            return;
        }
        let cur = self
            .files_sel
            .as_deref()
            .and_then(|s| list.iter().position(|(n, _, _)| n == s));
        let idx = match cur {
            Some(i) => {
                if down {
                    (i + 1).min(list.len() - 1)
                } else {
                    i.saturating_sub(1)
                }
            }
            None => 0,
        };
        self.files_sel = Some(list[idx].0.clone());
        // Follow with the scroll window.
        if idx < self.files_scroll {
            self.files_scroll = idx;
        } else if idx >= self.files_scroll + FILES_MAX_ROWS {
            self.files_scroll = idx + 1 - FILES_MAX_ROWS;
        }
        self.mark_top_window();
    }

    /// Keyboard activation (Enter): open the selected row — enter a dir / location, or preview a
    /// file — exactly like a mouse click on it.
    fn files_activate(&mut self) {
        let Some(sel) = self.files_sel.clone() else { return };
        let list = self.files_list();
        if let Some((name, is_dir, _)) = list.iter().find(|(n, _, _)| *n == sel) {
            let (n, d) = (name.clone(), *is_dir);
            self.files_click(&n, d);
        }
    }

    /// A Files toolbar button was clicked (index into `FILES_TOOLBAR`).
    fn files_tool(&mut self, i: usize) {
        self.files_status.clear();
        match i {
            0 => self.files_begin_name(NameMode::NewFile), // New File
            1 => self.files_begin_name(NameMode::NewDir),  // New Dir
            2 => {
                // Rename: needs a real selection (not ".." / the disk location).
                if self.files_sel_real().is_some() {
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

    /// The selection, unless it is a pseudo-entry (`..` / the disk location) that ops must skip.
    fn files_sel_real(&self) -> Option<String> {
        self.files_sel
            .clone()
            .filter(|s| s != ".." && s != Self::DISK_ENTRY)
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
        if self.files_on_disk {
            // Persistent-disk (QOSFS, flat) variants of the ops.
            if !crate::diskfs::is_formatted() {
                self.files_status = "disk unformatted — run dformat in the Terminal".to_string();
                self.mark_full();
                return;
            }
            match mode {
                NameMode::NewFile => match crate::diskfs::write(name.as_bytes(), b"") {
                    Ok(()) => {
                        self.files_status = format!("created {} on disk", name);
                        self.files_sel = Some(name);
                    }
                    Err(e) => self.files_status = format!("error: {}", e),
                },
                NameMode::NewDir => {
                    self.files_status = "disk fs is flat (no directories yet)".to_string();
                }
                NameMode::Rename => {
                    // QOSFS has no rename; emulate via read + write + remove.
                    if let Some(old) = self.files_sel.clone() {
                        match crate::diskfs::read(old.as_bytes()) {
                            Some(bytes) => match crate::diskfs::write(name.as_bytes(), &bytes) {
                                Ok(()) => {
                                    crate::diskfs::remove(old.as_bytes());
                                    self.files_status = format!("renamed to {}", name);
                                    self.files_sel = Some(name);
                                    self.files_preview = None;
                                }
                                Err(e) => self.files_status = format!("error: {}", e),
                            },
                            None => self.files_status = "error: cannot read source".to_string(),
                        }
                    }
                }
            }
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
        let Some(name) = self.files_sel_real() else {
            self.files_status = "select an item first".to_string();
            self.mark_full();
            return;
        };
        if self.files_on_disk {
            if crate::diskfs::remove(name.as_bytes()) {
                self.files_status = format!("deleted {} from disk", name);
                self.files_sel = None;
                self.files_preview = None;
            } else {
                self.files_status = "cannot delete (disk)".to_string();
            }
            self.mark_full();
            return;
        }
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
        let Some(name) = self.files_sel_real() else {
            self.files_status = "select a file first".to_string();
            self.mark_full();
            return;
        };
        // `.qasm` sources open in QASM Studio (the quantum toolchain); everything else in the
        // plain Text Editor.
        let is_qasm = name.ends_with(".qasm");
        if self.files_on_disk {
            let path = format!("disk:{}", name);
            if is_qasm {
                let content = crate::diskfs::read(name.as_bytes())
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                self.qasm_open(Some(path), content);
            } else {
                self.editor_open(&path);
                self.open_app(AppKind::Editor);
            }
            return;
        }
        if crate::fs::is_dir(self.files_path(&name).as_bytes()) {
            self.files_status = "cannot edit a directory".to_string();
            self.mark_full();
            return;
        }
        let path = self.files_path(&name);
        if is_qasm {
            let content = crate::fs::read(path.as_bytes())
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            self.qasm_open(Some(path), content);
        } else {
            self.editor_open(&path);
            self.open_app(AppKind::Editor);
        }
    }

    /// Load `path` into the Text Editor buffer. A `disk:` prefix targets the persistent disk
    /// (QOSFS); anything else is the RAM fs.
    fn editor_open(&mut self, path: &str) {
        let bytes = match path.strip_prefix("disk:") {
            Some(name) => crate::diskfs::read(name.as_bytes()),
            None => crate::fs::read(path.as_bytes()),
        };
        match bytes {
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

    /// Save the Text Editor buffer back to its file — the persistent disk for `disk:` paths, the
    /// RAM fs otherwise.
    fn editor_save(&mut self) {
        match self.editor_path.clone() {
            Some(path) => {
                let res = match path.strip_prefix("disk:") {
                    Some(name) => crate::diskfs::write(name.as_bytes(), self.editor_buf.as_bytes()),
                    None => crate::fs::write(path.as_bytes(), self.editor_buf.as_bytes()),
                };
                match res {
                    Ok(()) => self.editor_status = format!("saved {}  ({} bytes)", path, self.editor_buf.len()),
                    Err(e) => self.editor_status = format!("save error: {}", e),
                }
            }
            None => self.editor_status = "no file — create one from Files first".to_string(),
        }
        self.mark_full();
    }

    // ---- Quantum Lab circuit editor ----
    /// Index of the gate occupying wire `q` at column `col` (as control or CX target), if any.
    fn qlab_gate_at(&self, q: usize, col: usize) -> Option<usize> {
        self.qlab_gates
            .iter()
            .position(|g| g.col == col && (g.q == q || (g.kind == QG::Cx && g.q2 == q)))
    }

    /// Place the selected palette gate at (q, col) — or remove the gate already there. CX takes
    /// two placements: first the control cell, then the target on another wire of the same column.
    fn qlab_place(&mut self, q: usize, col: usize) {
        self.qlab_status.clear();
        // Clicking an occupied cell removes that gate (uniform, predictable editing).
        if let Some(i) = self.qlab_gate_at(q, col) {
            self.qlab_gates.remove(i);
            self.qlab_pending = None;
            self.mark_top_window();
            return;
        }
        let (kind, _) = QLAB_PALETTE[self.qlab_sel];
        if kind == QG::Cx {
            match self.qlab_pending {
                None => {
                    self.qlab_pending = Some((q, col));
                    self.qlab_status = "CX: now pick the target wire (same column)".to_string();
                }
                Some((cq, ccol)) => {
                    if ccol == col && cq != q {
                        self.qlab_gates.push(QGate { kind: QG::Cx, q: cq, q2: q, col, angle: 2 });
                        self.qlab_pending = None;
                    } else {
                        self.qlab_pending = None;
                        self.qlab_status = "CX cancelled (target must share the column)".to_string();
                    }
                }
            }
        } else {
            self.qlab_gates.push(QGate { kind, q, q2: q, col, angle: self.qlab_angle });
        }
        self.mark_top_window();
    }

    /// Run the edited circuit on the real statevector simulator and keep the top outcomes.
    fn qlab_run(&mut self) {
        use crate::quantum::parser::Instruction as I;
        let mut gates = self.qlab_gates.clone();
        gates.sort_by_key(|g| (g.col, g.q));
        let instrs: Vec<I> = gates
            .iter()
            .map(|g| {
                let th = g.angle as f64 * core::f64::consts::FRAC_PI_4;
                match g.kind {
                    QG::H => I::H(g.q),
                    QG::X => I::X(g.q),
                    QG::Y => I::Y(g.q),
                    QG::Z => I::Z(g.q),
                    QG::S => I::S(g.q),
                    QG::T => I::T(g.q),
                    QG::Rx => I::Rx(g.q, th),
                    QG::Ry => I::Ry(g.q, th),
                    QG::Rz => I::Rz(g.q, th),
                    QG::Cx => I::Cx(g.q, g.q2),
                }
            })
            .collect();
        let n_inst = instrs.len();
        match crate::quantum::sim::run_program(self.qlab_qubits, self.qlab_qubits, instrs, QLAB_SHOTS) {
            Some(res) => {
                let mut pairs: Vec<(String, u64)> =
                    res.counts.iter().map(|(k, v)| (k.clone(), *v)).collect();
                pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                pairs.truncate(6);
                self.qlab_result = pairs;
                self.qlab_status = format!("{} gates · {} shots", n_inst, QLAB_SHOTS);
            }
            None => self.qlab_status = "run failed (qubit count out of range)".to_string(),
        }
        self.mark_top_window();
    }

    /// The focused window: the topmost one, unless it is minimized (then nothing has focus).
    fn focused(&self) -> Option<&Win> {
        self.wins.last().filter(|w| !w.minimized)
    }

    // ---- Quantum IDE (WP-07): real editing core + live preview + toolchain ----
    /// The buffer as one source string (for the parser / saving).
    fn qasm_text(&self) -> String {
        self.qasm_lines.join("\n")
    }

    /// Replace the buffer, put the cursor at the end, and refresh the live preview.
    fn qasm_set_text(&mut self, s: &str) {
        self.qasm_lines = s.split('\n').map(|l| l.to_string()).collect();
        if self.qasm_lines.is_empty() {
            self.qasm_lines.push(String::new());
        }
        let last = self.qasm_lines.len() - 1;
        self.qasm_cur = (last, self.qasm_lines[last].chars().count());
        self.qasm_reparse();
    }

    /// Live-parse the buffer for the circuit preview + problem marker (cheap for editor-sized
    /// sources; capped so a huge paste can't stall the UI).
    fn qasm_reparse(&mut self) {
        use crate::quantum::{parser, sim};
        let text = self.qasm_text();
        if text.len() > 16 * 1024 {
            self.qasm_preview = None;
            self.qasm_problem = Some((0, "source too large for live preview".to_string()));
            return;
        }
        match parser::parse_qasm2(text.as_bytes()) {
            Ok(prog) if prog.n_qubits >= 1 && prog.n_qubits <= sim::MAX_QUBITS => {
                self.qasm_problem = None;
                self.qasm_preview = Some((prog.n_qubits, prog.instructions));
            }
            Ok(prog) => {
                self.qasm_preview = None;
                self.qasm_problem = Some((0, format!("qubit count {} out of range", prog.n_qubits)));
            }
            Err(e) => {
                self.qasm_preview = None;
                self.qasm_problem = Some((e.line, e.message()));
            }
        }
    }

    /// Jump the cursor to the active problem's line (problems-row click / keyboard).
    fn qasm_goto_problem(&mut self) {
        if let Some((line, _)) = &self.qasm_problem {
            if *line > 0 {
                let l = (*line - 1).min(self.qasm_lines.len().saturating_sub(1));
                self.qasm_cur = (l, 0);
                self.mark_top_window();
            }
        }
    }

    /// Clamp the cursor column to the current line length (after vertical moves).
    fn qasm_clamp_col(&mut self) {
        let line_len = self.qasm_lines[self.qasm_cur.0].chars().count();
        if self.qasm_cur.1 > line_len {
            self.qasm_cur.1 = line_len;
        }
    }

    /// Cursor movement: dx = -1/+1 within the line (wrapping over line ends), dy = -1/+1 lines.
    fn qasm_move(&mut self, dx: i32, dy: i32) {
        if dy < 0 && self.qasm_cur.0 > 0 {
            self.qasm_cur.0 -= 1;
            self.qasm_clamp_col();
        } else if dy > 0 && self.qasm_cur.0 + 1 < self.qasm_lines.len() {
            self.qasm_cur.0 += 1;
            self.qasm_clamp_col();
        }
        if dx < 0 {
            if self.qasm_cur.1 > 0 {
                self.qasm_cur.1 -= 1;
            } else if self.qasm_cur.0 > 0 {
                self.qasm_cur.0 -= 1;
                self.qasm_cur.1 = self.qasm_lines[self.qasm_cur.0].chars().count();
            }
        } else if dx > 0 {
            let len = self.qasm_lines[self.qasm_cur.0].chars().count();
            if self.qasm_cur.1 < len {
                self.qasm_cur.1 += 1;
            } else if self.qasm_cur.0 + 1 < self.qasm_lines.len() {
                self.qasm_cur.0 += 1;
                self.qasm_cur.1 = 0;
            }
        }
        self.mark_top_window();
    }

    /// Byte offset of char-column `col` in `line` (UTF-8 safe).
    fn byte_at(line: &str, col: usize) -> usize {
        line.char_indices().nth(col).map(|(i, _)| i).unwrap_or(line.len())
    }

    /// Insert a character at the cursor.
    fn qasm_insert(&mut self, c: char) {
        if self.qasm_text().len() >= 32 * 1024 {
            return;
        }
        let (l, col) = self.qasm_cur;
        let at = Self::byte_at(&self.qasm_lines[l], col);
        self.qasm_lines[l].insert(at, c);
        self.qasm_cur.1 += 1;
        self.qasm_reparse();
        self.mark_top_window();
    }

    /// Backspace: delete before the cursor, joining lines at column 0.
    fn qasm_backspace(&mut self) {
        let (l, col) = self.qasm_cur;
        if col > 0 {
            let at = Self::byte_at(&self.qasm_lines[l], col - 1);
            self.qasm_lines[l].remove(at);
            self.qasm_cur.1 -= 1;
        } else if l > 0 {
            let tail = self.qasm_lines.remove(l);
            let prev_len = self.qasm_lines[l - 1].chars().count();
            self.qasm_lines[l - 1].push_str(&tail);
            self.qasm_cur = (l - 1, prev_len);
        }
        self.qasm_reparse();
        self.mark_top_window();
    }

    /// Enter: split the current line at the cursor.
    fn qasm_newline(&mut self) {
        let (l, col) = self.qasm_cur;
        let at = Self::byte_at(&self.qasm_lines[l], col);
        let tail = self.qasm_lines[l].split_off(at);
        self.qasm_lines.insert(l + 1, tail);
        self.qasm_cur = (l + 1, 0);
        self.qasm_reparse();
        self.mark_top_window();
    }

    /// Workspace `.qasm` sources for the sidebar: root + the `quantum/` folder (capped).
    fn qasm_workspace_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, is_dir, _) in crate::fs::get_entries(b"") {
            if !is_dir && name.ends_with(".qasm") {
                out.push(name);
            }
        }
        for (name, is_dir, _) in crate::fs::get_entries(b"quantum") {
            if !is_dir && name.ends_with(".qasm") {
                out.push(format!("quantum/{}", name));
            }
        }
        out.sort();
        out.truncate(10);
        out
    }

    /// Compile the buffer: parse + validate, then run the transpile passes (self-inverse pair
    /// cancellation + depth analysis) and report the stats — errors land in the status line.
    fn qasm_compile(&mut self) {
        use crate::quantum::{parser, sim, transpile};
        let text = self.qasm_text();
        match parser::parse_qasm2(text.as_bytes()) {
            Ok(prog) => {
                if prog.n_qubits == 0 || prog.n_qubits > sim::MAX_QUBITS {
                    self.qasm_status = format!("error: qubit count {} out of range (1..={})", prog.n_qubits, sim::MAX_QUBITS);
                } else {
                    let before = prog.instructions.len();
                    let (opt, removed) = transpile::cancel_pairs(prog.instructions);
                    let d = transpile::depth(&opt, prog.n_qubits);
                    self.qasm_status = if removed > 0 {
                        format!("compiled: {} qubits · {} -> {} gates ({} cancelled) · depth {}", prog.n_qubits, before, opt.len(), removed, d)
                    } else {
                        format!("compiled: {} qubits · {} gates · depth {}", prog.n_qubits, before, d)
                    };
                }
            }
            Err(e) => self.qasm_status = format!("error — {}", e.message()),
        }
        self.mark_top_window();
    }

    /// Run the buffer on the simulator (through the optimizer) and keep the top outcomes.
    fn qasm_run_buf(&mut self) {
        use crate::quantum::{parser, sim, transpile};
        let text = self.qasm_text();
        match parser::parse_qasm2(text.as_bytes()) {
            Ok(prog) => {
                let (opt, _) = transpile::cancel_pairs(prog.instructions);
                match sim::run_program(prog.n_qubits, prog.n_cbits.max(prog.n_qubits), opt, QLAB_SHOTS) {
                    Some(res) => {
                        let mut pairs: Vec<(String, u64)> =
                            res.counts.iter().map(|(k, v)| (k.clone(), *v)).collect();
                        pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                        pairs.truncate(4);
                        self.qasm_result = pairs;
                        self.qasm_status = format!("ran {} shots on {} qubits", QLAB_SHOTS, res.n_qubits);
                    }
                    None => self.qasm_status = "error: qubit count out of range".to_string(),
                }
            }
            Err(e) => self.qasm_status = format!("error — {}", e.message()),
        }
        self.mark_top_window();
    }

    /// Save the buffer to its backing file (RAM fs or `disk:`), defaulting to `draft.qasm`.
    fn qasm_save(&mut self) {
        let path = self.qasm_path.clone().unwrap_or_else(|| "draft.qasm".to_string());
        let text = self.qasm_text();
        let res = match path.strip_prefix("disk:") {
            Some(name) => crate::diskfs::write(name.as_bytes(), text.as_bytes()),
            None => crate::fs::write(path.as_bytes(), text.as_bytes()),
        };
        self.qasm_status = match res {
            Ok(()) => {
                self.qasm_path = Some(path.clone());
                format!("saved {} ({} B)", path, text.len())
            }
            Err(e) => format!("save error: {}", e),
        };
        self.mark_top_window();
    }

    /// Open a QASM source in the IDE (used by Files for `.qasm` and by the Lab's export).
    fn qasm_open(&mut self, path: Option<String>, content: String) {
        self.qasm_set_text(&content);
        self.qasm_path = path;
        self.qasm_result.clear();
        self.qasm_status = match &self.qasm_path {
            Some(p) => format!("editing {}", p),
            None => "exported from Quantum Lab (unsaved)".to_string(),
        };
        self.open_app(AppKind::Qasm);
    }

    /// Serialize the Quantum Lab circuit to OpenQASM 2.0 source.
    fn qlab_to_qasm(&self) -> String {
        let mut gates = self.qlab_gates.clone();
        gates.sort_by_key(|g| (g.col, g.q));
        let mut out = format!("OPENQASM 2.0;\nqreg q[{}];\ncreg c[{}];\n", self.qlab_qubits, self.qlab_qubits);
        for g in gates.iter().filter(|g| g.q < self.qlab_qubits && g.q2 < self.qlab_qubits) {
            // Always emit the parser-friendly product form (e.g. `3*pi/4`).
            let th = format!("{}*pi/4", g.angle);
            match g.kind {
                QG::H => out.push_str(&format!("h q[{}];\n", g.q)),
                QG::X => out.push_str(&format!("x q[{}];\n", g.q)),
                QG::Y => out.push_str(&format!("y q[{}];\n", g.q)),
                QG::Z => out.push_str(&format!("z q[{}];\n", g.q)),
                QG::S => out.push_str(&format!("s q[{}];\n", g.q)),
                QG::T => out.push_str(&format!("t q[{}];\n", g.q)),
                QG::Rx => out.push_str(&format!("rx({}) q[{}];\n", th, g.q)),
                QG::Ry => out.push_str(&format!("ry({}) q[{}];\n", th, g.q)),
                QG::Rz => out.push_str(&format!("rz({}) q[{}];\n", th, g.q)),
                QG::Cx => out.push_str(&format!("cx q[{}],q[{}];\n", g.q, g.q2)),
            }
        }
        for q in 0..self.qlab_qubits {
            out.push_str(&format!("measure q[{}] -> c[{}];\n", q, q));
        }
        out
    }

    /// True if the focused (topmost) window is the Terminal — then typed keys go to it.
    fn top_is_terminal(&self) -> bool {
        self.focused().map_or(false, |w| w.kind == AppKind::Terminal)
    }

    /// True if the focused window is the Text Editor — then typed keys edit its buffer.
    fn top_is_editor(&self) -> bool {
        self.focused().map_or(false, |w| w.kind == AppKind::Editor)
    }

    /// True if the focused window is the Calculator — then digits/operators go to it.
    fn top_is_calc(&self) -> bool {
        self.focused().map_or(false, |w| w.kind == AppKind::Calculator)
    }

    /// True if the focused window shows live data (System Monitor / Processes) — refreshed ~1 Hz.
    fn top_is_live(&self) -> bool {
        self.focused()
            .map_or(false, |w| matches!(w.kind, AppKind::Monitor | AppKind::Processes))
    }

    /// True while the Files naming modal is open and the focused window is Files — then typed keys
    /// go to the name buffer.
    fn files_naming_active(&self) -> bool {
        self.files_naming.is_some() && self.focused().map_or(false, |w| w.kind == AppKind::Files)
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
            let mut win = self.wins.remove(i);
            win.minimized = false; // restore if it was minimized
            self.wins.push(win); // raise
        } else {
            let n = self.wins.len() as i32;
            // Per-app default size: the Quantum IDE opens larger (VS Code-like layout needs room).
            let (ww, wh) = match kind {
                AppKind::Qasm => (760, 560),
                _ => (540, 440),
            };
            let rect = Rect::new(
                (self.w / 2 - ww / 2 + n * 24).clamp(12, (self.w - ww - 12).max(12)),
                (70 + n * 22).min((self.h - wh - 20).max(BAR_H + 8)),
                ww,
                wh,
            );
            self.wins.push(Win::new(rect, kind));
        }
        self.mark_full();
    }

    /// Toggle maximize for window `i`: fill the workspace (between the top bar and the dock), or
    /// restore the saved rect.
    fn toggle_maximize(&mut self, i: usize) {
        let dock_top = self.dock_rect().y;
        let win = &mut self.wins[i];
        match win.saved.take() {
            Some(prev) => win.rect = prev,
            None => {
                win.saved = Some(win.rect);
                win.rect = Rect::new(12, BAR_H + 10, self.w - 24, dock_top - BAR_H - 20);
            }
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
        // Windows, top-most first (minimized ones are not on screen — skip them).
        for i in (0..self.wins.len()).rev() {
            if self.wins[i].minimized {
                continue;
            }
            let r = self.wins[i].rect;
            let (dxc, dyc) = self.close_dot(&r);
            let dy_ok = (cy - dyc).abs() <= 9;
            if dy_ok && (cx - dxc).abs() <= 9 {
                self.wins.remove(i); // red → close
                self.mark_full();
                return;
            }
            if dy_ok && (cx - (r.x + 44)).abs() <= 9 {
                // Yellow → minimize: hide and sink to the bottom of the z-order so the topmost
                // window stays a visible one.
                let mut win = self.wins.remove(i);
                win.minimized = true;
                self.wins.insert(0, win);
                self.mark_full();
                return;
            }
            if dy_ok && (cx - (r.x + 66)).abs() <= 9 {
                // Green → maximize/restore (and raise).
                let win = self.wins.remove(i);
                self.wins.push(win);
                self.toggle_maximize(self.wins.len() - 1);
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
        // Mini app icon to the left of the centered title (real-OS touch).
        let mini = Rect::new(r.x + (r.w - tw) / 2 - 26, cy - 10, 20, 20);
        let tint = app_tint(kind, theme);
        s.rounded_rect(mini, 5, tint);
        draw_app_icon(s, kind, mini, tint);
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
                // Real listing of the current location (RAM fs or the persistent SATA disk).
                let path = if self.files_on_disk {
                    "disk:/   (persistent SATA)".to_string()
                } else if self.files_cwd.is_empty() {
                    "/".to_string()
                } else {
                    format!("/{}", self.files_cwd)
                };
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
                // Directory entries (selected row highlighted with the accent), scrolled window.
                let list = self.files_list();
                let scroll = self.files_scroll.min(list.len());
                for (v, (name, is_dir, size)) in list.iter().skip(scroll).take(FILES_MAX_ROWS).enumerate() {
                    let rr = files_row_rect(r, v);
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
                // Scroll indicator when entries are hidden above/below.
                if list.len() > FILES_MAX_ROWS {
                    let hidden_above = scroll;
                    let hidden_below = list.len().saturating_sub(scroll + FILES_MAX_ROWS);
                    let ind = format!("{} above · {} below (Up/Down scrolls)", hidden_above, hidden_below);
                    let iw = fr.text_width(&ind, 11.0);
                    fr.draw_text(s, r.right() - iw - 16, files_row_rect(r, FILES_MAX_ROWS).y + 4, &ind, 11.0, theme.text_dim);
                }
                // Status line + preview, below the rows.
                let rows = (list.len() - scroll).min(FILES_MAX_ROWS);
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
            AppKind::Qasm => {
                // ---- Quantum IDE (WP-07): toolbar | sidebar | code | preview | status ----
                for (i, label) in ["Compile", "Run", "Save"].iter().enumerate() {
                    let b = qasm_btn_rect(r, i);
                    let accent = i == 1;
                    s.rounded_rect(b, 7, if accent { theme.accent } else { theme.surface_alt });
                    let lw = fr.text_width(label, 13.0);
                    fr.draw_text(s, b.x + (b.w - lw) / 2, b.y + 18, label, 13.0, if accent { theme.on_accent } else { theme.text });
                }
                fr.draw_text(s, r.x + 16 + 3 * 96 + 8, r.y + HEADER_H + 28, "F4 · F5 · F2 · F10 close", 11.0, theme.text_dim);

                // Sidebar: workspace .qasm files (the open one highlighted).
                let side = qasm_side_rect(r);
                s.rounded_rect(side, 8, theme.surface_alt);
                fr.draw_text(s, side.x + 10, side.y + 17, "EXPLORER", 11.0, theme.text_dim);
                let files = self.qasm_workspace_files();
                for (i, f) in files.iter().enumerate() {
                    let row = qasm_side_row_rect(r, i);
                    if row.bottom() > side.bottom() - 4 {
                        break;
                    }
                    let open = self.qasm_path.as_deref() == Some(f.as_str());
                    if open {
                        s.rounded_rect(row, 5, theme.accent);
                    }
                    let name = f.rsplit('/').next().unwrap_or(f);
                    fr.draw_text(s, row.x + 8, row.y + 15, name, 12.0, if open { theme.on_accent } else { theme.text });
                }
                if files.is_empty() {
                    fr.draw_text(s, side.x + 10, side.y + 40, "(no .qasm files)", 11.0, theme.text_dim);
                }

                // Code pane: line numbers, current-line highlight, real cursor caret.
                let code = qasm_code_rect(r);
                s.rounded_rect(code, 8, qos_ui::rgb(0x10, 0x12, 0x18));
                let line_h = 18;
                let max_rows = ((code.h - 12) / line_h).max(1) as usize;
                let (cl, cc) = self.qasm_cur;
                // Scroll so the cursor line is visible.
                let first = if cl >= max_rows { cl + 1 - max_rows } else { 0 };
                let gutter = 34;
                let mono = qos_ui::rgb(0xd8, 0xdc, 0xe4);
                let key_col = qos_ui::rgb(0x7a, 0xc8, 0xb0);
                let mut ty = code.y + 20;
                let problem_line = self.qasm_problem.as_ref().map(|(l, _)| *l).unwrap_or(0);
                for (li, line) in self.qasm_lines.iter().enumerate().skip(first).take(max_rows) {
                    if li == cl {
                        // Current-line highlight bar.
                        s.fill_rect(Rect::new(code.x + 2, ty - 13, code.w - 4, line_h), qos_ui::rgb(0x1c, 0x20, 0x2c));
                    }
                    let num = format!("{}", li + 1);
                    let nw = fr.text_width(&num, 11.0);
                    // The problem line's number turns red (VS Code-style gutter marker).
                    let num_col = if problem_line == li + 1 { qos_ui::rgb(0xe0, 0x60, 0x50) } else { theme.text_dim };
                    fr.draw_text(s, code.x + gutter - 8 - nw, ty, &num, 11.0, num_col);
                    let col = if line.starts_with("OPENQASM") || line.starts_with("qreg") || line.starts_with("creg") || line.starts_with("measure") || line.starts_with("include") {
                        key_col
                    } else {
                        mono
                    };
                    fr.draw_text(s, code.x + gutter, ty, line, 13.0, col);
                    if li == cl {
                        // Caret at the cursor column (proportional-font width of the prefix).
                        let prefix: String = line.chars().take(cc).collect();
                        let cw = fr.text_width(&prefix, 13.0);
                        s.fill_rect(Rect::new(code.x + gutter + cw, ty - 12, 2, 15), theme.accent);
                    }
                    ty += line_h;
                }

                // Live circuit preview strip (reparsed on each edit) / problem marker.
                let pv = qasm_preview_rect(r);
                s.rounded_rect(pv, 8, theme.surface_alt);
                match (&self.qasm_problem, &self.qasm_preview) {
                    (Some((pline, msg)), _) => {
                        fr.draw_text(s, pv.x + 10, pv.y + 20, "(!)", 13.0, qos_ui::rgb(0xe0, 0x60, 0x50));
                        fr.draw_text(s, pv.x + 36, pv.y + 20, msg, 12.0, theme.text);
                        if *pline > 0 {
                            fr.draw_text(s, pv.x + 36, pv.y + 40, "click here (or F8) to jump to the line", 11.0, theme.text_dim);
                        }
                    }
                    (None, Some((nq, instrs))) => {
                        let shown_q = (*nq).min(4);
                        let wire_x0 = pv.x + 34;
                        let wire_x1 = pv.right() - 12;
                        let step = 26;
                        let max_cols = ((wire_x1 - wire_x0 - 8) / step).max(1) as usize;
                        for q in 0..shown_q {
                            let wy = pv.y + 16 + q as i32 * 20;
                            fr.draw_text(s, pv.x + 8, wy + 5, &format!("q{}", q), 11.0, theme.text_dim);
                            s.fill_rect(Rect::new(wire_x0, wy, wire_x1 - wire_x0, 1), theme.text_dim);
                        }
                        use crate::quantum::parser::Instruction as I;
                        let mut col = 0usize;
                        for inst in instrs.iter() {
                            if col >= max_cols {
                                fr.draw_text(s, wire_x1 - 18, pv.bottom() - 8, "...", 12.0, theme.text_dim);
                                break;
                            }
                            let gx = wire_x0 + 8 + col as i32 * step;
                            let wy = |q: usize| pv.y + 16 + (q.min(3)) as i32 * 20;
                            let boxed = |s: &mut Surface, fr: &mut FontRenderer, q: usize, label: &str| {
                                if q >= shown_q {
                                    return;
                                }
                                let b = Rect::new(gx - 9, wy(q) - 8, 18, 16);
                                s.rounded_rect(b, 4, qos_ui::rgb(0x8a, 0x5c, 0xd8));
                                let lw = fr.text_width(label, 10.0);
                                fr.draw_text(s, b.x + (b.w - lw) / 2, b.y + 12, label, 10.0, qos_ui::rgb(0xff, 0xff, 0xff));
                            };
                            match inst {
                                I::H(q) => boxed(s, fr, *q, "H"),
                                I::X(q) => boxed(s, fr, *q, "X"),
                                I::Y(q) => boxed(s, fr, *q, "Y"),
                                I::Z(q) => boxed(s, fr, *q, "Z"),
                                I::S(q) => boxed(s, fr, *q, "S"),
                                I::T(q) => boxed(s, fr, *q, "T"),
                                I::Rx(q, _) => boxed(s, fr, *q, "Rx"),
                                I::Ry(q, _) => boxed(s, fr, *q, "Ry"),
                                I::Rz(q, _) => boxed(s, fr, *q, "Rz"),
                                I::P(q, _) => boxed(s, fr, *q, "P"),
                                I::Cx(c, t) | I::Cz(c, t) | I::Swap(c, t) => {
                                    if *c < shown_q && *t < shown_q {
                                        let (y0, y1) = (wy(*c), wy(*t));
                                        let (top, bot) = if y0 < y1 { (y0, y1) } else { (y1, y0) };
                                        s.fill_rect(Rect::new(gx - 1, top, 2, bot - top), theme.accent);
                                        circle(s, gx, y0, 7, theme.accent);
                                        circle(s, gx, y1, 11, theme.accent);
                                        circle(s, gx, y1, 7, theme.surface_alt);
                                    }
                                }
                                I::Measure(q, _) => boxed(s, fr, *q, "M"),
                                I::Reset(q) => boxed(s, fr, *q, "0"),
                                I::Barrier(_) => {}
                            }
                            col += 1;
                        }
                        if *nq > shown_q {
                            fr.draw_text(s, pv.x + 8, pv.bottom() - 8, &format!("+{} more qubits", nq - shown_q), 10.0, theme.text_dim);
                        }
                        // Last run's top outcomes, right-aligned in the preview strip.
                        let mut hy = pv.y + 12;
                        for (bits, count) in self.qasm_result.iter().take(4) {
                            let label = format!("{} {}", bits, count);
                            let lw = fr.text_width(&label, 11.0);
                            fr.draw_text(s, pv.right() - lw - 10, hy + 6, &label, 11.0, theme.text);
                            hy += 16;
                        }
                    }
                    _ => {
                        fr.draw_text(s, pv.x + 10, pv.y + 20, "live circuit preview", 12.0, theme.text_dim);
                    }
                }

                // Status bar: compiler status left, cursor position right.
                let sy = pv.bottom() + 14;
                fr.draw_text(s, r.x + 12, sy, &self.qasm_status, 11.0, theme.text_dim);
                let pos = format!("Ln {}, Col {}", cl + 1, cc + 1);
                let pw = fr.text_width(&pos, 11.0);
                fr.draw_text(s, r.right() - pw - 12, sy, &pos, 11.0, theme.text_dim);
            }
            AppKind::Quantum => {
                // Gate palette (selected gate highlighted).
                for (i, (_, label)) in QLAB_PALETTE.iter().enumerate() {
                    let b = qlab_pal_rect(r, i);
                    let sel = i == self.qlab_sel;
                    s.rounded_rect(b, 6, if sel { theme.accent } else { theme.surface_alt });
                    let lw = fr.text_width(label, 12.0);
                    fr.draw_text(s, b.x + (b.w - lw) / 2, b.y + 17, label, 12.0, if sel { theme.on_accent } else { theme.text });
                }
                // Controls.
                let angle_lbl = format!("A = {}", qlab_angle_label(self.qlab_angle));
                let ctls: [(usize, &str, bool); 6] = [
                    (0, "Run", true),
                    (1, "Clear", false),
                    (2, "-", false),
                    (3, "+", false),
                    (4, angle_lbl.as_str(), false),
                    (5, "QASM", false),
                ];
                for (i, label, accent) in ctls {
                    let b = qlab_ctl_rect(r, i);
                    s.rounded_rect(b, 6, if accent { theme.accent } else { theme.surface_alt });
                    let lw = fr.text_width(label, 12.0);
                    fr.draw_text(s, b.x + (b.w - lw) / 2, b.y + 17, label, 12.0, if accent { theme.on_accent } else { theme.text });
                }
                // Circuit grid: one wire per qubit, gates drawn on top.
                let wire_col = theme.text_dim;
                for q in 0..self.qlab_qubits {
                    let cell0 = qlab_cell_rect(r, q, 0);
                    let wy = cell0.y + cell0.h / 2;
                    fr.draw_text(s, r.x + 14, wy + 5, &format!("q{}", q), 13.0, theme.text_dim);
                    let x0 = qlab_cell_rect(r, q, 0).x - 3;
                    let x1 = qlab_cell_rect(r, q, QLAB_COLS - 1).right() + 3;
                    s.fill_rect(Rect::new(x0, wy, x1 - x0, 2), wire_col);
                }
                // Cursor ring on the active cell.
                let (cq, ccol) = self.qlab_cursor;
                if cq < self.qlab_qubits {
                    let cell = qlab_cell_rect(r, cq, ccol);
                    s.rounded_rect(cell.inflate(3), 8, theme.accent);
                    s.rounded_rect(cell.inflate(1), 6, theme.surface);
                    // Redraw the wire segment through the cursor cell.
                    let wy = cell.y + cell.h / 2;
                    s.fill_rect(Rect::new(cell.x, wy, cell.w, 2), wire_col);
                }
                // Pending CX control marker.
                if let Some((pq, pcol)) = self.qlab_pending {
                    if pq < self.qlab_qubits {
                        let cell = qlab_cell_rect(r, pq, pcol);
                        circle(s, cell.x + cell.w / 2, cell.y + cell.h / 2, 12, theme.accent);
                    }
                }
                // Placed gates.
                for g in self.qlab_gates.iter() {
                    if g.q >= self.qlab_qubits || g.q2 >= self.qlab_qubits {
                        continue;
                    }
                    let cell = qlab_cell_rect(r, g.q, g.col);
                    let (ccx, ccy) = (cell.x + cell.w / 2, cell.y + cell.h / 2);
                    if g.kind == QG::Cx {
                        let tcell = qlab_cell_rect(r, g.q2, g.col);
                        let (tx, ty) = (tcell.x + tcell.w / 2, tcell.y + tcell.h / 2);
                        // Vertical connector, control dot, target ⊕.
                        let (top, bot) = if ccy < ty { (ccy, ty) } else { (ty, ccy) };
                        s.fill_rect(Rect::new(ccx - 1, top, 2, bot - top), theme.accent);
                        circle(s, ccx, ccy, 10, theme.accent);
                        circle(s, tx, ty, 16, theme.accent);
                        circle(s, tx, ty, 10, theme.surface);
                        s.fill_rect(Rect::new(tx - 7, ty - 1, 14, 2), theme.accent);
                        s.fill_rect(Rect::new(tx - 1, ty - 7, 2, 14), theme.accent);
                    } else {
                        let tint = qos_ui::rgb(0x8a, 0x5c, 0xd8);
                        s.rounded_rect(cell, 6, tint);
                        let is_r = matches!(g.kind, QG::Rx | QG::Ry | QG::Rz);
                        let label = QLAB_PALETTE.iter().find(|(k, _)| *k == g.kind).map(|(_, l)| *l).unwrap_or("?");
                        let size = if is_r { 11.0 } else { 14.0 };
                        let lw = fr.text_width(label, size);
                        fr.draw_text(s, ccx - lw / 2, ccy + 5, label, size, qos_ui::rgb(0xff, 0xff, 0xff));
                    }
                }
                // Status + histogram of the last run.
                let hist_y = qlab_cell_rect(r, self.qlab_qubits - 1, 0).bottom() + 14;
                fr.draw_text(s, r.x + 14, hist_y, &self.qlab_status, 12.0, theme.text_dim);
                let mut hy = hist_y + 12;
                let bar_max = r.w - 160;
                for (bits, count) in self.qlab_result.iter() {
                    fr.draw_text(s, r.x + 14, hy + 13, bits, 13.0, theme.text);
                    let bw = ((*count as i64 * bar_max as i64) / QLAB_SHOTS as i64) as i32;
                    s.rounded_rect(Rect::new(r.x + 76, hy + 2, bw.max(3), 14), 4, theme.accent);
                    fr.draw_text(s, r.x + 82 + bw.max(3), hy + 13, &format!("{}", count), 12.0, theme.text_dim);
                    hy += 20;
                }
                if self.qlab_result.is_empty() {
                    fr.draw_text(s, r.x + 14, hy + 14, "press Run to execute on the statevector simulator", 12.0, theme.text_dim);
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
                let disk_line = if crate::ahci::present() {
                    let mib = crate::ahci::capacity_sectors() * 512 / 1024 / 1024;
                    let fmt = if crate::diskfs::is_formatted() { "QOSFS" } else { "unformatted" };
                    format!("RAM fs active   -   SATA disk: {} MiB ({})", mib, fmt)
                } else {
                    "RAM fs active   -   SATA disk: none attached".to_string()
                };
                fr.draw_text(s, bx + 8, y, &disk_line, 14.0, theme.text_dim);
            }
            AppKind::Settings => {
                // Appearance: two clickable theme cards with a live mini-preview.
                fr.draw_text(s, bx, by, "Appearance", 15.0, theme.accent);
                for (i, name) in ["Dark", "Light"].iter().enumerate() {
                    let card = settings_card_rect(r, i);
                    let selected = theme.is_dark == (i == 0);
                    if selected {
                        s.rounded_rect(card.inflate(3), 12, theme.accent);
                    }
                    let (bg, fg, alt) = if i == 0 {
                        (qos_ui::rgb(0x1a, 0x1d, 0x26), qos_ui::rgb(0xe6, 0xea, 0xf2), qos_ui::rgb(0x2a, 0x2e, 0x3a))
                    } else {
                        (qos_ui::rgb(0xf2, 0xf3, 0xf7), qos_ui::rgb(0x20, 0x24, 0x2e), qos_ui::rgb(0xdd, 0xe0, 0xe8))
                    };
                    s.rounded_rect(card, 10, bg);
                    // Mini window preview inside the card.
                    s.rounded_rect(Rect::new(card.x + 14, card.y + 14, card.w - 28, 34), 6, alt);
                    s.fill_rect(Rect::new(card.x + 20, card.y + 24, card.w - 60, 4), fg);
                    s.fill_rect(Rect::new(card.x + 20, card.y + 34, card.w - 84, 4), fg);
                    let nw = fr.text_width(name, 14.0);
                    fr.draw_text(s, card.x + (card.w - nw) / 2, card.bottom() - 12, name, 14.0, fg);
                }
                // Info sections: real Display / Input / Storage / About rows.
                let mut y = settings_card_rect(r, 0).bottom() + 34;
                let (kbd, mice) = crate::xhci::hid_device_counts();
                let (used, total) = crate::allocator::heap_stats();
                let ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
                let disk = if crate::ahci::present() {
                    let mib = crate::ahci::capacity_sectors() * 512 / 1024 / 1024;
                    format!("SATA {} MiB — {}", mib, if crate::diskfs::is_formatted() { "QOSFS" } else { "unformatted" })
                } else {
                    "no disk attached".to_string()
                };
                let rows: [(&str, String); 5] = [
                    ("Display", format!("{} x {}  ·  32-bit true color  ·  UEFI framebuffer", self.w, self.h)),
                    ("Input", format!("{} USB keyboard(s), {} USB mouse/mice  ·  MSI-X", kbd, mice)),
                    ("Storage", disk),
                    ("Security", crate::security::status_line()),
                    ("About", format!("QOS 0.1 — Heptapus Group  ·  heap {}/{} MiB  ·  up {} s", used / 1024 / 1024, total / 1024 / 1024, ticks / 100)),
                ];
                for (label, value) in rows.iter() {
                    let row = Rect::new(r.x + 24, y, r.w - 48, 34);
                    s.rounded_rect(row, 8, theme.surface_alt);
                    fr.draw_text(s, row.x + 12, row.y + 22, label, 13.0, theme.accent);
                    fr.draw_text(s, row.x + 96, row.y + 22, value, 12.0, theme.text);
                    y += 40;
                }
            }
            AppKind::Calculator => {
                // Display.
                let dr = calc_display_rect(r);
                s.rounded_rect(dr, 8, qos_ui::rgb(0x12, 0x14, 0x1a));
                let dtxt = &self.calc.display;
                let dw = fr.text_width(dtxt, 24.0);
                fr.draw_text(s, dr.right() - dw - 14, dr.y + 31, dtxt, 24.0, qos_ui::rgb(0xd8, 0xdc, 0xe4));
                if let Some(op) = self.calc.pending {
                    let mut b = [0u8; 4];
                    fr.draw_text(s, dr.x + 12, dr.y + 31, op.encode_utf8(&mut b), 18.0, theme.accent);
                }
                // Button grid.
                for (i, label) in CALC_BTNS.iter().enumerate() {
                    let b = calc_btn_rect(r, i);
                    let is_op = matches!(*label, "/" | "*" | "-" | "+" | "=");
                    s.rounded_rect(b, 10, if is_op { theme.accent } else { theme.surface_alt });
                    let col = if is_op { theme.on_accent } else { theme.text };
                    let lw = fr.text_width(label, 20.0);
                    fr.draw_text(s, b.x + (b.w - lw) / 2, b.y + b.h / 2 + 8, label, 20.0, col);
                }
                let cb = calc_clear_rect(r);
                s.rounded_rect(cb, 8, theme.surface_alt);
                let cl = "Clear";
                let cw = fr.text_width(cl, 14.0);
                fr.draw_text(s, cb.x + (cb.w - cw) / 2, cb.y + 23, cl, 14.0, theme.text);
            }
            AppKind::Devices => {
                // Real hardware inventory: full PCI listing + input / storage / network summary.
                let mut y = by;
                let devs = crate::pci::devices();
                fr.draw_text(s, bx, y, &format!("PCI bus — {} devices", devs.len()), 15.0, theme.accent);
                y += 22;
                for d in devs.iter().take(9) {
                    let line = format!(
                        "{:02x}:{:02x}.{}  {:04x}:{:04x}  {}  {}",
                        d.bus, d.device, d.function, d.vendor_id, d.device_id,
                        crate::pci::vendor_name(d.vendor_id), d.class_name()
                    );
                    fr.draw_text(s, bx + 8, y, &line, 12.0, theme.text_dim);
                    y += 17;
                }
                y += 10;
                let (kbd, mice) = crate::xhci::hid_device_counts();
                fr.draw_text(s, bx, y, "Input (USB HID)", 15.0, theme.accent);
                y += 20;
                fr.draw_text(s, bx + 8, y, &format!("{} keyboard(s), {} mouse/mice — xHCI, MSI-X interrupts", kbd, mice), 13.0, theme.text);
                y += 26;
                fr.draw_text(s, bx, y, "Storage (AHCI/SATA)", 15.0, theme.accent);
                y += 20;
                let st = if crate::ahci::present() {
                    let mib = crate::ahci::capacity_sectors() * 512 / 1024 / 1024;
                    format!("{} MiB data disk — {}", mib, if crate::diskfs::is_formatted() { "QOSFS" } else { "unformatted" })
                } else {
                    "no SATA disk attached".to_string()
                };
                fr.draw_text(s, bx + 8, y, &st, 13.0, theme.text);
                y += 26;
                fr.draw_text(s, bx, y, "Network", 15.0, theme.accent);
                y += 20;
                let net = devs.iter().find(|d| d.class_code == 0x02);
                let nline = match net {
                    Some(d) => format!("{:04x}:{:04x} {} (no driver yet)", d.vendor_id, d.device_id, crate::pci::vendor_name(d.vendor_id)),
                    None => "no network device".to_string(),
                };
                fr.draw_text(s, bx + 8, y, &nline, 13.0, theme.text);
            }
            AppKind::Processes => {
                // Real runtime state: open UI apps (windows) + live kernel subsystem activity.
                let mut y = by;
                fr.draw_text(s, bx, y, &format!("Apps — {} window(s)", self.wins.len()), 15.0, theme.accent);
                y += 22;
                let top = self.wins.len().saturating_sub(1);
                for (i, win) in self.wins.iter().enumerate() {
                    let state = if i == top { "focused" } else { "running" };
                    fr.draw_text(s, bx + 8, y, &format!("{}  —  {}", app_title(win.kind), state), 13.0,
                        if i == top { theme.text } else { theme.text_dim });
                    y += 18;
                }
                y += 10;
                fr.draw_text(s, bx, y, "Kernel subsystems", 15.0, theme.accent);
                y += 22;
                let ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
                let (used, total) = crate::allocator::heap_stats();
                let jobs = crate::quantum::sim::SIM_JOBS.load(core::sync::atomic::Ordering::Relaxed);
                let (kbd, mice) = crate::xhci::hid_device_counts();
                let rows = [
                    format!("timer/APIC     100 Hz — {} ticks (uptime {} s)", ticks, ticks / 100),
                    format!("memory         heap {} KiB / {} MiB", used / 1024, total / 1024 / 1024),
                    format!("usb/xhci       {} kbd, {} mouse — interrupt-driven", kbd, mice),
                    format!("storage/ahci   {}", if crate::ahci::present() { "SATA data disk online" } else { "no disk" }),
                    format!("quantum/sim    {} job(s) executed since boot", jobs),
                ];
                for line in rows.iter() {
                    fr.draw_text(s, bx + 8, y, line, 13.0, theme.text);
                    y += 19;
                }
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
        // Real RTC clock (24-hour, refreshed each minute by the main loop).
        let dt = crate::rtc::read_datetime();
        let clock = format!("{:02}:{:02}", dt.hour, dt.minute);
        let cw = fr.text_width(&clock, 15.0);
        fr.draw_text(s, w - cw - 16, 21, &clock, 15.0, theme.text);

        // Windows in z-order (topmost non-minimized = focused; minimized ones are hidden).
        let top = self.wins.iter().rposition(|w| !w.minimized);
        for i in 0..self.wins.len() {
            if self.wins[i].minimized {
                continue;
            }
            self.draw_window(s, fr, i, Some(i) == top);
        }

        // Dock with app icons (open apps get an accent underline dot).
        let dock = self.dock_rect();
        s.drop_shadow(dock, 20, 18, theme.shadow, if theme.is_dark { 140 } else { 70 });
        s.rounded_rect_blend(dock, 20, theme.dock, 238);
        for (i, &kind) in APPS.iter().enumerate() {
            let ir = self.dock_icon_rect(i as i32);
            let tint = app_tint(kind, theme);
            s.rounded_rect(ir, 12, tint);
            draw_app_icon(s, kind, ir, tint);
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
    let mut last_minute = crate::rtc::read_datetime().minute;

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
                    if scancode == 0x44 {
                        // F10 closes the focused window from ANY app — the universal keyboard
                        // escape hatch (text-entry apps consume letters, so 'w' can't be it).
                        if desk.wins.pop().is_some() {
                            desk.mark_full();
                        }
                        continue;
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
                    } else if desk.focused().map_or(false, |w| w.kind == AppKind::Qasm) {
                        // Focused Quantum IDE: real cursor editing + toolchain function keys.
                        match scancode {
                            0x3E => desk.qasm_compile(),      // F4
                            0x3F => desk.qasm_run_buf(),      // F5
                            0x3C => desk.qasm_save(),         // F2
                            0x42 => desk.qasm_goto_problem(), // F8 → jump to problem line
                            0x48 => desk.qasm_move(0, -1),    // Up
                            0x50 => desk.qasm_move(0, 1),     // Down
                            0x4B => desk.qasm_move(-1, 0),    // Left
                            0x4D => desk.qasm_move(1, 0),     // Right
                            0x0E => desk.qasm_backspace(),    // Backspace (at cursor)
                            0x1C => desk.qasm_newline(),      // Enter (split at cursor)
                            _ => {
                                if let Some(c) = scancode_to_char(scancode, shift) {
                                    desk.qasm_insert(c);
                                }
                            }
                        }
                    } else if desk.focused().map_or(false, |w| w.kind == AppKind::Quantum) {
                        // Focused Quantum Lab: full keyboard circuit editing.
                        match scancode {
                            0x11 => {
                                desk.wins.pop();
                                desk.mark_full();
                            } // w → close
                            0x48 => {
                                desk.qlab_cursor.0 = desk.qlab_cursor.0.saturating_sub(1);
                                desk.mark_top_window();
                            } // Up
                            0x50 => {
                                desk.qlab_cursor.0 = (desk.qlab_cursor.0 + 1).min(desk.qlab_qubits - 1);
                                desk.mark_top_window();
                            } // Down
                            0x4B => {
                                desk.qlab_cursor.1 = desk.qlab_cursor.1.saturating_sub(1);
                                desk.mark_top_window();
                            } // Left
                            0x4D => {
                                desk.qlab_cursor.1 = (desk.qlab_cursor.1 + 1).min(QLAB_COLS - 1);
                                desk.mark_top_window();
                            } // Right
                            0x39 => {
                                let (q, col) = desk.qlab_cursor;
                                desk.qlab_place(q, col);
                            } // Space → place/remove
                            0x1C => desk.qlab_run(), // Enter → run
                            0x0E => {
                                desk.qlab_gates.clear();
                                desk.qlab_result.clear();
                                desk.qlab_pending = None;
                                desk.mark_top_window();
                            } // Backspace → clear circuit
                            _ => {
                                if let Some(c) = scancode_to_char(scancode, shift) {
                                    let sel = match c {
                                        'h' => Some(0),
                                        'x' => Some(1),
                                        'y' => Some(2),
                                        'z' => Some(3),
                                        's' => Some(4),
                                        't' => Some(5),
                                        'r' => Some(match desk.qlab_sel {
                                            6 => 7,
                                            7 => 8,
                                            _ => 6,
                                        }), // r cycles RX→RY→RZ
                                        'c' => Some(9),
                                        _ => None,
                                    };
                                    if let Some(i) = sel {
                                        desk.qlab_sel = i;
                                        desk.qlab_pending = None;
                                        desk.mark_top_window();
                                    } else if c == 'a' {
                                        desk.qlab_angle = if desk.qlab_angle >= 8 { 1 } else { desk.qlab_angle + 1 };
                                        desk.mark_top_window();
                                    } else if c == 'e' {
                                        // Export the circuit to QASM Studio.
                                        let src = desk.qlab_to_qasm();
                                        desk.qasm_open(None, src);
                                    }
                                }
                            }
                        }
                    } else if desk.top_is_calc() {
                        // Focused Calculator: digits + operators from the keyboard ('w' still
                        // closes the window so the keyboard never gets trapped).
                        match scancode {
                            0x11 => {
                                desk.wins.pop();
                                desk.mark_full();
                            } // w → close
                            0x0E => {
                                desk.calc.input('C');
                                desk.mark_top_window();
                            } // Backspace → clear
                            0x1C => {
                                desk.calc.input('=');
                                desk.mark_top_window();
                            } // Enter → =
                            _ => {
                                if let Some(c) = scancode_to_char(scancode, shift) {
                                    if matches!(c, '0'..='9' | '.' | '+' | '-' | '*' | '/' | '=' | 'c' | 'C') {
                                        desk.calc.input(c);
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
                            s @ 0x02..=0x0B => {
                                // Number keys 1–9 and 0 (10th) open the dock apps in order.
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
                            0x48 if files_focused => desk.files_nav(false), // Up → selection up
                            0x50 if files_focused => desk.files_nav(true),  // Down → selection down
                            0x1C if files_focused => desk.files_activate(), // Enter → open selection
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
        // Live refresh (~1 Hz): repaint a focused live-data window (System Monitor / Processes),
        // and repaint the top bar when the RTC minute rolls over so the menu-bar clock is real.
        let now_ticks = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64;
        if now_ticks - last_refresh >= 100 {
            last_refresh = now_ticks;
            if desk.top_is_live() {
                desk.mark_top_window();
            }
            let minute = crate::rtc::read_datetime().minute;
            if minute != last_minute {
                last_minute = minute;
                desk.mark_region(Rect::new(0, 0, w as i32, BAR_H));
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
