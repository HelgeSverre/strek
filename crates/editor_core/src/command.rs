use glam::Affine2;

use crate::node::{FrameData, Node, NodeId, TextData};
use crate::path::PathData;
use crate::style::Style;
use crate::Document;

/// An atomic change to the document that can be applied forward or backward.
#[derive(Debug, Clone)]
pub enum Patch {
    /// Change a node's transform
    SetTransform {
        id: NodeId,
        before: Affine2,
        after: Affine2,
    },

    /// Change a node's style
    SetStyle {
        id: NodeId,
        before: Style,
        after: Style,
    },

    /// Change a shape's path data
    SetPath {
        id: NodeId,
        before: PathData,
        after: PathData,
    },

    /// Change a text node's semantic content and layout properties
    SetText {
        id: NodeId,
        before: TextData,
        after: TextData,
    },

    /// Change a frame's explicit dimensions or background
    SetFrame {
        id: NodeId,
        before: FrameData,
        after: FrameData,
    },

    /// Change a node's name
    SetName {
        id: NodeId,
        before: String,
        after: String,
    },

    /// Change a node's visibility
    SetVisible {
        id: NodeId,
        before: bool,
        after: bool,
    },

    /// Change a node's locked state
    SetLocked {
        id: NodeId,
        before: bool,
        after: bool,
    },

    /// Move a node to a different parent
    Reparent {
        id: NodeId,
        before_parent: NodeId,
        after_parent: NodeId,
        before_index: usize,
        after_index: usize,
    },

    /// Create a new node
    CreateNode {
        id: NodeId,
        node: Box<Node>,
        parent: NodeId,
        index: usize,
    },

    /// Delete a node (stores node for undo)
    DeleteNode {
        id: NodeId,
        node: Box<Node>,
        parent: NodeId,
        index: usize,
    },

    /// Reorder children of a node
    ReorderChildren {
        parent: NodeId,
        before: Vec<NodeId>,
        after: Vec<NodeId>,
    },
}

impl Patch {
    /// Apply the patch in the forward direction.
    pub fn apply_forward(&self, doc: &mut Document) {
        match self {
            Patch::SetTransform { id, after, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.transform = *after;
                }
                doc.mark_transform_dirty(*id);
            }

            Patch::SetStyle { id, after, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.style = after.clone();
                }
            }

