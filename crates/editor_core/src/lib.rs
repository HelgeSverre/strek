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
pub mod command;
pub mod editor;
pub mod history;
pub mod input;
pub mod layers;
pub mod layout;
pub mod node;
pub mod path;
pub mod render;
pub mod selection;
pub mod snap;
pub mod style;
pub mod text;
pub mod transaction;
pub mod transform;

pub use action::{ActionCategory, ActionMeta, EditorAction, Shortcut};
pub use layers::{LayerEntry, LayerIcon, LayerPanelState};
pub use cache::{Cache, DirtyFlags};
pub use command::{Command, Patch};
pub use editor::{Editor, Tool};
pub use history::History;
pub use input::{Cursor, Effects, InputEvent, Key, Modifiers, MouseButton};
pub use layout::{AlignCross, AlignMain, AutoLayout, Direction, Edges, LayoutOffset};
pub use node::{FontSpec, FrameData, Layout, Node, NodeId, NodeKind, TextAlign, TextData};
pub use path::{PathCmd, PathData, Rect};
pub use selection::{Selection, SelectionAction};
pub use snap::{AlignAxis, DistributeAxis, SnapConfig, SnapEngine, SnapKind, SnapPoint, SnapResult};
pub use style::{Paint, Stroke, Style};
pub use text::{FontId, GlyphRun, PositionedGlyph, SimpleTextEngine, TextEngine, TextMetrics};
pub use transaction::Transaction;
pub use transform::View;

use glam::{Affine2, Vec2};
use serde::{Deserialize, Serialize};
use slotmap::SlotMap;

/// Serializable document format (excludes derived cache data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedDocument {
    /// File format version for future migration
    pub version: u32,

    /// Primary node storage
    pub nodes: SlotMap<NodeId, Node>,

    /// Root node ID
    pub root: NodeId,
}

impl SavedDocument {
    /// Current file format version
    pub const CURRENT_VERSION: u32 = 1;
}

/// The main document structure containing the scene graph.
#[derive(Debug, Clone)]
pub struct Document {
    /// Primary node storage (generational arena)
    pub nodes: SlotMap<NodeId, Node>,

    /// Root node ID (always a Group)
    pub root: NodeId,

    /// Cached derived data
    pub cache: Cache,

    /// Dirty flags for incremental updates
    pub dirty: DirtyFlags,
}

impl Document {
    /// Create a new document with an empty root group.
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(Node::group("Root"));

