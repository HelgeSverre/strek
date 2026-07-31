use crate::command::Command;
use crate::Document;

#[derive(Debug)]
struct HistoryEntry {
    command: Command,
    before_revision: u64,
    after_revision: u64,
}

/// Undo/redo history stack.
#[derive(Debug)]
pub struct History {
    /// Stack of commands that can be undone
    undo_stack: Vec<HistoryEntry>,

    /// Stack of commands that can be redone
    redo_stack: Vec<HistoryEntry>,

    /// Maximum number of commands to keep in history
    max_size: Option<usize>,

    current_revision: u64,
    saved_revision: u64,
    next_revision: u64,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_size: None,
            current_revision: 0,
            saved_revision: 0,
            next_revision: 1,
        }
    }
}

impl History {
    /// Create a new empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a history with a maximum size.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            max_size: Some(max_size),
            ..Default::default()
        }
    }

    /// Push a command onto the history.
    /// This clears the redo stack.
    pub fn push(&mut self, cmd: Command) {
        // Don't push empty commands
        if cmd.is_empty() {
            return;
        }

        let after_revision = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
        self.undo_stack.push(HistoryEntry {
            command: cmd,
            before_revision: self.current_revision,
            after_revision,
        });
        self.current_revision = after_revision;
        self.redo_stack.clear();

        // Enforce max size
        if let Some(max) = self.max_size {
            while self.undo_stack.len() > max {
                self.undo_stack.remove(0);
            }
        }
    }

    /// Undo the last command.
    /// Returns true if a command was undone.
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(entry) = self.undo_stack.pop() {
            entry.command.unapply(doc);
            self.current_revision = entry.before_revision;
            self.redo_stack.push(entry);
            true
        } else {
            false
        }
    }

    /// Redo the last undone command.
    /// Returns true if a command was redone.
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(entry) = self.redo_stack.pop() {
            entry.command.apply(doc);
            self.current_revision = entry.after_revision;
            self.undo_stack.push(entry);
            true
        } else {
            false
        }
    }

    /// Check if undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Get the description of the next undo command.
    pub fn undo_description(&self) -> Option<&str> {
        self.undo_stack
            .last()
            .map(|entry| entry.command.description.as_str())
    }

    /// Get the description of the next redo command.
    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack
            .last()
            .map(|entry| entry.command.description.as_str())
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Mark the current document revision as persisted.
    pub fn mark_saved(&mut self) {
        self.saved_revision = self.current_revision;
    }

    /// Whether the current document revision differs from the saved revision.
    pub fn is_dirty(&self) -> bool {
        self.current_revision != self.saved_revision
    }

    /// Get the number of commands in the undo stack.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Get the number of commands in the redo stack.
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Patch;
    use crate::node::Node;
    use crate::path::PathData;
    use glam::{Affine2, Vec2};

    #[test]
    fn test_undo_redo() {
        let mut doc = Document::new();
        let mut history = History::new();

        let shape = Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let id = doc.add_child(doc.root, shape).unwrap();

        let before = Affine2::IDENTITY;
        let after = Affine2::from_translation(Vec2::new(100.0, 0.0));

        // Apply a command
        let cmd = Command::new("Move").with_patch(Patch::SetTransform { id, before, after });
        cmd.apply(&mut doc);
        history.push(cmd);

        assert_eq!(doc.get(id).unwrap().transform, after);
        assert!(history.can_undo());
        assert!(!history.can_redo());

        // Undo
        assert!(history.undo(&mut doc));
        assert_eq!(doc.get(id).unwrap().transform, before);
        assert!(!history.can_undo());
        assert!(history.can_redo());

        // Redo
        assert!(history.redo(&mut doc));
        assert_eq!(doc.get(id).unwrap().transform, after);
        assert!(history.can_undo());
        assert!(!history.can_redo());
    }

    #[test]
    fn test_redo_cleared_on_new_command() {
        let mut doc = Document::new();
        let mut history = History::new();

        let shape = Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let id = doc.add_child(doc.root, shape).unwrap();

        // First command
        let cmd1 = Command::new("Move 1").with_patch(Patch::SetTransform {
            id,
            before: Affine2::IDENTITY,
            after: Affine2::from_translation(Vec2::new(10.0, 0.0)),
        });
        cmd1.apply(&mut doc);
        history.push(cmd1);

        // Undo
        history.undo(&mut doc);
        assert!(history.can_redo());

        // New command should clear redo
        let cmd2 = Command::new("Move 2").with_patch(Patch::SetTransform {
            id,
            before: Affine2::IDENTITY,
            after: Affine2::from_translation(Vec2::new(20.0, 0.0)),
        });
        cmd2.apply(&mut doc);
        history.push(cmd2);

        assert!(!history.can_redo());
    }

    #[test]
    fn test_max_size() {
        let mut doc = Document::new();
        let mut history = History::with_max_size(3);

        let shape = Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0));
        let id = doc.add_child(doc.root, shape).unwrap();

        // Push 5 commands
        for i in 0..5 {
            let cmd = Command::new(format!("Move {}", i)).with_patch(Patch::SetTransform {
                id,
                before: Affine2::IDENTITY,
                after: Affine2::from_translation(Vec2::new(i as f32 * 10.0, 0.0)),
            });
            history.push(cmd);
        }

        // Should only have 3 commands
        assert_eq!(history.undo_count(), 3);
    }

    #[test]
    fn test_descriptions() {
        let mut history = History::new();

        assert_eq!(history.undo_description(), None);
        assert_eq!(history.redo_description(), None);

        let cmd = Command::new("Test Command").with_patch(Patch::SetName {
            id: crate::node::NodeId::default(),
            before: "".into(),
            after: "".into(),
        });
        history.push(cmd);

        assert_eq!(history.undo_description(), Some("Test Command"));
    }

    #[test]
    fn dirty_state_tracks_save_undo_redo_and_branches() {
        let mut doc = Document::new();
        let mut history = History::new();
        let id = doc
            .add_child(
                doc.root,
                Node::shape("Rect", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();

        assert!(!history.is_dirty());

        let first = Command::new("Move").with_patch(Patch::SetTransform {
            id,
            before: Affine2::IDENTITY,
            after: Affine2::from_translation(Vec2::new(10.0, 0.0)),
        });
        first.apply(&mut doc);
        history.push(first);
        assert!(history.is_dirty());

        history.mark_saved();
        assert!(!history.is_dirty());

        assert!(history.undo(&mut doc));
        assert!(history.is_dirty());
        assert!(history.redo(&mut doc));
        assert!(!history.is_dirty());

        assert!(history.undo(&mut doc));
        let branch = Command::new("Move elsewhere").with_patch(Patch::SetTransform {
            id,
            before: Affine2::IDENTITY,
            after: Affine2::from_translation(Vec2::new(20.0, 0.0)),
        });
        branch.apply(&mut doc);
        history.push(branch);
        assert!(history.is_dirty());
        assert!(!history.can_redo());
    }
}
