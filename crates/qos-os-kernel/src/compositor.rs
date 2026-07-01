//! Modern desktop compositor (WP-05 / E-70) — kernel seam over the portable `qos-ui` core.
//!
//! Allocates a true-color back [`Surface`] at the framebuffer's **native** resolution, composes a
//! modern themed scene (gradient wallpaper, translucent top bar + dock, rounded windows with soft
//! drop shadows), and blits it to the UEFI framebuffer. This is the foundation the boot splash
//! (step 2), TrueType text (step 3), widgets/WM (step 4) and apps (step 5) build on.
//!
//! Opt-in for now via the `modern` shell command (fallback-first, ADR-0015): it does not replace
//! the legacy desktop until the toolkit is ready.

use qos_ui::font::{Font, FontRenderer};
use qos_ui::{Rect, Surface, Theme};

/// Heptapus boot-splash logo coverage mask (WP-05 step 2): the octopus + "HEPTAPUS GROUP" shape,
/// generated from `heptapus_logo_primary_black.png`'s alpha by `scripts/gen_logo_mask.py`. One byte
/// per pixel; tinted per theme at draw time.
static LOGO_MASK: &[u8] = include_bytes!("assets/heptapus_logo_mask.bin");
const LOGO_W: usize = 400;
const LOGO_H: usize = 400;

/// A filled circle via a maximally-rounded square (used for the macOS-style window dots + dock).
fn circle(s: &mut Surface, cx: i32, cy: i32, d: i32, color: qos_ui::Rgb) {
    s.rounded_rect(Rect::new(cx - d / 2, cy - d / 2, d, d), d / 2, color);
}

/// Draw one macOS/GNOME-hybrid window: soft drop shadow, rounded body, a header strip with the
/// three traffic-light dots, a centered title, and an accent button with a label.
fn draw_window(s: &mut Surface, fr: &mut FontRenderer, theme: &Theme, r: Rect, title: &str, button: &str) {
    let radius = 14;
    // Soft drop shadow, offset slightly down for depth.
    s.drop_shadow(Rect::new(r.x, r.y + 6, r.w, r.h), radius, 22, theme.shadow, if theme.is_dark { 150 } else { 90 });
    // Window body.
    s.rounded_rect(r, radius, theme.surface);
    // Header strip (same rounded top; the body underneath keeps the bottom square).
    let header_h = 42;
    s.rounded_rect(Rect::new(r.x, r.y, r.w, header_h), radius, theme.surface_alt);
    s.fill_rect(Rect::new(r.x, r.y + radius, r.w, header_h - radius), theme.surface_alt);
    // Hairline under the header.
    s.fill_rect(Rect::new(r.x, r.y + header_h, r.w, 1), theme.border);
    // Traffic-light dots.
    let cy = r.y + header_h / 2;
    circle(s, r.x + 22, cy, 14, qos_ui::rgb(0xff, 0x5f, 0x57)); // close (red)
    circle(s, r.x + 44, cy, 14, qos_ui::rgb(0xfe, 0xbc, 0x2e)); // minimize (yellow)
    circle(s, r.x + 66, cy, 14, qos_ui::rgb(0x28, 0xc8, 0x40)); // maximize (green)
    // Centered window title.
    let tsize = 18.0;
    let tw = fr.text_width(title, tsize);
    fr.draw_text(s, r.x + (r.w - tw) / 2, cy + 6, title, tsize, theme.text);
    // A primary accent button with a centered label.
    let btn = Rect::new(r.x + 24, r.y + header_h + 28, 150, 40);
    s.rounded_rect(btn, 10, theme.accent);
    let bw = fr.text_width(button, 16.0);
    fr.draw_text(s, btn.x + (btn.w - bw) / 2, btn.y + 26, button, 16.0, theme.on_accent);
    // A couple of "content" rows to suggest a list.
    for i in 0..3 {
        let row = Rect::new(r.x + 24, r.y + header_h + 92 + i * 30, r.w - 48, 18);
        s.rounded_rect(row, 6, theme.surface_alt);
    }
}

