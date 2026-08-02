//! Editor Core - Document model for a vector editor
//!
//! This crate provides the core data model and operations for a vector graphics editor.
//! It includes:
//! - Scene graph with generational arena storage
//! - Node types (groups, shapes, text)
//! - Transform hierarchy
//! - Caching for world transforms and bounds
//! - Selection model
//! - Command/patch system for undo/redo
//! - Transaction support for interactive edits
//! - Action system for unified command execution

pub mod action;
pub mod cache;
pub mod color_library;
pub mod command;
pub mod editor;
pub mod history;
pub mod input;
pub mod layers;
pub mod layout;
pub mod node;
pub mod path;
pub mod precision;
mod property_scrub;
pub mod render;
pub mod selection;
pub mod snap;
pub mod style;
pub mod text;
pub mod transaction;
pub mod transform;

pub use action::{
    ActionCategory, ActionMeta, EditorAction, NudgeDirection, NudgeDistance, Shortcut,
};
pub use cache::{Cache, DirtyFlags};
pub use color_library::{
    ColorGroup, ColorGroupId, ColorLibrary, ColorLibraryError, ColorSortMode, RgbaColor,
    SavedColor, SavedColorId, SortDirection, MAX_COLOR_GROUPS, MAX_COLOR_NAME_BYTES,
    MAX_SAVED_COLORS,
};
pub use command::{Command, Patch};
pub use editor::{
    AnchorRef, ArtworkSnapshot, CreationStyleDefaults, Editor, EditorClipboard, GroupLayoutMode,
    InteractionKind, TextInputSnapshot, TextNavigation, Tool, TransformComponents,
};
pub use history::History;
pub use input::{Cursor, Effects, InputEvent, Key, Modifiers, MouseButton};
pub use layers::{LayerEntry, LayerIcon, LayerPanelState};
pub use layout::{AlignCross, AlignMain, AutoLayout, Direction, Edges, LayoutOffset};
pub use node::{
    ClipNode, ClipPath, ClipShape, FontSpec, FrameData, Layout, Node, NodeId, NodeKind, TextAlign,
    TextData, TextSizing,
};
pub use path::{HandleMode, PathAnchor, PathCmd, PathContour, PathData, Rect};
pub use precision::{GridSettings, Guide, GuideAxis, GuideId, PrecisionError, MAX_GUIDES};
pub use property_scrub::{
    NumericAdjustmentDirection, NumericAdjustmentMode, NumericPropertyScrubSession,
    NumericPropertyTarget,
};
pub use selection::{Selection, SelectionAction};
pub use snap::{
    AlignAxis, DistributeAxis, SnapConfig, SnapEngine, SnapKind, SnapPoint, SnapResult,
    SnapSettings, SnapSettingsError, SnapTarget, MAX_SNAP_TOLERANCE, MIN_SNAP_TOLERANCE,
};
pub use style::{
    FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, PaintOrder, RadialGradient,
    SpreadMethod, Stroke, Style,
};
pub use text::{
    estimate_text_metrics, FontId, GlyphRun, PositionedGlyph, SimpleTextEngine, TextEngine,
    TextLayout, TextLayoutLine, TextMetrics,
};
pub use transaction::Transaction;
pub use transform::View;

use glam::{Affine2, Vec2};
use serde::{de::IgnoredAny, Deserialize, Serialize};
use slotmap::SlotMap;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static CLIP_CONTAINMENT_EVALUATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Serializable document format (excludes derived cache data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedDocument {
    /// File format version for future migration
    pub version: u32,

    /// Primary node storage
    pub nodes: SlotMap<NodeId, Node>,

    /// Root node ID
    pub root: NodeId,

    /// Document-owned square grid definition.
    #[serde(default)]
    pub grid: GridSettings,

    /// Persistent ruler guides.
    #[serde(default)]
    pub guides: Vec<Guide>,

    /// Document-local, unlinked saved colors.
    #[serde(default)]
    pub color_library: ColorLibrary,
}

impl SavedDocument {
    /// Current file format version
    pub const CURRENT_VERSION: u32 = 4;
}

/// Error returned while decoding a saved document.
#[derive(Debug)]
pub enum DocumentLoadError {
    InvalidJson(serde_json::Error),
    UnsupportedVersion { version: u32, current: u32 },
    InvalidDocument { reason: DocumentValidationError },
}

/// Structural reason a serialized document cannot be loaded safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentValidationError {
    InvalidPrecisionData {
        reason: PrecisionError,
    },
    InvalidColorLibrary {
        reason: ColorLibraryError,
    },
    TooManyNodes {
        count: usize,
        limit: usize,
    },
    TooManyNodeSlots {
        count: usize,
        limit: usize,
    },
    TooManyChildren {
        node: NodeId,
        count: usize,
        limit: usize,
    },
    TooManyPathAnchors {
        node: NodeId,
        count: usize,
        limit: usize,
    },
    TooManyPathContours {
        node: NodeId,
        count: usize,
        limit: usize,
    },
    TextTooLong {
        node: NodeId,
        bytes: usize,
        limit: usize,
    },
    VisualComplexity {
        node: NodeId,
        resource: &'static str,
        count: usize,
        limit: usize,
    },
    InvalidVisualData {
        node: NodeId,
        reason: &'static str,
    },
    MissingRoot {
        root: NodeId,
    },
    DeletedRoot {
        root: NodeId,
    },
    RootHasParent {
        root: NodeId,
        parent: NodeId,
    },
    RootNotGroup {
        root: NodeId,
    },
    DanglingParent {
        node: NodeId,
        parent: NodeId,
    },
    DanglingChild {
        parent: NodeId,
        child: NodeId,
    },
    DuplicateChild {
        parent: NodeId,
        child: NodeId,
    },
    ParentChildMismatch {
        parent: NodeId,
        child: NodeId,
    },
    MultipleRoots {
        declared_root: NodeId,
        additional_root: NodeId,
    },
    Cycle {
        node: NodeId,
    },
    UnreachableNode {
        node: NodeId,
    },
}

impl fmt::Display for DocumentValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPrecisionData { reason } => {
                write!(formatter, "invalid precision data: {reason}")
            }
            Self::InvalidColorLibrary { reason } => {
                write!(formatter, "invalid color library: {reason}")
            }
            Self::TooManyNodes { count, limit } => {
                write!(formatter, "document contains {count} nodes; limit is {limit}")
            }
            Self::TooManyNodeSlots { count, limit } => write!(
                formatter,
                "document contains {count} serialized node slots; limit is {limit}"
            ),
            Self::TooManyChildren { node, count, limit } => write!(
                formatter,
                "node {node:?} contains {count} children; limit is {limit}"
            ),
            Self::TooManyPathAnchors { node, count, limit } => write!(
                formatter,
                "path {node:?} contains {count} anchors; limit is {limit}"
            ),
            Self::TooManyPathContours { node, count, limit } => write!(
                formatter,
                "path {node:?} contains {count} contours; limit is {limit}"
            ),
            Self::TextTooLong { node, bytes, limit } => write!(
                formatter,
                "text node {node:?} contains {bytes} bytes; limit is {limit}"
            ),
            Self::VisualComplexity {
                node,
                resource,
                count,
                limit,
            } => write!(
                formatter,
                "node {node:?} contains {count} {resource}; limit is {limit}"
            ),
            Self::InvalidVisualData { node, reason } => {
                write!(formatter, "node {node:?} has invalid visual data: {reason}")
            }
            Self::MissingRoot { root } => write!(formatter, "root {root:?} does not exist"),
            Self::DeletedRoot { root } => write!(formatter, "root {root:?} is deleted"),
            Self::RootHasParent { root, parent } => {
                write!(formatter, "root {root:?} has parent {parent:?}")
            }
            Self::RootNotGroup { root } => write!(formatter, "root {root:?} is not a group"),
            Self::DanglingParent { node, parent } => {
                write!(formatter, "node {node:?} references missing parent {parent:?}")
            }
            Self::DanglingChild { parent, child } => {
                write!(formatter, "node {parent:?} references missing child {child:?}")
            }
            Self::DuplicateChild { parent, child } => {
                write!(formatter, "node {parent:?} contains duplicate child {child:?}")
            }
            Self::ParentChildMismatch { parent, child } => write!(
                formatter,
                "parent {parent:?} and child {child:?} do not reference each other"
            ),
            Self::MultipleRoots {
                declared_root,
                additional_root,
            } => write!(
                formatter,
                "document declares root {declared_root:?} but live node {additional_root:?} is also root"
            ),
            Self::Cycle { node } => {
                write!(formatter, "node graph contains a cycle at {node:?}")
            }
            Self::UnreachableNode { node } => {
                write!(formatter, "live node {node:?} is unreachable from the root")
            }
        }
    }
}

impl std::error::Error for DocumentValidationError {}

#[derive(Debug, Clone, Copy)]
struct DocumentComplexityLimits {
    nodes: usize,
    children_per_node: usize,
    path_contours_per_node: usize,
    path_anchors_per_node: usize,
    text_bytes_per_node: usize,
}

impl DocumentComplexityLimits {
    const DEFAULT: Self = Self {
        nodes: 100_000,
        children_per_node: 100_000,
        path_contours_per_node: 10_000,
        path_anchors_per_node: 1_000_000,
        text_bytes_per_node: 4 * 1024 * 1024,
    };
}

const MAX_GRADIENT_STOPS_PER_NODE: usize = 4_096;
const MAX_STROKE_DASH_VALUES_PER_NODE: usize = 4_096;
const MAX_CLIP_NODES_PER_NODE: usize = 100_000;
const MAX_CLIP_CONTOURS_PER_SHAPE: usize = 10_000;
const MAX_CLIP_CONTOURS_PER_NODE: usize = 100_000;
const MAX_CLIP_ANCHORS_PER_NODE: usize = 1_000_000;
const MAX_CLIP_DEPTH: usize = 128;

impl fmt::Display for DocumentLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid document JSON: {error}"),
            Self::UnsupportedVersion { version, current } => write!(
                formatter,
                "document version {version} is newer than supported version {current}"
            ),
            Self::InvalidDocument { reason } => write!(formatter, "invalid document: {reason}"),
        }
    }
}

impl std::error::Error for DocumentLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            Self::InvalidDocument { reason } => Some(reason),
            Self::UnsupportedVersion { .. } => None,
        }
    }
}

impl From<serde_json::Error> for DocumentLoadError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidJson(error)
    }
}

/// The main document structure containing the scene graph.
#[derive(Debug, Clone)]
pub struct Document {
    /// Primary node storage (generational arena)
    pub nodes: SlotMap<NodeId, Node>,

    /// Root node ID (always a Group)
    pub root: NodeId,

    /// Document-owned square grid definition.
    pub grid: GridSettings,

    /// Persistent document-wide ruler guides.
    pub guides: Vec<Guide>,

    /// Document-local, copy-by-value saved colors.
    pub color_library: ColorLibrary,

    /// Cached derived data
    pub cache: Cache,

    /// Dirty flags for incremental updates
    pub dirty: DirtyFlags,

    /// Platform-shaped text layouts keyed by node. This is derived runtime data
    /// and is intentionally excluded from the saved document.
    text_layouts: HashMap<NodeId, text::TextLayout>,

    next_guide_id: u64,
    next_color_group_id: u64,
    next_saved_color_id: u64,
}

impl Document {
    /// Create a new document with an empty root group.
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(Node::group("Root"));

