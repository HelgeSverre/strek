//! SVG Renderer - Convert display list to SVG string
//!
//! This crate provides SVG rendering for the vector editor,
//! useful for web targets and SVG export.

use std::fmt::Write;

use editor_render::{
    DisplayItem, DisplayList, LineCap, LineJoin, Paint, PathCmd, PathData, TextAlignment, TextItem,
};
use glam::{Affine2, Vec2};
use unicode_segmentation::UnicodeSegmentation;

/// Convert a display list to an SVG string.
///
/// The resulting SVG uses the given width and height as the viewport dimensions.
/// Uses two-pass rendering: document content first, then UI elements (selection, guides) on top.
pub fn to_svg_string(display_list: &DisplayList, width: f32, height: f32) -> String {
    let mut svg = String::new();
    let mut ui_svg = String::new();

    // SVG header
    write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">"#,
        width, height, width, height
    )
    .unwrap();

    // Render items (document content to svg, UI to ui_svg)
    for (index, item) in display_list.items.iter().enumerate() {
        match item {
            DisplayItem::FillPath {
                path,
                paint,
                transform,
                opacity,
            } => {
                let d = path_to_d(path);
                let fill = paint_to_css(paint);
                let matrix = transform_to_matrix(transform);

                write!(
                    svg,
                    r#"<path data-index="{}" d="{}" fill="{}" opacity="{}" transform="{}"/>"#,
                    index, d, fill, opacity, matrix
                )
                .unwrap();
            }

            DisplayItem::StrokePath {
                path,
                stroke,
                transform,
                opacity,
            } => {
                let d = path_to_d(path);
                let stroke_css = paint_to_css(&stroke.paint);
                let matrix = transform_to_matrix(transform);

                write!(
                    svg,
                    r#"<path data-index="{}" d="{}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}" opacity="{}" transform="{}"/>"#,
                    index,
                    d,
                    stroke_css,
                    stroke.width,
                    line_cap_to_css(stroke.line_cap),
                    line_join_to_css(stroke.line_join),
                    opacity,
                    matrix
                )
                .unwrap();
            }

            DisplayItem::ToolPreview {
                path,
                fill,
                stroke,
                transform,
            } => {
                write_tool_preview(&mut ui_svg, index, path, fill, stroke, transform);
            }

            DisplayItem::Text {
                text,
                transform,
                opacity,
            } => {
                write_text(&mut svg, index, text, transform, *opacity);
            }

            // UI elements go to separate buffer (rendered on top)
            DisplayItem::SnapGuide {
                start,
                end,
                horizontal: _,
            } => {
                write!(
                    ui_svg,
                    r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#ff00ff" stroke-width="1" stroke-dasharray="4,4" opacity="0.8"/>"##,
                    start.x, start.y, end.x, end.y
                )
                .unwrap();
            }

            DisplayItem::SelectionRect { min, max } => {
                let w = max.x - min.x;
                let h = max.y - min.y;
                write!(
                    ui_svg,
                    r##"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="#0066ff" stroke-width="2" stroke-dasharray="4"/>"##,
                    min.x, min.y, w, h
                )
                .unwrap();
            }
            DisplayItem::MarqueeRect { min, max } => {
                let w = max.x - min.x;
                let h = max.y - min.y;
                write!(
                    ui_svg,
                    r##"<rect x="{}" y="{}" width="{}" height="{}" fill="rgba(0,128,255,0.15)" stroke="#0080ff" stroke-width="1"/>"##,
                    min.x, min.y, w, h
                )
                .unwrap();
            }
            DisplayItem::VectorAnchor { .. }
            | DisplayItem::VectorHandle { .. }
            | DisplayItem::TextCaret { .. }
            | DisplayItem::TextSelectionRect { .. }
            | DisplayItem::TransformHandle { .. } => {}
        }
    }

    // Append UI elements at the end (on top)
    svg.push_str(&ui_svg);
    svg.push_str("</svg>");
    svg
}

