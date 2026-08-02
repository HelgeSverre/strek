use glam::{Affine2, Vec2};
use serde::{Deserialize, Serialize};

/// How colors outside a gradient's 0-1 range are extended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SpreadMethod {
    /// Extend the first and last stop colors.
    #[default]
    Pad,
    /// Mirror the gradient on every repetition.
    Reflect,
    /// Repeat the gradient on every repetition.
    Repeat,
}

/// A color stop in a gradient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// Stop position in the normalized 0-1 gradient range.
    pub offset: f32,
    /// Stop color [R, G, B, A] in the 0.0-1.0 range.
    pub color: [f32; 4],
}

/// A linear gradient in the painted node's local coordinate system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearGradient {
    pub start: Vec2,
    pub end: Vec2,
    #[serde(default)]
    pub transform: Affine2,
    #[serde(default)]
    pub spread: SpreadMethod,
    pub stops: Vec<GradientStop>,
}

/// A radial gradient in the painted node's local coordinate system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadialGradient {
    pub center: Vec2,
    pub focal: Vec2,
    pub radius: f32,
    #[serde(default)]
    pub transform: Affine2,
    #[serde(default)]
    pub spread: SpreadMethod,
    pub stops: Vec<GradientStop>,
}

/// Paint defines how a shape is filled or stroked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    /// Solid color [R, G, B, A] in 0.0-1.0 range
    Solid([f32; 4]),
    /// Linear gradient paint.
    LinearGradient(LinearGradient),
    /// Radial gradient paint.
    RadialGradient(RadialGradient),
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

    /// Return this paint's color when it is solid.
    pub fn solid_color(&self) -> Option<[f32; 4]> {
        match self {
            Self::Solid(color) => Some(*color),
            Self::LinearGradient(_) | Self::RadialGradient(_) => None,
        }
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

    /// Shape drawn at the ends of open subpaths.
    #[serde(default)]
    pub line_cap: LineCap,

    /// Shape drawn where path segments meet.
    #[serde(default)]
    pub line_join: LineJoin,

    /// Maximum ratio between miter length and stroke width.
    #[serde(default = "default_miter_limit")]
    pub miter_limit: f32,

    /// Alternating dash and gap lengths. Empty means a solid stroke.
    #[serde(default)]
    pub dash_array: Vec<f32>,

    /// Offset into the dash pattern.
    #[serde(default)]
    pub dash_offset: f32,
}

fn default_miter_limit() -> f32 {
    4.0
}

/// Shape drawn at the ends of open stroked subpaths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// Shape drawn where stroked path segments meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineJoin {
    #[default]
    Miter,
    MiterClip,
    Round,
    Bevel,
}

impl Stroke {
    /// Create a new stroke with given width and paint.
    pub fn new(width: f32, paint: Paint) -> Self {
        Self {
            width,
            paint,
            ..Self::default()
        }
    }

    /// Create a black stroke with given width.
    pub fn black(width: f32) -> Self {
        Self {
            width,
            paint: Paint::black(),
            ..Self::default()
        }
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: 1.0,
            paint: Paint::black(),
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: default_miter_limit(),
            dash_array: Vec::new(),
            dash_offset: 0.0,
        }
    }
}

/// Rule used to decide which regions of a path are filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// Ordering of a path's fill and stroke paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PaintOrder {
    #[default]
    FillAndStroke,
    StrokeAndFill,
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

    /// Rule used when filling compound paths.
    #[serde(default)]
    pub fill_rule: FillRule,

    /// Whether the fill or stroke is painted first.
    #[serde(default)]
    pub paint_order: PaintOrder,
}

impl Style {
    /// Create a style with only a fill.
    pub fn fill(paint: Paint) -> Self {
        Self {
            fill: Some(paint),
            stroke: None,
            opacity: 1.0,
            fill_rule: FillRule::NonZero,
            paint_order: PaintOrder::FillAndStroke,
        }
    }

    /// Create a style with only a stroke.
    pub fn stroke(stroke: Stroke) -> Self {
        Self {
            fill: None,
            stroke: Some(stroke),
            opacity: 1.0,
            fill_rule: FillRule::NonZero,
            paint_order: PaintOrder::FillAndStroke,
        }
    }

    /// Create a style with both fill and stroke.
    pub fn fill_and_stroke(fill: Paint, stroke: Stroke) -> Self {
        Self {
            fill: Some(fill),
            stroke: Some(stroke),
            opacity: 1.0,
            fill_rule: FillRule::NonZero,
            paint_order: PaintOrder::FillAndStroke,
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Some(Paint::black()),
            stroke: None,
            opacity: 1.0,
            fill_rule: FillRule::NonZero,
            paint_order: PaintOrder::FillAndStroke,
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