            Patch::SetPath { id, after, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    if let Some(path) = node.path_mut() {
                        *path = after.clone();
                    }
                }
                doc.mark_bounds_dirty(*id);
                doc.mark_layout_dirty();
            }

            Patch::SetText { id, after, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    if let crate::NodeKind::Text(text) = &mut node.kind {
                        *text = after.clone();
                    }
                }
                doc.mark_bounds_dirty(*id);
                doc.mark_layout_dirty();
            }

            Patch::SetFrame { id, after, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    if let crate::NodeKind::Frame(frame) = &mut node.kind {
                        *frame = after.clone();
                    }
                }
                doc.mark_bounds_dirty(*id);
                doc.mark_layout_dirty();
            }

            Patch::SetName { id, after, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.name = after.clone();
                }
            }

            Patch::SetVisible { id, after, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.visible = *after;
                }
                doc.mark_bounds_dirty(*id);
            }

            Patch::SetLocked { id, after, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.locked = *after;
                }
            }

            Patch::Reparent {
                id,
                before_parent,
                after_parent,
                after_index,
                ..
            } => {
                // Remove from old parent
                if let Some(parent) = doc.nodes.get_mut(*before_parent) {
                    parent.children.retain(|&c| c != *id);
                }

                // Add to new parent
                if let Some(parent) = doc.nodes.get_mut(*after_parent) {
                    let idx = (*after_index).min(parent.children.len());
                    parent.children.insert(idx, *id);
                }

                // Update node's parent reference
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.parent = Some(*after_parent);
                }

                doc.mark_transform_dirty(*id);
                doc.mark_bounds_dirty(*before_parent);
                doc.mark_bounds_dirty(*after_parent);
                doc.mark_layout_dirty();
            }

            Patch::CreateNode {
                id,
                node,
                parent,
                index,
            } => {
                // Check if node already exists (redo case - was soft-deleted)
                if let Some(existing) = doc.nodes.get_mut(*id) {
                    // Restore soft-deleted node
                    existing.deleted = false;
                    existing.parent = Some(*parent);
                } else {
                    // Initial creation - insert new node
                    let actual_id = doc.nodes.insert((**node).clone());
                    debug_assert_eq!(actual_id, *id, "Node ID mismatch during create");

                    // Set parent reference
                    if let Some(n) = doc.nodes.get_mut(*id) {
                        n.parent = Some(*parent);
                    }
                }

                // Add to parent's children
                if let Some(parent_node) = doc.nodes.get_mut(*parent) {
                    let idx = (*index).min(parent_node.children.len());
                    parent_node.children.insert(idx, *id);
                }

                doc.mark_transform_dirty(*id);
                doc.mark_layout_dirty();
            }

            Patch::DeleteNode { id, parent, .. } => {
                // Remove from parent's children list
                if let Some(parent_node) = doc.nodes.get_mut(*parent) {
                    parent_node.children.retain(|&c| c != *id);
                }

                // Soft-delete: mark as deleted but keep in slotmap to preserve ID
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.deleted = true;
                }
                doc.cache.invalidate(*id);
                doc.mark_bounds_dirty(*parent);
                doc.mark_layout_dirty();
            }

            Patch::ReorderChildren { parent, after, .. } => {
                if let Some(parent_node) = doc.nodes.get_mut(*parent) {
                    parent_node.children = after.clone();
                }
                doc.mark_layout_dirty();
            }
        }
    }

    /// Apply the patch in the backward direction (undo).
    pub fn apply_backward(&self, doc: &mut Document) {
        match self {
            Patch::SetTransform { id, before, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.transform = *before;
                }
                doc.mark_transform_dirty(*id);
            }

            Patch::SetStyle { id, before, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.style = before.clone();
                }
            }

            Patch::SetPath { id, before, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    if let Some(path) = node.path_mut() {
                        *path = before.clone();
                    }
                }
                doc.mark_bounds_dirty(*id);
                doc.mark_layout_dirty();
            }

            Patch::SetText { id, before, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    if let crate::NodeKind::Text(text) = &mut node.kind {
                        *text = before.clone();
                    }
                }
                doc.mark_bounds_dirty(*id);
                doc.mark_layout_dirty();
            }

            Patch::SetFrame { id, before, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    if let crate::NodeKind::Frame(frame) = &mut node.kind {
                        *frame = before.clone();
                    }
                }
                doc.mark_bounds_dirty(*id);
                doc.mark_layout_dirty();
            }

            Patch::SetName { id, before, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.name = before.clone();
                }
            }

            Patch::SetVisible { id, before, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.visible = *before;
                }
                doc.mark_bounds_dirty(*id);
            }

            Patch::SetLocked { id, before, .. } => {
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.locked = *before;
                }
            }

            Patch::Reparent {
                id,
                before_parent,
                after_parent,
                before_index,
                ..
            } => {
                // Remove from current parent
                if let Some(parent) = doc.nodes.get_mut(*after_parent) {
                    parent.children.retain(|&c| c != *id);
                }

                // Add back to original parent
                if let Some(parent) = doc.nodes.get_mut(*before_parent) {
                    let idx = (*before_index).min(parent.children.len());
                    parent.children.insert(idx, *id);
                }

                // Update node's parent reference
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.parent = Some(*before_parent);
                }

                doc.mark_transform_dirty(*id);
                doc.mark_bounds_dirty(*before_parent);
                doc.mark_bounds_dirty(*after_parent);
                doc.mark_layout_dirty();
            }

            Patch::CreateNode { id, parent, .. } => {
                // Undo create = soft-delete
                if let Some(parent_node) = doc.nodes.get_mut(*parent) {
                    parent_node.children.retain(|&c| c != *id);
                }
                if let Some(node) = doc.nodes.get_mut(*id) {
                    node.deleted = true;
                }
                doc.cache.invalidate(*id);
                doc.mark_bounds_dirty(*parent);
                doc.mark_layout_dirty();
            }

            Patch::DeleteNode {
                id,
                node: _,
                parent,
                index,
            } => {
                // Undo delete = restore soft-deleted node
                if let Some(n) = doc.nodes.get_mut(*id) {
                    n.deleted = false;
                    n.parent = Some(*parent);
                }

                // Add back to parent's children at original index
                if let Some(parent_node) = doc.nodes.get_mut(*parent) {
                    let idx = (*index).min(parent_node.children.len());
                    parent_node.children.insert(idx, *id);
                }

                doc.mark_transform_dirty(*id);
                doc.mark_layout_dirty();
            }

            Patch::ReorderChildren { parent, before, .. } => {
                if let Some(parent_node) = doc.nodes.get_mut(*parent) {
                    parent_node.children = before.clone();
                }
                doc.mark_layout_dirty();
            }
        }
    }
}

