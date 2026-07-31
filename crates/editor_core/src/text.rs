//! Text engine abstraction for text measurement and shaping.
//!
//! This module provides traits and types for platform-agnostic text handling.
//! Platform frontends may attach their native shaper while the core keeps a
//! deterministic fallback for document bounds and tests.

use glam::Vec2;
use unicode_segmentation::UnicodeSegmentation;

use crate::node::{TextAlign, TextData};
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

#[derive(Debug, Clone, PartialEq)]
pub struct TextLayoutLine {
    pub range: std::ops::Range<usize>,
    pub x: f32,
    pub character_count: usize,
    pub hard_break: bool,
    /// UTF-8 byte boundaries and their horizontal positions relative to this line.
    pub positions: Vec<(usize, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    pub lines: Vec<TextLayoutLine>,
    pub character_width: f32,
    pub line_height: f32,
    pub width: f32,
}

pub(crate) type FallbackTextLine = TextLayoutLine;
pub(crate) type FallbackTextLayout = TextLayout;

impl TextLayout {
    pub fn position_for_byte(&self, content: &str, byte: usize) -> Vec2 {
        let byte = byte.min(content.len());
        let last_line = self.lines.len().saturating_sub(1);
        let (line_index, line) = self
            .lines
            .iter()
            .enumerate()
            .find(|(index, line)| {
                byte < line.range.end
                    || (byte == line.range.end && (line.hard_break || *index == last_line))
            })
            .or_else(|| self.lines.last().map(|line| (last_line, line)))
            .expect("fallback text layout always has a line");
        let end = byte.clamp(line.range.start, line.range.end);
        let x = line
            .positions
            .iter()
            .rev()
            .find_map(|(boundary, x)| (*boundary <= end).then_some(*x))
            .unwrap_or(0.0);
        Vec2::new(line.x + x, line_index as f32 * self.line_height)
    }

    pub fn byte_for_position(&self, _content: &str, position: Vec2) -> usize {
        let line_index = ((position.y / self.line_height).floor().max(0.0) as usize)
            .min(self.lines.len().saturating_sub(1));
        let line = &self.lines[line_index];
        let x = position.x - line.x;
        line.positions
            .iter()
            .min_by(|(_, left), (_, right)| (x - *left).abs().total_cmp(&(x - *right).abs()))
            .map_or(line.range.end, |(byte, _)| *byte)
    }

    pub fn range_rects(&self, _content: &str, range: std::ops::Range<usize>) -> Vec<Rect> {
        if range.is_empty() {
            return Vec::new();
        }
        self.lines
            .iter()
            .enumerate()
            .filter_map(|(line_index, line)| {
                let start = range.start.max(line.range.start).min(line.range.end);
                let end = range.end.max(start).min(line.range.end);
                let includes_break =
                    line.hard_break && range.start <= line.range.end && range.end > line.range.end;
                if start == end && !includes_break {
                    return None;
                }
                let x_for_byte = |byte| {
                    line.x
                        + line
                            .positions
                            .iter()
                            .rev()
                            .find_map(|(boundary, x)| (*boundary <= byte).then_some(*x))
                            .unwrap_or(0.0)
                };
                let start_x = x_for_byte(start);
                let end_x = x_for_byte(end);
                let width = (end_x - start_x).max(if includes_break {
                    self.character_width * 0.5
                } else {
                    1.0
                });
                Some(Rect::from_pos_size(
                    Vec2::new(start_x, line_index as f32 * self.line_height),
                    Vec2::new(width, self.line_height),
                ))
            })
            .collect()
    }