        Self {
            nodes,
            root,
            grid: GridSettings::default(),
            guides: Vec::new(),
            color_library: ColorLibrary::default(),
            cache: Cache::new(),
            dirty: DirtyFlags::all_dirty(),
            text_layouts: HashMap::new(),
            next_guide_id: 1,
            next_color_group_id: 1,
            next_saved_color_id: 1,
        }
    }

    pub(crate) fn allocate_guide_id(&mut self) -> Result<GuideId, PrecisionError> {
        take_next_id(&mut self.next_guide_id)
            .map(GuideId::from_raw)
            .ok_or(PrecisionError::IdSpaceExhausted)
    }

    pub(crate) fn allocate_color_group_id(&mut self) -> Result<ColorGroupId, ColorLibraryError> {
        take_next_id(&mut self.next_color_group_id)
            .map(ColorGroupId::from_raw)
            .ok_or(ColorLibraryError::IdSpaceExhausted)
    }

    pub(crate) fn allocate_saved_color_id(&mut self) -> Result<SavedColorId, ColorLibraryError> {
        take_next_id(&mut self.next_saved_color_id)
            .map(SavedColorId::from_raw)
            .ok_or(ColorLibraryError::IdSpaceExhausted)
    }

    /// Get a reference to a node by ID.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Get a mutable reference to a node by ID.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// Find a persistent guide by stable ID.
    pub fn guide(&self, id: GuideId) -> Option<&Guide> {
        self.guides.iter().find(|guide| guide.id == id)
    }

    /// Add a new node as a child of the given parent.
    /// Returns the ID of the newly created node.
    pub fn add_child(&mut self, parent: NodeId, mut node: Node) -> Option<NodeId> {
        // Verify parent exists
        if !self.nodes.contains_key(parent) {
            return None;
        }

        // Set parent reference
        node.parent = Some(parent);

        // Insert node
        let id = self.nodes.insert(node);

        // Add to parent's children
        if let Some(parent_node) = self.nodes.get_mut(parent) {
            parent_node.children.push(id);
        }

        // Mark transforms dirty
        self.mark_layout_dirty();

        Some(id)
    }

    /// Remove a node and all its descendants from the document (soft delete).
    /// Returns true if the node was removed.
    ///
    /// Note: This uses soft delete to preserve node IDs for the undo system.
    /// Deleted nodes remain in the slotmap but are excluded from iteration.
    pub fn remove(&mut self, id: NodeId) -> bool {
        // Can't remove root
        if id == self.root {
            return false;
        }

        // Get parent before removing
        let parent = self.nodes.get(id).and_then(|n| n.parent);

        // Recursively collect all descendants (including already-deleted ones for cleanup)
        let mut to_remove = vec![id];
        let mut i = 0;
        while i < to_remove.len() {
            if let Some(node) = self.nodes.get(to_remove[i]) {
                to_remove.extend(node.children.iter().cloned());
            }
            i += 1;
        }

        // Remove from parent's children
        if let Some(parent_id) = parent {
            if let Some(parent_node) = self.nodes.get_mut(parent_id) {
                parent_node.children.retain(|&child| child != id);
            }
        }

        // Soft-delete all nodes (mark as deleted but keep in slotmap)
        for node_id in &to_remove {
            if let Some(node) = self.nodes.get_mut(*node_id) {
                node.deleted = true;
            }
            self.cache.invalidate(*node_id);
        }

        self.mark_layout_dirty();

        true
    }

    /// Move a node to a new parent.
    pub fn reparent(&mut self, id: NodeId, new_parent: NodeId, index: usize) -> bool {
        // Can't reparent root
        if id == self.root {
            return false;
        }

        // Verify both nodes exist
        if !self.nodes.contains_key(id) || !self.nodes.contains_key(new_parent) {
            return false;
        }

        // Can't reparent to self or descendant
        if id == new_parent || self.is_ancestor_of(id, new_parent) {
            return false;
        }

        // Remove from old parent
        if let Some(old_parent) = self.nodes.get(id).and_then(|n| n.parent) {
            if let Some(parent_node) = self.nodes.get_mut(old_parent) {
                parent_node.children.retain(|&child| child != id);
            }
        }

        // Add to new parent
        if let Some(parent_node) = self.nodes.get_mut(new_parent) {
            let idx = index.min(parent_node.children.len());
            parent_node.children.insert(idx, id);
        }

        // Update parent reference
        if let Some(node) = self.nodes.get_mut(id) {
            node.parent = Some(new_parent);
        }

        self.mark_layout_dirty();

        true
    }

    /// Check if `ancestor` is an ancestor of `node`.
    pub fn is_ancestor_of(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = node;
        while let Some(parent) = self.nodes.get(current).and_then(|n| n.parent) {
            if parent == ancestor {
                return true;
            }
            current = parent;
        }
        false
    }

    /// Iterate over all ancestors of a node (excluding the node itself).
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.nodes.get(id).and_then(|n| n.parent), move |&id| {
            self.nodes.get(id).and_then(|n| n.parent)
        })
    }

    /// Whether a node and its ancestor chain are live and visible.
    pub fn is_effectively_visible(&self, id: NodeId) -> bool {
        self.nodes
            .get(id)
            .is_some_and(|node| !node.deleted && node.visible)
            && self.ancestors(id).all(|ancestor| {
                self.nodes
                    .get(ancestor)
                    .is_some_and(|node| !node.deleted && node.visible)
            })
    }

    /// Whether a node and its ancestor chain are live, visible, and unlocked.
    pub fn is_effectively_editable(&self, id: NodeId) -> bool {
        self.is_effectively_visible(id)
            && !self
                .ancestors(id)
                .chain(std::iter::once(id))
                .any(|node| self.nodes.get(node).is_some_and(|node| node.locked))
    }

    /// Iterate over all descendants of a node (excluding the node itself).
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        DescendantIterator::new(self, id)
    }

    /// Check if any ancestor of `id` is in the given set.
    pub fn has_selected_ancestor(
        &self,
        id: NodeId,
        selected: &std::collections::HashSet<NodeId>,
    ) -> bool {
        self.ancestors(id)
            .any(|ancestor| selected.contains(&ancestor))
    }

    /// Filter a selection to exclude nodes whose ancestors are also selected.
    ///
    /// This is important for move/transform operations: when moving a group,
    /// we only want to transform the group's root, not its children (since
    /// children inherit the parent's transform automatically).
    pub fn filter_selection_for_transform(&self, selection: &[NodeId]) -> Vec<NodeId> {
        let selected_set: std::collections::HashSet<NodeId> = selection.iter().copied().collect();
        selection
            .iter()
            .copied()
            .filter(|&id| !self.has_selected_ancestor(id, &selected_set))
            .collect()
    }

    /// Iterate nodes in paint order (DFS, children in order).
    /// This is the order in which nodes should be rendered (back to front).
    pub fn paint_order(&self) -> impl Iterator<Item = NodeId> + '_ {
        PaintOrderIterator::new(self)
    }

    /// Iterate nodes in reverse paint order (for hit testing).
    /// This returns nodes from front to back.
    pub fn reverse_paint_order(&self) -> impl Iterator<Item = NodeId> + '_ {
        // Collect and reverse (simple implementation)
        let mut nodes: Vec<_> = self.paint_order().collect();
        nodes.reverse();
        nodes.into_iter()
    }

    /// Mark transforms as dirty for a node and its descendants.
    pub fn mark_transform_dirty(&mut self, id: NodeId) {
        self.dirty.transforms = true;
        self.dirty.bounds = true;
        self.dirty.layout = true;

        // Invalidate cache for this node and descendants
        self.cache.invalidate(id);
        for descendant in self.descendants(id).collect::<Vec<_>>() {
            self.cache.invalidate(descendant);
        }
        for ancestor in self.ancestors(id).collect::<Vec<_>>() {
            self.cache.world_bounds.remove(ancestor);
        }
    }

    /// Mark geometry-dependent bounds as dirty.
    pub fn mark_bounds_dirty(&mut self, id: NodeId) {
        self.dirty.bounds = true;
        self.dirty.layout = true;
        self.dirty.transforms = true;
        self.text_layouts.remove(&id);
        self.cache.world_bounds.remove(id);
        for ancestor in self.ancestors(id).collect::<Vec<_>>() {
            self.cache.world_bounds.remove(ancestor);
        }
    }

    /// Mark layout as dirty (triggers re-layout computation).
    pub fn mark_layout_dirty(&mut self) {
        self.dirty.layout = true;
        self.dirty.transforms = true;
        self.dirty.bounds = true;
    }

    /// Replace platform-shaped text layouts used by bounds and editor interaction geometry.
    pub fn set_text_layouts(&mut self, layouts: HashMap<NodeId, text::TextLayout>) -> bool {
        if self.text_layouts == layouts {
            return false;
        }
        self.text_layouts = layouts;
        self.mark_layout_dirty();
        self.cache.world_bounds.clear();
        true
    }

    /// Resolve the platform layout for a text node, falling back to deterministic metrics.
    pub fn text_layout(&self, id: NodeId) -> Option<text::TextLayout> {
        let node = self.nodes.get(id)?;
        let NodeKind::Text(text) = &node.kind else {
            return None;
        };
        Some(
            self.text_layouts
                .get(&id)
                .cloned()
                .unwrap_or_else(|| text::fallback_text_layout(text)),
        )
    }

    /// Get the layout offset for a node (from auto-layout parent).
    pub fn layout_offset(&mut self, id: NodeId) -> Vec2 {
        self.ensure_layout_valid();
        self.cache.get_layout_offset(id)
    }

    /// Set the layout for a node and mark layout dirty.
    pub fn set_layout(&mut self, id: NodeId, layout: Layout) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.layout = layout;
            self.mark_layout_dirty();
        }
    }

    /// Ensure all layout is up to date.
    fn ensure_layout_valid(&mut self) {
        if self.dirty.layout {
            self.recompute_layout();
            self.dirty.layout = false;
        }
    }

    /// Recompute auto-layout for all groups.
    fn recompute_layout(&mut self) {
        // Clear existing layout offsets
        self.cache.layout_offset.clear();

        // Process layout in DFS order (parents before children)
        self.recompute_layout_recursive(self.root);
    }

    /// Recursively compute layout for auto-layout groups.
    fn recompute_layout_recursive(&mut self, id: NodeId) {
        let node = match self.nodes.get(id) {
            Some(n) => n,
            None => return,
        };

        // Check if this is an auto-layout group
        if let (NodeKind::Group, Layout::Auto(auto_layout)) = (&node.kind, &node.layout) {
            let auto_layout = auto_layout.clone();
            let children = node.children.clone();

            // Measure all children
            let mut entries = Vec::with_capacity(children.len());
            for &child_id in &children {
                if let Some(size) = self.measure_for_layout(child_id) {
                    entries.push(layout::LayoutEntry { id: child_id, size });
                }
            }

            // Compute layout
            let offsets = layout::compute_layout(&auto_layout, &entries, None);

            // Store offsets in cache
            for (child_id, layout_offset) in offsets {
                self.cache.set_layout_offset(child_id, layout_offset.offset);
            }
        }

        // Process children recursively
        let children: Vec<NodeId> = self
            .nodes
            .get(id)
            .map(|n| n.children.clone())
            .unwrap_or_default();

        for child in children {
            self.recompute_layout_recursive(child);
        }
    }

    /// Measure a node for layout purposes.
    fn measure_for_layout(&self, id: NodeId) -> Option<layout::MeasuredSize> {
        let node = self.nodes.get(id)?;

        match &node.kind {
            NodeKind::Group => {
                // For groups, compute bounding box of children
                let mut min = Vec2::splat(f32::INFINITY);
                let mut max = Vec2::splat(f32::NEG_INFINITY);

                for &child_id in &node.children {
                    if let Some(child_size) = self.measure_for_layout(child_id) {
                        let child_node = self.nodes.get(child_id)?;
                        let offset = child_node.transform.translation;

                        min = min.min(offset);
                        max = max.max(offset + child_size.as_vec2());
                    }
                }

                if min.x < f32::INFINITY {
                    Some(layout::MeasuredSize::new(max.x - min.x, max.y - min.y))
                } else {
                    Some(layout::MeasuredSize::new(0.0, 0.0))
                }
            }
            NodeKind::Frame(frame) => {
                // Frame has explicit dimensions
                Some(layout::MeasuredSize::new(frame.width, frame.height))
            }
            NodeKind::Shape(path) => {
                let bounds = path.bounds()?;
                Some(layout::MeasuredSize::from_rect(bounds))
            }
            NodeKind::Text(text) => {
                let platform = self.text_layouts.get(&id);
                let width = platform
                    .map(|layout| layout.width)
                    .unwrap_or_else(|| crate::text::estimate_text_metrics(text).width);
                let height = platform
                    .map(text::TextLayout::height)
                    .unwrap_or_else(|| crate::text::estimate_text_metrics(text).height);
                Some(layout::MeasuredSize::new(width, height))
            }
        }
    }

    /// Ensure all transforms are up to date.
    fn ensure_transforms_valid(&mut self) {
        // First ensure layout is valid (layout affects transforms)
        self.ensure_layout_valid();

        if self.dirty.transforms {
            self.recompute_world_transforms();
            self.dirty.transforms = false;
        }
    }

    /// Clear bounds cached before the most recent geometry or tree change.
    fn ensure_bounds_valid(&mut self) {
        if self.dirty.bounds {
            self.cache.world_bounds.clear();
            self.dirty.bounds = false;
        }
    }

    /// Recompute all world transforms.
    fn recompute_world_transforms(&mut self) {
        self.cache.world_transform.clear();
        self.recompute_world_transform_recursive(self.root, Affine2::IDENTITY);
    }

    /// Recursively compute world transforms.
    /// Uses dual transform model: World = Parent × LayoutOffset × AuthorTransform
    fn recompute_world_transform_recursive(&mut self, id: NodeId, parent_world: Affine2) {
        let node = match self.nodes.get(id) {
            Some(n) => n,
            None => return,
        };

        // Get layout offset (computed by auto-layout) and author transform
        let layout_offset = self.cache.get_layout_offset(id);
        let author_transform = node.transform;

        // Compose: Parent × LayoutOffset × AuthorTransform
        let layout_transform = Affine2::from_translation(layout_offset);
        let world = parent_world * layout_transform * author_transform;

        self.cache.world_transform.insert(id, world);

        // Get children (need to clone to avoid borrow issues)
        let children: Vec<NodeId> = self
            .nodes
            .get(id)
            .map(|n| n.children.clone())
            .unwrap_or_default();

        for child in children {
            self.recompute_world_transform_recursive(child, world);
        }
    }

    /// Get the world transform for a node.
    pub fn world_transform(&mut self, id: NodeId) -> Affine2 {
        self.ensure_transforms_valid();
        self.cache
            .world_transform
            .get(id)
            .copied()
            .unwrap_or(Affine2::IDENTITY)
    }

    /// Convert a desired world transform into a node-local author transform.
    pub fn author_transform_from_world(&mut self, id: NodeId, world: Affine2) -> Affine2 {
        self.ensure_transforms_valid();
        let parent_world = self
            .nodes
            .get(id)
            .and_then(|node| node.parent)
            .and_then(|parent| self.cache.world_transform.get(parent).copied())
            .unwrap_or(Affine2::IDENTITY);
        let layout = Affine2::from_translation(self.cache.get_layout_offset(id));
        let local_from_world = parent_world * layout;
        let Some(local_inverse) = finite_affine_inverse(local_from_world) else {
            return self
                .nodes
                .get(id)
                .map(|node| node.transform)
                .unwrap_or(Affine2::IDENTITY);
        };
        if !affine_is_finite(world) {
            return self
                .nodes
                .get(id)
                .map(|node| node.transform)
                .unwrap_or(Affine2::IDENTITY);
        }
        local_inverse * world
    }

    /// Reapply desired world transforms until ancestor auto-layout measurement
    /// and parent-controlled offsets stabilize.
    pub(crate) fn restore_world_transforms(&mut self, preserved: &[(NodeId, Affine2)]) {
        const MAX_LAYOUT_PASSES: usize = 16;
        const TRANSFORM_EPSILON: f32 = 1e-6;

        for _ in 0..MAX_LAYOUT_PASSES {
            // Derive the whole batch from one stable layout snapshot. Applying
            // one child before deriving the next would make the result depend
            // on selection order while the ancestor group is being measured.
            let updates = preserved
                .iter()
                .filter_map(|&(id, world)| {
                    let transform = self.author_transform_from_world(id, world);
                    let before = self.nodes.get(id)?.transform;
                    before
                        .to_cols_array()
                        .into_iter()
                        .zip(transform.to_cols_array())
                        .any(|(before, after)| (before - after).abs() > TRANSFORM_EPSILON)
                        .then_some((id, transform))
                })
                .collect::<Vec<_>>();
            if updates.is_empty() {
                break;
            }
            for &(id, transform) in &updates {
                if let Some(node) = self.nodes.get_mut(id) {
                    node.transform = transform;
                }
            }
            for (id, _) in updates {
                self.mark_transform_dirty(id);
            }
        }
    }

    /// Apply a world-space delta to an original node transform.
    pub fn transform_with_world_delta(
        &mut self,
        id: NodeId,
        original: Affine2,
        delta: Affine2,
    ) -> Affine2 {
        let current = self.nodes.get(id).map(|node| node.transform);
        if let Some(node) = self.nodes.get_mut(id) {
            node.transform = original;
        }
        self.mark_transform_dirty(id);
        let original_world = self.world_transform(id);
        let transformed = self.author_transform_from_world(id, delta * original_world);
        if let (Some(node), Some(current)) = (self.nodes.get_mut(id), current) {
            node.transform = current;
        }
        self.mark_transform_dirty(id);
        transformed
    }

    /// Get the local bounds for a node (in node's coordinate space).
    /// - Group: None (computed from children via world_bounds)
    /// - Frame: explicit width × height (independent of children)
    /// - Shape: path bounding box
    /// - Text: text metrics estimate
    pub fn local_bounds(&self, id: NodeId) -> Option<Rect> {
        let node = self.nodes.get(id)?;

        match &node.kind {
            NodeKind::Group => {
                // Group bounds are computed from children
                // For now, return None (will be computed from world bounds)
                None
            }
            NodeKind::Frame(frame) => {
                // Frame has explicit dimensions independent of children
                Some(Rect::from_pos_size(
                    Vec2::ZERO,
                    Vec2::new(frame.width, frame.height),
                ))
            }
            NodeKind::Shape(path) => path.bounds(),
            NodeKind::Text(text) => Some(self.text_layouts.get(&id).map_or_else(
                || crate::text::estimate_text_metrics(text).bounds(),
                |layout| Rect::from_pos_size(Vec2::ZERO, Vec2::new(layout.width, layout.height())),
            )),
        }
    }

    /// Get the world bounds for a node (axis-aligned in world space).
    pub fn world_bounds(&mut self, id: NodeId) -> Option<Rect> {
        self.ensure_transforms_valid();
        self.ensure_bounds_valid();

        // Check cache first
        if let Some(&bounds) = self.cache.world_bounds.get(id) {
            if !bounds.is_empty() {
                return Some(bounds);
            }
        }

        let bounds = self.compute_world_bounds(id)?;
        self.cache.world_bounds.insert(id, bounds);
        Some(bounds)
    }

    /// Compute world bounds for a node (not cached).
    fn compute_world_bounds(&mut self, id: NodeId) -> Option<Rect> {
        // Extract node data first to avoid borrow conflicts
        let node = self.nodes.get(id)?;
        let kind = node.kind.clone();
        let children = node.children.clone();

        let world_transform = self.world_transform(id);

        match kind {
            NodeKind::Group => {
                // Union of all children's world bounds
                let mut combined = Rect::empty();

                for child in children {
                    if self
                        .nodes
                        .get(child)
                        .is_none_or(|node| node.deleted || !node.visible)
                    {
                        continue;
                    }
                    if let Some(child_bounds) = self.world_bounds(child) {
                        if combined.is_empty() {
                            combined = child_bounds;
                        } else {
                            combined = combined.union(&child_bounds);
                        }
                    }
                }

                if combined.is_empty() {
                    None
                } else {
                    Some(combined)
                }
            }
            NodeKind::Frame(frame) => {
                // Frame has explicit dimensions
                let local_bounds =
                    Rect::from_pos_size(Vec2::ZERO, Vec2::new(frame.width, frame.height));
                Some(transform_rect(local_bounds, world_transform))
            }
            NodeKind::Shape(path) => {
                let local_bounds = path.bounds()?;
                Some(transform_rect(local_bounds, world_transform))
            }
            NodeKind::Text(text) => {
                let local_bounds = self.text_layouts.get(&id).map_or_else(
                    || crate::text::estimate_text_metrics(&text).bounds(),
                    |layout| {
                        Rect::from_pos_size(Vec2::ZERO, Vec2::new(layout.width, layout.height()))
                    },
                );
                Some(transform_rect(local_bounds, world_transform))
            }
        }
    }

    /// Get snap points for a node (corners, centers, edges).
    pub fn snap_points(&mut self, id: NodeId) -> Vec<snap::SnapPoint> {
        if let Some(bounds) = self.world_bounds(id) {
            snap::SnapPoint::from_bounds(&bounds, Some(id))
        } else {
            vec![]
        }
    }

    /// Get all snap points outside the given excluded subtrees.
    pub fn all_snap_points(&mut self, exclude: &[NodeId]) -> Vec<snap::SnapPoint> {
        let mut points = Vec::new();
        let excluded: HashSet<_> = exclude.iter().copied().collect();

        // Collect all visible, non-group nodes
        let ids: Vec<NodeId> = self
            .descendants(self.root)
            .filter(|&id| {
                if excluded.contains(&id) || self.has_selected_ancestor(id, &excluded) {
                    return false;
                }
                if let Some(node) = self.nodes.get(id) {
                    !node.deleted
                        && node.visible
                        && !node.locked
                        && !node.is_group()
                        && !self.ancestors(id).any(|ancestor| {
                            self.nodes.get(ancestor).is_some_and(|ancestor| {
                                ancestor.deleted || !ancestor.visible || ancestor.locked
                            })
                        })
                } else {
                    false
                }
            })
            .collect();

        for id in ids {
            points.extend(self.snap_points(id));
        }

        points.extend(self.guides.iter().copied().map(snap::SnapPoint::from_guide));

        points
    }

    /// Hit test: find the topmost visible node at a world position.
    ///
    /// By default, this only hits leaf nodes (shapes, text). Groups are skipped.
    /// Use `hit_test_with_groups` to include groups.
    pub fn hit_test(&mut self, world_pos: Vec2) -> Option<NodeId> {
        self.hit_test_internal(world_pos, false)
    }

    /// Hit test including groups.
    ///
    /// This is useful when Ctrl+clicking to select group containers.
    pub fn hit_test_with_groups(&mut self, world_pos: Vec2) -> Option<NodeId> {
        self.hit_test_internal(world_pos, true)
    }

    /// Internal hit test implementation.
    fn hit_test_internal(&mut self, world_pos: Vec2, include_groups: bool) -> Option<NodeId> {
        self.hit_test_with_tolerance(world_pos, include_groups, 4.0)
    }

    /// Hit test with a caller-provided world-space stroke tolerance.
    pub fn hit_test_with_tolerance(
        &mut self,
        world_pos: Vec2,
        include_groups: bool,
        tolerance: f32,
    ) -> Option<NodeId> {
        let mut clip_containment_cache = HashMap::new();
        for id in self.reverse_paint_order().collect::<Vec<_>>() {
            let Some(node) = self.nodes.get(id) else {
                continue;
            };

            if node.deleted
                || !node.visible
                || node.locked
                || self.ancestors(id).any(|ancestor| {
                    self.nodes
                        .get(ancestor)
                        .is_some_and(|node| node.deleted || !node.visible || node.locked)
                })
            {
                continue;
            }

            if !include_groups && node.is_group() {
                continue;
            }

            let kind = node.kind.clone();
            let style = node.style.clone();
            if !self.clip_chain_contains(id, world_pos, &mut clip_containment_cache) {
                continue;
            }
            if matches!(kind, NodeKind::Group) {
                if self
                    .world_bounds(id)
                    .is_some_and(|bounds| bounds.contains(world_pos))
                {
                    return Some(id);
                }
                continue;
            }

            let world = self.world_transform(id);
            let Some(world_inverse) = finite_affine_inverse(world) else {
                continue;
            };
            let local = world_inverse.transform_point2(world_pos);
            let minimum_scale = world
                .matrix2
                .x_axis
                .length()
                .min(world.matrix2.y_axis.length())
                .max(f32::EPSILON);
            let local_tolerance = tolerance / minimum_scale;

            let hit = match kind {
                NodeKind::Group => false,
                NodeKind::Frame(frame) => {
                    Rect::from_pos_size(Vec2::ZERO, Vec2::new(frame.width, frame.height))
                        .contains(local)
                }
                NodeKind::Text(_) => self
                    .local_bounds(id)
                    .is_some_and(|bounds| bounds.contains(local)),
                NodeKind::Shape(path) => {
                    let fill_hit = style.fill.is_some()
                        && path.contains_point_with_rule(local, style.fill_rule);
                    let stroke_hit = style.stroke.is_some_and(|stroke| {
                        path.distance_to_point(local, local_tolerance * 0.25)
                            <= stroke.width * 0.5 + local_tolerance
                    });
                    fill_hit || stroke_hit
                }
            };

            if hit {
                return Some(id);
            }
        }

        None
    }

    fn clip_chain_contains(
        &mut self,
        id: NodeId,
        world_point: Vec2,
        containment_cache: &mut HashMap<NodeId, bool>,
    ) -> bool {
        let clipped_nodes = std::iter::once(id)
            .chain(self.ancestors(id))
            .filter(|node_id| {
                self.nodes
                    .get(*node_id)
                    .is_some_and(|node| node.clip_path.is_some())
            })
            .collect::<Vec<_>>();
        for clipped_node in clipped_nodes {
            let contains = if let Some(&contains) = containment_cache.get(&clipped_node) {
                contains
            } else {
                let world = self.world_transform(clipped_node);
                let contains = finite_affine_inverse(world).is_some_and(|world_inverse| {
                    let local_point = world_inverse.transform_point2(world_point);
                    self.nodes
                        .get(clipped_node)
                        .and_then(|node| node.clip_path.as_ref())
                        .is_some_and(|clip| clip_path_contains_point(clip, local_point))
                });
                #[cfg(test)]
                CLIP_CONTAINMENT_EVALUATIONS
                    .with(|evaluations| evaluations.set(evaluations.get() + 1));
                containment_cache.insert(clipped_node, contains);
                contains
            };
            if !contains {
                return false;
            }
        }
        true
    }

    /// Group the given nodes into a new group.
    /// Returns the ID of the new group, or None if grouping failed.
    ///
    /// The new group is created as a sibling of the first node's parent,
    /// at the position of the first node in the z-order.
    pub fn group_nodes(&mut self, nodes: &[NodeId]) -> Option<NodeId> {
        if nodes.is_empty() {
            return None;
        }

        // Can't group the root
        if nodes.contains(&self.root) {
            return None;
        }

        // Verify all nodes exist exactly once and get their common parent.
        let unique = nodes.iter().copied().collect::<HashSet<_>>();
        if unique.len() != nodes.len() {
            return None;
        }
        let first_parent = self.nodes.get(nodes[0])?.parent?;
        if self.nodes.get(first_parent)?.deleted {
            return None;
        }

        // All nodes must have the same parent for simple grouping.
        for &id in nodes {
            let node = self.nodes.get(id)?;
            if node.deleted || node.parent != Some(first_parent) {
                return None;
            }
        }

        let parent_children = self.nodes.get(first_parent)?.children.clone();
        let mut nodes = nodes.to_vec();
        nodes.sort_by_key(|id| {
            parent_children
                .iter()
                .position(|child| child == id)
                .unwrap_or(usize::MAX)
        });
        if nodes
            .iter()
            .any(|id| !parent_children.iter().any(|child| child == id))
        {
            return None;
        }
        let min_index = parent_children
            .iter()
            .position(|child| child == &nodes[0])?;

        // Reparenting changes both ancestor transforms and auto-layout
        // translations. Snapshot the visual result before changing the tree.
        let parent_world = self.world_transform(first_parent);
        finite_affine_inverse(parent_world)?;
        let preserved = nodes
            .iter()
            .map(|&id| (id, self.world_transform(id)))
            .collect::<Vec<_>>();

        // Create the new group.
        let group = Node::group("Group");
        let group_id = self.nodes.insert(group);

        if let Some(g) = self.nodes.get_mut(group_id) {
            g.parent = Some(first_parent);
        }

        if let Some(parent) = self.nodes.get_mut(first_parent) {
            parent.children.retain(|c| !unique.contains(c));
            parent.children.insert(min_index, group_id);
        }

        for &id in &nodes {
            if let Some(node) = self.nodes.get_mut(id) {
                node.parent = Some(group_id);
            }
        }
        if let Some(g) = self.nodes.get_mut(group_id) {
            g.children = nodes;
        }

        self.mark_layout_dirty();
        self.restore_world_transforms(&preserved);

        Some(group_id)
    }

    /// Ungroup a group node, moving its children to the group's parent.
    /// Returns the IDs of the ungrouped children, or None if ungrouping failed.
    pub fn ungroup(&mut self, group_id: NodeId) -> Option<Vec<NodeId>> {
        // Can't ungroup root
        if group_id == self.root {
            return None;
        }

        let node = self.nodes.get(group_id)?;

        // Must be a group
        if node.deleted || !node.is_group() {
            return None;
        }

        let parent_id = node.parent?;
        let children = node.children.clone();

        if children.is_empty() {
            // Just remove empty group
            self.remove(group_id);
            return Some(vec![]);
        }

        let parent_world = self.world_transform(parent_id);
        finite_affine_inverse(parent_world)?;
        let preserved = children
            .iter()
            .map(|&id| (id, self.world_transform(id)))
            .collect::<Vec<_>>();

        // Find group's index in parent.
        let group_index = self
            .nodes
            .get(parent_id)?
            .children
            .iter()
            .position(|&c| c == group_id)?;

        // Remove group from parent
        if let Some(parent) = self.nodes.get_mut(parent_id) {
            parent.children.retain(|&c| c != group_id);
        }

        // Insert children at group's position
        for (i, &child_id) in children.iter().enumerate() {
            if let Some(child) = self.nodes.get_mut(child_id) {
                child.parent = Some(parent_id);
            }
            if let Some(parent) = self.nodes.get_mut(parent_id) {
                parent.children.insert(group_index + i, child_id);
            }
        }

        // Soft-delete the group node
        if let Some(group) = self.nodes.get_mut(group_id) {
            group.deleted = true;
        }
        self.cache.invalidate(group_id);

        self.mark_layout_dirty();
        self.restore_world_transforms(&preserved);

        Some(children)
    }

    /// Bring a node to the front (top of z-order) within its parent.
    pub fn bring_to_front(&mut self, id: NodeId) -> bool {
        if id == self.root {
            return false;
        }

        let parent_id = self.nodes.get(id).and_then(|n| n.parent);
        if let Some(parent_id) = parent_id {
            if let Some(parent) = self.nodes.get_mut(parent_id) {
                parent.children.retain(|&c| c != id);
                parent.children.push(id);
                self.mark_layout_dirty();
                return true;
            }
        }
        false
    }

    /// Send a node to the back (bottom of z-order) within its parent.
    pub fn send_to_back(&mut self, id: NodeId) -> bool {
        if id == self.root {
            return false;
        }

        let parent_id = self.nodes.get(id).and_then(|n| n.parent);
        if let Some(parent_id) = parent_id {
            if let Some(parent) = self.nodes.get_mut(parent_id) {
                parent.children.retain(|&c| c != id);
                parent.children.insert(0, id);
                self.mark_layout_dirty();
                return true;
            }
        }
        false
    }

    /// Duplicate a node (and its descendants).
    /// Returns the ID of the new node, or None if duplication failed.
    pub fn duplicate(&mut self, id: NodeId) -> Option<NodeId> {
        if id == self.root {
            return None;
        }

        let node = self.nodes.get(id)?.clone();
        let parent = node.parent?;

        // Find index in parent
        let index = self
            .nodes
            .get(parent)?
            .children
            .iter()
            .position(|&c| c == id)?;

        // Deep clone the node and its descendants
        let new_id = self.deep_clone(id, parent, index + 1)?;

        Some(new_id)
    }

    /// Deep clone a node and its descendants.
    fn deep_clone(&mut self, id: NodeId, new_parent: NodeId, index: usize) -> Option<NodeId> {
        let node = self.nodes.get(id)?;
        let mut cloned = node.clone();
        cloned.parent = Some(new_parent);
        cloned.name = format!("{} copy", cloned.name);

        let old_children = cloned.children.clone();
        cloned.children.clear();

        let new_id = self.nodes.insert(cloned);

        // Add to parent
        if let Some(parent) = self.nodes.get_mut(new_parent) {
            let idx = index.min(parent.children.len());
            parent.children.insert(idx, new_id);
        }

        // Clone children
        for (i, &child_id) in old_children.iter().enumerate() {
            // `deep_clone` inserts the cloned child into `new_id`; appending it
            // again here would duplicate every child reference.
            self.deep_clone(child_id, new_id, i)?;
        }

        self.mark_layout_dirty();

        Some(new_id)
    }

    // === Serialization ===

    /// Convert document to a serializable format.
    pub fn to_saved(&self) -> SavedDocument {
        let mut nodes = SlotMap::with_capacity_and_key(self.nodes.len());
        let mut remapped_ids = HashMap::with_capacity(self.nodes.len());
        for (id, node) in &self.nodes {
            if !node.deleted {
                remapped_ids.insert(id, nodes.insert(node.clone()));
            }
        }

        for node in nodes.values_mut() {
            node.parent = node
                .parent
                .and_then(|parent| remapped_ids.get(&parent).copied());
            node.children = node
                .children
                .iter()
                .filter_map(|child| remapped_ids.get(child).copied())
                .collect();
        }
        let root = remapped_ids.get(&self.root).copied().unwrap_or_else(|| {
            let missing_root = nodes.insert(Node::group("Invalid missing root"));
            nodes.remove(missing_root);
            missing_root
        });
        SavedDocument {
            version: SavedDocument::CURRENT_VERSION,
            nodes,
            root,
            grid: self.grid,
            guides: self.guides.clone(),
            color_library: self.color_library.clone(),
        }
    }

    /// Build and validate the exact snapshot that will be persisted.
    pub fn to_validated_saved(&self) -> Result<SavedDocument, DocumentValidationError> {
        let saved = self.to_saved();
        Self::validate_saved_graph(&saved)?;
        Ok(saved)
    }

    /// Create a document from a saved format after validating its live scene graph.
    pub fn from_saved(mut saved: SavedDocument) -> Result<Self, DocumentLoadError> {
        if saved.version > SavedDocument::CURRENT_VERSION {
            return Err(DocumentLoadError::UnsupportedVersion {
                version: saved.version,
                current: SavedDocument::CURRENT_VERSION,
            });
        }
        if saved.version < 3 {
            saved.grid = GridSettings::default();
            saved.guides.clear();
            saved.color_library = ColorLibrary::default();
        }
        Self::validate_saved_graph(&saved)
            .map_err(|reason| DocumentLoadError::InvalidDocument { reason })?;

        let next_guide_id = next_stable_id(saved.guides.iter().map(|guide| guide.id.get()));
        let next_color_group_id = next_stable_id(
            saved
                .color_library
                .groups
                .iter()
                .map(|group| group.id.get()),
        );
        let next_saved_color_id = next_stable_id(
            saved
                .color_library
                .colors
                .iter()
                .map(|color| color.id.get()),
        );

        Ok(Self {
            nodes: saved.nodes,
            root: saved.root,
            grid: saved.grid,
            guides: saved.guides,
            color_library: saved.color_library,
            cache: Cache::new(),
            dirty: DirtyFlags::all_dirty(),
            text_layouts: HashMap::new(),
            next_guide_id,
            next_color_group_id,
            next_saved_color_id,
        })
    }

    fn validate_saved_graph(saved: &SavedDocument) -> Result<(), DocumentValidationError> {
        Self::validate_saved_complexity(saved, DocumentComplexityLimits::DEFAULT)?;
        Self::validate_precision_data(saved)?;
        saved
            .color_library
            .validate()
            .map_err(|reason| DocumentValidationError::InvalidColorLibrary { reason })?;

        let root = saved
            .nodes
            .get(saved.root)
            .ok_or(DocumentValidationError::MissingRoot { root: saved.root })?;
        if root.deleted {
            return Err(DocumentValidationError::DeletedRoot { root: saved.root });
        }
        if let Some(parent) = root.parent {
            return Err(DocumentValidationError::RootHasParent {
                root: saved.root,
                parent,
            });
        }
        if !root.is_group() {
            return Err(DocumentValidationError::RootNotGroup { root: saved.root });
        }

        for (node_id, node) in &saved.nodes {
            if let Some(parent) = node.parent {
                if !saved.nodes.contains_key(parent) {
                    return Err(DocumentValidationError::DanglingParent {
                        node: node_id,
                        parent,
                    });
                }
            }

            let mut children = HashSet::with_capacity(node.children.len());
            for &child in &node.children {
                if !saved.nodes.contains_key(child) {
                    return Err(DocumentValidationError::DanglingChild {
                        parent: node_id,
                        child,
                    });
                }
                if !children.insert(child) {
                    return Err(DocumentValidationError::DuplicateChild {
                        parent: node_id,
                        child,
                    });
                }
            }
        }

        let live_nodes = saved
            .nodes
            .iter()
            .filter_map(|(id, node)| (!node.deleted).then_some(id))
            .collect::<Vec<_>>();

        for &node_id in &live_nodes {
            let node = &saved.nodes[node_id];
            if node_id != saved.root && node.parent.is_none() {
                return Err(DocumentValidationError::MultipleRoots {
                    declared_root: saved.root,
                    additional_root: node_id,
                });
            }

            for &child in &node.children {
                let child_node = &saved.nodes[child];
                if !child_node.deleted && child_node.parent != Some(node_id) {
                    return Err(DocumentValidationError::ParentChildMismatch {
                        parent: node_id,
                        child,
                    });
                }
            }

            if let Some(parent) = node.parent {
                let parent_node = &saved.nodes[parent];
                if !parent_node.deleted && !parent_node.children.contains(&node_id) {
                    return Err(DocumentValidationError::ParentChildMismatch {
                        parent,
                        child: node_id,
                    });
                }
            }
        }

        let all_nodes = saved.nodes.keys().collect::<Vec<_>>();
        let mut checked = HashSet::with_capacity(all_nodes.len());
        for &start in &all_nodes {
            if checked.contains(&start) {
                continue;
            }

            let mut path = HashSet::new();
            let mut chain = Vec::new();
            let mut current = start;
            loop {
                if checked.contains(&current) {
                    break;
                }
                if !path.insert(current) {
                    return Err(DocumentValidationError::Cycle { node: current });
                }
                chain.push(current);

                let Some(parent) = saved.nodes[current].parent else {
                    break;
                };
                current = parent;
            }
            checked.extend(chain);
        }

        let mut incoming_edges = all_nodes
            .iter()
            .copied()
            .map(|node| (node, 0_usize))
            .collect::<HashMap<_, _>>();
        for (parent, node) in &saved.nodes {
            for child in &node.children {
                let incoming = incoming_edges.get_mut(child).ok_or(
                    DocumentValidationError::DanglingChild {
                        parent,
                        child: *child,
                    },
                )?;
                *incoming += 1;
            }
        }
        let mut leaves = incoming_edges
            .iter()
            .filter_map(|(&node, &incoming)| (incoming == 0).then_some(node))
            .collect::<VecDeque<_>>();
        let mut visited_count = 0;
        while let Some(node_id) = leaves.pop_front() {
            visited_count += 1;
            for child in &saved.nodes[node_id].children {
                let incoming = incoming_edges.get_mut(child).ok_or(
                    DocumentValidationError::DanglingChild {
                        parent: node_id,
                        child: *child,
                    },
                )?;
                *incoming -= 1;
                if *incoming == 0 {
                    leaves.push_back(*child);
                }
            }
        }
        if visited_count != all_nodes.len() {
            if let Some(node) = incoming_edges
                .into_iter()
                .find_map(|(node, incoming)| (incoming > 0).then_some(node))
            {
                return Err(DocumentValidationError::Cycle { node });
            }
        }

        let mut reachable = HashSet::with_capacity(live_nodes.len());
        let mut stack = vec![saved.root];
        while let Some(node_id) = stack.pop() {
            if !reachable.insert(node_id) {
                continue;
            }
            stack.extend(
                saved.nodes[node_id]
                    .children
                    .iter()
                    .copied()
                    .filter(|child| !saved.nodes[*child].deleted),
            );
        }

        if let Some(&node) = live_nodes.iter().find(|node| !reachable.contains(node)) {
            return Err(DocumentValidationError::UnreachableNode { node });
        }

        Ok(())
    }

    fn validate_precision_data(saved: &SavedDocument) -> Result<(), DocumentValidationError> {
        saved
            .grid
            .validate()
            .map_err(|reason| DocumentValidationError::InvalidPrecisionData { reason })?;
        if saved.guides.len() > MAX_GUIDES {
            return Err(DocumentValidationError::InvalidPrecisionData {
                reason: PrecisionError::TooManyGuides,
            });
        }

        let mut ids = HashSet::with_capacity(saved.guides.len());
        for guide in &saved.guides {
            if guide.id.get() == 0 {
                return Err(DocumentValidationError::InvalidPrecisionData {
                    reason: PrecisionError::InvalidGuideId,
                });
            }
            if !guide.position.is_finite() {
                return Err(DocumentValidationError::InvalidPrecisionData {
                    reason: PrecisionError::InvalidGuidePosition,
                });
            }
            if !ids.insert(guide.id) {
                return Err(DocumentValidationError::InvalidPrecisionData {
                    reason: PrecisionError::DuplicateGuideId(guide.id),
                });
            }
        }
        Ok(())
    }

    fn validate_saved_complexity(
        saved: &SavedDocument,
        limits: DocumentComplexityLimits,
    ) -> Result<(), DocumentValidationError> {
        if saved.nodes.len() > limits.nodes {
            return Err(DocumentValidationError::TooManyNodes {
                count: saved.nodes.len(),
                limit: limits.nodes,
            });
        }

        for (node_id, node) in &saved.nodes {
            if node.children.len() > limits.children_per_node {
                return Err(DocumentValidationError::TooManyChildren {
                    node: node_id,
                    count: node.children.len(),
                    limit: limits.children_per_node,
                });
            }

            validate_node_visual_data(node_id, node)?;

            match &node.kind {
                NodeKind::Shape(path) => {
                    if path.contours.len() > limits.path_contours_per_node {
                        return Err(DocumentValidationError::TooManyPathContours {
                            node: node_id,
                            count: path.contours.len(),
                            limit: limits.path_contours_per_node,
                        });
                    }
                    let anchor_count = path.contours.iter().fold(0_usize, |count, contour| {
                        count.saturating_add(contour.anchors.len())
                    });
                    if anchor_count > limits.path_anchors_per_node {
                        return Err(DocumentValidationError::TooManyPathAnchors {
                            node: node_id,
                            count: anchor_count,
                            limit: limits.path_anchors_per_node,
                        });
                    }
                }
                NodeKind::Text(text) if text.content.len() > limits.text_bytes_per_node => {
                    return Err(DocumentValidationError::TextTooLong {
                        node: node_id,
                        bytes: text.content.len(),
                        limit: limits.text_bytes_per_node,
                    });
                }
                NodeKind::Group | NodeKind::Frame(_) | NodeKind::Text(_) => {}
            }
        }

        Ok(())
    }

    fn validate_serialized_slot_count(
        count: usize,
        limit: usize,
    ) -> Result<(), DocumentValidationError> {
        if count > limit {
            return Err(DocumentValidationError::TooManyNodeSlots { count, limit });
        }
        Ok(())
    }

    /// Serialize document to JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.to_saved())
    }

    /// Deserialize document from JSON string.
    pub fn from_json(json: &str) -> Result<Self, DocumentLoadError> {
        #[derive(Default, Deserialize)]
        struct ColorLibraryHeader {
            #[serde(default)]
            groups: Vec<IgnoredAny>,
            #[serde(default)]
            colors: Vec<IgnoredAny>,
        }

        #[derive(Deserialize)]
        struct StorageHeader {
            version: u32,
            #[serde(default)]
            nodes: Vec<IgnoredAny>,
            #[serde(default)]
            guides: Vec<IgnoredAny>,
            #[serde(default)]
            color_library: ColorLibraryHeader,
        }
        let header: StorageHeader = serde_json::from_str(json)?;
        if header.version > SavedDocument::CURRENT_VERSION {
            return Err(DocumentLoadError::UnsupportedVersion {
                version: header.version,
                current: SavedDocument::CURRENT_VERSION,
            });
        }
        Self::validate_serialized_slot_count(
            header.nodes.len(),
            DocumentComplexityLimits::DEFAULT.nodes.saturating_add(1),
        )
        .map_err(|reason| DocumentLoadError::InvalidDocument { reason })?;
        if header.guides.len() > MAX_GUIDES {
            return Err(DocumentLoadError::InvalidDocument {
                reason: DocumentValidationError::InvalidPrecisionData {
                    reason: PrecisionError::TooManyGuides,
                },
            });
        }
        if header.color_library.groups.len() > MAX_COLOR_GROUPS {
            return Err(DocumentLoadError::InvalidDocument {
                reason: DocumentValidationError::InvalidColorLibrary {
                    reason: ColorLibraryError::TooManyGroups,
                },
            });
        }
        if header.color_library.colors.len() > MAX_SAVED_COLORS {
            return Err(DocumentLoadError::InvalidDocument {
                reason: DocumentValidationError::InvalidColorLibrary {
                    reason: ColorLibraryError::TooManyColors,
                },
            });
        }
        let saved: SavedDocument = serde_json::from_str(json)?;
        Self::from_saved(saved)
    }
}

