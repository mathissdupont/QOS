//! Graphical desktop (Phase 1 — see docs/PLAN.md).
//!
//! An interactive Mode 13h UI driven entirely by the unified input event queue (Phase 0.1):
//! a live mouse cursor with save-under, an event loop (draw → wait event → update), draggable
//! windows with clickable close buttons, generic reusable widgets (`Button`, `Menu`), a bottom
//! taskbar with a Start menu, and a launchable **Quantum Lab** app that runs a real Bell-state
//! circuit on the in-kernel statevector simulator and plots the measurement histogram.
//! ESC returns to the text shell.

use crate::draw::{self, color};
use crate::input::{self, InputEvent, MouseButton};
use crate::quantum;

const W: usize = 320;
const H: usize = 200;

const TOPBAR_H: i32 = 12;
const TASKBAR_H: i32 = 14;

const CW: usize = 9;
const CH: usize = 16;

/// Arrow cursor: 'X' = black outline, 'O' = white fill, ' ' = transparent.
static CURSOR: [&[u8; CW]; CH] = [
    b"X        ",
    b"XX       ",
    b"XOX      ",
    b"XOOX     ",
    b"XOOOX    ",
    b"XOOOOX   ",
    b"XOOOOOX  ",
    b"XOOOOOOX ",
    b"XOOOOOOOX",
    b"XOOOOXXXX",
    b"XOOXOX   ",
    b"XOX XOX  ",
    b"XX  XOX  ",
    b"X    XOX ",
    b"      XOX",
    b"       XX",
];

struct Cursor {
    x: i32,
    y: i32,
    saved: [u8; CW * CH],
}

impl Cursor {
    fn new(x: i32, y: i32) -> Self {
        Self { x, y, saved: [0; CW * CH] }
    }

    fn save_bg(&mut self) {
        for row in 0..CH {
            for col in 0..CW {
                let px = (self.x + col as i32).max(0) as usize;
                let py = (self.y + row as i32).max(0) as usize;
                self.saved[row * CW + col] = draw::get_pixel(px, py);
            }
        }
    }

    fn restore_bg(&self) {
        for row in 0..CH {
            for col in 0..CW {
                let px = (self.x + col as i32).max(0) as usize;
                let py = (self.y + row as i32).max(0) as usize;
                draw::put_pixel(px, py, self.saved[row * CW + col]);
            }
        }
    }

    fn draw(&self) {
        for row in 0..CH {
            for col in 0..CW {
                let px = (self.x + col as i32).max(0) as usize;
                let py = (self.y + row as i32).max(0) as usize;
                match CURSOR[row][col] {
                    b'X' => draw::put_pixel(px, py, color::BLACK),
                    b'O' => draw::put_pixel(px, py, color::WHITE),
                    _ => {}
                }
            }
        }
    }

    fn hotspot(&self) -> (i32, i32) {
        (self.x, self.y) // tip of the arrow
    }
}

// ── Generic widgets ─────────────────────────────────────────────────────────────────────

/// A clickable push-button with a 3D bevel (Win95-style: light top/left, dark bottom/right;
/// inverted while pressed). Coordinates are absolute screen pixels.
struct Button {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    label: &'static str,
}

impl Button {
    fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    fn draw(&self, pressed: bool) {
        use color::*;
        let (x, y, w, h) = (self.x as usize, self.y as usize, self.w as usize, self.h as usize);
        draw::fill_rect(x, y, w, h, LTGRAY);
        let (tl, br) = if pressed { (DKGRAY, WHITE) } else { (WHITE, DKGRAY) };
        draw::fill_rect(x, y, w, 1, tl); // top
        draw::fill_rect(x, y, 1, h, tl); // left
        draw::fill_rect(x, y + h - 1, w, 1, br); // bottom
        draw::fill_rect(x + w - 1, y, 1, h, br); // right
        // Centered label, nudged 1px when pressed for a tactile feel.
        let tw = self.label.len() as i32 * 8;
        let off = if pressed { 1 } else { 0 };
        let tx = (self.x + (self.w - tw) / 2 + off).max(self.x + 1) as usize;
        let ty = (self.y + (self.h - 8) / 2 + off) as usize;
        draw::draw_string(tx, ty, self.label, BLACK, LTGRAY);
    }
}

