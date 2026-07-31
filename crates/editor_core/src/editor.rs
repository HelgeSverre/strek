//! Main editor state combining document, selection, history, and tools.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use editor_render::DisplayList;
use glam::{Affine2, Vec2};
use unicode_segmentation::UnicodeSegmentation;

use crate::action::EditorAction;
use crate::command::{Command, Patch};
use crate::history::History;
use crate::input::{Cursor, Effects, InputEvent, Key, Modifiers, MouseButton};
use crate::layers::{LayerEntry, LayerIcon, LayerPanelState};
use crate::layout::{AlignCross, AlignMain, AutoLayout, Direction};
use crate::node::{Layout, Node, NodeId, NodeKind, TextData, TextSizing};
use crate::path::{HandleMode, PathAnchor, PathContour, PathData, Rect};
use crate::property_scrub::{
    NumericPropertyScrubSession, NumericPropertyScrubState, NumericPropertySnapshot,
    NumericPropertyTarget, StyleSnapshot, TransformSnapshot,
};
use crate::selection::{Selection, SelectionAction};
use crate::snap::{SnapEngine, SnapKind, SnapPoint, SnapResult};
use crate::style::{Paint, Stroke, Style};
use crate::transaction::Transaction;
use crate::transform::View;
use crate::Document;

/// The active tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Frame,
    Rectangle,
    Ellipse,
    Line,
    Pen,
    Text,
    VectorEdit,
}

/// Session-scoped styles used when creating new vector objects.
///
/// Closed shapes and paths keep separate presets because their useful defaults
/// differ: closed shapes are normally filled, while open paths need a stroke
/// to remain visible. These defaults are editor state, not document state.
#[derive(Debug, Clone, PartialEq)]
pub struct CreationStyleDefaults {
    closed_shapes: Style,
    paths: Style,
}

impl Default for CreationStyleDefaults {
    fn default() -> Self {
        Self {
            closed_shapes: Style::fill(Paint::rgb(0.85, 0.86, 0.88)),
            paths: Style::stroke(Stroke::new(1.5, Paint::rgb(0.85, 0.86, 0.88))),
        }
    }
}

/// Layout mode exposed by the single-group design inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupLayoutMode {
    Free,
    Horizontal,
    Vertical,
}

/// Stable affine components suitable for displaying in a transform inspector.
///
/// The decomposition uses the transformed x-axis as the rotation basis and
/// reports any remaining x-shear separately. Singular and non-finite matrices
/// do not have a useful inspector representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformComponents {
    pub translation: Vec2,
    pub scale: Vec2,
    pub rotation_radians: f32,
    pub skew_radians: f32,
}