fn validate_node_visual_data(node_id: NodeId, node: &Node) -> Result<(), DocumentValidationError> {
    if !affine_is_finite(node.transform) {
        return Err(invalid_visual(node_id, "non-finite node transform"));
    }
    if !is_normalized(node.style.opacity) {
        return Err(invalid_visual(
            node_id,
            "opacity must be between zero and one",
        ));
    }
    if let Some(fill) = &node.style.fill {
        validate_paint(node_id, fill)?;
    }
    if let Some(stroke) = &node.style.stroke {
        validate_stroke(node_id, stroke)?;
    }
    if let NodeKind::Shape(path) = &node.kind {
        validate_path_geometry(node_id, path, "non-finite path geometry")?;
    }
    if let NodeKind::Frame(frame) = &node.kind {
        if !frame.width.is_finite()
            || !frame.height.is_finite()
            || frame.width < 0.0
            || frame.height < 0.0
        {
            return Err(invalid_visual(
                node_id,
                "frame dimensions must be finite and nonnegative",
            ));
        }
        if let Some(background) = &frame.background {
            validate_paint(node_id, background)?;
        }
    }
    if let Some(clip) = &node.clip_path {
        let mut state = ClipValidationState::default();
        validate_clip_path(node_id, clip, 0, &mut state)?;
    }
    Ok(())
}

