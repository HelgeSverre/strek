use glam::Affine2;
use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, Key, KeyData};

use crate::layout::AutoLayout;
use crate::path::PathData;
use crate::style::{Paint, Style};

// Generate a new key type for node IDs using slotmap's generational arena
new_key_type! {
    /// Stable identifier for nodes in the document.
    /// Uses generational indexing to detect stale references.
    pub struct NodeId;
}

impl NodeId {
    /// Convert this key into an opaque session-stable handle for external APIs.
    ///
    /// The value must only be converted back with [`Self::from_opaque`]. Its
    /// numeric layout is intentionally not part of the editor's public model.
    pub fn to_opaque(self) -> u64 {
        self.data().as_ffi()
    }

    /// Reconstruct a key previously returned by [`Self::to_opaque`].
    ///
    /// Arbitrary values are safe to construct but will not resolve unless they
    /// identify a live node in the current document.
    pub fn from_opaque(value: u64) -> Self {
        KeyData::from_ffi(value).into()
    }
}

/// The type-specific data for a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Container node with ordered children
    Group,

    /// Frame: container with explicit dimensions (like Figma frames/artboards)
    Frame(FrameData),

    /// Vector path (lines, curves, shapes)
    Shape(PathData),

    /// Semantic editable text content and layout properties
    Text(TextData),
}

/// Font specification for text nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSpec {
    /// Font family name
    pub family: String,

    /// Font weight (100-900, 400 = normal, 700 = bold)
    pub weight: u16,

    /// Italic flag
    pub italic: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: "sans-serif".into(),
            weight: 400,
            italic: false,
        }
    }
}

impl FontSpec {
    /// Create a new font spec with the given family.
    pub fn new(family: impl Into<String>) -> Self {
        Self {
            family: family.into(),
            ..Default::default()
        }
    }

    /// Set the font weight.
    pub fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    /// Set italic.
    pub fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    /// Create a bold font spec.
    pub fn bold(family: impl Into<String>) -> Self {
        Self::new(family).with_weight(700)
    }
}

/// Text alignment within bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Horizontal sizing behavior for a text node.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TextSizing {
    /// The text box grows to the measured line width.
    #[default]
    AutoWidth,
    /// Text wraps to an explicit local-space width.
    FixedWidth { width: f32 },
}

/// Text node data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextData {
    /// Text content (UTF-8)
    pub content: String,

    /// Font specification
    pub font: FontSpec,

    /// Font size in local units
    pub font_size: f32,

    /// Line height multiplier (1.0 = normal)
    pub line_height: f32,

    /// Text alignment within bounds
    pub align: TextAlign,

    /// Auto-width or wrapped fixed-width sizing.
    #[serde(default)]
    pub sizing: TextSizing,
}

impl Default for TextData {
    fn default() -> Self {
        Self {
            content: String::new(),
            font: FontSpec::default(),
            font_size: 16.0,
            line_height: 1.2,
            align: TextAlign::Left,
            sizing: TextSizing::AutoWidth,
        }
    }
}

impl TextData {
    /// Create a new text data with content.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            ..Default::default()
        }
    }

    /// Set the font size.
    pub fn with_font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Set the font specification.
    pub fn with_font(mut self, font: FontSpec) -> Self {
        self.font = font;
        self
    }

    /// Set the line height.
    pub fn with_line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }

    /// Set the text alignment.
    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Set a fixed wrapping width.
    pub fn with_fixed_width(mut self, width: f32) -> Self {
        self.sizing = TextSizing::FixedWidth {
            width: width.max(1.0),
        };
        self
    }

    /// Return the explicit wrapping width, if any.
    pub fn fixed_width(&self) -> Option<f32> {
        match self.sizing {
            TextSizing::AutoWidth => None,
            TextSizing::FixedWidth { width } => Some(width),
        }
    }

    // Convenience accessors for backward compatibility
    /// Get the font family.
    pub fn font_family(&self) -> &str {
        &self.font.family
    }

    /// Get the font weight.
    pub fn weight(&self) -> u16 {
        self.font.weight
    }

    /// Get the italic flag.
    pub fn italic(&self) -> bool {
        self.font.italic
    }
}

/// Frame data for containers with explicit dimensions.
/// Unlike Group, Frame has fixed width/height independent of children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameData {
    /// Explicit width in local units
    pub width: f32,

    /// Explicit height in local units
    pub height: f32,

    /// Optional background (rendered behind children)
    #[serde(default = "default_frame_background")]
    pub background: Option<Paint>,
}

fn default_frame_background() -> Option<Paint> {
    Some(Paint::white())
}

impl Default for FrameData {
    fn default() -> Self {
        Self {
            width: 100.0,
            height: 100.0,
            background: default_frame_background(),
        }
    }
}

impl FrameData {
    /// Create a new frame with given dimensions.
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            background: default_frame_background(),
        }
    }

    /// Create a frame with background.
    pub fn with_background(width: f32, height: f32, background: Paint) -> Self {
        Self {
            width,
            height,
            background: Some(background),
        }
    }
}

/// Layout mode for a node.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum Layout {
    /// Manual positioning via transform
    #[default]
    Free,

    /// Automatic child positioning
    Auto(AutoLayout),
}

impl Layout {
    /// Check if this is auto-layout.
    pub fn is_auto(&self) -> bool {
        matches!(self, Layout::Auto(_))
    }

    /// Get the auto-layout configuration if this is auto-layout.
    pub fn as_auto(&self) -> Option<&AutoLayout> {
        match self {
            Layout::Auto(auto) => Some(auto),
            _ => None,
        }
    }

