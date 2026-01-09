//! Rectangle type for UI layout and hit testing.

use glam::Vec2;

/// An axis-aligned rectangle defined by position and size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Top-left corner position.
    pub x: f32,
    pub y: f32,
    /// Size of the rectangle.
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Create a new rectangle.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Create a rectangle from position and size.
    pub fn from_pos_size(pos: Vec2, size: Vec2) -> Self {
        Self {
            x: pos.x,
            y: pos.y,
            width: size.x,
            height: size.y,
        }
    }

    /// Create a rectangle from min and max corners.
    pub fn from_min_max(min: Vec2, max: Vec2) -> Self {
        Self {
            x: min.x,
            y: min.y,
            width: max.x - min.x,
            height: max.y - min.y,
        }
    }

    /// Create a zero-sized rectangle at the origin.
    pub const fn zero() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }

    /// Get the position (top-left corner).
    pub fn pos(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    /// Get the size.
    pub fn size(&self) -> Vec2 {
        Vec2::new(self.width, self.height)
    }

    /// Get the minimum corner (top-left).
    pub fn min(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    /// Get the maximum corner (bottom-right).
    pub fn max(&self) -> Vec2 {
        Vec2::new(self.x + self.width, self.y + self.height)
    }

    /// Get the center point.
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    /// Get the left edge x coordinate.
    pub fn left(&self) -> f32 {
        self.x
    }

    /// Get the right edge x coordinate.
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// Get the top edge y coordinate.
    pub fn top(&self) -> f32 {
        self.y
    }

    /// Get the bottom edge y coordinate.
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Check if a point is inside the rectangle.
    pub fn contains(&self, point: Vec2) -> bool {
        point.x >= self.x
            && point.x < self.x + self.width
            && point.y >= self.y
            && point.y < self.y + self.height
    }

    /// Check if this rectangle intersects another.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    /// Compute the intersection of two rectangles.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);

        if right > x && bottom > y {
            Some(Rect::new(x, y, right - x, bottom - y))
        } else {
            None
        }
    }

    /// Offset the rectangle by a vector.
    pub fn offset(&self, offset: Vec2) -> Self {
        Self {
            x: self.x + offset.x,
            y: self.y + offset.y,
            ..*self
        }
    }

    /// Inset the rectangle by a uniform amount.
    pub fn inset(&self, amount: f32) -> Self {
        Self {
            x: self.x + amount,
            y: self.y + amount,
            width: (self.width - 2.0 * amount).max(0.0),
            height: (self.height - 2.0 * amount).max(0.0),
        }
    }

    /// Expand the rectangle by a uniform amount.
    pub fn expand(&self, amount: f32) -> Self {
        self.inset(-amount)
    }

    /// Check if the rectangle has zero or negative area.
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
}