        Self {
            nodes,
            root,
            cache: Cache::new(),
            dirty: DirtyFlags::all_dirty(),
        }
    }

    /// Get a reference to a node by ID.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Get a mutable reference to a node by ID.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
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
        self.dirty.transforms = true;
        self.dirty.bounds = true;

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

        self.dirty.transforms = true;
        self.dirty.bounds = true;

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

        self.dirty.transforms = true;
        self.dirty.bounds = true;

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

    /// Iterate over all descendants of a node (excluding the node itself).
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        DescendantIterator::new(self, id)
    }

    /// Check if any ancestor of `id` is in the given set.
    pub fn has_selected_ancestor(&self, id: NodeId, selected: &std::collections::HashSet<NodeId>) -> bool {
        self.ancestors(id).any(|ancestor| selected.contains(&ancestor))
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

        // Invalidate cache for this node and descendants
        self.cache.invalidate(id);
        for descendant in self.descendants(id).collect::<Vec<_>>() {
            self.cache.invalidate(descendant);
        }
    }

    /// Mark layout as dirty (triggers re-layout computation).
    pub fn mark_layout_dirty(&mut self) {
        self.dirty.layout = true;
        self.dirty.transforms = true;
        self.dirty.bounds = true;
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
                // Estimate text bounds
                let width = text.content.len() as f32 * text.font_size * 0.6;
                let height = text.font_size;
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
            NodeKind::Text(text) => {
                // Placeholder: estimate text bounds
                let width = text.content.len() as f32 * text.font_size * 0.6;
                let height = text.font_size;
                Some(Rect::from_pos_size(Vec2::ZERO, Vec2::new(width, height)))
            }
        }
    }

    /// Get the world bounds for a node (axis-aligned in world space).
    pub fn world_bounds(&mut self, id: NodeId) -> Option<Rect> {
        self.ensure_transforms_valid();

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
                let width = text.content.len() as f32 * text.font_size * 0.6;
                let height = text.font_size;
                let local_bounds = Rect::from_pos_size(Vec2::ZERO, Vec2::new(width, height));
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

    /// Get all snap points from nodes except the given excluded nodes.
    pub fn all_snap_points(&mut self, exclude: &[NodeId]) -> Vec<snap::SnapPoint> {
        let mut points = Vec::new();

        // Collect all visible, non-group nodes
        let ids: Vec<NodeId> = self
            .descendants(self.root)
            .filter(|&id| {
                if exclude.contains(&id) {
                    return false;
                }
                if let Some(node) = self.nodes.get(id) {
                    node.visible && !node.is_group()
                } else {
                    false
                }
            })
            .collect();

        for id in ids {
            points.extend(self.snap_points(id));
        }

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
        // Check nodes in reverse paint order (front to back)
        for id in self.reverse_paint_order().collect::<Vec<_>>() {
            let node = self.nodes.get(id)?;

            // Skip deleted or invisible nodes
            if node.deleted || !node.visible {
                continue;
            }

            // Skip groups unless explicitly including them
            if !include_groups && node.is_group() {
                continue;
            }

            // Check bounds
            if let Some(bounds) = self.world_bounds(id) {
                if bounds.contains(world_pos) {
                    return Some(id);
                }
            }
        }

        None
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

        // Verify all nodes exist and get their common parent
        let first_parent = self.nodes.get(nodes[0])?.parent?;

        // All nodes must have the same parent for simple grouping
        for &id in nodes {
            let node = self.nodes.get(id)?;
            if node.parent != Some(first_parent) {
                return None;
            }
        }

        // Find the minimum index among the nodes (for group placement)
        let parent_children = &self.nodes.get(first_parent)?.children;
        let min_index = nodes
            .iter()
            .filter_map(|&id| parent_children.iter().position(|&c| c == id))
            .min()?;

        // Create the new group
        let group = Node::group("Group");
        let group_id = self.nodes.insert(group);

        // Set the group's parent
        if let Some(g) = self.nodes.get_mut(group_id) {
            g.parent = Some(first_parent);
        }

        // Remove nodes from parent and add to group
        if let Some(parent) = self.nodes.get_mut(first_parent) {
            parent.children.retain(|c| !nodes.contains(c));
            parent.children.insert(min_index, group_id);
        }

        // Add nodes to the group and update their parent
        for &id in nodes {
            if let Some(node) = self.nodes.get_mut(id) {
                node.parent = Some(group_id);
            }
        }
        if let Some(g) = self.nodes.get_mut(group_id) {
            g.children = nodes.to_vec();
        }

        self.dirty.transforms = true;
        self.dirty.bounds = true;

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
        if !node.is_group() {
            return None;
        }

        let parent_id = node.parent?;
        let children = node.children.clone();

        if children.is_empty() {
            // Just remove empty group
            self.remove(group_id);
            return Some(vec![]);
        }

        // Find group's index in parent
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

        self.dirty.transforms = true;
        self.dirty.bounds = true;

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
            if let Some(new_child) = self.deep_clone(child_id, new_id, i) {
                if let Some(new_node) = self.nodes.get_mut(new_id) {
                    new_node.children.push(new_child);
                }
            }
        }

        self.dirty.transforms = true;
        self.dirty.bounds = true;

        Some(new_id)
    }

    // === Serialization ===

    /// Convert document to a serializable format.
    pub fn to_saved(&self) -> SavedDocument {
        SavedDocument {
            version: SavedDocument::CURRENT_VERSION,
            nodes: self.nodes.clone(),
            root: self.root,
        }
    }

    /// Create a document from a saved format.
    pub fn from_saved(saved: SavedDocument) -> Self {
        Self {
            nodes: saved.nodes,
            root: saved.root,
            cache: Cache::new(),
            dirty: DirtyFlags::all_dirty(),
        }
    }

    /// Serialize document to JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.to_saved())
    }

    /// Deserialize document from JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let saved: SavedDocument = serde_json::from_str(json)?;
        Ok(Self::from_saved(saved))
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
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
}