    pub fn height(&self) -> f32 {
        self.lines.len().max(1) as f32 * self.line_height
    }
}

pub(crate) fn fallback_text_layout(text: &TextData) -> FallbackTextLayout {
    let character_width = (text.font_size * 0.6).max(f32::EPSILON);
    let line_height = text.font_size * text.line_height.max(0.1);
    let wrap_limit = text
        .fixed_width()
        .map(|width| (width / character_width).floor().max(1.0) as usize);
    let mut lines = Vec::new();
    let mut line_start = 0usize;
    let mut character_count = 0usize;
    let mut just_wrapped = false;

    for (byte, grapheme) in text.content.grapheme_indices(true) {
        if grapheme == "\n" {
            if character_count > 0 || !just_wrapped {
                lines.push(FallbackTextLine {
                    range: line_start..byte,
                    x: 0.0,
                    character_count,
                    hard_break: true,
                    positions: Vec::new(),
                });
            } else if let Some(line) = lines.last_mut() {
                line.hard_break = true;
            }
            line_start = byte + grapheme.len();
            character_count = 0;
            just_wrapped = false;
            continue;
        }

        character_count += 1;
        just_wrapped = false;
        if wrap_limit.is_some_and(|limit| character_count == limit) {
            let end = byte + grapheme.len();
            lines.push(FallbackTextLine {
                range: line_start..end,
                x: 0.0,
                character_count,
                hard_break: false,
                positions: Vec::new(),
            });
            line_start = end;
            character_count = 0;
            just_wrapped = true;
        }
    }

    if character_count > 0 || lines.is_empty() || !just_wrapped {
        lines.push(FallbackTextLine {
            range: line_start..text.content.len(),
            x: 0.0,
            character_count,
            hard_break: false,
            positions: Vec::new(),
        });
    }

    let content_width = lines
        .iter()
        .map(|line| line.character_count as f32 * character_width)
        .fold(character_width, f32::max);
    let width = text.fixed_width().unwrap_or(content_width);
    for line in &mut lines {
        let line_width = line.character_count as f32 * character_width;
        line.x = match text.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (width - line_width) * 0.5,
            TextAlign::Right => width - line_width,
        };
        line.positions = std::iter::once((line.range.start, 0.0))
            .chain(
                text.content[line.range.clone()]
                    .grapheme_indices(true)
                    .enumerate()
                    .map(|(index, (byte, grapheme))| {
                        (
                            line.range.start + byte + grapheme.len(),
                            (index + 1) as f32 * character_width,
                        )
                    }),
            )
            .collect();
    }

    FallbackTextLayout {
        lines,
        character_width,
        line_height,
        width,
    }
}