const MENU_ITEM_H: i32 = 12;

/// A popup menu: a bordered vertical list of labels anchored at (x, y). The hovered row is
/// highlighted. `item_at` maps a screen point to an item index for hit-testing.
struct Menu {
    x: i32,
    y: i32,
    w: i32,
    items: &'static [&'static str],
}

impl Menu {
    fn height(&self) -> i32 {
        self.items.len() as i32 * MENU_ITEM_H + 2
    }

    fn item_at(&self, px: i32, py: i32) -> Option<usize> {
        if px < self.x || px >= self.x + self.w || py < self.y + 1 || py >= self.y + self.height() - 1
        {
            return None;
        }
        let idx = ((py - self.y - 1) / MENU_ITEM_H) as usize;
        if idx < self.items.len() {
            Some(idx)
        } else {
            None
        }
    }

    fn draw(&self, hover: Option<usize>) {
        use color::*;
        let (x, y, w, h) = (self.x as usize, self.y as usize, self.w as usize, self.height() as usize);
        draw::fill_rect(x, y, w, h, LTGRAY);
        draw::rect(x, y, w, h, DKGRAY);
        for (i, label) in self.items.iter().enumerate() {
            let iy = self.y + 1 + i as i32 * MENU_ITEM_H;
            let (bg, fg) = if hover == Some(i) { (BLUE, WHITE) } else { (LTGRAY, BLACK) };
            draw::fill_rect(self.x as usize + 1, iy as usize, w - 2, MENU_ITEM_H as usize, bg);
            draw::draw_string(self.x as usize + 5, iy as usize + 2, label, fg, bg);
        }
    }
}

/// A draggable, closable window frame. Body content is drawn by the owner via `body_origin`.
struct Window {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    title: &'static str,
    open: bool,
}

impl Window {
    const TITLE_H: i32 = 12;

    fn body_origin(&self) -> (i32, i32) {
        (self.x + 4, self.y + Self::TITLE_H + 4)
    }

    fn draw_frame(&self) {
        use color::*;
        if !self.open {
            return;
        }
        let (x, y, w, h) = (self.x as usize, self.y as usize, self.w as usize, self.h as usize);
        draw::fill_rect(x, y, w, h, LTGRAY);
        draw::rect(x, y, w, h, DKGRAY);
        draw::fill_rect(x + 2, y + 2, w - 4, Self::TITLE_H as usize, BLUE);
        draw::draw_string(x + 5, y + 4, self.title, WHITE, BLUE);
        let cb = self.close_btn();
        draw::fill_rect(cb.0 as usize, cb.1 as usize, 10, 10, RED);
        draw::draw_string(cb.0 as usize + 1, cb.1 as usize + 1, "x", WHITE, RED);
    }

    fn close_btn(&self) -> (i32, i32) {
        (self.x + self.w - 13, self.y + 3)
    }

    fn in_titlebar(&self, px: i32, py: i32) -> bool {
        self.open
            && px >= self.x
            && px < self.x + self.w
            && py >= self.y
            && py < self.y + Self::TITLE_H + 2
    }

    fn in_close(&self, px: i32, py: i32) -> bool {
        let (cx, cy) = self.close_btn();
        self.open && px >= cx && px < cx + 10 && py >= cy && py < cy + 10
    }

    fn clamp_onscreen(&mut self) {
        self.x = self.x.clamp(0, W as i32 - self.w);
        self.y = self.y.clamp(TOPBAR_H, H as i32 - TASKBAR_H - self.h);
    }
}

