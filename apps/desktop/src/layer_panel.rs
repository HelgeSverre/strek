//! Layer panel rendering for the desktop app.
//!
//! This module builds the layer panel UI from the editor's layer tree.

use editor_core::layers::{LayerEntry, LayerIcon};
use editor_core::node::NodeId;
use ui::{Color, DrawList, IconKind, Rect, TextStyle};

/// What was clicked in the layer panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerPanelAction {
    /// Click on a layer row (to select it).
    SelectLayer(NodeId),
    /// Click on expand/collapse chevron.
    ToggleExpand(NodeId),
    /// Click on visibility icon.
    ToggleVisibility(NodeId),
    /// Click on lock icon.
    ToggleLock(NodeId),
}

/// Result of hit testing the layer panel.
#[derive(Debug, Clone, Copy)]
pub struct HitTestResult {
    /// The action to perform, if any.
    pub action: Option<LayerPanelAction>,
}

/// Configuration for the layer panel.
pub struct LayerPanelConfig {
    /// Panel width.
    pub width: f32,
    /// Row height for each layer entry.
    pub row_height: f32,
    /// Indent per depth level.
    pub indent: f32,
    /// Icon size.
    pub icon_size: f32,
    /// Spacing between elements.
    pub spacing: f32,
    /// Font size for layer names.
    pub font_size: f32,
    /// Header height.
    pub header_height: f32,
}

impl Default for LayerPanelConfig {
    fn default() -> Self {
        Self {
            width: 240.0,
            row_height: 28.0,
            indent: 16.0,
            icon_size: 16.0,
            spacing: 8.0,
            font_size: 12.0,
            header_height: 32.0,
        }
    }
}

/// Theme colors for the layer panel.
#[allow(dead_code)]
struct Colors {
    background: Color,
    header_background: Color,
    border: Color,
    row_hover: Color,
    row_selected: Color,
    text_primary: Color,
    text_secondary: Color,
    icon_default: Color,
    icon_hidden: Color,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            background: Color::from_hex(0x2a2a2a),
            header_background: Color::from_hex(0x333333),
            border: Color::from_hex(0x1a1a1a),
            row_hover: Color::from_hex(0x3a3a3a),
            row_selected: Color::from_hex(0x0d6efd).with_alpha(0.3),
            text_primary: Color::from_hex(0xffffff),
            text_secondary: Color::from_hex(0x888888),
            icon_default: Color::from_hex(0xbbbbbb),
            icon_hidden: Color::from_hex(0x555555),
        }
    }
}

/// Build the layer panel draw list.
pub fn build_layer_panel(
    entries: &[LayerEntry],
    viewport_height: f32,
    config: &LayerPanelConfig,
) -> DrawList {
    let mut draw_list = DrawList::new();
    let colors = Colors::default();

    // Panel positioned on the right side
    let panel_x = 0.0; // Will be offset when rendering
    let panel_rect = Rect::new(panel_x, 0.0, config.width, viewport_height);

    // Background
    draw_list.fill_rect(panel_rect, colors.background);

    // Left border
    draw_list.fill_rect(Rect::new(panel_x, 0.0, 1.0, viewport_height), colors.border);

    // Header
    let header_rect = Rect::new(panel_x, 0.0, config.width, config.header_height);
    draw_list.fill_rect(header_rect, colors.header_background);

    // Header title
    let title_rect = Rect::new(
        panel_x + config.spacing,
        0.0,
        config.width - config.spacing * 2.0,
        config.header_height,
    );
    draw_list.text(
        "LAYERS",
        title_rect,
        TextStyle {
            font_size: 10.0,
            color: colors.text_secondary,
            bold: true,
            italic: false,
        },
    );

    // Header bottom border
    draw_list.fill_rect(
        Rect::new(panel_x, config.header_height - 1.0, config.width, 1.0),
        colors.border,
    );

    // Layer entries
    let mut y = config.header_height;

    for entry in entries {
        let row_rect = Rect::new(panel_x, y, config.width, config.row_height);

        // Row background (selected or hover)
        if entry.selected {
            draw_list.fill_rect(row_rect, colors.row_selected);
        }

        // Calculate x offset based on depth
        let depth_offset = entry.depth as f32 * config.indent;
        let mut x = panel_x + config.spacing + depth_offset;

        // Expand/collapse chevron for containers
        if entry.is_container && entry.has_children {
            let chevron_rect = Rect::new(x, y + (config.row_height - config.icon_size) / 2.0, config.icon_size, config.icon_size);
            let chevron_icon = if entry.expanded {
                IconKind::ChevronDown
            } else {
                IconKind::ChevronRight
            };
            draw_list.icon(chevron_icon, chevron_rect, colors.icon_default);
            x += config.icon_size + 4.0;
        } else {
            // Placeholder space for alignment
            x += config.icon_size + 4.0;
        }

        // Layer type icon
        let icon_rect = Rect::new(x, y + (config.row_height - config.icon_size) / 2.0, config.icon_size, config.icon_size);
        let layer_icon = match entry.icon {
            LayerIcon::Group => IconKind::Group,
            LayerIcon::Frame => IconKind::Frame,
            LayerIcon::Shape => IconKind::Rectangle,
            LayerIcon::Text => IconKind::Text,
        };
        let icon_color = if entry.visible {
            colors.icon_default
        } else {
            colors.icon_hidden
        };
        draw_list.icon(layer_icon, icon_rect, icon_color);
        x += config.icon_size + config.spacing;

        // Layer name
        let name_width = config.width - x - config.icon_size * 2.0 - config.spacing * 2.0;
        let name_rect = Rect::new(x, y, name_width, config.row_height);
        let text_color = if entry.visible {
            colors.text_primary
        } else {
            colors.text_secondary
        };
        draw_list.text_clipped(
            &entry.name,
            name_rect,
            TextStyle {
                font_size: config.font_size,
                color: text_color,
                bold: false,
                italic: false,
            },
        );

        // Right side icons (visibility, lock)
        let icons_x = panel_x + config.width - config.spacing - config.icon_size * 2.0 - 4.0;

        // Visibility icon
        let eye_rect = Rect::new(icons_x, y + (config.row_height - config.icon_size) / 2.0, config.icon_size, config.icon_size);
        let eye_icon = if entry.visible {
            IconKind::Eye
        } else {
            IconKind::EyeOff
        };
        draw_list.icon(eye_icon, eye_rect, colors.icon_default);

        // Lock icon (only if locked)
        if entry.locked {
            let lock_rect = Rect::new(
                icons_x + config.icon_size + 4.0,
                y + (config.row_height - config.icon_size) / 2.0,
                config.icon_size,
                config.icon_size,
            );
            draw_list.icon(IconKind::Lock, lock_rect, colors.icon_default);
        }

        y += config.row_height;
    }

    draw_list
}

