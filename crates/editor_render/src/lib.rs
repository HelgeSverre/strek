//! Editor Render - Backend-agnostic rendering primitives
//!
//! This crate provides the display list intermediate representation that
//! can be consumed by different rendering backends (wgpu, SVG, etc.)

use glam::{Affine2, Vec2};

/// A list of display items to render.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

impl DisplayList {
    /// Create a new empty display list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an item to the display list.
    pub fn push(&mut self, item: DisplayItem) {
        self.items.push(item);
    }

    /// Check if the display list is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Get the number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Clear the display list.
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

/// A single item to render.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    /// Begin an isolated artwork group.
    ///
    /// Group opacity is applied after its children are composited. An optional
    /// clip path is expressed in the same screen-space coordinate system as
    /// the group's child display items.
    BeginGroup {
        opacity: f32,
        clip_path: Option<ClipPath>,
    },

    /// Finish the most recently opened artwork group.
    EndGroup,

    /// Fill a path with a paint.
    FillPath {
        path: PathData,
        paint: Paint,
        fill_rule: FillRule,
        transform: Affine2,
        opacity: f32,
    },

    /// Stroke a path.
    StrokePath {
        path: PathData,
        stroke: Stroke,
        transform: Affine2,
        opacity: f32,
    },

    /// Render text.
    Text {
        text: TextItem,
        transform: Affine2,
        opacity: f32,
    },

    /// Transient shape shown while a drawing tool is active.
    ///
    /// This is UI-only and must not be included in exported artwork.
    ToolPreview {
        path: PathData,
        fill: Paint,
        stroke: Stroke,
        transform: Affine2,
    },

    /// Snap guide line for visual feedback during dragging.
    SnapGuide {
        /// Start point in screen coordinates.
        start: Vec2,
        /// End point in screen coordinates.
        end: Vec2,
        /// Whether this is a horizontal guide (otherwise vertical).
        horizontal: bool,
    },

    /// Selection rectangle around a selected node (UI-only, not for export).
    SelectionRect {
        /// Minimum corner in screen coordinates.
        min: Vec2,
        /// Maximum corner in screen coordinates.
        max: Vec2,
    },

    /// Rotated selection outline in screen coordinates.
    SelectionQuad { corners: [Vec2; 4] },

    /// Marquee selection rectangle (drawn while dragging to select).
    MarqueeRect {
        /// Minimum corner in screen coordinates.
        min: Vec2,
        /// Maximum corner in screen coordinates.
        max: Vec2,
    },

    /// Anchor point shown during direct vector editing.
    VectorAnchor { position: Vec2, selected: bool },

    /// Line connecting an anchor to one Bézier handle.
    VectorHandle { anchor: Vec2, handle: Vec2 },

    /// Text caret line in screen coordinates.
    TextCaret { start: Vec2, end: Vec2 },

    /// One visual line of an active text selection or IME composition.
    TextSelectionRect { corners: [Vec2; 4], marked: bool },

    /// Resize or rotation control for the current selection.
    TransformHandle { position: Vec2, rotation: bool },
}

/// Path data for rendering.
#[derive(Debug, Clone, Default, PartialEq)]
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

    /// Create a circle path (approximated with bezier curves).
    pub fn circle(cx: f32, cy: f32, r: f32) -> Self {
        // Approximate circle with 4 cubic bezier curves
        // Magic number for control point distance
        let k = r * 0.552_284_8;

        Self {
            commands: vec![
                PathCmd::MoveTo(Vec2::new(cx + r, cy)),
                PathCmd::CubicTo {
                    c1: Vec2::new(cx + r, cy + k),
                    c2: Vec2::new(cx + k, cy + r),
                    p: Vec2::new(cx, cy + r),
                },
                PathCmd::CubicTo {
                    c1: Vec2::new(cx - k, cy + r),
                    c2: Vec2::new(cx - r, cy + k),
                    p: Vec2::new(cx - r, cy),
                },
                PathCmd::CubicTo {
                    c1: Vec2::new(cx - r, cy - k),
                    c2: Vec2::new(cx - k, cy - r),
                    p: Vec2::new(cx, cy - r),
                },
                PathCmd::CubicTo {
                    c1: Vec2::new(cx + k, cy - r),
                    c2: Vec2::new(cx + r, cy - k),
                    p: Vec2::new(cx + r, cy),
                },
                PathCmd::Close,
            ],
        }
    }
}

/// Path command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCmd {
    /// Move to position (start new subpath)
    MoveTo(Vec2),

    /// Line to position
    LineTo(Vec2),

    /// Cubic bezier curve
    CubicTo { c1: Vec2, c2: Vec2, p: Vec2 },

    /// Close current subpath
    Close,
}

/// Paint for fills and strokes.
#[derive(Debug, Clone, PartialEq)]
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

    /// Create a solid color from RGB values.
    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Paint::Solid([r, g, b, 1.0])
    }

    /// Create a solid color from RGBA values.
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

/// Stroke properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    /// Stroke width
    pub width: f32,

    /// Stroke paint
    pub paint: Paint,

    /// Line cap style
    pub line_cap: LineCap,

    /// Line join style
    pub line_join: LineJoin,

    /// Maximum ratio between miter length and stroke width.
    pub miter_limit: f32,

    /// Alternating dash and gap lengths. Empty means a solid stroke.
    pub dash_array: Vec<f32>,

    /// Offset into the dash pattern.
    pub dash_offset: f32,
}