impl TransformComponents {
    /// Decompose an affine transform without panicking on invalid matrices.
    pub fn from_affine(transform: Affine2) -> Option<Self> {
        if !transform.matrix2.is_finite() || !transform.translation.is_finite() {
            return None;
        }

        let determinant = transform.matrix2.determinant();
        let scale_x = transform.matrix2.x_axis.length();
        if !determinant.is_finite()
            || determinant.abs() <= f32::EPSILON
            || !scale_x.is_finite()
            || scale_x <= f32::EPSILON
        {
            return None;
        }

        let x_axis = transform.matrix2.x_axis / scale_x;
        let shear = x_axis.dot(transform.matrix2.y_axis);
        let orthogonal_y = transform.matrix2.y_axis - x_axis * shear;
        let scale_y_magnitude = orthogonal_y.length();
        if !scale_y_magnitude.is_finite() || scale_y_magnitude <= f32::EPSILON {
            return None;
        }

        let scale_y = scale_y_magnitude.copysign(determinant);
        let rotation_radians = x_axis.y.atan2(x_axis.x);
        let skew_radians = (shear / scale_y).atan();
        if !rotation_radians.is_finite() || !skew_radians.is_finite() {
            return None;
        }

        Some(Self {
            translation: transform.translation,
            scale: Vec2::new(scale_x, scale_y),
            rotation_radians,
            skew_radians,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShapeKind {
    Rectangle,
    Ellipse,
    Line,
}

impl ShapeKind {
    fn from_tool(tool: Tool) -> Option<Self> {
        match tool {
            Tool::Select | Tool::Frame | Tool::Pen | Tool::Text | Tool::VectorEdit => None,
            Tool::Rectangle => Some(Self::Rectangle),
            Tool::Ellipse => Some(Self::Ellipse),
            Tool::Line => Some(Self::Line),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
        }
    }

    fn path(self, size: Vec2) -> PathData {
        match self {
            Self::Rectangle => PathData::rect(0.0, 0.0, size.x, size.y),
            Self::Ellipse => PathData::ellipse(0.0, 0.0, size.x, size.y),
            Self::Line => unreachable!("line geometry depends on drag direction"),
        }
    }
}

/// Stable public summary of the active editor interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InteractionKind {
    #[default]
    Idle,
    Moving,
    Resizing,
    Rotating,
    Marquee,
    CreatingShape,
    CreatingFrame,
    Pen,
    CreatingText,
    TextEditing,
    VectorEditing,
    Panning,
}

/// Immutable world-space artwork prepared for serialization or rasterization.
#[derive(Debug, Clone)]
pub struct ArtworkSnapshot {
    /// Document-only display items with world-space transforms.
    pub display_list: DisplayList,
    /// Conservative visual bounds, including transformed shape strokes.
    pub bounds: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeHandle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

impl ResizeHandle {
    fn affects_x(self) -> bool {
        !matches!(self, Self::North | Self::South)
    }

    fn affects_y(self) -> bool {
        !matches!(self, Self::East | Self::West)
    }

    fn is_corner(self) -> bool {
        self.affects_x() && self.affects_y()
    }
}

#[derive(Debug, Clone)]
struct TransformNodeSnapshot {
    transform: Affine2,
    kind: NodeKind,
    world: Affine2,
}

#[derive(Debug)]
struct TransformSession {
    description: &'static str,
    bounds: Rect,
    start: Vec2,
    handle: Option<ResizeHandle>,
    nodes: HashMap<NodeId, TransformNodeSnapshot>,
    preserved_children: HashMap<NodeId, (Affine2, Affine2)>,
}

/// An anchor selected for direct vector editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnchorRef {
    pub node: NodeId,
    pub contour: usize,
    pub anchor: usize,
}

/// Platform-neutral snapshot exposed to the native IME adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputSnapshot {
    pub node: NodeId,
    pub text: String,
    pub selection: Range<usize>,
    pub marked: Option<Range<usize>>,
    pub selection_reversed: bool,
}

/// Logical caret navigation understood by platform frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextNavigation {
    PreviousGrapheme,
    NextGrapheme,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
}

#[derive(Debug, Clone)]
struct PenSession {
    parent: NodeId,
    anchors: Vec<PathAnchor>,
    redo: Vec<PathAnchor>,
    hover: Vec2,
    active_anchor: Option<usize>,
    pointer_down: bool,
}

#[derive(Debug, Clone)]
struct VectorEditSession {
    node: NodeId,
    before: HashMap<NodeId, PathData>,
    selected: HashSet<AnchorRef>,
    dragging: Option<(VectorDragTarget, Vec2, HashMap<NodeId, PathData>)>,
    undo: Vec<VectorEditSnapshot>,
    redo: Vec<VectorEditSnapshot>,
}

#[derive(Debug, Clone)]
struct VectorEditSnapshot {
    paths: HashMap<NodeId, PathData>,
    selected: HashSet<AnchorRef>,
}

#[derive(Debug, Clone, Copy)]
enum VectorDragTarget {
    Anchor(AnchorRef),
    InHandle(AnchorRef),
    OutHandle(AnchorRef),
}

#[derive(Debug, Clone)]
struct TextEditSession {
    node: NodeId,
    before: Option<TextData>,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    selection_reversed: bool,
    parent: NodeId,
    index: usize,
    undo: Vec<TextEditSnapshot>,
    redo: Vec<TextEditSnapshot>,
    pointer_selection_anchor: Option<usize>,
}

#[derive(Debug, Clone)]
struct TextEditSnapshot {
    text: TextData,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
    selection_reversed: bool,
}

#[derive(Debug, Clone)]
struct ClipboardNode {
    node: Node,
    children: Vec<ClipboardNode>,
}

#[derive(Debug, Clone)]
struct ClipboardRoot {
    node: ClipboardNode,
    world: Affine2,
}

/// Frontend-neutral snapshot of copied editor objects.
#[derive(Debug, Clone, Default)]
pub struct EditorClipboard {
    roots: Vec<ClipboardRoot>,
    paste_count: u32,
}

impl EditorClipboard {
    /// Whether this clipboard contains any copied objects.
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

fn snapshot_clipboard_node(document: &Document, id: NodeId) -> Option<ClipboardNode> {
    let source = document.get(id)?;
    if source.deleted {
        return None;
    }
    let child_ids = source.children.clone();
    let mut node = source.clone();
    node.parent = None;
    node.children.clear();
    node.deleted = false;
    let children = child_ids
        .into_iter()
        .map(|child| snapshot_clipboard_node(document, child))
        .collect::<Option<Vec<_>>>()?;
    Some(ClipboardNode { node, children })
}

/// Explicit state machine for all pointer and editing interactions.
#[derive(Default)]
enum InteractionState {
    /// No drag in progress
    #[default]
    None,
    /// Dragging to move selected nodes
    Moving {
        start_pos: Vec2,
        transaction: Transaction,
        /// Snap points from the moving nodes (for snap source)
        source_snap_points: Vec<SnapPoint>,
        /// Current snap result (for rendering guides)
        snap_result: Option<SnapResult>,
    },
    /// Resizing selected objects from one selection-box handle.
    Resizing(TransformSession),
    /// Rotating selected objects around the selection center.
    Rotating(TransformSession),
    /// Marquee selection
    Marquee { start_pos: Vec2, current_pos: Vec2 },
    /// Drawing a new shape.
    CreatingShape {
        shape: ShapeKind,
        start_pos: Vec2,
        current_pos: Vec2,
        modifiers: Modifiers,
        snap_result: Option<SnapResult>,
    },
    /// Drawing a new frame.
    CreatingFrame {
        start_pos: Vec2,
        current_pos: Vec2,
        modifiers: Modifiers,
        snap_result: Option<SnapResult>,
    },
    /// Constructing a Bézier path across multiple pointer gestures.
    Pen(PenSession),
    /// Dragging a point or fixed-width text box.
    CreatingText { start_pos: Vec2, current_pos: Vec2 },
    /// Editing semantic text through the platform input bridge.
    TextEditing(TextEditSession),
    /// Direct anchor and handle editing.
    VectorEditing(VectorEditSession),
    /// Panning the view
    Panning {
        start_pos: Vec2,
        start_pan: Vec2,
        resume: Option<Box<InteractionState>>,
    },
}

type DragState = InteractionState;

fn creation_endpoint(start: Vec2, current: Vec2, constrain_proportions: bool) -> Vec2 {
    let mut delta = current - start;

    if constrain_proportions {
        let side = delta.x.abs().max(delta.y.abs());
        let x_sign = if delta.x < 0.0 { -1.0 } else { 1.0 };
        let y_sign = if delta.y < 0.0 { -1.0 } else { 1.0 };
        delta = Vec2::new(x_sign * side, y_sign * side);
    }

    start + delta
}

fn creation_bounds(start: Vec2, current: Vec2, modifiers: Modifiers) -> Rect {
    let end = creation_endpoint(start, current, modifiers.shift);
    let delta = end - start;

    if modifiers.alt {
        let radius = delta.abs();
        Rect::new(start - radius, start + radius)
    } else {
        Rect::new(start.min(end), start.max(end))
    }
}

fn constrained_line_endpoint(start: Vec2, current: Vec2, constrain_angle: bool) -> Vec2 {
    let delta = current - start;
    if !constrain_angle || delta.length_squared() <= f32::EPSILON {
        return current;
    }

    let increment = std::f32::consts::FRAC_PI_4;
    let angle = (delta.y.atan2(delta.x) / increment).round() * increment;
    start + Vec2::new(angle.cos(), angle.sin()) * delta.length()
}

fn creation_geometry(
    shape: ShapeKind,
    start: Vec2,
    current: Vec2,
    modifiers: Modifiers,
) -> (Rect, PathData) {
    if shape != ShapeKind::Line {
        let bounds = creation_bounds(start, current, modifiers);
        return (bounds, shape.path(bounds.size()));
    }

    let end = constrained_line_endpoint(start, current, modifiers.shift);
    let delta = end - start;
    let (line_start, line_end) = if modifiers.alt {
        (start - delta, start + delta)
    } else {
        (start, end)
    };
    let bounds = Rect::new(line_start.min(line_end), line_start.max(line_end));
    let path = PathData {
        contours: vec![PathContour::open([
            PathAnchor::corner(line_start - bounds.min),
            PathAnchor::corner(line_end - bounds.min),
        ])],
    };
    (bounds, path)
}

fn creation_snap_points(position: Vec2) -> [SnapPoint; 2] {
    [
        SnapPoint::new(position, SnapKind::Left, None),
        SnapPoint::new(position, SnapKind::Top, None),
    ]
}

fn canonical_font_weight(weight: u16) -> u16 {
    ((weight.clamp(100, 900) + 50) / 100) * 100
}

fn bounded_numeric_sum(value: f32, delta: f32, minimum: f32, maximum: f32) -> Option<f32> {
    let sum = value + delta;
    sum.is_finite().then(|| sum.clamp(minimum, maximum))
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn swap_contour_handles(contour: &mut PathContour) {
    for anchor in &mut contour.anchors {
        std::mem::swap(&mut anchor.in_handle, &mut anchor.out_handle);
    }
}

fn transform_contour(contour: &mut PathContour, transform: Affine2) {
    for anchor in &mut contour.anchors {
        anchor.position = transform.transform_point2(anchor.position);
        anchor.in_handle = anchor
            .in_handle
            .map(|handle| transform.transform_vector2(handle));
        anchor.out_handle = anchor
            .out_handle
            .map(|handle| transform.transform_vector2(handle));
    }
}

fn cubic_point_at(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    let one_minus_t = 1.0 - t;
    one_minus_t.powi(3) * p0
        + 3.0 * one_minus_t.powi(2) * t * p1
        + 3.0 * one_minus_t * t.powi(2) * p2
        + t.powi(3) * p3
}

fn estimated_text_position(
    layout: &crate::text::TextLayout,
    content: &str,
    byte_index: usize,
) -> Vec2 {
    layout.position_for_byte(content, byte_index)
}

fn estimated_text_range_rects(
    layout: &crate::text::TextLayout,
    content: &str,
    range: Range<usize>,
) -> Vec<Rect> {
    layout.range_rects(content, range)
}

fn resize_handle_points(bounds: Rect) -> [(ResizeHandle, Vec2); 8] {
    let center = bounds.center();
    [
        (ResizeHandle::NorthWest, bounds.min),
        (ResizeHandle::North, Vec2::new(center.x, bounds.min.y)),
        (
            ResizeHandle::NorthEast,
            Vec2::new(bounds.max.x, bounds.min.y),
        ),
        (ResizeHandle::East, Vec2::new(bounds.max.x, center.y)),
        (ResizeHandle::SouthEast, bounds.max),
        (ResizeHandle::South, Vec2::new(center.x, bounds.max.y)),
        (
            ResizeHandle::SouthWest,
            Vec2::new(bounds.min.x, bounds.max.y),
        ),
        (ResizeHandle::West, Vec2::new(bounds.min.x, center.y)),
    ]
}

fn resize_handle_position(handle: ResizeHandle, bounds: Rect) -> Vec2 {
    resize_handle_points(bounds)
        .into_iter()
        .find_map(|(candidate, position)| (candidate == handle).then_some(position))
        .expect("all resize handles have a position")
}

fn opposite_handle(handle: ResizeHandle) -> ResizeHandle {
    match handle {
        ResizeHandle::NorthWest => ResizeHandle::SouthEast,
        ResizeHandle::North => ResizeHandle::South,
        ResizeHandle::NorthEast => ResizeHandle::SouthWest,
        ResizeHandle::East => ResizeHandle::West,
        ResizeHandle::SouthEast => ResizeHandle::NorthWest,
        ResizeHandle::South => ResizeHandle::North,
        ResizeHandle::SouthWest => ResizeHandle::NorthEast,
        ResizeHandle::West => ResizeHandle::East,
    }
}

/// The main editor state.
pub struct Editor {
    /// The document being edited
    pub document: Document,

    /// Current selection
    pub selection: Selection,

    /// Undo/redo history
    pub history: History,

    /// Camera/viewport state
    pub view: View,

    /// Size of the drawable canvas in screen pixels.
    viewport_size: Vec2,

    /// Current tool
    pub tool: Tool,

    /// Non-document style presets for newly created vector objects.
    creation_styles: CreationStyleDefaults,

    /// Live numeric property edit, kept outside document history until commit.
    numeric_property_scrub: Option<NumericPropertyScrubState>,

    /// Per-editor identity carried by opaque numeric property scrub tokens.
    numeric_property_scrub_owner: Arc<()>,

    /// Monotonic identity for opaque numeric property scrub tokens.
    next_numeric_property_scrub_id: u64,

    /// Drag state
    drag: DragState,

    /// Whether a redraw is needed
    needs_redraw: bool,

    /// Snap engine for finding snap targets
    snap_engine: SnapEngine,

    /// Layer panel UI state (expanded/collapsed)
    pub layer_panel: LayerPanelState,

    /// Whether Space currently turns the primary pointer into a hand tool.
    space_pan_active: bool,
}

impl Editor {
    /// Create a new editor with an empty document.
    pub fn new() -> Self {
        Self {
            document: Document::new(),
            selection: Selection::new(),
            history: History::new(),
            view: View::default(),
            viewport_size: Vec2::new(800.0, 600.0),
            tool: Tool::Select,
            creation_styles: CreationStyleDefaults::default(),
            numeric_property_scrub: None,
            numeric_property_scrub_owner: Arc::new(()),
            next_numeric_property_scrub_id: 1,
            drag: DragState::None,
            needs_redraw: true,
            snap_engine: SnapEngine::new(),
            layer_panel: LayerPanelState::new(),
            space_pan_active: false,
        }
    }

    /// Create a clean editor session for an existing document.
    pub fn from_document(document: Document) -> Self {
        Self {
            document,
            ..Self::new()
        }
    }

    /// Create an editor with a demo document.
    pub fn with_demo_content() -> Self {
        let mut editor = Self::new();

        // Add some demo shapes
        let rect1 = Node::shape("Blue Rectangle", PathData::rect(0.0, 0.0, 150.0, 100.0))
            .with_style(Style::fill(Paint::rgb(0.2, 0.4, 0.8)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(50.0, 50.0)));

        let rect2 = Node::shape("Red Rectangle", PathData::rect(0.0, 0.0, 100.0, 150.0))
            .with_style(Style::fill(Paint::rgb(0.8, 0.2, 0.2)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(250.0, 100.0)));

        let rect3 = Node::shape("Green Rectangle", PathData::rect(0.0, 0.0, 120.0, 80.0))
            .with_style(Style::fill(Paint::rgb(0.2, 0.7, 0.3)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(150.0, 300.0)));

        // Add demo text
        let text1 = Node::text("Hello Text", "Hello, World!")
            .with_style(Style::fill(Paint::rgb(0.1, 0.1, 0.1)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(400.0, 80.0)));

        let text2 = Node::text("Title", "Strek")
            .with_style(Style::fill(Paint::rgb(0.3, 0.3, 0.8)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(450.0, 250.0)));

        editor.document.add_child(editor.document.root, rect1);
        editor.document.add_child(editor.document.root, rect2);
        editor.document.add_child(editor.document.root, rect3);
        editor.document.add_child(editor.document.root, text1);
        editor.document.add_child(editor.document.root, text2);

        editor.needs_redraw = true;
        editor
    }

    // === Action System ===

    /// Execute an editor action.
    /// Returns true if the action was executed.
    pub fn execute_action(&mut self, action: EditorAction) -> bool {
        if !self.can_execute(action) {
            return false;
        }

        if !matches!(action, EditorAction::Undo | EditorAction::Redo) {
            self.cancel_active_numeric_property_scrub();
        }

        if matches!(
            action,
            EditorAction::ZoomIn
                | EditorAction::ZoomOut
                | EditorAction::ZoomReset
                | EditorAction::ZoomResetAll
                | EditorAction::ZoomToFit
                | EditorAction::ZoomToSelection
        ) {
            self.cancel_pointer_interaction();
        }

        let action_manages_interaction = matches!(
            action,
            EditorAction::ToolSelect
                | EditorAction::ToolFrame
                | EditorAction::ToolRectangle
                | EditorAction::ToolEllipse
                | EditorAction::ToolLine
                | EditorAction::ToolPen
                | EditorAction::ToolText
                | EditorAction::EnterVectorEdit
                | EditorAction::FinishEditing
                | EditorAction::JoinPaths
                | EditorAction::SplitPath
                | EditorAction::ReversePath
                | EditorAction::TogglePathClosed
                | EditorAction::Undo
                | EditorAction::Redo
                | EditorAction::ZoomIn
                | EditorAction::ZoomOut
                | EditorAction::ZoomReset
                | EditorAction::ZoomResetAll
                | EditorAction::ZoomToFit
                | EditorAction::ZoomToSelection
        );
        if !action_manages_interaction {
            if matches!(
                self.drag,
                DragState::Pen(_) | DragState::TextEditing(_) | DragState::VectorEditing(_)
            ) {
                self.finish_editing();
            } else {
                self.cancel_interaction();
            }
        }

        match action {
            // Selection
            EditorAction::SelectAll => {
                let scope = self.selection_scope();
                let all_nodes = self
                    .document
                    .get(scope)
                    .map(|node| node.children.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|id| self.document.is_effectively_editable(*id))
                    .collect::<Vec<_>>();
                self.selection.set(all_nodes);
                self.needs_redraw = true;
            }
            EditorAction::DeselectAll => {
                self.selection.clear();
                self.needs_redraw = true;
            }
            EditorAction::InvertSelection => {
                let scope = self.selection_scope();
                let selected: HashSet<_> = self
                    .selection
                    .iter()
                    .filter_map(|id| self.direct_child_on_path(scope, id))
                    .collect();
                let inverted = self
                    .document
                    .get(scope)
                    .map(|node| node.children.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|id| {
                        !selected.contains(id) && self.document.is_effectively_editable(*id)
                    })
                    .collect::<Vec<_>>();
                self.selection.set(inverted);
                self.needs_redraw = true;
            }

            // Edit
            EditorAction::Undo => {
                self.undo_in_context();
            }
            EditorAction::Redo => {
                self.redo_in_context();
            }
            EditorAction::Delete => {
                self.delete_selection();
            }
            EditorAction::Duplicate => {
                self.duplicate_selection();
            }

            // Arrange
            EditorAction::Group => {
                self.group_selection();
            }
            EditorAction::Ungroup => {
                self.ungroup_selection();
            }
            EditorAction::BringToFront => {
                self.bring_selection_to_front();
            }
            EditorAction::SendToBack => {
                self.send_selection_to_back();
            }
            EditorAction::BringForward => {
                self.bring_selection_forward();
            }
            EditorAction::SendBackward => {
                self.send_selection_backward();
            }
            EditorAction::AlignLeft => {
                self.align_selection(crate::snap::AlignAxis::Left);
            }
            EditorAction::AlignCenter => {
                self.align_selection(crate::snap::AlignAxis::CenterX);
            }
            EditorAction::AlignRight => {
                self.align_selection(crate::snap::AlignAxis::Right);
            }
            EditorAction::AlignTop => {
                self.align_selection(crate::snap::AlignAxis::Top);
            }
            EditorAction::AlignMiddle => {
                self.align_selection(crate::snap::AlignAxis::CenterY);
            }
            EditorAction::AlignBottom => {
                self.align_selection(crate::snap::AlignAxis::Bottom);
            }
            EditorAction::DistributeHorizontal => {
                self.distribute_selection(crate::snap::DistributeAxis::Horizontal);
            }
            EditorAction::DistributeVertical => {
                self.distribute_selection(crate::snap::DistributeAxis::Vertical);
            }

            // Nudge
            EditorAction::NudgeUp => {
                self.nudge_selection(Vec2::new(0.0, -1.0));
            }
            EditorAction::NudgeDown => {
                self.nudge_selection(Vec2::new(0.0, 1.0));
            }
            EditorAction::NudgeLeft => {
                self.nudge_selection(Vec2::new(-1.0, 0.0));
            }
            EditorAction::NudgeRight => {
                self.nudge_selection(Vec2::new(1.0, 0.0));
            }
            EditorAction::NudgeUpLarge => {
                self.nudge_selection(Vec2::new(0.0, -10.0));
            }
            EditorAction::NudgeDownLarge => {
                self.nudge_selection(Vec2::new(0.0, 10.0));
            }
            EditorAction::NudgeLeftLarge => {
                self.nudge_selection(Vec2::new(-10.0, 0.0));
            }
            EditorAction::NudgeRightLarge => {
                self.nudge_selection(Vec2::new(10.0, 0.0));
            }

            // View
            EditorAction::ZoomIn => {
                self.set_zoom(self.view.zoom * 1.25);
            }
            EditorAction::ZoomOut => {
                self.set_zoom(self.view.zoom / 1.25);
            }
            EditorAction::ZoomReset => {
                self.set_zoom(1.0);
            }
            EditorAction::ZoomResetAll => {
                self.view.zoom = 1.0;
                self.view.pan = Vec2::ZERO;
                self.needs_redraw = true;
            }
            EditorAction::ZoomToFit => {
                self.zoom_to_fit();
            }
            EditorAction::ZoomToSelection => {
                self.zoom_to_selection();
            }

            // Tools
            EditorAction::ToolSelect => {
                self.set_tool(Tool::Select);
            }
            EditorAction::ToolFrame => {
                self.set_tool(Tool::Frame);
            }
            EditorAction::ToolRectangle => {
                self.set_tool(Tool::Rectangle);
            }
            EditorAction::ToolEllipse => {
                self.set_tool(Tool::Ellipse);
            }
            EditorAction::ToolLine => {
                self.set_tool(Tool::Line);
            }
            EditorAction::ToolPen => {
                self.set_tool(Tool::Pen);
            }
            EditorAction::ToolText => {
                self.set_tool(Tool::Text);
            }
            EditorAction::EnterVectorEdit => {
                self.enter_vector_edit();
            }
            EditorAction::FinishEditing => {
                self.finish_editing();
            }
            EditorAction::JoinPaths => {
                self.join_selected_paths();
            }
            EditorAction::SplitPath => {
                self.split_selected_path();
            }
            EditorAction::ReversePath => {
                self.reverse_selected_paths();
            }
            EditorAction::TogglePathClosed => {
                self.toggle_selected_contour_closed();
            }
        }

        true
    }

    /// Check if an action can be executed in the current state.
    pub fn can_execute(&self, action: EditorAction) -> bool {
        match action {
            // Always available
            EditorAction::SelectAll
            | EditorAction::DeselectAll
            | EditorAction::InvertSelection
            | EditorAction::ZoomIn
            | EditorAction::ZoomOut
            | EditorAction::ZoomReset
            | EditorAction::ZoomResetAll
            | EditorAction::ZoomToFit
            | EditorAction::ToolSelect
            | EditorAction::ToolFrame
            | EditorAction::ToolRectangle
            | EditorAction::ToolEllipse
            | EditorAction::ToolLine
            | EditorAction::ToolPen
            | EditorAction::ToolText => true,

            // Require selection
            EditorAction::Delete
            | EditorAction::Duplicate
            | EditorAction::BringToFront
            | EditorAction::SendToBack
            | EditorAction::BringForward
            | EditorAction::SendBackward
            | EditorAction::NudgeUp
            | EditorAction::NudgeDown
            | EditorAction::NudgeLeft
            | EditorAction::NudgeRight
            | EditorAction::NudgeUpLarge
            | EditorAction::NudgeDownLarge
            | EditorAction::NudgeLeftLarge
            | EditorAction::NudgeRightLarge
            | EditorAction::ZoomToSelection => !self.selection.is_empty(),

            // Group requires at least two top-level sibling selections.
            EditorAction::Group => {
                let selected: Vec<_> = self.selection.iter().collect();
                let nodes = self.document.filter_selection_for_transform(&selected);
                nodes.len() >= 2
                    && self
                        .document
                        .get(nodes[0])
                        .and_then(|node| node.parent)
                        .is_some_and(|parent| {
                            nodes[1..].iter().all(|id| {
                                self.document.get(*id).and_then(|node| node.parent) == Some(parent)
                            })
                        })
            }

            EditorAction::AlignLeft
            | EditorAction::AlignCenter
            | EditorAction::AlignRight
            | EditorAction::AlignTop
            | EditorAction::AlignMiddle
            | EditorAction::AlignBottom => self.can_transform_selection(2),

            EditorAction::DistributeHorizontal | EditorAction::DistributeVertical => {
                self.can_transform_selection(3)
            }

            // Ungroup requires grouped selection
            EditorAction::Ungroup => self
                .selection
                .iter()
                .any(|id| self.document.get(id).map(|n| n.is_group()).unwrap_or(false)),

            // History actions
            EditorAction::Undo => self.can_undo_in_context(),
            EditorAction::Redo => self.can_redo_in_context(),

            EditorAction::EnterVectorEdit => {
                self.selection.len() == 1
                    && self
                        .selection
                        .primary()
                        .is_some_and(|id| self.document.get(id).and_then(Node::path).is_some())
            }
            EditorAction::FinishEditing => matches!(
                self.drag,
                DragState::Pen(_) | DragState::TextEditing(_) | DragState::VectorEditing(_)
            ),
            EditorAction::JoinPaths => self.can_join_selected_paths(),
            EditorAction::SplitPath => self.vector_selected_anchors().len() == 1,
            EditorAction::ReversePath | EditorAction::TogglePathClosed => {
                matches!(self.drag, DragState::VectorEditing(_))
            }
        }
    }

    fn can_transform_selection(&self, minimum: usize) -> bool {
        let selected: Vec<_> = self.selection.iter().collect();
        let nodes = self.document.filter_selection_for_transform(&selected);
        nodes.len() >= minimum
            && nodes
                .into_iter()
                .all(|id| self.node_has_transform_bounds(id))
    }

    fn node_has_transform_bounds(&self, id: NodeId) -> bool {
        let Some(node) = self.document.get(id) else {
            return false;
        };
        if matches!(node.kind, NodeKind::Group) {
            return node.children.iter().copied().any(|child| {
                self.document.is_effectively_visible(child) && self.node_has_transform_bounds(child)
            });
        }
        self.document
            .local_bounds(id)
            .is_some_and(|bounds| !bounds.is_empty())
    }

    /// Find the action for a key press.
    pub fn action_for_key(&self, key: Key, modifiers: Modifiers) -> Option<EditorAction> {
        for action in EditorAction::all() {
            if let Some(shortcut) = action.meta().default_shortcut {
                if shortcut.key == key
                    && shortcut.ctrl == modifiers.primary()
                    && shortcut.shift == modifiers.shift
                    && shortcut.alt == modifiers.alt
                {
                    return Some(*action);
                }
            }
        }

        // Handle Backspace as alternative for Delete
        if key == Key::Backspace && !modifiers.any() {
            return Some(EditorAction::Delete);
        }

        // Handle Ctrl+Y as alternative for Redo
        if key == Key::Y && modifiers.primary() && !modifiers.shift {
            return Some(EditorAction::Redo);
        }

        None
    }

    fn set_tool(&mut self, tool: Tool) {
        self.settle_interaction();
        self.tool = tool;
        self.needs_redraw = true;
    }

    fn settle_interaction(&mut self) {
        self.cancel_active_numeric_property_scrub();
        if matches!(self.drag, DragState::Panning { .. }) {
            let DragState::Panning { resume, .. } = std::mem::take(&mut self.drag) else {
                unreachable!("interaction was checked as panning");
            };
            if let Some(resume) = resume {
                self.drag = *resume;
            }
        }
        if !self.finish_editing() {
            self.cancel_interaction();
        }
    }

    fn cancel_interaction(&mut self) -> bool {
        let drag = std::mem::take(&mut self.drag);
        let cancelled = !matches!(drag, DragState::None);

        match drag {
            DragState::Moving { transaction, .. } => transaction.cancel(&mut self.document),
            DragState::Resizing(session) | DragState::Rotating(session) => {
                self.restore_transform_session(&session)
            }
            DragState::Panning {
                start_pan, resume, ..
            } => {
                self.view.pan = start_pan;
                if let Some(resume) = resume {
                    self.drag = *resume;
                }
            }
            DragState::TextEditing(session) => self.cancel_text_edit(session),
            DragState::VectorEditing(session) => {
                for (id, before) in session.before {
                    if let Some(node) = self.document.get_mut(id) {
                        if let Some(path) = node.path_mut() {
                            *path = before;
                        }
                    }
                    self.document.mark_bounds_dirty(id);
                }
                self.tool = Tool::Select;
            }
            DragState::None
            | DragState::Marquee { .. }
            | DragState::CreatingShape { .. }
            | DragState::CreatingFrame { .. }
            | DragState::CreatingText { .. }
            | DragState::Pen(_) => {}
        }

        self.snap_engine.clear_targets();
        if cancelled {
            self.needs_redraw = true;
        }
        cancelled
    }

    /// Commit the active multi-event editing session.
    pub fn finish_editing(&mut self) -> bool {
        let state = std::mem::take(&mut self.drag);
        let finished = match state {
            DragState::Pen(session) => {
                if session.anchors.len() >= 2 {
                    self.commit_pen_session(session, false);
                }
                true
            }
            DragState::TextEditing(session) => {
                self.commit_text_edit(session);
                true
            }
            DragState::VectorEditing(session) => {
                self.commit_vector_edit(session);
                self.tool = Tool::Select;
                true
            }
            other => {
                self.drag = other;
                false
            }
        };

        if finished {
            self.needs_redraw = true;
        }
        finished
    }

    /// Return the stable public interaction summary used by the desktop chrome.
    pub fn interaction_kind(&self) -> InteractionKind {
        match self.drag {
            DragState::None => InteractionKind::Idle,
            DragState::Moving { .. } => InteractionKind::Moving,
            DragState::Resizing(_) => InteractionKind::Resizing,
            DragState::Rotating(_) => InteractionKind::Rotating,
            DragState::Marquee { .. } => InteractionKind::Marquee,
            DragState::CreatingShape { .. } => InteractionKind::CreatingShape,
            DragState::CreatingFrame { .. } => InteractionKind::CreatingFrame,
            DragState::Pen(_) => InteractionKind::Pen,
            DragState::CreatingText { .. } => InteractionKind::CreatingText,
            DragState::TextEditing(_) => InteractionKind::TextEditing,
            DragState::VectorEditing(_) => InteractionKind::VectorEditing,
            DragState::Panning { .. } => InteractionKind::Panning,
        }
    }

    /// Cancel pointer-driven work before a viewport layout change.
    ///
    /// Text, vector, and Pen editing sessions remain active. An in-progress
    /// text selection or Pen gesture is ended without discarding its editing
    /// session. An interrupted vector drag is restored to its pointer-down
    /// snapshot. If panning temporarily suspended another pointer interaction,
    /// both transient layers are cancelled; suspended editing is restored.
    pub fn cancel_pointer_interaction(&mut self) -> bool {
        let mut cancelled = std::mem::take(&mut self.space_pan_active);
        self.needs_redraw |= cancelled;
        loop {
            match &mut self.drag {
                DragState::None => return cancelled,
                DragState::TextEditing(session) => {
                    let pointer_cancelled = session.pointer_selection_anchor.take().is_some();
                    self.needs_redraw |= pointer_cancelled;
                    return cancelled || pointer_cancelled;
                }
                DragState::VectorEditing(session) => {
                    let originals = session.dragging.take().map(|(_, _, originals)| originals);
                    let Some(originals) = originals else {
                        return cancelled;
                    };
                    for (id, path) in originals {
                        if let Some(node) = self.document.get_mut(id) {
                            if let Some(document_path) = node.path_mut() {
                                *document_path = path;
                            }
                        }
                        self.document.mark_bounds_dirty(id);
                    }
                    self.needs_redraw = true;
                    return true;
                }
                DragState::Pen(session) => {
                    let pointer_cancelled = session.pointer_down || session.active_anchor.is_some();
                    session.pointer_down = false;
                    session.active_anchor = None;
                    if pointer_cancelled {
                        self.needs_redraw = true;
                    }
                    return cancelled || pointer_cancelled;
                }
                _ => {}
            }

            if !self.cancel_interaction() {
                return cancelled;
            }
            cancelled = true;
        }
    }

    /// Settle transient editing state before saving, loading, or exporting.
    pub fn settle_for_document_io(&mut self) {
        self.settle_interaction();
    }

    /// Mark the current document revision as persisted.
    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    /// Mark a specific document revision as persisted.
    pub fn mark_revision_saved(&mut self, revision: u64) {
        self.history.mark_revision_saved(revision);
    }

    /// The revision represented by the current document state.
    pub fn current_revision(&self) -> u64 {
        self.history.current_revision()
    }

    /// Whether the current document differs from its saved revision.
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    fn commit_pen_session(&mut self, session: PenSession, closed: bool) {
        if session.anchors.len() < 2 {
            return;
        }

        let mut world_path = PathData {
            contours: vec![PathContour {
                anchors: session.anchors,
                closed,
            }],
        };
        let Some(bounds) = world_path.bounds() else {
            return;
        };
        let node_world = Affine2::from_translation(bounds.min);
        let inverse_node_world = node_world.inverse();
        for contour in &mut world_path.contours {
            for anchor in &mut contour.anchors {
                let world_position = anchor.position;
                anchor.position = inverse_node_world.transform_point2(world_position);
                anchor.in_handle = anchor
                    .in_handle
                    .map(|handle| inverse_node_world.transform_vector2(handle));
                anchor.out_handle = anchor
                    .out_handle
                    .map(|handle| inverse_node_world.transform_vector2(handle));
            }
        }

        let parent_world = self.document.world_transform(session.parent);
        let transform = parent_world.inverse() * node_world;
        let node = Node::shape("Path", world_path)
            .with_style(self.creation_styles.paths.clone())
            .with_transform(transform);
        let index = self
            .document
            .get(session.parent)
            .map(|parent| parent.children.len())
            .unwrap_or_default();
        let id = self.document.nodes.insert(node.clone());
        let command = Command::new("Create Path").with_patch(Patch::CreateNode {
            id,
            node: Box::new(node),
            parent: session.parent,
            index,
        });
        command.apply(&mut self.document);
        self.history.push(command);
        self.selection.select(id);
    }

    fn commit_vector_edit(&mut self, session: VectorEditSession) {
        let primary = session.node;
        let edited_nodes = session.before.keys().copied().collect::<Vec<_>>();
        let mut patches = Vec::new();
        for (id, before) in session.before {
            let Some(after) = self.document.get(id).and_then(|node| node.path()).cloned() else {
                continue;
            };
            if before != after {
                patches.push(Patch::SetPath {
                    id,
                    before,
                    after: after.clone(),
                });
            }
            if after.contours.is_empty() {
                let Some(node) = self.document.get(id).cloned() else {
                    continue;
                };
                let Some(parent) = node.parent else {
                    continue;
                };
                let index = self
                    .document
                    .get(parent)
                    .and_then(|parent| parent.children.iter().position(|child| *child == id))
                    .unwrap_or_default();
                patches.push(Patch::DeleteNode {
                    id,
                    node: Box::new(node),
                    parent,
                    index,
                });
            }
        }
        if !patches.is_empty() {
            let command = Command::new("Edit Vector").with_patches(patches);
            command.apply(&mut self.document);
            self.history.push(command);
        }
        if self.document.get(primary).is_some_and(|node| !node.deleted) {
            self.selection.select(primary);
        } else if let Some(id) = edited_nodes
            .into_iter()
            .find(|id| self.document.get(*id).is_some_and(|node| !node.deleted))
        {
            self.selection.select(id);
        } else {
            self.selection.clear();
        }
    }

    fn cancel_text_edit(&mut self, session: TextEditSession) {
        if let Some(before) = session.before {
            if let Some(node) = self.document.get_mut(session.node) {
                if let NodeKind::Text(text) = &mut node.kind {
                    *text = before;
                }
            }
            self.document.mark_bounds_dirty(session.node);
        } else {
            self.document.remove(session.node);
            self.selection.clear();
        }
    }

    fn commit_text_edit(&mut self, session: TextEditSession) {
        let Some(after) = self
            .document
            .get(session.node)
            .and_then(|node| match &node.kind {
                NodeKind::Text(text) => Some(text.clone()),
                _ => None,
            })
        else {
            return;
        };

        if let Some(before) = session.before {
            if before != after {
                self.history
                    .push(Command::new("Edit Text").with_patch(Patch::SetText {
                        id: session.node,
                        before,
                        after,
                    }));
            }
        } else if after.content.is_empty() {
            self.document.remove(session.node);
            self.selection.clear();
        } else {
            let Some(node) = self.document.get(session.node).cloned() else {
                return;
            };
            self.history
                .push(Command::new("Create Text").with_patch(Patch::CreateNode {
                    id: session.node,
                    node: Box::new(node),
                    parent: session.parent,
                    index: session.index,
                }));
        }
    }

    /// Remove selected IDs that are no longer reachable in the live document.
    fn prune_selection(&mut self) {
        let live_nodes: HashSet<_> = self.document.descendants(self.document.root).collect();
        let retained = self
            .selection
            .iter()
            .filter(|id| live_nodes.contains(id) && self.document.is_effectively_editable(*id))
            .collect::<Vec<_>>();
        self.selection.set(retained);
    }

    // === Layer Panel ===

    /// Build the layer tree for the layer panel UI.
    /// Returns a flat list of entries in display order (top to bottom).
    pub fn build_layer_tree(&self) -> Vec<LayerEntry> {
        let mut entries = Vec::new();
        self.build_layer_tree_recursive(self.document.root, 0, &mut entries);
        entries
    }

    fn build_layer_tree_recursive(&self, id: NodeId, depth: usize, entries: &mut Vec<LayerEntry>) {
        let Some(node) = self.document.get(id) else {
            return;
        };

        // Skip the root node itself, but process its children
        if id != self.document.root {
            let is_container = matches!(node.kind, NodeKind::Group | NodeKind::Frame(_));
            let has_children = !node.children.is_empty();
            let expanded = self.layer_panel.is_expanded(id);

            let icon = match &node.kind {
                NodeKind::Group => LayerIcon::Group,
                NodeKind::Frame(_) => LayerIcon::Frame,
                NodeKind::Shape(_) => LayerIcon::Shape,
                NodeKind::Text(_) => LayerIcon::Text,
            };

            entries.push(LayerEntry {
                id,
                name: node.name.clone(),
                depth,
                selected: self.selection.contains(id),
                visible: node.visible,
                locked: node.locked,
                editable: self.document.is_effectively_editable(id),
                is_container,
                expanded,
                has_children,
                icon,
            });

            // If collapsed, don't show children
            if is_container && !expanded {
                return;
            }
        }

        // Process children in reverse order (topmost first in the layer panel)
        for child_id in node.children.iter().rev() {
            let child_depth = if id == self.document.root {
                0
            } else {
                depth + 1
            };
            self.build_layer_tree_recursive(*child_id, child_depth, entries);
        }
    }

    /// Toggle visibility of a node.
    pub fn toggle_visibility(&mut self, id: NodeId) {
        self.settle_interaction();
        let Some(before) = self.document.get(id).map(|node| node.visible) else {
            return;
        };
        let command = Command::new("Toggle Visibility").with_patch(Patch::SetVisible {
            id,
            before,
            after: !before,
        });
        command.apply(&mut self.document);
        self.history.push(command);
        self.prune_selection();
        self.needs_redraw = true;
    }

    /// Toggle locked state of a node.
    pub fn toggle_locked(&mut self, id: NodeId) {
        self.settle_interaction();
        let Some(before) = self.document.get(id).map(|node| node.locked) else {
            return;
        };
        let command = Command::new("Toggle Lock").with_patch(Patch::SetLocked {
            id,
            before,
            after: !before,
        });
        command.apply(&mut self.document);
        self.history.push(command);
        self.prune_selection();
        self.needs_redraw = true;
    }

    /// Select a node from the layer panel, or toggle it in a multi-selection.
    pub fn select_layer(&mut self, id: NodeId, toggle_selection: bool) {
        if !self.document.is_effectively_editable(id) {
            return;
        }
        self.settle_interaction();
        if toggle_selection {
            self.toggle_selection_target(id);
        } else {
            self.selection.select(id);
        }
        self.needs_redraw = true;
    }

    /// Rename a layer with undo support.
    pub fn rename_layer(&mut self, id: NodeId, name: &str) -> bool {
        let name = name.trim();
        let Some(before) = self
            .document
            .get(id)
            .filter(|node| !node.deleted)
            .map(|node| node.name.clone())
        else {
            return false;
        };
        if id == self.document.root || name.is_empty() || before == name {
            return false;
        }

        self.settle_interaction();
        let command = Command::new("Rename Layer").with_patch(Patch::SetName {
            id,
            before,
            after: name.to_owned(),
        });
        command.apply(&mut self.document);
        self.history.push(command);
        self.needs_redraw = true;
        true
    }

    /// Move a layer to a parent and final child index with undo support.
    ///
    /// The index is interpreted after removing the layer from its current
    /// parent. Cross-parent moves preserve the layer's world transform.
    pub fn reparent_layer(&mut self, id: NodeId, after_parent: NodeId, after_index: usize) -> bool {
        if id == self.document.root
            || id == after_parent
            || self.document.is_ancestor_of(id, after_parent)
            || !self.document.is_effectively_editable(id)
        {
            return false;
        }

        let Some(after_parent_node) = self.document.get(after_parent) else {
            return false;
        };
        if after_parent_node.deleted
            || (after_parent != self.document.root
                && !matches!(after_parent_node.kind, NodeKind::Group | NodeKind::Frame(_)))
        {
            return false;
        }

        let Some((before_parent, before_transform)) = self
            .document
            .get(id)
            .and_then(|node| node.parent.map(|parent| (parent, node.transform)))
        else {
            return false;
        };
        let Some(before_index) = self
            .document
            .get(before_parent)
            .and_then(|parent| parent.children.iter().position(|child| *child == id))
        else {
            return false;
        };

        let available_slots = self
            .document
            .get(after_parent)
            .map(|parent| {
                parent
                    .children
                    .len()
                    .saturating_sub(usize::from(before_parent == after_parent))
            })
            .unwrap_or_default();
        let after_index = after_index.min(available_slots);
        if before_parent == after_parent && before_index == after_index {
            return false;
        }
        if before_parent != after_parent {
            let after_parent_world = self.document.world_transform(after_parent);
            let determinant = after_parent_world.matrix2.determinant();
            if !determinant.is_finite()
                || determinant.abs() <= f32::EPSILON
                || !after_parent_world.translation.is_finite()
            {
                return false;
            }
        }

        self.settle_interaction();
        let before_world = self.document.world_transform(id);
        let mut patches = vec![Patch::Reparent {
            id,
            before_parent,
            after_parent,
            before_index,
            after_index,
        }];

        if before_parent != after_parent {
            let mut moved_document = self.document.clone();
            patches[0].apply_forward(&mut moved_document);
            moved_document.restore_world_transforms(&[(id, before_world)]);
            if let Some(after) = moved_document.get(id).map(|node| node.transform) {
                if before_transform != after {
                    patches.push(Patch::SetTransform {
                        id,
                        before: before_transform,
                        after,
                    });
                }
            }
        }

        let command = Command::new("Move Layer").with_patches(patches);
        command.apply(&mut self.document);
        self.history.push(command);
        self.layer_panel.expand(after_parent);
        self.selection.select(id);
        self.needs_redraw = true;
        true
    }

    /// Toggle expand/collapse state of a layer in the layer panel.
    pub fn toggle_layer_expand(&mut self, id: NodeId) {
        self.layer_panel.toggle(id);
        self.needs_redraw = true;
    }

    /// Handle an input event and return effects.
    pub fn handle_event(&mut self, event: InputEvent) -> Effects {
        match event {
            InputEvent::PointerDown {
                position,
                button,
                modifiers,
            } => self.handle_pointer_down(position, button, modifiers),
            InputEvent::PointerMove {
                position,
                modifiers,
            } => self.handle_pointer_move(position, modifiers),
            InputEvent::PointerUp {
                position,
                button,
                modifiers,
            } => self.handle_pointer_up(position, button, modifiers),
            InputEvent::Scroll {
                position,
                delta,
                modifiers,
            } => self.handle_scroll(position, delta, modifiers),
            InputEvent::KeyDown { key, modifiers } => self.handle_key_down(key, modifiers),
            InputEvent::KeyUp { key, modifiers } => self.handle_key_up(key, modifiers),
            InputEvent::ModifiersChanged { modifiers } => self.handle_modifiers_changed(modifiers),
        }
    }

    /// Resolve a painted leaf to the object users expect to manipulate.
    ///
    /// The common parent of the current selection is the active selection
    /// scope. A normal click selects the direct child of that scope, while the
    /// primary modifier deliberately bypasses the scope for deep selection.
    /// With no scoped selection, the document root is used, so groups and
    /// frames behave as single objects on first click.
    fn selection_target_for_hit(&self, hit: NodeId, deep_select: bool) -> NodeId {
        if deep_select {
            return hit;
        }

        let scope = self.selection_scope();
        self.direct_child_on_path(scope, hit)
            .or_else(|| self.direct_child_on_path(self.document.root, hit))
            .unwrap_or(hit)
    }

    fn selection_scope(&self) -> NodeId {
        let mut selected = self.selection.iter();
        let Some(first) = selected.next() else {
            return self.document.root;
        };
        let Some(mut scope) = self.document.get(first).and_then(|node| node.parent) else {
            return self.document.root;
        };

        for id in selected {
            let Some(parent) = self.document.get(id).and_then(|node| node.parent) else {
                return self.document.root;
            };
            scope = self
                .lowest_common_ancestor(scope, parent)
                .unwrap_or(self.document.root);
        }

        scope
    }

    fn lowest_common_ancestor(&self, first: NodeId, second: NodeId) -> Option<NodeId> {
        let first_lineage = std::iter::once(first)
            .chain(self.document.ancestors(first))
            .collect::<HashSet<_>>();
        std::iter::once(second)
            .chain(self.document.ancestors(second))
            .find(|candidate| first_lineage.contains(candidate))
    }

    fn direct_child_on_path(&self, ancestor: NodeId, descendant: NodeId) -> Option<NodeId> {
        let mut current = descendant;
        while let Some(parent) = self.document.get(current).and_then(|node| node.parent) {
            if parent == ancestor {
                return Some(current);
            }
            current = parent;
        }
        None
    }

    fn add_selection_target(&mut self, id: NodeId) {
        let conflicts = self
            .selection
            .iter()
            .filter(|selected| {
                self.document.is_ancestor_of(*selected, id)
                    || self.document.is_ancestor_of(id, *selected)
            })
            .collect::<Vec<_>>();
        for conflict in conflicts {
            self.selection.remove(conflict);
        }
        self.selection.add(id);
    }

    fn toggle_selection_target(&mut self, id: NodeId) {
        if self.selection.contains(id) {
            self.selection.remove(id);
        } else {
            self.add_selection_target(id);
        }
    }

    fn select_one_level_toward(&mut self, descendant: NodeId) -> bool {
        if self.selection.len() != 1 {
            return false;
        }
        let Some(selected) = self.selection.primary() else {
            return false;
        };
        let Some(child) = self.direct_child_on_path(selected, descendant) else {
            return false;
        };
        self.selection.select(child);
        self.layer_panel.expand(selected);
        self.needs_redraw = true;
        true
    }

    /// Descend one selection level, or begin editing an already-selected leaf.
    pub fn enter_selection(&mut self) -> bool {
        if self.selection.len() != 1 || !matches!(self.drag, DragState::None) {
            return false;
        }
        let Some(selected) = self.selection.primary() else {
            return false;
        };
        let child = self.document.get(selected).and_then(|node| {
            node.children
                .iter()
                .rev()
                .copied()
                .find(|child| self.document.is_effectively_editable(*child))
        });
        if let Some(child) = child {
            self.selection.select(child);
            self.layer_panel.expand(selected);
            self.needs_redraw = true;
            return true;
        }

        if self
            .document
            .get(selected)
            .is_some_and(|node| matches!(node.kind, NodeKind::Shape(_)))
        {
            self.enter_vector_edit();
            return true;
        }
        if self
            .document
            .get(selected)
            .is_some_and(|node| matches!(node.kind, NodeKind::Text(_)))
        {
            self.begin_existing_text_edit(selected);
            return true;
        }
        false
    }

    /// Move the entire selection one level toward the document root.
    ///
    /// The operation is atomic when any selected node is already top-level.
    /// Overlapping parents collapse to their outermost selected ancestor.
    pub fn select_parent(&mut self) -> bool {
        if self.selection.is_empty() || !matches!(self.drag, DragState::None) {
            return false;
        }

        let mut parents = Vec::with_capacity(self.selection.len());
        for id in self.selection.iter() {
            let Some(parent) = self.document.get(id).and_then(|node| node.parent) else {
                return false;
            };
            if parent == self.document.root {
                return false;
            }
            parents.push(parent);
        }

        let parents = self.document.filter_selection_for_transform(&parents);
        self.selection.set(parents);
        self.needs_redraw = true;
        true
    }

    fn handle_pointer_down(
        &mut self,
        screen_pos: Vec2,
        button: MouseButton,
        modifiers: Modifiers,
    ) -> Effects {
        let world_pos = self.view.to_world(screen_pos);

        if self.space_pan_active
            && button == MouseButton::Left
            && !matches!(
                self.drag,
                DragState::TextEditing(_) | DragState::Panning { .. }
            )
        {
            let suspended = std::mem::take(&mut self.drag);
            let resume = (!matches!(suspended, DragState::None)).then(|| Box::new(suspended));
            self.drag = DragState::Panning {
                start_pos: screen_pos,
                start_pan: self.view.pan,
                resume,
            };
            self.needs_redraw = true;
            return Effects {
                redraw: true,
                cursor: Some(Cursor::Grabbing),
            };
        }

        if matches!(self.drag, DragState::Pen(_)) && button == MouseButton::Left {
            self.pen_pointer_down(world_pos, modifiers);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        if matches!(self.drag, DragState::VectorEditing(_)) && button == MouseButton::Left {
            self.vector_pointer_down(world_pos, modifiers);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        if matches!(self.drag, DragState::TextEditing(_)) && button == MouseButton::Left {
            self.text_pointer_down(world_pos, modifiers);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }

        if !matches!(self.drag, DragState::None) {
            return Effects {
                redraw: false,
                cursor: Some(self.cursor_for_state()),
            };
        }

        match button {
            MouseButton::Left => {
                match self.tool {
                    Tool::Select => {
                        if self.start_selection_transform(screen_pos, world_pos) {
                            return Effects {
                                redraw: true,
                                cursor: Some(self.cursor_for_state()),
                            };
                        }
                        let hit = self
                            .document
                            .hit_test_with_tolerance(
                                world_pos,
                                false,
                                4.0 / self.view.zoom.abs().max(f32::EPSILON),
                            )
                            .map(|node| self.selection_target_for_hit(node, modifiers.primary()));
                        let action = self.selection.action_for_click(hit, modifiers.shift);

                        match action {
                            SelectionAction::ClickUnselected(id) => {
                                self.selection.select(id);
                                // Start move drag - filter to exclude children of selected parents
                                let all_nodes: Vec<_> = self.selection.iter().collect();
                                let nodes =
                                    self.document.filter_selection_for_transform(&all_nodes);
                                let transaction =
                                    Transaction::start_move("Move", &nodes, &mut self.document);

                                // Collect source snap points from selected nodes' bounds
                                let mut source_snap_points = Vec::new();
                                for &node_id in &nodes {
                                    source_snap_points.extend(self.document.snap_points(node_id));
                                }

                                // Set snap targets (all nodes except selected ones)
                                let targets = self.document.all_snap_points(&nodes);
                                self.snap_engine.set_targets(targets);

                                self.drag = DragState::Moving {
                                    start_pos: world_pos,
                                    transaction,
                                    source_snap_points,
                                    snap_result: None,
                                };
                                self.needs_redraw = true;
                            }
                            SelectionAction::ClickSelected(_) => {
                                // Start move drag with current selection - filter to exclude children
                                let all_nodes: Vec<_> = self.selection.iter().collect();
                                let nodes =
                                    self.document.filter_selection_for_transform(&all_nodes);
                                let transaction =
                                    Transaction::start_move("Move", &nodes, &mut self.document);

                                // Collect source snap points from selected nodes' bounds
                                let mut source_snap_points = Vec::new();
                                for &node_id in &nodes {
                                    source_snap_points.extend(self.document.snap_points(node_id));
                                }

                                // Set snap targets (all nodes except selected ones)
                                let targets = self.document.all_snap_points(&nodes);
                                self.snap_engine.set_targets(targets);

                                self.drag = DragState::Moving {
                                    start_pos: world_pos,
                                    transaction,
                                    source_snap_points,
                                    snap_result: None,
                                };
                            }
                            SelectionAction::ToggleNode(id) => {
                                self.toggle_selection_target(id);
                                self.needs_redraw = true;
                            }
                            SelectionAction::StartMarquee => {
                                self.drag = DragState::Marquee {
                                    start_pos: world_pos,
                                    current_pos: world_pos,
                                };
                            }
                        }
                    }
                    Tool::Rectangle | Tool::Ellipse | Tool::Line => {
                        let Some(shape) = ShapeKind::from_tool(self.tool) else {
                            unreachable!("shape tools map to a shape kind")
                        };
                        let targets = self.document.all_snap_points(&[]);
                        self.snap_engine.set_targets(targets);

                        let start_snap = self.snap_engine.find_snap(
                            &creation_snap_points(world_pos),
                            Vec2::ZERO,
                            self.view.zoom,
                        );
                        let start_pos = world_pos + start_snap.position;
                        self.drag = DragState::CreatingShape {
                            shape,
                            start_pos,
                            current_pos: start_pos,
                            modifiers,
                            snap_result: start_snap.has_snap().then_some(start_snap),
                        };
                        self.needs_redraw = true;
                    }
                    Tool::Frame => {
                        let targets = self.document.all_snap_points(&[]);
                        self.snap_engine.set_targets(targets);
                        let start_snap = self.snap_engine.find_snap(
                            &creation_snap_points(world_pos),
                            Vec2::ZERO,
                            self.view.zoom,
                        );
                        let start_pos = world_pos + start_snap.position;
                        self.drag = DragState::CreatingFrame {
                            start_pos,
                            current_pos: start_pos,
                            modifiers,
                            snap_result: start_snap.has_snap().then_some(start_snap),
                        };
                        self.needs_redraw = true;
                    }
                    Tool::Pen => {
                        let parent = self.deepest_frame_at(world_pos);
                        self.drag = DragState::Pen(PenSession {
                            parent,
                            anchors: Vec::new(),
                            redo: Vec::new(),
                            hover: world_pos,
                            active_anchor: None,
                            pointer_down: false,
                        });
                        self.pen_pointer_down(world_pos, modifiers);
                    }
                    Tool::Text => {
                        self.drag = DragState::CreatingText {
                            start_pos: world_pos,
                            current_pos: world_pos,
                        };
                        self.needs_redraw = true;
                    }
                    Tool::VectorEdit => {
                        self.enter_vector_edit();
                    }
                }
            }
            MouseButton::Middle => {
                // Start panning
                self.drag = DragState::Panning {
                    start_pos: screen_pos,
                    start_pan: self.view.pan,
                    resume: None,
                };
            }
            MouseButton::Right => {
                // Could show context menu
            }
        }

        Effects {
            redraw: self.needs_redraw,
            cursor: Some(self.cursor_for_state()),
        }
    }

    fn handle_pointer_move(&mut self, screen_pos: Vec2, modifiers: Modifiers) -> Effects {
        let world_pos = self.view.to_world(screen_pos);

        if matches!(self.drag, DragState::Pen(_)) {
            self.pen_pointer_move(world_pos, modifiers);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        if matches!(self.drag, DragState::VectorEditing(_)) {
            self.vector_pointer_move(world_pos, modifiers);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        if matches!(
            self.drag,
            DragState::TextEditing(TextEditSession {
                pointer_selection_anchor: Some(_),
                ..
            })
        ) {
            self.text_pointer_move(world_pos);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        if matches!(self.drag, DragState::Resizing(_)) {
            self.update_resize(world_pos, modifiers);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        if matches!(self.drag, DragState::Rotating(_)) {
            self.update_rotation(world_pos, modifiers);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }

        match &mut self.drag {
            DragState::None => {
                // Update cursor based on hover
            }
            DragState::Resizing(_) | DragState::Rotating(_) => {}
            DragState::Moving {
                start_pos,
                transaction,
                source_snap_points,
                snap_result,
            } => {
                // Calculate raw delta
                let raw_delta = world_pos - *start_pos;

                // Try to snap the source points to targets
                let result =
                    self.snap_engine
                        .find_snap(source_snap_points, raw_delta, self.view.zoom);

                // Use snapped position if available
                let final_delta = result.position;
                transaction.translate(final_delta, &mut self.document);

                // Store snap result for rendering guides
                *snap_result = Some(result);

                self.needs_redraw = true;
            }
            DragState::Marquee { current_pos, .. } => {
                *current_pos = world_pos;
                self.needs_redraw = true;
            }
            DragState::CreatingShape {
                shape,
                start_pos,
                current_pos,
                modifiers: current_modifiers,
                snap_result,
            } => {
                let raw_delta = world_pos - *start_pos;
                let source_points = creation_snap_points(*start_pos);
                let mut result =
                    self.snap_engine
                        .find_snap(&source_points, raw_delta, self.view.zoom);
                let snapped_pos = *start_pos + result.position;
                let constrained_pos = if *shape == ShapeKind::Line {
                    constrained_line_endpoint(*start_pos, snapped_pos, modifiers.shift)
                } else {
                    creation_endpoint(*start_pos, snapped_pos, modifiers.shift)
                };

                if (constrained_pos.x - snapped_pos.x).abs() > f32::EPSILON {
                    result.snap_x = None;
                }
                if (constrained_pos.y - snapped_pos.y).abs() > f32::EPSILON {
                    result.snap_y = None;
                }

                *current_pos = snapped_pos;
                *current_modifiers = modifiers;
                *snap_result = Some(result);
                self.needs_redraw = true;
            }
            DragState::CreatingFrame {
                start_pos,
                current_pos,
                modifiers: current_modifiers,
                snap_result,
            } => {
                let raw_delta = world_pos - *start_pos;
                let source_points = creation_snap_points(*start_pos);
                let result = self
                    .snap_engine
                    .find_snap(&source_points, raw_delta, self.view.zoom);
                *current_pos = *start_pos + result.position;
                *current_modifiers = modifiers;
                *snap_result = Some(result);
                self.needs_redraw = true;
            }
            DragState::CreatingText { current_pos, .. } => {
                *current_pos = world_pos;
                self.needs_redraw = true;
            }
            DragState::TextEditing(_) | DragState::VectorEditing(_) | DragState::Pen(_) => {}
            DragState::Panning {
                start_pos,
                start_pan,
                ..
            } => {
                let delta = screen_pos - *start_pos;
                self.view.pan = *start_pan + delta;
                self.needs_redraw = true;
            }
        }

        Effects {
            redraw: self.needs_redraw,
            cursor: Some(self.cursor_for_state()),
        }
    }

    fn handle_pointer_up(
        &mut self,
        screen_pos: Vec2,
        button: MouseButton,
        modifiers: Modifiers,
    ) -> Effects {
        let world_pos = self.view.to_world(screen_pos);
        if matches!(self.drag, DragState::Pen(_)) && button == MouseButton::Left {
            self.pen_pointer_up(world_pos);
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        if matches!(self.drag, DragState::VectorEditing(_)) && button == MouseButton::Left {
            self.vector_pointer_up();
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        if let DragState::TextEditing(session) = &mut self.drag {
            if button == MouseButton::Left && session.pointer_selection_anchor.take().is_some() {
                self.needs_redraw = true;
                return Effects {
                    redraw: true,
                    cursor: Some(self.cursor_for_state()),
                };
            }
        }

        let completes_drag = matches!(
            (&self.drag, button),
            (
                DragState::Moving { .. }
                    | DragState::Resizing(_)
                    | DragState::Rotating(_)
                    | DragState::Marquee { .. }
                    | DragState::CreatingShape { .. }
                    | DragState::CreatingFrame { .. }
                    | DragState::CreatingText { .. },
                MouseButton::Left
            ) | (
                DragState::Panning { .. },
                MouseButton::Left | MouseButton::Middle
            )
        );

        if !completes_drag {
            return Effects {
                redraw: false,
                cursor: Some(self.cursor_for_state()),
            };
        }

        // Platforms are not required to emit a final move event immediately
        // before pointer-up. Apply the release position so a quick drag does
        // not lose its last delta.
        self.handle_pointer_move(screen_pos, modifiers);

        match std::mem::take(&mut self.drag) {
            DragState::None => {}
            DragState::Moving { transaction, .. } => {
                // Clear snap targets when done dragging
                self.snap_engine.clear_targets();
                let cmd = transaction.commit(&self.document);
                if !cmd.is_empty() {
                    self.history.push(cmd);
                }
            }
            DragState::Resizing(session) | DragState::Rotating(session) => {
                self.commit_transform_session(session);
            }
            DragState::Marquee {
                start_pos,
                current_pos,
            } => {
                // Check if this was a click (minimal drag) vs actual marquee drag
                let drag_distance = (current_pos - start_pos).length() * self.view.zoom.abs();
                const MIN_MARQUEE_SIZE: f32 = 3.0;

                if drag_distance < MIN_MARQUEE_SIZE {
                    // Just a click on empty space - clear selection (unless Shift held)
                    if !modifiers.shift {
                        self.selection.clear();
                    }
                } else {
                    // Actual marquee selection
                    // Ctrl/Cmd enables deep select (bypasses groups, selects children)
                    let deep_select = modifiers.ctrl || modifiers.meta;
                    let selected =
                        self.compute_marquee_selection(start_pos, current_pos, deep_select);

                    if modifiers.shift {
                        // Selection depth and additive selection are independent.
                        for id in selected {
                            self.add_selection_target(id);
                        }
                    } else {
                        // No modifiers: Replace selection
                        self.selection.set(selected);
                    }
                }
                self.needs_redraw = true;
            }
            DragState::CreatingShape {
                shape,
                start_pos,
                current_pos,
                modifiers,
                ..
            } => {
                self.snap_engine.clear_targets();
                let (bounds, path) = creation_geometry(shape, start_pos, current_pos, modifiers);
                const MIN_SHAPE_SIZE: f32 = 2.0;

                let large_enough = if shape == ShapeKind::Line {
                    path.contours
                        .first()
                        .and_then(|contour| {
                            let [start, end] = contour.anchors.as_slice() else {
                                return None;
                            };
                            Some(
                                start.position.distance(end.position) * self.view.zoom.abs()
                                    >= MIN_SHAPE_SIZE,
                            )
                        })
                        .unwrap_or(false)
                } else {
                    let screen_size = bounds.size() * self.view.zoom.abs();
                    screen_size.x >= MIN_SHAPE_SIZE && screen_size.y >= MIN_SHAPE_SIZE
                };
                if large_enough {
                    self.create_shape(shape, bounds, path, start_pos);
                }
            }
            DragState::CreatingFrame {
                start_pos,
                current_pos,
                modifiers,
                ..
            } => {
                self.snap_engine.clear_targets();
                let bounds = creation_bounds(start_pos, current_pos, modifiers);
                let screen_size = bounds.size() * self.view.zoom.abs();
                if screen_size.x >= 2.0 && screen_size.y >= 2.0 {
                    self.create_frame(bounds, start_pos);
                }
            }
            DragState::CreatingText {
                start_pos,
                current_pos,
            } => {
                self.begin_text_edit(start_pos, current_pos);
            }
            DragState::Pen(_) | DragState::TextEditing(_) | DragState::VectorEditing(_) => {}
            DragState::Panning { resume, .. } => {
                if let Some(resume) = resume {
                    self.drag = *resume;
                }
            }
        }

        self.needs_redraw = true;
        Effects {
            redraw: true,
            cursor: Some(self.cursor_for_state()),
        }
    }

    /// Compute which nodes are within the marquee rectangle.
    ///
    /// The default marquee selects direct children of the active selection
    /// scope. Deep-select marquee traverses the scope and selects editable
    /// leaves, never a container together with its descendants.
    fn compute_marquee_selection(
        &mut self,
        start: Vec2,
        end: Vec2,
        deep_select: bool,
    ) -> Vec<NodeId> {
        use crate::path::Rect;

        // Create the marquee rectangle
        let min_x = start.x.min(end.x);
        let min_y = start.y.min(end.y);
        let max_x = start.x.max(end.x);
        let max_y = start.y.max(end.y);
        let marquee = Rect::new(Vec2::new(min_x, min_y), Vec2::new(max_x, max_y));

        let active_scope = self.selection_scope();
        let scope = if active_scope == self.document.root
            || self
                .document
                .world_bounds(active_scope)
                .is_some_and(|bounds| bounds.contains(start))
        {
            active_scope
        } else {
            self.document.root
        };
        let candidates = if deep_select {
            self.document.descendants(scope).collect::<Vec<_>>()
        } else {
            self.document
                .get(scope)
                .map(|node| node.children.clone())
                .unwrap_or_default()
        };

        let mut selected = Vec::new();
        for id in candidates {
            if !self.document.is_effectively_editable(id) {
                continue;
            }
            let is_container = self
                .document
                .get(id)
                .is_some_and(|node| !node.children.is_empty());
            if deep_select && is_container {
                continue;
            }
            if self
                .document
                .world_bounds(id)
                .is_some_and(|bounds| marquee.intersects(&bounds))
            {
                selected.push(id);
            }
        }

        selected
    }

    fn handle_scroll(&mut self, screen_pos: Vec2, delta: Vec2, modifiers: Modifiers) -> Effects {
        if (modifiers.ctrl || modifiers.meta) && delta.y.abs() <= f32::EPSILON {
            return Effects::default();
        }

        let interaction_cancelled = self.cancel_pointer_interaction();
        if modifiers.ctrl || modifiers.meta {
            // Zoom
            let zoom_factor = if delta.y > 0.0 { 0.9 } else { 1.1 };
            self.view.zoom_at(zoom_factor, screen_pos);
        } else {
            // Pan
            self.view.pan -= delta;
        }
        self.needs_redraw = true;

        Effects {
            redraw: true,
            cursor: interaction_cancelled.then(|| self.cursor_for_state()),
        }
    }

    fn handle_key_down(&mut self, key: Key, modifiers: Modifiers) -> Effects {
        if key == Key::Space && !modifiers.any() && !matches!(self.drag, DragState::TextEditing(_))
        {
            self.space_pan_active = true;
            self.needs_redraw = true;
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }

        if matches!(self.drag, DragState::Pen(_)) {
            match key {
                Key::Escape | Key::Enter if !modifiers.any() => {
                    self.finish_editing();
                }
                Key::Backspace if !modifiers.any() => {
                    self.undo_pen_edit();
                }
                Key::Z if modifiers.primary() => {
                    if modifiers.shift {
                        self.redo_pen_edit();
                    } else {
                        self.undo_pen_edit();
                    }
                }
                _ => {}
            }
            return Effects {
                redraw: self.needs_redraw,
                cursor: Some(self.cursor_for_state()),
            };
        }

        if matches!(self.drag, DragState::TextEditing(_)) {
            if key == Key::Escape && !modifiers.any() {
                self.cancel_interaction();
            } else if key == Key::Enter && modifiers.primary() {
                self.finish_editing();
            } else if key == Key::Z && modifiers.primary() {
                if modifiers.shift {
                    self.redo_text_edit();
                } else {
                    self.undo_text_edit();
                }
            }
            return Effects {
                redraw: self.needs_redraw,
                cursor: Some(self.cursor_for_state()),
            };
        }

        if matches!(self.drag, DragState::VectorEditing(_)) {
            if matches!(key, Key::Escape | Key::Enter) && !modifiers.any() {
                self.finish_editing();
            } else if key == Key::Z && modifiers.primary() {
                if modifiers.shift {
                    self.redo_vector_edit();
                } else {
                    self.undo_vector_edit();
                }
            } else if matches!(key, Key::Backspace | Key::Delete) && !modifiers.any() {
                self.delete_selected_vector_anchors();
            }
            return Effects {
                redraw: self.needs_redraw,
                cursor: Some(self.cursor_for_state()),
            };
        }

        if key == Key::Enter && !modifiers.primary() && !modifiers.alt {
            if modifiers.shift {
                self.select_parent();
            } else {
                self.enter_selection();
            }
            return Effects {
                redraw: self.needs_redraw,
                cursor: Some(self.cursor_for_state()),
            };
        }

        if key == Key::Escape && !modifiers.any() {
            if self.cancel_interaction() {
                return Effects {
                    redraw: true,
                    cursor: Some(self.cursor_for_state()),
                };
            }
            if self.tool != Tool::Select {
                self.set_tool(Tool::Select);
                return Effects {
                    redraw: true,
                    cursor: Some(self.cursor_for_state()),
                };
            }
        }

        // Use the action system to handle keyboard shortcuts
        if let Some(action) = self.action_for_key(key, modifiers) {
            self.execute_action(action);
        }

        Effects {
            redraw: self.needs_redraw,
            cursor: Some(self.cursor_for_state()),
        }
    }

    fn handle_key_up(&mut self, key: Key, _modifiers: Modifiers) -> Effects {
        if key == Key::Space {
            self.space_pan_active = false;
            self.needs_redraw = true;
            return Effects {
                redraw: true,
                cursor: Some(self.cursor_for_state()),
            };
        }
        Effects::default()
    }

    fn handle_modifiers_changed(&mut self, modifiers: Modifiers) -> Effects {
        let mut redraw = false;
        if let DragState::CreatingShape {
            modifiers: active_modifiers,
            snap_result,
            ..
        } = &mut self.drag
        {
            *active_modifiers = modifiers;
            *snap_result = None;
            self.needs_redraw = true;
            redraw = true;
        }
        if let DragState::CreatingFrame {
            modifiers: active_modifiers,
            snap_result,
            ..
        } = &mut self.drag
        {
            *active_modifiers = modifiers;
            *snap_result = None;
            self.needs_redraw = true;
            redraw = true;
        }

        Effects {
            redraw,
            cursor: Some(self.cursor_for_state()),
        }
    }

    fn create_shape(
        &mut self,
        shape: ShapeKind,
        bounds: Rect,
        path: PathData,
        creation_point: Vec2,
    ) {
        let parent = self.deepest_frame_at(creation_point);
        let parent_world = self.document.world_transform(parent);
        let style = match shape {
            ShapeKind::Rectangle | ShapeKind::Ellipse => self.creation_styles.closed_shapes.clone(),
            ShapeKind::Line => self.creation_styles.paths.clone(),
        };
        let node = Node::shape(shape.name(), path)
            .with_style(style)
            .with_transform(parent_world.inverse() * Affine2::from_translation(bounds.min));
        let index = self
            .document
            .get(parent)
            .map(|node| node.children.len())
            .unwrap_or_default();
        let id = self.document.nodes.insert(node.clone());
        let command =
            Command::new(format!("Create {}", shape.name())).with_patch(Patch::CreateNode {
                id,
                node: Box::new(node),
                parent,
                index,
            });

        command.apply(&mut self.document);
        self.history.push(command);
        self.selection.select(id);
        self.needs_redraw = true;
    }

    fn deepest_frame_at(&mut self, world_position: Vec2) -> NodeId {
        let ids = self.document.paint_order().collect::<Vec<_>>();
        let mut best = self.document.root;
        let mut best_depth = 0usize;

        for id in ids {
            let Some((size, visible, locked)) = self.document.get(id).and_then(|node| {
                let NodeKind::Frame(frame) = &node.kind else {
                    return None;
                };
                Some((
                    Vec2::new(frame.width, frame.height),
                    node.visible,
                    node.locked,
                ))
            }) else {
                continue;
            };
            if !visible || locked || !self.document.is_effectively_editable(id) {
                continue;
            }
            let depth = self.document.ancestors(id).count();
            let world = self.document.world_transform(id);
            let determinant = world.matrix2.determinant();
            if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
                continue;
            }
            if Rect::from_pos_size(Vec2::ZERO, size)
                .contains(world.inverse().transform_point2(world_position))
                && depth >= best_depth
            {
                best = id;
                best_depth = depth;
            }
        }
        best
    }

    fn create_frame(&mut self, bounds: Rect, creation_point: Vec2) {
        let parent = self.deepest_frame_at(creation_point);
        let parent_world = self.document.world_transform(parent);
        let frame_world = Affine2::from_translation(bounds.min);
        let frame = Node::frame("Frame", bounds.width(), bounds.height())
            .with_transform(parent_world.inverse() * frame_world);

        let siblings = self
            .document
            .get(parent)
            .map(|node| node.children.clone())
            .unwrap_or_default();
        let mut enclosed = Vec::new();
        for (index, id) in siblings.iter().copied().enumerate() {
            let Some((locked, deleted, visible, transform)) = self
                .document
                .get(id)
                .map(|node| (node.locked, node.deleted, node.visible, node.transform))
            else {
                continue;
            };
            if locked || deleted || !visible {
                continue;
            }
            if let Some(node_bounds) = self.document.world_bounds(id) {
                if bounds.contains(node_bounds.min) && bounds.contains(node_bounds.max) {
                    enclosed.push((index, id, transform, self.document.world_transform(id)));
                }
            }
        }

        let frame_index = enclosed
            .first()
            .map(|(index, ..)| *index)
            .unwrap_or(siblings.len());
        let frame_id = self.document.nodes.insert(frame.clone());
        let mut patches = vec![Patch::CreateNode {
            id: frame_id,
            node: Box::new(frame),
            parent,
            index: frame_index,
        }];

        for (after_index, (before_index, id, before, child_world)) in
            enclosed.into_iter().enumerate()
        {
            patches.push(Patch::Reparent {
                id,
                before_parent: parent,
                after_parent: frame_id,
                before_index,
                after_index,
            });
            patches.push(Patch::SetTransform {
                id,
                before,
                after: frame_world.inverse() * child_world,
            });
        }

        let command = Command::new("Create Frame").with_patches(patches);
        command.apply(&mut self.document);
        self.history.push(command);
        self.selection.select(frame_id);
        self.needs_redraw = true;
    }

    fn begin_text_edit(&mut self, start: Vec2, end: Vec2) {
        let drag_distance = (end - start).length() * self.view.zoom.abs();
        let parent = self.deepest_frame_at(start);
        let parent_world = self.document.world_transform(parent);
        let mut text = TextData::default();
        let origin = if drag_distance >= 3.0 {
            let bounds = Rect::new(start.min(end), start.max(end));
            text.sizing = TextSizing::FixedWidth {
                width: bounds.width().max(1.0),
            };
            bounds.min
        } else {
            start
        };
        let node = Node::text("Text", "")
            .with_style(Style::fill(Paint::rgb(0.12, 0.13, 0.15)))
            .with_transform(parent_world.inverse() * Affine2::from_translation(origin));
        let mut node = node;
        node.kind = NodeKind::Text(text);
        node.parent = Some(parent);
        let index = self
            .document
            .get(parent)
            .map(|node| node.children.len())
            .unwrap_or_default();
        let id = self.document.nodes.insert(node);
        if let Some(parent_node) = self.document.get_mut(parent) {
            parent_node.children.push(id);
        }
        self.document.mark_transform_dirty(id);
        self.selection.select(id);
        self.drag = DragState::TextEditing(TextEditSession {
            node: id,
            before: None,
            selection: 0..0,
            marked: None,
            selection_reversed: false,
            parent,
            index,
            undo: Vec::new(),
            redo: Vec::new(),
            pointer_selection_anchor: None,
        });
        self.needs_redraw = true;
    }

    fn pen_pointer_down(&mut self, world_position: Vec2, modifiers: Modifiers) {
        let close_tolerance = 8.0 / self.view.zoom.abs().max(f32::EPSILON);
        let should_close = match &self.drag {
            DragState::Pen(session) => {
                session.anchors.len() >= 3
                    && session.anchors.first().is_some_and(|anchor| {
                        anchor.position.distance(world_position) <= close_tolerance
                    })
            }
            _ => false,
        };
        if should_close {
            let DragState::Pen(session) = std::mem::take(&mut self.drag) else {
                return;
            };
            self.commit_pen_session(session, true);
            self.needs_redraw = true;
            return;
        }

        let DragState::Pen(session) = &mut self.drag else {
            return;
        };
        session.redo.clear();
        session.anchors.push(PathAnchor::corner(world_position));
        session.active_anchor = Some(session.anchors.len() - 1);
        session.pointer_down = true;
        session.hover = world_position;
        if modifiers.alt {
            if let Some(anchor) = session.anchors.last_mut() {
                anchor.mode = HandleMode::Independent;
            }
        }
        self.needs_redraw = true;
    }

    fn pen_pointer_move(&mut self, world_position: Vec2, modifiers: Modifiers) {
        let DragState::Pen(session) = &mut self.drag else {
            return;
        };
        session.hover = world_position;
        if session.pointer_down {
            if let Some(index) = session.active_anchor {
                let anchor = &mut session.anchors[index];
                let mut handle = world_position - anchor.position;
                if modifiers.shift && handle.length_squared() > f32::EPSILON {
                    let length = handle.length();
                    let angle = (handle.y.atan2(handle.x) / std::f32::consts::FRAC_PI_4).round()
                        * std::f32::consts::FRAC_PI_4;
                    handle = Vec2::new(angle.cos(), angle.sin()) * length;
                }
                if handle.length() > 1.0 / self.view.zoom.abs().max(f32::EPSILON) {
                    anchor.out_handle = Some(handle);
                    anchor.in_handle = (!modifiers.alt).then_some(-handle);
                    anchor.mode = if modifiers.alt {
                        HandleMode::Independent
                    } else {
                        HandleMode::Mirrored
                    };
                }
            }
        }
        self.needs_redraw = true;
    }

    fn pen_pointer_up(&mut self, world_position: Vec2) {
        if let DragState::Pen(session) = &mut self.drag {
            session.hover = world_position;
            session.pointer_down = false;
            session.active_anchor = None;
            self.needs_redraw = true;
        }
    }

    /// Dispatch a pointer-down while preserving the platform click count.
    pub fn handle_pointer_down_with_click_count(
        &mut self,
        position: Vec2,
        button: MouseButton,
        modifiers: Modifiers,
        click_count: usize,
    ) -> Effects {
        if click_count >= 2 && button == MouseButton::Left {
            if matches!(self.drag, DragState::VectorEditing(_)) {
                let world = self.view.to_world(position);
                if !self.insert_vector_anchor(world) {
                    self.vector_pointer_down(world, modifiers);
                }
                return Effects {
                    redraw: true,
                    cursor: Some(self.cursor_for_state()),
                };
            }
            if matches!(self.drag, DragState::Pen(_)) {
                self.finish_editing();
                return Effects {
                    redraw: true,
                    cursor: Some(self.cursor_for_state()),
                };
            }
            let world = self.view.to_world(position);
            if let Some(id) = self.document.hit_test_with_tolerance(
                world,
                false,
                4.0 / self.view.zoom.abs().max(f32::EPSILON),
            ) {
                if self.select_one_level_toward(id) {
                    return Effects {
                        redraw: true,
                        cursor: Some(self.cursor_for_state()),
                    };
                }
                let hit_is_selected =
                    self.selection.len() == 1 && self.selection.primary() == Some(id);
                if self
                    .document
                    .get(id)
                    .is_some_and(|node| matches!(node.kind, NodeKind::Shape(_)))
                    && hit_is_selected
                {
                    self.enter_vector_edit();
                    return Effects {
                        redraw: true,
                        cursor: Some(self.cursor_for_state()),
                    };
                }
                if self
                    .document
                    .get(id)
                    .is_some_and(|node| matches!(node.kind, NodeKind::Text(_)))
                    && hit_is_selected
                {
                    self.begin_existing_text_edit(id);
                    return Effects {
                        redraw: true,
                        cursor: Some(self.cursor_for_state()),
                    };
                }
            }
        }

        self.handle_pointer_down(position, button, modifiers)
    }

    fn begin_existing_text_edit(&mut self, id: NodeId) {
        self.finish_editing();
        let Some((before, parent, index)) = self.document.get(id).and_then(|node| {
            let NodeKind::Text(text) = &node.kind else {
                return None;
            };
            let parent = node.parent?;
            let index = self
                .document
                .get(parent)?
                .children
                .iter()
                .position(|child| *child == id)?;
            Some((text.clone(), parent, index))
        }) else {
            return;
        };
        let caret = before.content.len();
        self.selection.select(id);
        self.tool = Tool::Text;
        self.drag = DragState::TextEditing(TextEditSession {
            node: id,
            before: Some(before),
            selection: caret..caret,
            marked: None,
            selection_reversed: false,
            parent,
            index,
            undo: Vec::new(),
            redo: Vec::new(),
            pointer_selection_anchor: None,
        });
        self.needs_redraw = true;
    }

    fn text_pointer_down(&mut self, world_position: Vec2, modifiers: Modifiers) {
        let (node_id, world) = match &self.drag {
            DragState::TextEditing(session) => {
                (session.node, self.document.world_transform(session.node))
            }
            _ => return,
        };
        let determinant = world.matrix2.determinant();
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return;
        }
        let local = world.inverse().transform_point2(world_position);
        if self
            .document
            .local_bounds(node_id)
            .is_some_and(|bounds| bounds.contains(local))
        {
            let index = self.estimated_text_index(node_id, local);
            if let DragState::TextEditing(session) = &mut self.drag {
                let anchor = if modifiers.shift {
                    if session.selection_reversed {
                        session.selection.end
                    } else {
                        session.selection.start
                    }
                } else {
                    index
                };
                session.selection = anchor.min(index)..anchor.max(index);
                session.selection_reversed = index < anchor;
                session.marked = None;
                session.pointer_selection_anchor = Some(anchor);
            }
        } else {
            self.finish_editing();
            if self.tool == Tool::Text {
                self.drag = DragState::CreatingText {
                    start_pos: world_position,
                    current_pos: world_position,
                };
            }
        }
        self.needs_redraw = true;
    }

    fn text_pointer_move(&mut self, world_position: Vec2) {
        let (node, anchor, world) = match &self.drag {
            DragState::TextEditing(session) => {
                let Some(anchor) = session.pointer_selection_anchor else {
                    return;
                };
                (
                    session.node,
                    anchor,
                    self.document.world_transform(session.node),
                )
            }
            _ => return,
        };
        let determinant = world.matrix2.determinant();
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return;
        }
        let index =
            self.estimated_text_index(node, world.inverse().transform_point2(world_position));
        if let DragState::TextEditing(session) = &mut self.drag {
            session.selection = anchor.min(index)..anchor.max(index);
            session.selection_reversed = index < anchor;
            session.marked = None;
        }
        self.needs_redraw = true;
    }

    fn estimated_text_index(&self, id: NodeId, local: Vec2) -> usize {
        let Some(text) = self.document.get(id).and_then(|node| match &node.kind {
            NodeKind::Text(text) => Some(text),
            _ => None,
        }) else {
            return 0;
        };
        self.document
            .text_layout(id)
            .map_or(0, |layout| layout.byte_for_position(&text.content, local))
    }

    /// Snapshot the active semantic text edit for a native input handler.
    pub fn text_input_snapshot(&self) -> Option<TextInputSnapshot> {
        let DragState::TextEditing(session) = &self.drag else {
            return None;
        };
        let text = self
            .document
            .get(session.node)
            .and_then(|node| match &node.kind {
                NodeKind::Text(text) => Some(text.content.clone()),
                _ => None,
            })?;
        Some(TextInputSnapshot {
            node: session.node,
            text,
            selection: session.selection.clone(),
            marked: session.marked.clone(),
            selection_reversed: session.selection_reversed,
        })
    }

    /// Replace UTF-8 text in the active edit session.
    pub fn replace_text(&mut self, range: Option<Range<usize>>, replacement: &str) -> bool {
        let (node_id, fallback_range) = match &self.drag {
            DragState::TextEditing(session) => (
                session.node,
                session
                    .marked
                    .clone()
                    .unwrap_or_else(|| session.selection.clone()),
            ),
            _ => return false,
        };
        let Some(text_data) = self
            .document
            .get(node_id)
            .and_then(|node| match &node.kind {
                NodeKind::Text(text) => Some(text.clone()),
                _ => None,
            })
        else {
            return false;
        };
        let text = text_data.content.clone();
        let requested = range.unwrap_or(fallback_range);
        let start = floor_char_boundary(&text, requested.start.min(text.len()));
        let end = ceil_char_boundary(&text, requested.end.min(text.len())).max(start);
        let before_edit = match &self.drag {
            DragState::TextEditing(session) => TextEditSnapshot {
                text: text_data,
                selection: session.selection.clone(),
                marked: session.marked.clone(),
                selection_reversed: session.selection_reversed,
            },
            _ => return false,
        };
        let mut updated = text;
        updated.replace_range(start..end, replacement);
        if updated == before_edit.text.content {
            let caret = start + replacement.len();
            if let DragState::TextEditing(session) = &mut self.drag {
                session.selection = caret..caret;
                session.marked = None;
                session.selection_reversed = false;
            }
            self.needs_redraw = true;
            return true;
        }
        let caret = start + replacement.len();
        if let Some(node) = self.document.get_mut(node_id) {
            if let NodeKind::Text(text) = &mut node.kind {
                text.content = updated;
            }
        }
        if let DragState::TextEditing(session) = &mut self.drag {
            session.undo.push(before_edit);
            session.redo.clear();
            session.selection = caret..caret;
            session.marked = None;
            session.selection_reversed = false;
        }
        self.document.mark_bounds_dirty(node_id);
        self.needs_redraw = true;
        true
    }

    /// Undo one mutation inside the active text session without touching document history.
    pub fn undo_text_edit(&mut self) -> bool {
        self.restore_text_edit_snapshot(false)
    }

    /// Whether the active text session has a local mutation to undo.
    pub fn can_undo_text_edit(&self) -> bool {
        matches!(&self.drag, DragState::TextEditing(session) if !session.undo.is_empty())
    }

    /// Redo one mutation inside the active text session without touching document history.
    pub fn redo_text_edit(&mut self) -> bool {
        self.restore_text_edit_snapshot(true)
    }

    /// Whether the active text session has a local mutation to redo.
    pub fn can_redo_text_edit(&self) -> bool {
        matches!(&self.drag, DragState::TextEditing(session) if !session.redo.is_empty())
    }

    /// Undo one Pen anchor without committing or leaving the active path session.
    pub fn undo_pen_edit(&mut self) -> bool {
        let DragState::Pen(session) = &mut self.drag else {
            return false;
        };
        session.pointer_down = false;
        session.active_anchor = None;
        let Some(anchor) = session.anchors.pop() else {
            return false;
        };
        session.redo.push(anchor);
        self.needs_redraw = true;
        true
    }

    /// Whether the active Pen session has an anchor to undo.
    pub fn can_undo_pen_edit(&self) -> bool {
        matches!(&self.drag, DragState::Pen(session) if !session.anchors.is_empty())
    }

    /// Redo one locally undone Pen anchor.
    pub fn redo_pen_edit(&mut self) -> bool {
        let DragState::Pen(session) = &mut self.drag else {
            return false;
        };
        let Some(anchor) = session.redo.pop() else {
            return false;
        };
        session.anchors.push(anchor);
        self.needs_redraw = true;
        true
    }

    /// Whether the active Pen session has an anchor to redo.
    pub fn can_redo_pen_edit(&self) -> bool {
        matches!(&self.drag, DragState::Pen(session) if !session.redo.is_empty())
    }

    /// Whether Undo has a meaningful target in the current editing context.
    pub fn can_undo_in_context(&self) -> bool {
        if self.numeric_property_scrub.is_some() {
            return true;
        }
        match &self.drag {
            DragState::Pen(_) => self.can_undo_pen_edit(),
            DragState::TextEditing(_) => self.can_undo_text_edit(),
            DragState::VectorEditing(session) => !session.undo.is_empty(),
            DragState::None => self.history.can_undo(),
            DragState::Moving { .. }
            | DragState::Resizing(_)
            | DragState::Rotating(_)
            | DragState::Marquee { .. }
            | DragState::CreatingShape { .. }
            | DragState::CreatingFrame { .. }
            | DragState::CreatingText { .. }
            | DragState::Panning { .. } => true,
        }
    }

    /// Undo within an active semantic edit before falling back to document history.
    pub fn undo_in_context(&mut self) -> bool {
        if self.cancel_active_numeric_property_scrub() {
            return true;
        }
        match self.interaction_kind() {
            InteractionKind::Pen => self.undo_pen_edit(),
            InteractionKind::TextEditing => self.undo_text_edit(),
            InteractionKind::VectorEditing => self.undo_vector_edit(),
            InteractionKind::Idle => self.undo_document(),
            InteractionKind::Moving
            | InteractionKind::Resizing
            | InteractionKind::Rotating
            | InteractionKind::Marquee
            | InteractionKind::CreatingShape
            | InteractionKind::CreatingFrame
            | InteractionKind::CreatingText => self.cancel_interaction(),
            InteractionKind::Panning => {
                let resumes_edit = matches!(
                    &self.drag,
                    DragState::Panning {
                        resume: Some(_),
                        ..
                    }
                );
                self.cancel_interaction();
                if resumes_edit {
                    self.undo_in_context()
                } else {
                    true
                }
            }
        }
    }

    /// Whether Redo has a meaningful target in the current editing context.
    pub fn can_redo_in_context(&self) -> bool {
        if self.numeric_property_scrub.is_some() {
            return true;
        }
        match &self.drag {
            DragState::Pen(_) => self.can_redo_pen_edit(),
            DragState::TextEditing(_) => self.can_redo_text_edit(),
            DragState::VectorEditing(session) => !session.redo.is_empty(),
            DragState::None => self.history.can_redo(),
            DragState::Panning { resume, .. } => {
                resume.as_deref().is_some_and(|resume| match resume {
                    DragState::Pen(session) => !session.redo.is_empty(),
                    DragState::TextEditing(session) => !session.redo.is_empty(),
                    DragState::VectorEditing(session) => !session.redo.is_empty(),
                    DragState::None => self.history.can_redo(),
                    _ => false,
                })
            }
            DragState::Moving { .. }
            | DragState::Resizing(_)
            | DragState::Rotating(_)
            | DragState::Marquee { .. }
            | DragState::CreatingShape { .. }
            | DragState::CreatingFrame { .. }
            | DragState::CreatingText { .. } => false,
        }
    }

    /// Redo within an active semantic edit before falling back to document history.
    pub fn redo_in_context(&mut self) -> bool {
        if self.cancel_active_numeric_property_scrub() {
            return true;
        }
        match self.interaction_kind() {
            InteractionKind::Pen => self.redo_pen_edit(),
            InteractionKind::TextEditing => self.redo_text_edit(),
            InteractionKind::VectorEditing => self.redo_vector_edit(),
            InteractionKind::Idle => self.redo_document(),
            InteractionKind::Panning => {
                self.cancel_interaction();
                self.redo_in_context()
            }
            InteractionKind::Moving
            | InteractionKind::Resizing
            | InteractionKind::Rotating
            | InteractionKind::Marquee
            | InteractionKind::CreatingShape
            | InteractionKind::CreatingFrame
            | InteractionKind::CreatingText => false,
        }
    }

    fn undo_document(&mut self) -> bool {
        if !self.history.undo(&mut self.document) {
            return false;
        }
        self.prune_selection();
        self.needs_redraw = true;
        true
    }

    fn redo_document(&mut self) -> bool {
        if !self.history.redo(&mut self.document) {
            return false;
        }
        self.prune_selection();
        self.needs_redraw = true;
        true
    }

    fn restore_text_edit_snapshot(&mut self, redo: bool) -> bool {
        let (node, target, current) = match &mut self.drag {
            DragState::TextEditing(session) => {
                let target = if redo {
                    session.redo.pop()
                } else {
                    session.undo.pop()
                };
                let Some(target) = target else {
                    return false;
                };
                let Some(text) = self.document.get(session.node).and_then(|node| {
                    let NodeKind::Text(text) = &node.kind else {
                        return None;
                    };
                    Some(text.clone())
                }) else {
                    return false;
                };
                let current = TextEditSnapshot {
                    text,
                    selection: session.selection.clone(),
                    marked: session.marked.clone(),
                    selection_reversed: session.selection_reversed,
                };
                (session.node, target, current)
            }
            _ => return false,
        };
        if let DragState::TextEditing(session) = &mut self.drag {
            if redo {
                session.undo.push(current);
            } else {
                session.redo.push(current);
            }
            session.selection = target.selection;
            session.marked = target.marked;
            session.selection_reversed = target.selection_reversed;
        }
        if let Some(node) = self.document.get_mut(node) {
            node.kind = NodeKind::Text(target.text);
        }
        self.document.mark_bounds_dirty(node);
        self.needs_redraw = true;
        true
    }

    /// Set the active UTF-8 selection after clamping to character boundaries.
    pub fn set_text_selection(&mut self, range: Range<usize>) -> bool {
        let Some(snapshot) = self.text_input_snapshot() else {
            return false;
        };
        let start = floor_char_boundary(&snapshot.text, range.start.min(snapshot.text.len()));
        let end = ceil_char_boundary(&snapshot.text, range.end.min(snapshot.text.len())).max(start);
        if let DragState::TextEditing(session) = &mut self.drag {
            session.selection = start..end;
            session.marked = None;
            session.selection_reversed = false;
        }
        self.needs_redraw = true;
        true
    }

    /// Move the active text caret by a grapheme or line boundary.
    pub fn navigate_text(&mut self, movement: TextNavigation, extend: bool) -> bool {
        let Some(snapshot) = self.text_input_snapshot() else {
            return false;
        };
        let cursor = if snapshot.selection_reversed {
            snapshot.selection.start
        } else {
            snapshot.selection.end
        };
        let target = match movement {
            TextNavigation::PreviousGrapheme => snapshot
                .text
                .grapheme_indices(true)
                .rev()
                .find_map(|(index, _)| (index < cursor).then_some(index))
                .unwrap_or(0),
            TextNavigation::NextGrapheme => snapshot
                .text
                .grapheme_indices(true)
                .find_map(|(index, _)| (index > cursor).then_some(index))
                .unwrap_or(snapshot.text.len()),
            TextNavigation::LineStart => snapshot.text[..cursor]
                .rfind('\n')
                .map_or(0, |index| index + 1),
            TextNavigation::LineEnd => snapshot.text[cursor..]
                .find('\n')
                .map_or(snapshot.text.len(), |index| cursor + index),
            TextNavigation::DocumentStart => 0,
            TextNavigation::DocumentEnd => snapshot.text.len(),
        };

        if let DragState::TextEditing(session) = &mut self.drag {
            if !extend {
                let collapse = if !session.selection.is_empty() {
                    match movement {
                        TextNavigation::PreviousGrapheme
                        | TextNavigation::LineStart
                        | TextNavigation::DocumentStart => session.selection.start,
                        TextNavigation::NextGrapheme
                        | TextNavigation::LineEnd
                        | TextNavigation::DocumentEnd => session.selection.end,
                    }
                } else {
                    target
                };
                session.selection = collapse..collapse;
                session.selection_reversed = false;
            } else {
                let anchor = if session.selection_reversed {
                    session.selection.end
                } else {
                    session.selection.start
                };
                if target < anchor {
                    session.selection = target..anchor;
                    session.selection_reversed = true;
                } else {
                    session.selection = anchor..target;
                    session.selection_reversed = false;
                }
            }
            session.marked = None;
        }
        self.needs_redraw = true;
        true
    }

    /// Delete one grapheme before the caret or the active selection.
    pub fn delete_text_backward(&mut self) -> bool {
        let Some(snapshot) = self.text_input_snapshot() else {
            return false;
        };
        let range = if snapshot.selection.is_empty() {
            let previous = snapshot
                .text
                .grapheme_indices(true)
                .rev()
                .find_map(|(index, _)| (index < snapshot.selection.start).then_some(index))
                .unwrap_or(0);
            previous..snapshot.selection.start
        } else {
            snapshot.selection
        };
        self.replace_text(Some(range), "")
    }

    /// Delete one grapheme after the caret or the active selection.
    pub fn delete_text_forward(&mut self) -> bool {
        let Some(snapshot) = self.text_input_snapshot() else {
            return false;
        };
        let range = if snapshot.selection.is_empty() {
            let next = snapshot
                .text
                .grapheme_indices(true)
                .find_map(|(index, _)| (index > snapshot.selection.end).then_some(index))
                .unwrap_or(snapshot.text.len());
            snapshot.selection.end..next
        } else {
            snapshot.selection
        };
        self.replace_text(Some(range), "")
    }

    /// Select all semantic text in the active edit session.
    pub fn select_all_text(&mut self) -> bool {
        let Some(snapshot) = self.text_input_snapshot() else {
            return false;
        };
        self.set_text_selection(0..snapshot.text.len())
    }

    /// Set or clear the active IME marked range in UTF-8 bytes.
    pub fn set_marked_text_range(&mut self, range: Option<Range<usize>>) -> bool {
        let Some(snapshot) = self.text_input_snapshot() else {
            return false;
        };
        let range = range.map(|range| {
            floor_char_boundary(&snapshot.text, range.start.min(snapshot.text.len()))
                ..ceil_char_boundary(&snapshot.text, range.end.min(snapshot.text.len()))
        });
        if let DragState::TextEditing(session) = &mut self.drag {
            session.marked = range;
        }
        self.needs_redraw = true;
        true
    }

    /// Approximate a UTF-8 text range in canvas screen coordinates.
    pub fn text_range_screen_bounds(&mut self, range: Range<usize>) -> Option<Rect> {
        let snapshot = self.text_input_snapshot()?;
        let text = self
            .document
            .get(snapshot.node)
            .and_then(|node| match &node.kind {
                NodeKind::Text(text) => Some(text.clone()),
                _ => None,
            })?;
        let world = self.document.world_transform(snapshot.node);
        let layout = self.document.text_layout(snapshot.node)?;
        let line_height = layout.line_height;
        let screen = self.view.screen_from_world() * world;
        let range = range.start.min(text.content.len())..range.end.min(text.content.len());
        let local_rects = estimated_text_range_rects(&layout, &text.content, range.clone());
        if local_rects.is_empty() {
            let caret = estimated_text_position(&layout, &text.content, range.start);
            return Some(crate::transform_rect(
                Rect::from_pos_size(caret, Vec2::new(1.0, line_height)),
                screen,
            ));
        }
        local_rects
            .into_iter()
            .map(|rect| crate::transform_rect(rect, screen))
            .reduce(|combined, rect| combined.union(&rect))
    }

    /// Map a canvas screen point to a UTF-8 text offset.
    pub fn text_index_at_screen_point(&mut self, screen_position: Vec2) -> Option<usize> {
        let node = match &self.drag {
            DragState::TextEditing(session) => session.node,
            _ => return None,
        };
        let world = self.document.world_transform(node);
        let determinant = world.matrix2.determinant();
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return None;
        }
        let local = world
            .inverse()
            .transform_point2(self.view.to_world(screen_position));
        Some(self.estimated_text_index(node, local))
    }

    fn enter_vector_edit(&mut self) {
        self.settle_interaction();
        let Some(node) = self.selection.primary() else {
            return;
        };
        let Some(path) = self
            .document
            .get(node)
            .and_then(|node| node.path())
            .cloned()
        else {
            return;
        };
        self.tool = Tool::VectorEdit;
        let mut before = HashMap::new();
        before.insert(node, path);
        self.drag = DragState::VectorEditing(VectorEditSession {
            node,
            before,
            selected: HashSet::new(),
            dragging: None,
            undo: Vec::new(),
            redo: Vec::new(),
        });
        self.needs_redraw = true;
    }

    fn vector_edit_snapshot(&self) -> Option<VectorEditSnapshot> {
        let DragState::VectorEditing(session) = &self.drag else {
            return None;
        };
        let paths = session
            .before
            .keys()
            .filter_map(|id| {
                self.document
                    .get(*id)
                    .and_then(|node| node.path())
                    .cloned()
                    .map(|path| (*id, path))
            })
            .collect();
        Some(VectorEditSnapshot {
            paths,
            selected: session.selected.clone(),
        })
    }

    fn record_vector_edit(&mut self, before: Option<VectorEditSnapshot>) {
        let Some(before) = before else {
            return;
        };
        let Some(after) = self.vector_edit_snapshot() else {
            return;
        };
        if before.paths == after.paths {
            return;
        }
        if let DragState::VectorEditing(session) = &mut self.drag {
            session.undo.push(before);
            session.redo.clear();
        }
    }

    fn restore_vector_edit_snapshot(&mut self, redo: bool) -> bool {
        let current = match self.vector_edit_snapshot() {
            Some(snapshot) => snapshot,
            None => return false,
        };
        let target = match &mut self.drag {
            DragState::VectorEditing(session) => {
                if redo {
                    session.redo.pop()
                } else {
                    session.undo.pop()
                }
            }
            _ => None,
        };
        let Some(target) = target else {
            return false;
        };
        for (id, path) in &target.paths {
            if let Some(node) = self.document.get_mut(*id) {
                if let Some(document_path) = node.path_mut() {
                    *document_path = path.clone();
                }
            }
            self.document.mark_bounds_dirty(*id);
        }
        if let DragState::VectorEditing(session) = &mut self.drag {
            if redo {
                session.undo.push(current);
            } else {
                session.redo.push(current);
            }
            session.selected = target.selected;
            session.dragging = None;
        }
        self.needs_redraw = true;
        true
    }

    /// Undo one mutation inside the active vector-edit session.
    pub fn undo_vector_edit(&mut self) -> bool {
        self.restore_vector_edit_snapshot(false)
    }

    /// Redo one mutation inside the active vector-edit session.
    pub fn redo_vector_edit(&mut self) -> bool {
        self.restore_vector_edit_snapshot(true)
    }

    fn insert_vector_anchor(&mut self, world_position: Vec2) -> bool {
        let before = self.vector_edit_snapshot();
        let node = match &self.drag {
            DragState::VectorEditing(session) => session.node,
            _ => return false,
        };
        let Some(mut path) = self
            .document
            .get(node)
            .and_then(|node| node.path())
            .cloned()
        else {
            return false;
        };
        let world = self.document.world_transform(node);
        let tolerance = 8.0 / self.view.zoom.abs().max(f32::EPSILON);
        let mut best: Option<(usize, usize, usize, f32, f32)> = None;

        for (contour_index, contour) in path.contours.iter().enumerate() {
            let segment_count = contour.anchors.len().saturating_sub(1)
                + usize::from(contour.closed && contour.anchors.len() > 1);
            for start_index in 0..segment_count {
                let end_index = (start_index + 1) % contour.anchors.len();
                let from = contour.anchors[start_index];
                let to = contour.anchors[end_index];
                let p0 = from.position;
                let p1 = p0 + from.out_handle.unwrap_or(Vec2::ZERO);
                let p3 = to.position;
                let p2 = p3 + to.in_handle.unwrap_or(Vec2::ZERO);
                for step in 1..32 {
                    let t = step as f32 / 32.0;
                    let point = world.transform_point2(cubic_point_at(p0, p1, p2, p3, t));
                    let distance = point.distance(world_position);
                    if distance <= tolerance
                        && best.is_none_or(|(_, _, _, _, best_distance)| distance < best_distance)
                    {
                        best = Some((contour_index, start_index, end_index, t, distance));
                    }
                }
            }
        }

        let Some((contour_index, start_index, end_index, t, _)) = best else {
            return false;
        };
        let contour = &mut path.contours[contour_index];
        let from = contour.anchors[start_index];
        let to = contour.anchors[end_index];
        let curved = from.out_handle.is_some() || to.in_handle.is_some();
        let p0 = from.position;
        let p1 = p0 + from.out_handle.unwrap_or(Vec2::ZERO);
        let p3 = to.position;
        let p2 = p3 + to.in_handle.unwrap_or(Vec2::ZERO);
        let q0 = p0.lerp(p1, t);
        let q1 = p1.lerp(p2, t);
        let q2 = p2.lerp(p3, t);
        let r0 = q0.lerp(q1, t);
        let r1 = q1.lerp(q2, t);
        let position = r0.lerp(r1, t);
        let inserted = if curved {
            contour.anchors[start_index].out_handle = Some(q0 - p0);
            contour.anchors[start_index].mode = HandleMode::Independent;
            contour.anchors[end_index].in_handle = Some(q2 - p3);
            contour.anchors[end_index].mode = HandleMode::Independent;
            PathAnchor {
                position,
                in_handle: Some(r0 - position),
                out_handle: Some(r1 - position),
                mode: HandleMode::Independent,
            }
        } else {
            PathAnchor::corner(position)
        };
        let inserted_index = if end_index == 0 {
            contour.anchors.push(inserted);
            contour.anchors.len() - 1
        } else {
            contour.anchors.insert(end_index, inserted);
            end_index
        };
        if let Some(document_node) = self.document.get_mut(node) {
            if let Some(document_path) = document_node.path_mut() {
                *document_path = path;
            }
        }
        self.document.mark_bounds_dirty(node);
        if let DragState::VectorEditing(session) = &mut self.drag {
            session.selected.clear();
            session.selected.insert(AnchorRef {
                node,
                contour: contour_index,
                anchor: inserted_index,
            });
        }
        self.needs_redraw = true;
        self.record_vector_edit(before);
        true
    }

    fn vector_pointer_down(&mut self, world_position: Vec2, modifiers: Modifiers) {
        let (node, path, mut originals) = match &self.drag {
            DragState::VectorEditing(session) => {
                let Some(path) = self
                    .document
                    .get(session.node)
                    .and_then(|node| node.path())
                    .cloned()
                else {
                    return;
                };
                let originals: HashMap<NodeId, PathData> = session
                    .before
                    .keys()
                    .filter_map(|id| {
                        self.document
                            .get(*id)
                            .and_then(|node| node.path())
                            .cloned()
                            .map(|path| (*id, path))
                    })
                    .collect();
                (session.node, path, originals)
            }
            _ => return,
        };
        let world = self.document.world_transform(node);
        let tolerance = 7.0 / self.view.zoom.abs().max(f32::EPSILON);
        let handle_hit =
            path.contours
                .iter()
                .enumerate()
                .flat_map(|(contour, contour_data)| {
                    contour_data.anchors.iter().enumerate().flat_map(
                        move |(anchor, anchor_data)| {
                            let reference = AnchorRef {
                                node,
                                contour,
                                anchor,
                            };
                            [
                                anchor_data.in_handle.map(|handle| {
                                    (
                                        VectorDragTarget::InHandle(reference),
                                        anchor_data.position + handle,
                                    )
                                }),
                                anchor_data.out_handle.map(|handle| {
                                    (
                                        VectorDragTarget::OutHandle(reference),
                                        anchor_data.position + handle,
                                    )
                                }),
                            ]
                            .into_iter()
                            .flatten()
                        },
                    )
                })
                .filter_map(|(target, position)| {
                    let distance = world.transform_point2(position).distance(world_position);
                    (distance <= tolerance).then_some((target, distance))
                })
                .min_by(|left, right| left.1.total_cmp(&right.1))
                .map(|(target, _)| target);
        let mut anchor_hit = path
            .contours
            .iter()
            .enumerate()
            .flat_map(|(contour, contour_data)| {
                contour_data
                    .anchors
                    .iter()
                    .enumerate()
                    .map(move |(anchor, anchor_data)| (contour, anchor, anchor_data.position))
            })
            .filter_map(|(contour, anchor, position)| {
                let distance = world.transform_point2(position).distance(world_position);
                (distance <= tolerance).then_some((
                    AnchorRef {
                        node,
                        contour,
                        anchor,
                    },
                    distance,
                ))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(anchor, _)| anchor);
        if anchor_hit.is_none() && modifiers.shift {
            let candidates = self
                .document
                .descendants(self.document.root)
                .filter(|id| *id != node)
                .filter(|id| {
                    self.document.is_effectively_editable(*id)
                        && self
                            .document
                            .get(*id)
                            .is_some_and(|node| matches!(node.kind, NodeKind::Shape(_)))
                })
                .collect::<Vec<_>>();
            let mut best = None;
            for candidate in candidates {
                let Some(candidate_path) = self
                    .document
                    .get(candidate)
                    .and_then(|node| node.path())
                    .cloned()
                else {
                    continue;
                };
                let candidate_world = self.document.world_transform(candidate);
                for (contour, contour_data) in candidate_path.contours.iter().enumerate() {
                    for (anchor, anchor_data) in contour_data.anchors.iter().enumerate() {
                        let distance = candidate_world
                            .transform_point2(anchor_data.position)
                            .distance(world_position);
                        if distance <= tolerance
                            && best.is_none_or(|(_, best_distance): (AnchorRef, f32)| {
                                distance < best_distance
                            })
                        {
                            best = Some((
                                AnchorRef {
                                    node: candidate,
                                    contour,
                                    anchor,
                                },
                                distance,
                            ));
                        }
                    }
                }
            }
            anchor_hit = best.map(|(anchor, _)| anchor);
            if let Some(anchor) = anchor_hit {
                if let Some(path) = self
                    .document
                    .get(anchor.node)
                    .and_then(|node| node.path())
                    .cloned()
                {
                    originals.insert(anchor.node, path);
                }
            }
        }

        let DragState::VectorEditing(session) = &mut self.drag else {
            return;
        };
        if let Some(target) = handle_hit {
            let anchor = match target {
                VectorDragTarget::Anchor(anchor)
                | VectorDragTarget::InHandle(anchor)
                | VectorDragTarget::OutHandle(anchor) => anchor,
            };
            session.selected.clear();
            session.selected.insert(anchor);
            session.dragging = Some((target, world_position, originals.clone()));
        } else if let Some(anchor) = anchor_hit {
            if let Some(path) = originals.get(&anchor.node) {
                session
                    .before
                    .entry(anchor.node)
                    .or_insert_with(|| path.clone());
            }
            if modifiers.shift {
                if !session.selected.remove(&anchor) {
                    session.selected.insert(anchor);
                }
            } else if !session.selected.contains(&anchor) {
                session.selected.clear();
                session.selected.insert(anchor);
            }
            session.dragging = Some((VectorDragTarget::Anchor(anchor), world_position, originals));
        } else if !modifiers.shift {
            session.selected.clear();
        }
        self.needs_redraw = true;
    }

    fn vector_pointer_move(&mut self, world_position: Vec2, modifiers: Modifiers) {
        let (target, selected, start, original) = match &self.drag {
            DragState::VectorEditing(session) => {
                let Some((target, start, original)) = &session.dragging else {
                    return;
                };
                (*target, session.selected.clone(), *start, original.clone())
            }
            _ => return,
        };
        match target {
            VectorDragTarget::Anchor(_) => {
                let affected = selected
                    .iter()
                    .map(|anchor| anchor.node)
                    .collect::<HashSet<_>>();
                for node in affected {
                    let world = self.document.world_transform(node);
                    if world.matrix2.determinant().abs() <= f32::EPSILON {
                        continue;
                    }
                    let local_delta = world.inverse().transform_vector2(world_position - start);
                    let Some(mut updated) = original.get(&node).cloned() else {
                        continue;
                    };
                    for anchor in selected.iter().filter(|anchor| anchor.node == node) {
                        if let Some(anchor_data) = updated
                            .contours
                            .get_mut(anchor.contour)
                            .and_then(|contour| contour.anchors.get_mut(anchor.anchor))
                        {
                            anchor_data.position += local_delta;
                        }
                    }
                    if let Some(document_node) = self.document.get_mut(node) {
                        if let Some(path) = document_node.path_mut() {
                            *path = updated;
                        }
                    }
                    self.document.mark_bounds_dirty(node);
                }
            }
            VectorDragTarget::InHandle(reference) | VectorDragTarget::OutHandle(reference) => {
                let node = reference.node;
                let world = self.document.world_transform(node);
                if world.matrix2.determinant().abs() <= f32::EPSILON {
                    return;
                }
                let inverse = world.inverse();
                let Some(mut updated) = original.get(&node).cloned() else {
                    return;
                };
                let Some(anchor) = updated
                    .contours
                    .get_mut(reference.contour)
                    .and_then(|contour| contour.anchors.get_mut(reference.anchor))
                else {
                    return;
                };
                let handle = inverse.transform_point2(world_position) - anchor.position;
                let dragging_in = matches!(target, VectorDragTarget::InHandle(_));
                let other = if dragging_in {
                    anchor.out_handle
                } else {
                    anchor.in_handle
                };
                if dragging_in {
                    anchor.in_handle = Some(handle);
                } else {
                    anchor.out_handle = Some(handle);
                }
                if modifiers.alt {
                    anchor.mode = HandleMode::Independent;
                } else {
                    match anchor.mode {
                        HandleMode::Mirrored => {
                            if dragging_in {
                                anchor.out_handle = Some(-handle);
                            } else {
                                anchor.in_handle = Some(-handle);
                            }
                        }
                        HandleMode::Aligned => {
                            let other_length = other.map_or(0.0, Vec2::length);
                            let aligned = if handle.length_squared() > f32::EPSILON {
                                -handle.normalize() * other_length
                            } else {
                                Vec2::ZERO
                            };
                            if dragging_in {
                                anchor.out_handle = Some(aligned);
                            } else {
                                anchor.in_handle = Some(aligned);
                            }
                        }
                        HandleMode::Corner | HandleMode::Independent => {
                            anchor.mode = HandleMode::Independent;
                        }
                    }
                }
                if let Some(document_node) = self.document.get_mut(node) {
                    if let Some(path) = document_node.path_mut() {
                        *path = updated;
                    }
                }
                self.document.mark_bounds_dirty(node);
            }
        }
        self.needs_redraw = true;
    }

    fn vector_pointer_up(&mut self) {
        let before = match &mut self.drag {
            DragState::VectorEditing(session) => {
                session
                    .dragging
                    .take()
                    .map(|(_, _, paths)| VectorEditSnapshot {
                        paths,
                        selected: session.selected.clone(),
                    })
            }
            _ => None,
        };
        self.record_vector_edit(before);
        self.needs_redraw = true;
    }

    fn vector_selected_anchors(&self) -> Vec<AnchorRef> {
        match &self.drag {
            DragState::VectorEditing(session) => session.selected.iter().copied().collect(),
            _ => Vec::new(),
        }
    }

    /// Delete the selected anchors in an active vector-edit session.
    pub fn delete_selected_vector_anchors(&mut self) -> bool {
        let before = self.vector_edit_snapshot();
        let selected = match &self.drag {
            DragState::VectorEditing(session) => session.selected.clone(),
            _ => return false,
        };
        if selected.is_empty() {
            return false;
        }
        let mut by_node: HashMap<NodeId, HashMap<usize, Vec<usize>>> = HashMap::new();
        for anchor in selected {
            by_node
                .entry(anchor.node)
                .or_default()
                .entry(anchor.contour)
                .or_default()
                .push(anchor.anchor);
        }
        for (node, by_contour) in by_node {
            let Some(mut path) = self
                .document
                .get(node)
                .and_then(|node| node.path())
                .cloned()
            else {
                continue;
            };
            for (contour_index, mut anchors) in by_contour {
                let Some(contour) = path.contours.get_mut(contour_index) else {
                    continue;
                };
                anchors.sort_unstable_by(|left, right| right.cmp(left));
                anchors.dedup();
                for anchor in anchors {
                    if anchor < contour.anchors.len() {
                        contour.anchors.remove(anchor);
                    }
                }
            }
            path.contours
                .retain(|contour| contour.anchors.len() >= if contour.closed { 3 } else { 2 });
            if let Some(document_node) = self.document.get_mut(node) {
                if let Some(document_path) = document_node.path_mut() {
                    *document_path = path;
                }
            }
            self.document.mark_bounds_dirty(node);
        }
        if let DragState::VectorEditing(session) = &mut self.drag {
            session.selected.clear();
        }
        self.needs_redraw = true;
        self.record_vector_edit(before);
        true
    }

    fn can_join_selected_paths(&self) -> bool {
        let DragState::VectorEditing(session) = &self.drag else {
            return false;
        };
        if session.selected.len() != 2 {
            return false;
        }
        session.selected.iter().all(|anchor| {
            self.document
                .get(anchor.node)
                .and_then(|node| node.path())
                .and_then(|path| path.contours.get(anchor.contour))
                .is_some_and(|contour| {
                    !contour.closed
                        && (anchor.anchor == 0 || anchor.anchor + 1 == contour.anchors.len())
                })
        })
    }

    fn join_selected_paths(&mut self) {
        if !self.can_join_selected_paths() {
            return;
        }
        let before = self.vector_edit_snapshot();
        let (node, selected) = match &self.drag {
            DragState::VectorEditing(session) => (
                session.node,
                session.selected.iter().copied().collect::<Vec<_>>(),
            ),
            _ => return,
        };
        let first = selected[0];
        let second = selected[1];
        if first.node == second.node {
            let Some(mut path) = self
                .document
                .get(first.node)
                .and_then(|node| node.path())
                .cloned()
            else {
                return;
            };
            if first.contour == second.contour {
                if let Some(contour) = path.contours.get_mut(first.contour) {
                    contour.closed = true;
                }
            } else {
                let (low, high) = if first.contour < second.contour {
                    (first, second)
                } else {
                    (second, first)
                };
                let mut right = path.contours.remove(high.contour);
                let left = &mut path.contours[low.contour];
                if low.anchor == 0 {
                    left.anchors.reverse();
                    swap_contour_handles(left);
                }
                if high.anchor + 1 == right.anchors.len() {
                    right.anchors.reverse();
                    swap_contour_handles(&mut right);
                }
                left.anchors.extend(right.anchors);
            }
            if let Some(document_node) = self.document.get_mut(first.node) {
                if let Some(document_path) = document_node.path_mut() {
                    *document_path = path;
                }
            }
            self.document.mark_bounds_dirty(first.node);
        } else {
            let (target, source) = if first.node == node {
                (first, second)
            } else {
                (second, first)
            };
            let Some(mut target_path) = self
                .document
                .get(target.node)
                .and_then(|node| node.path())
                .cloned()
            else {
                return;
            };
            let Some(mut source_path) = self
                .document
                .get(source.node)
                .and_then(|node| node.path())
                .cloned()
            else {
                return;
            };
            if target.contour >= target_path.contours.len()
                || source.contour >= source_path.contours.len()
            {
                return;
            }
            let mut source_contour = source_path.contours.remove(source.contour);
            let target_world = self.document.world_transform(target.node);
            if target_world.matrix2.determinant().abs() <= f32::EPSILON {
                return;
            }
            let source_to_target =
                target_world.inverse() * self.document.world_transform(source.node);
            transform_contour(&mut source_contour, source_to_target);
            let target_contour = &mut target_path.contours[target.contour];
            if target.anchor == 0 {
                target_contour.anchors.reverse();
                swap_contour_handles(target_contour);
            }
            if source.anchor + 1 == source_contour.anchors.len() {
                source_contour.anchors.reverse();
                swap_contour_handles(&mut source_contour);
            }
            target_contour.anchors.extend(source_contour.anchors);
            if let Some(document_node) = self.document.get_mut(target.node) {
                if let Some(path) = document_node.path_mut() {
                    *path = target_path;
                }
            }
            if let Some(document_node) = self.document.get_mut(source.node) {
                if let Some(path) = document_node.path_mut() {
                    *path = source_path;
                }
            }
            self.document.mark_bounds_dirty(target.node);
            self.document.mark_bounds_dirty(source.node);
        }
        if let DragState::VectorEditing(session) = &mut self.drag {
            session.selected.clear();
        }
        self.needs_redraw = true;
        self.record_vector_edit(before);
    }

    fn split_selected_path(&mut self) {
        let before = self.vector_edit_snapshot();
        let selected = self.vector_selected_anchors();
        if selected.len() != 1 {
            return;
        }
        let selected = selected[0];
        let Some(mut path) = self
            .document
            .get(selected.node)
            .and_then(|node| node.path())
            .cloned()
        else {
            return;
        };
        let Some(contour) = path.contours.get_mut(selected.contour) else {
            return;
        };
        if selected.anchor >= contour.anchors.len() {
            return;
        }
        if contour.closed {
            let mut terminal = contour.anchors[selected.anchor];
            terminal.out_handle = None;
            contour.anchors.rotate_left(selected.anchor);
            if let Some(start) = contour.anchors.first_mut() {
                start.in_handle = None;
            }
            contour.anchors.push(terminal);
            contour.closed = false;
        } else if selected.anchor > 0 && selected.anchor + 1 < contour.anchors.len() {
            let mut right_start = contour.anchors[selected.anchor];
            right_start.in_handle = None;
            contour.anchors[selected.anchor].out_handle = None;
            let mut right_anchors = contour.anchors.split_off(selected.anchor + 1);
            right_anchors.insert(0, right_start);
            let right = PathContour::open(right_anchors);
            path.contours.insert(selected.contour + 1, right);
        } else {
            return;
        }
        if let Some(node) = self.document.get_mut(selected.node) {
            if let Some(document_path) = node.path_mut() {
                *document_path = path;
            }
        }
        self.document.mark_bounds_dirty(selected.node);
        if let DragState::VectorEditing(session) = &mut self.drag {
            session.selected.clear();
        }
        self.needs_redraw = true;
        self.record_vector_edit(before);
    }

    fn reverse_selected_paths(&mut self) {
        let before = self.vector_edit_snapshot();
        let (node, selected) = match &self.drag {
            DragState::VectorEditing(session) => (session.node, session.selected.clone()),
            _ => return,
        };
        let mut by_node: HashMap<NodeId, HashSet<usize>> = HashMap::new();
        if selected.is_empty() {
            let Some(path) = self.document.get(node).and_then(|node| node.path()) else {
                return;
            };
            by_node.insert(node, (0..path.contours.len()).collect());
        } else {
            for anchor in selected {
                by_node
                    .entry(anchor.node)
                    .or_default()
                    .insert(anchor.contour);
            }
        }
        for (node, contours) in by_node {
            let Some(mut path) = self
                .document
                .get(node)
                .and_then(|node| node.path())
                .cloned()
            else {
                continue;
            };
            for contour in contours {
                if let Some(contour) = path.contours.get_mut(contour) {
                    contour.anchors.reverse();
                    swap_contour_handles(contour);
                }
            }
            if let Some(document_node) = self.document.get_mut(node) {
                if let Some(document_path) = document_node.path_mut() {
                    *document_path = path;
                }
            }
            self.document.mark_bounds_dirty(node);
        }
        if let DragState::VectorEditing(session) = &mut self.drag {
            session.selected.clear();
        }
        self.needs_redraw = true;
        self.record_vector_edit(before);
    }

    fn toggle_selected_contour_closed(&mut self) {
        let before = self.vector_edit_snapshot();
        let (node, selected) = match &self.drag {
            DragState::VectorEditing(session) => (session.node, session.selected.clone()),
            _ => return,
        };
        let mut by_node: HashMap<NodeId, HashSet<usize>> = HashMap::new();
        if selected.is_empty() {
            let Some(path) = self.document.get(node).and_then(|node| node.path()) else {
                return;
            };
            by_node.insert(node, (0..path.contours.len()).collect());
        } else {
            for anchor in selected {
                by_node
                    .entry(anchor.node)
                    .or_default()
                    .insert(anchor.contour);
            }
        }
        for (node, contours) in by_node {
            let Some(mut path) = self
                .document
                .get(node)
                .and_then(|node| node.path())
                .cloned()
            else {
                continue;
            };
            for contour in contours {
                if let Some(contour) = path.contours.get_mut(contour) {
                    if contour.anchors.len() >= 3 {
                        contour.closed = !contour.closed;
                    }
                }
            }
            if let Some(document_node) = self.document.get_mut(node) {
                if let Some(document_path) = document_node.path_mut() {
                    *document_path = path;
                }
            }
            self.document.mark_bounds_dirty(node);
        }
        self.needs_redraw = true;
        self.record_vector_edit(before);
    }

    /// Return the union of the current selection's world-space bounds.
    pub fn selection_world_bounds(&mut self) -> Option<Rect> {
        let mut bounds = Rect::empty();
        for id in self.selection.iter() {
            let node_bounds = self.document.world_bounds(id)?;
            bounds = if bounds.is_empty() {
                node_bounds
            } else {
                bounds.union(&node_bounds)
            };
        }
        (!bounds.is_empty()).then_some(bounds)
    }

    fn start_selection_transform(&mut self, screen: Vec2, world: Vec2) -> bool {
        if self.selection.is_empty() {
            return false;
        }
        let Some(bounds) = self.selection_world_bounds() else {
            return false;
        };
        let center_screen = self.view.to_screen(bounds.center());
        let rotation_position =
            Vec2::new(center_screen.x, self.view.to_screen(bounds.min).y - 24.0);
        let rotation_hit = screen.distance(rotation_position) <= 8.0;
        let text_only = self.selection.len() == 1
            && self.selection.primary().is_some_and(|id| {
                self.document
                    .get(id)
                    .is_some_and(|node| matches!(node.kind, NodeKind::Text(_)))
            });
        let handle = resize_handle_points(bounds)
            .into_iter()
            .filter(|(handle, _)| {
                !text_only || !matches!(handle, ResizeHandle::North | ResizeHandle::South)
            })
            .find_map(|(handle, position)| {
                (self.view.to_screen(position).distance(screen) <= 7.0).then_some(handle)
            });
        if !rotation_hit && handle.is_none() {
            return false;
        }

        let selected = self
            .document
            .filter_selection_for_transform(&self.selection.iter().collect::<Vec<_>>());
        let mut nodes = HashMap::new();
        let mut preserved_children = HashMap::new();
        for id in selected {
            let Some(node) = self.document.get(id).cloned() else {
                continue;
            };
            let world_transform = self.document.world_transform(id);
            if matches!(node.kind, NodeKind::Frame(_)) {
                for child in node.children {
                    if let Some(transform) = self.document.get(child).map(|child| child.transform) {
                        preserved_children
                            .insert(child, (transform, self.document.world_transform(child)));
                    }
                }
            }
            nodes.insert(
                id,
                TransformNodeSnapshot {
                    transform: node.transform,
                    kind: node.kind,
                    world: world_transform,
                },
            );
        }
        if nodes.is_empty() {
            return false;
        }

        let session = TransformSession {
            description: if rotation_hit { "Rotate" } else { "Resize" },
            bounds,
            start: world,
            handle,
            nodes,
            preserved_children,
        };
        self.drag = if rotation_hit {
            DragState::Rotating(session)
        } else {
            DragState::Resizing(session)
        };
        self.needs_redraw = true;
        true
    }

    fn update_resize(&mut self, world_position: Vec2, modifiers: Modifiers) {
        let (bounds, handle, nodes, preserved_children) = match &self.drag {
            DragState::Resizing(session) => (
                session.bounds,
                session.handle,
                session.nodes.clone(),
                session.preserved_children.clone(),
            ),
            _ => return,
        };
        let Some(handle) = handle else {
            return;
        };
        let handle_position = resize_handle_position(handle, bounds);
        let fixed = if modifiers.alt {
            bounds.center()
        } else {
            resize_handle_position(opposite_handle(handle), bounds)
        };
        let initial = handle_position - fixed;
        let current = world_position - fixed;
        let mut scale = Vec2::ONE;
        if handle.affects_x() && initial.x.abs() > f32::EPSILON {
            scale.x = current.x / initial.x;
        }
        if handle.affects_y() && initial.y.abs() > f32::EPSILON {
            scale.y = current.y / initial.y;
        }
        if modifiers.shift && handle.is_corner() {
            let uniform = if (scale.x.abs() - 1.0).abs() >= (scale.y.abs() - 1.0).abs() {
                scale.x
            } else {
                scale.y
            };
            scale = Vec2::splat(uniform);
        }
        // Semantic frame/text dimensions cannot encode a reflected box. Stop
        // the active handle at the opposite edge instead of mixing a signed
        // origin delta with absolute dimensions.
        scale.x = scale.x.max(0.01);
        scale.y = scale.y.max(0.01);
        let delta = Affine2::from_translation(fixed)
            * Affine2::from_scale(scale)
            * Affine2::from_translation(-fixed);

        for (id, snapshot) in nodes {
            match snapshot.kind {
                NodeKind::Shape(_) | NodeKind::Group => {
                    let after = self
                        .document
                        .author_transform_from_world(id, delta * snapshot.world);
                    if let Some(node) = self.document.get_mut(id) {
                        node.transform = after;
                    }
                    self.document.mark_transform_dirty(id);
                    self.document.mark_bounds_dirty(id);
                }
                NodeKind::Frame(mut frame) => {
                    if handle.affects_x() {
                        frame.width = (frame.width * scale.x.abs()).max(1.0);
                    }
                    if handle.affects_y() {
                        frame.height = (frame.height * scale.y.abs()).max(1.0);
                    }
                    let desired_origin = delta.transform_point2(snapshot.world.translation);
                    let desired_world =
                        Affine2::from_mat2_translation(snapshot.world.matrix2, desired_origin);
                    let after = self.document.author_transform_from_world(id, desired_world);
                    if let Some(node) = self.document.get_mut(id) {
                        node.transform = after;
                        node.kind = NodeKind::Frame(frame);
                    }
                    self.document.mark_transform_dirty(id);
                    self.document.mark_bounds_dirty(id);
                }
                NodeKind::Text(mut text) => {
                    if handle.is_corner() {
                        let factor = scale.x.abs().max(scale.y.abs()).max(0.01);
                        text.font_size = (text.font_size * factor).max(1.0);
                        if let TextSizing::FixedWidth { width } = &mut text.sizing {
                            *width = (*width * factor).max(1.0);
                        }
                    } else if handle.affects_x() {
                        let width = text
                            .fixed_width()
                            .unwrap_or_else(|| crate::text::estimate_text_metrics(&text).width);
                        text.sizing = TextSizing::FixedWidth {
                            width: (width * scale.x.abs()).max(1.0),
                        };
                    }
                    let desired_origin = delta.transform_point2(snapshot.world.translation);
                    let desired_world =
                        Affine2::from_mat2_translation(snapshot.world.matrix2, desired_origin);
                    let after = self.document.author_transform_from_world(id, desired_world);
                    if let Some(node) = self.document.get_mut(id) {
                        node.transform = after;
                        node.kind = NodeKind::Text(text);
                    }
                    self.document.mark_transform_dirty(id);
                    self.document.mark_bounds_dirty(id);
                }
            }
        }

        for (child, (_, original_world)) in preserved_children {
            let transform = self
                .document
                .author_transform_from_world(child, original_world);
            if let Some(node) = self.document.get_mut(child) {
                node.transform = transform;
            }
            self.document.mark_transform_dirty(child);
        }
        self.needs_redraw = true;
    }

    fn update_rotation(&mut self, world_position: Vec2, modifiers: Modifiers) {
        let (bounds, start, nodes) = match &self.drag {
            DragState::Rotating(session) => (session.bounds, session.start, session.nodes.clone()),
            _ => return,
        };
        let center = bounds.center();
        let mut angle = (world_position - center)
            .y
            .atan2((world_position - center).x)
            - (start - center).y.atan2((start - center).x);
        if modifiers.shift {
            let increment = std::f32::consts::PI / 12.0;
            angle = (angle / increment).round() * increment;
        }
        let delta = Affine2::from_translation(center)
            * Affine2::from_angle(angle)
            * Affine2::from_translation(-center);
        for (id, snapshot) in nodes {
            let after = self
                .document
                .author_transform_from_world(id, delta * snapshot.world);
            if let Some(node) = self.document.get_mut(id) {
                node.transform = after;
            }
            self.document.mark_transform_dirty(id);
        }
        self.needs_redraw = true;
    }

    fn restore_transform_session(&mut self, session: &TransformSession) {
        for (id, snapshot) in &session.nodes {
            if let Some(node) = self.document.get_mut(*id) {
                node.transform = snapshot.transform;
                node.kind = snapshot.kind.clone();
            }
            self.document.mark_transform_dirty(*id);
            if matches!(
                snapshot.kind,
                NodeKind::Frame(_) | NodeKind::Text(_) | NodeKind::Shape(_)
            ) {
                self.document.mark_bounds_dirty(*id);
            }
        }
        for (id, (transform, _)) in &session.preserved_children {
            if let Some(node) = self.document.get_mut(*id) {
                node.transform = *transform;
            }
            self.document.mark_transform_dirty(*id);
        }
    }

    fn commit_transform_session(&mut self, session: TransformSession) {
        let mut patches = Vec::new();
        for (id, snapshot) in session.nodes {
            let Some(node) = self.document.get(id) else {
                continue;
            };
            if node.transform != snapshot.transform {
                patches.push(Patch::SetTransform {
                    id,
                    before: snapshot.transform,
                    after: node.transform,
                });
            }
            match (&snapshot.kind, &node.kind) {
                (NodeKind::Frame(before), NodeKind::Frame(after)) if before != after => {
                    patches.push(Patch::SetFrame {
                        id,
                        before: before.clone(),
                        after: after.clone(),
                    });
                }
                (NodeKind::Text(before), NodeKind::Text(after)) if before != after => {
                    patches.push(Patch::SetText {
                        id,
                        before: before.clone(),
                        after: after.clone(),
                    });
                }
                _ => {}
            }
        }
        for (id, (before, _)) in session.preserved_children {
            if let Some(after) = self.document.get(id).map(|node| node.transform) {
                if before != after {
                    patches.push(Patch::SetTransform { id, before, after });
                }
            }
        }
        if !patches.is_empty() {
            self.history
                .push(Command::new(session.description).with_patches(patches));
        }
        self.needs_redraw = true;
    }

    /// Nudge the selection by a world-space delta.
    fn nudge_selection(&mut self, delta: Vec2) {
        self.translate_selection(delta, "Nudge");
    }

    fn translate_selection(&mut self, delta: Vec2, description: &'static str) -> bool {
        if !delta.is_finite() || delta.length_squared() <= f32::EPSILON {
            return false;
        }
        if self.selection.is_empty() {
            return false;
        }

        // Filter selection to exclude children of selected parents
        let all_nodes: Vec<_> = self.selection.iter().collect();
        let nodes = self.document.filter_selection_for_transform(&all_nodes);

        let mut patches = Vec::new();

        for id in nodes {
            let Some(before) = self.document.get(id).map(|node| node.transform) else {
                continue;
            };
            let world = self.document.world_transform(id);
            let after = self
                .document
                .author_transform_from_world(id, Affine2::from_translation(delta) * world);
            patches.push(Patch::SetTransform { id, before, after });
        }

        if !patches.is_empty() {
            // Apply patches to document
            for patch in &patches {
                patch.apply_forward(&mut self.document);
            }

            // Record in history
            self.history
                .push(Command::new(description).with_patches(patches));

            self.document.dirty.transforms = true;
            self.document.dirty.bounds = true;
            self.needs_redraw = true;
            true
        } else {
            false
        }
    }

    fn cursor_for_state(&self) -> Cursor {
        if self.space_pan_active
            && !matches!(
                self.drag,
                DragState::TextEditing(_) | DragState::Panning { .. }
            )
        {
            return Cursor::Grab;
        }
        match &self.drag {
            DragState::None => match self.tool {
                Tool::Select => Cursor::Default,
                Tool::Frame | Tool::Rectangle | Tool::Ellipse | Tool::Line | Tool::Pen => {
                    Cursor::Crosshair
                }
                Tool::Text => Cursor::Text,
                Tool::VectorEdit => Cursor::Default,
            },
            DragState::Moving { .. } => Cursor::Move,
            DragState::Resizing(session) => match session.handle {
                Some(ResizeHandle::North | ResizeHandle::South) => Cursor::ResizeNS,
                Some(ResizeHandle::East | ResizeHandle::West) => Cursor::ResizeEW,
                Some(ResizeHandle::NorthEast | ResizeHandle::SouthWest) => Cursor::ResizeNESW,
                Some(ResizeHandle::NorthWest | ResizeHandle::SouthEast) => Cursor::ResizeNWSE,
                None => Cursor::Default,
            },
            DragState::Rotating(_) => Cursor::Crosshair,
            DragState::Marquee { .. }
            | DragState::CreatingShape { .. }
            | DragState::CreatingFrame { .. }
            | DragState::Pen(_) => Cursor::Crosshair,
            DragState::CreatingText { .. } | DragState::TextEditing(_) => Cursor::Text,
            DragState::VectorEditing(_) => Cursor::Default,
            DragState::Panning { .. } => Cursor::Grabbing,
        }
    }

    /// Get the cursor for the active tool and interaction.
    pub fn cursor(&self) -> Cursor {
        self.cursor_for_state()
    }

    /// Delete the current selection.
    pub fn delete_selection(&mut self) {
        self.cancel_active_numeric_property_scrub();
        if self.selection.is_empty() {
            return;
        }

        // Filter out children of selected parents to avoid double-deletion
        let all_nodes: Vec<_> = self.selection.iter().collect();
        let nodes = self.document.filter_selection_for_transform(&all_nodes);

        let mut patches = Vec::new();

        for id in nodes {
            if let Some(node) = self.document.get(id) {
                if let Some(parent) = node.parent {
                    let index = self
                        .document
                        .get(parent)
                        .map(|p| p.children.iter().position(|&c| c == id).unwrap_or(0))
                        .unwrap_or(0);

                    patches.push(Patch::DeleteNode {
                        id,
                        node: Box::new(node.clone()),
                        parent,
                        index,
                    });
                }
            }
        }

        if !patches.is_empty() {
            let cmd = Command {
                description: "Delete".into(),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);
            self.selection.clear();
            self.needs_redraw = true;
        }
    }

    /// Group the current selection (with undo support).
    pub fn group_selection(&mut self) {
        self.cancel_active_numeric_property_scrub();
        let selected: Vec<_> = self.selection.iter().collect();
        let mut nodes = self.document.filter_selection_for_transform(&selected);
        if nodes.len() < 2 {
            return;
        }

        // Verify all nodes have the same parent
        let first_parent = match self.document.get(nodes[0]).and_then(|n| n.parent) {
            Some(p) => p,
            None => return,
        };

        for &id in &nodes[1..] {
            if self.document.get(id).and_then(|n| n.parent) != Some(first_parent) {
                return; // Can't group nodes with different parents
            }
        }
        let parent_world = self.document.world_transform(first_parent);
        let determinant = parent_world.matrix2.determinant();
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return;
        }

        // Find the minimum index among the nodes (for group placement)
        let parent_children = match self.document.get(first_parent) {
            Some(p) => p.children.clone(),
            None => return,
        };

        // HashSet-backed selection order is intentionally unspecified. Scene
        // order must determine the order of children inside the new group.
        nodes.sort_by_key(|id| {
            parent_children
                .iter()
                .position(|child| child == id)
                .unwrap_or(usize::MAX)
        });

        let Some(min_index) = parent_children.iter().position(|child| child == &nodes[0]) else {
            return;
        };

        // Auto-layout contributes a parent-controlled translation to each
        // child. Reparenting removes those individual offsets and gives the
        // new group a single offset instead, so preserving author transforms
        // alone would move the selected content. Snapshot world transforms
        // before changing the tree and derive new author transforms from the
        // final grouped hierarchy below.
        let mut preserved = Vec::with_capacity(nodes.len());
        for &id in &nodes {
            let Some(before_transform) = self.document.get(id).map(|node| node.transform) else {
                return;
            };
            let before_world = self.document.world_transform(id);
            preserved.push((id, before_transform, before_world));
        }

        // Create a new group node
        let group_node = Node::group("Group");
        let group_id = self.document.nodes.insert(group_node.clone());

        // Build patches for the operation
        let mut patches = Vec::new();

        // 1. Create the group node
        patches.push(Patch::CreateNode {
            id: group_id,
            node: Box::new(group_node),
            parent: first_parent,
            index: min_index,
        });

        // 2. Reparent each selected node to the group
        for (after_index, &id) in nodes.iter().enumerate() {
            let Some(before_index) = parent_children.iter().position(|&child| child == id) else {
                continue;
            };

            patches.push(Patch::Reparent {
                id,
                before_parent: first_parent,
                after_parent: group_id,
                before_index,
                after_index,
            });
        }

        // Resolve author transforms against the final hierarchy in a
        // temporary document. Reapply the desired world transforms until the
        // new group's measured size and parent auto-layout offset stabilize.
        let mut grouped_document = self.document.clone();
        for patch in &patches {
            patch.apply_forward(&mut grouped_document);
        }
        let desired_world = preserved
            .iter()
            .map(|(id, _, world)| (*id, *world))
            .collect::<Vec<_>>();
        grouped_document.restore_world_transforms(&desired_world);
        for (id, before, _) in preserved {
            let Some(after) = grouped_document.get(id).map(|node| node.transform) else {
                continue;
            };
            if before != after {
                patches.push(Patch::SetTransform { id, before, after });
            }
        }

        let cmd = Command {
            description: "Group".into(),
            patches,
        };
        cmd.apply(&mut self.document);
        self.history.push(cmd);

        self.selection.select(group_id);
        self.needs_redraw = true;
    }

    /// Ungroup the selected group(s) (with undo support).
    pub fn ungroup_selection(&mut self) {
        self.cancel_active_numeric_property_scrub();
        let selected_groups: Vec<_> = self
            .selection
            .iter()
            .filter(|&id| self.document.get(id).map(|n| n.is_group()).unwrap_or(false))
            .collect();
        let groups = self
            .document
            .filter_selection_for_transform(&selected_groups);

        if groups.is_empty() {
            return;
        }

        // Process siblings from front to back (highest index first). Expanding
        // a group changes the indices after it, but cannot affect lower ones.
        let mut groups_by_parent: HashMap<NodeId, Vec<(usize, NodeId)>> = HashMap::new();
        for group_id in groups {
            let Some(group) = self.document.get(group_id) else {
                continue;
            };
            let Some(parent_id) = group.parent else {
                continue;
            };
            let Some(group_index) = self
                .document
                .get(parent_id)
                .and_then(|parent| parent.children.iter().position(|&id| id == group_id))
            else {
                continue;
            };

            groups_by_parent
                .entry(parent_id)
                .or_default()
                .push((group_index, group_id));
        }
        for groups in groups_by_parent.values_mut() {
            groups.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
        }

        let mut patches = Vec::new();
        let mut preserved = Vec::new();
        let mut new_selection = Vec::new();

        for (parent_id, groups) in groups_by_parent {
            let parent_world = self.document.world_transform(parent_id);
            let determinant = parent_world.matrix2.determinant();
            if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
                continue;
            }

            for (group_index, group_id) in groups {
                let Some(group) = self.document.get(group_id).cloned() else {
                    continue;
                };
                let children = group.children.clone();

                // Preserve each child's world transform when removing the
                // group's transform from its ancestor chain.
                for (after_offset, &child_id) in children.iter().enumerate() {
                    let Some(before_transform) =
                        self.document.get(child_id).map(|child| child.transform)
                    else {
                        continue;
                    };
                    let child_world = self.document.world_transform(child_id);
                    preserved.push((child_id, before_transform, child_world));
                    patches.push(Patch::Reparent {
                        id: child_id,
                        before_parent: group_id,
                        after_parent: parent_id,
                        before_index: after_offset,
                        after_index: group_index + after_offset,
                    });
                    new_selection.push(child_id);
                }

                patches.push(Patch::DeleteNode {
                    id: group_id,
                    node: Box::new(group),
                    parent: parent_id,
                    index: group_index,
                });
            }
        }

        if !patches.is_empty() {
            // Resolve author transforms only after simulating the complete
            // hierarchy change. This accounts for auto-layout offsets applied
            // to each child once the wrapping group is gone.
            let mut ungrouped_document = self.document.clone();
            for patch in &patches {
                patch.apply_forward(&mut ungrouped_document);
            }
            let desired_world = preserved
                .iter()
                .map(|(id, _, world)| (*id, *world))
                .collect::<Vec<_>>();
            ungrouped_document.restore_world_transforms(&desired_world);
            for (id, before, _) in preserved {
                let Some(after) = ungrouped_document.get(id).map(|node| node.transform) else {
                    continue;
                };
                if before != after {
                    patches.push(Patch::SetTransform { id, before, after });
                }
            }

            let cmd = Command {
                description: "Ungroup".into(),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);

            self.selection.set(new_selection);
            self.needs_redraw = true;
        }
    }

    /// Duplicate the current selection (with undo support).
    pub fn duplicate_selection(&mut self) {
        self.cancel_active_numeric_property_scrub();
        // Filter selection to avoid duplicating children of selected parents
        let all_nodes: Vec<_> = self.selection.iter().collect();
        let nodes = self.document.filter_selection_for_transform(&all_nodes);

        if nodes.is_empty() {
            return;
        }

        let mut patches = Vec::new();
        let mut new_selection = Vec::new();

        let mut nodes_by_parent: HashMap<NodeId, Vec<(usize, NodeId)>> = HashMap::new();
        for id in nodes {
            let Some(parent) = self.document.get(id).and_then(|node| node.parent) else {
                continue;
            };
            let Some(index) = self
                .document
                .get(parent)
                .and_then(|node| node.children.iter().position(|&child| child == id))
            else {
                continue;
            };
            nodes_by_parent.entry(parent).or_default().push((index, id));
        }

        for (parent, mut siblings) in nodes_by_parent {
            siblings.sort_by_key(|(index, _)| *index);

            for (copies_before, (source_index, id)) in siblings.into_iter().enumerate() {
                let Some(node) = self.document.get(id).cloned() else {
                    continue;
                };

                let mut duplicate = node.clone();
                duplicate.name = format!("{} copy", duplicate.name);
                duplicate.parent = Some(parent);
                let has_children = !duplicate.children.is_empty();
                duplicate.children.clear();

                let new_id = self.document.nodes.insert(duplicate.clone());
                patches.push(Patch::CreateNode {
                    id: new_id,
                    node: Box::new(duplicate),
                    parent,
                    index: source_index + copies_before + 1,
                });

                if has_children {
                    self.collect_deep_clone_patches(id, new_id, &mut patches);
                }

                new_selection.push(new_id);
            }
        }

        if !patches.is_empty() {
            let cmd = Command {
                description: "Duplicate".into(),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);

            self.selection.set(new_selection);
            self.needs_redraw = true;
        }
    }

    /// Snapshot the selected object subtrees in paint order.
    pub fn copy_selection(&mut self) -> Option<EditorClipboard> {
        self.cancel_active_numeric_property_scrub();
        let selected = self.selection.iter().collect::<Vec<_>>();
        let roots = self.document.filter_selection_for_transform(&selected);
        if roots.is_empty() {
            return None;
        }

        let roots = roots.into_iter().collect::<HashSet<_>>();
        let ordered = self
            .document
            .paint_order()
            .filter(|id| roots.contains(id))
            .collect::<Vec<_>>();
        let mut snapshots = Vec::with_capacity(ordered.len());
        for id in ordered {
            let world = self.document.world_transform(id);
            let node = snapshot_clipboard_node(&self.document, id)?;
            snapshots.push(ClipboardRoot { node, world });
        }

        Some(EditorClipboard {
            roots: snapshots,
            paste_count: 0,
        })
    }

    /// Paste copied object subtrees at a cascading world-space offset.
    pub fn paste_clipboard(&mut self, clipboard: &mut EditorClipboard) -> bool {
        if clipboard.is_empty() {
            return false;
        }

        self.settle_interaction();
        let parent = self.document.root;
        let first_index = self
            .document
            .get(parent)
            .map_or(0, |node| node.children.len());
        let paste_count = clipboard.paste_count.saturating_add(1);
        let offset = Affine2::from_translation(Vec2::splat(paste_count as f32 * 10.0));
        let mut patches = Vec::new();
        let mut pasted_roots = Vec::with_capacity(clipboard.roots.len());
        let mut desired_world = Vec::with_capacity(clipboard.roots.len());

        for (index, root) in clipboard.roots.iter().enumerate() {
            let id = self.instantiate_clipboard_node(
                &root.node,
                parent,
                first_index + index,
                &mut patches,
            );
            pasted_roots.push(id);
            desired_world.push((id, offset * root.world));
        }

        let mut simulated = self.document.clone();
        for patch in &patches {
            patch.apply_forward(&mut simulated);
        }
        for (id, world) in desired_world {
            let Some(before) = simulated.get(id).map(|node| node.transform) else {
                continue;
            };
            let after = simulated.author_transform_from_world(id, world);
            if before != after {
                let patch = Patch::SetTransform { id, before, after };
                patch.apply_forward(&mut simulated);
                patches.push(patch);
            }
        }

        let command = Command {
            description: "Paste".into(),
            patches,
        };
        command.apply(&mut self.document);
        self.history.push(command);
        self.selection.set(pasted_roots);
        clipboard.paste_count = paste_count;
        self.needs_redraw = true;
        true
    }

    fn instantiate_clipboard_node(
        &mut self,
        snapshot: &ClipboardNode,
        parent: NodeId,
        index: usize,
        patches: &mut Vec<Patch>,
    ) -> NodeId {
        let mut node = snapshot.node.clone();
        node.parent = Some(parent);
        node.children.clear();
        node.deleted = false;
        let id = self.document.nodes.insert(node.clone());
        patches.push(Patch::CreateNode {
            id,
            node: Box::new(node),
            parent,
            index,
        });
        for (child_index, child) in snapshot.children.iter().enumerate() {
            self.instantiate_clipboard_node(child, id, child_index, patches);
        }
        id
    }

    /// Helper to collect patches for deep cloning children.
    fn collect_deep_clone_patches(
        &mut self,
        source_id: NodeId,
        new_parent_id: NodeId,
        patches: &mut Vec<Patch>,
    ) {
        let children = match self.document.get(source_id) {
            Some(n) => n.children.clone(),
            None => return,
        };

        for (i, &child_id) in children.iter().enumerate() {
            let child = match self.document.get(child_id) {
                Some(n) => n.clone(),
                None => continue,
            };

            let mut dup_child = child.clone();
            dup_child.parent = Some(new_parent_id);
            let child_has_children = !dup_child.children.is_empty();
            dup_child.children.clear();

            let new_child_id = self.document.nodes.insert(dup_child.clone());

            patches.push(Patch::CreateNode {
                id: new_child_id,
                node: Box::new(dup_child),
                parent: new_parent_id,
                index: i,
            });

            if child_has_children {
                self.collect_deep_clone_patches(child_id, new_child_id, patches);
            }
        }
    }

    /// Align selected nodes along an axis.
    pub fn align_selection(&mut self, axis: crate::snap::AlignAxis) {
        self.cancel_active_numeric_property_scrub();
        let selected: Vec<_> = self.selection.iter().collect();
        let nodes = self.document.filter_selection_for_transform(&selected);
        if nodes.len() < 2 {
            return;
        }

        // Get bounds for all selected nodes
        let bounds: Vec<_> = nodes
            .iter()
            .filter_map(|&id| self.document.world_bounds(id))
            .collect();

        if bounds.len() != nodes.len() {
            return; // Some nodes have no bounds
        }

        // Calculate alignment deltas
        let deltas = crate::snap::calculate_alignment(&bounds, axis);

        // Apply deltas to each node
        let mut patches = Vec::new();
        for (id, delta) in nodes.iter().zip(deltas.iter()) {
            if delta.length_squared() < 0.0001 {
                continue; // No change needed
            }

            let Some(before) = self.document.get(*id).map(|node| node.transform) else {
                continue;
            };
            let world = self.document.world_transform(*id);
            let after = self
                .document
                .author_transform_from_world(*id, glam::Affine2::from_translation(*delta) * world);

            patches.push(Patch::SetTransform {
                id: *id,
                before,
                after,
            });
        }

        if !patches.is_empty() {
            let cmd = Command {
                description: format!("Align {:?}", axis),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);
            self.needs_redraw = true;
        }
    }

    /// Distribute selected nodes evenly along an axis.
    pub fn distribute_selection(&mut self, axis: crate::snap::DistributeAxis) {
        self.cancel_active_numeric_property_scrub();
        let selected: Vec<_> = self.selection.iter().collect();
        let nodes = self.document.filter_selection_for_transform(&selected);
        if nodes.len() < 3 {
            return; // Need at least 3 nodes to distribute
        }

        // Get bounds for all selected nodes
        let bounds: Vec<_> = nodes
            .iter()
            .filter_map(|&id| self.document.world_bounds(id))
            .collect();

        if bounds.len() != nodes.len() {
            return; // Some nodes have no bounds
        }

        // Calculate distribution deltas
        let deltas = crate::snap::calculate_distribution(&bounds, axis);

        // Apply deltas to each node
        let mut patches = Vec::new();
        for (id, delta) in nodes.iter().zip(deltas.iter()) {
            if delta.length_squared() < 0.0001 {
                continue; // No change needed
            }

            let Some(before) = self.document.get(*id).map(|node| node.transform) else {
                continue;
            };
            let world = self.document.world_transform(*id);
            let after = self
                .document
                .author_transform_from_world(*id, glam::Affine2::from_translation(*delta) * world);

            patches.push(Patch::SetTransform {
                id: *id,
                before,
                after,
            });
        }

        if !patches.is_empty() {
            let cmd = Command {
                description: format!("Distribute {:?}", axis),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);
            self.needs_redraw = true;
        }
    }

    /// Bring selected nodes to front of z-order (with undo support).
    pub fn bring_selection_to_front(&mut self) {
        self.reorder_selected_siblings("Bring to Front", |children, selected| {
            children.sort_by_key(|id| selected.contains(id));
        });
    }

    /// Send selected nodes to back of z-order (with undo support).
    pub fn send_selection_to_back(&mut self) {
        self.reorder_selected_siblings("Send to Back", |children, selected| {
            children.sort_by_key(|id| !selected.contains(id));
        });
    }

    /// Bring selected nodes forward one step in z-order (with undo support).
    pub fn bring_selection_forward(&mut self) {
        self.reorder_selected_siblings("Bring Forward", |children, selected| {
            for index in (0..children.len().saturating_sub(1)).rev() {
                if selected.contains(&children[index]) && !selected.contains(&children[index + 1]) {
                    children.swap(index, index + 1);
                }
            }
        });
    }

    /// Send selected nodes backward one step in z-order (with undo support).
    pub fn send_selection_backward(&mut self) {
        self.reorder_selected_siblings("Send Backward", |children, selected| {
            for index in 1..children.len() {
                if selected.contains(&children[index]) && !selected.contains(&children[index - 1]) {
                    children.swap(index, index - 1);
                }
            }
        });
    }

    fn reorder_selected_siblings(
        &mut self,
        description: &str,
        mut reorder: impl FnMut(&mut Vec<NodeId>, &HashSet<NodeId>),
    ) {
        self.cancel_active_numeric_property_scrub();
        if self.selection.is_empty() {
            return;
        }

        let selection = self.selection.iter().collect::<Vec<_>>();
        let selected_roots = self.document.filter_selection_for_transform(&selection);
        let selected: HashSet<_> = selected_roots.into_iter().collect();
        let parent_ids: HashSet<_> = selected
            .iter()
            .filter_map(|id| self.document.get(*id).and_then(|node| node.parent))
            .collect();
        let mut patches = Vec::new();

        for parent_id in parent_ids {
            let Some(before) = self
                .document
                .get(parent_id)
                .map(|parent| parent.children.clone())
            else {
                continue;
            };
            let mut after = before.clone();
            reorder(&mut after, &selected);
            if before != after {
                patches.push(Patch::ReorderChildren {
                    parent: parent_id,
                    before,
                    after,
                });
            }
        }

        if !patches.is_empty() {
            let cmd = Command {
                description: description.into(),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);
            self.needs_redraw = true;
        }
    }

    /// Zoom to fit all content in the viewport.
    pub fn zoom_to_fit(&mut self) {
        if let Some(bounds) = self.artwork_bounds() {
            self.zoom_to_bounds(bounds);
        }
    }

    /// Set an absolute canvas zoom around the viewport center.
    ///
    /// Zoom is viewport state rather than document state, so this does not create an undo entry.
    /// Returns whether the view changed.
    pub fn set_zoom(&mut self, zoom: f32) -> bool {
        let changed = self.view.set_zoom_at(zoom, self.viewport_size * 0.5);
        self.needs_redraw |= changed;
        changed
    }

    /// Return conservative world-space bounds for all effectively visible artwork.
    ///
    /// Shape bounds include enough room for the SVG renderer's default miter limit, so exported
    /// strokes are not clipped. Frames contribute their explicit dimensions even when their
    /// background is transparent.
    pub fn artwork_bounds(&mut self) -> Option<Rect> {
        let ids = self
            .document
            .descendants(self.document.root)
            .filter(|id| self.document.is_effectively_visible(*id))
            .collect::<Vec<_>>();
        let mut combined = Rect::empty();

        for id in ids {
            let stroke_outset = self
                .document
                .get(id)
                .and_then(|node| match &node.kind {
                    NodeKind::Shape(_) => node.style.stroke.as_ref(),
                    NodeKind::Group | NodeKind::Text(_) | NodeKind::Frame(_) => None,
                })
                .map(|stroke| stroke.width.max(0.0))
                .unwrap_or(0.0);
            let transform_scale = if stroke_outset > 0.0 {
                max_affine_scale(self.document.world_transform(id))
            } else {
                0.0
            };
            let Some(mut bounds) = self.document.world_bounds(id) else {
                continue;
            };
            if !bounds.min.is_finite() || !bounds.max.is_finite() {
                continue;
            }

            // SVG's default miter limit is four times the half-stroke width.
            let margin = stroke_outset * transform_scale * 2.0;
            if margin.is_finite() && margin > 0.0 {
                bounds.min -= Vec2::splat(margin);
                bounds.max += Vec2::splat(margin);
            }
            combined = if combined.is_empty() {
                bounds
            } else {
                combined.union(&bounds)
            };
        }

        (!combined.is_empty()).then_some(combined)
    }

    /// Build a document-only display list in world coordinates with its visual bounds.
    pub fn artwork_snapshot(&mut self) -> Option<ArtworkSnapshot> {
        let bounds = self.artwork_bounds()?;
        let display_list = self.document.build_display_list(&View::default());
        Some(ArtworkSnapshot {
            display_list,
            bounds,
        })
    }

    /// Zoom to fit the current selection.
    pub fn zoom_to_selection(&mut self) {
        use crate::path::Rect;

        if self.selection.is_empty() {
            return;
        }

        let mut combined = Rect::empty();
        for id in self.selection.iter() {
            if let Some(bounds) = self.document.world_bounds(id) {
                if combined.is_empty() {
                    combined = bounds;
                } else {
                    combined = combined.union(&bounds);
                }
            }
        }

        if !combined.is_empty() {
            self.zoom_to_bounds(combined);
        }
    }

    /// Zoom to fit the given bounds with some padding.
    fn zoom_to_bounds(&mut self, bounds: crate::path::Rect) {
        let padding = 50.0;
        let viewport_width = self.viewport_size.x;
        let viewport_height = self.viewport_size.y;

        let content_width = bounds.width();
        let content_height = bounds.height();

        if !content_width.is_finite()
            || !content_height.is_finite()
            || content_width < 0.0
            || content_height < 0.0
        {
            return;
        }

        // Lines and point-like paths have a zero-sized axis. Give that axis a
        // small visual extent so fit-to-selection remains meaningful.
        let content_width = content_width.max(1.0);
        let content_height = content_height.max(1.0);

        // Calculate zoom to fit
        let available_width = (viewport_width - 2.0 * padding).max(1.0);
        let available_height = (viewport_height - 2.0 * padding).max(1.0);
        let zoom_x = available_width / content_width;
        let zoom_y = available_height / content_height;
        self.view.zoom = zoom_x.min(zoom_y).clamp(0.1, 10.0);

        // Center the content
        let center = bounds.center();
        self.view.pan = Vec2::new(
            viewport_width / 2.0 - center.x * self.view.zoom,
            viewport_height / 2.0 - center.y * self.view.zoom,
        );

        self.needs_redraw = true;
    }

    /// Update the drawable canvas size used by viewport-relative actions.
    pub fn set_viewport_size(&mut self, size: Vec2) {
        if size.is_finite() && size.x > 0.0 && size.y > 0.0 {
            self.viewport_size = size;
        }
    }

    /// Build a display list for rendering.
    pub fn build_display_list(&mut self) -> DisplayList {
        let mut display_list = self.document.build_display_list(&self.view);

        if let DragState::CreatingShape {
            shape,
            start_pos,
            current_pos,
            modifiers,
            ..
        } = &self.drag
        {
            let (bounds, path) = creation_geometry(*shape, *start_pos, *current_pos, *modifiers);
            let has_extent = if *shape == ShapeKind::Line {
                path.contours
                    .first()
                    .is_some_and(|contour| contour.anchors.len() == 2)
                    && (current_pos - start_pos).length_squared() > f32::EPSILON
            } else {
                bounds.width() > 0.0 && bounds.height() > 0.0
            };
            if has_extent {
                let path = crate::render::convert_path(&path);
                let transform =
                    self.view.screen_from_world() * Affine2::from_translation(bounds.min);
                let accent = editor_render::Paint::rgb(0.047, 0.55, 0.91);

                display_list.push(editor_render::DisplayItem::ToolPreview {
                    path,
                    fill: editor_render::Paint::rgba(0.047, 0.55, 0.91, 0.16),
                    stroke: editor_render::Stroke::new(
                        1.0 / self.view.zoom.abs().max(f32::EPSILON),
                        accent,
                    ),
                    transform,
                });
            }
        }

        if let DragState::CreatingFrame {
            start_pos,
            current_pos,
            modifiers,
            ..
        } = &self.drag
        {
            let bounds = creation_bounds(*start_pos, *current_pos, *modifiers);
            if bounds.width() > 0.0 && bounds.height() > 0.0 {
                display_list.push(editor_render::DisplayItem::ToolPreview {
                    path: editor_render::PathData::rect(0.0, 0.0, bounds.width(), bounds.height()),
                    fill: editor_render::Paint::rgba(1.0, 1.0, 1.0, 0.82),
                    stroke: editor_render::Stroke::new(
                        1.0 / self.view.zoom.abs().max(f32::EPSILON),
                        editor_render::Paint::rgb(0.047, 0.55, 0.91),
                    ),
                    transform: self.view.screen_from_world()
                        * Affine2::from_translation(bounds.min),
                });
            }
        }

        if let DragState::CreatingText {
            start_pos,
            current_pos,
        } = &self.drag
        {
            let bounds = Rect::new(start_pos.min(*current_pos), start_pos.max(*current_pos));
            if bounds.width() * self.view.zoom.abs() >= 3.0 {
                display_list.push(editor_render::DisplayItem::ToolPreview {
                    path: editor_render::PathData::rect(
                        0.0,
                        0.0,
                        bounds.width(),
                        bounds
                            .height()
                            .max(24.0 / self.view.zoom.abs().max(f32::EPSILON)),
                    ),
                    fill: editor_render::Paint::rgba(0.047, 0.55, 0.91, 0.04),
                    stroke: editor_render::Stroke::new(
                        1.0 / self.view.zoom.abs().max(f32::EPSILON),
                        editor_render::Paint::rgb(0.047, 0.55, 0.91),
                    ),
                    transform: self.view.screen_from_world()
                        * Affine2::from_translation(bounds.min),
                });
            }
        }

        if let DragState::Pen(session) = &self.drag {
            let mut anchors = session.anchors.clone();
            if !session.pointer_down && !anchors.is_empty() {
                anchors.push(PathAnchor::corner(session.hover));
            }
            if anchors.len() >= 2 {
                let path = PathData {
                    contours: vec![PathContour::open(anchors)],
                };
                display_list.push(editor_render::DisplayItem::ToolPreview {
                    path: crate::render::convert_path(&path),
                    fill: editor_render::Paint::rgba(0.0, 0.0, 0.0, 0.0),
                    stroke: editor_render::Stroke::new(
                        1.5 / self.view.zoom.abs().max(f32::EPSILON),
                        editor_render::Paint::rgb(0.047, 0.55, 0.91),
                    ),
                    transform: self.view.screen_from_world(),
                });
            }
            for anchor in &session.anchors {
                display_list.push(editor_render::DisplayItem::VectorAnchor {
                    position: self.view.to_screen(anchor.position),
                    selected: false,
                });
            }
        }

        // Add snap guides if we're dragging and have snap results
        let snap_result = match &self.drag {
            DragState::Moving { snap_result, .. }
            | DragState::CreatingShape { snap_result, .. }
            | DragState::CreatingFrame { snap_result, .. } => *snap_result,
            DragState::None
            | DragState::Resizing(_)
            | DragState::Rotating(_)
            | DragState::Marquee { .. }
            | DragState::Pen(_)
            | DragState::CreatingText { .. }
            | DragState::TextEditing(_)
            | DragState::VectorEditing(_)
            | DragState::Panning { .. } => None,
        };
        if let Some(result) = snap_result {
            // Add vertical snap guide (for X alignment)
            if let Some(snap_x) = result.snap_x {
                let screen_x = self.view.to_screen(Vec2::new(snap_x.value, 0.0)).x;
                display_list.push(editor_render::DisplayItem::SnapGuide {
                    start: Vec2::new(screen_x, 0.0),
                    end: Vec2::new(screen_x, self.viewport_size.y),
                    horizontal: false, // vertical line for X snap
                });
            }

            // Add horizontal snap guide (for Y alignment)
            if let Some(snap_y) = result.snap_y {
                let screen_y = self.view.to_screen(Vec2::new(0.0, snap_y.value)).y;
                display_list.push(editor_render::DisplayItem::SnapGuide {
                    start: Vec2::new(0.0, screen_y),
                    end: Vec2::new(self.viewport_size.x, screen_y),
                    horizontal: true, // horizontal line for Y snap
                });
            }
        }

        // Add selection rectangles for selected nodes
        for id in self.selection.iter() {
            if let Some(bounds) = self.document.world_bounds(id) {
                // Transform bounds to screen space
                let screen_min = self.view.to_screen(bounds.min);
                let screen_max = self.view.to_screen(bounds.max);

                display_list.push(editor_render::DisplayItem::SelectionRect {
                    min: screen_min,
                    max: screen_max,
                });
            }
        }

        if self.tool == Tool::Select
            && !self.selection.is_empty()
            && !matches!(
                self.drag,
                DragState::Marquee { .. } | DragState::Moving { .. }
            )
        {
            let text_only = self.selection.len() == 1
                && self.selection.primary().is_some_and(|id| {
                    self.document
                        .get(id)
                        .is_some_and(|node| matches!(node.kind, NodeKind::Text(_)))
                });
            if let Some(bounds) = self.selection_world_bounds() {
                for (handle, position) in resize_handle_points(bounds) {
                    if text_only && matches!(handle, ResizeHandle::North | ResizeHandle::South) {
                        continue;
                    }
                    display_list.push(editor_render::DisplayItem::TransformHandle {
                        position: self.view.to_screen(position),
                        rotation: false,
                    });
                }
                let top = self
                    .view
                    .to_screen(Vec2::new(bounds.center().x, bounds.min.y));
                display_list.push(editor_render::DisplayItem::TransformHandle {
                    position: top - Vec2::new(0.0, 24.0),
                    rotation: true,
                });
            }
        }

        if let DragState::VectorEditing(session) = &self.drag {
            let nodes = session.before.keys().copied().collect::<Vec<_>>();
            let selected = session.selected.clone();
            for node in nodes {
                let Some(path) = self
                    .document
                    .get(node)
                    .and_then(|node| node.path())
                    .cloned()
                else {
                    continue;
                };
                let transform = self.document.world_transform(node);
                for (contour_index, contour) in path.contours.iter().enumerate() {
                    for (anchor_index, anchor) in contour.anchors.iter().enumerate() {
                        let anchor_world = transform.transform_point2(anchor.position);
                        let anchor_screen = self.view.to_screen(anchor_world);
                        for handle in [anchor.in_handle, anchor.out_handle].into_iter().flatten() {
                            let handle_screen = self
                                .view
                                .to_screen(transform.transform_point2(anchor.position + handle));
                            display_list.push(editor_render::DisplayItem::VectorHandle {
                                anchor: anchor_screen,
                                handle: handle_screen,
                            });
                        }
                        display_list.push(editor_render::DisplayItem::VectorAnchor {
                            position: anchor_screen,
                            selected: selected.contains(&AnchorRef {
                                node,
                                contour: contour_index,
                                anchor: anchor_index,
                            }),
                        });
                    }
                }
            }
        }

        if let DragState::TextEditing(session) = &self.drag {
            if let Some(text) = self.document.get(session.node).and_then(|node| {
                let NodeKind::Text(text) = &node.kind else {
                    return None;
                };
                Some(text.clone())
            }) {
                let world = self.document.world_transform(session.node);
                let screen = self.view.screen_from_world() * world;
                let Some(layout) = self.document.text_layout(session.node) else {
                    return display_list;
                };
                for (range, marked) in [
                    (session.selection.clone(), false),
                    (session.marked.clone().unwrap_or(0..0), true),
                ] {
                    for rect in estimated_text_range_rects(&layout, &text.content, range) {
                        let corners = [
                            rect.min,
                            Vec2::new(rect.max.x, rect.min.y),
                            rect.max,
                            Vec2::new(rect.min.x, rect.max.y),
                        ]
                        .map(|point| screen.transform_point2(point));
                        display_list.push(editor_render::DisplayItem::TextSelectionRect {
                            corners,
                            marked,
                        });
                    }
                }
                let caret = if session.selection_reversed {
                    session.selection.start
                } else {
                    session.selection.end
                }
                .min(text.content.len());
                if session.selection.is_empty() {
                    let local = estimated_text_position(&layout, &text.content, caret);
                    display_list.push(editor_render::DisplayItem::TextCaret {
                        start: screen.transform_point2(local),
                        end: screen.transform_point2(local + Vec2::new(0.0, layout.line_height)),
                    });
                }
            }
        }

        // Add marquee selection rectangle if dragging
        if let DragState::Marquee {
            start_pos,
            current_pos,
        } = &self.drag
        {
            // Transform world positions to screen coordinates
            let screen_start = self.view.to_screen(*start_pos);
            let screen_current = self.view.to_screen(*current_pos);

            // Compute min/max corners
            let min = Vec2::new(
                screen_start.x.min(screen_current.x),
                screen_start.y.min(screen_current.y),
            );
            let max = Vec2::new(
                screen_start.x.max(screen_current.x),
                screen_start.y.max(screen_current.y),
            );

            display_list.push(editor_render::DisplayItem::MarqueeRect { min, max });
        }

        display_list
    }

    /// Check if a redraw is needed and clear the flag.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    /// Mark the editor as needing a redraw.
    pub fn set_needs_redraw(&mut self, needs_redraw: bool) {
        self.needs_redraw = needs_redraw;
    }

    /// Get the current view.
    pub fn view(&self) -> &View {
        &self.view
    }

    /// Get a reference to the document.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Install platform-shaped text layouts before building bounds and display geometry.
    pub fn set_text_layouts(&mut self, layouts: HashMap<NodeId, crate::text::TextLayout>) {
        self.document.set_text_layouts(layouts);
    }

    /// Get a reference to the selection.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Return the session-scoped styles used for newly created vector objects.
    pub fn creation_style_defaults(&self) -> &CreationStyleDefaults {
        &self.creation_styles
    }

    /// Replace the session-scoped creation styles without affecting history.
    pub fn set_creation_style_defaults(&mut self, defaults: CreationStyleDefaults) {
        if self.creation_styles != defaults {
            self.creation_styles = defaults;
            self.needs_redraw = true;
        }
    }

    /// Return the creation style associated with the active drawing tool.
    pub fn active_creation_style(&self) -> Option<&Style> {
        match self.tool {
            Tool::Rectangle | Tool::Ellipse => Some(&self.creation_styles.closed_shapes),
            Tool::Line | Tool::Pen => Some(&self.creation_styles.paths),
            Tool::Select | Tool::Frame | Tool::Text | Tool::VectorEdit => None,
        }
    }

    /// Set or remove the fill used by the active drawing tool.
    pub fn set_creation_fill(&mut self, fill: Option<Paint>) -> bool {
        self.update_active_creation_style(move |style| style.fill = fill)
    }

    /// Toggle the fill used by the active drawing tool.
    ///
    /// Enabling an absent fill starts from opaque black, matching the selected
    /// object style controls.
    pub fn toggle_creation_fill(&mut self) -> bool {
        self.update_active_creation_style(|style| {
            style.fill = if style.fill.is_some() {
                None
            } else {
                Some(Paint::black())
            };
        })
    }

    /// Set the stroke paint used by the active drawing tool.
    ///
    /// A missing stroke is created at the standard creation width.
    pub fn set_creation_stroke_paint(&mut self, paint: Paint) -> bool {
        self.update_active_creation_style(move |style| {
            style.stroke.get_or_insert_with(|| Stroke::black(1.5)).paint = paint;
        })
    }

    /// Toggle the stroke used by the active drawing tool.
    pub fn toggle_creation_stroke(&mut self) -> bool {
        self.update_active_creation_style(|style| {
            style.stroke = if style.stroke.is_some() {
                None
            } else {
                Some(Stroke::black(1.5))
            };
        })
    }

    /// Adjust the creation stroke width, creating a stroke when absent.
    pub fn adjust_creation_stroke_width(&mut self, delta: f32) -> bool {
        if !delta.is_finite() {
            return false;
        }
        self.update_active_creation_style(|style| {
            let stroke = style.stroke.get_or_insert_with(|| Stroke::black(1.5));
            stroke.width = (stroke.width + delta).clamp(0.1, 1000.0);
        })
    }

    fn update_active_creation_style(&mut self, update: impl FnOnce(&mut Style)) -> bool {
        let style = match self.tool {
            Tool::Rectangle | Tool::Ellipse => &mut self.creation_styles.closed_shapes,
            Tool::Line | Tool::Pen => &mut self.creation_styles.paths,
            Tool::Select | Tool::Frame | Tool::Text | Tool::VectorEdit => return false,
        };
        let before = style.clone();
        update(style);
        if *style == before {
            return false;
        }
        self.needs_redraw = true;
        true
    }

    /// Begin a live numeric-property scrub for the current selection.
    ///
    /// Starting a scrub settles pointer/text interactions and cancels any
    /// existing scrub before capturing a fresh immutable snapshot.
    pub fn begin_numeric_property_scrub(
        &mut self,
        target: NumericPropertyTarget,
    ) -> Option<NumericPropertyScrubSession> {
        self.settle_interaction();
        let selection = self.selection.to_vec();
        let snapshot = self.capture_numeric_property_snapshot(target)?;
        let id = self.next_numeric_property_scrub_id;
        self.next_numeric_property_scrub_id = self.next_numeric_property_scrub_id.wrapping_add(1);
        if self.next_numeric_property_scrub_id == 0 {
            self.next_numeric_property_scrub_id = 1;
        }
        self.numeric_property_scrub = Some(NumericPropertyScrubState {
            id,
            target,
            selection,
            revision: self.history.current_revision(),
            snapshot,
        });
        Some(NumericPropertyScrubSession::new(
            Arc::clone(&self.numeric_property_scrub_owner),
            id,
        ))
    }

    /// Preview an absolute delta from the values captured when the scrub began.
    ///
    /// Repeated calls never accumulate from the preceding preview. Invalid
    /// deltas leave the current preview unchanged and return `false`.
    pub fn preview_numeric_property_scrub(
        &mut self,
        session: &NumericPropertyScrubSession,
        delta: f32,
    ) -> bool {
        if !session.belongs_to(&self.numeric_property_scrub_owner)
            || !delta.is_finite()
            || self
                .numeric_property_scrub
                .as_ref()
                .is_none_or(|state| state.id != session.id())
        {
            return false;
        }
        if !self.numeric_property_scrub_is_valid() {
            self.cancel_active_numeric_property_scrub();
            return false;
        }

        let state = self
            .numeric_property_scrub
            .take()
            .expect("the scrub was validated above");
        let changed = self
            .apply_numeric_property_preview(&state, delta)
            .unwrap_or(false);
        self.numeric_property_scrub = Some(state);
        if changed {
            self.needs_redraw = true;
        }
        changed
    }

    /// Commit the current preview as exactly one undoable command.
    ///
    /// A no-op or invalidated session is consumed without adding history.
    pub fn commit_numeric_property_scrub(&mut self, session: NumericPropertyScrubSession) -> bool {
        if !session.belongs_to(&self.numeric_property_scrub_owner)
            || self
                .numeric_property_scrub
                .as_ref()
                .is_none_or(|state| state.id != session.id())
        {
            return false;
        }
        if !self.numeric_property_scrub_is_valid() {
            self.cancel_active_numeric_property_scrub();
            return false;
        }

        let state = self
            .numeric_property_scrub
            .take()
            .expect("the scrub was validated above");
        let patches = self.numeric_property_scrub_patches(&state);
        if patches.is_empty() {
            return false;
        }
        self.history
            .push(Command::new(state.target.description()).with_patches(patches));
        self.needs_redraw = true;
        true
    }

    /// Cancel a scrub and restore its original property values without history.
    pub fn cancel_numeric_property_scrub(&mut self, session: NumericPropertyScrubSession) -> bool {
        if !session.belongs_to(&self.numeric_property_scrub_owner)
            || self
                .numeric_property_scrub
                .as_ref()
                .is_none_or(|state| state.id != session.id())
        {
            return false;
        }
        self.cancel_active_numeric_property_scrub()
    }

    fn capture_numeric_property_snapshot(
        &mut self,
        target: NumericPropertyTarget,
    ) -> Option<NumericPropertySnapshot> {
        match target {
            NumericPropertyTarget::WorldX | NumericPropertyTarget::WorldY => {
                let selected = self
                    .document
                    .filter_selection_for_transform(&self.selection.iter().collect::<Vec<_>>());
                let mut snapshots = Vec::new();
                for id in selected {
                    if !self.document.is_effectively_editable(id) {
                        continue;
                    }
                    let node = self.document.get(id)?;
                    let snapshot = TransformSnapshot {
                        id,
                        parent: node.parent,
                        before: node.transform,
                        world: self.document.world_transform(id),
                    };
                    if snapshot.before.matrix2.is_finite()
                        && snapshot.before.translation.is_finite()
                        && snapshot.world.matrix2.is_finite()
                        && snapshot.world.translation.is_finite()
                    {
                        snapshots.push(snapshot);
                    }
                }
                (!snapshots.is_empty()).then_some(NumericPropertySnapshot::Transforms(snapshots))
            }
            NumericPropertyTarget::Opacity | NumericPropertyTarget::StrokeWidth => {
                let selected = self
                    .document
                    .filter_selection_for_transform(&self.selection.iter().collect::<Vec<_>>());
                let snapshots = selected
                    .into_iter()
                    .filter(|id| self.document.is_effectively_editable(*id))
                    .filter_map(|id| {
                        let before = self.document.get(id)?.style.clone();
                        let valid = match target {
                            NumericPropertyTarget::Opacity => before.opacity.is_finite(),
                            NumericPropertyTarget::StrokeWidth => before
                                .stroke
                                .as_ref()
                                .is_some_and(|stroke| stroke.width.is_finite()),
                            NumericPropertyTarget::WorldX
                            | NumericPropertyTarget::WorldY
                            | NumericPropertyTarget::TextSize
                            | NumericPropertyTarget::AutoLayoutSpacing
                            | NumericPropertyTarget::AutoLayoutPadding => false,
                        };
                        valid.then_some(StyleSnapshot { id, before })
                    })
                    .collect::<Vec<_>>();
                (!snapshots.is_empty()).then_some(NumericPropertySnapshot::Styles(snapshots))
            }
            NumericPropertyTarget::TextSize => {
                let id = self.single_editable_selection()?;
                let before = self.document.get(id).and_then(|node| match &node.kind {
                    NodeKind::Text(text) if text.font_size.is_finite() => Some(text.clone()),
                    _ => None,
                })?;
                Some(NumericPropertySnapshot::Text { id, before })
            }
            NumericPropertyTarget::AutoLayoutSpacing | NumericPropertyTarget::AutoLayoutPadding => {
                let id = self.single_editable_selection()?;
                let before = self.document.get(id).and_then(|node| {
                    matches!(node.kind, NodeKind::Group).then(|| node.layout.clone())
                })?;
                let Layout::Auto(layout) = &before else {
                    return None;
                };
                let valid = match target {
                    NumericPropertyTarget::AutoLayoutSpacing => layout.spacing.is_finite(),
                    NumericPropertyTarget::AutoLayoutPadding => [
                        layout.padding.left,
                        layout.padding.top,
                        layout.padding.right,
                        layout.padding.bottom,
                    ]
                    .into_iter()
                    .all(f32::is_finite),
                    NumericPropertyTarget::WorldX
                    | NumericPropertyTarget::WorldY
                    | NumericPropertyTarget::Opacity
                    | NumericPropertyTarget::StrokeWidth
                    | NumericPropertyTarget::TextSize => false,
                };
                valid.then_some(NumericPropertySnapshot::Layout { id, before })
            }
        }
    }

    fn single_editable_selection(&self) -> Option<NodeId> {
        (self.selection.len() == 1)
            .then(|| self.selection.primary())
            .flatten()
            .filter(|id| self.document.is_effectively_editable(*id))
    }

    fn numeric_property_scrub_is_valid(&self) -> bool {
        let Some(state) = &self.numeric_property_scrub else {
            return false;
        };
        if state.revision != self.history.current_revision()
            || state.selection != self.selection.to_vec()
        {
            return false;
        }
        match &state.snapshot {
            NumericPropertySnapshot::Transforms(snapshots) => snapshots.iter().all(|snapshot| {
                self.document.get(snapshot.id).is_some_and(|node| {
                    !node.deleted
                        && node.parent == snapshot.parent
                        && self.document.is_effectively_editable(snapshot.id)
                })
            }),
            NumericPropertySnapshot::Styles(snapshots) => snapshots.iter().all(|snapshot| {
                self.document
                    .get(snapshot.id)
                    .is_some_and(|node| !node.deleted)
                    && self.document.is_effectively_editable(snapshot.id)
            }),
            NumericPropertySnapshot::Text { id, .. } => {
                self.document.get(*id).is_some_and(|node| {
                    !node.deleted
                        && matches!(node.kind, NodeKind::Text(_))
                        && self.document.is_effectively_editable(*id)
                })
            }
            NumericPropertySnapshot::Layout { id, .. } => {
                self.document.get(*id).is_some_and(|node| {
                    !node.deleted
                        && matches!(node.kind, NodeKind::Group)
                        && matches!(node.layout, Layout::Auto(_))
                        && self.document.is_effectively_editable(*id)
                })
            }
        }
    }

    fn apply_numeric_property_preview(
        &mut self,
        state: &NumericPropertyScrubState,
        delta: f32,
    ) -> Option<bool> {
        match (&state.target, &state.snapshot) {
            (
                NumericPropertyTarget::WorldX | NumericPropertyTarget::WorldY,
                NumericPropertySnapshot::Transforms(snapshots),
            ) => self.preview_numeric_world_position(state.target, snapshots, delta),
            (
                NumericPropertyTarget::Opacity | NumericPropertyTarget::StrokeWidth,
                NumericPropertySnapshot::Styles(snapshots),
            ) => self.preview_numeric_style(state.target, snapshots, delta),
            (NumericPropertyTarget::TextSize, NumericPropertySnapshot::Text { id, before }) => {
                self.preview_numeric_text(*id, before, delta)
            }
            (
                NumericPropertyTarget::AutoLayoutSpacing | NumericPropertyTarget::AutoLayoutPadding,
                NumericPropertySnapshot::Layout { id, before },
            ) => self.preview_numeric_layout(state.target, *id, before, delta),
            _ => None,
        }
    }

    fn preview_numeric_world_position(
        &mut self,
        target: NumericPropertyTarget,
        snapshots: &[TransformSnapshot],
        delta: f32,
    ) -> Option<bool> {
        let translation = match target {
            NumericPropertyTarget::WorldX => Vec2::new(delta, 0.0),
            NumericPropertyTarget::WorldY => Vec2::new(0.0, delta),
            NumericPropertyTarget::Opacity
            | NumericPropertyTarget::StrokeWidth
            | NumericPropertyTarget::TextSize
            | NumericPropertyTarget::AutoLayoutSpacing
            | NumericPropertyTarget::AutoLayoutPadding => return None,
        };
        let desired = snapshots
            .iter()
            .map(|snapshot| {
                let world = Affine2::from_translation(translation) * snapshot.world;
                (world.translation.is_finite() && world.matrix2.is_finite())
                    .then_some((snapshot.id, world))
            })
            .collect::<Option<Vec<_>>>()?;
        let current = snapshots
            .iter()
            .map(|snapshot| Some((snapshot.id, self.document.get(snapshot.id)?.transform)))
            .collect::<Option<Vec<_>>>()?;

        self.restore_numeric_transforms(snapshots);
        if delta != 0.0 {
            self.document.restore_world_transforms(&desired);
        }

        Some(current.into_iter().any(|(id, before)| {
            self.document
                .get(id)
                .is_some_and(|node| node.transform != before)
        }))
    }

    fn preview_numeric_style(
        &mut self,
        target: NumericPropertyTarget,
        snapshots: &[StyleSnapshot],
        delta: f32,
    ) -> Option<bool> {
        let after = snapshots
            .iter()
            .map(|snapshot| {
                let mut style = snapshot.before.clone();
                match target {
                    NumericPropertyTarget::Opacity => {
                        style.opacity = bounded_numeric_sum(style.opacity, delta, 0.0, 1.0)?;
                    }
                    NumericPropertyTarget::StrokeWidth => {
                        let stroke = style.stroke.as_mut()?;
                        stroke.width = bounded_numeric_sum(stroke.width, delta, 0.1, 1000.0)?;
                    }
                    NumericPropertyTarget::WorldX
                    | NumericPropertyTarget::WorldY
                    | NumericPropertyTarget::TextSize
                    | NumericPropertyTarget::AutoLayoutSpacing
                    | NumericPropertyTarget::AutoLayoutPadding => return None,
                }
                Some((snapshot.id, style))
            })
            .collect::<Option<Vec<_>>>()?;

        let mut changed = false;
        for (id, after) in after {
            let node = self.document.get_mut(id)?;
            if node.style != after {
                node.style = after;
                self.document.mark_bounds_dirty(id);
                changed = true;
            }
        }
        Some(changed)
    }

    fn preview_numeric_text(&mut self, id: NodeId, before: &TextData, delta: f32) -> Option<bool> {
        let mut after = before.clone();
        after.font_size = bounded_numeric_sum(before.font_size, delta, 1.0, 512.0)?;
        let node = self.document.get_mut(id)?;
        let NodeKind::Text(current) = &mut node.kind else {
            return None;
        };
        if *current == after {
            return Some(false);
        }
        *current = after;
        self.document.mark_bounds_dirty(id);
        self.document.mark_layout_dirty();
        Some(true)
    }

    fn preview_numeric_layout(
        &mut self,
        target: NumericPropertyTarget,
        id: NodeId,
        before: &Layout,
        delta: f32,
    ) -> Option<bool> {
        let Layout::Auto(mut after) = before.clone() else {
            return None;
        };
        match target {
            NumericPropertyTarget::AutoLayoutSpacing => {
                after.spacing = bounded_numeric_sum(after.spacing, delta, 0.0, f32::MAX)?;
            }
            NumericPropertyTarget::AutoLayoutPadding => {
                after.padding.left = bounded_numeric_sum(after.padding.left, delta, 0.0, f32::MAX)?;
                after.padding.top = bounded_numeric_sum(after.padding.top, delta, 0.0, f32::MAX)?;
                after.padding.right =
                    bounded_numeric_sum(after.padding.right, delta, 0.0, f32::MAX)?;
                after.padding.bottom =
                    bounded_numeric_sum(after.padding.bottom, delta, 0.0, f32::MAX)?;
            }
            NumericPropertyTarget::WorldX
            | NumericPropertyTarget::WorldY
            | NumericPropertyTarget::Opacity
            | NumericPropertyTarget::StrokeWidth
            | NumericPropertyTarget::TextSize => return None,
        }
        let after = Layout::Auto(after);
        let node = self.document.get_mut(id)?;
        if node.layout == after {
            return Some(false);
        }
        node.layout = after;
        self.document.mark_layout_dirty();
        Some(true)
    }

    fn numeric_property_scrub_patches(&self, state: &NumericPropertyScrubState) -> Vec<Patch> {
        match &state.snapshot {
            NumericPropertySnapshot::Transforms(snapshots) => snapshots
                .iter()
                .filter_map(|snapshot| {
                    let after = self.document.get(snapshot.id)?.transform;
                    (after != snapshot.before).then_some(Patch::SetTransform {
                        id: snapshot.id,
                        before: snapshot.before,
                        after,
                    })
                })
                .collect(),
            NumericPropertySnapshot::Styles(snapshots) => snapshots
                .iter()
                .filter_map(|snapshot| {
                    let after = self.document.get(snapshot.id)?.style.clone();
                    (after != snapshot.before).then(|| Patch::SetStyle {
                        id: snapshot.id,
                        before: snapshot.before.clone(),
                        after,
                    })
                })
                .collect(),
            NumericPropertySnapshot::Text { id, before } => self
                .document
                .get(*id)
                .and_then(|node| match &node.kind {
                    NodeKind::Text(after) if after != before => Some(Patch::SetText {
                        id: *id,
                        before: before.clone(),
                        after: after.clone(),
                    }),
                    _ => None,
                })
                .into_iter()
                .collect(),
            NumericPropertySnapshot::Layout { id, before } => self
                .document
                .get(*id)
                .and_then(|node| {
                    (node.layout != *before).then(|| Patch::SetLayout {
                        id: *id,
                        before: before.clone(),
                        after: node.layout.clone(),
                    })
                })
                .into_iter()
                .collect(),
        }
    }

    fn cancel_active_numeric_property_scrub(&mut self) -> bool {
        let Some(state) = self.numeric_property_scrub.take() else {
            return false;
        };
        if self.restore_numeric_property_snapshot(&state.snapshot) {
            self.needs_redraw = true;
        }
        true
    }

    fn restore_numeric_property_snapshot(&mut self, snapshot: &NumericPropertySnapshot) -> bool {
        match snapshot {
            NumericPropertySnapshot::Transforms(snapshots) => {
                self.restore_numeric_transforms(snapshots)
            }
            NumericPropertySnapshot::Styles(snapshots) => {
                let mut changed = false;
                for snapshot in snapshots {
                    let Some(node) = self.document.get_mut(snapshot.id) else {
                        continue;
                    };
                    if node.deleted || node.style == snapshot.before {
                        continue;
                    }
                    node.style = snapshot.before.clone();
                    self.document.mark_bounds_dirty(snapshot.id);
                    changed = true;
                }
                changed
            }
            NumericPropertySnapshot::Text { id, before } => {
                let Some(node) = self.document.get_mut(*id) else {
                    return false;
                };
                let NodeKind::Text(current) = &mut node.kind else {
                    return false;
                };
                if node.deleted || current == before {
                    return false;
                }
                *current = before.clone();
                self.document.mark_bounds_dirty(*id);
                self.document.mark_layout_dirty();
                true
            }
            NumericPropertySnapshot::Layout { id, before } => {
                let Some(node) = self.document.get_mut(*id) else {
                    return false;
                };
                if node.deleted || node.layout == *before {
                    return false;
                }
                node.layout = before.clone();
                self.document.mark_layout_dirty();
                true
            }
        }
    }

    fn restore_numeric_transforms(&mut self, snapshots: &[TransformSnapshot]) -> bool {
        let mut changed = false;
        for snapshot in snapshots {
            let Some(node) = self.document.get_mut(snapshot.id) else {
                continue;
            };
            if node.deleted || node.transform == snapshot.before {
                continue;
            }
            node.transform = snapshot.before;
            self.document.mark_transform_dirty(snapshot.id);
            changed = true;
        }
        changed
    }

    /// Return the selected text properties used by contextual desktop controls.
    pub fn selected_text_data(&self) -> Option<TextData> {
        let id = self.selection.primary()?;
        self.document.get(id).and_then(|node| match &node.kind {
            NodeKind::Text(text) => Some(text.clone()),
            _ => None,
        })
    }

    /// Return the selected frame properties used by contextual desktop controls.
    pub fn selected_frame_data(&self) -> Option<crate::FrameData> {
        let id = self.selection.primary()?;
        self.document.get(id).and_then(|node| match &node.kind {
            NodeKind::Frame(frame) => Some(frame.clone()),
            _ => None,
        })
    }

    /// Return the selected group's current layout configuration.
    pub fn selected_group_layout(&self) -> Option<Layout> {
        if self.selection.len() != 1 {
            return None;
        }
        let id = self.selection.primary()?;
        self.document
            .get(id)
            .and_then(|node| matches!(node.kind, NodeKind::Group).then(|| node.layout.clone()))
    }

    /// Return world-space affine components for a single selected layer.
    pub fn selected_transform_components(&mut self) -> Option<TransformComponents> {
        if self.selection.len() != 1 {
            return None;
        }
        let id = self.selection.primary()?;
        TransformComponents::from_affine(self.document.world_transform(id))
    }

    /// Change the selected group between free, horizontal, and vertical layout.
    ///
    /// Crossing the free/automatic boundary bakes or removes derived layout
    /// offsets in the child author transforms so merely enabling or disabling
    /// layout does not visually move existing artwork.
    pub fn set_selected_group_layout_mode(&mut self, mode: GroupLayoutMode) -> bool {
        self.settle_interaction();
        let Some(before) = self.selected_group_layout() else {
            return false;
        };
        let after = match (mode, &before) {
            (GroupLayoutMode::Free, _) => Layout::Free,
            (GroupLayoutMode::Horizontal, Layout::Auto(layout)) => {
                let mut layout = layout.clone();
                layout.direction = Direction::Horizontal;
                Layout::Auto(layout)
            }
            (GroupLayoutMode::Vertical, Layout::Auto(layout)) => {
                let mut layout = layout.clone();
                layout.direction = Direction::Vertical;
                Layout::Auto(layout)
            }
            (GroupLayoutMode::Horizontal, Layout::Free) => Layout::Auto(AutoLayout::horizontal()),
            (GroupLayoutMode::Vertical, Layout::Free) => Layout::Auto(AutoLayout::vertical()),
        };
        let preserve_children = before.is_auto() != after.is_auto();
        self.apply_selected_group_layout("Change Layout Mode", after, preserve_children)
    }

    /// Set nonnegative spacing for the selected auto-layout group.
    pub fn set_selected_group_spacing(&mut self, spacing: f32) -> bool {
        if !spacing.is_finite() || spacing < 0.0 {
            return false;
        }
        self.update_selected_auto_layout("Change Layout Spacing", |layout| {
            layout.spacing = spacing;
        })
    }

    /// Set equal, nonnegative padding on every edge of the selected group.
    pub fn set_selected_group_uniform_padding(&mut self, padding: f32) -> bool {
        if !padding.is_finite() || padding < 0.0 {
            return false;
        }
        self.update_selected_auto_layout("Change Layout Padding", |layout| {
            layout.padding = crate::Edges::uniform(padding);
        })
    }

    /// Set main-axis alignment for the selected auto-layout group.
    pub fn set_selected_group_main_alignment(&mut self, alignment: AlignMain) -> bool {
        self.update_selected_auto_layout("Change Main Alignment", |layout| {
            layout.align_main = alignment;
        })
    }

    /// Set cross-axis alignment for the selected auto-layout group.
    pub fn set_selected_group_cross_alignment(&mut self, alignment: AlignCross) -> bool {
        self.update_selected_auto_layout("Change Cross Alignment", |layout| {
            layout.align_cross = alignment;
        })
    }

    fn update_selected_auto_layout(
        &mut self,
        description: &'static str,
        update: impl FnOnce(&mut AutoLayout),
    ) -> bool {
        self.settle_interaction();
        let Some(Layout::Auto(mut after)) = self.selected_group_layout() else {
            return false;
        };
        update(&mut after);
        self.apply_selected_group_layout(description, Layout::Auto(after), false)
    }

    fn apply_selected_group_layout(
        &mut self,
        description: &'static str,
        after: Layout,
        preserve_children: bool,
    ) -> bool {
        self.settle_interaction();
        let Some(id) = self.selection.primary() else {
            return false;
        };
        if self.selection.len() != 1 || !self.document.is_effectively_editable(id) {
            return false;
        }
        let Some((before, children)) = self.document.get(id).and_then(|node| {
            matches!(node.kind, NodeKind::Group)
                .then(|| (node.layout.clone(), node.children.clone()))
        }) else {
            return false;
        };
        if before == after {
            return false;
        }

        let mut preserved = Vec::new();
        if preserve_children && !children.is_empty() {
            let group_world = self.document.world_transform(id);
            let determinant = group_world.matrix2.determinant();
            if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
                return false;
            }
            preserved.reserve(children.len());
            for child in children {
                let Some(transform) = self.document.get(child).map(|node| node.transform) else {
                    return false;
                };
                let world = self.document.world_transform(child);
                preserved.push((child, transform, world));
            }
        }

        let mut patches = vec![Patch::SetLayout { id, before, after }];
        if !preserved.is_empty() {
            let mut simulated = self.document.clone();
            patches[0].apply_forward(&mut simulated);
            for (child, before, world) in preserved {
                let after = simulated.author_transform_from_world(child, world);
                if before != after {
                    let patch = Patch::SetTransform {
                        id: child,
                        before,
                        after,
                    };
                    patch.apply_forward(&mut simulated);
                    patches.push(patch);
                }
            }
        }

        let command = Command::new(description).with_patches(patches);
        command.apply(&mut self.document);
        self.history.push(command);
        self.needs_redraw = true;
        true
    }

    /// Move the current selection by a world-space delta.
    pub fn move_selection_by(&mut self, delta: Vec2) -> bool {
        self.settle_interaction();
        self.translate_selection(delta, "Move")
    }

    /// Rotate the current selection around its world-space center.
    pub fn rotate_selection_by(&mut self, angle_radians: f32) -> bool {
        if !angle_radians.is_finite() || angle_radians.abs() <= f32::EPSILON {
            return false;
        }
        self.settle_interaction();
        let Some(bounds) = self.selection_world_bounds() else {
            return false;
        };
        let selected = self
            .document
            .filter_selection_for_transform(&self.selection.iter().collect::<Vec<_>>());
        let center = bounds.center();
        let delta = Affine2::from_translation(center)
            * Affine2::from_angle(angle_radians)
            * Affine2::from_translation(-center);
        let mut patches = Vec::new();
        for id in selected {
            let Some(before) = self.document.get(id).map(|node| node.transform) else {
                continue;
            };
            let world = self.document.world_transform(id);
            let after = self.document.author_transform_from_world(id, delta * world);
            if before != after {
                patches.push(Patch::SetTransform { id, before, after });
            }
        }
        if patches.is_empty() {
            return false;
        }

        let command = Command::new("Rotate").with_patches(patches);
        command.apply(&mut self.document);
        self.history.push(command);
        self.needs_redraw = true;
        true
    }

    /// Set opacity for every independently selected layer.
    pub fn set_selected_opacity(&mut self, opacity: f32) -> bool {
        let opacity = opacity.clamp(0.0, 1.0);
        self.update_selected_styles("Change Opacity", |style| style.opacity = opacity)
    }

    /// Set or remove the fill for every independently selected layer.
    pub fn set_selected_fill(&mut self, fill: Option<Paint>) -> bool {
        self.update_selected_styles("Change Fill", move |style| style.fill = fill.clone())
    }

    /// Set the stroke paint for every independently selected layer.
    ///
    /// Layers without a stroke receive a one-unit stroke.
    pub fn set_selected_stroke_paint(&mut self, paint: Paint) -> bool {
        self.update_selected_styles("Change Stroke Color", move |style| {
            let stroke = style.stroke.get_or_insert_with(|| Stroke::black(1.0));
            stroke.paint = paint.clone();
        })
    }

    /// Toggle a default stroke for every independently selected layer.
    pub fn toggle_selected_stroke(&mut self) -> bool {
        self.update_selected_styles("Toggle Stroke", |style| {
            style.stroke = if style.stroke.is_some() {
                None
            } else {
                Some(Stroke::black(1.0))
            };
        })
    }

    /// Adjust selected stroke widths, creating a default stroke when absent.
    pub fn adjust_selected_stroke_width(&mut self, delta: f32) -> bool {
        if !delta.is_finite() {
            return false;
        }
        self.update_selected_styles("Change Stroke Width", |style| {
            let stroke = style.stroke.get_or_insert_with(|| Stroke::black(1.0));
            stroke.width = (stroke.width + delta).clamp(0.1, 1000.0);
        })
    }

    fn update_selected_styles(
        &mut self,
        description: &'static str,
        mut update: impl FnMut(&mut Style),
    ) -> bool {
        self.settle_interaction();
        let selected = self
            .document
            .filter_selection_for_transform(&self.selection.iter().collect::<Vec<_>>());
        let mut patches = Vec::new();
        for id in selected {
            let Some(before) = self.document.get(id).map(|node| node.style.clone()) else {
                continue;
            };
            let mut after = before.clone();
            update(&mut after);
            if before != after {
                patches.push(Patch::SetStyle { id, before, after });
            }
        }
        if patches.is_empty() {
            return false;
        }

        let command = Command::new(description).with_patches(patches);
        command.apply(&mut self.document);
        self.history.push(command);
        self.needs_redraw = true;
        true
    }

    /// Adjust the selected text size as one undoable property edit.
    pub fn adjust_selected_text_size(&mut self, delta: f32) -> bool {
        self.update_selected_text("Change Text Size", |text| {
            text.font_size = (text.font_size + delta).clamp(1.0, 512.0);
        })
    }

    /// Set the selected text alignment as one undoable property edit.
    pub fn set_selected_text_alignment(&mut self, alignment: crate::TextAlign) -> bool {
        self.update_selected_text("Align Text", |text| text.align = alignment)
    }

    /// Set the selected text's font family as one undoable property edit.
    ///
    /// Family names are trimmed, but otherwise preserved so imported documents
    /// can continue to use fonts beyond the portable families offered by the UI.
    pub fn set_selected_text_font_family(&mut self, family: &str) -> bool {
        let family = family.trim();
        if family.is_empty() {
            return false;
        }
        let family = family.to_owned();
        self.update_selected_text("Change Font Family", move |text| {
            text.font.family = family;
        })
    }

    /// Set the selected text's font weight as one undoable property edit.
    ///
    /// The weight is clamped to the supported CSS range and canonicalized to
    /// the nearest hundred so font lookup and the UI share stable presets.
    pub fn set_selected_text_font_weight(&mut self, weight: u16) -> bool {
        let weight = canonical_font_weight(weight);
        self.update_selected_text("Change Font Weight", |text| {
            text.font.weight = weight;
        })
    }

    /// Set the selected text's italic style as one undoable property edit.
    pub fn set_selected_text_italic(&mut self, italic: bool) -> bool {
        self.update_selected_text("Change Font Style", |text| {
            text.font.italic = italic;
        })
    }

    fn update_selected_text(
        &mut self,
        description: &'static str,
        update: impl FnOnce(&mut TextData),
    ) -> bool {
        self.cancel_active_numeric_property_scrub();
        let Some(id) = self.selection.primary() else {
            return false;
        };
        let Some(before) = self.document.get(id).and_then(|node| match &node.kind {
            NodeKind::Text(text) => Some(text.clone()),
            _ => None,
        }) else {
            return false;
        };
        let mut after = before.clone();
        update(&mut after);
        if after == before {
            return false;
        }
        if let Some(node) = self.document.get_mut(id) {
            node.kind = NodeKind::Text(after.clone());
        }
        self.document.mark_bounds_dirty(id);
        let part_of_text_session = matches!(
            &self.drag,
            DragState::TextEditing(session) if session.node == id
        );
        if part_of_text_session {
            if let DragState::TextEditing(session) = &mut self.drag {
                session.undo.push(TextEditSnapshot {
                    text: before,
                    selection: session.selection.clone(),
                    marked: session.marked.clone(),
                    selection_reversed: session.selection_reversed,
                });
                session.redo.clear();
            }
        } else {
            self.history
                .push(Command::new(description).with_patch(Patch::SetText { id, before, after }));
        }
        self.needs_redraw = true;
        true
    }

    /// Toggle the selected frame between its default white artboard and transparent.
    pub fn toggle_selected_frame_background(&mut self) -> bool {
        self.cancel_active_numeric_property_scrub();
        let Some(id) = self.selection.primary() else {
            return false;
        };
        let Some(before) = self.document.get(id).and_then(|node| match &node.kind {
            NodeKind::Frame(frame) => Some(frame.clone()),
            _ => None,
        }) else {
            return false;
        };
        let mut after = before.clone();
        after.background = if after.background.is_some() {
            None
        } else {
            Some(Paint::white())
        };
        let command = Command::new("Toggle Frame Background").with_patch(Patch::SetFrame {
            id,
            before,
            after,
        });
        command.apply(&mut self.document);
        self.history.push(command);
        self.needs_redraw = true;
        true
    }
}

fn max_affine_scale(transform: Affine2) -> f32 {
    let matrix = transform.matrix2;
    let trace = matrix.x_axis.length_squared() + matrix.y_axis.length_squared();
    let determinant = matrix.determinant();
    let discriminant = (trace * trace - 4.0 * determinant * determinant).max(0.0);
    ((trace + discriminant.sqrt()) * 0.5).sqrt()
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with_named_shapes(names: &[&str]) -> (Editor, Vec<NodeId>) {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let ids = names
            .iter()
            .map(|name| {
                editor
                    .document
                    .add_child(
                        root,
                        Node::shape(*name, PathData::rect(0.0, 0.0, 10.0, 10.0)),
                    )
                    .unwrap()
            })
            .collect();
        (editor, ids)
    }

    fn editor_with_grouped_shapes() -> (Editor, NodeId, NodeId, NodeId) {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let group = editor
            .document
            .add_child(root, Node::group("Group"))
            .unwrap();
        let first = editor
            .document
            .add_child(
                group,
                Node::shape("First", PathData::rect(0.0, 0.0, 20.0, 20.0))
                    .with_style(Style::fill(Paint::black())),
            )
            .unwrap();
        let second = editor
            .document
            .add_child(
                group,
                Node::shape("Second", PathData::rect(0.0, 0.0, 20.0, 20.0))
                    .with_style(Style::fill(Paint::black()))
                    .with_transform(Affine2::from_translation(Vec2::new(40.0, 0.0))),
            )
            .unwrap();
        (editor, group, first, second)
    }

    fn editor_with_nested_branches() -> (Editor, NodeId, [NodeId; 3], [NodeId; 3]) {
        let mut editor = Editor::new();
        let outer = editor
            .document
            .add_child(editor.document.root, Node::group("Outer"))
            .unwrap();
        let mut branches = Vec::new();
        let mut leaves = Vec::new();

        for (index, name) in ["Left", "Middle", "Right"].into_iter().enumerate() {
            let branch = editor
                .document
                .add_child(outer, Node::group(format!("{name} branch")))
                .unwrap();
            let leaf = editor
                .document
                .add_child(
                    branch,
                    Node::shape(
                        format!("{name} leaf"),
                        PathData::rect(index as f32 * 40.0, 0.0, 20.0, 20.0),
                    )
                    .with_style(Style::fill(Paint::black())),
                )
                .unwrap();
            branches.push(branch);
            leaves.push(leaf);
        }

        (
            editor,
            outer,
            branches.try_into().unwrap(),
            leaves.try_into().unwrap(),
        )
    }

    fn child_names(editor: &Editor, parent: NodeId) -> Vec<&str> {
        editor
            .document
            .get(parent)
            .unwrap()
            .children
            .iter()
            .map(|id| editor.document.get(*id).unwrap().name.as_str())
            .collect()
    }

    fn assert_affine_approx_eq(actual: Affine2, expected: Affine2) {
        for (actual, expected) in actual
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
        }
    }

    #[test]
    fn test_new_editor() {
        let editor = Editor::new();
        assert!(editor.selection.is_empty());
        assert_eq!(editor.tool, Tool::Select);
    }

    #[test]
    fn test_demo_content() {
        let editor = Editor::with_demo_content();
        // Should have 3 shapes + 2 text nodes = 5 items
        let count = editor.document.descendants(editor.document.root).count();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_click_to_select() {
        let mut editor = Editor::with_demo_content();

        // Click on the first shape (at approximately 125, 100 - center of first rect)
        let event = InputEvent::PointerDown {
            position: Vec2::new(125.0, 100.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        };

        editor.handle_event(event);

        // Something should be selected
        assert!(!editor.selection.is_empty());
    }

    #[test]
    fn canvas_drag_selects_and_moves_the_group_by_default() {
        let (mut editor, group, first, second) = editor_with_grouped_shapes();
        let group_before = editor.document.world_transform(group);
        let first_local_before = editor.document.get(first).unwrap().transform;
        let first_before = editor.document.world_transform(first);
        let second_before = editor.document.world_transform(second);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.selection.primary(), Some(group));
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::splat(20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let delta = Affine2::from_translation(Vec2::splat(10.0));
        assert_affine_approx_eq(editor.document.world_transform(group), delta * group_before);
        assert_affine_approx_eq(
            editor.document.get(first).unwrap().transform,
            first_local_before,
        );
        assert_affine_approx_eq(editor.document.world_transform(first), delta * first_before);
        assert_affine_approx_eq(
            editor.document.world_transform(second),
            delta * second_before,
        );
    }

    #[test]
    fn small_group_drag_does_not_snap_to_its_own_children() {
        let (mut editor, group, _, _) = editor_with_grouped_shapes();
        let before = editor.document.world_transform(group);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(13.0, 14.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        assert_affine_approx_eq(
            editor.document.world_transform(group),
            Affine2::from_translation(Vec2::new(3.0, 4.0)) * before,
        );
    }

    #[test]
    fn primary_modifier_drag_deep_selects_and_moves_only_the_child() {
        let (mut editor, group, first, second) = editor_with_grouped_shapes();
        let group_before = editor.document.world_transform(group);
        let first_before = editor.document.world_transform(first);
        let second_before = editor.document.world_transform(second);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: Modifiers {
                meta: true,
                ..Modifiers::default()
            },
        });
        assert_eq!(editor.selection.primary(), Some(first));
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(20.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers {
                meta: true,
                ..Modifiers::default()
            },
        });

        let delta = Affine2::from_translation(Vec2::new(10.0, 0.0));
        assert_affine_approx_eq(editor.document.world_transform(group), group_before);
        assert_affine_approx_eq(editor.document.world_transform(first), delta * first_before);
        assert_affine_approx_eq(editor.document.world_transform(second), second_before);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(20.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.selection.primary(), Some(first));
    }

    #[test]
    fn shift_click_toggles_the_resolved_container_or_deep_child() {
        let (mut editor, group, first, _) = editor_with_grouped_shapes();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: shift,
        });
        assert_eq!(editor.selection.to_vec(), vec![group]);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: shift,
        });
        assert!(editor.selection.is_empty());

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: Modifiers {
                shift: true,
                meta: true,
                ..Modifiers::default()
            },
        });
        assert_eq!(editor.selection.to_vec(), vec![first]);
    }

    #[test]
    fn select_all_and_invert_stay_within_the_active_hierarchy_scope() {
        let (mut editor, group, first, second) = editor_with_grouped_shapes();
        let root_sibling = editor
            .document
            .add_child(
                editor.document.root,
                Node::shape("Root sibling", PathData::rect(80.0, 0.0, 20.0, 20.0)),
            )
            .unwrap();

        assert!(editor.execute_action(EditorAction::SelectAll));
        assert_eq!(editor.selection.to_vec(), vec![group, root_sibling]);

        editor.selection.select(first);
        assert!(editor.execute_action(EditorAction::SelectAll));
        assert_eq!(editor.selection.to_vec(), vec![first, second]);

        editor.selection.select(first);
        assert!(editor.execute_action(EditorAction::InvertSelection));
        assert_eq!(editor.selection.to_vec(), vec![second]);
    }

    #[test]
    fn nested_branch_selection_uses_its_lowest_common_scope() {
        let (mut editor, outer, branches, leaves) = editor_with_nested_branches();
        let root_sibling = editor
            .document
            .add_child(
                editor.document.root,
                Node::shape("Root sibling", PathData::rect(200.0, 0.0, 20.0, 20.0)),
            )
            .unwrap();

        editor.selection.set([leaves[0], leaves[1]]);
        assert_eq!(editor.selection_scope(), outer);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(90.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.selection.to_vec(), vec![branches[2]]);
        assert!(editor.cancel_pointer_interaction());

        editor.selection.set([leaves[0], leaves[1]]);
        assert!(editor.execute_action(EditorAction::SelectAll));
        assert_eq!(editor.selection.to_vec(), branches);
        assert!(!editor.selection.contains(root_sibling));

        editor.selection.set([leaves[0], leaves[1]]);
        assert!(editor.execute_action(EditorAction::InvertSelection));
        assert_eq!(editor.selection.to_vec(), vec![branches[2]]);

        editor.selection.set([leaves[0], leaves[1]]);
        assert_eq!(
            editor.compute_marquee_selection(Vec2::new(1.0, 1.0), Vec2::new(99.0, 19.0), false,),
            branches
        );
    }

    #[test]
    fn select_parent_normalizes_overlaps_and_is_atomic_at_the_root() {
        let (mut editor, outer, branches, leaves) = editor_with_nested_branches();
        let direct_leaf = editor
            .document
            .add_child(
                outer,
                Node::shape("Direct leaf", PathData::rect(120.0, 0.0, 20.0, 20.0)),
            )
            .unwrap();
        let root_leaf = editor
            .document
            .add_child(
                editor.document.root,
                Node::shape("Root leaf", PathData::rect(160.0, 0.0, 20.0, 20.0)),
            )
            .unwrap();

        editor.selection.set([leaves[0], direct_leaf]);
        assert!(editor.select_parent());
        assert_eq!(editor.selection.to_vec(), vec![outer]);
        assert!(!editor.selection.contains(branches[0]));

        editor.selection.set([leaves[0], root_leaf]);
        let before = editor.selection.to_vec();
        assert!(!editor.select_parent());
        assert_eq!(editor.selection.to_vec(), before);
    }

    #[test]
    fn cancelling_pointer_work_restores_moves_but_preserves_text_editing() {
        let (mut editor, group, _, _) = editor_with_grouped_shapes();
        let group_before = editor.document.world_transform(group);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(14.0, 15.0),
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::Moving);
        assert_ne!(editor.document.world_transform(group), group_before);

        assert!(editor.cancel_pointer_interaction());
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_affine_approx_eq(editor.document.world_transform(group), group_before);

        let text = editor
            .document
            .add_child(editor.document.root, Node::text("Text", "Editing"))
            .unwrap();
        editor.begin_existing_text_edit(text);
        assert_eq!(editor.interaction_kind(), InteractionKind::TextEditing);
        assert!(!editor.cancel_pointer_interaction());
        assert_eq!(editor.interaction_kind(), InteractionKind::TextEditing);
    }

    #[test]
    fn cancelling_pointer_work_ends_text_selection_without_leaving_text_editing() {
        let mut editor = Editor::new();
        let text = editor
            .document
            .add_child(editor.document.root, Node::text("Text", "Editing"))
            .unwrap();
        editor.begin_existing_text_edit(text);
        editor.text_pointer_down(Vec2::new(1.0, 1.0), Modifiers::default());
        let DragState::TextEditing(session) = &editor.drag else {
            panic!("expected text editing");
        };
        assert!(session.pointer_selection_anchor.is_some());

        assert!(editor.cancel_pointer_interaction());

        let DragState::TextEditing(session) = &editor.drag else {
            panic!("text editing should remain active");
        };
        assert!(session.pointer_selection_anchor.is_none());
    }

    #[test]
    fn cancelling_pointer_work_restores_an_active_vector_drag() {
        let mut editor = Editor::new();
        let id = editor
            .document
            .add_child(
                editor.document.root,
                Node::shape("Path", PathData::rect(0.0, 0.0, 20.0, 20.0)),
            )
            .unwrap();
        let original = editor.document.get(id).unwrap().path().unwrap().clone();
        editor.selection.select(id);
        assert!(editor.execute_action(EditorAction::EnterVectorEdit));
        editor.vector_pointer_down(Vec2::ZERO, Modifiers::default());
        editor.vector_pointer_move(Vec2::splat(10.0), Modifiers::default());
        assert_ne!(editor.document.get(id).unwrap().path(), Some(&original));

        assert!(editor.cancel_pointer_interaction());

        let DragState::VectorEditing(session) = &editor.drag else {
            panic!("vector editing should remain active");
        };
        assert!(session.dragging.is_none());
        assert!(session.undo.is_empty());
        assert_eq!(editor.document.get(id).unwrap().path(), Some(&original));

        editor.vector_pointer_move(Vec2::splat(30.0), Modifiers::default());
        assert_eq!(editor.document.get(id).unwrap().path(), Some(&original));
    }

    #[test]
    fn cancelling_pointer_work_releases_the_temporary_hand_tool() {
        let mut editor = Editor::new();
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Space,
            modifiers: Modifiers::default(),
        });
        assert!(editor.space_pan_active);

        assert!(editor.cancel_pointer_interaction());

        assert!(!editor.space_pan_active);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_ne!(editor.interaction_kind(), InteractionKind::Panning);
    }

    #[test]
    fn cancelling_pointer_work_preserves_pen_anchors_and_ends_only_its_pointer_gesture() {
        let mut editor = Editor::new();
        assert!(editor.execute_action(EditorAction::ToolPen));
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(20.0, 20.0),
            modifiers: Modifiers::default(),
        });

        let anchor_before = match &editor.drag {
            DragState::Pen(session) => {
                assert!(session.pointer_down);
                assert_eq!(session.active_anchor, Some(0));
                assert_eq!(session.anchors.len(), 1);
                session.anchors[0]
            }
            _ => panic!("expected Pen session"),
        };

        assert!(editor.cancel_pointer_interaction());
        assert_eq!(editor.interaction_kind(), InteractionKind::Pen);
        let DragState::Pen(session) = &editor.drag else {
            panic!("expected Pen session");
        };
        assert_eq!(session.anchors, vec![anchor_before]);
        assert!(!session.pointer_down);
        assert_eq!(session.active_anchor, None);

        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(30.0, 30.0),
            modifiers: Modifiers::default(),
        });
        let DragState::Pen(session) = &editor.drag else {
            panic!("expected Pen session");
        };
        assert_eq!(session.anchors, vec![anchor_before]);

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Space,
            modifiers: Modifiers::default(),
        });
        let pan_before = editor.view.pan;
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(40.0, 40.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(50.0, 50.0),
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::Panning);

        assert!(editor.cancel_pointer_interaction());
        assert_eq!(editor.interaction_kind(), InteractionKind::Pen);
        assert_eq!(editor.view.pan, pan_before);
        let DragState::Pen(session) = &editor.drag else {
            panic!("expected restored Pen session");
        };
        assert_eq!(session.anchors, vec![anchor_before]);
        assert!(!session.pointer_down);
        assert_eq!(session.active_anchor, None);
    }

    #[test]
    fn double_click_and_enter_descend_one_level_and_shift_enter_goes_up() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let outer = editor
            .document
            .add_child(root, Node::group("Outer"))
            .unwrap();
        let inner = editor
            .document
            .add_child(outer, Node::group("Inner"))
            .unwrap();
        let leaf = editor
            .document
            .add_child(
                inner,
                Node::shape("Leaf", PathData::rect(0.0, 0.0, 20.0, 20.0))
                    .with_style(Style::fill(Paint::black())),
            )
            .unwrap();

        editor.handle_pointer_down_with_click_count(
            Vec2::splat(10.0),
            MouseButton::Left,
            Modifiers::default(),
            1,
        );
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::splat(10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.selection.primary(), Some(outer));

        editor.handle_pointer_down_with_click_count(
            Vec2::splat(10.0),
            MouseButton::Left,
            Modifiers::default(),
            2,
        );
        assert_eq!(editor.selection.primary(), Some(inner));
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.selection.primary(), Some(leaf));
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Enter,
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        });
        assert_eq!(editor.selection.primary(), Some(inner));

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.selection.primary(), Some(leaf));
        assert_eq!(editor.interaction_kind(), InteractionKind::VectorEditing);
    }

    #[test]
    fn test_click_empty_deselects() {
        let mut editor = Editor::with_demo_content();

        // First select something
        let shapes: Vec<_> = editor.document.descendants(editor.document.root).collect();
        editor.selection.select(shapes[0]);
        assert!(!editor.selection.is_empty());

        // Click on empty space (pointer down starts marquee mode)
        let down_event = InputEvent::PointerDown {
            position: Vec2::new(500.0, 500.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        };
        editor.handle_event(down_event);

        // Selection not cleared yet (we're in marquee mode)
        assert!(!editor.selection.is_empty());

        // Pointer up without drag clears selection
        let up_event = InputEvent::PointerUp {
            position: Vec2::new(500.0, 500.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        };
        editor.handle_event(up_event);

        // Now selection should be cleared
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn test_undo_redo() {
        let mut editor = Editor::with_demo_content();

        // Get a shape and move it
        let shapes: Vec<_> = editor.document.descendants(editor.document.root).collect();
        let shape_id = shapes[0];

        let original_pos = editor.document.get(shape_id).unwrap().transform.translation;

        // Simulate drag
        editor.selection.select(shape_id);
        let transaction = Transaction::start_move("Move", &[shape_id], &mut editor.document);
        transaction.translate(Vec2::new(50.0, 50.0), &mut editor.document);
        let cmd = transaction.commit(&editor.document);
        editor.history.push(cmd);

        let moved_pos = editor.document.get(shape_id).unwrap().transform.translation;
        assert_ne!(original_pos, moved_pos);

        // Undo
        editor.history.undo(&mut editor.document);
        let after_undo = editor.document.get(shape_id).unwrap().transform.translation;
        assert_eq!(original_pos, after_undo);

        // Redo
        editor.history.redo(&mut editor.document);
        let after_redo = editor.document.get(shape_id).unwrap().transform.translation;
        assert_eq!(moved_pos, after_redo);
    }

    #[test]
    fn deleting_a_container_keeps_saved_document_loadable_and_undo_restores_subtree() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let group = editor
            .document
            .add_child(root, Node::group("Group"))
            .unwrap();
        let nested = editor
            .document
            .add_child(group, Node::group("Nested"))
            .unwrap();
        let child = editor
            .document
            .add_child(
                nested,
                Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();

        editor.selection.select(group);
        editor.delete_selection();

        for id in [group, nested, child] {
            assert!(editor.document.get(id).unwrap().deleted);
        }
        let deleted_json = editor.document.to_json().unwrap();
        assert!(Document::from_json(&deleted_json).is_ok());

        assert!(editor.undo_in_context());
        for id in [group, nested, child] {
            assert!(!editor.document.get(id).unwrap().deleted);
        }
        assert_eq!(editor.document.get(root).unwrap().children, vec![group]);
        assert_eq!(editor.document.get(group).unwrap().children, vec![nested]);
        assert_eq!(editor.document.get(nested).unwrap().children, vec![child]);

        assert!(editor.redo_in_context());
        let redone_json = editor.document.to_json().unwrap();
        assert!(Document::from_json(&redone_json).is_ok());
    }

    #[test]
    fn pointer_up_applies_final_move_position() {
        let (mut editor, ids) = editor_with_named_shapes(&["A"]);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(5.0, 5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(25.0, 5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        assert_eq!(
            editor.document.get(ids[0]).unwrap().transform.translation,
            Vec2::new(20.0, 0.0)
        );
    }

    #[test]
    fn pointer_up_applies_final_marquee_position() {
        let (mut editor, ids) = editor_with_named_shapes(&["A"]);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(-5.0, -5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(15.0, 15.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        assert!(editor.selection.contains(ids[0]));
    }

    #[test]
    fn multi_selection_z_order_is_stable_and_undoable() {
        let (mut editor, ids) = editor_with_named_shapes(&["A", "B", "C", "D"]);
        let root = editor.document.root;
        editor.selection.set([ids[1], ids[3]]);

        editor.bring_selection_to_front();
        assert_eq!(child_names(&editor, root), ["A", "C", "B", "D"]);

        assert!(editor.history.undo(&mut editor.document));
        assert_eq!(child_names(&editor, root), ["A", "B", "C", "D"]);

        assert!(editor.history.redo(&mut editor.document));
        assert_eq!(child_names(&editor, root), ["A", "C", "B", "D"]);
    }

    #[test]
    fn multi_selection_moves_forward_as_one_ordered_set() {
        let (mut editor, ids) = editor_with_named_shapes(&["A", "B", "C", "D"]);
        let root = editor.document.root;
        editor.selection.set([ids[0], ids[2]]);

        editor.bring_selection_forward();

        assert_eq!(child_names(&editor, root), ["B", "A", "D", "C"]);
    }

    #[test]
    fn duplicate_selection_keeps_each_copy_beside_its_source() {
        let (mut editor, ids) = editor_with_named_shapes(&["A", "B", "C"]);
        let root = editor.document.root;
        editor.selection.set([ids[0], ids[1]]);

        editor.duplicate_selection();
        assert_eq!(
            child_names(&editor, root),
            ["A", "A copy", "B", "B copy", "C"]
        );

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(child_names(&editor, root), ["A", "B", "C"]);
        assert!(editor.selection.is_empty());

        assert!(editor.execute_action(EditorAction::Redo));
        assert_eq!(
            child_names(&editor, root),
            ["A", "A copy", "B", "B copy", "C"]
        );
    }

    #[test]
    fn object_clipboard_preserves_subtrees_world_space_and_history() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let parent = editor
            .document
            .add_child(
                root,
                Node::group("Parent").with_transform(Affine2::from_scale_angle_translation(
                    Vec2::new(1.5, 0.75),
                    0.3,
                    Vec2::new(40.0, 25.0),
                )),
            )
            .unwrap();
        let group = editor
            .document
            .add_child(
                parent,
                Node::group("Copied group")
                    .with_transform(Affine2::from_translation(Vec2::new(8.0, 6.0))),
            )
            .unwrap();
        let child = editor
            .document
            .add_child(
                group,
                Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 12.0))
                    .with_transform(Affine2::from_translation(Vec2::new(3.0, 4.0))),
            )
            .unwrap();
        let group_world = editor.document.world_transform(group);
        let child_world = editor.document.world_transform(child);
        editor.selection.set([child, group]);
        let mut clipboard = editor.copy_selection().expect("object clipboard");

        assert!(editor.paste_clipboard(&mut clipboard));

        let pasted_group = editor.selection.primary().expect("pasted root");
        let pasted_child = editor.document.get(pasted_group).unwrap().children[0];
        assert_eq!(
            editor.document.get(pasted_group).unwrap().parent,
            Some(root)
        );
        assert_affine_approx_eq(
            editor.document.world_transform(pasted_group),
            Affine2::from_translation(Vec2::splat(10.0)) * group_world,
        );
        assert_affine_approx_eq(
            editor.document.world_transform(pasted_child),
            Affine2::from_translation(Vec2::splat(10.0)) * child_world,
        );

        assert!(editor.execute_action(EditorAction::Undo));
        assert!(editor.document.get(pasted_group).unwrap().deleted);
        assert!(editor.document.get(pasted_child).unwrap().deleted);
        assert!(editor.execute_action(EditorAction::Redo));
        assert!(!editor.document.get(pasted_group).unwrap().deleted);
        assert!(!editor.document.get(pasted_child).unwrap().deleted);

        assert!(editor.paste_clipboard(&mut clipboard));
        let second_group = editor.selection.primary().expect("second pasted root");
        assert_affine_approx_eq(
            editor.document.world_transform(second_group),
            Affine2::from_translation(Vec2::splat(20.0)) * group_world,
        );
    }

    #[test]
    fn group_is_enabled_only_for_two_or_more_siblings() {
        let (mut editor, ids) = editor_with_named_shapes(&["A", "B"]);

        editor.selection.select(ids[0]);
        assert!(!editor.can_execute(EditorAction::Group));

        editor.selection.set([ids[0], ids[1]]);
        assert!(editor.can_execute(EditorAction::Group));

        let group = editor
            .document
            .add_child(editor.document.root, Node::group("Other parent"))
            .unwrap();
        let nested = editor
            .document
            .add_child(
                group,
                Node::shape("Nested", PathData::rect(0.0, 0.0, 1.0, 1.0)),
            )
            .unwrap();
        editor.selection.set([ids[0], nested]);
        assert!(!editor.can_execute(EditorAction::Group));
    }

    #[test]
    fn alignment_is_undoable_and_ignores_selected_descendants() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let group = editor
            .document
            .add_child(
                root,
                Node::group("Group")
                    .with_transform(Affine2::from_translation(Vec2::new(30.0, 0.0))),
            )
            .unwrap();
        let child = editor
            .document
            .add_child(
                group,
                Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let sibling = editor
            .document
            .add_child(
                root,
                Node::shape("Sibling", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::new(80.0, 0.0))),
            )
            .unwrap();
        let child_before = editor.document.world_transform(child);
        let sibling_before = editor.document.world_transform(sibling);
        editor.selection.set([group, child, sibling]);

        assert!(editor.can_execute(EditorAction::AlignLeft));
        assert!(!editor.can_execute(EditorAction::DistributeHorizontal));
        assert!(editor.execute_action(EditorAction::AlignLeft));
        assert_eq!(
            editor.document.world_bounds(group).unwrap().min.x,
            editor.document.world_bounds(sibling).unwrap().min.x
        );
        assert_affine_approx_eq(editor.document.world_transform(child), child_before);

        assert!(editor.execute_action(EditorAction::Undo));
        assert_affine_approx_eq(editor.document.world_transform(child), child_before);
        assert_affine_approx_eq(editor.document.world_transform(sibling), sibling_before);
    }

    #[test]
    fn arrangement_actions_are_disabled_when_a_transform_root_has_no_bounds() {
        let (mut editor, ids) = editor_with_named_shapes(&["A", "B", "C"]);
        let empty_group = editor
            .document
            .add_child(editor.document.root, Node::group("Empty"))
            .unwrap();

        editor.selection.set([empty_group, ids[0]]);
        assert!(!editor.can_execute(EditorAction::AlignLeft));

        editor.selection.set([empty_group, ids[0], ids[1]]);
        assert!(!editor.can_execute(EditorAction::DistributeHorizontal));

        editor.selection.set([ids[0], ids[1]]);
        assert!(editor.can_execute(EditorAction::AlignLeft));
        editor.selection.set(ids);
        assert!(editor.can_execute(EditorAction::DistributeHorizontal));
    }

    #[test]
    fn inspector_style_and_transform_edits_are_undoable() {
        let (mut editor, ids) = editor_with_named_shapes(&["First", "Second"]);
        editor.selection.set(ids.iter().copied());
        let before = ids
            .iter()
            .map(|id| editor.document.world_transform(*id))
            .collect::<Vec<_>>();

        assert!(editor.move_selection_by(Vec2::new(12.0, -4.0)));
        for (&id, &before) in ids.iter().zip(&before) {
            assert_affine_approx_eq(
                editor.document.world_transform(id),
                Affine2::from_translation(Vec2::new(12.0, -4.0)) * before,
            );
        }
        assert!(editor.execute_action(EditorAction::Undo));
        for (&id, &before) in ids.iter().zip(&before) {
            assert_affine_approx_eq(editor.document.world_transform(id), before);
        }

        assert!(editor.set_selected_opacity(0.4));
        assert!(ids
            .iter()
            .all(|id| editor.document.get(*id).unwrap().style.opacity == 0.4));
        assert!(editor.set_selected_fill(Some(Paint::rgb(0.2, 0.4, 0.8))));
        assert!(editor.toggle_selected_stroke());
        assert!(editor.set_selected_stroke_paint(Paint::rgb(0.9, 0.1, 0.3)));
        assert!(editor.adjust_selected_stroke_width(2.0));
        assert!(ids.iter().all(|id| {
            let style = &editor.document.get(*id).unwrap().style;
            style.fill == Some(Paint::rgb(0.2, 0.4, 0.8))
                && style.stroke.as_ref().is_some_and(|stroke| {
                    stroke.width == 3.0 && stroke.paint == Paint::rgb(0.9, 0.1, 0.3)
                })
        }));

        let transform_before_rotation = ids
            .iter()
            .map(|id| editor.document.world_transform(*id))
            .collect::<Vec<_>>();
        assert!(editor.rotate_selection_by(std::f32::consts::FRAC_PI_2));
        assert!(ids
            .iter()
            .zip(&transform_before_rotation)
            .any(|(id, before)| editor.document.world_transform(*id) != *before));
        assert!(editor.execute_action(EditorAction::Undo));
        for (&id, &before) in ids.iter().zip(&transform_before_rotation) {
            assert_affine_approx_eq(editor.document.world_transform(id), before);
        }
    }

    #[test]
    fn grouping_uses_scene_order_not_selection_hash_order() {
        let (mut editor, ids) = editor_with_named_shapes(&["A", "B", "C"]);
        let root = editor.document.root;
        editor.selection.set([ids[2], ids[0]]);

        editor.group_selection();

        let group_id = editor.selection.primary().unwrap();
        assert_eq!(child_names(&editor, root), ["Group", "B"]);
        assert_eq!(child_names(&editor, group_id), ["A", "C"]);

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(child_names(&editor, root), ["A", "B", "C"]);
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn grouping_in_transformed_auto_layout_preserves_selected_world_transforms() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let parent = editor
            .document
            .add_child(
                root,
                Node::group("Auto layout")
                    .with_layout(crate::Layout::Auto(
                        crate::AutoLayout::horizontal()
                            .with_spacing(13.0)
                            .with_uniform_padding(7.0)
                            .with_align_cross(crate::AlignCross::End),
                    ))
                    .with_transform(Affine2::from_scale_angle_translation(
                        Vec2::new(1.7, 0.8),
                        0.4,
                        Vec2::new(40.0, 25.0),
                    )),
            )
            .unwrap();
        let first = editor
            .document
            .add_child(
                parent,
                Node::shape("A", PathData::rect(0.0, 0.0, 10.0, 10.0)).with_transform(
                    Affine2::from_scale_angle_translation(Vec2::ONE, 0.2, Vec2::new(2.0, 3.0)),
                ),
            )
            .unwrap();
        editor
            .document
            .add_child(
                parent,
                Node::shape("B", PathData::rect(0.0, 0.0, 14.0, 8.0)),
            )
            .unwrap();
        let last = editor
            .document
            .add_child(
                parent,
                Node::shape("C", PathData::rect(0.0, 0.0, 12.0, 16.0)).with_transform(
                    Affine2::from_scale_angle_translation(Vec2::ONE, -0.3, Vec2::new(-4.0, 5.0)),
                ),
            )
            .unwrap();
        let first_world = editor.document.world_transform(first);
        let last_world = editor.document.world_transform(last);
        editor.selection.set([last, first]);

        editor.group_selection();

        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(last), last_world);

        assert!(editor.history.undo(&mut editor.document));
        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(last), last_world);

        assert!(editor.history.redo(&mut editor.document));
        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(last), last_world);
    }

    #[test]
    fn layout_mode_transitions_preserve_existing_child_world_transforms() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let group = editor
            .document
            .add_child(
                root,
                Node::group("Group").with_transform(Affine2::from_scale_angle_translation(
                    Vec2::new(1.5, 0.75),
                    0.35,
                    Vec2::new(40.0, 25.0),
                )),
            )
            .unwrap();
        let first = editor
            .document
            .add_child(
                group,
                Node::shape("A", PathData::rect(0.0, 0.0, 10.0, 8.0))
                    .with_transform(Affine2::from_translation(Vec2::new(3.0, 7.0))),
            )
            .unwrap();
        let second = editor
            .document
            .add_child(
                group,
                Node::shape("B", PathData::rect(0.0, 0.0, 12.0, 6.0))
                    .with_transform(Affine2::from_translation(Vec2::new(28.0, 4.0))),
            )
            .unwrap();
        let first_world = editor.document.world_transform(first);
        let second_world = editor.document.world_transform(second);
        editor.selection.select(group);

        assert!(editor.set_selected_group_layout_mode(GroupLayoutMode::Horizontal));
        assert!(matches!(
            editor.selected_group_layout(),
            Some(Layout::Auto(AutoLayout {
                direction: Direction::Horizontal,
                ..
            }))
        ));
        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(second), second_world);

        assert!(editor.set_selected_group_layout_mode(GroupLayoutMode::Free));
        assert_eq!(editor.selected_group_layout(), Some(Layout::Free));
        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(second), second_world);

        assert!(editor.history.undo(&mut editor.document));
        assert!(matches!(
            editor.selected_group_layout(),
            Some(Layout::Auto(_))
        ));
        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(second), second_world);

        assert!(editor.history.redo(&mut editor.document));
        assert_eq!(editor.selected_group_layout(), Some(Layout::Free));
        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(second), second_world);
    }

    #[test]
    fn auto_layout_inspector_edits_are_validated_and_undoable() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let group = editor
            .document
            .add_child(
                root,
                Node::group("Group").with_layout(Layout::Auto(AutoLayout::horizontal())),
            )
            .unwrap();
        editor.selection.select(group);

        assert!(!editor.set_selected_group_spacing(f32::NAN));
        assert!(!editor.set_selected_group_spacing(-1.0));
        assert!(!editor.set_selected_group_uniform_padding(f32::INFINITY));
        assert!(!editor.set_selected_group_uniform_padding(-0.5));
        assert_eq!(editor.history.undo_count(), 0);

        assert!(editor.set_selected_group_spacing(12.0));
        assert!(editor.set_selected_group_uniform_padding(8.0));
        assert!(editor.set_selected_group_main_alignment(AlignMain::SpaceBetween));
        assert!(editor.set_selected_group_cross_alignment(AlignCross::End));
        let Some(Layout::Auto(layout)) = editor.selected_group_layout() else {
            panic!("selected group should use auto layout");
        };
        assert_eq!(layout.spacing, 12.0);
        assert_eq!(layout.padding, crate::Edges::uniform(8.0));
        assert_eq!(layout.align_main, AlignMain::SpaceBetween);
        assert_eq!(layout.align_cross, AlignCross::End);

        assert!(editor.history.undo(&mut editor.document));
        let Some(Layout::Auto(layout)) = editor.selected_group_layout() else {
            panic!("selected group should use auto layout");
        };
        assert_eq!(layout.align_cross, AlignCross::Start);
        assert!(editor.history.redo(&mut editor.document));
        let Some(Layout::Auto(layout)) = editor.selected_group_layout() else {
            panic!("selected group should use auto layout");
        };
        assert_eq!(layout.align_cross, AlignCross::End);
    }

    #[test]
    fn transform_components_report_rotation_scale_reflection_and_skew_safely() {
        let transform =
            Affine2::from_scale_angle_translation(Vec2::new(2.0, -3.0), 0.4, Vec2::new(12.0, 18.0));
        let components = TransformComponents::from_affine(transform).unwrap();
        assert!((components.rotation_radians - 0.4).abs() < 0.0001);
        assert!((components.scale.x - 2.0).abs() < 0.0001);
        assert!((components.scale.y + 3.0).abs() < 0.0001);
        assert!(components.skew_radians.abs() < 0.0001);
        assert_eq!(components.translation, Vec2::new(12.0, 18.0));

        let sheared = Affine2::from_mat2_translation(
            glam::Mat2::from_cols(Vec2::new(2.0, 0.0), Vec2::new(1.0, 3.0)),
            Vec2::ZERO,
        );
        let components = TransformComponents::from_affine(sheared).unwrap();
        assert!((components.skew_radians - 1.0f32.atan2(3.0)).abs() < 0.0001);

        let reflected_skew = 0.25_f32;
        let reflected_sheared = Affine2::from_mat2_translation(
            glam::Mat2::from_cols(
                Vec2::new(2.0, 0.0),
                Vec2::new(reflected_skew.tan() * -3.0, -3.0),
            ),
            Vec2::ZERO,
        );
        let components = TransformComponents::from_affine(reflected_sheared).unwrap();
        assert!((components.scale.y + 3.0).abs() < 0.0001);
        assert!((components.skew_radians - reflected_skew).abs() < 0.0001);

        assert!(TransformComponents::from_affine(Affine2::from_scale(Vec2::ZERO)).is_none());
        assert!(
            TransformComponents::from_affine(Affine2::from_translation(Vec2::splat(f32::NAN)))
                .is_none()
        );
    }

    #[test]
    fn ungroup_preserves_child_world_transform() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let group = Node::group("Group").with_transform(Affine2::from_scale_angle_translation(
            Vec2::splat(2.0),
            0.3,
            Vec2::new(40.0, 25.0),
        ));
        let group_id = editor.document.add_child(root, group).unwrap();
        let child = Node::shape("A", PathData::rect(0.0, 0.0, 10.0, 10.0))
            .with_transform(Affine2::from_translation(Vec2::new(8.0, 6.0)));
        let child_id = editor.document.add_child(group_id, child).unwrap();
        let before = editor.document.world_transform(child_id);
        editor.selection.select(group_id);

        editor.ungroup_selection();

        assert_eq!(editor.document.get(child_id).unwrap().parent, Some(root));
        let after = editor.document.world_transform(child_id);
        assert_affine_approx_eq(after, before);

        assert!(editor.history.undo(&mut editor.document));
        assert_eq!(
            editor.document.get(child_id).unwrap().parent,
            Some(group_id)
        );
        assert_affine_approx_eq(editor.document.world_transform(child_id), before);
    }

    #[test]
    fn ungroup_in_transformed_auto_layout_preserves_world_transforms_and_history() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let parent = editor
            .document
            .add_child(
                root,
                Node::group("Auto layout")
                    .with_layout(crate::Layout::Auto(
                        crate::AutoLayout::horizontal()
                            .with_spacing(9.0)
                            .with_uniform_padding(6.0)
                            .with_align_cross(crate::AlignCross::Center),
                    ))
                    .with_transform(Affine2::from_scale_angle_translation(
                        Vec2::new(1.6, 0.9),
                        0.35,
                        Vec2::new(45.0, 20.0),
                    )),
            )
            .unwrap();
        editor
            .document
            .add_child(
                parent,
                Node::shape("Before", PathData::rect(0.0, 0.0, 13.0, 10.0)),
            )
            .unwrap();
        let group = editor
            .document
            .add_child(
                parent,
                Node::group("Group").with_transform(Affine2::from_scale_angle_translation(
                    Vec2::new(0.75, 1.2),
                    -0.2,
                    Vec2::new(4.0, 3.0),
                )),
            )
            .unwrap();
        let first = editor
            .document
            .add_child(
                group,
                Node::shape("A", PathData::rect(0.0, 0.0, 10.0, 8.0))
                    .with_transform(Affine2::from_translation(Vec2::new(2.0, 1.0))),
            )
            .unwrap();
        let second = editor
            .document
            .add_child(
                group,
                Node::shape("B", PathData::rect(0.0, 0.0, 7.0, 12.0))
                    .with_transform(Affine2::from_translation(Vec2::new(16.0, -2.0))),
            )
            .unwrap();
        editor
            .document
            .add_child(
                parent,
                Node::shape("After", PathData::rect(0.0, 0.0, 11.0, 9.0)),
            )
            .unwrap();
        let first_world = editor.document.world_transform(first);
        let second_world = editor.document.world_transform(second);
        editor.selection.select(group);

        editor.ungroup_selection();

        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(second), second_world);

        assert!(editor.history.undo(&mut editor.document));
        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(second), second_world);

        assert!(editor.history.redo(&mut editor.document));
        assert_affine_approx_eq(editor.document.world_transform(first), first_world);
        assert_affine_approx_eq(editor.document.world_transform(second), second_world);
    }

    #[test]
    fn ungrouping_multiple_siblings_keeps_scene_order() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let first_group = editor
            .document
            .add_child(root, Node::group("First"))
            .unwrap();
        editor
            .document
            .add_child(
                first_group,
                Node::shape("A", PathData::rect(0.0, 0.0, 1.0, 1.0)),
            )
            .unwrap();
        editor
            .document
            .add_child(
                first_group,
                Node::shape("B", PathData::rect(0.0, 0.0, 1.0, 1.0)),
            )
            .unwrap();
        editor
            .document
            .add_child(root, Node::shape("X", PathData::rect(0.0, 0.0, 1.0, 1.0)))
            .unwrap();
        let second_group = editor
            .document
            .add_child(root, Node::group("Second"))
            .unwrap();
        editor
            .document
            .add_child(
                second_group,
                Node::shape("C", PathData::rect(0.0, 0.0, 1.0, 1.0)),
            )
            .unwrap();
        editor
            .document
            .add_child(
                second_group,
                Node::shape("D", PathData::rect(0.0, 0.0, 1.0, 1.0)),
            )
            .unwrap();
        editor.selection.set([first_group, second_group]);

        editor.ungroup_selection();

        assert_eq!(child_names(&editor, root), ["A", "B", "X", "C", "D"]);
    }

    #[test]
    fn zoom_to_fit_uses_configured_viewport() {
        let (mut editor, _) = editor_with_named_shapes(&["A"]);
        editor.set_viewport_size(Vec2::new(1000.0, 500.0));

        editor.zoom_to_fit();

        assert_eq!(editor.view.zoom, 10.0);
        assert_eq!(editor.view.pan, Vec2::new(450.0, 200.0));
    }

    #[test]
    fn zoom_to_fit_includes_visible_content_overflowing_a_frame() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let frame = editor
            .document
            .add_child(root, Node::frame("Frame", 100.0, 100.0))
            .unwrap();
        editor
            .document
            .add_child(
                frame,
                Node::shape("Overflow", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::new(200.0, 0.0))),
            )
            .unwrap();
        editor.set_viewport_size(Vec2::new(1000.0, 500.0));

        editor.zoom_to_fit();

        assert_eq!(editor.view.zoom, 4.0);
        assert_eq!(editor.view.pan, Vec2::new(80.0, 50.0));
    }

    #[test]
    fn zoom_to_selection_handles_horizontal_and_vertical_lines() {
        for (name, end) in [
            ("Horizontal", Vec2::new(100.0, 0.0)),
            ("Vertical", Vec2::new(0.0, 100.0)),
        ] {
            let mut editor = Editor::new();
            editor.set_viewport_size(Vec2::new(800.0, 600.0));
            let path = PathData {
                contours: vec![PathContour::open([
                    PathAnchor::corner(Vec2::ZERO),
                    PathAnchor::corner(end),
                ])],
            };
            let id = editor
                .document
                .add_child(
                    editor.document.root,
                    Node::shape(name, path).with_style(Style::stroke(Stroke::black(1.5))),
                )
                .unwrap();
            editor.selection.select(id);

            editor.zoom_to_selection();

            assert!(editor.view.zoom > 1.0, "{name} should be fitted");
            assert!(
                (editor.view.to_screen(end * 0.5) - Vec2::new(400.0, 300.0)).length() < 0.001,
                "{name} should be centered"
            );
        }
    }

    #[test]
    fn absolute_zoom_preserves_the_viewport_center() {
        let mut editor = Editor::new();
        editor.set_viewport_size(Vec2::new(1000.0, 600.0));
        editor.view.pan = Vec2::new(90.0, -40.0);
        let center = Vec2::new(500.0, 300.0);
        let world_before = editor.view.to_world(center);

        assert!(editor.set_zoom(2.5));

        assert_eq!(editor.view.zoom, 2.5);
        assert!((editor.view.to_world(center) - world_before).length() < 0.001);
        assert!(editor.take_redraw());
    }

    #[test]
    fn repeated_zoom_actions_stop_at_supported_limits() {
        let mut editor = Editor::new();

        for _ in 0..100 {
            editor.execute_action(EditorAction::ZoomOut);
        }
        assert_eq!(editor.view.zoom, crate::transform::MIN_ZOOM);

        for _ in 0..100 {
            editor.execute_action(EditorAction::ZoomIn);
        }
        assert_eq!(editor.view.zoom, crate::transform::MAX_ZOOM);
    }

    #[test]
    fn view_changes_cancel_active_move_previews() {
        let (mut editor, ids) = editor_with_named_shapes(&["Shape"]);
        editor.selection.select(ids[0]);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::splat(25.0),
            modifiers: Modifiers::default(),
        });

        assert!(editor.execute_action(EditorAction::ZoomIn));
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(
            editor.document.get(ids[0]).unwrap().transform,
            Affine2::IDENTITY
        );
        assert!(!editor.history.can_undo());

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::splat(25.0),
            modifiers: Modifiers::default(),
        });
        let effects = editor.handle_event(InputEvent::Scroll {
            position: Vec2::splat(50.0),
            delta: Vec2::new(0.0, 10.0),
            modifiers: Modifiers::default(),
        });

        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(
            editor.document.get(ids[0]).unwrap().transform,
            Affine2::IDENTITY
        );
        assert!(!editor.history.can_undo());
        assert!(effects.cursor.is_some());
    }

    #[test]
    fn horizontal_primary_scroll_does_not_zoom() {
        let (mut editor, ids) = editor_with_named_shapes(&["Shape"]);
        editor.selection.select(ids[0]);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::splat(25.0),
            modifiers: Modifiers::default(),
        });
        let zoom = editor.view.zoom;
        let interaction = editor.interaction_kind();

        let effects = editor.handle_event(InputEvent::Scroll {
            position: Vec2::splat(50.0),
            delta: Vec2::new(10.0, 0.0),
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        });

        assert_eq!(editor.view.zoom, zoom);
        assert_eq!(editor.interaction_kind(), interaction);
        assert!(!effects.redraw);
    }

    #[test]
    fn artwork_snapshot_uses_world_space_and_excludes_hidden_content() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        editor
            .document
            .add_child(
                root,
                Node::shape("Visible", PathData::rect(0.0, 0.0, 20.0, 10.0))
                    .with_style(Style::fill_and_stroke(Paint::black(), Stroke::black(2.0)))
                    .with_transform(Affine2::from_translation(Vec2::new(-25.0, 40.0))),
            )
            .unwrap();
        editor
            .document
            .add_child(
                root,
                Node::shape("Hidden", PathData::rect(0.0, 0.0, 100.0, 100.0))
                    .with_visible(false)
                    .with_transform(Affine2::from_translation(Vec2::splat(1_000.0))),
            )
            .unwrap();
        editor.execute_action(EditorAction::ZoomIn);

        let snapshot = editor.artwork_snapshot().unwrap();

        assert_eq!(snapshot.bounds.min, Vec2::new(-29.0, 36.0));
        assert_eq!(snapshot.bounds.max, Vec2::new(-1.0, 54.0));
        assert_eq!(snapshot.display_list.len(), 2);
        let editor_render::DisplayItem::FillPath { transform, .. } =
            &snapshot.display_list.items[0]
        else {
            panic!("first visible display item should be the shape fill");
        };
        assert_eq!(transform.translation, Vec2::new(-25.0, 40.0));
    }

    #[test]
    fn artwork_snapshot_is_absent_for_an_empty_document() {
        assert!(Editor::new().artwork_snapshot().is_none());
    }

    #[test]
    fn shape_tool_actions_are_executable() {
        let mut editor = Editor::new();

        assert!(editor.execute_action(EditorAction::ToolRectangle));
        assert_eq!(editor.tool, Tool::Rectangle);
        assert_eq!(editor.cursor(), Cursor::Crosshair);

        assert!(editor.execute_action(EditorAction::ToolEllipse));
        assert_eq!(editor.tool, Tool::Ellipse);

        assert!(editor.execute_action(EditorAction::ToolFrame));
        assert_eq!(editor.tool, Tool::Frame);
        assert!(editor.execute_action(EditorAction::ToolPen));
        assert_eq!(editor.tool, Tool::Pen);
        assert!(editor.execute_action(EditorAction::ToolText));
        assert_eq!(editor.tool, Tool::Text);
    }

    #[test]
    fn nudge_moves_in_document_units_under_zoom_and_transformed_parent() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let parent = editor
            .document
            .add_child(
                root,
                Node::group("Parent").with_transform(Affine2::from_scale_angle_translation(
                    Vec2::new(1.7, 0.8),
                    0.35,
                    Vec2::new(30.0, 20.0),
                )),
            )
            .unwrap();
        let child = editor
            .document
            .add_child(
                parent,
                Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::new(4.0, 6.0))),
            )
            .unwrap();
        editor.view.zoom = 4.0;
        editor.selection.select(child);
        let before = editor.document.world_transform(child);

        assert!(editor.execute_action(EditorAction::NudgeRight));

        let after = editor.document.world_transform(child);
        assert_affine_approx_eq(after, Affine2::from_translation(Vec2::X) * before);

        assert!(editor.execute_action(EditorAction::Undo));
        assert_affine_approx_eq(editor.document.world_transform(child), before);
    }

    #[test]
    fn frame_creation_adopts_only_enclosed_siblings_and_preserves_world_space() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let enclosed = editor
            .document
            .add_child(
                root,
                Node::shape("Inside", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::new(20.0, 20.0))),
            )
            .unwrap();
        let outside = editor
            .document
            .add_child(
                root,
                Node::shape("Outside", PathData::rect(0.0, 0.0, 20.0, 20.0))
                    .with_transform(Affine2::from_translation(Vec2::new(130.0, 80.0))),
            )
            .unwrap();
        let enclosed_world = editor.document.world_transform(enclosed);

        editor.execute_action(EditorAction::ToolFrame);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::ZERO,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::splat(100.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let frame = editor.selection.primary().unwrap();
        let NodeKind::Frame(data) = &editor.document.get(frame).unwrap().kind else {
            panic!("frame tool must create a frame");
        };
        assert_eq!((data.width, data.height), (100.0, 100.0));
        assert_eq!(data.background, Some(Paint::white()));
        assert_eq!(editor.document.get(enclosed).unwrap().parent, Some(frame));
        assert_eq!(editor.document.get(outside).unwrap().parent, Some(root));
        assert_affine_approx_eq(editor.document.world_transform(enclosed), enclosed_world);
        assert_eq!(child_names(&editor, root), ["Frame", "Outside"]);
        assert_eq!(child_names(&editor, frame), ["Inside"]);

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(child_names(&editor, root), ["Inside", "Outside"]);
        assert_eq!(editor.document.get(enclosed).unwrap().parent, Some(root));
        assert_affine_approx_eq(editor.document.world_transform(enclosed), enclosed_world);

        assert!(editor.execute_action(EditorAction::Redo));
        assert_eq!(editor.document.get(enclosed).unwrap().parent, Some(frame));
        assert_affine_approx_eq(editor.document.world_transform(enclosed), enclosed_world);
    }

    #[test]
    fn pen_session_commits_once_and_supports_local_anchor_undo() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolPen);

        for position in [Vec2::new(10.0, 20.0), Vec2::new(40.0, 20.0)] {
            editor.handle_event(InputEvent::PointerDown {
                position,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            });
            editor.handle_event(InputEvent::PointerUp {
                position,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            });
        }
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Backspace,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Z,
            modifiers: Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::default()
            },
        });
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });

        let path_id = editor.selection.primary().unwrap();
        let path = editor.document.get(path_id).unwrap().path().unwrap();
        assert_eq!(path.contours.len(), 1);
        assert_eq!(path.contours[0].anchors.len(), 2);
        assert!(!path.contours[0].closed);
        assert_eq!(editor.history.undo_description(), Some("Create Path"));
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.document.descendants(editor.document.root).count(), 0);
    }

    #[test]
    fn escape_finishes_pen_session_without_switching_tools() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolPen);
        for position in [Vec2::ZERO, Vec2::new(20.0, 0.0)] {
            editor.handle_event(InputEvent::PointerDown {
                position,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            });
            editor.handle_event(InputEvent::PointerUp {
                position,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
            });
        }

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });

        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.tool, Tool::Pen);
        assert_eq!(editor.history.undo_description(), Some("Create Path"));
        assert!(editor.selection.primary().is_some());
    }

    #[test]
    fn space_pan_temporarily_suspends_and_restores_pen_editing() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolPen);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(10.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::Pen);

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Space,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(50.0, 50.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(70.0, 60.0),
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::Panning);
        assert_eq!(editor.view.pan, Vec2::new(20.0, 10.0));
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(70.0, 60.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::Pen);
        editor.handle_event(InputEvent::KeyUp {
            key: Key::Space,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.cursor(), Cursor::Crosshair);
    }

    #[test]
    fn tool_switch_terminates_a_pen_session_suspended_by_space_pan() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolPen);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(10.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Space,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(20.0, 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::Panning);

        assert!(editor.execute_action(EditorAction::ToolSelect));

        assert_eq!(editor.tool, Tool::Select);
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.document.descendants(editor.document.root).count(), 0);
    }

    #[test]
    fn selecting_a_layer_settles_the_active_text_session() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let first = editor
            .document
            .add_child(root, Node::text("First", "A"))
            .unwrap();
        let second = editor
            .document
            .add_child(root, Node::text("Second", "B"))
            .unwrap();
        editor.begin_existing_text_edit(first);
        assert!(editor.replace_text(None, "!"));

        editor.select_layer(second, false);

        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.selection.primary(), Some(second));
        let NodeKind::Text(text) = &editor.document.get(first).unwrap().kind else {
            panic!("expected text");
        };
        assert_eq!(text.content, "A!");
        assert_eq!(editor.history.undo_description(), Some("Edit Text"));
    }

    #[test]
    fn layer_multi_selection_mode_toggles_and_single_selection_replaces() {
        let (mut editor, ids) = editor_with_named_shapes(&["First", "Second", "Third"]);

        editor.select_layer(ids[0], false);
        editor.select_layer(ids[1], true);
        assert_eq!(editor.selection.iter().collect::<Vec<_>>(), ids[..2]);
        assert_eq!(editor.selection.primary(), Some(ids[0]));

        editor.select_layer(ids[0], true);
        assert_eq!(editor.selection.iter().collect::<Vec<_>>(), [ids[1]]);
        assert_eq!(editor.selection.primary(), Some(ids[1]));

        editor.select_layer(ids[2], false);
        assert_eq!(editor.selection.iter().collect::<Vec<_>>(), [ids[2]]);
        assert_eq!(editor.selection.primary(), Some(ids[2]));
    }

    #[test]
    fn layer_tree_marks_descendants_of_locked_containers_uneditable() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let group = editor
            .document
            .add_child(root, Node::group("Locked group"))
            .unwrap();
        let child = editor
            .document
            .add_child(
                group,
                Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        editor.document.get_mut(group).unwrap().locked = true;

        let entries = editor.build_layer_tree();
        assert!(
            !entries
                .iter()
                .find(|entry| entry.id == group)
                .unwrap()
                .editable
        );
        assert!(
            !entries
                .iter()
                .find(|entry| entry.id == child)
                .unwrap()
                .editable
        );
    }

    #[test]
    fn layer_rename_is_validated_and_undoable() {
        let (mut editor, ids) = editor_with_named_shapes(&["Original"]);

        assert!(!editor.rename_layer(ids[0], "  "));
        assert!(!editor.history.can_undo());
        assert!(editor.rename_layer(ids[0], "  Renamed  "));
        assert_eq!(editor.document.get(ids[0]).unwrap().name, "Renamed");

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.document.get(ids[0]).unwrap().name, "Original");
        assert!(editor.execute_action(EditorAction::Redo));
        assert_eq!(editor.document.get(ids[0]).unwrap().name, "Renamed");
    }

    #[test]
    fn layer_reparent_preserves_world_transform_and_history() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let first_parent = editor
            .document
            .add_child(
                root,
                Node::group("First").with_transform(Affine2::from_scale_angle_translation(
                    Vec2::new(1.5, 0.75),
                    0.2,
                    Vec2::new(40.0, 20.0),
                )),
            )
            .unwrap();
        let second_parent = editor
            .document
            .add_child(
                root,
                Node::group("Second").with_transform(Affine2::from_scale_angle_translation(
                    Vec2::new(0.8, 1.25),
                    -0.3,
                    Vec2::new(180.0, 60.0),
                )),
            )
            .unwrap();
        let child = editor
            .document
            .add_child(
                first_parent,
                Node::shape("Child", PathData::rect(0.0, 0.0, 20.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::new(12.0, 8.0))),
            )
            .unwrap();
        let before_world = editor.document.world_transform(child);

        assert!(editor.reparent_layer(child, second_parent, 0));
        assert_eq!(
            editor.document.get(child).and_then(|node| node.parent),
            Some(second_parent)
        );
        assert_affine_approx_eq(editor.document.world_transform(child), before_world);

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(
            editor.document.get(child).and_then(|node| node.parent),
            Some(first_parent)
        );
        assert_affine_approx_eq(editor.document.world_transform(child), before_world);

        assert!(editor.execute_action(EditorAction::Redo));
        assert_eq!(
            editor.document.get(child).and_then(|node| node.parent),
            Some(second_parent)
        );
        assert_affine_approx_eq(editor.document.world_transform(child), before_world);
    }

    #[test]
    fn layer_reorder_uses_the_final_sibling_index() {
        let (mut editor, ids) = editor_with_named_shapes(&["Back", "Middle", "Front"]);
        let root = editor.document.root;

        assert!(editor.reparent_layer(ids[0], root, 2));
        assert_eq!(
            editor.document.get(root).unwrap().children,
            [ids[1], ids[2], ids[0]]
        );
        assert!(!editor.reparent_layer(ids[0], root, 2));

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.document.get(root).unwrap().children, ids);
    }

    #[test]
    fn layer_reparent_rejects_a_singular_destination() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let child = editor
            .document
            .add_child(
                root,
                Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let singular_parent = editor
            .document
            .add_child(
                root,
                Node::group("Singular").with_transform(Affine2::from_scale(Vec2::new(0.0, 1.0))),
            )
            .unwrap();

        assert!(!editor.reparent_layer(child, singular_parent, 0));
        assert_eq!(
            editor.document.get(child).and_then(|node| node.parent),
            Some(root)
        );
        assert!(!editor.history.can_undo());
    }

    #[test]
    fn uneditable_layer_selection_does_not_settle_active_editing() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let editing = editor
            .document
            .add_child(root, Node::text("Editing", "A"))
            .unwrap();
        let hidden = editor
            .document
            .add_child(root, Node::text("Hidden", "B"))
            .unwrap();
        editor.document.get_mut(hidden).unwrap().visible = false;
        let locked = editor
            .document
            .add_child(root, Node::text("Locked", "C"))
            .unwrap();
        editor.document.get_mut(locked).unwrap().locked = true;
        editor.begin_existing_text_edit(editing);
        assert!(editor.replace_text(None, "!"));

        for id in [hidden, locked] {
            editor.select_layer(id, false);
            assert_eq!(editor.interaction_kind(), InteractionKind::TextEditing);
            assert_eq!(editor.selection.primary(), Some(editing));
            assert!(!editor.history.can_undo());
        }

        let NodeKind::Text(text) = &editor.document.get(editing).unwrap().kind else {
            panic!("expected text");
        };
        assert_eq!(text.content, "A!");
    }

    #[test]
    fn marquee_ignores_hidden_and_ancestor_locked_nodes() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let hidden = editor
            .document
            .add_child(root, Node::group("Hidden"))
            .unwrap();
        editor.document.get_mut(hidden).unwrap().visible = false;
        let hidden_child = editor
            .document
            .add_child(
                hidden,
                Node::shape("Hidden child", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let locked = editor
            .document
            .add_child(root, Node::group("Locked"))
            .unwrap();
        editor.document.get_mut(locked).unwrap().locked = true;
        let locked_child = editor
            .document
            .add_child(
                locked,
                Node::shape("Locked child", PathData::rect(20.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();

        let selected =
            editor.compute_marquee_selection(Vec2::splat(-5.0), Vec2::new(35.0, 15.0), true);

        assert!(!selected.contains(&hidden_child));
        assert!(!selected.contains(&locked_child));
        assert!(selected.is_empty());
    }

    #[test]
    fn marquee_uses_selection_scope_and_deep_mode_returns_only_leaves() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let frame = editor
            .document
            .add_child(root, Node::frame("Frame", 100.0, 100.0))
            .unwrap();
        let group = editor
            .document
            .add_child(frame, Node::group("Group"))
            .unwrap();
        let first = editor
            .document
            .add_child(
                group,
                Node::shape("First", PathData::rect(0.0, 0.0, 20.0, 20.0)),
            )
            .unwrap();
        let second = editor
            .document
            .add_child(
                group,
                Node::shape("Second", PathData::rect(30.0, 0.0, 20.0, 20.0)),
            )
            .unwrap();
        let start = Vec2::splat(-5.0);
        let end = Vec2::new(105.0, 105.0);

        assert_eq!(
            editor.compute_marquee_selection(start, end, false),
            vec![frame]
        );
        assert_eq!(
            editor.compute_marquee_selection(start, end, true),
            vec![first, second]
        );

        editor.selection.select(first);
        assert_eq!(
            editor.compute_marquee_selection(Vec2::new(5.0, 5.0), Vec2::new(55.0, 25.0), false),
            vec![first, second]
        );
        assert_eq!(
            editor.compute_marquee_selection(start, end, false),
            vec![frame]
        );
    }

    #[test]
    fn vector_edit_requires_exactly_one_selected_path() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let path = editor
            .document
            .add_child(
                root,
                Node::shape("Path", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let text = editor
            .document
            .add_child(root, Node::text("Text", "A"))
            .unwrap();

        editor.selection.select(text);
        assert!(!editor.can_execute(EditorAction::EnterVectorEdit));
        assert!(!editor.execute_action(EditorAction::EnterVectorEdit));

        editor.selection.set([path, text]);
        assert_eq!(editor.selection.primary(), Some(path));
        assert!(!editor.can_execute(EditorAction::EnterVectorEdit));
        assert!(!editor.execute_action(EditorAction::EnterVectorEdit));

        editor.selection.select(path);
        assert!(editor.can_execute(EditorAction::EnterVectorEdit));
        assert!(editor.execute_action(EditorAction::EnterVectorEdit));
        assert_eq!(editor.interaction_kind(), InteractionKind::VectorEditing);
    }

    #[test]
    fn entering_vector_edit_cancels_an_active_move_preview() {
        let (mut editor, ids) = editor_with_named_shapes(&["Shape"]);
        editor.selection.select(ids[0]);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::splat(25.0),
            modifiers: Modifiers::default(),
        });
        assert_ne!(
            editor.document.get(ids[0]).unwrap().transform,
            Affine2::IDENTITY
        );

        assert!(editor.execute_action(EditorAction::EnterVectorEdit));

        assert_eq!(editor.interaction_kind(), InteractionKind::VectorEditing);
        assert_eq!(
            editor.document.get(ids[0]).unwrap().transform,
            Affine2::IDENTITY
        );
    }

    #[test]
    fn escape_finishes_vector_edit_and_returns_to_select() {
        let (mut editor, ids) = editor_with_named_shapes(&["Path"]);
        editor.selection.select(ids[0]);
        assert!(editor.execute_action(EditorAction::EnterVectorEdit));

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });

        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.tool, Tool::Select);
        assert_eq!(editor.selection.primary(), Some(ids[0]));
    }

    #[test]
    fn escape_cancels_text_edit_without_advancing_idle_tool_ladder() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let id = editor
            .document
            .add_child(root, Node::text("Text", "A"))
            .unwrap();
        editor.begin_existing_text_edit(id);
        assert!(editor.replace_text(None, "!"));

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });

        let NodeKind::Text(text) = &editor.document.get(id).unwrap().kind else {
            panic!("expected text");
        };
        assert_eq!(text.content, "A");
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.tool, Tool::Text);
        assert_eq!(editor.selection.primary(), Some(id));
        assert!(!editor.history.can_undo());
    }

    #[test]
    fn typography_properties_are_canonical_and_undoable() {
        let mut editor = Editor::new();
        let text = editor
            .document
            .add_child(editor.document.root, Node::text("Label", "Hello"))
            .unwrap();
        editor.selection.select(text);

        assert!(!editor.set_selected_text_font_family("   "));
        assert!(editor.set_selected_text_font_family("  serif  "));
        assert!(!editor.set_selected_text_font_family("serif"));
        assert_eq!(
            editor.history.undo_description(),
            Some("Change Font Family")
        );
        assert!(editor.set_selected_text_font_weight(749));
        assert_eq!(
            editor.history.undo_description(),
            Some("Change Font Weight")
        );
        assert!(editor.set_selected_text_italic(true));
        assert!(!editor.set_selected_text_italic(true));
        assert_eq!(editor.history.undo_description(), Some("Change Font Style"));

        let typography = editor.selected_text_data().unwrap().font;
        assert_eq!(typography.family, "serif");
        assert_eq!(typography.weight, 700);
        assert!(typography.italic);

        assert!(editor.execute_action(EditorAction::Undo));
        assert!(!editor.selected_text_data().unwrap().font.italic);
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.selected_text_data().unwrap().font.weight, 400);
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(
            editor.selected_text_data().unwrap().font.family,
            "sans-serif"
        );

        assert_eq!(canonical_font_weight(0), 100);
        assert_eq!(canonical_font_weight(149), 100);
        assert_eq!(canonical_font_weight(150), 200);
        assert_eq!(canonical_font_weight(750), 800);
        assert_eq!(canonical_font_weight(999), 900);
    }

    #[test]
    fn text_editing_is_unicode_safe_and_one_history_item() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolText);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(30.0, 40.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(130.0, 70.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::TextEditing);
        assert!(editor.replace_text(None, "Hello 🇳🇴"));
        assert!(editor.delete_text_backward());
        assert_eq!(editor.text_input_snapshot().unwrap().text, "Hello ");
        assert!(editor.replace_text(None, "world"));
        assert!(editor.undo_text_edit());
        assert_eq!(editor.text_input_snapshot().unwrap().text, "Hello ");
        assert!(editor.redo_text_edit());
        assert_eq!(editor.text_input_snapshot().unwrap().text, "Hello world");
        assert!(editor.adjust_selected_text_size(1.0));
        assert!(editor.undo_text_edit());
        assert_eq!(editor.selected_text_data().unwrap().font_size, 16.0);
        assert!(editor.redo_text_edit());
        assert_eq!(editor.selected_text_data().unwrap().font_size, 17.0);
        assert!(editor.set_text_selection(0..5));
        assert!(editor
            .build_display_list()
            .items
            .iter()
            .any(|item| matches!(
                item,
                editor_render::DisplayItem::TextSelectionRect { marked: false, .. }
            )));
        assert!(editor.set_text_selection(11..11));
        assert!(editor.finish_editing());

        let text_id = editor.selection.primary().unwrap();
        let NodeKind::Text(text) = &editor.document.get(text_id).unwrap().kind else {
            panic!("text tool must create text");
        };
        assert_eq!(text.content, "Hello world");
        assert_eq!(text.sizing, TextSizing::FixedWidth { width: 100.0 });
        assert_eq!(editor.history.undo_description(), Some("Create Text"));
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.document.descendants(editor.document.root).count(), 0);
        assert!(editor.execute_action(EditorAction::Redo));
        assert_eq!(editor.document.descendants(editor.document.root).count(), 1);
    }

    #[test]
    fn direct_vector_edit_can_move_handles_and_insert_on_a_curve() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let path = PathData {
            contours: vec![PathContour::open([
                PathAnchor::mirrored(Vec2::ZERO, Vec2::new(-10.0, 0.0), Vec2::new(30.0, 0.0)),
                PathAnchor::mirrored(
                    Vec2::new(60.0, 0.0),
                    Vec2::new(-30.0, 0.0),
                    Vec2::new(10.0, 0.0),
                ),
            ])],
        };
        let id = editor
            .document
            .add_child(root, Node::shape("Curve", path))
            .unwrap();
        editor.selection.select(id);
        assert!(editor.execute_action(EditorAction::EnterVectorEdit));

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(30.0, 0.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(25.0, 15.0),
            modifiers: Modifiers {
                alt: true,
                ..Modifiers::default()
            },
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(25.0, 15.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        let first = editor.document.get(id).unwrap().path().unwrap().contours[0].anchors[0];
        assert_eq!(first.out_handle, Some(Vec2::new(25.0, 15.0)));
        assert_eq!(first.in_handle, Some(Vec2::new(-10.0, 0.0)));
        assert_eq!(first.mode, HandleMode::Independent);

        editor.handle_pointer_down_with_click_count(
            Vec2::new(30.0, 11.0),
            MouseButton::Left,
            Modifiers::default(),
            2,
        );
        assert_eq!(
            editor.document.get(id).unwrap().path().unwrap().contours[0]
                .anchors
                .len(),
            3
        );
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.history.undo_description(), Some("Edit Vector"));
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(
            editor.document.get(id).unwrap().path().unwrap().contours[0]
                .anchors
                .len(),
            2
        );
    }

    #[test]
    fn vector_edit_joins_endpoints_across_transformed_nodes_in_one_history_item() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let line = || PathData {
            contours: vec![PathContour::open([
                PathAnchor::corner(Vec2::ZERO),
                PathAnchor::corner(Vec2::new(10.0, 0.0)),
            ])],
        };
        let first = editor
            .document
            .add_child(root, Node::shape("First", line()))
            .unwrap();
        let second = editor
            .document
            .add_child(
                root,
                Node::shape("Second", line())
                    .with_transform(Affine2::from_translation(Vec2::new(20.0, 0.0))),
            )
            .unwrap();
        editor.selection.select(first);
        assert!(editor.execute_action(EditorAction::EnterVectorEdit));
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 0.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(10.0, 0.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(20.0, 0.0),
            button: MouseButton::Left,
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(20.0, 0.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert!(editor.execute_action(EditorAction::JoinPaths));
        assert!(editor.finish_editing());

        let joined = editor.document.get(first).unwrap().path().unwrap();
        assert_eq!(joined.contours[0].anchors.len(), 4);
        assert_eq!(joined.contours[0].anchors[2].position, Vec2::new(20.0, 0.0));
        assert!(editor.document.get(second).unwrap().deleted);
        assert_eq!(editor.history.undo_description(), Some("Edit Vector"));

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(
            editor.document.get(first).unwrap().path().unwrap().contours[0]
                .anchors
                .len(),
            2
        );
        assert!(!editor.document.get(second).unwrap().deleted);
        assert_eq!(
            editor
                .document
                .get(second)
                .unwrap()
                .path()
                .unwrap()
                .contours[0]
                .anchors
                .len(),
            2
        );
    }

    #[test]
    fn vector_split_preserves_the_anchor_and_has_local_undo_redo() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let anchors = [
            PathAnchor::corner(Vec2::new(0.0, 0.0)),
            PathAnchor::corner(Vec2::new(10.0, 0.0)),
            PathAnchor::corner(Vec2::new(20.0, 0.0)),
        ];
        let id = editor
            .document
            .add_child(
                root,
                Node::shape(
                    "Line",
                    PathData {
                        contours: vec![PathContour::open(anchors)],
                    },
                ),
            )
            .unwrap();
        editor.selection.select(id);
        assert!(editor.execute_action(EditorAction::EnterVectorEdit));
        let DragState::VectorEditing(session) = &mut editor.drag else {
            panic!("expected vector editing");
        };
        session.selected.insert(AnchorRef {
            node: id,
            contour: 0,
            anchor: 1,
        });

        assert!(editor.execute_action(EditorAction::SplitPath));
        let split = editor.document.get(id).unwrap().path().unwrap();
        assert_eq!(split.contours.len(), 2);
        assert_eq!(split.contours[0].anchors.len(), 2);
        assert_eq!(split.contours[1].anchors.len(), 2);
        assert_eq!(split.contours[0].anchors[1].position, Vec2::new(10.0, 0.0));
        assert_eq!(split.contours[1].anchors[0].position, Vec2::new(10.0, 0.0));

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.interaction_kind(), InteractionKind::VectorEditing);
        assert_eq!(
            editor
                .document
                .get(id)
                .unwrap()
                .path()
                .unwrap()
                .contours
                .len(),
            1
        );
        assert!(editor.execute_action(EditorAction::Redo));
        assert_eq!(editor.interaction_kind(), InteractionKind::VectorEditing);
        assert_eq!(
            editor
                .document
                .get(id)
                .unwrap()
                .path()
                .unwrap()
                .contours
                .len(),
            2
        );
    }

    #[test]
    fn splitting_a_closed_contour_preserves_the_closing_segment_endpoints() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let id = editor
            .document
            .add_child(
                root,
                Node::shape(
                    "Triangle",
                    PathData {
                        contours: vec![PathContour::closed([
                            PathAnchor::corner(Vec2::new(0.0, 0.0)),
                            PathAnchor::corner(Vec2::new(10.0, 0.0)),
                            PathAnchor::corner(Vec2::new(10.0, 10.0)),
                        ])],
                    },
                ),
            )
            .unwrap();
        editor.selection.select(id);
        assert!(editor.execute_action(EditorAction::EnterVectorEdit));
        let DragState::VectorEditing(session) = &mut editor.drag else {
            panic!("expected vector editing");
        };
        session.selected.insert(AnchorRef {
            node: id,
            contour: 0,
            anchor: 1,
        });

        assert!(editor.execute_action(EditorAction::SplitPath));

        let contour = &editor.document.get(id).unwrap().path().unwrap().contours[0];
        assert!(!contour.closed);
        assert_eq!(contour.anchors.len(), 4);
        assert_eq!(
            contour.anchors.first().unwrap().position,
            Vec2::new(10.0, 0.0)
        );
        assert_eq!(
            contour.anchors.last().unwrap().position,
            Vec2::new(10.0, 0.0)
        );
    }

    #[test]
    fn deleting_the_last_vector_contour_clears_selection_and_is_undoable() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let id = editor
            .document
            .add_child(
                root,
                Node::shape(
                    "Line",
                    PathData {
                        contours: vec![PathContour::open([
                            PathAnchor::corner(Vec2::ZERO),
                            PathAnchor::corner(Vec2::new(10.0, 0.0)),
                        ])],
                    },
                ),
            )
            .unwrap();
        editor.selection.select(id);
        assert!(editor.execute_action(EditorAction::EnterVectorEdit));
        let DragState::VectorEditing(session) = &mut editor.drag else {
            panic!("expected vector editing");
        };
        session.selected.extend([
            AnchorRef {
                node: id,
                contour: 0,
                anchor: 0,
            },
            AnchorRef {
                node: id,
                contour: 0,
                anchor: 1,
            },
        ]);
        assert!(editor.delete_selected_vector_anchors());
        assert!(editor.finish_editing());

        assert!(editor.document.get(id).unwrap().deleted);
        assert!(editor.selection.is_empty());
        assert!(editor.execute_action(EditorAction::Undo));
        assert!(!editor.document.get(id).unwrap().deleted);
        assert_eq!(
            editor.document.get(id).unwrap().path().unwrap().contours[0]
                .anchors
                .len(),
            2
        );
    }

    #[test]
    fn pen_undo_while_pointer_is_down_resets_the_active_anchor() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolPen);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Z,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(20.0, 20.0),
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(20.0, 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let DragState::Pen(session) = &editor.drag else {
            panic!("expected pen session");
        };
        assert!(session.anchors.is_empty());
        assert!(!session.pointer_down);
        assert_eq!(session.active_anchor, None);
    }

    #[test]
    fn undo_during_shape_preview_only_cancels_the_preview() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolRectangle);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(30.0, 30.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        let committed = editor.selection.primary().unwrap();

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(40.0, 40.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(60.0, 60.0),
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::CreatingShape);

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert!(!editor.document.get(committed).unwrap().deleted);
        assert!(editor.can_execute(EditorAction::Undo));

        let start_pan = editor.view.pan;
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(5.0, 5.0),
            button: MouseButton::Middle,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(25.0, 15.0),
            modifiers: Modifiers::default(),
        });
        assert_ne!(editor.view.pan, start_pan);
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.view.pan, start_pan);
        assert!(!editor.document.get(committed).unwrap().deleted);

        assert!(editor.execute_action(EditorAction::Undo));
        assert!(editor.document.get(committed).unwrap().deleted);
    }

    #[test]
    fn resizing_a_frame_does_not_scale_its_children() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let frame = editor
            .document
            .add_child(root, Node::frame("Frame", 100.0, 100.0))
            .unwrap();
        let child = editor
            .document
            .add_child(
                frame,
                Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::splat(20.0))),
            )
            .unwrap();
        let child_world = editor.document.world_transform(child);
        editor.selection.select(frame);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(100.0, 50.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(150.0, 50.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let NodeKind::Frame(data) = &editor.document.get(frame).unwrap().kind else {
            panic!("expected a frame");
        };
        assert_eq!(data.width, 150.0);
        assert_affine_approx_eq(editor.document.world_transform(child), child_world);
        assert!(editor.execute_action(EditorAction::Undo));
        let NodeKind::Frame(data) = &editor.document.get(frame).unwrap().kind else {
            panic!("expected a frame");
        };
        assert_eq!(data.width, 100.0);
        assert_affine_approx_eq(editor.document.world_transform(child), child_world);
    }

    #[test]
    fn frame_resize_handle_stops_at_the_opposite_edge() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let frame = editor
            .document
            .add_child(root, Node::frame("Frame", 100.0, 100.0))
            .unwrap();
        editor.selection.select(frame);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(100.0, 50.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(-50.0, 50.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let NodeKind::Frame(data) = &editor.document.get(frame).unwrap().kind else {
            panic!("expected frame");
        };
        assert_eq!(data.width, 1.0);
        assert_eq!(editor.document.world_transform(frame).translation.x, 0.0);
    }

    #[test]
    fn text_side_resize_changes_box_width_while_corner_resize_changes_font_size() {
        let mut side_editor = Editor::new();
        let side_root = side_editor.document.root;
        let side_text = side_editor
            .document
            .add_child(side_root, Node::text("Label", "Hello"))
            .unwrap();
        side_editor.selection.select(side_text);
        side_editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(48.0, 9.6),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        side_editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(96.0, 9.6),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        let NodeKind::Text(side_data) = &side_editor.document.get(side_text).unwrap().kind else {
            panic!("expected text");
        };
        assert_eq!(side_data.font_size, 16.0);
        assert_eq!(side_data.sizing, TextSizing::FixedWidth { width: 96.0 });

        let mut corner_editor = Editor::new();
        let corner_root = corner_editor.document.root;
        let corner_text = corner_editor
            .document
            .add_child(corner_root, Node::text("Label", "Hello"))
            .unwrap();
        corner_editor.selection.select(corner_text);
        corner_editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(48.0, 19.2),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        corner_editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(96.0, 38.4),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        let NodeKind::Text(corner_data) = &corner_editor.document.get(corner_text).unwrap().kind
        else {
            panic!("expected text");
        };
        assert!((corner_data.font_size - 32.0).abs() < 0.001);
        assert_eq!(corner_data.sizing, TextSizing::AutoWidth);
    }

    #[test]
    fn rotation_is_applied_around_selection_center_and_is_undoable() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let id = editor
            .document
            .add_child(
                root,
                Node::shape("Square", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        editor.selection.select(id);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(5.0, -24.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(34.0, 5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let rotated_origin = editor
            .document
            .world_transform(id)
            .transform_point2(Vec2::ZERO);
        assert!((rotated_origin - Vec2::new(10.0, 0.0)).length() < 0.001);
        assert_eq!(editor.history.undo_description(), Some("Rotate"));
        assert!(editor.execute_action(EditorAction::Undo));
        assert_affine_approx_eq(editor.document.world_transform(id), Affine2::IDENTITY);
    }

    #[test]
    fn rectangle_drag_creates_selected_undoable_shape() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolRectangle);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(100.0, 80.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(40.0, 30.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let id = editor.selection.primary().unwrap();
        let node = editor.document.get(id).unwrap();
        assert_eq!(node.name, "Rectangle");
        assert_eq!(node.transform.translation, Vec2::new(40.0, 30.0));
        let NodeKind::Shape(path) = &node.kind else {
            panic!("rectangle tool should create a shape");
        };
        assert_eq!(path.bounds().unwrap().size(), Vec2::new(60.0, 50.0));
        assert_eq!(editor.history.undo_description(), Some("Create Rectangle"));
        assert_eq!(editor.tool, Tool::Rectangle);

        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.document.descendants(editor.document.root).count(), 0);
        assert!(editor.selection.is_empty());

        assert!(editor.execute_action(EditorAction::Redo));
        assert_eq!(editor.document.descendants(editor.document.root).count(), 1);
        assert_eq!(editor.tool, Tool::Rectangle);
    }

    #[test]
    fn creation_style_defaults_are_non_historical_and_tool_scoped() {
        let mut editor = Editor::new();
        let revision = editor.current_revision();
        let initial = editor.creation_style_defaults().clone();

        assert!(editor.execute_action(EditorAction::ToolRectangle));
        assert!(editor.set_creation_fill(Some(Paint::rgb(0.9, 0.2, 0.1))));
        assert!(editor.toggle_creation_stroke());
        assert!(editor.set_creation_stroke_paint(Paint::rgb(0.1, 0.2, 0.9)));
        assert!(editor.adjust_creation_stroke_width(0.5));
        assert!(!editor.adjust_creation_stroke_width(f32::NAN));
        let shape_style = editor.active_creation_style().cloned().unwrap();
        assert_eq!(shape_style.fill, Some(Paint::rgb(0.9, 0.2, 0.1)));
        assert_eq!(
            shape_style.stroke,
            Some(Stroke::new(2.0, Paint::rgb(0.1, 0.2, 0.9)))
        );

        assert!(editor.execute_action(EditorAction::ToolLine));
        assert_eq!(editor.active_creation_style(), Some(&initial.paths));
        assert!(editor.toggle_creation_fill());
        assert!(editor.adjust_creation_stroke_width(1.0));
        let path_style = editor.active_creation_style().cloned().unwrap();
        assert_eq!(path_style.fill, Some(Paint::black()));
        assert_eq!(path_style.stroke.as_ref().unwrap().width, 2.5);

        assert!(editor.execute_action(EditorAction::ToolEllipse));
        assert_eq!(editor.active_creation_style(), Some(&shape_style));
        assert!(editor.execute_action(EditorAction::ToolText));
        assert_eq!(editor.active_creation_style(), None);
        assert!(!editor.toggle_creation_fill());
        assert!(!editor.toggle_creation_stroke());

        assert_eq!(editor.current_revision(), revision);
        assert!(!editor.is_dirty());
        assert!(!editor.history.can_undo());
    }

    #[test]
    fn created_shapes_capture_defaults_across_later_changes_and_history() {
        let mut editor = Editor::new();
        assert!(editor.execute_action(EditorAction::ToolRectangle));
        assert!(editor.set_creation_fill(Some(Paint::rgb(0.8, 0.1, 0.2))));
        assert!(editor.toggle_creation_stroke());
        let expected = editor.active_creation_style().cloned().unwrap();

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(50.0, 60.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        let id = editor.selection.primary().unwrap();
        assert_eq!(editor.document.get(id).unwrap().style, expected);
        assert_eq!(editor.history.undo_description(), Some("Create Rectangle"));

        let creation_revision = editor.current_revision();
        assert!(editor.set_creation_fill(Some(Paint::black())));
        assert_eq!(editor.current_revision(), creation_revision);
        assert_eq!(editor.document.get(id).unwrap().style, expected);

        assert!(editor.execute_action(EditorAction::Undo));
        assert!(editor.execute_action(EditorAction::Redo));
        assert_eq!(editor.document.get(id).unwrap().style, expected);
    }

    #[test]
    fn line_and_pen_share_path_defaults_while_text_keeps_its_own_fill() {
        let mut editor = Editor::new();
        assert!(editor.execute_action(EditorAction::ToolLine));
        assert!(editor.set_creation_fill(Some(Paint::rgb(0.2, 0.7, 0.3))));
        assert!(editor.set_creation_stroke_paint(Paint::rgb(0.1, 0.4, 0.9)));
        assert!(editor.adjust_creation_stroke_width(1.5));
        let expected_path = editor.active_creation_style().cloned().unwrap();

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(40.0, 30.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        let line = editor.selection.primary().unwrap();
        assert_eq!(editor.document.get(line).unwrap().style, expected_path);

        assert!(editor.execute_action(EditorAction::ToolPen));
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(60.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(60.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert!(editor.set_creation_stroke_paint(Paint::rgb(0.9, 0.4, 0.1)));
        let expected_pen = editor.active_creation_style().cloned().unwrap();
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(90.0, 30.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(90.0, 30.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert!(editor.finish_editing());
        let path = editor.selection.primary().unwrap();
        assert_eq!(editor.document.get(path).unwrap().style, expected_pen);
        assert_eq!(editor.document.get(line).unwrap().style, expected_path);

        assert!(editor.execute_action(EditorAction::ToolText));
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(120.0, 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(120.0, 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        assert!(editor.replace_text(None, "Text"));
        assert!(editor.finish_editing());
        let text = editor.selection.primary().unwrap();
        assert_eq!(
            editor.document.get(text).unwrap().style,
            Style::fill(Paint::rgb(0.12, 0.13, 0.15))
        );
    }

    #[test]
    fn line_drag_preserves_direction_and_creates_a_stroked_open_path() {
        let mut editor = Editor::new();
        assert!(editor.execute_action(EditorAction::ToolLine));

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(30.0, 10.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(10.0, 30.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let id = editor.selection.primary().unwrap();
        let node = editor.document.get(id).unwrap();
        let NodeKind::Shape(path) = &node.kind else {
            panic!("line tool should create a shape");
        };
        assert_eq!(node.name, "Line");
        assert_eq!(node.transform.translation, Vec2::splat(10.0));
        assert!(node.style.fill.is_none());
        assert_eq!(node.style.stroke.as_ref().unwrap().width, 1.5);
        assert!(!path.contours[0].closed);
        assert_eq!(path.contours[0].anchors[0].position, Vec2::new(20.0, 0.0));
        assert_eq!(path.contours[0].anchors[1].position, Vec2::new(0.0, 20.0));
        assert_eq!(editor.history.undo_description(), Some("Create Line"));
        assert_eq!(editor.tool, Tool::Line);
    }

    #[test]
    fn line_modifiers_snap_to_45_degrees_and_draw_from_center() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolLine);
        let modifiers = Modifiers {
            shift: true,
            alt: true,
            ..Default::default()
        };

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(50.0),
            button: MouseButton::Left,
            modifiers,
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(80.0, 60.0),
            button: MouseButton::Left,
            modifiers,
        });

        let id = editor.selection.primary().unwrap();
        let node = editor.document.get(id).unwrap();
        let NodeKind::Shape(path) = &node.kind else {
            panic!("line tool should create a shape");
        };
        let world = node.transform;
        let start = world.transform_point2(path.contours[0].anchors[0].position);
        let end = world.transform_point2(path.contours[0].anchors[1].position);
        assert!((start.y - end.y).abs() < 0.001);
        assert!(((start + end) * 0.5 - Vec2::splat(50.0)).length() < 0.001);
    }

    #[test]
    fn ellipse_preview_and_commit_use_ellipse_geometry() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolEllipse);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(10.0, 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(50.0, 50.0),
            modifiers: Modifiers::default(),
        });

        let display_list = editor.build_display_list();
        let preview_path = display_list
            .items
            .iter()
            .find_map(|item| match item {
                editor_render::DisplayItem::ToolPreview { path, .. } => Some(path),
                _ => None,
            })
            .expect("ellipse drag should emit a preview");
        assert_eq!(preview_path.commands.len(), 6);
        assert!(matches!(
            preview_path.commands[1],
            editor_render::PathCmd::CubicTo { .. }
        ));

        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(50.0, 50.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        let node = editor
            .document
            .get(editor.selection.primary().unwrap())
            .unwrap();
        let NodeKind::Shape(path) = &node.kind else {
            panic!("ellipse tool should create a shape");
        };
        assert_eq!(path.bounds().unwrap().size(), Vec2::new(40.0, 30.0));
        assert!(matches!(
            path.to_commands()[1],
            crate::PathCmd::CubicTo { .. }
        ));
    }

    #[test]
    fn shape_modifiers_constrain_and_draw_from_center() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolEllipse);
        let modifiers = Modifiers {
            shift: true,
            alt: true,
            ..Modifiers::default()
        };

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(50.0, 50.0),
            button: MouseButton::Left,
            modifiers,
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(70.0, 60.0),
            button: MouseButton::Left,
            modifiers,
        });

        let bounds = editor
            .document
            .world_bounds(editor.selection.primary().unwrap())
            .unwrap();
        assert_eq!(bounds.min, Vec2::new(30.0, 30.0));
        assert_eq!(bounds.max, Vec2::new(70.0, 70.0));
    }

    #[test]
    fn modifier_changes_update_shape_preview_without_pointer_motion() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolRectangle);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::ZERO,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(40.0, 20.0),
            modifiers: Modifiers::default(),
        });

        editor.handle_event(InputEvent::ModifiersChanged {
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        });

        let display_list = editor.build_display_list();
        let preview_path = display_list
            .items
            .iter()
            .find_map(|item| match item {
                editor_render::DisplayItem::ToolPreview { path, .. } => Some(path),
                _ => None,
            })
            .expect("rectangle drag should emit a preview");
        assert_eq!(
            preview_path.commands[2],
            editor_render::PathCmd::LineTo(Vec2::splat(40.0))
        );
    }

    #[test]
    fn shape_endpoint_snaps_and_emits_guides() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        editor
            .document
            .add_child(
                root,
                Node::shape("Target", PathData::rect(0.0, 0.0, 20.0, 20.0))
                    .with_transform(Affine2::from_translation(Vec2::new(100.0, 100.0))),
            )
            .unwrap();
        editor.execute_action(EditorAction::ToolRectangle);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::ZERO,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(95.0, 95.0),
            modifiers: Modifiers::default(),
        });

        let display_list = editor.build_display_list();
        assert!(display_list
            .items
            .iter()
            .any(|item| matches!(item, editor_render::DisplayItem::ToolPreview { .. })));
        assert_eq!(
            display_list
                .items
                .iter()
                .filter(|item| matches!(item, editor_render::DisplayItem::SnapGuide { .. }))
                .count(),
            2
        );

        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::new(95.0, 95.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        let bounds = editor
            .document
            .world_bounds(editor.selection.primary().unwrap())
            .unwrap();
        assert_eq!(bounds.max, Vec2::splat(100.0));
    }

    #[test]
    fn constrained_line_hides_snap_guides_its_endpoint_does_not_reach() {
        let mut editor = Editor::new();
        editor
            .document
            .add_child(
                editor.document.root,
                Node::shape("Target", PathData::rect(0.0, 0.0, 20.0, 20.0))
                    .with_transform(Affine2::from_translation(Vec2::new(100.0, 200.0))),
            )
            .unwrap();
        editor.execute_action(EditorAction::ToolLine);
        let modifiers = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::ZERO,
            button: MouseButton::Left,
            modifiers,
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(95.0, 50.0),
            modifiers,
        });

        let display_list = editor.build_display_list();

        assert_eq!(
            display_list
                .items
                .iter()
                .filter(|item| matches!(item, editor_render::DisplayItem::SnapGuide { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn click_without_drag_does_not_create_shape_or_history() {
        let mut editor = Editor::new();
        editor.execute_action(EditorAction::ToolRectangle);

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(25.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerUp {
            position: Vec2::splat(25.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });

        assert_eq!(editor.document.descendants(editor.document.root).count(), 0);
        assert!(!editor.history.can_undo());
    }

    #[test]
    fn escape_cancels_then_selects_then_deselects() {
        let (mut editor, ids) = editor_with_named_shapes(&["Selected"]);
        editor.selection.select(ids[0]);
        editor.execute_action(EditorAction::ToolRectangle);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::new(20.0, 20.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(40.0, 40.0),
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::CreatingShape);

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.tool, Tool::Rectangle);
        assert_eq!(editor.selection.primary(), Some(ids[0]));

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.tool, Tool::Select);
        assert_eq!(editor.selection.primary(), Some(ids[0]));

        editor.handle_event(InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn escape_cancels_shape_preview_and_move_transaction() {
        let (mut editor, ids) = editor_with_named_shapes(&["A"]);
        let original = editor.document.get(ids[0]).unwrap().transform;

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(25.0, 15.0),
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });
        assert_eq!(editor.document.get(ids[0]).unwrap().transform, original);
        assert!(!editor.history.can_undo());

        editor.execute_action(EditorAction::ToolRectangle);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::ZERO,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::splat(30.0),
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        });

        assert!(!editor
            .build_display_list()
            .items
            .iter()
            .any(|item| matches!(item, editor_render::DisplayItem::ToolPreview { .. })));
        assert_eq!(editor.document.descendants(editor.document.root).count(), 1);
        assert_eq!(editor.cursor(), Cursor::Crosshair);
    }

    #[test]
    fn changing_tools_cancels_an_in_progress_move() {
        let (mut editor, ids) = editor_with_named_shapes(&["A"]);
        let original = editor.document.get(ids[0]).unwrap().transform;

        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(35.0, 15.0),
            modifiers: Modifiers::default(),
        });
        assert_ne!(editor.document.get(ids[0]).unwrap().transform, original);

        assert!(editor.execute_action(EditorAction::ToolEllipse));

        assert_eq!(editor.document.get(ids[0]).unwrap().transform, original);
        assert_eq!(editor.tool, Tool::Ellipse);
        assert!(!editor.history.can_undo());
    }

    #[test]
    fn numeric_world_scrub_is_absolute_nested_and_one_history_entry() {
        let mut editor = Editor::new();
        let group = editor
            .document
            .add_child(
                editor.document.root,
                Node::group("Parent").with_transform(Affine2::from_scale_angle_translation(
                    Vec2::new(1.5, 0.75),
                    0.35,
                    Vec2::new(80.0, 40.0),
                )),
            )
            .unwrap();
        let child = editor
            .document
            .add_child(
                group,
                Node::shape("Child", PathData::rect(0.0, 0.0, 20.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::new(12.0, 8.0))),
            )
            .unwrap();
        editor.selection.select(child);
        let original = editor.document.world_transform(child);

        let x_scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::WorldX)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&x_scrub, 10.0));
        let first = editor.document.world_transform(child);
        assert!((first.translation.x - original.translation.x - 10.0).abs() < 0.001);
        assert!((first.translation.y - original.translation.y).abs() < 0.001);
        assert!(editor.preview_numeric_property_scrub(&x_scrub, 25.0));
        let second = editor.document.world_transform(child);
        assert!((second.translation.x - original.translation.x - 25.0).abs() < 0.001);
        assert_eq!(editor.history.undo_count(), 0);
        assert_eq!(editor.current_revision(), 0);
        assert!(!editor.is_dirty());

        assert!(editor.commit_numeric_property_scrub(x_scrub));
        assert_eq!(editor.history.undo_count(), 1);
        assert_eq!(editor.history.undo_description(), Some("Move X"));
        assert_eq!(editor.current_revision(), 1);
        assert!(editor.is_dirty());
        assert!(editor.execute_action(EditorAction::Undo));
        assert_affine_approx_eq(editor.document.world_transform(child), original);
        assert!(!editor.is_dirty());

        let y_scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::WorldY)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&y_scrub, -12.0));
        let moved = editor.document.world_transform(child);
        assert!((moved.translation.x - original.translation.x).abs() < 0.001);
        assert!((moved.translation.y - original.translation.y + 12.0).abs() < 0.001);
        assert!(editor.commit_numeric_property_scrub(y_scrub));
        assert_eq!(editor.history.undo_count(), 1);
        assert_eq!(editor.history.undo_description(), Some("Move Y"));
        assert!(editor.execute_action(EditorAction::Undo));
        assert_affine_approx_eq(editor.document.world_transform(child), original);
    }

    #[test]
    fn numeric_style_scrubs_clamp_without_preview_drift() {
        let mut editor = Editor::new();
        let shape = editor
            .document
            .add_child(
                editor.document.root,
                Node::shape("Shape", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_style(Style::fill_and_stroke(Paint::white(), Stroke::black(2.0))),
            )
            .unwrap();
        editor.selection.select(shape);

        let opacity = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&opacity, -0.25));
        assert_eq!(editor.document.get(shape).unwrap().style.opacity, 0.75);
        assert!(editor.preview_numeric_property_scrub(&opacity, -2.0));
        assert_eq!(editor.document.get(shape).unwrap().style.opacity, 0.0);
        assert!(!editor.preview_numeric_property_scrub(&opacity, -2.0));
        assert_eq!(editor.history.undo_count(), 0);
        assert!(editor.commit_numeric_property_scrub(opacity));
        assert_eq!(editor.history.undo_count(), 1);
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.document.get(shape).unwrap().style.opacity, 1.0);

        let stroke = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::StrokeWidth)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&stroke, -20.0));
        assert_eq!(
            editor.document.get(shape).unwrap().style.stroke,
            Some(Stroke::black(0.1))
        );
        assert!(editor.preview_numeric_property_scrub(&stroke, 3.0));
        assert_eq!(
            editor.document.get(shape).unwrap().style.stroke,
            Some(Stroke::black(5.0))
        );
        assert!(editor.commit_numeric_property_scrub(stroke));
        assert_eq!(editor.history.undo_count(), 1);
        assert_eq!(
            editor.history.undo_description(),
            Some("Change Stroke Width")
        );
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(
            editor.document.get(shape).unwrap().style.stroke,
            Some(Stroke::black(2.0))
        );
    }

    #[test]
    fn numeric_text_and_layout_scrubs_commit_or_cancel_cleanly() {
        let mut editor = Editor::new();
        let text = editor
            .document
            .add_child(editor.document.root, Node::text("Text", "Hello"))
            .unwrap();
        editor.selection.select(text);

        let text_scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::TextSize)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&text_scrub, 800.0));
        assert_eq!(editor.selected_text_data().unwrap().font_size, 512.0);
        assert!(editor.preview_numeric_property_scrub(&text_scrub, -20.0));
        assert_eq!(editor.selected_text_data().unwrap().font_size, 1.0);
        assert!(editor.cancel_numeric_property_scrub(text_scrub));
        assert_eq!(editor.selected_text_data().unwrap().font_size, 16.0);
        assert_eq!(editor.history.undo_count(), 0);
        assert!(!editor.is_dirty());

        let text_scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::TextSize)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&text_scrub, 4.0));
        assert!(editor.commit_numeric_property_scrub(text_scrub));
        assert_eq!(editor.selected_text_data().unwrap().font_size, 20.0);
        assert_eq!(editor.history.undo_count(), 1);
        assert!(editor.execute_action(EditorAction::Undo));
        assert_eq!(editor.selected_text_data().unwrap().font_size, 16.0);

        let layout = AutoLayout::horizontal()
            .with_spacing(10.0)
            .with_uniform_padding(5.0);
        let group = editor
            .document
            .add_child(
                editor.document.root,
                Node::group("Stack").with_layout(Layout::Auto(layout)),
            )
            .unwrap();
        editor.selection.select(group);

        let spacing = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::AutoLayoutSpacing)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&spacing, -100.0));
        let Layout::Auto(preview) = editor.selected_group_layout().unwrap() else {
            panic!("expected auto layout");
        };
        assert_eq!(preview.spacing, 0.0);
        assert!(editor.preview_numeric_property_scrub(&spacing, 3.0));
        let Layout::Auto(preview) = editor.selected_group_layout().unwrap() else {
            panic!("expected auto layout");
        };
        assert_eq!(preview.spacing, 13.0);
        assert!(editor.commit_numeric_property_scrub(spacing));
        assert_eq!(editor.history.undo_count(), 1);
        assert!(editor.execute_action(EditorAction::Undo));
        let Layout::Auto(restored) = editor.selected_group_layout().unwrap() else {
            panic!("expected auto layout");
        };
        assert_eq!(restored.spacing, 10.0);

        let padding = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::AutoLayoutPadding)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&padding, -100.0));
        let Layout::Auto(preview) = editor.selected_group_layout().unwrap() else {
            panic!("expected auto layout");
        };
        assert_eq!(preview.padding, crate::Edges::uniform(0.0));
        assert!(editor.preview_numeric_property_scrub(&padding, 3.0));
        let Layout::Auto(preview) = editor.selected_group_layout().unwrap() else {
            panic!("expected auto layout");
        };
        assert_eq!(preview.padding, crate::Edges::uniform(8.0));
        assert!(editor.cancel_numeric_property_scrub(padding));
        let Layout::Auto(restored) = editor.selected_group_layout().unwrap() else {
            panic!("expected auto layout");
        };
        assert_eq!(restored.padding, crate::Edges::uniform(5.0));
        assert_eq!(editor.history.undo_count(), 0);
        assert!(!editor.is_dirty());
    }

    #[test]
    fn numeric_scrubs_reject_noops_and_invalidate_safely() {
        let (mut editor, ids) = editor_with_named_shapes(&["A", "B"]);
        editor.selection.select(ids[0]);
        assert!(editor
            .begin_numeric_property_scrub(NumericPropertyTarget::StrokeWidth)
            .is_none());

        let no_op = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();
        assert!(!editor.preview_numeric_property_scrub(&no_op, 0.0));
        assert!(!editor.preview_numeric_property_scrub(&no_op, f32::NAN));
        assert!(!editor.preview_numeric_property_scrub(&no_op, f32::INFINITY));
        assert!(!editor.commit_numeric_property_scrub(no_op));
        assert_eq!(editor.history.undo_count(), 0);
        assert_eq!(editor.current_revision(), 0);
        assert!(!editor.is_dirty());

        let changed_selection = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&changed_selection, -0.4));
        editor.selection.select(ids[1]);
        assert!(!editor.preview_numeric_property_scrub(&changed_selection, -0.6));
        assert_eq!(editor.document.get(ids[0]).unwrap().style.opacity, 1.0);
        assert!(!editor.commit_numeric_property_scrub(changed_selection));
        assert_eq!(editor.history.undo_count(), 0);

        editor.selection.select(ids[0]);
        let deleted = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&deleted, -0.3));
        assert!(editor.document.remove(ids[0]));
        assert!(!editor.commit_numeric_property_scrub(deleted));
        assert!(editor.document.get(ids[0]).unwrap().deleted);
        assert_eq!(editor.history.undo_count(), 0);
    }

    #[test]
    fn beginning_numeric_scrub_settles_incompatible_move_preview() {
        let (mut editor, ids) = editor_with_named_shapes(&["A"]);
        let original = editor.document.get(ids[0]).unwrap().transform;
        editor.selection.select(ids[0]);
        editor.handle_event(InputEvent::PointerDown {
            position: Vec2::splat(5.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        });
        editor.handle_event(InputEvent::PointerMove {
            position: Vec2::new(25.0, 15.0),
            modifiers: Modifiers::default(),
        });
        assert_ne!(editor.document.get(ids[0]).unwrap().transform, original);

        let scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();

        assert_eq!(editor.document.get(ids[0]).unwrap().transform, original);
        assert_eq!(editor.interaction_kind(), InteractionKind::Idle);
        assert_eq!(editor.history.undo_count(), 0);
        assert!(editor.cancel_numeric_property_scrub(scrub));
    }

    #[test]
    fn numeric_scrub_consumes_undo_and_redo_before_document_history() {
        let (mut editor, ids) = editor_with_named_shapes(&["Shape"]);
        editor.selection.select(ids[0]);
        assert!(editor.set_selected_opacity(0.8));
        assert_eq!(editor.history.undo_count(), 1);

        let scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&scrub, -0.3));
        assert_eq!(editor.document.get(ids[0]).unwrap().style.opacity, 0.5);
        assert!(editor.can_undo_in_context());
        assert!(editor.undo_in_context());
        assert_eq!(editor.document.get(ids[0]).unwrap().style.opacity, 0.8);
        assert_eq!(editor.history.undo_count(), 1);

        assert!(editor.undo_in_context());
        assert_eq!(editor.document.get(ids[0]).unwrap().style.opacity, 1.0);
        assert_eq!(editor.history.redo_count(), 1);

        let scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&scrub, -0.4));
        assert!(editor.can_redo_in_context());
        assert!(editor.redo_in_context());
        assert_eq!(editor.document.get(ids[0]).unwrap().style.opacity, 1.0);
        assert_eq!(editor.history.redo_count(), 1);

        assert!(editor.redo_in_context());
        assert_eq!(editor.document.get(ids[0]).unwrap().style.opacity, 0.8);
        assert_eq!(editor.history.redo_count(), 0);
    }

    #[test]
    fn clipboard_and_delete_never_capture_a_numeric_preview() {
        let (mut editor, ids) = editor_with_named_shapes(&["Shape"]);
        editor.selection.select(ids[0]);

        let copy_scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&copy_scrub, -0.4));
        let clipboard = editor.copy_selection().unwrap();
        assert_eq!(clipboard.roots[0].node.node.style.opacity, 1.0);
        assert!(!editor.commit_numeric_property_scrub(copy_scrub));

        let delete_scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::Opacity)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&delete_scrub, -0.4));
        editor.delete_selection();
        assert!(editor.document.get(ids[0]).unwrap().deleted);
        assert!(!editor.commit_numeric_property_scrub(delete_scrub));

        assert!(editor.undo_in_context());
        let restored = editor.document.get(ids[0]).unwrap();
        assert!(!restored.deleted);
        assert_eq!(restored.style.opacity, 1.0);
    }

    #[test]
    fn direct_text_property_edit_restores_numeric_preview_before_history() {
        let mut editor = Editor::new();
        let text = editor
            .document
            .add_child(editor.document.root, Node::text("Text", "Hello"))
            .unwrap();
        editor.selection.select(text);

        let scrub = editor
            .begin_numeric_property_scrub(NumericPropertyTarget::TextSize)
            .unwrap();
        assert!(editor.preview_numeric_property_scrub(&scrub, 4.0));
        assert_eq!(editor.selected_text_data().unwrap().font_size, 20.0);
        assert!(editor.set_selected_text_font_family("serif"));
        let current = editor.selected_text_data().unwrap();
        assert_eq!(current.font_size, 16.0);
        assert_eq!(current.font.family, "serif");

        assert!(editor.undo_in_context());
        let restored = editor.selected_text_data().unwrap();
        assert_eq!(restored.font_size, 16.0);
        assert_eq!(restored.font.family, "sans-serif");
    }
}