/// Convert a display list to an SVG string fragment (no wrapper element).
/// Useful for embedding in an existing SVG document.
/// Uses two-pass rendering: document content first, then UI elements (selection, guides) on top.
pub fn to_svg_fragment(display_list: &DisplayList) -> String {
    let mut svg = String::new();
    let mut ui_svg = String::new();

    // First pass: render document content
    for (index, item) in display_list.items.iter().enumerate() {
        match item {
            DisplayItem::FillPath {
                path,
                paint,
                transform,
                opacity,
            } => {
                let d = path_to_d(path);
                let fill = paint_to_css(paint);
                let matrix = transform_to_matrix(transform);

                write!(
                    svg,
                    r#"<path data-index="{}" d="{}" fill="{}" opacity="{}" transform="{}"/>"#,
                    index, d, fill, opacity, matrix
                )
                .unwrap();
            }

            DisplayItem::StrokePath {
                path,
                stroke,
                transform,
                opacity,
            } => {
                let d = path_to_d(path);
                let stroke_css = paint_to_css(&stroke.paint);
                let matrix = transform_to_matrix(transform);

                write!(
                    svg,
                    r#"<path data-index="{}" d="{}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}" opacity="{}" transform="{}"/>"#,
                    index,
                    d,
                    stroke_css,
                    stroke.width,
                    line_cap_to_css(stroke.line_cap),
                    line_join_to_css(stroke.line_join),
                    opacity,
                    matrix
                )
                .unwrap();
            }

            DisplayItem::ToolPreview {
                path,
                fill,
                stroke,
                transform,
            } => {
                write_tool_preview(&mut ui_svg, index, path, fill, stroke, transform);
            }

            DisplayItem::Text {
                text,
                transform,
                opacity,
            } => {
                write_text(&mut svg, index, text, transform, *opacity);
            }

            // UI elements go to separate buffer (rendered on top)
            DisplayItem::SnapGuide {
                start,
                end,
                horizontal: _,
            } => {
                write!(
                    ui_svg,
                    r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#ff00ff" stroke-width="1" stroke-dasharray="4,4" opacity="0.8"/>"##,
                    start.x, start.y, end.x, end.y
                )
                .unwrap();
            }

            DisplayItem::SelectionRect { min, max } => {
                let width = max.x - min.x;
                let height = max.y - min.y;
                write!(
                    ui_svg,
                    r##"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="#0066ff" stroke-width="2" stroke-dasharray="4"/>"##,
                    min.x, min.y, width, height
                )
                .unwrap();
            }
            DisplayItem::MarqueeRect { min, max } => {
                let width = max.x - min.x;
                let height = max.y - min.y;
                write!(
                    ui_svg,
                    r##"<rect x="{}" y="{}" width="{}" height="{}" fill="rgba(0,128,255,0.15)" stroke="#0080ff" stroke-width="1"/>"##,
                    min.x, min.y, width, height
                )
                .unwrap();
            }
            DisplayItem::VectorAnchor { .. }
            | DisplayItem::VectorHandle { .. }
            | DisplayItem::TextCaret { .. }
            | DisplayItem::TextSelectionRect { .. }
            | DisplayItem::TransformHandle { .. } => {}
        }
    }

    // Append UI elements at the end (on top)
    svg.push_str(&ui_svg);
    svg
}

/// Convert a display list to an SVG string for export (excludes UI elements like selection/guides).
pub fn to_svg_string_export(display_list: &DisplayList, width: f32, height: f32) -> String {
    to_svg_string_export_with_view_box(display_list, Vec2::ZERO, Vec2::new(width, height))
}

/// Convert a display list to an SVG string for export using an explicit world-space view box.
///
/// This is suitable for artwork positioned away from the world origin, including content with
/// negative coordinates. UI-only display items are excluded.
pub fn to_svg_string_export_with_view_box(
    display_list: &DisplayList,
    view_min: Vec2,
    size: Vec2,
) -> String {
    let mut svg = String::new();

    // SVG header
    write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{} {} {} {}">"#,
        size.x, size.y, view_min.x, view_min.y, size.x, size.y
    )
    .unwrap();

    // Render items, skipping UI-only elements
    for (index, item) in display_list.items.iter().enumerate() {
        match item {
            DisplayItem::FillPath {
                path,
                paint,
                transform,
                opacity,
            } => {
                let d = path_to_d(path);
                let fill = paint_to_css(paint);
                let matrix = transform_to_matrix(transform);

                write!(
                    svg,
                    r#"<path data-index="{}" d="{}" fill="{}" opacity="{}" transform="{}"/>"#,
                    index, d, fill, opacity, matrix
                )
                .unwrap();
            }

            DisplayItem::StrokePath {
                path,
                stroke,
                transform,
                opacity,
            } => {
                let d = path_to_d(path);
                let stroke_css = paint_to_css(&stroke.paint);
                let matrix = transform_to_matrix(transform);

                write!(
                    svg,
                    r#"<path data-index="{}" d="{}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}" opacity="{}" transform="{}"/>"#,
                    index,
                    d,
                    stroke_css,
                    stroke.width,
                    line_cap_to_css(stroke.line_cap),
                    line_join_to_css(stroke.line_join),
                    opacity,
                    matrix
                )
                .unwrap();
            }

            DisplayItem::Text {
                text,
                transform,
                opacity,
            } => {
                write_text(&mut svg, index, text, transform, *opacity);
            }

            // Skip UI-only elements for export
            DisplayItem::ToolPreview { .. } => {}
            DisplayItem::SnapGuide { .. } => {}
            DisplayItem::SelectionRect { .. } => {}
            DisplayItem::MarqueeRect { .. } => {}
            DisplayItem::VectorAnchor { .. } => {}
            DisplayItem::VectorHandle { .. } => {}
            DisplayItem::TextCaret { .. } => {}
            DisplayItem::TextSelectionRect { .. } => {}
            DisplayItem::TransformHandle { .. } => {}
        }
    }

    svg.push_str("</svg>");
    svg
}

