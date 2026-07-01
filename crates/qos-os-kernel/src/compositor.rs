//! Modern desktop compositor (WP-05 / E-70) — kernel seam over the portable `qos-ui` core.
//!
//! Allocates a true-color back [`Surface`] at the framebuffer's **native** resolution, composes a
//! modern themed scene (gradient wallpaper, translucent top bar + dock, rounded windows with soft
//! drop shadows), and blits it to the UEFI framebuffer. This is the foundation the boot splash
//! (step 2), TrueType text (step 3), widgets/WM (step 4) and apps (step 5) build on.
//!
//! Opt-in for now via the `modern` shell command (fallback-first, ADR-0015): it does not replace
//! the legacy desktop until the toolkit is ready.

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

/// An open window on the desktop.
struct Win {
    rect: Rect,
    kind: AppKind,
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
}

impl Desktop {
    fn new(w: i32, h: i32) -> Self {
        // Start with two cascaded windows so the desktop looks alive.
        let wins = vec![
            Win { rect: Rect::new(w / 2 - 430, 96, 520, 360), kind: AppKind::Terminal },
            Win { rect: Rect::new(w / 2 - 20, 250, 500, 320), kind: AppKind::Files },
        ];
        Desktop { w, h, theme: Theme::dark(), wins, cursor: (w / 2, h / 2), drag: None, dirty: true }
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
            let rect = Rect::new((self.w / 2 - 260 + n * 28).max(20), (110 + n * 28).min(self.h - 360), 500, 320);
            self.wins.push(Win { rect, kind });
        }
        self.dirty = true;
    }

    // ---- input ----
    fn on_mouse_move(&mut self, dx: i16, dy: i16) {
        self.cursor.0 = (self.cursor.0 + dx as i32).clamp(0, self.w - 1);
        // InputEvent dy is +up (PS/2 convention); screen y grows downward.
        self.cursor.1 = (self.cursor.1 - dy as i32).clamp(0, self.h - 1);
        if let Some((idx, ox, oy)) = self.drag {
            if idx < self.wins.len() {
                let nx = self.cursor.0 - ox;
                let ny = (self.cursor.1 - oy).max(BAR_H);
                self.wins[idx].rect.x = nx;
                self.wins[idx].rect.y = ny;
            }
        }
        self.dirty = true;
    }

    fn on_left_down(&mut self) {
        let (cx, cy) = self.cursor;
        // Top menu bar: theme toggle.
        if self.theme_btn().contains(cx, cy) {
            self.theme = self.theme.toggled();
            self.dirty = true;
            return;
        }
        // Windows, top-most first.
        for i in (0..self.wins.len()).rev() {
            let r = self.wins[i].rect;
            let (dxc, dyc) = self.close_dot(&r);
            if (cx - dxc).abs() <= 9 && (cy - dyc).abs() <= 9 {
                self.wins.remove(i); // close
                self.dirty = true;
                return;
            }
            if r.contains(cx, cy) {
                // Raise; if on the header, begin dragging.
                let win = self.wins.remove(i);
                let on_header = cy < win.rect.y + HEADER_H;
                let off = (cx - win.rect.x, cy - win.rect.y);
                self.wins.push(win);
                if on_header {
                    self.drag = Some((self.wins.len() - 1, off.0, off.1));
                }
                self.dirty = true;
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
            self.dirty = true;
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
                s.rounded_rect(Rect::new(r.x + 12, r.y + HEADER_H + 10, r.w - 24, r.h - HEADER_H - 22), 8, qos_ui::rgb(0x10, 0x12, 0x18));
                fr.draw_text(s, bx, by + 8, "qos:\\> run bell.qasm", 15.0, qos_ui::rgb(0x6e, 0xe0, 0x7a));
                fr.draw_text(s, bx, by + 34, "measuring 2 qubits...", 15.0, theme.text_dim);
                fr.draw_text(s, bx, by + 60, "00 -> 512   11 -> 512", 15.0, theme.text);
                fr.draw_text(s, bx, by + 90, "qos:\\> _", 15.0, qos_ui::rgb(0x6e, 0xe0, 0x7a));
            }
            AppKind::Files => {
                for (j, name) in ["Documents", "quantum", "readme.txt", "bell.qasm"].iter().enumerate() {
                    let ry = by + j as i32 * 34;
                    s.rounded_rect(Rect::new(bx, ry, r.w - 44, 26), 6, theme.surface_alt);
                    fr.draw_text(s, bx + 12, ry + 19, name, 15.0, theme.text);
                }
            }
            AppKind::Quantum => {
                fr.draw_text(s, bx, by + 8, "Circuit: Bell state", 16.0, theme.text);
                for (j, line) in ["q0 : |0> --[H]--*--", "q1 : |0> -------X--"].iter().enumerate() {
                    fr.draw_text(s, bx, by + 44 + j as i32 * 28, line, 15.0, theme.text_dim);
                }
                let btn = Rect::new(bx, by + 120, 130, 38);
                s.rounded_rect(btn, 9, theme.accent);
                let bw = fr.text_width("Run", 16.0);
                fr.draw_text(s, btn.x + (btn.w - bw) / 2, btn.y + 25, "Run", 16.0, theme.on_accent);
            }
            AppKind::Settings => {
                fr.draw_text(s, bx, by + 8, "Appearance", 16.0, theme.text);
                let label = if theme.is_dark { "Theme:  Dark" } else { "Theme:  Light" };
                fr.draw_text(s, bx, by + 44, label, 15.0, theme.text_dim);
                fr.draw_text(s, bx, by + 96, "System", 16.0, theme.text);
                fr.draw_text(s, bx, by + 132, "QOS 0.1  -  1280x800  -  USB kbd+mouse", 14.0, theme.text_dim);
            }
        }
    }

    fn draw_cursor(&self, s: &mut Surface) {
        let (cx, cy) = self.cursor;
        for (row, line) in CURSOR.iter().enumerate() {
            for (col, ch) in line.bytes().enumerate() {
                let color = match ch {
                    b'#' => qos_ui::rgb(0x10, 0x12, 0x18),
                    b'o' => qos_ui::rgb(0xff, 0xff, 0xff),
                    _ => continue,
                };
                s.put(cx + col as i32, cy + row as i32, color);
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

        self.draw_cursor(s);
    }
}

/// Render one frame of the boot splash for animation parameter `e` in `0..=DURATION` ticks.
fn splash_frame(s: &mut Surface, theme: &Theme, e: i32, duration: i32) {
    let (w, h) = (s.width as i32, s.height as i32);
    // Phases (in ticks): fade+scale in, hold, fade out.
    let in_end = duration * 33 / 100;
    let hold_end = duration * 73 / 100;
    let alpha: i32 = if e < in_end {
        e * 255 / in_end
    } else if e < hold_end {
        255
    } else {
        255 - (e - hold_end) * 255 / (duration - hold_end).max(1)
    };
    let alpha = alpha.clamp(0, 255) as u8;
    // Scale from 82% to 100% during the fade-in (ease toward full), then steady.
    let scale_pct = if e < in_end { 82 + 18 * e / in_end } else { 100 };

    // Background: the dark theme wallpaper gradient.
    s.gradient_v(Rect::new(0, 0, w, h), theme.wallpaper_top, theme.wallpaper_bottom);

    // Centered logo, tinted near-white, growing + fading in.
    let base = (h * 44 / 100).min(w * 44 / 100); // fit comfortably on screen
    let size = base * scale_pct / 100;
    let dst = Rect::new((w - size) / 2, (h - size) / 2 - h / 20, size, size);
    s.blit_mask_scaled(LOGO_MASK, LOGO_W, LOGO_H, dst, theme.text, alpha);

    // A slim accent loading bar near the bottom that fills with progress.
    let bar_w = w * 22 / 100;
    let bar_h = 6;
    let bx = (w - bar_w) / 2;
    let by = h * 82 / 100;
    s.rounded_rect(Rect::new(bx, by, bar_w, bar_h), 3, theme.surface_alt);
    let fill = (bar_w * e / duration).clamp(0, bar_w);
    if fill > 0 {
        s.rounded_rect(Rect::new(bx, by, fill, bar_h), 3, theme.accent);
    }
}

/// Play the branded animated boot splash (WP-05 step 2): the Heptapus logo fades in, grows, holds,
/// then fades out over ~1.5 s, with a loading bar. Runs after init (heap/timer/framebuffer ready),
/// before the shell. A keypress skips it. No-op without a linear framebuffer.
pub fn run_splash() {
    let info = match crate::framebuffer::info() {
        Some(i) => i,
        None => return,
    };
    let (w, h) = (info.width, info.height);
    let theme = Theme::dark();
    let mut surface = Surface::new(w, h);
    crate::serial_println!("[UI] boot splash: {}x{} Heptapus animation", w, h);

    const DURATION: i32 = 150; // ~1.5 s at the 100 Hz APIC tick
    // Drain any input queued during boot so a stale event doesn't instantly "skip" the splash;
    // only a key pressed *during* the splash should skip it.
    while crate::input::poll().is_some() {}
    let start = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64;
    let mut last_e = -1;
    loop {
        let now = crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed) as i64;
        let e = (now - start) as i32;
        if e >= DURATION {
            break;
        }
        // Skip on a key pressed during the splash.
        if let Some(crate::input::InputEvent::Key { pressed: true, .. }) = crate::input::poll() {
            break;
        }
        if e != last_e {
            splash_frame(&mut surface, &theme, e, DURATION);
            crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);
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
    let mut last_sec = -1;

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
                    last_sec = -1; // force redraw so the cursor moves
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
            // Redraw the chooser (background, logo, title, two cards, countdown, cursor).
            surface.gradient_v(Rect::new(0, 0, wi, hi), theme.wallpaper_top, theme.wallpaper_bottom);
            let logo = 150;
            surface.blit_mask_scaled(LOGO_MASK, LOGO_W, LOGO_H, Rect::new(wi / 2 - logo / 2, hi / 6, logo, logo), theme.text, 255);
            let title = "Welcome to QOS";
            let tw = fr.text_width(title, 34.0);
            fr.draw_text(&mut surface, wi / 2 - tw / 2, hi / 6 + logo + 40, title, 34.0, theme.text);
            let sub = "Choose how to start";
            let sw = fr.text_width(sub, 17.0);
            fr.draw_text(&mut surface, wi / 2 - sw / 2, hi / 6 + logo + 72, sub, 17.0, theme.text_dim);

            // Desktop card (accent) + Shell card (surface).
            surface.drop_shadow(desktop_card, 16, 18, theme.shadow, 130);
            surface.rounded_rect(desktop_card, 16, theme.accent);
            let d1 = "Modern Desktop";
            let d1w = fr.text_width(d1, 22.0);
            fr.draw_text(&mut surface, desktop_card.x + (card_w - d1w) / 2, desktop_card.y + 88, d1, 22.0, theme.on_accent);
            let d2 = "Enter  /  D";
            let d2w = fr.text_width(d2, 15.0);
            fr.draw_text(&mut surface, desktop_card.x + (card_w - d2w) / 2, desktop_card.y + 130, d2, 15.0, theme.on_accent);

            surface.drop_shadow(shell_card, 16, 18, theme.shadow, 100);
            surface.rounded_rect(shell_card, 16, theme.surface);
            let s1 = "Terminal";
            let s1w = fr.text_width(s1, 22.0);
            fr.draw_text(&mut surface, shell_card.x + (card_w - s1w) / 2, shell_card.y + 88, s1, 22.0, theme.text);
            let s2 = "S";
            let s2w = fr.text_width(s2, 15.0);
            fr.draw_text(&mut surface, shell_card.x + (card_w - s2w) / 2, shell_card.y + 130, s2, 15.0, theme.text_dim);

            // Countdown hint.
            let mut buf = [0u8; 48];
            let hint = fmt_countdown(&mut buf, sec.max(0));
            let hw = fr.text_width(hint, 14.0);
            fr.draw_text(&mut surface, wi / 2 - hw / 2, shell_card.bottom() + 44, hint, 14.0, theme.text_dim);

            // Cursor.
            for (row, line) in CURSOR.iter().enumerate() {
                for (col, ch) in line.bytes().enumerate() {
                    let color = match ch {
                        b'#' => qos_ui::rgb(0x10, 0x12, 0x18),
                        b'o' => qos_ui::rgb(0xff, 0xff, 0xff),
                        _ => continue,
                    };
                    surface.put(cursor.0 + col as i32, cursor.1 + row as i32, color);
                }
            }
            crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);
            last_sec = sec;
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

    loop {
        // Pump USB HID here too: `run_demo` runs synchronously (it blocks the scheduler loop that
        // normally queues the interrupt-IN report TRB), so drive it directly to keep USB keyboard +
        // mouse alive on the desktop. Cheap (try_lock) and harmless alongside the scheduler.
        crate::xhci::poll();
        while let Some(ev) = crate::input::poll() {
            match ev {
                InputEvent::Key { scancode, pressed: true } => match scancode {
                    0x01 => {
                        crate::framebuffer::clear(0x000000);
                        crate::framebuffer::reset_cursor();
                        return; // Esc → shell
                    }
                    0x14 => {
                        desk.theme = desk.theme.toggled();
                        desk.dirty = true;
                    } // t
                    0x02 => desk.open_app(AppKind::Terminal), // 1
                    0x03 => desk.open_app(AppKind::Files),    // 2
                    0x04 => desk.open_app(AppKind::Quantum),  // 3
                    0x05 => desk.open_app(AppKind::Settings), // 4
                    0x11 => {
                        if desk.wins.pop().is_some() {
                            desk.dirty = true;
                        }
                    } // w → close focused
                    _ => {}
                },
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
            desk.compose(&mut surface, &mut fr);
            crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);
            desk.dirty = false;
        }
        crate::arch::hlt();
    }
}
