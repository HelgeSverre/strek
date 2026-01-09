//! SVG Renderer - Convert display list to SVG string
//!
//! This crate provides SVG rendering for the vector editor,
//! useful for web targets and SVG export.

use std::fmt::Write;

use editor_render::{DisplayItem, DisplayList, LineCap, LineJoin, Paint, PathCmd, PathData};
use glam::Affine2;

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

            DisplayItem::Text {
                text,
                transform,
                opacity,
            } => {
                let fill = paint_to_css(&text.fill);
                let matrix = transform_to_matrix(transform);
                let font_style = if text.font_italic { "italic" } else { "normal" };
                let escaped_content = html_escape(&text.content);

                write!(
                    svg,
                    r#"<text data-index="{}" font-family="{}" font-size="{}" font-weight="{}" font-style="{}" fill="{}" opacity="{}" transform="{}">{}</text>"#,
                    index,
                    text.font_family,
                    text.font_size,
                    text.font_weight,
                    font_style,
                    fill,
                    opacity,
                    matrix,
                    escaped_content
                )
                .unwrap();
            }

            DisplayItem::BeginClip { path, transform } => {
                let d = path_to_d(path);
                let matrix = transform_to_matrix(transform);
                write!(
                    svg,
                    r#"<defs><clipPath id="clip-{}"><path d="{}" transform="{}"/></clipPath></defs><g clip-path="url(#clip-{})">"#,
                    index, d, matrix, index
                )
                .unwrap();
            }

            DisplayItem::EndClip => {
                svg.push_str("</g>");
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

            DisplayItem::Text {
                text,
                transform,
                opacity,
            } => {
                let fill = paint_to_css(&text.fill);
                let matrix = transform_to_matrix(transform);
                let font_style = if text.font_italic { "italic" } else { "normal" };
                let escaped_content = html_escape(&text.content);

                write!(
                    svg,
                    r#"<text data-index="{}" font-family="{}" font-size="{}" font-weight="{}" font-style="{}" fill="{}" opacity="{}" transform="{}">{}</text>"#,
                    index,
                    text.font_family,
                    text.font_size,
                    text.font_weight,
                    font_style,
                    fill,
                    opacity,
                    matrix,
                    escaped_content
                )
                .unwrap();
            }

            DisplayItem::BeginClip { path, transform } => {
                let d = path_to_d(path);
                let matrix = transform_to_matrix(transform);
                write!(
                    svg,
                    r#"<defs><clipPath id="clip-{}"><path d="{}" transform="{}"/></clipPath></defs><g clip-path="url(#clip-{})">"#,
                    index, d, matrix, index
                )
                .unwrap();
            }

            DisplayItem::EndClip => {
                svg.push_str("</g>");
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
        }
    }

    // Append UI elements at the end (on top)
    svg.push_str(&ui_svg);
    svg
}

/// Convert a display list to an SVG string for export (excludes UI elements like selection/guides).
pub fn to_svg_string_export(display_list: &DisplayList, width: f32, height: f32) -> String {
    let mut svg = String::new();

    // SVG header
    write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">"#,
        width, height, width, height
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
                let fill = paint_to_css(&text.fill);
                let matrix = transform_to_matrix(transform);
                let font_style = if text.font_italic { "italic" } else { "normal" };
                let escaped_content = html_escape(&text.content);

                write!(
                    svg,
                    r#"<text data-index="{}" font-family="{}" font-size="{}" font-weight="{}" font-style="{}" fill="{}" opacity="{}" transform="{}">{}</text>"#,
                    index,
                    text.font_family,
                    text.font_size,
                    text.font_weight,
                    font_style,
                    fill,
                    opacity,
                    matrix,
                    escaped_content
                )
                .unwrap();
            }

            DisplayItem::BeginClip { path, transform } => {
                let d = path_to_d(path);
                let matrix = transform_to_matrix(transform);
                write!(
                    svg,
                    r#"<defs><clipPath id="clip-{}"><path d="{}" transform="{}"/></clipPath></defs><g clip-path="url(#clip-{})">"#,
                    index, d, matrix, index
                )
                .unwrap();
            }

            DisplayItem::EndClip => {
                svg.push_str("</g>");
            }

            // Skip UI-only elements for export
            DisplayItem::SnapGuide { .. } => {}
            DisplayItem::SelectionRect { .. } => {}
        }
    }

    svg.push_str("</svg>");
    svg
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
    use editor_render::{Paint, Stroke, TextItem};
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