fn validate_stroke(node_id: NodeId, stroke: &Stroke) -> Result<(), DocumentValidationError> {
    validate_paint(node_id, &stroke.paint)?;
    if !stroke.width.is_finite() || stroke.width < 0.0 {
        return Err(invalid_visual(
            node_id,
            "stroke width must be finite and nonnegative",
        ));
    }
    if !stroke.miter_limit.is_finite() || stroke.miter_limit < 1.0 {
        return Err(invalid_visual(
            node_id,
            "stroke miter limit must be finite and at least one",
        ));
    }
    let maximum_outset = stroke.width * 0.5 * stroke.miter_limit;
    if !maximum_outset.is_finite() {
        return Err(invalid_visual(
            node_id,
            "stroke width and miter limit produce a non-finite visual extent",
        ));
    }
    if stroke.dash_array.len() > MAX_STROKE_DASH_VALUES_PER_NODE {
        return Err(visual_complexity(
            node_id,
            "stroke dash values",
            stroke.dash_array.len(),
            MAX_STROKE_DASH_VALUES_PER_NODE,
        ));
    }
    let dash_total = stroke.dash_array.iter().sum::<f32>();
    if !stroke.dash_offset.is_finite()
        || stroke
            .dash_array
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        || (!stroke.dash_array.is_empty() && (!dash_total.is_finite() || dash_total <= 0.0))
    {
        return Err(invalid_visual(node_id, "invalid stroke dash pattern"));
    }
    Ok(())
}

fn validate_paint(node_id: NodeId, paint: &Paint) -> Result<(), DocumentValidationError> {
    match paint {
        Paint::Solid(color) => validate_color(node_id, color),
        Paint::LinearGradient(gradient) => {
            if !gradient.start.is_finite()
                || !gradient.end.is_finite()
                || !affine_is_finite(gradient.transform)
            {
                return Err(invalid_visual(node_id, "invalid linear gradient geometry"));
            }
            validate_gradient_stops(node_id, &gradient.stops)
        }
        Paint::RadialGradient(gradient) => {
            if !gradient.center.is_finite()
                || !gradient.focal.is_finite()
                || !gradient.radius.is_finite()
                || gradient.radius <= 0.0
                || !affine_is_finite(gradient.transform)
            {
                return Err(invalid_visual(node_id, "invalid radial gradient geometry"));
            }
            validate_gradient_stops(node_id, &gradient.stops)
        }
    }
}

