//! Theming support for consistent UI styling.

use crate::Color;

/// UI theme with colors and sizing.
#[derive(Debug, Clone)]
pub struct Theme {
    // Background colors
    pub bg_primary: Color,
    pub bg_secondary: Color,
    pub bg_tertiary: Color,
    pub bg_hover: Color,
    pub bg_selected: Color,
    pub bg_pressed: Color,

    // Text colors
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_disabled: Color,
    pub text_inverse: Color,

    // Accent colors
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,

    // Border colors
    pub border: Color,
    pub border_light: Color,

    // Semantic colors
    pub error: Color,
    pub warning: Color,
    pub success: Color,

    // Sizing
    pub font_size: f32,
    pub font_size_small: f32,
    pub font_size_large: f32,
    pub line_height: f32,
    pub spacing: f32,
    pub spacing_small: f32,
    pub spacing_large: f32,
    pub border_radius: f32,
    pub icon_size: f32,
}

impl Theme {
    /// Create the default dark theme.
    pub fn dark() -> Self {
        Self {
            // Backgrounds
            bg_primary: Color::from_hex(0x1e1e1e),
            bg_secondary: Color::from_hex(0x2a2a2a),
            bg_tertiary: Color::from_hex(0x333333),
            bg_hover: Color::from_hex(0x3a3a3a),
            bg_selected: Color::from_hex(0x0066cc),
            bg_pressed: Color::from_hex(0x444444),

            // Text
            text_primary: Color::from_hex(0xe0e0e0),
            text_secondary: Color::from_hex(0x888888),
            text_disabled: Color::from_hex(0x555555),
            text_inverse: Color::WHITE,

            // Accent
            accent: Color::from_hex(0x0078d4),
            accent_hover: Color::from_hex(0x1084d8),
            accent_pressed: Color::from_hex(0x006cbe),

            // Borders
            border: Color::from_hex(0x1a1a1a),
            border_light: Color::from_hex(0x404040),

            // Semantic
            error: Color::from_hex(0xf44336),
            warning: Color::from_hex(0xff9800),
            success: Color::from_hex(0x4caf50),

            // Sizing
            font_size: 13.0,
            font_size_small: 11.0,
            font_size_large: 16.0,
            line_height: 1.4,
            spacing: 8.0,
            spacing_small: 4.0,
            spacing_large: 16.0,
            border_radius: 4.0,
            icon_size: 16.0,
        }
    }

    /// Create a light theme.
    pub fn light() -> Self {
        Self {
            // Backgrounds
            bg_primary: Color::from_hex(0xffffff),
            bg_secondary: Color::from_hex(0xf5f5f5),
            bg_tertiary: Color::from_hex(0xeeeeee),
            bg_hover: Color::from_hex(0xe8e8e8),
            bg_selected: Color::from_hex(0x0078d4),
            bg_pressed: Color::from_hex(0xdddddd),

            // Text
            text_primary: Color::from_hex(0x1a1a1a),
            text_secondary: Color::from_hex(0x666666),
            text_disabled: Color::from_hex(0x999999),
            text_inverse: Color::WHITE,

            // Accent
            accent: Color::from_hex(0x0078d4),
            accent_hover: Color::from_hex(0x1084d8),
            accent_pressed: Color::from_hex(0x006cbe),

            // Borders
            border: Color::from_hex(0xdddddd),
            border_light: Color::from_hex(0xeeeeee),

            // Semantic
            error: Color::from_hex(0xd32f2f),
            warning: Color::from_hex(0xf57c00),
            success: Color::from_hex(0x388e3c),

            // Sizing
            font_size: 13.0,
            font_size_small: 11.0,
            font_size_large: 16.0,
            line_height: 1.4,
            spacing: 8.0,
            spacing_small: 4.0,
            spacing_large: 16.0,
            border_radius: 4.0,
            icon_size: 16.0,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}