/// A command is a named collection of patches that form a logical operation.
#[derive(Debug, Clone)]
pub struct Command {
    /// Human-readable description (for undo menu)
    pub description: String,

    /// Patches to apply (in order)
    pub patches: Vec<Patch>,
}

impl Command {
    /// Create a new command with the given description.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            patches: Vec::new(),
        }
    }

    /// Add a patch to the command.
    pub fn with_patch(mut self, patch: Patch) -> Self {
        self.patches.push(patch);
        self
    }

    /// Add multiple patches to the command.
    pub fn with_patches(mut self, patches: impl IntoIterator<Item = Patch>) -> Self {
        self.patches.extend(patches);
        self
    }

    /// Apply the command to the document.
    pub fn apply(&self, doc: &mut Document) {
        for patch in &self.patches {
            patch.apply_forward(doc);
        }
    }

    /// Unapply the command (undo).
    pub fn unapply(&self, doc: &mut Document) {
        // Apply patches in reverse order
        for patch in self.patches.iter().rev() {
            patch.apply_backward(doc);
        }
    }

    /// Check if the command is empty (no patches).
    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::PathData;
    use glam::Vec2;

    #[test]
    fn test_set_transform_patch() {
        let mut doc = Document::new();
        let shape = Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let id = doc.add_child(doc.root, shape).unwrap();

        let before = doc.get(id).unwrap().transform;
        let after = Affine2::from_translation(Vec2::new(100.0, 50.0));

        let patch = Patch::SetTransform { id, before, after };

        patch.apply_forward(&mut doc);
        assert_eq!(doc.get(id).unwrap().transform, after);

        patch.apply_backward(&mut doc);
        assert_eq!(doc.get(id).unwrap().transform, before);
    }

    #[test]
    fn test_command_apply_unapply() {
        let mut doc = Document::new();
        let shape = Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let id = doc.add_child(doc.root, shape).unwrap();

        let original_name = doc.get(id).unwrap().name.clone();

        let cmd = Command::new("Rename").with_patch(Patch::SetName {
            id,
            before: original_name.clone(),
            after: "New Name".into(),
        });

        cmd.apply(&mut doc);
        assert_eq!(doc.get(id).unwrap().name, "New Name");

        cmd.unapply(&mut doc);
        assert_eq!(doc.get(id).unwrap().name, original_name);
    }

    #[test]
    fn deleting_a_child_invalidates_cached_ancestor_bounds() {
        let mut doc = Document::new();
        let root = doc.root;
        let group = doc.add_child(root, Node::group("Group")).unwrap();
        doc.add_child(
            group,
            Node::shape("A", PathData::rect(0.0, 0.0, 10.0, 10.0)),
        );
        let far = doc
            .add_child(
                group,
                Node::shape("B", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::new(100.0, 0.0))),
            )
            .unwrap();
        assert_eq!(doc.world_bounds(group).unwrap().max.x, 110.0);
        let node = doc.get(far).unwrap().clone();
        let command = Command::new("Delete").with_patch(Patch::DeleteNode {
            id: far,
            node: Box::new(node),
            parent: group,
            index: 1,
        });

        command.apply(&mut doc);
        assert_eq!(doc.world_bounds(group).unwrap().max.x, 10.0);
        command.unapply(&mut doc);
        assert_eq!(doc.world_bounds(group).unwrap().max.x, 110.0);
    }

    #[test]
    fn geometry_patch_recomputes_warm_auto_layout_offsets() {
        let mut doc = Document::new();
        let group = Node::group("Auto").with_layout(crate::Layout::Auto(
            crate::AutoLayout::horizontal().with_spacing(10.0),
        ));
        let group = doc.add_child(doc.root, group).unwrap();
        let first = doc
            .add_child(
                group,
                Node::shape("First", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let second = doc
            .add_child(
                group,
                Node::shape("Second", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        assert_eq!(doc.world_transform(second).translation.x, 20.0);

        Patch::SetPath {
            id: first,
            before: PathData::rect(0.0, 0.0, 10.0, 10.0),
            after: PathData::rect(0.0, 0.0, 30.0, 10.0),
        }
        .apply_forward(&mut doc);

        assert_eq!(doc.world_transform(second).translation.x, 40.0);
    }
}