    /// Get mutable auto-layout configuration if this is auto-layout.
    pub fn as_auto_mut(&mut self) -> Option<&mut AutoLayout> {
        match self {
            Layout::Auto(auto) => Some(auto),
            _ => None,
        }
    }
}

/// A node in the scene graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Human-readable name (for layers panel)
    pub name: String,

    /// Parent reference (None for root)
    pub parent: Option<NodeId>,

    /// Ordered children (layer order: last = topmost)
    pub children: Vec<NodeId>,

    /// Node type and type-specific data
    pub kind: NodeKind,

    /// Local transform (relative to parent)
    pub transform: Affine2,

    /// Visual style (fill, stroke, opacity)
    pub style: Style,

    /// Layout mode (free or auto-layout)
    pub layout: Layout,

    /// Visibility flag
    pub visible: bool,

    /// Lock flag (prevent editing)
    pub locked: bool,

    /// Soft-delete flag for undo/redo support.
    /// Deleted nodes remain in the slotmap to preserve ID stability.
    #[serde(default)]
    pub deleted: bool,
}

impl Node {
    /// Create a new group node.
    pub fn group(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Group,
            transform: Affine2::IDENTITY,
            style: Style::default(),
            layout: Layout::Free,
            visible: true,
            locked: false,
            deleted: false,
        }
    }

    /// Create a new shape node with the given path.
    pub fn shape(name: impl Into<String>, path: PathData) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Shape(path),
            transform: Affine2::IDENTITY,
            style: Style::default(),
            layout: Layout::Free,
            visible: true,
            locked: false,
            deleted: false,
        }
    }

    /// Create a new text node.
    pub fn text(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Text(TextData {
                content: content.into(),
                ..Default::default()
            }),
            transform: Affine2::IDENTITY,
            style: Style::default(),
            layout: Layout::Free,
            visible: true,
            locked: false,
            deleted: false,
        }
    }

    /// Create a new frame node with given dimensions.
    pub fn frame(name: impl Into<String>, width: f32, height: f32) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Frame(FrameData::new(width, height)),
            transform: Affine2::IDENTITY,
            style: Style::default(),
            layout: Layout::Free,
            visible: true,
            locked: false,
            deleted: false,
        }
    }

    /// Check if this node is a group.
    pub fn is_group(&self) -> bool {
        matches!(self.kind, NodeKind::Group)
    }

    /// Check if this node is a shape.
    pub fn is_shape(&self) -> bool {
        matches!(self.kind, NodeKind::Shape(_))
    }

    /// Check if this node is text.
    pub fn is_text(&self) -> bool {
        matches!(self.kind, NodeKind::Text(_))
    }

    /// Check if this node is a frame.
    pub fn is_frame(&self) -> bool {
        matches!(self.kind, NodeKind::Frame(_))
    }

    /// Get the path data if this is a shape node.
    pub fn path(&self) -> Option<&PathData> {
        match &self.kind {
            NodeKind::Shape(path) => Some(path),
            _ => None,
        }
    }

    /// Get mutable path data if this is a shape node.
    pub fn path_mut(&mut self) -> Option<&mut PathData> {
        match &mut self.kind {
            NodeKind::Shape(path) => Some(path),
            _ => None,
        }
    }

    /// Get the text data if this is a text node.
    pub fn text_data(&self) -> Option<&TextData> {
        match &self.kind {
            NodeKind::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Get the frame data if this is a frame node.
    pub fn frame_data(&self) -> Option<&FrameData> {
        match &self.kind {
            NodeKind::Frame(frame) => Some(frame),
            _ => None,
        }
    }

    /// Get mutable frame data if this is a frame node.
    pub fn frame_data_mut(&mut self) -> Option<&mut FrameData> {
        match &mut self.kind {
            NodeKind::Frame(frame) => Some(frame),
            _ => None,
        }
    }

    /// Set the local transform.
    pub fn with_transform(mut self, transform: Affine2) -> Self {
        self.transform = transform;
        self
    }

    /// Set the style.
    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Set visibility.
    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Set the layout mode.
    pub fn with_layout(mut self, layout: Layout) -> Self {
        self.layout = layout;
        self
    }

    /// Set auto-layout (horizontal).
    pub fn with_auto_layout_horizontal(mut self) -> Self {
        self.layout = Layout::Auto(AutoLayout::horizontal());
        self
    }

    /// Set auto-layout (vertical).
    pub fn with_auto_layout_vertical(mut self) -> Self {
        self.layout = Layout::Auto(AutoLayout::vertical());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::PathData;
    use slotmap::SlotMap;

    #[test]
    fn opaque_node_ids_round_trip_without_exposing_key_layout() {
        let mut nodes: SlotMap<NodeId, ()> = SlotMap::with_key();
        let id = nodes.insert(());

        assert_eq!(NodeId::from_opaque(id.to_opaque()), id);
    }

    #[test]
    fn test_create_group() {
        let node = Node::group("My Group");
        assert!(node.is_group());
        assert_eq!(node.name, "My Group");
        assert!(node.visible);
        assert!(!node.locked);
    }

    #[test]
    fn test_create_shape() {
        let path = PathData::rect(0.0, 0.0, 100.0, 100.0);
        let node = Node::shape("Rectangle", path);
        assert!(node.is_shape());
        assert!(node.path().is_some());
    }

    #[test]
    fn test_builder_pattern() {
        let node = Node::group("Test")
            .with_visible(false)
            .with_transform(Affine2::from_translation(glam::Vec2::new(10.0, 20.0)));

        assert!(!node.visible);
        assert_eq!(node.transform.translation, glam::Vec2::new(10.0, 20.0));
    }
}
