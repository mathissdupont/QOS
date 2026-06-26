//! Graphical desktop (Phase 1 — see docs/PLAN.md).
//!
//! An interactive Mode 13h UI driven entirely by the unified input event queue (Phase 0.1):
//! a live mouse cursor with save-under, an event loop (draw → wait event → update), draggable
//! windows with clickable close buttons, generic reusable widgets (`Button`, `Menu`), a bottom
//! taskbar with a Start menu, and a launchable **Quantum Lab** app that runs a real Bell-state
//! circuit on the in-kernel statevector simulator and plots the measurement histogram.
//! ESC returns to the text shell.

use crate::input::{self, InputEvent, MouseButton};
use crate::quantum;
use crate::vga13h::{self, color};

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
                self.saved[row * CW + col] = vga13h::get_pixel(px, py);
            }
        }
    }

    fn restore_bg(&self) {
        for row in 0..CH {
            for col in 0..CW {
                let px = (self.x + col as i32).max(0) as usize;
                let py = (self.y + row as i32).max(0) as usize;
                vga13h::put_pixel(px, py, self.saved[row * CW + col]);
            }
        }
    }

    fn draw(&self) {
        for row in 0..CH {
            for col in 0..CW {
                let px = (self.x + col as i32).max(0) as usize;
                let py = (self.y + row as i32).max(0) as usize;
                match CURSOR[row][col] {
                    b'X' => vga13h::put_pixel(px, py, color::BLACK),
                    b'O' => vga13h::put_pixel(px, py, color::WHITE),
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
        vga13h::fill_rect(x, y, w, h, LTGRAY);
        let (tl, br) = if pressed { (DKGRAY, WHITE) } else { (WHITE, DKGRAY) };
        vga13h::fill_rect(x, y, w, 1, tl); // top
        vga13h::fill_rect(x, y, 1, h, tl); // left
        vga13h::fill_rect(x, y + h - 1, w, 1, br); // bottom
        vga13h::fill_rect(x + w - 1, y, 1, h, br); // right
        // Centered label, nudged 1px when pressed for a tactile feel.
        let tw = self.label.len() as i32 * 8;
        let off = if pressed { 1 } else { 0 };
        let tx = (self.x + (self.w - tw) / 2 + off).max(self.x + 1) as usize;
        let ty = (self.y + (self.h - 8) / 2 + off) as usize;
        vga13h::draw_string(tx, ty, self.label, BLACK, LTGRAY);
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
        vga13h::fill_rect(x, y, w, h, LTGRAY);
        vga13h::rect(x, y, w, h, DKGRAY);
        for (i, label) in self.items.iter().enumerate() {
            let iy = self.y + 1 + i as i32 * MENU_ITEM_H;
            let (bg, fg) = if hover == Some(i) { (BLUE, WHITE) } else { (LTGRAY, BLACK) };
            vga13h::fill_rect(self.x as usize + 1, iy as usize, w - 2, MENU_ITEM_H as usize, bg);
            vga13h::draw_string(self.x as usize + 5, iy as usize + 2, label, fg, bg);
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
        vga13h::fill_rect(x, y, w, h, LTGRAY);
        vga13h::rect(x, y, w, h, DKGRAY);
        vga13h::fill_rect(x + 2, y + 2, w - 4, Self::TITLE_H as usize, BLUE);
        vga13h::draw_string(x + 5, y + 4, self.title, WHITE, BLUE);
        let cb = self.close_btn();
        vga13h::fill_rect(cb.0 as usize, cb.1 as usize, 10, 10, RED);
        vga13h::draw_string(cb.0 as usize + 1, cb.1 as usize + 1, "x", WHITE, RED);
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
        vga13h::draw_string(bx, wire0 - 3, "q0", BLACK, LTGRAY);
        vga13h::draw_string(bx, wire1 - 3, "q1", BLACK, LTGRAY);
        vga13h::fill_rect(wstart, wire0, wend - wstart, 1, BLACK);
        vga13h::fill_rect(wstart, wire1, wend - wstart, 1, BLACK);

        // H gate box on q0.
        let hx = wstart + 16;
        vga13h::fill_rect(hx, wire0 - 5, 11, 11, WHITE);
        vga13h::rect(hx, wire0 - 5, 11, 11, BLACK);
        vga13h::draw_string(hx + 2, wire0 - 3, "H", BLACK, WHITE);

        // CNOT: control dot on q0, target (+) on q1, connected vertically.
        let cx = wstart + 56;
        vga13h::fill_rect(cx, wire0, 1, wire1 - wire0, BLACK); // vertical link
        vga13h::fill_rect(cx - 2, wire0 - 2, 5, 5, BLACK); // control dot
        vga13h::rect(cx - 4, wire1 - 4, 9, 9, BLACK); // target ring
        vga13h::fill_rect(cx, wire1 - 4, 1, 9, BLACK); // target plus (v)
        vga13h::fill_rect(cx - 4, wire1, 9, 1, BLACK); // target plus (h)

        // Histogram of the last run (or a hint to press Run).
        let chart_y = by + 48;
        match self.result {
            None => {
                vga13h::draw_string(bx, chart_y, "Press Run to measure", DKGRAY, LTGRAY);
                vga13h::draw_string(bx, chart_y + 10, "the Bell state.", DKGRAY, LTGRAY);
            }
            Some((zeros, ones, shots)) => {
                let max_bar = 48i32;
                let p00 = (zeros * 100 / shots) as i32;
                let p11 = (ones * 100 / shots) as i32;
                let b00 = (zeros as i32 * max_bar / shots as i32).max(1);
                let b11 = (ones as i32 * max_bar / shots as i32).max(1);
                let base = chart_y + max_bar as usize;
                // |00> bar
                vga13h::fill_rect(bx + 8, base - b00 as usize, 18, b00 as usize, GREEN);
                vga13h::draw_string(bx + 6, base + 2, "00", BLACK, LTGRAY);
                draw_pct(bx + 4, chart_y - 2, p00);
                // |11> bar
                vga13h::fill_rect(bx + 48, base - b11 as usize, 18, b11 as usize, LTBLUE);
                vga13h::draw_string(bx + 46, base + 2, "11", BLACK, LTGRAY);
                draw_pct(bx + 44, chart_y - 2, p11);
                vga13h::draw_string(bx + 80, chart_y + 8, "Bell state:", BLACK, LTGRAY);
                vga13h::draw_string(bx + 80, chart_y + 18, "|00>+|11>", BLACK, LTGRAY);
                vga13h::draw_string(bx + 80, chart_y + 30, "01,10 ~ 0%", DKGRAY, LTGRAY);
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
    vga13h::draw_string(x, y, text, color::BLACK, color::LTGRAY);
}

// ── Welcome window ──────────────────────────────────────────────────────────────────────

fn draw_welcome(win: &Window) {
    use color::*;
    if !win.open {
        return;
    }
    win.draw_frame();
    let (bx, by) = win.body_origin();
    vga13h::draw_string(bx as usize, by as usize, "Welcome to QOS.", BLACK, LTGRAY);
    vga13h::draw_string(bx as usize, by as usize + 12, "Click Start, or press", BLACK, LTGRAY);
    vga13h::draw_string(bx as usize, by as usize + 24, "Q for Quantum Lab.", BLACK, LTGRAY);
    vga13h::draw_string(bx as usize, by as usize + 40, "Drag the title bar.", DKGRAY, LTGRAY);
    vga13h::draw_string(bx as usize, by as usize + 52, "[x] closes a window.", DKGRAY, LTGRAY);
}

// ── Desktop composition ─────────────────────────────────────────────────────────────────

const START_BTN: Button = Button { x: 2, y: (H as i32) - 12, w: 50, h: 10, label: "Start" };

const MENU_ITEMS: [&str; 3] = ["Quantum Lab", "Welcome", "Exit"];

fn start_menu() -> Menu {
    let items: &'static [&'static str] = &MENU_ITEMS;
    let m_w = 96;
    let m = Menu { x: 2, y: 0, w: m_w, items };
    Menu { y: H as i32 - TASKBAR_H - m.height(), ..m }
}

struct Scene {
    welcome: Window,
    qapp: QuantumApp,
    start_open: bool,
}

fn draw_scene(scene: &Scene, hover: Option<usize>, start_pressed: bool, run_pressed: bool) {
    use color::*;
    vga13h::clear(TEAL);
    // Top status bar.
    vga13h::fill_rect(0, 0, W, TOPBAR_H as usize, BLUE);
    vga13h::draw_string(2, 2, "QOS Desktop", WHITE, BLUE);
    vga13h::draw_string(W - 17 * 8, 2, "ESC: exit to shell", WHITE, BLUE);

    // Windows (welcome below, quantum app on top).
    draw_welcome(&scene.welcome);
    scene.qapp.draw(run_pressed);

    // Taskbar.
    let ty = H - TASKBAR_H as usize;
    vga13h::fill_rect(0, ty, W, TASKBAR_H as usize, LTGRAY);
    vga13h::fill_rect(0, ty, W, 1, WHITE);
    START_BTN.draw(start_pressed || scene.start_open);
    if scene.qapp.win.open {
        vga13h::draw_string(58, ty + 3, "Quantum Lab", BLACK, LTGRAY);
    }

    // Start menu popup (drawn last so it's on top of everything).
    if scene.start_open {
        start_menu().draw(hover);
    }
}

/// Run the interactive desktop until ESC. Returns to text mode on exit.
pub fn run() {
    crate::serial_println!("[GFXUI] entering interactive desktop");
    vga13h::enter();

    let mut scene = Scene {
        welcome: Window { x: 30, y: 90, w: 168, h: 82, title: "Welcome", open: true },
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
        Welcome,
        Quantum,
    }
    let mut drag = Drag::None;
    let mut drag_dx = 0i32;
    let mut drag_dy = 0i32;

    draw_scene(&scene, hover, start_pressed, run_pressed);
    cur.save_bg();
    cur.draw();

    loop {
        if input::has_events() {
            cur.restore_bg();
            let mut dirty = false;

            while let Some(ev) = input::poll() {
                match ev {
                    InputEvent::Key { scancode: 0x01, pressed: true } => {
                        vga13h::leave();
                        crate::serial_println!("[GFXUI] exited to text mode");
                        return;
                    }
                    // Keyboard shortcuts (also makes the desktop usable without a mouse):
                    // Q = open Quantum Lab, R = run it, W = open Welcome.
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
                        scene.welcome.open = true;
                        scene.start_open = false;
                        dirty = true;
                    }
                    InputEvent::MouseMove { dx, dy } => {
                        cur.x = (cur.x + dx as i32).clamp(0, W as i32 - 1);
                        cur.y = (cur.y - dy as i32).clamp(0, H as i32 - 1); // PS/2 +dy = up
                        let (hx, hy) = cur.hotspot();
                        match drag {
                            Drag::Welcome => {
                                scene.welcome.x = hx - drag_dx;
                                scene.welcome.y = hy - drag_dy;
                                scene.welcome.clamp_onscreen();
                                dirty = true;
                            }
                            Drag::Quantum => {
                                scene.qapp.win.x = hx - drag_dx;
                                scene.qapp.win.y = hy - drag_dy;
                                scene.qapp.win.clamp_onscreen();
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
                                        "Welcome" => scene.welcome.open = true,
                                        "Exit" => {
                                            vga13h::leave();
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
                            // 4) Welcome window.
                            else if scene.welcome.in_close(hx, hy) {
                                scene.welcome.open = false;
                                dirty = true;
                            } else if scene.welcome.in_titlebar(hx, hy) {
                                drag = Drag::Welcome;
                                drag_dx = hx - scene.welcome.x;
                                drag_dy = hy - scene.welcome.y;
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
        }

        x86_64::instructions::hlt();
    }
}