fn write_tool_preview(
    output: &mut String,
    index: usize,
    path: &PathData,
    fill: &Paint,
    stroke: &editor_render::Stroke,
    transform: &Affine2,
) {
    write!(
        output,
        r#"<path data-index="{}" data-editor-ui="tool-preview" d="{}" fill="{}" stroke="{}" stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}" transform="{}"/>"#,
        index,
        path_to_d(path),
        paint_to_css(fill),
        paint_to_css(&stroke.paint),
        stroke.width,
        line_cap_to_css(stroke.line_cap),
        line_join_to_css(stroke.line_join),
        transform_to_matrix(transform),
    )
    .unwrap();
}

fn write_text(
    output: &mut String,
    index: usize,
    text: &TextItem,
    transform: &Affine2,
    opacity: f32,
) {
    let resolved_lines = text.layout.as_ref().and_then(|layout| {
        layout
            .lines
            .iter()
            .map(|line| {
                text.content
                    .get(line.range.clone())
                    .map(|content| (content.to_owned(), line.x))
            })
            .collect::<Option<Vec<_>>>()
            .map(|lines| (lines, layout.line_height))
    });
    let (lines, line_height, uses_resolved_layout) = resolved_lines
        .map(|(lines, line_height)| (lines, line_height, true))
        .unwrap_or_else(|| {
            let approximate_character_width = (text.font_size * 0.6).max(1.0);
            let wrap_limit = text
                .wrap_width
                .map(|width| (width / approximate_character_width).floor().max(1.0) as usize);
            let mut lines = Vec::new();
            for explicit_line in text.content.split('\n') {
                let graphemes = explicit_line.graphemes(true).collect::<Vec<_>>();
                match wrap_limit {
                    Some(limit) if graphemes.len() > limit => {
                        lines.extend(graphemes.chunks(limit).map(|chunk| chunk.concat()));
                    }
                    _ => lines.push(explicit_line.to_owned()),
                }
            }
            let width = text.wrap_width.unwrap_or_else(|| {
                lines
                    .iter()
                    .map(|line| line.graphemes(true).count() as f32 * approximate_character_width)
                    .fold(0.0, f32::max)
            });
            let x = match text.alignment {
                TextAlignment::Left => 0.0,
                TextAlignment::Center => width * 0.5,
                TextAlignment::Right => width,
            };
            (
                lines.into_iter().map(|line| (line, x)).collect(),
                text.font_size * text.line_height.max(0.1),
                false,
            )
        });
    let anchor = if uses_resolved_layout {
        "start"
    } else {
        match text.alignment {
            TextAlignment::Left => "start",
            TextAlignment::Center => "middle",
            TextAlignment::Right => "end",
        }
    };
    let font_style = if text.font_italic { "italic" } else { "normal" };
    write!(
        output,
        r#"<text data-index="{}" xml:space="preserve" font-family="{}" font-size="{}" font-weight="{}" font-style="{}" text-anchor="{}" fill="{}" opacity="{}" transform="{}">"#,
        index,
        html_escape(&text.font_family),
        text.font_size,
        text.font_weight,
        font_style,
        anchor,
        paint_to_css(&text.fill),
        opacity,
        transform_to_matrix(transform),
    )
    .unwrap();
    for (line_index, (line, x)) in lines.iter().enumerate() {
        write!(
            output,
            r#"<tspan x="{}" y="{}">{}</tspan>"#,
            x,
            text.font_size + line_index as f32 * line_height,
            html_escape(line),
        )
        .unwrap();
    }
    output.push_str("</text>");
}

