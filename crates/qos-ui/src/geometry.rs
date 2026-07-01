//! Integer rectangle geometry for the compositor. All coordinates are `i32` so off-screen and
//! negative positions (windows dragged partly off the edge) are representable; clipping happens
//! where pixels are actually written.

/// An axis-aligned rectangle at `(x, y)` with size `w × h`. Zero or negative size = empty.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    #[inline]
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    #[inline]
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// True if `(px, py)` lies inside the rectangle (right/bottom edges exclusive).
    #[inline]
    pub const fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    /// The overlap of two rectangles, or `None` if they do not intersect.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 > x0 && y1 > y0 {
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        } else {
            None
        }
    }

    /// The smallest rectangle covering both (empty rectangles are ignored).
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self.right().max(other.right());
        let y1 = self.bottom().max(other.bottom());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Grow (or shrink, for negative `d`) the rectangle by `d` on every side.
    pub fn inflate(&self, d: i32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2 * d, self.h + 2 * d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_edges() {
        let r = Rect::new(10, 10, 20, 10);
        assert!(r.contains(10, 10));
        assert!(r.contains(29, 19));
        assert!(!r.contains(30, 10)); // right edge exclusive
        assert!(!r.contains(10, 20)); // bottom edge exclusive
        assert!(!r.contains(9, 10));
    }

    #[test]
    fn intersect_overlap_and_disjoint() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Some(Rect::new(5, 5, 5, 5)));
        let c = Rect::new(100, 100, 5, 5);
        assert_eq!(a.intersect(&c), None);
        // touching edges do not intersect (exclusive)
        let d = Rect::new(10, 0, 5, 10);
        assert_eq!(a.intersect(&d), None);
    }

    #[test]
    fn union_ignores_empty() {
        let a = Rect::new(0, 0, 10, 10);
        let empty = Rect::new(5, 5, 0, 0);
        assert_eq!(a.union(&empty), a);
        let b = Rect::new(20, 0, 10, 10);
        assert_eq!(a.union(&b), Rect::new(0, 0, 30, 10));
    }

    #[test]
    fn inflate_grows_all_sides() {
        assert_eq!(Rect::new(10, 10, 5, 5).inflate(2), Rect::new(8, 8, 9, 9));
    }
}
