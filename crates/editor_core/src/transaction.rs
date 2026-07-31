use std::collections::HashMap;

use glam::Affine2;

use crate::command::{Command, Patch};
use crate::node::NodeId;
use crate::Document;

/// A transaction represents an in-progress interactive edit.
///
/// During a drag operation, for example:
/// 1. Start a transaction (captures original state)
/// 2. Update during drag (modifies document directly for preview)
/// 3. Commit (creates a Command with patches) OR Cancel (restores original state)
#[derive(Debug)]
pub struct Transaction {
    /// Description for the resulting command
    description: String,

    /// Original transforms for nodes being edited
    original_transforms: HashMap<NodeId, Affine2>,

    /// Original composed transforms used to apply screen/world-space deltas.
    original_world_transforms: HashMap<NodeId, Affine2>,

    /// Whether the transaction has been committed or cancelled
    finished: bool,
}

impl Transaction {
    /// Start a new transaction for moving the given nodes.
    pub fn start_move(
        description: impl Into<String>,
        nodes: &[NodeId],
        doc: &mut Document,
    ) -> Self {
        let original_transforms = nodes
            .iter()
            .filter_map(|&id| doc.get(id).map(|node| (id, node.transform)))
            .collect();
        let original_world_transforms = nodes
            .iter()
            .copied()
            .map(|id| (id, doc.world_transform(id)))
            .collect();

        Self {
            description: description.into(),
            original_transforms,
            original_world_transforms,
            finished: false,
        }
    }

    /// Update a node's transform during the transaction.
    /// This modifies the document directly for live preview.
    pub fn update_transform(&self, id: NodeId, new_transform: Affine2, doc: &mut Document) {
        if self.finished {
            return;
        }

        if let Some(node) = doc.nodes.get_mut(id) {
            node.transform = new_transform;
        }
        doc.mark_transform_dirty(id);
    }

    /// Apply a delta transform to all nodes in the transaction.
    pub fn apply_delta(&self, delta: Affine2, doc: &mut Document) {
        if self.finished {
            return;
        }

        for (&id, &original_world) in &self.original_world_transforms {
            let new_transform = doc.author_transform_from_world(id, delta * original_world);
            if let Some(node) = doc.nodes.get_mut(id) {
                node.transform = new_transform;
            }
            doc.mark_transform_dirty(id);
        }
    }

    /// Translate all nodes by a delta.
    pub fn translate(&self, delta: glam::Vec2, doc: &mut Document) {
        if self.finished {
            return;
        }

        let delta = Affine2::from_translation(delta);
        for (&id, &original_world) in &self.original_world_transforms {
            let new_transform = doc.author_transform_from_world(id, delta * original_world);
            if let Some(node) = doc.nodes.get_mut(id) {
                node.transform = new_transform;
            }
            doc.mark_transform_dirty(id);
        }
    }

    /// Commit the transaction, creating a Command with the changes.
    pub fn commit(mut self, doc: &Document) -> Command {
        self.finished = true;

        let patches: Vec<Patch> = self
            .original_transforms
            .iter()
            .filter_map(|(&id, &before)| {
                let after = doc.get(id)?.transform;
                // Only create patch if transform actually changed
                if before != after {
                    Some(Patch::SetTransform { id, before, after })
                } else {
                    None
                }
            })
            .collect();

        Command {
            description: self.description.clone(),
            patches,
        }
    }

    /// Cancel the transaction, restoring original state.
    pub fn cancel(mut self, doc: &mut Document) {
        self.finished = true;

        for (&id, &original) in &self.original_transforms {
            if let Some(node) = doc.nodes.get_mut(id) {
                node.transform = original;
            }
            doc.mark_transform_dirty(id);
        }
    }

    /// Check if the transaction has any changes.
    pub fn has_changes(&self, doc: &Document) -> bool {
        self.original_transforms
            .iter()
            .any(|(&id, &before)| doc.get(id).map(|n| n.transform != before).unwrap_or(false))
    }

    /// Get the IDs of nodes in this transaction.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.original_transforms.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;
    use crate::path::PathData;
    use glam::Vec2;

