use crate::command::Command;
use crate::Document;

/// Undo/redo history stack.
#[derive(Debug, Default)]
pub struct History {
    /// Stack of commands that can be undone
    undo_stack: Vec<Command>,

    /// Stack of commands that can be redone
    redo_stack: Vec<Command>,

    /// Maximum number of commands to keep in history
    max_size: Option<usize>,
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

        self.undo_stack.push(cmd);
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
        if let Some(cmd) = self.undo_stack.pop() {
            cmd.unapply(doc);
            self.redo_stack.push(cmd);
            true
        } else {
            false
        }
    }

    /// Redo the last undone command.
    /// Returns true if a command was redone.
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.redo_stack.pop() {
            cmd.apply(doc);
            self.undo_stack.push(cmd);
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
        self.undo_stack.last().map(|cmd| cmd.description.as_str())
    }

    /// Get the description of the next redo command.
    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack.last().map(|cmd| cmd.description.as_str())
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
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
}
