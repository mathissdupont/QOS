//! Damage tracking. The compositor only needs to blit what changed each frame. This keeps a single
//! bounding rectangle over all changes since the last [`take`](DirtyTracker::take) — simple and
//! cheap, and enough to skip full-screen blits when only a cursor or one widget moved. (A tile list
//! can replace this later without changing callers.)

use crate::geometry::Rect;

/// Accumulates changed regions into one bounding rectangle.
#[derive(Clone, Copy, Debug, Default)]
pub struct DirtyTracker {
    bounds: Rect,
    dirty: bool,
}

impl DirtyTracker {
    pub const fn new() -> Self {
        DirtyTracker { bounds: Rect::new(0, 0, 0, 0), dirty: false }
    }

    /// Mark a rectangle as changed (empty rectangles are ignored).
    pub fn mark(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        if self.dirty {
            self.bounds = self.bounds.union(&r);
        } else {
            self.bounds = r;
            self.dirty = true;
        }
    }

    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Take the accumulated damage rectangle (clamped to `clip`) and reset to clean. Returns `None`
    /// if nothing changed or the damage fell entirely outside `clip`.
    pub fn take(&mut self, clip: Rect) -> Option<Rect> {
        if !self.dirty {
            return None;
        }
        let out = self.bounds.intersect(&clip);
        self.bounds = Rect::new(0, 0, 0, 0);
        self.dirty = false;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_bounding_box() {
        let mut d = DirtyTracker::new();
        assert!(!d.is_dirty());
        d.mark(Rect::new(10, 10, 5, 5));
        d.mark(Rect::new(20, 20, 5, 5));
        assert!(d.is_dirty());
        let screen = Rect::new(0, 0, 100, 100);
        assert_eq!(d.take(screen), Some(Rect::new(10, 10, 15, 15)));
        // taking resets
        assert!(!d.is_dirty());
        assert_eq!(d.take(screen), None);
    }

    #[test]
    fn clamps_to_clip() {
        let mut d = DirtyTracker::new();
        d.mark(Rect::new(-10, -10, 20, 20));
        assert_eq!(d.take(Rect::new(0, 0, 50, 50)), Some(Rect::new(0, 0, 10, 10)));
    }

    #[test]
    fn empty_marks_ignored() {
        let mut d = DirtyTracker::new();
        d.mark(Rect::new(0, 0, 0, 0));
        assert!(!d.is_dirty());
    }
}
