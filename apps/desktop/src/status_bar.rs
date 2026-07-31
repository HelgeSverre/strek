//! Status bar rendering for the desktop app.
//!
//! Shows tool state, selection info, and cursor position at the bottom of the window.

use editor_core::Tool;
use glam::Vec2;
use ui::{Color, DrawList, Rect, TextStyle};

/// Configuration for the status bar.
pub struct StatusBarConfig {
    /// Status bar height.
    pub height: f32,
    /// Font size.
    pub font_size: f32,
    /// Padding.
    pub padding: f32,
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            height: 24.0,
            font_size: 11.0,
            padding: 8.0,
        }
    }
}

/// Theme colors for the status bar.
struct Colors {
    background: Color,
    border: Color,
    text_primary: Color,
    text_secondary: Color,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            background: Color::from_hex(0x252525),
            border: Color::from_hex(0x1a1a1a),
            text_primary: Color::from_hex(0xdddddd),
            text_secondary: Color::from_hex(0x888888),
        }
    }
}

/// Current state to display in the status bar.
pub struct StatusBarState {
    /// Current tool.
    pub tool: Tool,
    /// Number of selected nodes.
    pub selection_count: usize,
    /// Current mouse position in world coordinates.
    pub mouse_position: Option<Vec2>,
    /// Current zoom level.
    pub zoom: f32,
}

/// Build the status bar draw list.
pub fn build_status_bar(
    viewport_width: f32,
    viewport_height: f32,
    state: &StatusBarState,
    config: &StatusBarConfig,
) -> DrawList {
    let mut draw_list = DrawList::new();
    let colors = Colors::default();

    let y = viewport_height - config.height;

    // Background
    let bar_rect = Rect::new(0.0, y, viewport_width, config.height);
    draw_list.fill_rect(bar_rect, colors.background);

    // Top border
    draw_list.fill_rect(Rect::new(0.0, y, viewport_width, 1.0), colors.border);

    // Text style
    let text_style = TextStyle {
        font_size: config.font_size,
        color: colors.text_primary,
        bold: false,
        italic: false,
    };

    let secondary_style = TextStyle {
        font_size: config.font_size,
        color: colors.text_secondary,
        bold: false,
        italic: false,
    };

    // Left section: Tool info
    let tool_name = match state.tool {
        Tool::Select => "Select",
        Tool::Frame => "Frame",
        Tool::Rectangle => "Rectangle",
        Tool::Ellipse => "Ellipse",
        Tool::Line => "Line",
        Tool::Pen => "Pen",
        Tool::Text => "Text",
        Tool::VectorEdit => "Edit vector",
    };

    let tool_text = format!("Tool: {}", tool_name);
    draw_list.text(
        &tool_text,
        Rect::new(config.padding, y, 100.0, config.height),
        text_style.clone(),
    );

    // Middle section: Selection info
    let selection_text = if state.selection_count == 0 {
        "No selection".to_string()
    } else if state.selection_count == 1 {
        "1 layer selected".to_string()
    } else {
        format!("{} layers selected", state.selection_count)
    };

    draw_list.text(
        &selection_text,
        Rect::new(120.0, y, 150.0, config.height),
        secondary_style.clone(),
    );

    // Right section: Position and zoom
    let right_x = viewport_width - 200.0;

    // Mouse position
    if let Some(pos) = state.mouse_position {
        let pos_text = format!("X: {:.0}  Y: {:.0}", pos.x, pos.y);
        draw_list.text(
            &pos_text,
            Rect::new(right_x, y, 100.0, config.height),
            secondary_style.clone(),
        );
    }

    // Zoom level
    let zoom_text = format!("{:.0}%", state.zoom * 100.0);
    draw_list.text(
        &zoom_text,
        Rect::new(
            viewport_width - 60.0 - config.padding,
            y,
            60.0,
            config.height,
        ),
        secondary_style,
    );

    draw_list
}
