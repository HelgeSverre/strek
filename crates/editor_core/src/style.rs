use serde::{Deserialize, Serialize};

/// Paint defines how a shape is filled or stroked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    /// Solid color [R, G, B, A] in 0.0-1.0 range
    Solid([f32; 4]),
}

impl Paint {
    /// Create a solid black paint.
    pub fn black() -> Self {
        Paint::Solid([0.0, 0.0, 0.0, 1.0])
    }

    /// Create a solid white paint.
    pub fn white() -> Self {
        Paint::Solid([1.0, 1.0, 1.0, 1.0])
    }

    /// Create a solid color from RGB values (0.0-1.0).
    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Paint::Solid([r, g, b, 1.0])
    }

    /// Create a solid color from RGBA values (0.0-1.0).
    pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Paint::Solid([r, g, b, a])
    }
}

impl Default for Paint {
    fn default() -> Self {
        Paint::black()
    }
}

/// Stroke properties for path rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Stroke width in local units
    pub width: f32,

    /// Stroke paint
    pub paint: Paint,
}

impl Stroke {
    /// Create a new stroke with given width and paint.
    pub fn new(width: f32, paint: Paint) -> Self {
        Self { width, paint }
    }

    /// Create a black stroke with given width.
    pub fn black(width: f32) -> Self {
        Self {
            width,
            paint: Paint::black(),
        }
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: 1.0,
            paint: Paint::black(),
        }
    }
}

/// Visual style for a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    /// Fill paint (None = no fill)
    pub fill: Option<Paint>,

    /// Stroke properties (None = no stroke)
    pub stroke: Option<Stroke>,

    /// Overall opacity (0.0 - 1.0)
    pub opacity: f32,
}

impl Style {
    /// Create a style with only a fill.
    pub fn fill(paint: Paint) -> Self {
        Self {
            fill: Some(paint),
            stroke: None,
            opacity: 1.0,
        }
    }

    /// Create a style with only a stroke.
    pub fn stroke(stroke: Stroke) -> Self {
        Self {
            fill: None,
            stroke: Some(stroke),
            opacity: 1.0,
        }
    }

    /// Create a style with both fill and stroke.
    pub fn fill_and_stroke(fill: Paint, stroke: Stroke) -> Self {
        Self {
            fill: Some(fill),
            stroke: Some(stroke),
            opacity: 1.0,
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Some(Paint::black()),
            stroke: None,
            opacity: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_style() {
        let style = Style::default();
        assert!(style.fill.is_some());
        assert!(style.stroke.is_none());
        assert_eq!(style.opacity, 1.0);
    }

    #[test]
    fn test_paint_colors() {
        let black = Paint::black();
        let white = Paint::white();
        let red = Paint::rgb(1.0, 0.0, 0.0);

        assert_eq!(black, Paint::Solid([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(white, Paint::Solid([1.0, 1.0, 1.0, 1.0]));
        assert_eq!(red, Paint::Solid([1.0, 0.0, 0.0, 1.0]));
    }
}