// ── Quantum Lab app ─────────────────────────────────────────────────────────────────────

const SHOTS: u64 = 512;

/// A launchable app: a window showing the Bell circuit (H · CNOT) with a Run button that
/// executes it on the in-kernel statevector simulator and plots the measurement histogram.
struct QuantumApp {
    win: Window,
    /// (count_00, count_11, shots) from the last run, if any.
    result: Option<(u64, u64, u64)>,
}

impl QuantumApp {
    fn new() -> Self {
        Self {
            win: Window { x: 60, y: 28, w: 200, h: 150, title: "Quantum Lab", open: false },
            result: None,
        }
    }

    fn run_btn(&self) -> Button {
        Button {
            x: self.win.x + 8,
            y: self.win.y + self.win.h - 18,
            w: 78,
            h: 12,
            label: "Run 512x",
        }
    }

    /// Execute the Bell circuit on the in-kernel simulator and store the histogram.
    fn run(&mut self) {
        let circuit = quantum::bell_circuit();
        let res = circuit.run_shots(SHOTS);
        let zeros = res.get("00");
        let ones = res.get("11");
        self.result = Some((zeros, ones, res.shots));
        crate::serial_println!("[GFXUI] Quantum Lab: Bell {} shots -> 00={} 11={}", res.shots, zeros, ones);
    }

    fn draw(&self, run_pressed: bool) {
        use color::*;
        if !self.win.open {
            return;
        }
        self.win.draw_frame();
        let (bx, by) = self.win.body_origin();
        let (bx, by) = (bx as usize, by as usize);

        // Circuit diagram: two qubit wires with an H gate then a CNOT.
        let wire0 = by + 8;
        let wire1 = by + 30;
        let wstart = bx + 18;
        let wend = bx + 150;
        draw::draw_string(bx, wire0 - 3, "q0", BLACK, LTGRAY);
        draw::draw_string(bx, wire1 - 3, "q1", BLACK, LTGRAY);
        draw::fill_rect(wstart, wire0, wend - wstart, 1, BLACK);
        draw::fill_rect(wstart, wire1, wend - wstart, 1, BLACK);

        // H gate box on q0.
        let hx = wstart + 16;
        draw::fill_rect(hx, wire0 - 5, 11, 11, WHITE);
        draw::rect(hx, wire0 - 5, 11, 11, BLACK);
        draw::draw_string(hx + 2, wire0 - 3, "H", BLACK, WHITE);

        // CNOT: control dot on q0, target (+) on q1, connected vertically.
        let cx = wstart + 56;
        draw::fill_rect(cx, wire0, 1, wire1 - wire0, BLACK); // vertical link
        draw::fill_rect(cx - 2, wire0 - 2, 5, 5, BLACK); // control dot
        draw::rect(cx - 4, wire1 - 4, 9, 9, BLACK); // target ring
        draw::fill_rect(cx, wire1 - 4, 1, 9, BLACK); // target plus (v)
        draw::fill_rect(cx - 4, wire1, 9, 1, BLACK); // target plus (h)

        // Histogram of the last run (or a hint to press Run).
        let chart_y = by + 48;
        match self.result {
            None => {
                draw::draw_string(bx, chart_y, "Press Run to measure", DKGRAY, LTGRAY);
                draw::draw_string(bx, chart_y + 10, "the Bell state.", DKGRAY, LTGRAY);
            }
            Some((zeros, ones, shots)) => {
                let max_bar = 48i32;
                let p00 = (zeros * 100 / shots) as i32;
                let p11 = (ones * 100 / shots) as i32;
                let b00 = (zeros as i32 * max_bar / shots as i32).max(1);
                let b11 = (ones as i32 * max_bar / shots as i32).max(1);
                let base = chart_y + max_bar as usize;
                // |00> bar
                draw::fill_rect(bx + 8, base - b00 as usize, 18, b00 as usize, GREEN);
                draw::draw_string(bx + 6, base + 2, "00", BLACK, LTGRAY);
                draw_pct(bx + 4, chart_y - 2, p00);
                // |11> bar
                draw::fill_rect(bx + 48, base - b11 as usize, 18, b11 as usize, LTBLUE);
                draw::draw_string(bx + 46, base + 2, "11", BLACK, LTGRAY);
                draw_pct(bx + 44, chart_y - 2, p11);
                draw::draw_string(bx + 80, chart_y + 8, "Bell state:", BLACK, LTGRAY);
                draw::draw_string(bx + 80, chart_y + 18, "|00>+|11>", BLACK, LTGRAY);
                draw::draw_string(bx + 80, chart_y + 30, "01,10 ~ 0%", DKGRAY, LTGRAY);
            }
        }

        self.run_btn().draw(run_pressed);
    }
}