/// Hit test the layer panel to determine what was clicked.
///
/// `local_x` and `local_y` are coordinates relative to the panel's top-left corner.
pub fn hit_test_layer_panel(
    local_x: f32,
    local_y: f32,
    entries: &[LayerEntry],
    config: &LayerPanelConfig,
) -> HitTestResult {
    // Check if in header area
    if local_y < config.header_height {
        return HitTestResult { action: None };
    }

    // Check if outside panel width
    if local_x < 0.0 || local_x > config.width {
        return HitTestResult { action: None };
    }

    // Find which row was clicked
    let row_index = ((local_y - config.header_height) / config.row_height) as usize;

    if row_index >= entries.len() {
        return HitTestResult { action: None };
    }

    let entry = &entries[row_index];
    let row_y = config.header_height + row_index as f32 * config.row_height;

    // Calculate x positions for hit zones
    let depth_offset = entry.depth as f32 * config.indent;
    let chevron_x = config.spacing + depth_offset;
    let chevron_end = chevron_x + config.icon_size;

    let _type_icon_x = chevron_end + 4.0;

    // Right side icons
    let visibility_x = config.width - config.spacing - config.icon_size * 2.0 - 4.0;
    let visibility_end = visibility_x + config.icon_size;
    let lock_x = visibility_end + 4.0;
    let lock_end = lock_x + config.icon_size;

    // Icon vertical bounds (centered in row)
    let icon_y = row_y + (config.row_height - config.icon_size) / 2.0;
    let icon_y_end = icon_y + config.icon_size;

    // Check if click is in vertical range for icons
    let in_icon_y_range = local_y >= icon_y && local_y <= icon_y_end;

    // Check visibility icon
    if in_icon_y_range && local_x >= visibility_x && local_x <= visibility_end {
        return HitTestResult {
            action: Some(LayerPanelAction::ToggleVisibility(entry.id)),
        };
    }

    // Check lock icon
    if in_icon_y_range && local_x >= lock_x && local_x <= lock_end {
        return HitTestResult {
            action: Some(LayerPanelAction::ToggleLock(entry.id)),
        };
    }

    // Check chevron (only for containers with children)
    if entry.is_container && entry.has_children {
        if in_icon_y_range && local_x >= chevron_x && local_x <= chevron_end {
            return HitTestResult {
                action: Some(LayerPanelAction::ToggleExpand(entry.id)),
            };
        }
    }

    // Otherwise, clicking anywhere on the row selects the layer
    HitTestResult {
        action: Some(LayerPanelAction::SelectLayer(entry.id)),
    }
}