fn validate_gradient_stops(
    node_id: NodeId,
    stops: &[GradientStop],
) -> Result<(), DocumentValidationError> {
    if stops.len() < 2 {
        return Err(invalid_visual(
            node_id,
            "gradients require at least two stops",
        ));
    }
    if stops.len() > MAX_GRADIENT_STOPS_PER_NODE {
        return Err(visual_complexity(
            node_id,
            "gradient stops",
            stops.len(),
            MAX_GRADIENT_STOPS_PER_NODE,
        ));
    }
    let mut previous = 0.0;
    for stop in stops {
        if !is_normalized(stop.offset) || stop.offset < previous {
            return Err(invalid_visual(
                node_id,
                "gradient offsets must be finite, normalized, and ordered",
            ));
        }
        validate_color(node_id, &stop.color)?;
        previous = stop.offset;
    }
    Ok(())
}

fn validate_color(node_id: NodeId, color: &[f32; 4]) -> Result<(), DocumentValidationError> {
    if color.iter().copied().all(is_normalized) {
        Ok(())
    } else {
        Err(invalid_visual(
            node_id,
            "paint channels must be finite and between zero and one",
        ))
    }
}

#[derive(Default)]
struct ClipValidationState {
    nodes: usize,
    contours: usize,
    anchors: usize,
}

fn validate_clip_path(
    node_id: NodeId,
    clip: &ClipPath,
    depth: usize,
    state: &mut ClipValidationState,
) -> Result<(), DocumentValidationError> {
    if depth > MAX_CLIP_DEPTH {
        return Err(visual_complexity(
            node_id,
            "clip nesting levels",
            depth,
            MAX_CLIP_DEPTH,
        ));
    }
    if !affine_is_finite(clip.transform) {
        return Err(invalid_visual(node_id, "non-finite clip transform"));
    }
    state.nodes = state.nodes.saturating_add(1);
    if state.nodes > MAX_CLIP_NODES_PER_NODE {
        return Err(visual_complexity(
            node_id,
            "clip nodes",
            state.nodes,
            MAX_CLIP_NODES_PER_NODE,
        ));
    }
    for child in &clip.children {
        match child {
            ClipNode::Group(group) => validate_clip_path(node_id, group, depth + 1, state)?,
            ClipNode::Shape(shape) => {
                if !affine_is_finite(shape.transform) {
                    return Err(invalid_visual(node_id, "non-finite clip shape transform"));
                }
                validate_path_geometry(node_id, &shape.path, "non-finite clip path geometry")?;
                let contour_count = shape.path.contours.len();
                if contour_count > MAX_CLIP_CONTOURS_PER_SHAPE {
                    return Err(visual_complexity(
                        node_id,
                        "contours in one clip shape",
                        contour_count,
                        MAX_CLIP_CONTOURS_PER_SHAPE,
                    ));
                }
                state.nodes = state.nodes.saturating_add(1);
                state.contours = state.contours.saturating_add(contour_count);
                state.anchors = shape
                    .path
                    .contours
                    .iter()
                    .fold(state.anchors, |count, contour| {
                        count.saturating_add(contour.anchors.len())
                    });
                if state.nodes > MAX_CLIP_NODES_PER_NODE {
                    return Err(visual_complexity(
                        node_id,
                        "clip nodes",
                        state.nodes,
                        MAX_CLIP_NODES_PER_NODE,
                    ));
                }
                if state.contours > MAX_CLIP_CONTOURS_PER_NODE {
                    return Err(visual_complexity(
                        node_id,
                        "clip contours",
                        state.contours,
                        MAX_CLIP_CONTOURS_PER_NODE,
                    ));
                }
                if state.anchors > MAX_CLIP_ANCHORS_PER_NODE {
                    return Err(visual_complexity(
                        node_id,
                        "clip anchors",
                        state.anchors,
                        MAX_CLIP_ANCHORS_PER_NODE,
                    ));
                }
            }
        }
    }
    if let Some(nested) = &clip.clip_path {
        validate_clip_path(node_id, nested, depth + 1, state)?;
    }
    Ok(())
}

fn validate_path_geometry(
    node_id: NodeId,
    path: &PathData,
    reason: &'static str,
) -> Result<(), DocumentValidationError> {
    let is_finite = path.contours.iter().all(|contour| {
        contour.anchors.iter().all(|anchor| {
            anchor.position.is_finite()
                && anchor.in_handle.is_none_or(Vec2::is_finite)
                && anchor.out_handle.is_none_or(Vec2::is_finite)
        })
    });
    if is_finite {
        Ok(())
    } else {
        Err(invalid_visual(node_id, reason))
    }
}

fn affine_is_finite(transform: Affine2) -> bool {
    transform.matrix2.is_finite() && transform.translation.is_finite()
}

fn is_normalized(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn invalid_visual(node: NodeId, reason: &'static str) -> DocumentValidationError {
    DocumentValidationError::InvalidVisualData { node, reason }
}

fn visual_complexity(
    node: NodeId,
    resource: &'static str,
    count: usize,
    limit: usize,
) -> DocumentValidationError {
    DocumentValidationError::VisualComplexity {
        node,
        resource,
        count,
        limit,
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

fn next_stable_id(ids: impl Iterator<Item = u64>) -> u64 {
    ids.max().unwrap_or(0).checked_add(1).unwrap_or(0)
}

fn take_next_id(next: &mut u64) -> Option<u64> {
    let current = (*next != 0).then_some(*next)?;
    *next = current.checked_add(1).unwrap_or(0);
    Some(current)
}

fn clip_path_contains_point(clip: &ClipPath, point: Vec2) -> bool {
    let Some(clip_inverse) = finite_affine_inverse(clip.transform) else {
        return false;
    };
    let local_point = clip_inverse.transform_point2(point);
    let inside_children = clip.children.iter().any(|child| match child {
        ClipNode::Group(group) => clip_path_contains_point(group, local_point),
        ClipNode::Shape(shape) => finite_affine_inverse(shape.transform).is_some_and(|inverse| {
            shape
                .path
                .contains_point_with_rule(inverse.transform_point2(local_point), shape.fill_rule)
        }),
    });
    inside_children
        && clip
            .clip_path
            .as_ref()
            .is_none_or(|nested| clip_path_contains_point(nested, point))
}

fn finite_affine_inverse(transform: Affine2) -> Option<Affine2> {
    if !affine_is_finite(transform) {
        return None;
    }
    let determinant = transform.matrix2.determinant();
    if !determinant.is_finite() || determinant == 0.0 {
        return None;
    }
    let inverse = transform.inverse();
    affine_is_finite(inverse).then_some(inverse)
}

/// Transform a rectangle by an affine transform (returns axis-aligned bounding box).
fn transform_rect(rect: Rect, transform: Affine2) -> Rect {
    let corners = [
        rect.min,
        Vec2::new(rect.max.x, rect.min.y),
        rect.max,
        Vec2::new(rect.min.x, rect.max.y),
    ];

    let transformed: Vec<Vec2> = corners
        .iter()
        .map(|&p| transform.transform_point2(p))
        .collect();

    let min = transformed
        .iter()
        .fold(Vec2::splat(f32::INFINITY), |acc, &p| acc.min(p));
    let max = transformed
        .iter()
        .fold(Vec2::splat(f32::NEG_INFINITY), |acc, &p| acc.max(p));

    Rect::new(min, max)
}

/// Iterator for descendants in DFS order.
struct DescendantIterator<'a> {
    doc: &'a Document,
    stack: Vec<NodeId>,
}

impl<'a> DescendantIterator<'a> {
    fn new(doc: &'a Document, root: NodeId) -> Self {
        let stack = doc
            .nodes
            .get(root)
            .map(|n| n.children.iter().rev().cloned().collect())
            .unwrap_or_default();
        Self { doc, stack }
    }
}

impl<'a> Iterator for DescendantIterator<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let id = self.stack.pop()?;

            // Skip deleted nodes
            let node = self.doc.nodes.get(id)?;
            if node.deleted {
                continue;
            }

            // Push children in reverse order so they come out in order
            self.stack.extend(node.children.iter().rev().cloned());

            return Some(id);
        }
    }
}

/// Iterator for nodes in paint order (DFS, pre-order).
struct PaintOrderIterator<'a> {
    doc: &'a Document,
    stack: Vec<NodeId>,
}

impl<'a> PaintOrderIterator<'a> {
    fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            stack: vec![doc.root],
        }
    }
}