/// Compose the full modern desktop scene into `s` for the given `theme`.
fn compose(s: &mut Surface, fr: &mut FontRenderer, theme: &Theme) {
    let (w, h) = (s.width as i32, s.height as i32);

    // Wallpaper: vertical gradient across the whole screen.
    s.gradient_v(Rect::new(0, 0, w, h), theme.wallpaper_top, theme.wallpaper_bottom);

    // Top menu bar: translucent strip.
    let bar_h = 30;
    s.blend_rect(Rect::new(0, 0, w, bar_h), theme.bar, 210);
    // Logo spot + name on the left, menu items, and a clock on the right — real antialiased text.
    s.rounded_rect(Rect::new(12, 7, 16, 16), 5, theme.accent);
    fr.draw_text(s, 36, 21, "QOS", 16.0, theme.text);
    let mut mx = 84;
    for item in ["File", "Edit", "View", "Window", "Help"] {
        mx = fr.draw_text(s, mx, 21, item, 15.0, theme.text_dim) + 20;
    }
    let clock = "12:42";
    let cw = fr.text_width(clock, 15.0);
    fr.draw_text(s, w - cw - 16, 21, clock, 15.0, theme.text);

    // Two overlapping windows (shows z-order + shadows + titles).
    draw_window(s, fr, theme, Rect::new(w / 2 - 440, 90, 520, 360), "Terminal", "New");
    draw_window(s, fr, theme, Rect::new(w / 2 - 40, 240, 500, 330), "Files", "Open");

    // Dock: centered translucent rounded panel with icon tiles.
    let dock_w = 460;
    let dock_h = 66;
    let dock = Rect::new(w / 2 - dock_w / 2, h - dock_h - 14, dock_w, dock_h);
    s.drop_shadow(dock, 20, 18, theme.shadow, if theme.is_dark { 140 } else { 70 });
    s.rounded_rect_blend(dock, 20, theme.dock, 235);
    let icon = 46;
    let gap = 16;
    let count = 6;
    let total = count * icon + (count - 1) * gap;
    let mut ix = dock.x + (dock_w - total) / 2;
    let iy = dock.y + (dock_h - icon) / 2;
    let tints = [
        theme.accent,
        qos_ui::rgb(0x30, 0xb0, 0x60),
        qos_ui::rgb(0xe0, 0x7a, 0x2a),
        qos_ui::rgb(0x8a, 0x5c, 0xd8),
        qos_ui::rgb(0x27, 0xa8, 0xc8),
        theme.surface_alt,
    ];
    for t in tints.iter() {
        s.rounded_rect(Rect::new(ix, iy, icon, icon), 12, *t);
        ix += icon + gap;
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

/// Run the modern-desktop demo (opt-in via the `modern` shell command). Composes the scene, blits
/// it, and loops: `t` toggles light/dark, `Esc` returns to the shell. Proves the compositor renders
/// at native resolution before the widget toolkit lands.
pub fn run_demo() {
    let info = match crate::framebuffer::info() {
        Some(i) => i,
        None => {
            crate::println!("modern: no linear framebuffer (UEFI only)");
            return;
        }
    };
    let (w, h) = (info.width, info.height);
    let mut theme = Theme::dark();
    let mut surface = Surface::new(w, h);
    let mut fr = match Font::parse(qos_ui::font::DEFAULT_FONT) {
        Some(f) => FontRenderer::new(f),
        None => {
            crate::println!("modern: font parse failed");
            return;
        }
    };

    crate::serial_println!("[UI] modern compositor: {}x{} surface, native true-color", w, h);
    compose(&mut surface, &mut fr, &theme);
    crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);

    loop {
        if let Some(ev) = crate::input::poll() {
            if let crate::input::InputEvent::Key { scancode, pressed: true } = ev {
                match scancode {
                    0x01 => break,       // Esc → back to shell
                    0x14 => {            // 't' → toggle theme
                        theme = theme.toggled();
                        compose(&mut surface, &mut fr, &theme);
                        crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);
                    }
                    _ => {}
                }
            }
        }
        crate::arch::hlt();
    }
    // Leave the framebuffer text console in a clean state for the shell.
    crate::framebuffer::clear(0x000000);
    crate::framebuffer::reset_cursor();
}
