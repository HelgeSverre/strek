//! Drawing commands for UI rendering.
//!
//! Widgets produce DrawCommands that are collected into a DrawList,
//! which can then be rendered by any backend.

use crate::{Color, Rect};

/// Text styling.
#[derive(Debug, Clone)]
pub struct TextStyle {
    pub font_size: f32,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            color: Color::WHITE,
            bold: false,
            italic: false,
        }
    }
}

impl TextStyle {
    /// Create a new text style.
    pub fn new(font_size: f32, color: Color) -> Self {
        Self {
            font_size,
            color,
            bold: false,
            italic: false,
        }
    }

    /// Set bold.
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Set italic.
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
}

/// A drawing command.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    /// Fill a rectangle with a solid color.
    FillRect {
        rect: Rect,
        color: Color,
        corner_radius: f32,
    },

    /// Stroke a rectangle.
    StrokeRect {
        rect: Rect,
        color: Color,
        width: f32,
        corner_radius: f32,
    },

    /// Draw text.
    Text {
        text: String,
        rect: Rect,
        style: TextStyle,
        clip: bool,
    },

    /// Draw an icon (using icon font or predefined shapes).
    Icon {
        kind: IconKind,
        rect: Rect,
        color: Color,
    },

    /// Push a clip rectangle.
    PushClip(Rect),

    /// Pop the current clip rectangle.
    PopClip,
}

/// Built-in icon kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    // Arrows
    ChevronRight,
    ChevronDown,
    ChevronLeft,
    ChevronUp,

    // Actions
    Close,
    Plus,
    Minus,
    Check,

    // Objects
    Folder,
    File,
    Eye,
    EyeOff,
    Lock,
    Unlock,

    // Shapes (for layer panel)
    Rectangle,
    Ellipse,
    Text,
    Frame,
    Group,
}

/// A list of drawing commands.
#[derive(Debug, Clone, Default)]
pub struct DrawList {
    pub commands: Vec<DrawCommand>,
}

impl DrawList {
    /// Create a new empty draw list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a command to the list.
    pub fn push(&mut self, command: DrawCommand) {
        self.commands.push(command);
    }

    /// Fill a rectangle.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.commands.push(DrawCommand::FillRect {
            rect,
            color,
            corner_radius: 0.0,
        });
    }

    /// Fill a rounded rectangle.
    pub fn fill_rounded_rect(&mut self, rect: Rect, color: Color, corner_radius: f32) {
        self.commands.push(DrawCommand::FillRect {
            rect,
            color,
            corner_radius,
        });
    }

    /// Stroke a rectangle.
    pub fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32) {
        self.commands.push(DrawCommand::StrokeRect {
            rect,
            color,
            width,
            corner_radius: 0.0,
        });
    }

    /// Draw text.
    pub fn text(&mut self, text: impl Into<String>, rect: Rect, style: TextStyle) {
        self.commands.push(DrawCommand::Text {
            text: text.into(),
            rect,
            style,
            clip: false,
        });
    }

    /// Draw clipped text.
    pub fn text_clipped(&mut self, text: impl Into<String>, rect: Rect, style: TextStyle) {
        self.commands.push(DrawCommand::Text {
            text: text.into(),
            rect,
            style,
            clip: true,
        });
    }

    /// Draw an icon.
    pub fn icon(&mut self, kind: IconKind, rect: Rect, color: Color) {
        self.commands.push(DrawCommand::Icon { kind, rect, color });
    }

    /// Push a clip rectangle.
    pub fn push_clip(&mut self, rect: Rect) {
        self.commands.push(DrawCommand::PushClip(rect));
    }

    /// Pop the clip rectangle.
    pub fn pop_clip(&mut self) {
        self.commands.push(DrawCommand::PopClip);
    }

    /// Append another draw list.
    pub fn append(&mut self, other: &DrawList) {
        self.commands.extend(other.commands.iter().cloned());
    }

    /// Check if the list is empty.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Get the number of commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Clear all commands.
    pub fn clear(&mut self) {
        self.commands.clear();
    }
}
