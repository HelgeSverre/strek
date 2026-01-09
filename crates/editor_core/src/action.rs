//! Action system for editor operations.
//!
//! This module provides a unified way to define and execute editor operations,
//! with support for keyboard shortcuts, command palette, and menus.

use crate::input::Key;

/// All possible editor actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorAction {
    // === Selection ===
    /// Select all nodes in the document
    SelectAll,
    /// Clear the current selection
    DeselectAll,

    // === Edit ===
    /// Undo the last action
    Undo,
    /// Redo the last undone action
    Redo,
    /// Delete the current selection
    Delete,
    /// Duplicate the current selection
    Duplicate,

    // === Arrange ===
    /// Group the current selection
    Group,
    /// Ungroup the current selection
    Ungroup,

    // === Nudge ===
    /// Move selection up by 1 pixel
    NudgeUp,
    /// Move selection down by 1 pixel
    NudgeDown,
    /// Move selection left by 1 pixel
    NudgeLeft,
    /// Move selection right by 1 pixel
    NudgeRight,
    /// Move selection up by 10 pixels
    NudgeUpLarge,
    /// Move selection down by 10 pixels
    NudgeDownLarge,
    /// Move selection left by 10 pixels
    NudgeLeftLarge,
    /// Move selection right by 10 pixels
    NudgeRightLarge,

    // === View ===
    /// Zoom in
    ZoomIn,
    /// Zoom out
    ZoomOut,
    /// Reset zoom to 100%
    ZoomReset,
    /// Reset zoom and pan
    ZoomResetAll,
}

/// Category for grouping actions in UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionCategory {
    Selection,
    Edit,
    Arrange,
    View,
}

/// Keyboard shortcut definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Shortcut {
    /// Create a shortcut with just a key.
    pub const fn key(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    /// Create a shortcut with Ctrl/Cmd + key.
    pub const fn ctrl(key: Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: false,
        }
    }

    /// Create a shortcut with Ctrl/Cmd + Shift + key.
    pub const fn ctrl_shift(key: Key) -> Self {
        Self {
            key,
            ctrl: true,
            shift: true,
            alt: false,
        }
    }

    /// Create a shortcut with Shift + key.
    pub const fn shift(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            shift: true,
            alt: false,
        }
    }
}

/// Metadata for an action.
#[derive(Debug, Clone)]
pub struct ActionMeta {
    /// Display name for the action
    pub name: &'static str,
    /// Description of what the action does
    pub description: &'static str,
    /// Category for grouping
    pub category: ActionCategory,
    /// Default keyboard shortcut
    pub default_shortcut: Option<Shortcut>,
}