/// Draw a 0..=100 percentage as up to 3 digits + '%'.
fn draw_pct(x: usize, y: usize, pct: i32) {
    let mut buf = [b' '; 4];
    let p = pct.clamp(0, 100) as usize;
    let s: &[u8] = if p >= 100 {
        b"100%"
    } else if p >= 10 {
        buf[0] = b'0' + (p / 10) as u8;
        buf[1] = b'0' + (p % 10) as u8;
        buf[2] = b'%';
        &buf[..3]
    } else {
        buf[0] = b'0' + p as u8;
        buf[1] = b'%';
        &buf[..2]
    };
    let text = core::str::from_utf8(s).unwrap_or("?");
    draw::draw_string(x, y, text, color::BLACK, color::LTGRAY);
}

// ── Desktop composition ─────────────────────────────────────────────────────────────────

const START_BTN: Button = Button { x: 2, y: (H as i32) - 12, w: 50, h: 10, label: "Start" };

const MENU_ITEMS: [&str; 6] =
    ["Quantum Lab", "Files", "Task Monitor", "Display", "About", "Exit"];

fn start_menu() -> Menu {
    let items: &'static [&'static str] = &MENU_ITEMS;
    let m_w = 108;
    let m = Menu { x: 2, y: 0, w: m_w, items };
    Menu { y: H as i32 - TASKBAR_H - m.height(), ..m }
}

/// The simple (text-body) apps. Quantum Lab is separate because it has interactive widgets.
#[derive(Clone, Copy, PartialEq)]
enum AppKind {
    Welcome,
    About,
    SysMon,
    Files,
    Display,
}

/// Index i of `Scene::apps` corresponds to `APP_KINDS[i]`.
const APP_KINDS: [AppKind; 5] =
    [AppKind::Welcome, AppKind::About, AppKind::SysMon, AppKind::Files, AppKind::Display];
const SYSMON_IDX: usize = 2;

struct Scene {
    apps: [Window; 5],
    qapp: QuantumApp,
    start_open: bool,
}

impl Scene {
    fn open_kind(&mut self, kind: AppKind) {
        for (i, k) in APP_KINDS.iter().enumerate() {
            if *k == kind {
                self.apps[i].open = true;
                self.apps[i].clamp_onscreen();
            }
        }
    }
}