/// Convert path data to SVG `d` attribute.
pub fn path_to_d(path: &PathData) -> String {
    let mut d = String::new();

    for cmd in &path.commands {
        match cmd {
            PathCmd::MoveTo(p) => {
                write!(d, "M{:.3} {:.3} ", p.x, p.y).unwrap();
            }
            PathCmd::LineTo(p) => {
                write!(d, "L{:.3} {:.3} ", p.x, p.y).unwrap();
            }
            PathCmd::CubicTo { c1, c2, p } => {
                write!(
                    d,
                    "C{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} ",
                    c1.x, c1.y, c2.x, c2.y, p.x, p.y
                )
                .unwrap();
            }
            PathCmd::Close => {
                d.push_str("Z ");
            }
        }
    }

    d.trim_end().to_string()
}

/// Convert paint to CSS color string.
pub fn paint_to_css(paint: &Paint) -> String {
    match paint {
        Paint::Solid([r, g, b, a]) => {
            if *a >= 1.0 {
                // Use hex for fully opaque colors
                format!(
                    "#{:02x}{:02x}{:02x}",
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8
                )
            } else {
                // Use rgba for transparent colors
                format!(
                    "rgba({},{},{},{:.3})",
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8,
                    a
                )
            }
        }
    }
}

/// Convert transform to SVG matrix string.
pub fn transform_to_matrix(transform: &Affine2) -> String {
    let m = transform.matrix2;
    let t = transform.translation;

    format!(
        "matrix({:.6},{:.6},{:.6},{:.6},{:.6},{:.6})",
        m.x_axis.x, m.x_axis.y, m.y_axis.x, m.y_axis.y, t.x, t.y
    )
}

/// Convert line cap to CSS string.
fn line_cap_to_css(cap: LineCap) -> &'static str {
    match cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    }
}

