use glam::Vec2;
use serde::{Deserialize, Serialize};

/// A single path command in a vector path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    /// Move to position (start new subpath)
    MoveTo(Vec2),

    /// Line to position
    LineTo(Vec2),

    /// Cubic bezier curve with two control points and endpoint
    CubicTo { c1: Vec2, c2: Vec2, p: Vec2 },

    /// Close current subpath
    Close,
}

/// Vector path data containing a sequence of path commands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PathData {
    pub commands: Vec<PathCmd>,
}

impl PathData {
    /// Create a new empty path.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a rectangle path.
    pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            commands: vec![
                PathCmd::MoveTo(Vec2::new(x, y)),
                PathCmd::LineTo(Vec2::new(x + width, y)),
                PathCmd::LineTo(Vec2::new(x + width, y + height)),
                PathCmd::LineTo(Vec2::new(x, y + height)),
                PathCmd::Close,
            ],
        }
    }

    /// Compute the axis-aligned bounding box of this path in local coordinates.
    pub fn bounds(&self) -> Option<Rect> {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        let mut has_points = false;

        for cmd in &self.commands {
            let points: &[Vec2] = match cmd {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => std::slice::from_ref(p),
                PathCmd::CubicTo { c1, c2, p } => {
                    // For cubic bezier bounds, we include control points
                    // This is conservative but simple; proper bounds would
                    // compute curve extrema
                    min = min.min(*c1).min(*c2).min(*p);
                    max = max.max(*c1).max(*c2).max(*p);
                    has_points = true;
                    continue;
                }
                PathCmd::Close => continue,
            };

            for &p in points {
                min = min.min(p);
                max = max.max(p);
                has_points = true;
            }
        }

        if has_points {
            Some(Rect { min, max })
        } else {
            None
        }
    }
}

/// Axis-aligned bounding rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    /// Create a new rect from min and max corners.
    pub fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    /// Create a rect from position and size.
    pub fn from_pos_size(pos: Vec2, size: Vec2) -> Self {
        Self {
            min: pos,
            max: pos + size,
        }
    }

    /// Width of the rectangle.
    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    /// Height of the rectangle.
    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// Size as a Vec2.
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    /// Center point.
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    /// Check if a point is inside the rectangle.
    pub fn contains(&self, point: Vec2) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    /// Union of two rectangles.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Create an empty rect (for accumulating unions).
    pub fn empty() -> Self {
        Self {
            min: Vec2::splat(f32::INFINITY),
            max: Vec2::splat(f32::NEG_INFINITY),
        }
    }

    /// Check if rect is empty/invalid.
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y
    }

    /// Check if this rect intersects with another rect.
    pub fn intersects(&self, other: &Rect) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    /// Calculate the intersection of two rects, if any.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        if !self.intersects(other) {
            return None;
        }
        Some(Rect {
            min: Vec2::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y)),
            max: Vec2::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y)),
        })
    }
}

impl Default for Rect {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_path_bounds() {
        let path = PathData::rect(10.0, 20.0, 100.0, 50.0);
        let bounds = path.bounds().unwrap();
        assert_eq!(bounds.min, Vec2::new(10.0, 20.0));
        assert_eq!(bounds.max, Vec2::new(110.0, 70.0));
    }

    #[test]
    fn test_rect_contains() {
        let rect = Rect::new(Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0));
        assert!(rect.contains(Vec2::new(50.0, 50.0)));
        assert!(!rect.contains(Vec2::new(150.0, 50.0)));
    }
}
