//! # qos-ui
//!
//! Portable core of the QOS **modern UI** (ADR-0017 / WP-05): a true-color compositor surface with
//! antialiased drawing primitives, damage tracking, and light/dark themes. It is the resolution-
//! independent, allocator-using heart of the desktop — the kernel supplies only a thin blit from a
//! [`Surface`] to the real framebuffer.
//!
//! Like the other portable QOS crates it is `no_std` in the kernel (`#![cfg_attr(not(test),
//! no_std)]`) but uses `alloc` for the pixel buffers, and all of its logic is unit-tested on the
//! host. Colors are packed `0x00RRGGBB`, matching `qos-gfx` and the framebuffer backend.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod color;
pub mod dirty;
pub mod geometry;
pub mod surface;
pub mod theme;

pub use color::{blend, channels, lerp, rgb, scale_brightness, Rgb};
pub use dirty::DirtyTracker;
pub use geometry::Rect;
pub use surface::Surface;
pub use theme::Theme;