/// Deterministic fallback metrics used before a platform layout engine is attached.
pub fn estimate_text_metrics(text: &TextData) -> TextMetrics {
    let layout = fallback_text_layout(text);
    let height = layout.lines.len().max(1) as f32 * layout.line_height;
    TextMetrics {
        width: layout.width,
        height,
        ascent: text.font_size * 0.8,
        descent: text.font_size * 0.2,
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
}

impl TextEngine for SimpleTextEngine {
    fn measure(&self, text: &TextData) -> TextMetrics {
        estimate_text_metrics(text)
    }

    fn shape(&self, text: &TextData) -> Vec<GlyphRun> {
        let layout = fallback_text_layout(text);
        let character_width = layout.character_width;
        let line_height = layout.line_height;
        let glyphs = layout
            .lines
            .iter()
            .enumerate()
            .flat_map(|(line_index, line)| {
                text.content[line.range.clone()]
                    .graphemes(true)
                    .enumerate()
                    .map(move |(column, grapheme)| PositionedGlyph {
                        glyph_id: grapheme.chars().next().unwrap_or_default() as u16,
                        position: Vec2::new(
                            line.x + column as f32 * character_width,
                            line_index as f32 * line_height,
                        ),
                        size: Vec2::new(character_width, text.font_size),
                    })
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
        assert!((metrics.height - 19.2).abs() < 0.001);
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

    #[test]
    fn fallback_measurement_is_unicode_and_multiline_safe() {
        let engine = SimpleTextEngine::new();
        let text = TextData::new("é🙂\nabc").with_font_size(10.0);

        let metrics = engine.measure(&text);

        assert_eq!(metrics.width, 18.0);
        assert_eq!(metrics.height, 24.0);
    }

    #[test]
    fn fixed_width_wraps_fallback_metrics() {
        let engine = SimpleTextEngine::new();
        let text = TextData::new("abcdef")
            .with_font_size(10.0)
            .with_fixed_width(18.0);

        let metrics = engine.measure(&text);

        assert_eq!(metrics.width, 18.0);
        assert_eq!(metrics.height, 24.0);
    }

    #[test]
    fn aligned_wrapped_layout_round_trips_points_and_byte_offsets() {
        let mut text = TextData::new("abcd")
            .with_font_size(10.0)
            .with_fixed_width(18.0);
        text.align = TextAlign::Right;
        let layout = fallback_text_layout(&text);

        assert_eq!(layout.lines.len(), 2);
        assert_eq!(
            layout.position_for_byte(&text.content, 3),
            Vec2::new(12.0, 12.0)
        );
        for byte in [0, 1, 2, 3, 4] {
            let position = layout.position_for_byte(&text.content, byte);
            assert_eq!(layout.byte_for_position(&text.content, position), byte);
        }
    }

    #[test]
    fn exact_wrap_followed_by_newline_does_not_create_an_extra_visual_line() {
        let text = TextData::new("abc\ndef")
            .with_font_size(10.0)
            .with_fixed_width(18.0);

        let layout = fallback_text_layout(&text);

        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.lines[0].range, 0..3);
        assert_eq!(layout.lines[1].range, 4..7);
    }

    #[test]
    fn simple_shaping_matches_wrapped_multiline_layout() {
        let engine = SimpleTextEngine::new();
        let text = TextData::new("abc\nde")
            .with_font_size(10.0)
            .with_fixed_width(18.0);

        let glyphs = &engine.shape(&text)[0].glyphs;

        assert_eq!(glyphs.len(), 5);
        assert_eq!(glyphs[2].position, Vec2::new(12.0, 0.0));
        assert_eq!(glyphs[3].position, Vec2::new(0.0, 12.0));
    }

    #[test]
    fn fallback_layout_wraps_at_grapheme_boundaries() {
        let text = TextData::new("e\u{301}👨‍👩‍👧‍👦x")
            .with_font_size(10.0)
            .with_fixed_width(12.0);
        let layout = fallback_text_layout(&text);
        let second_grapheme = "e\u{301}".len();
        let third_grapheme = second_grapheme + "👨‍👩‍👧‍👦".len();

        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.lines[0].range, 0..third_grapheme);
        assert_eq!(layout.lines[0].character_count, 2);
        assert_eq!(
            layout.byte_for_position(&text.content, Vec2::new(6.0, 0.0)),
            second_grapheme
        );
        assert_eq!(
            layout.position_for_byte(&text.content, third_grapheme),
            Vec2::new(0.0, 12.0)
        );
    }

    #[test]
    fn shaped_layout_uses_variable_advances_for_caret_and_ranges() {
        let content = "iW";
        let layout = TextLayout {
            lines: vec![TextLayoutLine {
                range: 0..content.len(),
                x: 4.0,
                character_count: 2,
                hard_break: false,
                positions: vec![(0, 0.0), (1, 3.0), (2, 15.0)],
            }],
            character_width: 7.5,
            line_height: 12.0,
            width: 23.0,
        };

        assert_eq!(layout.position_for_byte(content, 1), Vec2::new(7.0, 0.0));
        assert_eq!(layout.byte_for_position(content, Vec2::new(8.0, 0.0)), 1);
        assert_eq!(
            layout.range_rects(content, 1..2),
            [Rect::from_pos_size(
                Vec2::new(7.0, 0.0),
                Vec2::new(12.0, 12.0)
            )]
        );
    }
}