/// Draw the text body of a simple app inside its window frame.
fn draw_app_body(kind: AppKind, win: &Window) {
    use color::*;
    let (bx, by) = win.body_origin();
    let (bx, by) = (bx as usize, by as usize);
    match kind {
        AppKind::Welcome => {
            draw::draw_string(bx, by, "Welcome to QOS.", BLACK, LTGRAY);
            draw::draw_string(bx, by + 12, "Click Start for apps,", BLACK, LTGRAY);
            draw::draw_string(bx, by + 24, "or Q = Quantum Lab.", BLACK, LTGRAY);
            draw::draw_string(bx, by + 40, "Drag title; [x] closes.", DKGRAY, LTGRAY);
        }
        AppKind::About => {
            draw::draw_string(bx, by, "QOS - Quantum OS", BLACK, LTGRAY);
            draw::draw_string(bx, by + 14, "Preemptive ring3 kernel", BLACK, LTGRAY);
            draw::draw_string(bx, by + 26, "W^X + per-proc paging", BLACK, LTGRAY);
            draw::draw_string(bx, by + 38, "Quantum-ready control", BLACK, LTGRAY);
            draw::draw_string(bx, by + 54, "Heptapus Group", DKGRAY, LTGRAY);
        }
        AppKind::SysMon => {
            let mut nb = [0u8; 20];
            let mut tb = [0u8; 20];
            draw::draw_string(bx, by, "Scheduler: preemptive", BLACK, LTGRAY);
            draw::draw_string(bx, by + 14, "Processes:", BLACK, LTGRAY);
            draw::draw_string(bx + 8, by + 26, "- shell    (ring0)", DKGRAY, LTGRAY);
            draw::draw_string(bx + 8, by + 38, "- bg-worker(ring0)", DKGRAY, LTGRAY);
            draw::draw_string(bx, by + 56, "bg ticks:", BLACK, LTGRAY);
            draw::draw_string(bx + 80, by + 56, fmt_u64(crate::kthread::bg_counter(), &mut nb), BLUE, LTGRAY);
            draw::draw_string(bx, by + 68, "uptime:", BLACK, LTGRAY);
            let t = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
            draw::draw_string(bx + 80, by + 68, fmt_u64(t, &mut tb), BLUE, LTGRAY);
        }
        AppKind::Display => {
            // Proof of the ADR-0014 Stage 3 framebuffer path: shows which backend the desktop is
            // rendering through and, when on the linear framebuffer, the physical resolution and
            // the integer scale factor applied to the 320x200 logical canvas.
            let (fb, pw, ph, scale) = draw::backend_info();
            draw::draw_string(bx, by, "Display", BLACK, LTGRAY);
            let mut wb = [0u8; 20];
            let mut hb = [0u8; 20];
            let mut sb = [0u8; 20];
            if fb {
                draw::draw_string(bx, by + 16, "Backend: framebuffer", BLUE, LTGRAY);
                draw::draw_string(bx, by + 28, "Res:", BLACK, LTGRAY);
                draw::draw_string(bx + 40, by + 28, fmt_u64(pw as u64, &mut wb), DKGRAY, LTGRAY);
                draw::draw_string(bx + 88, by + 28, "x", BLACK, LTGRAY);
                draw::draw_string(bx + 100, by + 28, fmt_u64(ph as u64, &mut hb), DKGRAY, LTGRAY);
                draw::draw_string(bx, by + 40, "Scale:", BLACK, LTGRAY);
                draw::draw_string(bx + 56, by + 40, fmt_u64(scale as u64, &mut sb), DKGRAY, LTGRAY);
                draw::draw_string(bx + 72, by + 40, "x  (logical 320x200)", DKGRAY, LTGRAY);
                draw::draw_string(bx, by + 56, "UEFI/VESA linear FB", BLACK, LTGRAY);
            } else {
                draw::draw_string(bx, by + 16, "Backend: VGA Mode 13h", BLUE, LTGRAY);
                draw::draw_string(bx, by + 28, "Res: 320 x 200 (1x)", DKGRAY, LTGRAY);
                draw::draw_string(bx, by + 44, "Legacy BIOS fallback", BLACK, LTGRAY);
            }
        }
        AppKind::Files => {
            draw::draw_string(bx, by, "File Manager   /", BLACK, LTGRAY);
            // Real directory listing from the in-kernel filesystem (Phase 3.4).
            let entries = crate::fs::get_entries(b"");
            if entries.is_empty() {
                draw::draw_string(bx + 4, by + 16, "(empty)", DKGRAY, LTGRAY);
            } else {
                let mut y = by + 16;
                for (name, is_dir, _size) in entries.iter().take(6) {
                    let tag = if *is_dir { "[d]" } else { "[f]" };
                    let fg = if *is_dir { BLUE } else { DKGRAY };
                    draw::draw_string(bx + 4, y, tag, fg, LTGRAY);
                    let shown = if name.len() > 16 { &name[..16] } else { name.as_str() };
                    draw::draw_string(bx + 32, y, shown, BLACK, LTGRAY);
                    y += 12;
                }
            }
        }
    }
}

