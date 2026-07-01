//! Modern desktop compositor (WP-05 / E-70) — kernel seam over the portable `qos-ui` core.
//!
//! Allocates a true-color back [`Surface`] at the framebuffer's **native** resolution, composes a
//! modern themed scene (gradient wallpaper, translucent top bar + dock, rounded windows with soft
//! drop shadows), and blits it to the UEFI framebuffer. This is the foundation the boot splash
//! (step 2), TrueType text (step 3), widgets/WM (step 4) and apps (step 5) build on.
//!
//! Opt-in for now via the `modern` shell command (fallback-first, ADR-0015): it does not replace
//! the legacy desktop until the toolkit is ready.

use qos_ui::{Rect, Surface, Theme};

/// A filled circle via a maximally-rounded square (used for the macOS-style window dots + dock).
fn circle(s: &mut Surface, cx: i32, cy: i32, d: i32, color: qos_ui::Rgb) {
    s.rounded_rect(Rect::new(cx - d / 2, cy - d / 2, d, d), d / 2, color);
}

/// Draw one macOS/GNOME-hybrid window: soft drop shadow, rounded body, a header strip with the
/// three traffic-light dots, and an accent button in the body.
fn draw_window(s: &mut Surface, theme: &Theme, r: Rect) {
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
    // A primary accent button in the body.
    let btn = Rect::new(r.x + 24, r.y + header_h + 28, 150, 40);
    s.rounded_rect(btn, 10, theme.accent);
    // A couple of "content" rows to suggest a list.
    for i in 0..3 {
        let row = Rect::new(r.x + 24, r.y + header_h + 92 + i * 30, r.w - 48, 18);
        s.rounded_rect(row, 6, theme.surface_alt);
    }
}

/// Compose the full modern desktop scene into `s` for the given `theme`.
fn compose(s: &mut Surface, theme: &Theme) {
    let (w, h) = (s.width as i32, s.height as i32);

    // Wallpaper: vertical gradient across the whole screen.
    s.gradient_v(Rect::new(0, 0, w, h), theme.wallpaper_top, theme.wallpaper_bottom);

    // Top menu bar: translucent strip.
    let bar_h = 30;
    s.blend_rect(Rect::new(0, 0, w, bar_h), theme.bar, 210);
    // Logo spot (accent rounded square) on the left + a "clock" pill on the right.
    s.rounded_rect(Rect::new(12, 7, 16, 16), 5, theme.accent);
    s.rounded_rect(Rect::new(w - 92, 6, 80, 18), 9, theme.surface_alt);

    // Two overlapping windows (shows z-order + shadows).
    draw_window(s, theme, Rect::new(w / 2 - 440, 90, 520, 360));
    draw_window(s, theme, Rect::new(w / 2 - 40, 240, 500, 330));

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

    crate::serial_println!("[UI] modern compositor: {}x{} surface, native true-color", w, h);
    compose(&mut surface, &theme);
    crate::framebuffer::blit_region(&surface.pixels, w, 0, 0, w, h);

    loop {
        if let Some(ev) = crate::input::poll() {
            if let crate::input::InputEvent::Key { scancode, pressed: true } = ev {
                match scancode {
                    0x01 => break,       // Esc → back to shell
                    0x14 => {            // 't' → toggle theme
                        theme = theme.toggled();
                        compose(&mut surface, &theme);
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
