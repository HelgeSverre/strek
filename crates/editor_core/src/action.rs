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
    /// Invert the current selection
    InvertSelection,

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
    /// Bring selection to front
    BringToFront,
    /// Send selection to back
    SendToBack,
    /// Bring selection forward one step
    BringForward,
    /// Send selection backward one step
    SendBackward,

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
    /// Zoom to fit all content
    ZoomToFit,
    /// Zoom to fit selection
    ZoomToSelection,

    // === Tools ===
    /// Switch to Select tool
    ToolSelect,
    /// Switch to Frame tool
    ToolFrame,
    /// Switch to Rectangle tool
    ToolRectangle,
    /// Switch to Ellipse tool
    ToolEllipse,
    /// Switch to Pen tool
    ToolPen,
    /// Switch to Text tool
    ToolText,

    // === Vector editing ===
    /// Enter direct vector-edit mode for the selected path
    EnterVectorEdit,
    /// Finish the active Pen, Text, or vector-edit session
    FinishEditing,
    /// Join two selected open contour endpoints
    JoinPaths,
    /// Split a contour at the selected anchor
    SplitPath,
    /// Reverse selected contours
    ReversePath,
    /// Open or close the active contour
    TogglePathClosed,
}

/// Category for grouping actions in UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionCategory {
    Selection,
    Edit,
    Arrange,
    View,
    Tools,
    Path,
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
            EditorAction::InvertSelection => ActionMeta {
                name: "Invert Selection",
                description: "Invert the current selection",
                category: ActionCategory::Selection,
                default_shortcut: Some(Shortcut::ctrl_shift(Key::I)),
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
            EditorAction::BringToFront => ActionMeta {
                name: "Bring to Front",
                description: "Bring selection to front of z-order",
                category: ActionCategory::Arrange,
                default_shortcut: Some(Shortcut::ctrl_shift(Key::BracketRight)),
            },
            EditorAction::SendToBack => ActionMeta {
                name: "Send to Back",
                description: "Send selection to back of z-order",
                category: ActionCategory::Arrange,
                default_shortcut: Some(Shortcut::ctrl_shift(Key::BracketLeft)),
            },
            EditorAction::BringForward => ActionMeta {
                name: "Bring Forward",
                description: "Bring selection forward one step",
                category: ActionCategory::Arrange,
                default_shortcut: Some(Shortcut::ctrl(Key::BracketRight)),
            },
            EditorAction::SendBackward => ActionMeta {
                name: "Send Backward",
                description: "Send selection backward one step",
                category: ActionCategory::Arrange,
                default_shortcut: Some(Shortcut::ctrl(Key::BracketLeft)),
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
            EditorAction::ZoomToFit => ActionMeta {
                name: "Zoom to Fit",
                description: "Zoom to fit all content",
                category: ActionCategory::View,
                default_shortcut: Some(Shortcut::ctrl(Key::Digit9)),
            },
            EditorAction::ZoomToSelection => ActionMeta {
                name: "Zoom to Selection",
                description: "Zoom to fit the current selection",
                category: ActionCategory::View,
                default_shortcut: Some(Shortcut::ctrl_shift(Key::Digit9)),
            },

            // Tools
            EditorAction::ToolSelect => ActionMeta {
                name: "Select Tool",
                description: "Switch to the selection tool",
                category: ActionCategory::Tools,
                default_shortcut: Some(Shortcut::key(Key::V)),
            },
            EditorAction::ToolFrame => ActionMeta {
                name: "Frame Tool",
                description: "Draw a frame and adopt enclosed layers",
                category: ActionCategory::Tools,
                default_shortcut: Some(Shortcut::key(Key::F)),
            },
            EditorAction::ToolRectangle => ActionMeta {
                name: "Rectangle Tool",
                description: "Switch to the rectangle drawing tool",
                category: ActionCategory::Tools,
                default_shortcut: Some(Shortcut::key(Key::R)),
            },
            EditorAction::ToolEllipse => ActionMeta {
                name: "Ellipse Tool",
                description: "Switch to the ellipse drawing tool",
                category: ActionCategory::Tools,
                default_shortcut: Some(Shortcut::key(Key::O)),
            },
            EditorAction::ToolPen => ActionMeta {
                name: "Pen Tool",
                description: "Switch to the pen drawing tool",
                category: ActionCategory::Tools,
                default_shortcut: Some(Shortcut::key(Key::P)),
            },
            EditorAction::ToolText => ActionMeta {
                name: "Text Tool",
                description: "Switch to the text tool",
                category: ActionCategory::Tools,
                default_shortcut: Some(Shortcut::key(Key::T)),
            },
            EditorAction::EnterVectorEdit => ActionMeta {
                name: "Edit Vector",
                description: "Edit anchors and Bézier handles",
                category: ActionCategory::Path,
                default_shortcut: Some(Shortcut::key(Key::Enter)),
            },
            EditorAction::FinishEditing => ActionMeta {
                name: "Finish Editing",
                description: "Commit the active editing session",
                category: ActionCategory::Path,
                default_shortcut: None,
            },
            EditorAction::JoinPaths => ActionMeta {
                name: "Join Paths",
                description: "Join two selected open endpoints",
                category: ActionCategory::Path,
                default_shortcut: Some(Shortcut::ctrl(Key::J)),
            },
            EditorAction::SplitPath => ActionMeta {
                name: "Split Path",
                description: "Split the active path at one anchor",
                category: ActionCategory::Path,
                default_shortcut: None,
            },
            EditorAction::ReversePath => ActionMeta {
                name: "Reverse Path",
                description: "Reverse selected path contours",
                category: ActionCategory::Path,
                default_shortcut: None,
            },
            EditorAction::TogglePathClosed => ActionMeta {
                name: "Open or Close Path",
                description: "Toggle the active contour closure",
                category: ActionCategory::Path,
                default_shortcut: None,
            },
        }
    }

    /// Get all actions.
    pub fn all() -> &'static [EditorAction] {
        &[
            // Selection
            EditorAction::SelectAll,
            EditorAction::DeselectAll,
            EditorAction::InvertSelection,
            // Edit
            EditorAction::Undo,
            EditorAction::Redo,
            EditorAction::Delete,
            EditorAction::Duplicate,
            // Arrange
            EditorAction::Group,
            EditorAction::Ungroup,
            EditorAction::BringToFront,
            EditorAction::SendToBack,
            EditorAction::BringForward,
            EditorAction::SendBackward,
            // Nudge
            EditorAction::NudgeUp,
            EditorAction::NudgeDown,
            EditorAction::NudgeLeft,
            EditorAction::NudgeRight,
            EditorAction::NudgeUpLarge,
            EditorAction::NudgeDownLarge,
            EditorAction::NudgeLeftLarge,
            EditorAction::NudgeRightLarge,
            // View
            EditorAction::ZoomIn,
            EditorAction::ZoomOut,
            EditorAction::ZoomReset,
            EditorAction::ZoomResetAll,
            EditorAction::ZoomToFit,
            EditorAction::ZoomToSelection,
            // Tools
            EditorAction::ToolSelect,
            EditorAction::ToolFrame,
            EditorAction::ToolRectangle,
            EditorAction::ToolEllipse,
            EditorAction::ToolPen,
            EditorAction::ToolText,
            // Path
            EditorAction::EnterVectorEdit,
            EditorAction::FinishEditing,
            EditorAction::JoinPaths,
            EditorAction::SplitPath,
            EditorAction::ReversePath,
            EditorAction::TogglePathClosed,
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