/// Format a u64 into a decimal string in `buf`, returning the slice.
fn fmt_u64(mut n: u64, buf: &mut [u8; 20]) -> &str {
    let mut i = buf.len();
    if n == 0 {
        i -= 1;
        buf[i] = b'0';
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("?")
}

/// Draw the live taskbar status (right side): a background-task counter (proof that a
/// preemptive kernel thread runs *behind* the GUI) and an HH:MM:SS clock. Cheap enough to
/// repaint a few times a second.
fn draw_status() {
    use color::*;
    let ty = H - TASKBAR_H as usize;
    let x0 = 116usize;
    draw::fill_rect(x0, ty + 1, W - x0, TASKBAR_H as usize - 2, LTGRAY);

    // Background-task counter — advances while you use the desktop (Phase 2 preemption, live).
    let mut nbuf = [0u8; 20];
    draw::draw_string(x0, ty + 3, "bg:", BLACK, LTGRAY);
    draw::draw_string(x0 + 24, ty + 3, fmt_u64(crate::kthread::bg_counter(), &mut nbuf), BLUE, LTGRAY);

    // Clock HH:MM:SS from the RTC.
    let dt = crate::rtc::read_datetime();
    let (h, m, s) = (dt.hour as u32, dt.minute as u32, dt.second as u32);
    let clk = [
        b'0' + (h / 10 % 10) as u8, b'0' + (h % 10) as u8, b':',
        b'0' + (m / 10 % 10) as u8, b'0' + (m % 10) as u8, b':',
        b'0' + (s / 10 % 10) as u8, b'0' + (s % 10) as u8,
    ];
    let clk = core::str::from_utf8(&clk).unwrap_or("--:--:--");
    draw::draw_string(W - 8 * 8 - 2, ty + 3, clk, BLACK, LTGRAY);
}

fn draw_scene(scene: &Scene, hover: Option<usize>, start_pressed: bool, run_pressed: bool) {
    use color::*;
    draw::clear(TEAL);
    // Top status bar.
    draw::fill_rect(0, 0, W, TOPBAR_H as usize, BLUE);
    draw::draw_string(2, 2, "QOS Desktop", WHITE, BLUE);
    draw::draw_string(W - 17 * 8, 2, "ESC: exit to shell", WHITE, BLUE);

    // Simple app windows (z-order = array order), then the Quantum Lab on top.
    for (i, win) in scene.apps.iter().enumerate() {
        if win.open {
            win.draw_frame();
            draw_app_body(APP_KINDS[i], win);
        }
    }
    scene.qapp.draw(run_pressed);

    // Taskbar.
    let ty = H - TASKBAR_H as usize;
    draw::fill_rect(0, ty, W, TASKBAR_H as usize, LTGRAY);
    draw::fill_rect(0, ty, W, 1, WHITE);
    START_BTN.draw(start_pressed || scene.start_open);
    if scene.qapp.win.open {
        draw::draw_string(58, ty + 3, "QLab", BLACK, LTGRAY);
    }
    draw_status();

    // Start menu popup (drawn last so it's on top of everything).
    if scene.start_open {
        start_menu().draw(hover);
    }
}

/// Run the interactive desktop until ESC. Returns to text mode on exit.
pub fn run() {
    crate::serial_println!("[GFXUI] entering interactive desktop");
    draw::enter();

    // Phase 3.1: a real preemptive background task runs behind the GUI (its counter ticks up
    // live in the taskbar) — Phase-2 multitasking made visible. Stopped on every exit path.
    crate::kthread::start_background_worker();

    let mut scene = Scene {
        apps: [
            Window { x: 16, y: 84, w: 184, h: 76, title: "Welcome", open: true },
            Window { x: 76, y: 40, w: 176, h: 82, title: "About", open: false },
            Window { x: 40, y: 28, w: 184, h: 104, title: "Task Monitor", open: false },
            Window { x: 52, y: 52, w: 176, h: 104, title: "Files", open: false },
            Window { x: 64, y: 44, w: 184, h: 92, title: "Display", open: false },
        ],
        qapp: QuantumApp::new(),
        start_open: false,
    };

    let mut cur = Cursor::new((W / 2) as i32, (H / 2) as i32);
    let mut hover: Option<usize> = None;
    let mut start_pressed = false;
    let mut run_pressed = false;

    // Drag state.
    #[derive(PartialEq)]
    enum Drag {
        None,
        Quantum,
        App(usize),
    }
    let mut drag = Drag::None;
    let mut drag_dx = 0i32;
    let mut drag_dy = 0i32;

    draw_scene(&scene, hover, start_pressed, run_pressed);
    cur.save_bg();
    cur.draw();

    let mut last_status = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);

    loop {
        if input::has_events() {
            cur.restore_bg();
            let mut dirty = false;

            while let Some(ev) = input::poll() {
                match ev {
                    InputEvent::Key { scancode: 0x01, pressed: true } => {
                        crate::kthread::stop_background_worker();
                        draw::leave();
                        crate::serial_println!("[GFXUI] exited to text mode");
                        return;
                    }
                    // Keyboard shortcuts (also makes the desktop usable without a mouse):
                    // Q=Quantum Lab, R=run it, W=Welcome, A=About, M=Monitor, F=Files.
                    InputEvent::Key { scancode: 0x10, pressed: true } => {
                        scene.qapp.win.open = true;
                        scene.qapp.win.clamp_onscreen();
                        scene.start_open = false;
                        dirty = true;
                    }
                    InputEvent::Key { scancode: 0x13, pressed: true } => {
                        if scene.qapp.win.open {
                            scene.qapp.run();
                            dirty = true;
                        }
                    }
                    InputEvent::Key { scancode: 0x11, pressed: true } => {
                        scene.open_kind(AppKind::Welcome);
                        scene.start_open = false;
                        dirty = true;
                    }
                    InputEvent::Key { scancode: 0x1E, pressed: true } => {
                        scene.open_kind(AppKind::About);
                        scene.start_open = false;
                        dirty = true;
                    }
                    InputEvent::Key { scancode: 0x32, pressed: true } => {
                        scene.open_kind(AppKind::SysMon);
                        scene.start_open = false;
                        dirty = true;
                    }
                    InputEvent::Key { scancode: 0x21, pressed: true } => {
                        scene.open_kind(AppKind::Files);
                        scene.start_open = false;
                        dirty = true;
                    }
                    // D = Display (backend / resolution / scale)
                    InputEvent::Key { scancode: 0x20, pressed: true } => {
                        scene.open_kind(AppKind::Display);
                        scene.start_open = false;
                        dirty = true;
                    }
                    InputEvent::MouseMove { dx, dy } => {
                        cur.x = (cur.x + dx as i32).clamp(0, W as i32 - 1);
                        cur.y = (cur.y - dy as i32).clamp(0, H as i32 - 1); // PS/2 +dy = up
                        let (hx, hy) = cur.hotspot();
                        match drag {
                            Drag::Quantum => {
                                scene.qapp.win.x = hx - drag_dx;
                                scene.qapp.win.y = hy - drag_dy;
                                scene.qapp.win.clamp_onscreen();
                                dirty = true;
                            }
                            Drag::App(i) => {
                                scene.apps[i].x = hx - drag_dx;
                                scene.apps[i].y = hy - drag_dy;
                                scene.apps[i].clamp_onscreen();
                                dirty = true;
                            }
                            Drag::None => {}
                        }
                        if scene.start_open {
                            let new_hover = start_menu().item_at(hx, hy);
                            if new_hover != hover {
                                hover = new_hover;
                                dirty = true;
                            }
                        }
                    }
                    InputEvent::MouseButton { button: MouseButton::Left, pressed } => {
                        let (hx, hy) = cur.hotspot();
                        if pressed {
                            // 1) Start menu has priority while open.
                            if scene.start_open {
                                if let Some(idx) = start_menu().item_at(hx, hy) {
                                    match MENU_ITEMS[idx] {
                                        "Quantum Lab" => {
                                            scene.qapp.win.open = true;
                                            scene.qapp.win.clamp_onscreen();
                                        }
                                        "Files" => scene.open_kind(AppKind::Files),
                                        "Task Monitor" => scene.open_kind(AppKind::SysMon),
                                        "Display" => scene.open_kind(AppKind::Display),
                                        "About" => scene.open_kind(AppKind::About),
                                        "Exit" => {
                                            crate::kthread::stop_background_worker();
                                            draw::leave();
                                            crate::serial_println!("[GFXUI] exited via Start>Exit");
                                            return;
                                        }
                                        _ => {}
                                    }
                                }
                                scene.start_open = false;
                                hover = None;
                                dirty = true;
                            }
                            // 2) Start button toggles the menu.
                            else if START_BTN.contains(hx, hy) {
                                scene.start_open = true;
                                start_pressed = true;
                                dirty = true;
                            }
                            // 3) Quantum app window (topmost).
                            else if scene.qapp.win.in_close(hx, hy) {
                                scene.qapp.win.open = false;
                                dirty = true;
                            } else if scene.qapp.win.open && scene.qapp.run_btn().contains(hx, hy) {
                                run_pressed = true;
                                scene.qapp.run();
                                dirty = true;
                            } else if scene.qapp.win.in_titlebar(hx, hy) {
                                drag = Drag::Quantum;
                                drag_dx = hx - scene.qapp.win.x;
                                drag_dy = hy - scene.qapp.win.y;
                            }
                            // 4) Simple app windows, topmost (last drawn) first.
                            else {
                                for i in (0..scene.apps.len()).rev() {
                                    if !scene.apps[i].open {
                                        continue;
                                    }
                                    if scene.apps[i].in_close(hx, hy) {
                                        scene.apps[i].open = false;
                                        dirty = true;
                                        break;
                                    }
                                    if scene.apps[i].in_titlebar(hx, hy) {
                                        drag = Drag::App(i);
                                        drag_dx = hx - scene.apps[i].x;
                                        drag_dy = hy - scene.apps[i].y;
                                        break;
                                    }
                                }
                            }
                        } else {
                            drag = Drag::None;
                            if start_pressed {
                                start_pressed = false;
                                dirty = true;
                            }
                            if run_pressed {
                                run_pressed = false;
                                dirty = true;
                            }
                        }
                    }
                    _ => {}
                }
            }

            if dirty {
                draw_scene(&scene, hover, start_pressed, run_pressed);
            }
            cur.save_bg();
            cur.draw();
            last_status = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
        } else {
            // No input: refresh the live taskbar (bg counter + clock) a few times a second
            // without a full repaint, so the desktop visibly "breathes".
            let now = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
            if now.wrapping_sub(last_status) >= 20 {
                last_status = now;
                cur.restore_bg();
                // Keep the Task Monitor live (its counters change); otherwise just the taskbar.
                if scene.apps[SYSMON_IDX].open {
                    draw_scene(&scene, hover, start_pressed, run_pressed);
                } else {
                    draw_status();
                }
                cur.save_bg();
                cur.draw();
            }
        }

        x86_64::instructions::hlt();
    }
}
