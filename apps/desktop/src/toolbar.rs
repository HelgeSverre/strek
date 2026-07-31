//! Toolbar rendering for the desktop app.
//!
//! Provides tool selection buttons at the top of the window.

use editor_core::Tool;
use ui::{Color, DrawList, IconKind, Rect};

/// What was clicked in the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarAction {
    /// Select a tool.
    SelectTool(ToolButton),
}

/// Tool buttons in the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolButton {
    Select,
    Rectangle,
    Ellipse,
    Pen,
    Text,
}

impl ToolButton {
    /// Get the icon for this tool button.
    pub fn icon(&self) -> IconKind {
        match self {
            ToolButton::Select => IconKind::ChevronUp, // Pointer-like
            ToolButton::Rectangle => IconKind::Rectangle,
            ToolButton::Ellipse => IconKind::Ellipse,
            ToolButton::Pen => IconKind::ChevronRight, // Pen-like
            ToolButton::Text => IconKind::Text,
        }
    }

    /// Get the tooltip/label for this tool.
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            ToolButton::Select => "Select (V)",
            ToolButton::Rectangle => "Rectangle (R)",
            ToolButton::Ellipse => "Ellipse (O)",
            ToolButton::Pen => "Pen (P)",
            ToolButton::Text => "Text (T)",
        }
    }

    /// Check if this tool is currently active.
    pub fn is_active(&self, current_tool: Tool) -> bool {
        matches!(
            (self, current_tool),
            (ToolButton::Select, Tool::Select)
                | (ToolButton::Rectangle, Tool::Rectangle)
                | (ToolButton::Ellipse, Tool::Ellipse)
        )
    }

    /// Check if this tool is enabled (implemented).
    pub fn is_enabled(&self) -> bool {
        match self {
            ToolButton::Select | ToolButton::Rectangle | ToolButton::Ellipse => true,
            ToolButton::Pen => false,
            ToolButton::Text => false,
        }
    }

    /// All tool buttons in order.
    pub fn all() -> &'static [ToolButton] {
        &[
            ToolButton::Select,
            ToolButton::Rectangle,
            ToolButton::Ellipse,
            ToolButton::Pen,
            ToolButton::Text,
        ]
    }
}

/// Configuration for the toolbar.
pub struct ToolbarConfig {
    /// Toolbar height.
    pub height: f32,
    /// Button size.
    pub button_size: f32,
    /// Spacing between buttons.
    pub spacing: f32,
    /// Left padding.
    pub padding_left: f32,
}

impl Default for ToolbarConfig {
    fn default() -> Self {
        Self {
            height: 40.0,
            button_size: 32.0,
            spacing: 4.0,
            padding_left: 8.0,
        }
    }
}

/// Theme colors for the toolbar.
struct Colors {
    background: Color,
    border: Color,
    button_default: Color,
    #[allow(dead_code)]
    button_hover: Color,
    button_active: Color,
    button_disabled: Color,
    icon_default: Color,
    icon_active: Color,
    icon_disabled: Color,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            background: Color::from_hex(0x333333),
            border: Color::from_hex(0x1a1a1a),
            button_default: Color::from_hex(0x3a3a3a),
            button_hover: Color::from_hex(0x4a4a4a),
            button_active: Color::from_hex(0x0d6efd),
            button_disabled: Color::from_hex(0x2a2a2a),
            icon_default: Color::from_hex(0xdddddd),
            icon_active: Color::from_hex(0xffffff),
            icon_disabled: Color::from_hex(0x666666),
        }
    }
}

/// Build the toolbar draw list.
pub fn build_toolbar(viewport_width: f32, current_tool: Tool, config: &ToolbarConfig) -> DrawList {
    let mut draw_list = DrawList::new();
    let colors = Colors::default();

    // Background
    let toolbar_rect = Rect::new(0.0, 0.0, viewport_width, config.height);
    draw_list.fill_rect(toolbar_rect, colors.background);

    // Bottom border
    draw_list.fill_rect(
        Rect::new(0.0, config.height - 1.0, viewport_width, 1.0),
        colors.border,
    );

    // Tool buttons
    let mut x = config.padding_left;
    let y = (config.height - config.button_size) / 2.0;

    for tool in ToolButton::all() {
        let button_rect = Rect::new(x, y, config.button_size, config.button_size);
        let is_active = tool.is_active(current_tool);
        let is_enabled = tool.is_enabled();

        // Button background
        let bg_color = if is_active {
            colors.button_active
        } else if !is_enabled {
            colors.button_disabled
        } else {
            colors.button_default
        };
        draw_list.fill_rounded_rect(button_rect, bg_color, 4.0);

        // Icon
        let icon_padding = 6.0;
        let icon_rect = Rect::new(
            x + icon_padding,
            y + icon_padding,
            config.button_size - icon_padding * 2.0,
            config.button_size - icon_padding * 2.0,
        );

        let icon_color = if is_active {
            colors.icon_active
        } else if !is_enabled {
            colors.icon_disabled
        } else {
            colors.icon_default
        };

        draw_list.icon(tool.icon(), icon_rect, icon_color);

        x += config.button_size + config.spacing;
    }

    // Separator after tools
    x += config.spacing * 2.0;
    draw_list.fill_rect(Rect::new(x, 8.0, 1.0, config.height - 16.0), colors.border);

    draw_list
}

/// Hit test the toolbar to determine what was clicked.
pub fn hit_test_toolbar(
    local_x: f32,
    local_y: f32,
    config: &ToolbarConfig,
) -> Option<ToolbarAction> {
    // Check if in toolbar bounds
    if local_y < 0.0 || local_y > config.height {
        return None;
    }

    // Check tool buttons
    let mut x = config.padding_left;
    let y = (config.height - config.button_size) / 2.0;

    for tool in ToolButton::all() {
        let button_rect = Rect::new(x, y, config.button_size, config.button_size);

        if local_x >= button_rect.x
            && local_x <= button_rect.x + button_rect.width
            && local_y >= button_rect.y
            && local_y <= button_rect.y + button_rect.height
        {
            // Only return action for enabled tools
            if tool.is_enabled() {
                return Some(ToolbarAction::SelectTool(*tool));
            } else {
                return None; // Clicked disabled tool
            }
        }

        x += config.button_size + config.spacing;
    }

    None
}