impl Stroke {
    /// Create a new stroke with given width and paint.
    pub fn new(width: f32, paint: Paint) -> Self {
        Self {
            width,
            paint,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: 4.0,
            dash_array: Vec::new(),
            dash_offset: 0.0,
        }
    }

    /// Create a black stroke with given width.
    pub fn black(width: f32) -> Self {
        Self::new(width, Paint::black())
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: 1.0,
            paint: Paint::black(),
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            miter_limit: 4.0,
            dash_array: Vec::new(),
            dash_offset: 0.0,
        }
    }
}

/// Line cap style for strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// Line join style for strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    MiterClip,
    Round,
    Bevel,
}

/// Rule used to decide which regions of a path are filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// How colors outside a gradient's 0-1 range are extended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpreadMethod {
    #[default]
    Pad,
    Reflect,
    Repeat,
}

/// A color stop in a gradient.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: [f32; 4],
}

/// Linear gradient in the painted path's local coordinate system.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub start: Vec2,
    pub end: Vec2,
    pub transform: Affine2,
    pub spread: SpreadMethod,
    pub stops: Vec<GradientStop>,
}

/// Radial gradient in the painted path's local coordinate system.
#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    pub center: Vec2,
    pub focal: Vec2,
    pub radius: f32,
    pub transform: Affine2,
    pub spread: SpreadMethod,
    pub stops: Vec<GradientStop>,
}

/// One path participating in a clip region.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipShape {
    pub path: PathData,
    pub transform: Affine2,
    pub fill_rule: FillRule,
}

/// One node in a clip-path subtree.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipNode {
    Group(ClipPath),
    Shape(ClipShape),
}

/// A clip region applied to an artwork group.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClipPath {
    pub children: Vec<ClipNode>,
    pub clip_path: Option<Box<ClipPath>>,
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// One line from a frontend-resolved text layout.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTextLine {
    /// UTF-8 byte range within the text item's content.
    pub range: std::ops::Range<usize>,

    /// Horizontal line origin in text-local coordinates.
    pub x: f32,
}

/// Frontend-resolved text layout shared with export renderers.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTextLayout {
    /// Visual lines in paint order.
    pub lines: Vec<ResolvedTextLine>,

    /// Distance between consecutive baselines.
    pub line_height: f32,

    /// Width of the text container in local coordinates.
    pub width: f32,
}

/// Text item for rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct TextItem {
    /// Text content
    pub content: String,

    /// Font family name
    pub font_family: String,

    /// Font size
    pub font_size: f32,

    /// Font weight (100-900)
    pub font_weight: u16,

    /// Italic flag
    pub font_italic: bool,

    /// Fill paint
    pub fill: Paint,

    /// Line-height multiplier.
    pub line_height: f32,

    /// Horizontal alignment within a fixed-width box.
    pub alignment: TextAlignment,

    /// Explicit wrapping width in local units.
    pub wrap_width: Option<f32>,

    /// Resolved line breaks and alignment from the active text shaper.
    pub layout: Option<ResolvedTextLayout>,
}

impl TextItem {
    /// Create a new text item.
    pub fn new(content: impl Into<String>, font_size: f32) -> Self {
        Self {
            content: content.into(),
            font_family: "sans-serif".into(),
            font_size,
            font_weight: 400,
            font_italic: false,
            fill: Paint::black(),
            line_height: 1.2,
            alignment: TextAlignment::Left,
            wrap_width: None,
            layout: None,
        }
    }

    /// Set the font family.
    pub fn with_font_family(mut self, family: impl Into<String>) -> Self {
        self.font_family = family.into();
        self
    }

    /// Set the font weight.
    pub fn with_weight(mut self, weight: u16) -> Self {
        self.font_weight = weight;
        self
    }

    /// Set italic.
    pub fn with_italic(mut self, italic: bool) -> Self {
        self.font_italic = italic;
        self
    }

    /// Set the fill paint.
    pub fn with_fill(mut self, fill: Paint) -> Self {
        self.fill = fill;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_list() {
        let mut list = DisplayList::new();
        assert!(list.is_empty());

        list.push(DisplayItem::FillPath {
            path: PathData::rect(0.0, 0.0, 100.0, 100.0),
            paint: Paint::black(),
            fill_rule: FillRule::NonZero,
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });

        assert_eq!(list.len(), 1);
        assert!(!list.is_empty());
    }

    #[test]
    fn test_path_rect() {
        let path = PathData::rect(10.0, 20.0, 100.0, 50.0);
        assert_eq!(path.commands.len(), 5);
    }

    #[test]
    fn test_path_circle() {
        let path = PathData::circle(50.0, 50.0, 25.0);
        // 1 MoveTo + 4 CubicTo + 1 Close = 6 commands
        assert_eq!(path.commands.len(), 6);
    }

    #[test]
    fn test_text_item() {
        let text = TextItem::new("Hello", 16.0)
            .with_font_family("Arial")
            .with_weight(700)
            .with_italic(true)
            .with_fill(Paint::rgb(1.0, 0.0, 0.0));

        assert_eq!(text.content, "Hello");
        assert_eq!(text.font_family, "Arial");
        assert_eq!(text.font_weight, 700);
        assert!(text.font_italic);
    }
}