impl EditorAction {
    /// Get metadata for this action.
    pub fn meta(&self) -> ActionMeta {
        match self {
            // Selection
            EditorAction::SelectAll => ActionMeta {
                name: "Select All",
                description: "Select all nodes in the document",
                category: ActionCategory::Selection,
                default_shortcut: Some(Shortcut::ctrl(Key::A)),
            },
            EditorAction::DeselectAll => ActionMeta {
                name: "Deselect All",
                description: "Clear the current selection",
                category: ActionCategory::Selection,
                default_shortcut: Some(Shortcut::key(Key::Escape)),
            },

            // Edit
            EditorAction::Undo => ActionMeta {
                name: "Undo",
                description: "Undo the last action",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::ctrl(Key::Z)),
            },
            EditorAction::Redo => ActionMeta {
                name: "Redo",
                description: "Redo the last undone action",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::ctrl_shift(Key::Z)),
            },
            EditorAction::Delete => ActionMeta {
                name: "Delete",
                description: "Delete the current selection",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::key(Key::Delete)),
            },
            EditorAction::Duplicate => ActionMeta {
                name: "Duplicate",
                description: "Duplicate the current selection",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::ctrl(Key::D)),
            },

            // Arrange
            EditorAction::Group => ActionMeta {
                name: "Group",
                description: "Group the current selection",
                category: ActionCategory::Arrange,
                default_shortcut: Some(Shortcut::ctrl(Key::G)),
            },
            EditorAction::Ungroup => ActionMeta {
                name: "Ungroup",
                description: "Ungroup the current selection",
                category: ActionCategory::Arrange,
                default_shortcut: Some(Shortcut::ctrl_shift(Key::G)),
            },

            // Nudge
            EditorAction::NudgeUp => ActionMeta {
                name: "Nudge Up",
                description: "Move selection up by 1 pixel",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::key(Key::ArrowUp)),
            },
            EditorAction::NudgeDown => ActionMeta {
                name: "Nudge Down",
                description: "Move selection down by 1 pixel",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::key(Key::ArrowDown)),
            },
            EditorAction::NudgeLeft => ActionMeta {
                name: "Nudge Left",
                description: "Move selection left by 1 pixel",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::key(Key::ArrowLeft)),
            },
            EditorAction::NudgeRight => ActionMeta {
                name: "Nudge Right",
                description: "Move selection right by 1 pixel",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::key(Key::ArrowRight)),
            },
            EditorAction::NudgeUpLarge => ActionMeta {
                name: "Nudge Up (Large)",
                description: "Move selection up by 10 pixels",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::shift(Key::ArrowUp)),
            },
            EditorAction::NudgeDownLarge => ActionMeta {
                name: "Nudge Down (Large)",
                description: "Move selection down by 10 pixels",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::shift(Key::ArrowDown)),
            },
            EditorAction::NudgeLeftLarge => ActionMeta {
                name: "Nudge Left (Large)",
                description: "Move selection left by 10 pixels",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::shift(Key::ArrowLeft)),
            },
            EditorAction::NudgeRightLarge => ActionMeta {
                name: "Nudge Right (Large)",
                description: "Move selection right by 10 pixels",
                category: ActionCategory::Edit,
                default_shortcut: Some(Shortcut::shift(Key::ArrowRight)),
            },

            // View
            EditorAction::ZoomIn => ActionMeta {
                name: "Zoom In",
                description: "Zoom in on the canvas",
                category: ActionCategory::View,
                default_shortcut: None, // Typically handled by scroll
            },
            EditorAction::ZoomOut => ActionMeta {
                name: "Zoom Out",
                description: "Zoom out on the canvas",
                category: ActionCategory::View,
                default_shortcut: None,
            },
            EditorAction::ZoomReset => ActionMeta {
                name: "Zoom to 100%",
                description: "Reset zoom to 100%",
                category: ActionCategory::View,
                default_shortcut: Some(Shortcut::ctrl(Key::Digit1)),
            },
            EditorAction::ZoomResetAll => ActionMeta {
                name: "Reset View",
                description: "Reset zoom and pan to default",
                category: ActionCategory::View,
                default_shortcut: Some(Shortcut::ctrl(Key::Digit0)),
            },
        }
    }

    /// Get all actions.
    pub fn all() -> &'static [EditorAction] {
        &[
            EditorAction::SelectAll,
            EditorAction::DeselectAll,
            EditorAction::Undo,
            EditorAction::Redo,
            EditorAction::Delete,
            EditorAction::Duplicate,
            EditorAction::Group,
            EditorAction::Ungroup,
            EditorAction::NudgeUp,
            EditorAction::NudgeDown,
            EditorAction::NudgeLeft,
            EditorAction::NudgeRight,
            EditorAction::NudgeUpLarge,
            EditorAction::NudgeDownLarge,
            EditorAction::NudgeLeftLarge,
            EditorAction::NudgeRightLarge,
            EditorAction::ZoomIn,
            EditorAction::ZoomOut,
            EditorAction::ZoomReset,
            EditorAction::ZoomResetAll,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_actions_have_meta() {
        for action in EditorAction::all() {
            let meta = action.meta();
            assert!(!meta.name.is_empty());
            assert!(!meta.description.is_empty());
        }
    }

    #[test]
    fn test_shortcut_creation() {
        let s = Shortcut::ctrl(Key::Z);
        assert!(s.ctrl);
        assert!(!s.shift);
        assert_eq!(s.key, Key::Z);

        let s2 = Shortcut::ctrl_shift(Key::Z);
        assert!(s2.ctrl);
        assert!(s2.shift);
    }
}
