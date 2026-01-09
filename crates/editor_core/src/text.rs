//! Text engine abstraction for text measurement and shaping.
//!
//! This module provides traits and types for platform-agnostic text handling.
//! Different platforms implement the `TextEngine` trait:
//! - Web: Browser handles text via SVG `<text>` elements
//! - Desktop: Uses fontdue/cosmic-text for font loading and shaping

use glam::Vec2;

use crate::node::TextData;
use crate::path::Rect;

/// A unique identifier for a font.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(pub u32);

/// A positioned glyph for rendering.
#[derive(Debug, Clone, Copy)]
pub struct PositionedGlyph {
    /// Glyph ID in the font.
    pub glyph_id: u16,

    /// Position of the glyph baseline origin.
    pub position: Vec2,

    /// Size of the glyph in pixels.
    pub size: Vec2,
}

/// A run of glyphs from a single font.
#[derive(Debug, Clone)]
pub struct GlyphRun {
    /// The positioned glyphs.
    pub glyphs: Vec<PositionedGlyph>,

    /// The font used for this run.
    pub font_id: FontId,
}

/// Text metrics for layout.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextMetrics {
    /// Total width of the text.
    pub width: f32,

    /// Height of the text (typically line height).
    pub height: f32,

    /// Distance from baseline to top of tallest glyph.
    pub ascent: f32,

    /// Distance from baseline to bottom of lowest glyph (positive value).
    pub descent: f32,
}

impl TextMetrics {
    /// Create metrics from width and font size.
    pub fn from_size(width: f32, font_size: f32) -> Self {
        Self {
            width,
            height: font_size,
            ascent: font_size * 0.8,
            descent: font_size * 0.2,
        }
    }

    /// Get the bounding rect for the text.
    pub fn bounds(&self) -> Rect {
        Rect::from_pos_size(Vec2::ZERO, Vec2::new(self.width, self.height))
    }
}

/// Trait for text measurement and shaping.
///
/// Different platforms implement this trait to provide text layout functionality.
pub trait TextEngine {
    /// Measure the bounds of text.
    fn measure(&self, text: &TextData) -> TextMetrics;

    /// Shape text into positioned glyphs for rendering.
    fn shape(&self, text: &TextData) -> Vec<GlyphRun>;

    /// Get the font ID for a font specification.
    fn font_id(&self, family: &str, weight: u16, italic: bool) -> Option<FontId>;
}

/// A simple text engine that estimates text bounds without real font metrics.
///
/// This is useful as a fallback when no real text engine is available,
/// or for testing purposes.
#[derive(Debug, Default)]
pub struct SimpleTextEngine;

impl SimpleTextEngine {
    /// Create a new simple text engine.
    pub fn new() -> Self {
        Self
    }

    /// Estimate character width based on font size.
    /// This is a rough approximation assuming a monospace-like average.
    fn char_width(&self, font_size: f32) -> f32 {
        font_size * 0.6
    }
}

impl TextEngine for SimpleTextEngine {
    fn measure(&self, text: &TextData) -> TextMetrics {
        let char_count = text.content.chars().count() as f32;
        let width = char_count * self.char_width(text.font_size);

        TextMetrics::from_size(width, text.font_size)
    }

    fn shape(&self, text: &TextData) -> Vec<GlyphRun> {
        // Simple shaping: position each character sequentially
        let char_width = self.char_width(text.font_size);
        let mut x = 0.0;

        let glyphs: Vec<PositionedGlyph> = text
            .content
            .chars()
            .map(|c| {
                let glyph = PositionedGlyph {
                    glyph_id: c as u16,
                    position: Vec2::new(x, 0.0),
                    size: Vec2::new(char_width, text.font_size),
                };
                x += char_width;
                glyph
            })
            .collect();

        vec![GlyphRun {
            glyphs,
            font_id: FontId(0),
        }]
    }

    fn font_id(&self, _family: &str, _weight: u16, _italic: bool) -> Option<FontId> {
        Some(FontId(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_text_engine_measure() {
        let engine = SimpleTextEngine::new();
        let text = TextData::new("Hello").with_font_size(16.0);

        let metrics = engine.measure(&text);

        // 5 chars * 16.0 * 0.6 = 48.0
        assert!((metrics.width - 48.0).abs() < 0.001);
        assert!((metrics.height - 16.0).abs() < 0.001);
    }

    #[test]
    fn test_simple_text_engine_shape() {
        let engine = SimpleTextEngine::new();
        let text = TextData::new("Hi").with_font_size(20.0);

        let runs = engine.shape(&text);

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].glyphs.len(), 2);

        // First glyph at x=0
        assert!((runs[0].glyphs[0].position.x - 0.0).abs() < 0.001);

        // Second glyph at x = 20.0 * 0.6 = 12.0
        assert!((runs[0].glyphs[1].position.x - 12.0).abs() < 0.001);
    }

    #[test]
    fn test_text_metrics_bounds() {
        let metrics = TextMetrics::from_size(100.0, 16.0);
        let bounds = metrics.bounds();

        assert!((bounds.width() - 100.0).abs() < 0.001);
        assert!((bounds.height() - 16.0).abs() < 0.001);
    }
}