impl<'a> Iterator for PaintOrderIterator<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let id = self.stack.pop()?;

            // Skip deleted nodes
            let node = self.doc.nodes.get(id)?;
            if node.deleted {
                continue;
            }

            // Push children in reverse order so they come out in order
            self.stack.extend(node.children.iter().rev().cloned());

            return Some(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_affine_approx_eq(actual: Affine2, expected: Affine2) {
        for (actual, expected) in actual
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!(
                (actual - expected).abs() < 0.001,
                "expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn test_new_document() {
        let doc = Document::new();
        assert!(doc.nodes.contains_key(doc.root));

        let root = doc.get(doc.root).unwrap();
        assert!(root.is_group());
        assert_eq!(root.name, "Root");
    }

    #[test]
    fn test_add_child() {
        let mut doc = Document::new();

        let rect = Node::shape("Rectangle", PathData::rect(0.0, 0.0, 100.0, 100.0));
        let rect_id = doc.add_child(doc.root, rect).unwrap();

        assert!(doc.nodes.contains_key(rect_id));

        let rect_node = doc.get(rect_id).unwrap();
        assert_eq!(rect_node.parent, Some(doc.root));

        let root = doc.get(doc.root).unwrap();
        assert!(root.children.contains(&rect_id));
    }

    #[test]
    fn test_remove_node() {
        let mut doc = Document::new();

        let group_id = doc.add_child(doc.root, Node::group("Group")).unwrap();
        let rect_id = doc
            .add_child(
                group_id,
                Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();

        assert!(doc.nodes.contains_key(rect_id));
        assert!(!doc.nodes.get(rect_id).unwrap().deleted);

        // Remove the group (should also remove the rect via soft delete)
        assert!(doc.remove(group_id));

        // Nodes are soft-deleted (still in slotmap but marked as deleted)
        assert!(doc.nodes.contains_key(group_id));
        assert!(doc.nodes.get(group_id).unwrap().deleted);
        assert!(doc.nodes.contains_key(rect_id));
        assert!(doc.nodes.get(rect_id).unwrap().deleted);

        // Soft-deleted nodes should not appear in iteration
        assert!(!doc.descendants(doc.root).any(|id| id == group_id));
        assert!(!doc.descendants(doc.root).any(|id| id == rect_id));
    }

    #[test]
    fn test_cannot_remove_root() {
        let mut doc = Document::new();
        assert!(!doc.remove(doc.root));
    }

    #[test]
    fn test_paint_order() {
        let mut doc = Document::new();

        let a = doc.add_child(doc.root, Node::group("A")).unwrap();
        let b = doc.add_child(doc.root, Node::group("B")).unwrap();
        let a1 = doc.add_child(a, Node::group("A1")).unwrap();
        let a2 = doc.add_child(a, Node::group("A2")).unwrap();

        let order: Vec<_> = doc.paint_order().collect();

        // Should be: root, A, A1, A2, B (DFS pre-order)
        assert_eq!(order, vec![doc.root, a, a1, a2, b]);
    }

    #[test]
    fn test_world_transform() {
        let mut doc = Document::new();

        // Create a group translated by (100, 0)
        let group =
            Node::group("Group").with_transform(Affine2::from_translation(Vec2::new(100.0, 0.0)));
        let group_id = doc.add_child(doc.root, group).unwrap();

        // Create a shape translated by (50, 0) within the group
        let shape = Node::shape("Shape", PathData::rect(0.0, 0.0, 10.0, 10.0))
            .with_transform(Affine2::from_translation(Vec2::new(50.0, 0.0)));
        let shape_id = doc.add_child(group_id, shape).unwrap();

        // World transform of shape should be translation by (150, 0)
        let world = doc.world_transform(shape_id);
        let origin = world.transform_point2(Vec2::ZERO);

        assert!((origin.x - 150.0).abs() < 0.001);
        assert!((origin.y - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_world_bounds() {
        let mut doc = Document::new();

        let shape = Node::shape("Shape", PathData::rect(0.0, 0.0, 100.0, 50.0))
            .with_transform(Affine2::from_translation(Vec2::new(10.0, 20.0)));
        let shape_id = doc.add_child(doc.root, shape).unwrap();

        let bounds = doc.world_bounds(shape_id).unwrap();

        assert!((bounds.min.x - 10.0).abs() < 0.001);
        assert!((bounds.min.y - 20.0).abs() < 0.001);
        assert!((bounds.max.x - 110.0).abs() < 0.001);
        assert!((bounds.max.y - 70.0).abs() < 0.001);
    }

    #[test]
    fn test_hit_test() {
        let mut doc = Document::new();

        let shape = Node::shape("Shape", PathData::rect(0.0, 0.0, 100.0, 100.0))
            .with_transform(Affine2::from_translation(Vec2::new(50.0, 50.0)));
        let shape_id = doc.add_child(doc.root, shape).unwrap();

        // Point inside shape
        assert_eq!(doc.hit_test(Vec2::new(75.0, 75.0)), Some(shape_id));

        // Point outside shape
        assert_eq!(doc.hit_test(Vec2::new(0.0, 0.0)), None);
    }

    #[test]
    fn hit_testing_respects_node_and_ancestor_clips() {
        let mut doc = Document::new();
        let clip = ClipPath {
            transform: Affine2::IDENTITY,
            children: vec![ClipNode::Shape(ClipShape {
                path: PathData::rect(0.0, 0.0, 25.0, 25.0),
                transform: Affine2::IDENTITY,
                fill_rule: FillRule::NonZero,
            })],
            clip_path: None,
        };
        let group = doc
            .add_child(doc.root, Node::group("Clipped").with_clip_path(clip))
            .unwrap();
        let shape = doc
            .add_child(
                group,
                Node::shape("Shape", PathData::rect(0.0, 0.0, 100.0, 100.0)),
            )
            .unwrap();

        assert_eq!(doc.hit_test(Vec2::new(10.0, 10.0)), Some(shape));
        assert_eq!(doc.hit_test(Vec2::new(50.0, 50.0)), None);
    }

    #[test]
    fn hit_testing_evaluates_a_shared_ancestor_clip_once() {
        let mut doc = Document::new();
        let clip = ClipPath {
            transform: Affine2::IDENTITY,
            children: vec![ClipNode::Shape(ClipShape {
                path: PathData::rect(0.0, 0.0, 100.0, 100.0),
                transform: Affine2::IDENTITY,
                fill_rule: FillRule::NonZero,
            })],
            clip_path: None,
        };
        let group = doc
            .add_child(doc.root, Node::group("Clipped").with_clip_path(clip))
            .unwrap();
        let target = doc
            .add_child(
                group,
                Node::shape("Target", PathData::rect(0.0, 0.0, 100.0, 100.0)),
            )
            .unwrap();
        doc.add_child(
            group,
            Node::shape("Top miss", PathData::rect(200.0, 200.0, 20.0, 20.0)),
        )
        .unwrap();

        CLIP_CONTAINMENT_EVALUATIONS.with(|evaluations| evaluations.set(0));
        assert_eq!(
            doc.hit_test_with_tolerance(Vec2::new(50.0, 50.0), false, 0.0),
            Some(target)
        );
        CLIP_CONTAINMENT_EVALUATIONS.with(|evaluations| assert_eq!(evaluations.get(), 1));
    }

    #[test]
    fn hit_testing_accepts_finite_inverses_for_small_scale_nodes_and_clips() {
        let small_scale = Affine2::from_scale(Vec2::splat(1e-4));
        let clip = ClipPath {
            transform: small_scale,
            children: vec![ClipNode::Group(ClipPath {
                transform: Affine2::from_scale(Vec2::splat(1e8)),
                children: vec![ClipNode::Shape(ClipShape {
                    path: PathData::rect(0.0, 0.0, 100.0, 100.0),
                    transform: small_scale,
                    fill_rule: FillRule::NonZero,
                })],
                clip_path: None,
            })],
            clip_path: None,
        };
        assert!(clip_path_contains_point(&clip, Vec2::splat(50.0)));

        let mut doc = Document::new();
        let shape = doc
            .add_child(
                doc.root,
                Node::shape("Small", PathData::rect(0.0, 0.0, 100.0, 100.0))
                    .with_transform(small_scale)
                    .with_clip_path(clip),
            )
            .unwrap();

        assert_eq!(
            doc.hit_test_with_tolerance(Vec2::splat(0.005), false, 0.0),
            Some(shape)
        );
        assert_eq!(
            doc.hit_test_with_tolerance(Vec2::splat(0.015), false, 0.0),
            None
        );
    }

    #[test]
    fn test_ancestors() {
        let mut doc = Document::new();

        let a = doc.add_child(doc.root, Node::group("A")).unwrap();
        let b = doc.add_child(a, Node::group("B")).unwrap();
        let c = doc.add_child(b, Node::group("C")).unwrap();

        let ancestors: Vec<_> = doc.ancestors(c).collect();
        assert_eq!(ancestors, vec![b, a, doc.root]);
    }

    #[test]
    fn test_descendants() {
        let mut doc = Document::new();

        let a = doc.add_child(doc.root, Node::group("A")).unwrap();
        let a1 = doc.add_child(a, Node::group("A1")).unwrap();
        let a2 = doc.add_child(a, Node::group("A2")).unwrap();

        let descendants: Vec<_> = doc.descendants(a).collect();
        assert_eq!(descendants, vec![a1, a2]);
    }

    #[test]
    fn test_reparent() {
        let mut doc = Document::new();

        let a = doc.add_child(doc.root, Node::group("A")).unwrap();
        let b = doc.add_child(doc.root, Node::group("B")).unwrap();
        let child = doc.add_child(a, Node::group("Child")).unwrap();

        // Move child from A to B
        assert!(doc.reparent(child, b, 0));

        let a_node = doc.get(a).unwrap();
        let b_node = doc.get(b).unwrap();
        let child_node = doc.get(child).unwrap();

        assert!(!a_node.children.contains(&child));
        assert!(b_node.children.contains(&child));
        assert_eq!(child_node.parent, Some(b));
    }

    #[test]
    fn test_group_nodes() {
        let mut doc = Document::new();

        let a = doc
            .add_child(
                doc.root,
                Node::shape("A", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let b = doc
            .add_child(
                doc.root,
                Node::shape("B", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let c = doc
            .add_child(
                doc.root,
                Node::shape("C", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();

        // Group A and B
        let group_id = doc.group_nodes(&[a, b]).unwrap();

        // Verify group was created
        assert!(doc.nodes.contains_key(group_id));
        assert!(doc.get(group_id).unwrap().is_group());

        // Verify group is child of root
        assert!(doc.get(doc.root).unwrap().children.contains(&group_id));

        // Verify A and B are now children of the group
        let group = doc.get(group_id).unwrap();
        assert!(group.children.contains(&a));
        assert!(group.children.contains(&b));
        assert_eq!(doc.get(a).unwrap().parent, Some(group_id));
        assert_eq!(doc.get(b).unwrap().parent, Some(group_id));

        // Verify C is still child of root
        assert!(doc.get(doc.root).unwrap().children.contains(&c));
    }

    #[test]
    fn test_ungroup() {
        let mut doc = Document::new();

        // Create a group with children
        let group_id = doc.add_child(doc.root, Node::group("Group")).unwrap();
        let a = doc
            .add_child(
                group_id,
                Node::shape("A", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let b = doc
            .add_child(
                group_id,
                Node::shape("B", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();

        // Ungroup
        let children = doc.ungroup(group_id).unwrap();

        // Verify group is soft-deleted (still exists but marked as deleted)
        assert!(doc.nodes.contains_key(group_id));
        assert!(doc.nodes.get(group_id).unwrap().deleted);

        // Verify children are now direct children of root
        assert_eq!(children, vec![a, b]);
        assert!(doc.get(doc.root).unwrap().children.contains(&a));
        assert!(doc.get(doc.root).unwrap().children.contains(&b));
        assert_eq!(doc.get(a).unwrap().parent, Some(doc.root));
        assert_eq!(doc.get(b).unwrap().parent, Some(doc.root));
    }

    #[test]
    fn test_bring_to_front() {
        let mut doc = Document::new();

        let a = doc.add_child(doc.root, Node::group("A")).unwrap();
        let b = doc.add_child(doc.root, Node::group("B")).unwrap();
        let c = doc.add_child(doc.root, Node::group("C")).unwrap();

        // Initially: [A, B, C]
        assert_eq!(doc.get(doc.root).unwrap().children, vec![a, b, c]);

        // Bring A to front
        assert!(doc.bring_to_front(a));

        // Should be: [B, C, A]
        assert_eq!(doc.get(doc.root).unwrap().children, vec![b, c, a]);
    }

    #[test]
    fn test_send_to_back() {
        let mut doc = Document::new();

        let a = doc.add_child(doc.root, Node::group("A")).unwrap();
        let b = doc.add_child(doc.root, Node::group("B")).unwrap();
        let c = doc.add_child(doc.root, Node::group("C")).unwrap();

        // Initially: [A, B, C]
        assert_eq!(doc.get(doc.root).unwrap().children, vec![a, b, c]);

        // Send C to back
        assert!(doc.send_to_back(c));

        // Should be: [C, A, B]
        assert_eq!(doc.get(doc.root).unwrap().children, vec![c, a, b]);
    }

    #[test]
    fn test_duplicate() {
        let mut doc = Document::new();

        let shape = Node::shape("Original", PathData::rect(0.0, 0.0, 10.0, 10.0))
            .with_transform(Affine2::from_translation(Vec2::new(50.0, 50.0)));
        let original_id = doc.add_child(doc.root, shape).unwrap();

        // Duplicate
        let copy_id = doc.duplicate(original_id).unwrap();

        // Verify copy exists
        assert!(doc.nodes.contains_key(copy_id));
        assert_ne!(original_id, copy_id);

        // Verify copy has same transform
        let original = doc.get(original_id).unwrap();
        let copy = doc.get(copy_id).unwrap();
        assert_eq!(original.transform, copy.transform);

        // Verify copy has modified name
        assert_eq!(copy.name, "Original copy");

        // Verify copy is sibling of original
        assert_eq!(copy.parent, original.parent);
    }

    #[test]
    fn duplicate_deep_clones_each_descendant_once() {
        let mut doc = Document::new();
        let group_id = doc.add_child(doc.root, Node::group("Group")).unwrap();
        let shape_id = doc
            .add_child(
                group_id,
                Node::shape("Shape", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let nested_group_id = doc
            .add_child(group_id, Node::group("Nested group"))
            .unwrap();
        doc.add_child(
            nested_group_id,
            Node::shape("Nested shape", PathData::rect(0.0, 0.0, 5.0, 5.0)),
        )
        .unwrap();

        let copy_id = doc.duplicate(group_id).unwrap();
        let copy_children = doc.get(copy_id).unwrap().children.clone();

        assert_eq!(copy_children.len(), 2);
        assert_ne!(copy_children[0], shape_id);
        assert_ne!(copy_children[1], nested_group_id);
        assert_eq!(doc.get(copy_children[0]).unwrap().parent, Some(copy_id));
        assert_eq!(doc.get(copy_children[1]).unwrap().parent, Some(copy_id));

        let nested_copy_children = &doc.get(copy_children[1]).unwrap().children;
        assert_eq!(nested_copy_children.len(), 1);
        assert_eq!(
            doc.get(nested_copy_children[0]).unwrap().parent,
            Some(copy_children[1])
        );
    }

    #[test]
    fn test_auto_layout_horizontal() {
        let mut doc = Document::new();

        // Create an auto-layout group
        let group = Node::group("Auto Group")
            .with_auto_layout_horizontal()
            .with_layout(Layout::Auto(
                layout::AutoLayout::horizontal().with_spacing(10.0),
            ));
        let group_id = doc.add_child(doc.root, group).unwrap();

        // Add three shapes of different widths
        let shape1 = Node::shape("A", PathData::rect(0.0, 0.0, 30.0, 20.0));
        let shape2 = Node::shape("B", PathData::rect(0.0, 0.0, 50.0, 20.0));
        let shape3 = Node::shape("C", PathData::rect(0.0, 0.0, 40.0, 20.0));

        let id1 = doc.add_child(group_id, shape1).unwrap();
        let id2 = doc.add_child(group_id, shape2).unwrap();
        let id3 = doc.add_child(group_id, shape3).unwrap();

        // Get world transforms - this triggers layout computation
        let world1 = doc.world_transform(id1);
        let world2 = doc.world_transform(id2);
        let world3 = doc.world_transform(id3);

        // Check positions: 0, 30+10=40, 40+50+10=100
        let pos1 = world1.transform_point2(Vec2::ZERO);
        let pos2 = world2.transform_point2(Vec2::ZERO);
        let pos3 = world3.transform_point2(Vec2::ZERO);

        assert!((pos1.x - 0.0).abs() < 0.001, "pos1.x = {}", pos1.x);
        assert!((pos2.x - 40.0).abs() < 0.001, "pos2.x = {}", pos2.x);
        assert!((pos3.x - 100.0).abs() < 0.001, "pos3.x = {}", pos3.x);

        // All should have same Y (aligned to start)
        assert!((pos1.y - 0.0).abs() < 0.001);
        assert!((pos2.y - 0.0).abs() < 0.001);
        assert!((pos3.y - 0.0).abs() < 0.001);
    }

    #[test]
    fn direct_reorder_invalidates_warm_auto_layout_offsets() {
        let mut doc = Document::new();
        let parent = doc
            .add_child(
                doc.root,
                Node::group("Row")
                    .with_layout(Layout::Auto(AutoLayout::horizontal().with_spacing(10.0))),
            )
            .unwrap();
        let first = doc
            .add_child(
                parent,
                Node::shape("A", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let second = doc
            .add_child(
                parent,
                Node::shape("B", PathData::rect(0.0, 0.0, 20.0, 10.0)),
            )
            .unwrap();

        assert_eq!(
            doc.world_transform(second).transform_point2(Vec2::ZERO),
            Vec2::new(20.0, 0.0)
        );
        assert!(doc.send_to_back(second));

        assert_eq!(
            doc.world_transform(second).transform_point2(Vec2::ZERO),
            Vec2::ZERO
        );
        assert_eq!(
            doc.world_transform(first).transform_point2(Vec2::ZERO),
            Vec2::new(30.0, 0.0)
        );
    }

    #[test]
    fn direct_group_and_ungroup_preserve_world_transforms_in_auto_layout() {
        let mut doc = Document::new();
        let parent = doc
            .add_child(
                doc.root,
                Node::group("Row")
                    .with_layout(Layout::Auto(
                        AutoLayout::horizontal()
                            .with_spacing(11.0)
                            .with_uniform_padding(5.0)
                            .with_align_cross(AlignCross::Center),
                    ))
                    .with_transform(Affine2::from_scale_angle_translation(
                        Vec2::new(1.4, 0.7),
                        0.3,
                        Vec2::new(50.0, 30.0),
                    )),
            )
            .unwrap();
        let first = doc
            .add_child(
                parent,
                Node::shape("A", PathData::rect(0.0, 0.0, 12.0, 9.0))
                    .with_transform(Affine2::from_translation(Vec2::new(3.0, 2.0))),
            )
            .unwrap();
        doc.add_child(
            parent,
            Node::shape("B", PathData::rect(0.0, 0.0, 8.0, 14.0)),
        )
        .unwrap();
        let last = doc
            .add_child(
                parent,
                Node::shape("C", PathData::rect(0.0, 0.0, 16.0, 7.0))
                    .with_transform(Affine2::from_translation(Vec2::new(-2.0, 4.0))),
            )
            .unwrap();
        let first_world = doc.world_transform(first);
        let last_world = doc.world_transform(last);

        let group = doc.group_nodes(&[last, first]).unwrap();

        assert_affine_approx_eq(doc.world_transform(first), first_world);
        assert_affine_approx_eq(doc.world_transform(last), last_world);
        assert_eq!(doc.get(group).unwrap().children, vec![first, last]);

        doc.get_mut(group).unwrap().transform =
            Affine2::from_scale_angle_translation(Vec2::new(0.8, 1.3), -0.25, Vec2::new(6.0, -3.0));
        doc.mark_transform_dirty(group);
        let transformed_first = doc.world_transform(first);
        let transformed_last = doc.world_transform(last);

        assert_eq!(doc.ungroup(group), Some(vec![first, last]));
        assert_affine_approx_eq(doc.world_transform(first), transformed_first);
        assert_affine_approx_eq(doc.world_transform(last), transformed_last);
    }

    #[test]
    fn test_auto_layout_vertical() {
        let mut doc = Document::new();

        // Create a vertical auto-layout group with padding
        let group = Node::group("Vertical").with_layout(Layout::Auto(
            layout::AutoLayout::vertical()
                .with_spacing(5.0)
                .with_uniform_padding(10.0),
        ));
        let group_id = doc.add_child(doc.root, group).unwrap();

        // Add two shapes
        let shape1 = Node::shape("A", PathData::rect(0.0, 0.0, 100.0, 30.0));
        let shape2 = Node::shape("B", PathData::rect(0.0, 0.0, 100.0, 40.0));

        let id1 = doc.add_child(group_id, shape1).unwrap();
        let id2 = doc.add_child(group_id, shape2).unwrap();

        let world1 = doc.world_transform(id1);
        let world2 = doc.world_transform(id2);

        let pos1 = world1.transform_point2(Vec2::ZERO);
        let pos2 = world2.transform_point2(Vec2::ZERO);

        // First at (padding, padding) = (10, 10)
        assert!((pos1.x - 10.0).abs() < 0.001, "pos1.x = {}", pos1.x);
        assert!((pos1.y - 10.0).abs() < 0.001, "pos1.y = {}", pos1.y);

        // Second at (10, 10 + 30 + 5) = (10, 45)
        assert!((pos2.x - 10.0).abs() < 0.001, "pos2.x = {}", pos2.x);
        assert!((pos2.y - 45.0).abs() < 0.001, "pos2.y = {}", pos2.y);
    }

    #[test]
    fn test_auto_layout_with_author_transform() {
        let mut doc = Document::new();

        // Create an auto-layout group
        let group = Node::group("Auto").with_layout(Layout::Auto(
            layout::AutoLayout::horizontal().with_spacing(10.0),
        ));
        let group_id = doc.add_child(doc.root, group).unwrap();

        // Add a shape with its own transform (rotation)
        let shape = Node::shape("Rotated", PathData::rect(0.0, 0.0, 50.0, 30.0))
            .with_transform(Affine2::from_angle(std::f32::consts::FRAC_PI_4)); // 45 degrees

        let shape_id = doc.add_child(group_id, shape).unwrap();

        // The layout offset should position the shape at (0, 0)
        // but the author transform (rotation) should still be applied
        let world = doc.world_transform(shape_id);

        // The layout offset is (0, 0) since it's the first child
        // The world transform should include the rotation
        let rotated_point = world.transform_point2(Vec2::new(1.0, 0.0));

        // After 45 degree rotation, (1, 0) becomes approximately (0.707, 0.707)
        let expected_x = (std::f32::consts::FRAC_PI_4).cos();
        let expected_y = (std::f32::consts::FRAC_PI_4).sin();

        assert!((rotated_point.x - expected_x).abs() < 0.001);
        assert!((rotated_point.y - expected_y).abs() < 0.001);
    }

    #[test]
    fn test_auto_layout_center_cross() {
        let mut doc = Document::new();

        // Create horizontal auto-layout with center cross alignment
        let group = Node::group("Centered").with_layout(Layout::Auto(
            layout::AutoLayout::horizontal().with_align_cross(layout::AlignCross::Center),
        ));
        let group_id = doc.add_child(doc.root, group).unwrap();

        // Add shapes with different heights
        let shape1 = Node::shape("Short", PathData::rect(0.0, 0.0, 30.0, 20.0));
        let shape2 = Node::shape("Tall", PathData::rect(0.0, 0.0, 30.0, 60.0));

        let id1 = doc.add_child(group_id, shape1).unwrap();
        let id2 = doc.add_child(group_id, shape2).unwrap();

        let world1 = doc.world_transform(id1);
        let world2 = doc.world_transform(id2);

        let pos1 = world1.transform_point2(Vec2::ZERO);
        let pos2 = world2.transform_point2(Vec2::ZERO);

        // Max height is 60, short shape (20) should be centered: y = (60-20)/2 = 20
        assert!((pos1.y - 20.0).abs() < 0.001, "pos1.y = {}", pos1.y);

        // Tall shape should be at y = 0
        assert!((pos2.y - 0.0).abs() < 0.001, "pos2.y = {}", pos2.y);
    }

    #[test]
    fn test_set_layout() {
        let mut doc = Document::new();

        let group_id = doc.add_child(doc.root, Node::group("Group")).unwrap();

        // Initially free layout
        assert!(!doc.get(group_id).unwrap().layout.is_auto());

        // Change to auto-layout
        doc.set_layout(group_id, Layout::Auto(layout::AutoLayout::horizontal()));

        assert!(doc.get(group_id).unwrap().layout.is_auto());
    }

    #[test]
    fn test_layout_offset_getter() {
        let mut doc = Document::new();

        // Create auto-layout group
        let group = Node::group("Auto").with_layout(Layout::Auto(
            layout::AutoLayout::horizontal().with_spacing(20.0),
        ));
        let group_id = doc.add_child(doc.root, group).unwrap();

        // Add two shapes
        let shape1 = Node::shape("A", PathData::rect(0.0, 0.0, 50.0, 30.0));
        let shape2 = Node::shape("B", PathData::rect(0.0, 0.0, 50.0, 30.0));

        let id1 = doc.add_child(group_id, shape1).unwrap();
        let id2 = doc.add_child(group_id, shape2).unwrap();

        // Get layout offsets
        let offset1 = doc.layout_offset(id1);
        let offset2 = doc.layout_offset(id2);

        // First child at (0, 0)
        assert!((offset1.x - 0.0).abs() < 0.001);
        assert!((offset1.y - 0.0).abs() < 0.001);

        // Second child at (50 + 20 = 70, 0)
        assert!((offset2.x - 70.0).abs() < 0.001);
        assert!((offset2.y - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_serialization_round_trip() {
        use crate::style::Paint;

        // Create a document with some content
        let mut doc = Document::new();

        // Add a rectangle
        let rect = Node::shape("Rectangle", PathData::rect(0.0, 0.0, 100.0, 50.0))
            .with_style(Style::fill(Paint::rgb(0.2, 0.4, 0.8)));
        doc.add_child(doc.root, rect);

        // Add a text node
        let text = Node::text("Label", "Hello World")
            .with_transform(Affine2::from_translation(Vec2::new(10.0, 20.0)));
        doc.add_child(doc.root, text);

        // Serialize to JSON
        let json = doc.to_json().expect("Serialization should succeed");
        assert!(!json.is_empty());
        assert!(json.contains("Rectangle"));
        assert!(json.contains("Hello World"));

        // Deserialize back
        let restored = Document::from_json(&json).expect("Deserialization should succeed");

        // Verify structure
        assert_eq!(restored.nodes.len(), doc.nodes.len());
        assert!(restored.get(restored.root).is_some());

        // Verify root children
        let root_children = &restored.nodes[restored.root].children;
        assert_eq!(root_children.len(), 2);

        // Verify content
        let rect_node = &restored.nodes[root_children[0]];
        assert_eq!(rect_node.name, "Rectangle");
        assert!(rect_node.is_shape());

        let text_node = &restored.nodes[root_children[1]];
        assert_eq!(text_node.name, "Label");
        assert!(text_node.is_text());
    }

    #[test]
    fn version_four_round_trips_advanced_svg_visual_data() {
        let mut doc = Document::new();
        let gradient = Paint::LinearGradient(LinearGradient {
            start: Vec2::ZERO,
            end: Vec2::new(100.0, 0.0),
            transform: Affine2::from_translation(Vec2::new(2.0, 3.0)),
            spread: SpreadMethod::Reflect,
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: [1.0, 0.0, 0.0, 0.5],
                },
                GradientStop {
                    offset: 1.0,
                    color: [0.0, 0.0, 1.0, 1.0],
                },
            ],
        });
        let mut stroke = Stroke::new(3.0, gradient.clone());
        stroke.line_cap = LineCap::Round;
        stroke.line_join = LineJoin::MiterClip;
        stroke.miter_limit = 8.0;
        stroke.dash_array = vec![5.0, 2.0];
        stroke.dash_offset = 1.0;
        let mut style = Style::fill_and_stroke(gradient, stroke);
        style.fill_rule = FillRule::EvenOdd;
        style.paint_order = PaintOrder::StrokeAndFill;
        style.opacity = 0.75;
        let clip = ClipPath {
            transform: Affine2::IDENTITY,
            children: vec![ClipNode::Shape(ClipShape {
                path: PathData::rect(0.0, 0.0, 50.0, 50.0),
                transform: Affine2::IDENTITY,
                fill_rule: FillRule::EvenOdd,
            })],
            clip_path: None,
        };
        doc.add_child(
            doc.root,
            Node::shape("Advanced", PathData::rect(0.0, 0.0, 100.0, 50.0))
                .with_style(style.clone())
                .with_clip_path(clip.clone()),
        );

        let json = doc.to_json().unwrap();
        let restored = Document::from_json(&json).unwrap();
        let node = &restored.nodes[restored.nodes[restored.root].children[0]];

        assert_eq!(restored.to_saved().version, 4);
        assert_eq!(node.style, style);
        assert_eq!(node.clip_path, Some(clip));
    }

    #[test]
    fn invalid_advanced_visual_data_is_rejected_before_persistence() {
        let mut doc = Document::new();
        let invalid_gradient = Paint::RadialGradient(RadialGradient {
            center: Vec2::ZERO,
            focal: Vec2::ZERO,
            radius: f32::NAN,
            transform: Affine2::IDENTITY,
            spread: SpreadMethod::Pad,
            stops: Vec::new(),
        });
        let node = Node::shape("Invalid", PathData::rect(0.0, 0.0, 10.0, 10.0))
            .with_style(Style::fill(invalid_gradient));
        doc.add_child(doc.root, node);

        assert!(matches!(
            doc.to_validated_saved(),
            Err(DocumentValidationError::InvalidVisualData { .. })
        ));

        for stops in [
            Vec::new(),
            vec![GradientStop {
                offset: 0.0,
                color: [1.0, 0.0, 0.0, 1.0],
            }],
        ] {
            let mut doc = Document::new();
            let paint = Paint::LinearGradient(LinearGradient {
                start: Vec2::ZERO,
                end: Vec2::X,
                transform: Affine2::IDENTITY,
                spread: SpreadMethod::Pad,
                stops,
            });
            doc.add_child(
                doc.root,
                Node::shape("Incomplete gradient", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_style(Style::fill(paint)),
            );
            assert!(matches!(
                doc.to_validated_saved(),
                Err(DocumentValidationError::InvalidVisualData {
                    reason: "gradients require at least two stops",
                    ..
                })
            ));
        }

        let mut invalid_path = PathData::rect(0.0, 0.0, 10.0, 10.0);
        invalid_path.contours[0].anchors[0].position.x = f32::NAN;
        let mut doc = Document::new();
        doc.add_child(doc.root, Node::shape("Invalid path", invalid_path));
        assert!(matches!(
            doc.to_validated_saved(),
            Err(DocumentValidationError::InvalidVisualData {
                reason: "non-finite path geometry",
                ..
            })
        ));

        let mut invalid_clip_path = PathData::rect(0.0, 0.0, 10.0, 10.0);
        invalid_clip_path.contours[0].anchors[0].out_handle = Some(Vec2::splat(f32::INFINITY));
        let clip = ClipPath {
            transform: Affine2::IDENTITY,
            children: vec![ClipNode::Shape(ClipShape {
                path: invalid_clip_path,
                transform: Affine2::IDENTITY,
                fill_rule: FillRule::NonZero,
            })],
            clip_path: None,
        };
        let mut doc = Document::new();
        doc.add_child(
            doc.root,
            Node::shape("Invalid clip", PathData::rect(0.0, 0.0, 10.0, 10.0)).with_clip_path(clip),
        );
        assert!(matches!(
            doc.to_validated_saved(),
            Err(DocumentValidationError::InvalidVisualData {
                reason: "non-finite clip path geometry",
                ..
            })
        ));

        let mut overflowing_stroke = Stroke::black(f32::MAX);
        overflowing_stroke.miter_limit = 4.0;
        let mut doc = Document::new();
        doc.add_child(
            doc.root,
            Node::shape("Overflowing stroke", PathData::rect(0.0, 0.0, 10.0, 10.0))
                .with_style(Style::stroke(overflowing_stroke)),
        );
        let saved = doc.to_saved();
        assert!(matches!(
            invalid_saved_reason(saved),
            DocumentValidationError::InvalidVisualData {
                reason: "stroke width and miter limit produce a non-finite visual extent",
                ..
            }
        ));

        let clip = ClipPath {
            transform: Affine2::IDENTITY,
            children: vec![ClipNode::Shape(ClipShape {
                path: PathData {
                    contours: vec![PathContour::default(); MAX_CLIP_CONTOURS_PER_SHAPE + 1],
                },
                transform: Affine2::IDENTITY,
                fill_rule: FillRule::NonZero,
            })],
            clip_path: None,
        };
        let mut doc = Document::new();
        doc.add_child(
            doc.root,
            Node::shape(
                "Excessive clip contours",
                PathData::rect(0.0, 0.0, 10.0, 10.0),
            )
            .with_clip_path(clip),
        );
        assert!(matches!(
            doc.to_validated_saved(),
            Err(DocumentValidationError::VisualComplexity {
                resource: "contours in one clip shape",
                limit: MAX_CLIP_CONTOURS_PER_SHAPE,
                ..
            })
        ));
    }

    #[test]
    fn version_three_metadata_round_trips() {
        let mut editor = Editor::new();
        editor
            .set_grid_settings(GridSettings::new(12.0, 3).unwrap())
            .unwrap();
        let guide = editor.add_guide(GuideAxis::Horizontal, 42.5).unwrap();
        let group = editor.add_color_group("Brand").unwrap();
        let color = editor
            .add_saved_color(
                Some(group),
                Some("Ink"),
                RgbaColor::new(0.1, 0.2, 0.3, 0.8).unwrap(),
            )
            .unwrap();

        let json = editor.document.to_json().unwrap();
        let restored = Document::from_json(&json).unwrap();
        assert_eq!(restored.grid, GridSettings::new(12.0, 3).unwrap());
        assert_eq!(
            restored.guides,
            vec![Guide {
                id: guide,
                axis: GuideAxis::Horizontal,
                position: 42.5
            }]
        );
        assert_eq!(
            restored.color_library.color(color).unwrap().name.as_deref(),
            Some("Ink")
        );
        assert_eq!(restored.to_saved().version, SavedDocument::CURRENT_VERSION);
    }

    #[test]
    fn invalid_precision_and_color_metadata_is_rejected() {
        let doc = Document::new();
        let mut invalid_grid = doc.to_saved();
        invalid_grid.grid.spacing = f32::NAN;
        assert!(matches!(
            invalid_saved_reason(invalid_grid),
            DocumentValidationError::InvalidPrecisionData {
                reason: PrecisionError::InvalidGridSpacing
            }
        ));

        let mut duplicate_guides = doc.to_saved();
        let id = GuideId::from_raw(1);
        duplicate_guides.guides = vec![
            Guide {
                id,
                axis: GuideAxis::Horizontal,
                position: 1.0,
            },
            Guide {
                id,
                axis: GuideAxis::Vertical,
                position: 2.0,
            },
        ];
        assert!(matches!(
            invalid_saved_reason(duplicate_guides),
            DocumentValidationError::InvalidPrecisionData {
                reason: PrecisionError::DuplicateGuideId(found)
            } if found == id
        ));

        let mut invalid_library = doc.to_saved();
        invalid_library.color_library.colors.push(SavedColor {
            id: SavedColorId::from_raw(1),
            group_id: Some(ColorGroupId::from_raw(999)),
            name: None,
            rgba: RgbaColor::new(0.0, 0.0, 0.0, 1.0).unwrap(),
            manual_order: 0,
        });
        assert!(matches!(
            invalid_saved_reason(invalid_library),
            DocumentValidationError::InvalidColorLibrary {
                reason: ColorLibraryError::GroupNotFound(_)
            }
        ));
    }

    fn invalid_saved_reason(saved: SavedDocument) -> DocumentValidationError {
        match Document::from_saved(saved) {
            Err(DocumentLoadError::InvalidDocument { reason }) => reason,
            result => panic!("expected an invalid document error, got {result:?}"),
        }
    }

    #[test]
    fn saved_document_rejects_missing_and_deleted_roots() {
        let doc = Document::new();
        let mut missing = doc.to_saved();
        missing.nodes.remove(missing.root);
        assert!(matches!(
            invalid_saved_reason(missing),
            DocumentValidationError::MissingRoot { .. }
        ));

        let mut deleted = doc.to_saved();
        deleted.nodes[deleted.root].deleted = true;
        assert!(matches!(
            invalid_saved_reason(deleted),
            DocumentValidationError::DeletedRoot { .. }
        ));
    }

    #[test]
    fn saved_document_rejects_dangling_parent_and_child_ids() {
        let mut doc = Document::new();
        let child = doc
            .add_child(doc.root, Node::group("Child"))
            .expect("root exists");

        let mut dangling_parent = doc.to_saved();
        let removed_parent = dangling_parent.nodes.insert(Node::group("Removed"));
        dangling_parent.nodes.remove(removed_parent);
        dangling_parent.nodes[child].parent = Some(removed_parent);
        assert!(matches!(
            invalid_saved_reason(dangling_parent),
            DocumentValidationError::DanglingParent { node, parent }
                if node == child && parent == removed_parent
        ));

        let mut dangling_child = doc.to_saved();
        dangling_child.nodes.remove(child);
        assert!(matches!(
            invalid_saved_reason(dangling_child),
            DocumentValidationError::DanglingChild { parent, child: missing }
                if parent == doc.root && missing == child
        ));
    }

    #[test]
    fn saved_document_rejects_duplicate_children_and_parent_mismatches() {
        let mut doc = Document::new();
        let child = doc
            .add_child(doc.root, Node::group("Child"))
            .expect("root exists");
        let other_parent = doc
            .add_child(doc.root, Node::group("Other"))
            .expect("root exists");

        let mut duplicate = doc.to_saved();
        duplicate.nodes[duplicate.root].children.push(child);
        assert!(matches!(
            invalid_saved_reason(duplicate),
            DocumentValidationError::DuplicateChild {
                parent,
                child: duplicate_child
            } if parent == doc.root && duplicate_child == child
        ));

        let mut mismatch = doc.to_saved();
        mismatch.nodes[child].parent = Some(other_parent);
        assert!(matches!(
            invalid_saved_reason(mismatch),
            DocumentValidationError::ParentChildMismatch {
                parent,
                child: mismatched_child
            } if parent == doc.root && mismatched_child == child
        ));
    }

    #[test]
    fn saved_document_rejects_multiple_roots_and_unreachable_live_nodes() {
        let doc = Document::new();
        let mut multiple_roots = doc.to_saved();
        let additional_root = multiple_roots.nodes.insert(Node::group("Other root"));
        assert!(matches!(
            invalid_saved_reason(multiple_roots),
            DocumentValidationError::MultipleRoots {
                declared_root,
                additional_root: found
            } if declared_root == doc.root && found == additional_root
        ));

        let mut doc = Document::new();
        let deleted_parent = doc
            .add_child(doc.root, Node::group("Deleted parent"))
            .expect("root exists");
        let child = doc
            .add_child(deleted_parent, Node::group("Live child"))
            .expect("parent exists");
        let mut unreachable = doc.to_saved();
        unreachable.nodes[unreachable.root]
            .children
            .retain(|node| *node != deleted_parent);
        unreachable.nodes[deleted_parent].deleted = true;
        assert!(matches!(
            invalid_saved_reason(unreachable),
            DocumentValidationError::UnreachableNode { node } if node == child
        ));
    }

    #[test]
    fn saved_document_rejects_cycles() {
        let doc = Document::new();
        let mut saved = doc.to_saved();
        let first = saved.nodes.insert(Node::group("First"));
        let second = saved.nodes.insert(Node::group("Second"));
        saved.nodes[first].parent = Some(second);
        saved.nodes[first].children.push(second);
        saved.nodes[second].parent = Some(first);
        saved.nodes[second].children.push(first);

        assert!(matches!(
            invalid_saved_reason(saved),
            DocumentValidationError::Cycle { node } if node == first || node == second
        ));
    }

    #[test]
    fn malformed_saved_graph_is_rejected_when_loaded_from_json() {
        let doc = Document::new();
        let mut saved = doc.to_saved();
        saved.nodes[saved.root].children.push(saved.root);
        saved.nodes[saved.root].children.push(saved.root);
        let json = serde_json::to_string(&saved).expect("saved document should serialize");

        assert!(matches!(
            Document::from_json(&json),
            Err(DocumentLoadError::InvalidDocument {
                reason: DocumentValidationError::DuplicateChild { .. }
            })
        ));
    }

    #[test]
    fn valid_soft_deleted_undo_state_still_loads() {
        let mut doc = Document::new();
        let group = doc
            .add_child(doc.root, Node::group("Group"))
            .expect("root exists");
        doc.add_child(group, Node::group("Child"))
            .expect("group exists");
        assert!(doc.remove(group));

        assert!(Document::from_saved(doc.to_saved()).is_ok());
    }

    #[test]
    fn saved_documents_omit_soft_deleted_content() {
        let mut doc = Document::new();
        let deleted = doc
            .add_child(
                doc.root,
                Node::text("Private note", "do not persist this secret"),
            )
            .expect("root exists");
        assert!(doc.remove(deleted));
        let survivor = doc
            .add_child(doc.root, Node::group("Survivor"))
            .expect("root exists");

        let json = doc.to_json().expect("document should serialize");
        assert!(!json.contains("do not persist this secret"));
        let serialized: serde_json::Value = serde_json::from_str(&json).unwrap();
        let live_node_count = doc.nodes.values().filter(|node| !node.deleted).count();
        assert_eq!(
            serialized["nodes"].as_array().unwrap().len(),
            live_node_count + 1,
            "saved slot map should contain only its sentinel and live nodes"
        );

        let restored = Document::from_json(&json).expect("saved document should load");
        assert_eq!(restored.nodes.len(), 2);
        assert!(restored.nodes.contains_key(restored.root));
        assert!(restored
            .descendants(restored.root)
            .any(|id| restored.nodes[id].name == "Survivor"));
        assert_ne!(
            survivor,
            restored.descendants(restored.root).next().unwrap()
        );
    }

    #[test]
    fn saved_document_complexity_is_bounded_before_graph_validation() {
        let mut doc = Document::new();
        let child = doc
            .add_child(doc.root, Node::text("Label", "too long"))
            .expect("root exists");
        let shape = doc
            .add_child(
                doc.root,
                Node::shape(
                    "Empty contours",
                    PathData {
                        contours: vec![PathContour::default(), PathContour::default()],
                    },
                ),
            )
            .expect("root exists");
        let saved = doc.to_saved();
        let limits = DocumentComplexityLimits {
            nodes: saved.nodes.len(),
            children_per_node: 0,
            path_contours_per_node: usize::MAX,
            path_anchors_per_node: usize::MAX,
            text_bytes_per_node: usize::MAX,
        };
        assert!(matches!(
            Document::validate_saved_complexity(&saved, limits),
            Err(DocumentValidationError::TooManyChildren { node, .. }) if node == saved.root
        ));

        let limits = DocumentComplexityLimits {
            nodes: saved.nodes.len(),
            children_per_node: usize::MAX,
            path_contours_per_node: usize::MAX,
            path_anchors_per_node: usize::MAX,
            text_bytes_per_node: 3,
        };
        assert!(matches!(
            Document::validate_saved_complexity(&saved, limits),
            Err(DocumentValidationError::TextTooLong { node, .. }) if node == child
        ));

        let limits = DocumentComplexityLimits {
            nodes: saved.nodes.len(),
            children_per_node: usize::MAX,
            path_contours_per_node: 1,
            path_anchors_per_node: usize::MAX,
            text_bytes_per_node: usize::MAX,
        };
        assert!(matches!(
            Document::validate_saved_complexity(&saved, limits),
            Err(DocumentValidationError::TooManyPathContours { node, .. }) if node == shape
        ));

        assert!(matches!(
            Document::validate_serialized_slot_count(3, 2),
            Err(DocumentValidationError::TooManyNodeSlots { count: 3, limit: 2 })
        ));
    }

    #[test]
    fn version_one_documents_are_migrated_on_load() {
        let mut doc = Document::new();
        doc.add_child(
            doc.root,
            Node::text("Legacy label", "Hello")
                .with_transform(Affine2::from_translation(Vec2::new(10.0, 20.0))),
        );
        fn remove_sizing(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(object) => {
                    object.remove("sizing");
                    for child in object.values_mut() {
                        remove_sizing(child);
                    }
                }
                serde_json::Value::Array(array) => {
                    for child in array {
                        remove_sizing(child);
                    }
                }
                _ => {}
            }
        }
        let mut saved: serde_json::Value = serde_json::from_str(&doc.to_json().unwrap()).unwrap();
        saved["version"] = serde_json::json!(1);
        let object = saved.as_object_mut().unwrap();
        object.remove("grid");
        object.remove("guides");
        object.remove("color_library");
        remove_sizing(&mut saved);
        let json = serde_json::to_string(&saved).unwrap();

        let restored = Document::from_json(&json).unwrap();
        let text_id = restored.nodes[restored.root].children[0];
        let NodeKind::Text(text) = &restored.nodes[text_id].kind else {
            panic!("legacy text node should survive migration");
        };
        assert_eq!(text.content, "Hello");
        assert_eq!(text.sizing, TextSizing::AutoWidth);
        assert_eq!(restored.grid, GridSettings::default());
        assert!(restored.guides.is_empty());
        assert_eq!(restored.color_library, ColorLibrary::default());
    }

    #[test]
    fn version_two_documents_receive_version_three_metadata_defaults() {
        let doc = Document::new();
        let mut saved: serde_json::Value = serde_json::from_str(&doc.to_json().unwrap()).unwrap();
        saved["version"] = serde_json::json!(2);
        let object = saved.as_object_mut().unwrap();
        object.remove("grid");
        object.remove("guides");
        object.remove("color_library");

        let restored = Document::from_json(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored.grid, GridSettings::default());
        assert!(restored.guides.is_empty());
        assert_eq!(restored.color_library, ColorLibrary::default());
        assert_eq!(restored.to_saved().version, SavedDocument::CURRENT_VERSION);
    }

    #[test]
    fn version_three_documents_receive_version_four_visual_defaults() {
        fn remove_visual_fields(value: &mut serde_json::Value) {
            match value {
                serde_json::Value::Object(object) => {
                    for field in [
                        "clip_path",
                        "fill_rule",
                        "paint_order",
                        "line_cap",
                        "line_join",
                        "miter_limit",
                        "dash_array",
                        "dash_offset",
                    ] {
                        object.remove(field);
                    }
                    for child in object.values_mut() {
                        remove_visual_fields(child);
                    }
                }
                serde_json::Value::Array(array) => {
                    for child in array {
                        remove_visual_fields(child);
                    }
                }
                _ => {}
            }
        }

        let mut doc = Document::new();
        doc.add_child(
            doc.root,
            Node::shape("Legacy", PathData::rect(0.0, 0.0, 10.0, 10.0))
                .with_style(Style::stroke(Stroke::black(2.0))),
        );
        let mut saved: serde_json::Value = serde_json::from_str(&doc.to_json().unwrap()).unwrap();
        saved["version"] = serde_json::json!(3);
        remove_visual_fields(&mut saved);

        let restored = Document::from_json(&serde_json::to_string(&saved).unwrap()).unwrap();
        let node = &restored.nodes[restored.nodes[restored.root].children[0]];
        let stroke = node.style.stroke.as_ref().unwrap();
        assert_eq!(node.style.fill_rule, FillRule::NonZero);
        assert_eq!(node.style.paint_order, PaintOrder::FillAndStroke);
        assert_eq!(stroke.line_cap, LineCap::Butt);
        assert_eq!(stroke.line_join, LineJoin::Miter);
        assert_eq!(stroke.miter_limit, 4.0);
        assert!(stroke.dash_array.is_empty());
        assert_eq!(stroke.dash_offset, 0.0);
        assert!(node.clip_path.is_none());
        assert_eq!(restored.to_saved().version, 4);
    }

    #[test]
    fn future_document_versions_are_rejected() {
        let future = SavedDocument::CURRENT_VERSION + 1;
        let json = format!(r#"{{"version": {future}, "unknown_future_data": true}}"#);

        let error = Document::from_json(&json).unwrap_err();
        assert!(matches!(
            error,
            DocumentLoadError::UnsupportedVersion { version, current }
                if version == future && current == SavedDocument::CURRENT_VERSION
        ));
    }

    #[test]
    fn test_filter_selection_for_transform() {
        let mut doc = Document::new();

        // Create: root -> group -> [shape1, shape2]
        let group = Node::group("Group");
        let group_id = doc.add_child(doc.root, group).unwrap();

        let shape1 = Node::shape("Shape1", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let shape1_id = doc.add_child(group_id, shape1).unwrap();

        let shape2 = Node::shape("Shape2", PathData::rect(20.0, 0.0, 10.0, 10.0));
        let shape2_id = doc.add_child(group_id, shape2).unwrap();

        // Case 1: Select just the group - should return group only
        let filtered = doc.filter_selection_for_transform(&[group_id]);
        assert_eq!(filtered, vec![group_id]);

        // Case 2: Select group and its children - should return group only
        // (children are excluded because their parent is selected)
        let filtered = doc.filter_selection_for_transform(&[group_id, shape1_id, shape2_id]);
        assert_eq!(filtered, vec![group_id]);

        // Case 3: Select just children - should return both children
        let filtered = doc.filter_selection_for_transform(&[shape1_id, shape2_id]);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains(&shape1_id));
        assert!(filtered.contains(&shape2_id));

        // Case 4: Select one child and group - should return group only
        let filtered = doc.filter_selection_for_transform(&[group_id, shape1_id]);
        assert_eq!(filtered, vec![group_id]);
    }

    #[test]
    fn child_geometry_change_invalidates_cached_group_bounds() {
        let mut doc = Document::new();
        let group_id = doc.add_child(doc.root, Node::group("Group")).unwrap();
        let shape_id = doc
            .add_child(
                group_id,
                Node::shape("Shape", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        assert_eq!(doc.world_bounds(group_id).unwrap().max, Vec2::splat(10.0));

        Patch::SetPath {
            id: shape_id,
            before: PathData::rect(0.0, 0.0, 10.0, 10.0),
            after: PathData::rect(0.0, 0.0, 40.0, 20.0),
        }
        .apply_forward(&mut doc);

        assert_eq!(
            doc.world_bounds(group_id).unwrap().max,
            Vec2::new(40.0, 20.0)
        );
    }

    #[test]
    fn world_delta_under_singular_parent_does_not_corrupt_child_transform() {
        let mut doc = Document::new();
        let group =
            Node::group("Collapsed").with_transform(Affine2::from_scale(Vec2::new(0.0, 1.0)));
        let group_id = doc.add_child(doc.root, group).unwrap();
        let child = Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 10.0))
            .with_transform(Affine2::from_translation(Vec2::new(5.0, 7.0)));
        let child_id = doc.add_child(group_id, child).unwrap();
        let before = doc.get(child_id).unwrap().transform;

        let after = doc.transform_with_world_delta(
            child_id,
            before,
            Affine2::from_translation(Vec2::new(10.0, 0.0)),
        );

        assert_eq!(after, before);
        assert!(after.matrix2.is_finite());
        assert!(after.translation.is_finite());
    }

    #[test]
    fn ellipse_hit_test_rejects_aabb_corner() {
        let mut doc = Document::new();
        let ellipse = Node::shape("Ellipse", PathData::ellipse(0.0, 0.0, 100.0, 100.0))
            .with_style(Style::fill(Paint::black()));
        let ellipse_id = doc.add_child(doc.root, ellipse).unwrap();

        assert_eq!(doc.hit_test(Vec2::splat(50.0)), Some(ellipse_id));
        assert_eq!(doc.hit_test(Vec2::splat(1.0)), None);
    }
}