/// Convert line join to CSS string.
fn line_join_to_css(join: LineJoin) -> &'static str {
    match join {
        LineJoin::Miter => "miter",
        LineJoin::Round => "round",
        LineJoin::Bevel => "bevel",
    }
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '&' => result.push_str("&amp;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#39;"),
            _ => result.push(c),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_render::{Paint, ResolvedTextLayout, ResolvedTextLine, Stroke, TextItem};
    use glam::Vec2;

    #[test]
    fn test_path_to_d_rect() {
        let path = PathData::rect(10.0, 20.0, 100.0, 50.0);
        let d = path_to_d(&path);

        assert!(d.contains("M10.000 20.000"));
        assert!(d.contains("L110.000 20.000"));
        assert!(d.contains("Z"));
    }

    #[test]
    fn test_paint_to_css_opaque() {
        let paint = Paint::Solid([1.0, 0.0, 0.0, 1.0]);
        let css = paint_to_css(&paint);
        assert_eq!(css, "#ff0000");
    }

    #[test]
    fn test_paint_to_css_transparent() {
        let paint = Paint::Solid([1.0, 0.0, 0.0, 0.5]);
        let css = paint_to_css(&paint);
        assert!(css.starts_with("rgba("));
        assert!(css.contains("0.5"));
    }

    #[test]
    fn test_transform_to_matrix_identity() {
        let transform = Affine2::IDENTITY;
        let matrix = transform_to_matrix(&transform);
        assert!(matrix.contains("matrix(1.0"));
    }

    #[test]
    fn test_transform_to_matrix_translation() {
        let transform = Affine2::from_translation(Vec2::new(100.0, 50.0));
        let matrix = transform_to_matrix(&transform);
        assert!(matrix.contains("100.0"));
        assert!(matrix.contains("50.0"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn test_to_svg_string_empty() {
        let list = DisplayList::new();
        let svg = to_svg_string(&list, 800.0, 600.0);

        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("width=\"800\""));
        assert!(svg.contains("height=\"600\""));
        assert!(svg.ends_with("</svg>"));
    }

    #[test]
    fn test_to_svg_string_with_path() {
        let mut list = DisplayList::new();
        list.push(DisplayItem::FillPath {
            path: PathData::rect(0.0, 0.0, 100.0, 100.0),
            paint: Paint::rgb(1.0, 0.0, 0.0),
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });

        let svg = to_svg_string(&list, 800.0, 600.0);

        assert!(svg.contains("<path"));
        assert!(svg.contains("fill=\"#ff0000\""));
        assert!(svg.contains("d=\"M0.000 0.000"));
    }

    #[test]
    fn test_to_svg_string_with_stroke() {
        let mut list = DisplayList::new();
        list.push(DisplayItem::StrokePath {
            path: PathData::rect(0.0, 0.0, 100.0, 100.0),
            stroke: Stroke::black(2.0),
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });

        let svg = to_svg_string(&list, 800.0, 600.0);

        assert!(svg.contains("<path"));
        assert!(svg.contains("fill=\"none\""));
        assert!(svg.contains("stroke=\"#000000\""));
        assert!(svg.contains("stroke-width=\"2\""));
    }

    #[test]
    fn tool_preview_is_visible_in_editor_svg_but_not_exported() {
        let mut list = DisplayList::new();
        list.push(DisplayItem::ToolPreview {
            path: PathData::rect(0.0, 0.0, 100.0, 100.0),
            fill: Paint::rgba(0.0, 0.5, 1.0, 0.15),
            stroke: Stroke::new(1.0, Paint::rgb(0.0, 0.5, 1.0)),
            transform: Affine2::IDENTITY,
        });

        let editor_svg = to_svg_string(&list, 800.0, 600.0);
        let exported_svg = to_svg_string_export(&list, 800.0, 600.0);

        assert!(editor_svg.contains("data-editor-ui=\"tool-preview\""));
        assert!(!exported_svg.contains("tool-preview"));
        assert!(!exported_svg.contains("<path"));
    }

    #[test]
    fn export_view_box_preserves_translated_and_negative_artwork() {
        let mut list = DisplayList::new();
        list.push(DisplayItem::FillPath {
            path: PathData::rect(0.0, 0.0, 20.0, 10.0),
            paint: Paint::black(),
            transform: Affine2::from_translation(Vec2::new(-25.0, 40.0)),
            opacity: 1.0,
        });

        let svg = to_svg_string_export_with_view_box(
            &list,
            Vec2::new(-27.0, 38.0),
            Vec2::new(24.0, 14.0),
        );

        assert!(svg.contains(r#"width="24" height="14""#));
        assert!(svg.contains(r#"viewBox="-27 38 24 14""#));
        assert!(svg.contains("matrix(1.000000,0.000000,0.000000,1.000000,-25.000000,40.000000)"));
    }

    #[test]
    fn test_to_svg_string_with_text() {
        let mut list = DisplayList::new();
        list.push(DisplayItem::Text {
            text: TextItem::new("Hello World", 16.0)
                .with_font_family("Arial")
                .with_weight(700),
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });

        let svg = to_svg_string(&list, 800.0, 600.0);

        assert!(svg.contains("<text"));
        assert!(svg.contains("font-family=\"Arial\""));
        assert!(svg.contains("font-size=\"16\""));
        assert!(svg.contains("font-weight=\"700\""));
        assert!(svg.contains("Hello World"));
    }

    #[test]
    fn resolved_text_layout_controls_svg_lines_and_alignment() {
        let mut text = TextItem::new("WWW iii", 16.0);
        text.wrap_width = Some(20.0);
        text.layout = Some(ResolvedTextLayout {
            lines: vec![
                ResolvedTextLine {
                    range: 0..1,
                    x: 3.0,
                },
                ResolvedTextLine {
                    range: 1..7,
                    x: 7.0,
                },
            ],
            line_height: 24.0,
            width: 20.0,
        });

        let mut list = DisplayList::new();
        list.push(DisplayItem::Text {
            text,
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });

        let svg = to_svg_string_export(&list, 800.0, 600.0);

        assert!(svg.contains(r#"text-anchor="start""#));
        assert!(svg.contains(r#"<tspan x="3" y="16">W</tspan>"#));
        assert!(svg.contains(r#"<tspan x="7" y="40">WW iii</tspan>"#));
    }

    #[test]
    fn test_to_svg_string_escapes_text() {
        let mut list = DisplayList::new();
        list.push(DisplayItem::Text {
            text: TextItem::new("<script>alert('xss')</script>", 16.0),
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });

        let svg = to_svg_string(&list, 800.0, 600.0);

        assert!(svg.contains("&lt;script&gt;"));
        assert!(!svg.contains("<script>"));
    }

    #[test]
    fn test_to_svg_fragment() {
        let mut list = DisplayList::new();
        list.push(DisplayItem::FillPath {
            path: PathData::rect(0.0, 0.0, 100.0, 100.0),
            paint: Paint::black(),
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });

        let fragment = to_svg_fragment(&list);

        // Fragment should not have svg wrapper
        assert!(!fragment.starts_with("<svg"));
        assert!(!fragment.ends_with("</svg>"));
        assert!(fragment.starts_with("<path"));
    }
}
