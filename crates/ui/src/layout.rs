//! Layout system inspired by flexbox.
//!
//! Layout is computed in two passes:
//! 1. **Measure**: Bottom-up pass to determine each widget's preferred size
//! 2. **Arrange**: Top-down pass to assign final positions and sizes

use glam::Vec2;

/// Size with width and height.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    /// Create a new size.
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Zero size.
    pub const fn zero() -> Self {
        Self::new(0.0, 0.0)
    }

    /// Infinite size (for unconstrained measurement).
    pub const fn infinite() -> Self {
        Self::new(f32::INFINITY, f32::INFINITY)
    }

    /// Convert to Vec2.
    pub fn to_vec2(self) -> Vec2 {
        Vec2::new(self.width, self.height)
    }
}

impl From<Vec2> for Size {
    fn from(v: Vec2) -> Self {
        Self::new(v.x, v.y)
    }
}

impl From<(f32, f32)> for Size {
    fn from((width, height): (f32, f32)) -> Self {
        Self::new(width, height)
    }
}

/// Constraints for layout measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxConstraints {
    /// Minimum width.
    pub min_width: f32,
    /// Maximum width.
    pub max_width: f32,
    /// Minimum height.
    pub min_height: f32,
    /// Maximum height.
    pub max_height: f32,
}

impl BoxConstraints {
    /// Create new constraints.
    pub const fn new(min_width: f32, max_width: f32, min_height: f32, max_height: f32) -> Self {
        Self {
            min_width,
            max_width,
            min_height,
            max_height,
        }
    }

    /// Unconstrained (any size allowed).
    pub const fn unbounded() -> Self {
        Self::new(0.0, f32::INFINITY, 0.0, f32::INFINITY)
    }

    /// Tight constraints (exact size required).
    pub fn tight(size: Size) -> Self {
        Self::new(size.width, size.width, size.height, size.height)
    }

    /// Loose constraints (up to a maximum size).
    pub fn loose(size: Size) -> Self {
        Self::new(0.0, size.width, 0.0, size.height)
    }

    /// Constrain a size to fit within these constraints.
    pub fn constrain(&self, size: Size) -> Size {
        Size::new(
            size.width.clamp(self.min_width, self.max_width),
            size.height.clamp(self.min_height, self.max_height),
        )
    }

    /// Get the maximum size allowed.
    pub fn max_size(&self) -> Size {
        Size::new(self.max_width, self.max_height)
    }

    /// Get the minimum size required.
    pub fn min_size(&self) -> Size {
        Size::new(self.min_width, self.min_height)
    }

    /// Check if the constraints have a bounded width.
    pub fn has_bounded_width(&self) -> bool {
        self.max_width.is_finite()
    }

    /// Check if the constraints have a bounded height.
    pub fn has_bounded_height(&self) -> bool {
        self.max_height.is_finite()
    }

    /// Check if the constraints are tight (exact size required).
    pub fn is_tight(&self) -> bool {
        self.min_width == self.max_width && self.min_height == self.max_height
    }

    /// Create constraints with a fixed width.
    pub fn with_width(self, width: f32) -> Self {
        Self {
            min_width: width,
            max_width: width,
            ..self
        }
    }

    /// Create constraints with a fixed height.
    pub fn with_height(self, height: f32) -> Self {
        Self {
            min_height: height,
            max_height: height,
            ..self
        }
    }

    /// Create constraints with maximum width.
    pub fn with_max_width(self, max_width: f32) -> Self {
        Self { max_width, ..self }
    }

    /// Create constraints with maximum height.
    pub fn with_max_height(self, max_height: f32) -> Self {
        Self { max_height, ..self }
    }
}

impl Default for BoxConstraints {
    fn default() -> Self {
        Self::unbounded()
    }
}

/// Layout axis (horizontal or vertical).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    /// Horizontal (left to right).
    #[default]
    Horizontal,
    /// Vertical (top to bottom).
    Vertical,
}

impl Axis {
    /// Get the cross axis.
    pub fn cross(self) -> Self {
        match self {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        }
    }

    /// Get the main axis component of a size.
    pub fn main(self, size: Size) -> f32 {
        match self {
            Axis::Horizontal => size.width,
            Axis::Vertical => size.height,
        }
    }

    /// Get the cross axis component of a size.
    pub fn cross_size(self, size: Size) -> f32 {
        match self {
            Axis::Horizontal => size.height,
            Axis::Vertical => size.width,
        }
    }

    /// Create a size from main and cross components.
    pub fn pack(self, main: f32, cross: f32) -> Size {
        match self {
            Axis::Horizontal => Size::new(main, cross),
            Axis::Vertical => Size::new(cross, main),
        }
    }

    /// Get the main axis component of constraints.
    pub fn main_constraint(self, constraints: BoxConstraints) -> (f32, f32) {
        match self {
            Axis::Horizontal => (constraints.min_width, constraints.max_width),
            Axis::Vertical => (constraints.min_height, constraints.max_height),
        }
    }

    /// Get the cross axis component of constraints.
    pub fn cross_constraint(self, constraints: BoxConstraints) -> (f32, f32) {
        match self {
            Axis::Horizontal => (constraints.min_height, constraints.max_height),
            Axis::Vertical => (constraints.min_width, constraints.max_width),
        }
    }
}

/// Main axis alignment for flex layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainAlign {
    /// Items packed at the start.
    #[default]
    Start,
    /// Items packed at the end.
    End,
    /// Items centered.
    Center,
    /// Items evenly distributed with space between.
    SpaceBetween,
    /// Items evenly distributed with space around.
    SpaceAround,
    /// Items evenly distributed with equal space.
    SpaceEvenly,
}

/// Cross axis alignment for flex layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CrossAlign {
    /// Items aligned at the start.
    #[default]
    Start,
    /// Items aligned at the end.
    End,
    /// Items centered.
    Center,
    /// Items stretched to fill.
    Stretch,
}

/// General alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

impl Align {
    /// Compute the offset for alignment within available space.
    pub fn offset(self, content_size: f32, available_size: f32) -> f32 {
        match self {
            Align::Start => 0.0,
            Align::Center => (available_size - content_size) * 0.5,
            Align::End => available_size - content_size,
        }
    }
}

/// Padding/margin on all four sides.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Padding {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Padding {
    /// Create padding with the same value on all sides.
    pub const fn all(value: f32) -> Self {
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }

    /// Create padding with symmetric horizontal and vertical values.
    pub const fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self {
            left: horizontal,
            right: horizontal,
            top: vertical,
            bottom: vertical,
        }
    }

    /// Create padding with individual values.
    pub const fn new(left: f32, right: f32, top: f32, bottom: f32) -> Self {
        Self {
            left,
            right,
            top,
            bottom,
        }
    }

    /// Zero padding.
    pub const fn zero() -> Self {
        Self::all(0.0)
    }

    /// Total horizontal padding.
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    /// Total vertical padding.
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }

    /// Total size added by padding.
    pub fn size(&self) -> Size {
        Size::new(self.horizontal(), self.vertical())
    }
}