    #[test]
    fn test_transaction_commit() {
        let mut doc = Document::new();
        let shape = Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let id = doc.add_child(doc.root, shape).unwrap();

        let original = doc.get(id).unwrap().transform;

        // Start transaction
        let txn = Transaction::start_move("Move", &[id], &mut doc);

        // Move during transaction
        txn.translate(Vec2::new(100.0, 50.0), &mut doc);

        // Verify document is updated
        let current = doc.get(id).unwrap().transform;
        assert_ne!(current, original);

        // Commit
        let cmd = txn.commit(&doc);
        assert_eq!(cmd.description, "Move");
        assert_eq!(cmd.patches.len(), 1);
    }

    #[test]
    fn test_transaction_cancel() {
        let mut doc = Document::new();
        let shape = Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let id = doc.add_child(doc.root, shape).unwrap();

        let original = doc.get(id).unwrap().transform;

        // Start transaction
        let txn = Transaction::start_move("Move", &[id], &mut doc);

        // Move during transaction
        txn.translate(Vec2::new(100.0, 50.0), &mut doc);

        // Verify document is updated
        assert_ne!(doc.get(id).unwrap().transform, original);

        // Cancel
        txn.cancel(&mut doc);

        // Verify restored to original
        assert_eq!(doc.get(id).unwrap().transform, original);
    }

    #[test]
    fn test_transaction_no_changes() {
        let mut doc = Document::new();
        let shape = Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let id = doc.add_child(doc.root, shape).unwrap();

        // Start transaction but don't modify anything
        let txn = Transaction::start_move("Move", &[id], &mut doc);

        assert!(!txn.has_changes(&doc));

        // Commit should produce empty command
        let cmd = txn.commit(&doc);
        assert!(cmd.is_empty());
    }

    #[test]
    fn test_transaction_multiple_nodes() {
        let mut doc = Document::new();
        let shape1 = Node::shape("Rect1", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let shape2 = Node::shape("Rect2", PathData::rect(0.0, 0.0, 10.0, 10.0))
            .with_transform(Affine2::from_translation(Vec2::new(50.0, 0.0)));

        let id1 = doc.add_child(doc.root, shape1).unwrap();
        let id2 = doc.add_child(doc.root, shape2).unwrap();

        let orig1 = doc.get(id1).unwrap().transform;
        let orig2 = doc.get(id2).unwrap().transform;

        // Start transaction with both nodes
        let txn = Transaction::start_move("Move", &[id1, id2], &mut doc);

        // Move both
        txn.translate(Vec2::new(10.0, 10.0), &mut doc);

        // Commit
        let cmd = txn.commit(&doc);
        assert_eq!(cmd.patches.len(), 2);

        // Verify both moved by same delta
        let new1 = doc.get(id1).unwrap().transform.translation;
        let new2 = doc.get(id2).unwrap().transform.translation;

        assert_eq!(new1 - orig1.translation, Vec2::new(10.0, 10.0));
        assert_eq!(new2 - orig2.translation, Vec2::new(10.0, 10.0));
    }

    #[test]
    fn translation_is_applied_in_world_space_under_transformed_parent() {
        let mut doc = Document::new();
        let group_id = doc
            .add_child(
                doc.root,
                Node::group("Group").with_transform(Affine2::from_scale_angle_translation(
                    Vec2::splat(2.0),
                    0.4,
                    Vec2::new(30.0, 20.0),
                )),
            )
            .unwrap();
        let shape_id = doc
            .add_child(
                group_id,
                Node::shape("Shape", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_transform(Affine2::from_translation(Vec2::new(5.0, 7.0))),
            )
            .unwrap();
        let before = doc.world_transform(shape_id).translation;
        let transaction = Transaction::start_move("Move", &[shape_id], &mut doc);

        transaction.translate(Vec2::new(12.0, -8.0), &mut doc);

        let after = doc.world_transform(shape_id).translation;
        assert!((after - before - Vec2::new(12.0, -8.0)).length() < 0.0001);
    }
}
