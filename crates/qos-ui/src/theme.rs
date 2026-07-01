//! Light and dark UI themes. A [`Theme`] is a flat palette the compositor and widgets read from,
//! so the whole desktop restyles by swapping one value. Colors are `0x00RRGGBB`.

use crate::color::{rgb, Rgb};

/// The full set of colors the modern desktop draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub is_dark: bool,
    /// Wallpaper vertical gradient (top → bottom).
    pub wallpaper_top: Rgb,
    pub wallpaper_bottom: Rgb,
    /// Translucent top menu bar and bottom dock base colors.
    pub bar: Rgb,
    pub dock: Rgb,
    /// Window/panel body and a slightly contrasting alternate (headers, rows).
    pub surface: Rgb,
    pub surface_alt: Rgb,
    /// Primary and dimmed text.
    pub text: Rgb,
    pub text_dim: Rgb,
    /// Accent (selection, focus, active dock item) and text drawn on the accent.
    pub accent: Rgb,
    pub on_accent: Rgb,
    /// Hairline borders/separators and the drop-shadow color.
    pub border: Rgb,
    pub shadow: Rgb,
}

impl Theme {
    /// A dark theme (deep blue-grey wallpaper, light text) — the default.
    pub const fn dark() -> Self {
        Theme {
            is_dark: true,
            wallpaper_top: rgb(0x1a, 0x1f, 0x2e),
            wallpaper_bottom: rgb(0x2b, 0x1d, 0x3a),
            bar: rgb(0x14, 0x16, 0x20),
            dock: rgb(0x22, 0x26, 0x33),
            surface: rgb(0x25, 0x2a, 0x38),
            surface_alt: rgb(0x2e, 0x34, 0x44),
            text: rgb(0xf0, 0xf2, 0xf6),
            text_dim: rgb(0x9a, 0xa2, 0xb2),
            accent: rgb(0x4c, 0x8d, 0xff),
            on_accent: rgb(0xff, 0xff, 0xff),
            border: rgb(0x3a, 0x41, 0x52),
            shadow: rgb(0x00, 0x00, 0x00),
        }
    }

    /// A light theme (soft grey wallpaper, dark text).
    pub const fn light() -> Self {
        Theme {
            is_dark: false,
            wallpaper_top: rgb(0xe9, 0xed, 0xf4),
            wallpaper_bottom: rgb(0xd7, 0xdd, 0xe8),
            bar: rgb(0xf4, 0xf6, 0xfa),
            dock: rgb(0xff, 0xff, 0xff),
            surface: rgb(0xff, 0xff, 0xff),
            surface_alt: rgb(0xf0, 0xf2, 0xf6),
            text: rgb(0x1a, 0x1e, 0x28),
            text_dim: rgb(0x6a, 0x72, 0x82),
            accent: rgb(0x2f, 0x6f, 0xed),
            on_accent: rgb(0xff, 0xff, 0xff),
            border: rgb(0xc8, 0xcf, 0xda),
            shadow: rgb(0x20, 0x28, 0x38),
        }
    }

    /// The opposite theme (for a light/dark toggle).
    pub const fn toggled(&self) -> Self {
        if self.is_dark {
            Self::light()
        } else {
            Self::dark()
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_flips_mode() {
        assert!(!Theme::dark().toggled().is_dark);
        assert!(Theme::light().toggled().is_dark);
    }

    #[test]
    fn dark_text_is_lighter_than_dark_background() {
        let t = Theme::dark();
        let (tr, _, _) = crate::color::channels(t.text);
        let (wr, _, _) = crate::color::channels(t.wallpaper_top);
        assert!(tr > wr, "dark theme text must contrast against the wallpaper");
    }
}
