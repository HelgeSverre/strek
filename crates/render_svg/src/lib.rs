//! SVG Renderer - Convert display list to SVG string
//!
//! This crate provides SVG rendering for the vector editor,
//! useful for web targets and SVG export.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fmt::Write;
use std::hash::{Hash, Hasher};

use editor_render::{
    ClipNode, ClipPath, DisplayItem, DisplayList, FillRule, GradientStop, LineCap, LineJoin, Paint,
    PathCmd, PathData, SpreadMethod, TextAlignment, TextItem,
};
use glam::{Affine2, Vec2};
use unicode_segmentation::UnicodeSegmentation;

/// Convert a display list to an SVG string.
///
/// The resulting SVG uses the given width and height as the viewport dimensions.
/// Uses two-pass rendering: document content first, then UI elements (selection, guides) on top.
pub fn to_svg_string(display_list: &DisplayList, width: f32, height: f32) -> String {
    render_svg(
        display_list,
        Vec2::ZERO,
        Vec2::new(width, height),
        true,
        true,
    )
}

/// Convert a display list to an SVG string fragment (no wrapper element).
/// Useful for embedding in an existing SVG document.
/// Uses two-pass rendering: document content first, then UI elements (selection, guides) on top.
pub fn to_svg_fragment(display_list: &DisplayList) -> String {
    render_svg(display_list, Vec2::ZERO, Vec2::ZERO, true, false)
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
    render_svg(display_list, view_min, size, false, true)
}

fn render_svg(
    display_list: &DisplayList,
    view_min: Vec2,
    size: Vec2,
    include_ui: bool,
    wrapper: bool,
) -> String {
    let mut writer = SvgWriter::default();
    for (index, item) in display_list.items.iter().enumerate() {
        writer.write_item(index, item, include_ui);
    }

    let mut svg = String::new();
    if wrapper {
        write!(
            svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{} {} {} {}">"#,
            size.x, size.y, view_min.x, view_min.y, size.x, size.y
        )
        .unwrap();
    }
    if !writer.defs.is_empty() {
        svg.push_str("<defs>");
        svg.push_str(&writer.defs);
        svg.push_str("</defs>");
    }
    svg.push_str(&writer.body);
    if include_ui {
        svg.push_str(&writer.ui);
    }
    if wrapper {
        svg.push_str("</svg>");
    }
    svg
}

#[derive(Default)]
struct SvgWriter {
    defs: String,
    body: String,
    ui: String,
    next_resource_id: usize,
    paint_resources: HashMap<u64, Vec<CachedResource<Paint>>>,
    clip_resources: HashMap<u64, Vec<CachedResource<ClipPath>>>,
}

struct CachedResource<T> {
    value: T,
    id: String,
}

impl SvgWriter {
    fn write_item(&mut self, index: usize, item: &DisplayItem, include_ui: bool) {
        match item {
            DisplayItem::BeginGroup { opacity, clip_path } => {
                let clip = clip_path.as_ref().map(|clip| self.write_clip_path(clip));
                write!(self.body, r#"<g opacity="{}""#, opacity).unwrap();
                if let Some(clip) = clip {
                    write!(self.body, r#" clip-path="url(#{clip})""#).unwrap();
                }
                self.body.push('>');
            }
            DisplayItem::EndGroup => self.body.push_str("</g>"),
            DisplayItem::FillPath {
                path,
                paint,
                fill_rule,
                transform,
                opacity,
            } => {
                let fill = self.write_paint(paint);
                write!(
                    self.body,
                    r#"<path data-index="{}" d="{}" fill="{}" fill-rule="{}" opacity="{}" transform="{}"/>"#,
                    index,
                    path_to_d(path),
                    fill,
                    fill_rule_to_css(*fill_rule),
                    opacity,
                    transform_to_matrix(transform),
                )
                .unwrap();
            }
            DisplayItem::StrokePath {
                path,
                stroke,
                transform,
                opacity,
            } => {
                let paint = self.write_paint(&stroke.paint);
                write!(
                    self.body,
                    r#"<path data-index="{}" d="{}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}" stroke-miterlimit="{}""#,
                    index,
                    path_to_d(path),
                    paint,
                    stroke.width,
                    line_cap_to_css(stroke.line_cap),
                    line_join_to_css(stroke.line_join),
                    stroke.miter_limit,
                )
                .unwrap();
                if !stroke.dash_array.is_empty() {
                    let dash_array = stroke
                        .dash_array
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ");
                    write!(
                        self.body,
                        r#" stroke-dasharray="{}" stroke-dashoffset="{}""#,
                        dash_array, stroke.dash_offset
                    )
                    .unwrap();
                }
                write!(
                    self.body,
                    r#" opacity="{}" transform="{}"/>"#,
                    opacity,
                    transform_to_matrix(transform),
                )
                .unwrap();
            }
            DisplayItem::Text {
                text,
                transform,
                opacity,
            } => {
                let fill = self.write_paint(&text.fill);
                write_text_with_fill(&mut self.body, index, text, transform, *opacity, &fill);
            }
            DisplayItem::ToolPreview {
                path,
                fill,
                stroke,
                transform,
            } if include_ui => {
                write_tool_preview(&mut self.ui, index, path, fill, stroke, transform);
            }
            DisplayItem::SnapGuide { start, end, .. } if include_ui => {
                write!(
                    self.ui,
                    r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#ff00ff" stroke-width="1" stroke-dasharray="4,4" opacity="0.8"/>"##,
                    start.x, start.y, end.x, end.y
                )
                .unwrap();
            }
            DisplayItem::SelectionRect { min, max } if include_ui => {
                write!(
                    self.ui,
                    r##"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="#0066ff" stroke-width="2" stroke-dasharray="4"/>"##,
                    min.x,
                    min.y,
                    max.x - min.x,
                    max.y - min.y,
                )
                .unwrap();
            }
            DisplayItem::MarqueeRect { min, max } if include_ui => {
                write!(
                    self.ui,
                    r##"<rect x="{}" y="{}" width="{}" height="{}" fill="rgba(0,128,255,0.15)" stroke="#0080ff" stroke-width="1"/>"##,
                    min.x,
                    min.y,
                    max.x - min.x,
                    max.y - min.y,
                )
                .unwrap();
            }
            DisplayItem::ToolPreview { .. }
            | DisplayItem::SnapGuide { .. }
            | DisplayItem::SelectionRect { .. }
            | DisplayItem::SelectionQuad { .. }
            | DisplayItem::MarqueeRect { .. }
            | DisplayItem::VectorAnchor { .. }
            | DisplayItem::VectorHandle { .. }
            | DisplayItem::TextCaret { .. }
            | DisplayItem::TextSelectionRect { .. }
            | DisplayItem::TransformHandle { .. } => {}
        }
    }

    fn write_paint(&mut self, paint: &Paint) -> String {
        if matches!(paint, Paint::Solid(_)) {
            return paint_to_css(paint);
        }

        let hash = paint_resource_hash(paint);
        if let Some(id) = cached_resource_id(&self.paint_resources, hash, paint) {
            return format!("url(#{id})");
        }

        let id = match paint {
            Paint::Solid(_) => unreachable!("solid paints return before resource serialization"),
            Paint::LinearGradient(gradient) => {
                let id = self.resource_id("linear-gradient");
                write!(
                    self.defs,
                    r#"<linearGradient id="{}" gradientUnits="userSpaceOnUse" x1="{}" y1="{}" x2="{}" y2="{}" gradientTransform="{}" spreadMethod="{}">"#,
                    id,
                    gradient.start.x,
                    gradient.start.y,
                    gradient.end.x,
                    gradient.end.y,
                    transform_to_matrix(&gradient.transform),
                    spread_method_to_css(gradient.spread),
                )
                .unwrap();
                write_gradient_stops(&mut self.defs, &gradient.stops);
                self.defs.push_str("</linearGradient>");
                id
            }
            Paint::RadialGradient(gradient) => {
                let id = self.resource_id("radial-gradient");
                write!(
                    self.defs,
                    r#"<radialGradient id="{}" gradientUnits="userSpaceOnUse" cx="{}" cy="{}" fx="{}" fy="{}" r="{}" gradientTransform="{}" spreadMethod="{}">"#,
                    id,
                    gradient.center.x,
                    gradient.center.y,
                    gradient.focal.x,
                    gradient.focal.y,
                    gradient.radius,
                    transform_to_matrix(&gradient.transform),
                    spread_method_to_css(gradient.spread),
                )
                .unwrap();
                write_gradient_stops(&mut self.defs, &gradient.stops);
                self.defs.push_str("</radialGradient>");
                id
            }
        };
        self.paint_resources
            .entry(hash)
            .or_default()
            .push(CachedResource {
                value: paint.clone(),
                id: id.clone(),
            });
        format!("url(#{id})")
    }

    fn write_clip_path(&mut self, clip: &ClipPath) -> String {
        let hash = clip_resource_hash(clip);
        if let Some(id) = cached_resource_id(&self.clip_resources, hash, clip) {
            return id.to_owned();
        }

        let id = self.resource_id("clip");
        let contents = self.clip_contents(clip);
        write!(
            self.defs,
            r#"<clipPath id="{}" clipPathUnits="userSpaceOnUse">{}</clipPath>"#,
            id, contents
        )
        .unwrap();
        self.clip_resources
            .entry(hash)
            .or_default()
            .push(CachedResource {
                value: clip.clone(),
                id: id.clone(),
            });
        id
    }

    fn clip_contents(&mut self, clip: &ClipPath) -> String {
        let mut output = String::new();
        let nested = clip
            .clip_path
            .as_ref()
            .map(|nested| self.write_clip_path(nested));
        if let Some(ref nested) = nested {
            write!(output, r#"<g clip-path="url(#{nested})">"#).unwrap();
        }
        for child in &clip.children {
            match child {
                ClipNode::Shape(shape) => {
                    write!(
                        output,
                        r#"<path d="{}" fill-rule="{}" clip-rule="{}" transform="{}"/>"#,
                        path_to_d(&shape.path),
                        fill_rule_to_css(shape.fill_rule),
                        fill_rule_to_css(shape.fill_rule),
                        transform_to_matrix(&shape.transform),
                    )
                    .unwrap();
                }
                ClipNode::Group(group) => {
                    let contents = self.clip_contents(group);
                    write!(output, "<g>{contents}</g>").unwrap();
                }
            }
        }
        if nested.is_some() {
            output.push_str("</g>");
        }
        output
    }

    fn resource_id(&mut self, prefix: &str) -> String {
        let id = format!("strek-{prefix}-{}", self.next_resource_id);
        self.next_resource_id += 1;
        id
    }
}

fn cached_resource_id<'a, T: PartialEq>(
    resources: &'a HashMap<u64, Vec<CachedResource<T>>>,
    hash: u64,
    value: &T,
) -> Option<&'a str> {
    resources
        .get(&hash)?
        .iter()
        .find(|resource| resource.value == *value)
        .map(|resource| resource.id.as_str())
}

fn paint_resource_hash(paint: &Paint) -> u64 {
    let mut hasher = DefaultHasher::new();
    match paint {
        Paint::Solid(color) => {
            0_u8.hash(&mut hasher);
            hash_color(color, &mut hasher);
        }
        Paint::LinearGradient(gradient) => {
            1_u8.hash(&mut hasher);
            hash_vec2(gradient.start, &mut hasher);
            hash_vec2(gradient.end, &mut hasher);
            hash_affine(gradient.transform, &mut hasher);
            hash_spread_method(gradient.spread, &mut hasher);
            hash_gradient_stops(&gradient.stops, &mut hasher);
        }
        Paint::RadialGradient(gradient) => {
            2_u8.hash(&mut hasher);
            hash_vec2(gradient.center, &mut hasher);
            hash_vec2(gradient.focal, &mut hasher);
            hash_f32(gradient.radius, &mut hasher);
            hash_affine(gradient.transform, &mut hasher);
            hash_spread_method(gradient.spread, &mut hasher);
            hash_gradient_stops(&gradient.stops, &mut hasher);
        }
    }
    hasher.finish()
}

fn clip_resource_hash(clip: &ClipPath) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_clip_path(clip, &mut hasher);
    hasher.finish()
}

fn hash_clip_path(clip: &ClipPath, hasher: &mut impl Hasher) {
    clip.children.len().hash(hasher);
    for child in &clip.children {
        match child {
            ClipNode::Group(group) => {
                0_u8.hash(hasher);
                hash_clip_path(group, hasher);
            }
            ClipNode::Shape(shape) => {
                1_u8.hash(hasher);
                hash_path(&shape.path, hasher);
                hash_affine(shape.transform, hasher);
                hash_fill_rule(shape.fill_rule, hasher);
            }
        }
    }
    match &clip.clip_path {
        Some(nested) => {
            1_u8.hash(hasher);
            hash_clip_path(nested, hasher);
        }
        None => 0_u8.hash(hasher),
    }
}

fn hash_path(path: &PathData, hasher: &mut impl Hasher) {
    path.commands.len().hash(hasher);
    for command in &path.commands {
        match command {
            PathCmd::MoveTo(point) => {
                0_u8.hash(hasher);
                hash_vec2(*point, hasher);
            }
            PathCmd::LineTo(point) => {
                1_u8.hash(hasher);
                hash_vec2(*point, hasher);
            }
            PathCmd::CubicTo { c1, c2, p } => {
                2_u8.hash(hasher);
                hash_vec2(*c1, hasher);
                hash_vec2(*c2, hasher);
                hash_vec2(*p, hasher);
            }
            PathCmd::Close => 3_u8.hash(hasher),
        }
    }
}

fn hash_gradient_stops(stops: &[GradientStop], hasher: &mut impl Hasher) {
    stops.len().hash(hasher);
    for stop in stops {
        hash_f32(stop.offset, hasher);
        hash_color(&stop.color, hasher);
    }
}

fn hash_color(color: &[f32; 4], hasher: &mut impl Hasher) {
    for channel in color {
        hash_f32(*channel, hasher);
    }
}

fn hash_affine(transform: Affine2, hasher: &mut impl Hasher) {
    for value in transform.to_cols_array() {
        hash_f32(value, hasher);
    }
}

fn hash_vec2(vector: Vec2, hasher: &mut impl Hasher) {
    hash_f32(vector.x, hasher);
    hash_f32(vector.y, hasher);
}

fn hash_f32(value: f32, hasher: &mut impl Hasher) {
    // `PartialEq` treats the two signed zero representations as equal, so the
    // resource hash must do the same before collision-safe equality checks.
    let bits = if value == 0.0 { 0 } else { value.to_bits() };
    bits.hash(hasher);
}

fn hash_spread_method(spread: SpreadMethod, hasher: &mut impl Hasher) {
    match spread {
        SpreadMethod::Pad => 0_u8,
        SpreadMethod::Reflect => 1,
        SpreadMethod::Repeat => 2,
    }
    .hash(hasher);
}

fn hash_fill_rule(fill_rule: FillRule, hasher: &mut impl Hasher) {
    match fill_rule {
        FillRule::NonZero => 0_u8,
        FillRule::EvenOdd => 1,
    }
    .hash(hasher);
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

fn write_text_with_fill(
    output: &mut String,
    index: usize,
    text: &TextItem,
    transform: &Affine2,
    opacity: f32,
    fill: &str,
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
        fill,
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
        Paint::LinearGradient(_) | Paint::RadialGradient(_) => "none".to_owned(),
    }
}

fn write_gradient_stops(output: &mut String, stops: &[GradientStop]) {
    for stop in stops {
        let [red, green, blue, alpha] = stop.color;
        let color = Paint::Solid([red, green, blue, 1.0]);
        write!(
            output,
            r#"<stop offset="{}" stop-color="{}" stop-opacity="{}"/>"#,
            stop.offset,
            paint_to_css(&color),
            alpha,
        )
        .unwrap();
    }
}

fn spread_method_to_css(spread: SpreadMethod) -> &'static str {
    match spread {
        SpreadMethod::Pad => "pad",
        SpreadMethod::Reflect => "reflect",
        SpreadMethod::Repeat => "repeat",
    }
}

fn fill_rule_to_css(fill_rule: FillRule) -> &'static str {
    match fill_rule {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
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
        LineJoin::MiterClip => "miter-clip",
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
    use editor_render::{
        ClipNode, ClipPath, ClipShape, GradientStop, LineJoin, LinearGradient, Paint,
        RadialGradient, ResolvedTextLayout, ResolvedTextLine, SpreadMethod, Stroke, TextItem,
    };
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
            fill_rule: FillRule::NonZero,
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
    fn advanced_svg_styles_and_clipping_are_serialized() {
        let clip = ClipPath {
            children: vec![ClipNode::Shape(ClipShape {
                path: PathData::circle(20.0, 20.0, 15.0),
                transform: Affine2::IDENTITY,
                fill_rule: FillRule::EvenOdd,
            })],
            clip_path: None,
        };
        let mut list = DisplayList::new();
        list.push(DisplayItem::BeginGroup {
            opacity: 0.4,
            clip_path: Some(clip),
        });
        list.push(DisplayItem::FillPath {
            path: PathData::rect(0.0, 0.0, 40.0, 40.0),
            paint: Paint::LinearGradient(LinearGradient {
                start: Vec2::ZERO,
                end: Vec2::new(40.0, 0.0),
                transform: Affine2::from_translation(Vec2::new(2.0, 3.0)),
                spread: SpreadMethod::Reflect,
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: [1.0, 0.0, 0.0, 0.5],
                    },
                    GradientStop {
                        offset: 1.0,
                        color: [0.0, 0.0, 1.0, 1.0],
                    },
                ],
            }),
            fill_rule: FillRule::EvenOdd,
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });
        let mut stroke = Stroke::new(
            3.0,
            Paint::RadialGradient(RadialGradient {
                center: Vec2::new(20.0, 20.0),
                focal: Vec2::new(18.0, 18.0),
                radius: 20.0,
                transform: Affine2::IDENTITY,
                spread: SpreadMethod::Repeat,
                stops: vec![GradientStop {
                    offset: 0.0,
                    color: [0.0, 1.0, 0.0, 1.0],
                }],
            }),
        );
        stroke.line_join = LineJoin::MiterClip;
        stroke.miter_limit = 7.0;
        stroke.dash_array = vec![4.0, 2.0];
        stroke.dash_offset = 1.5;
        list.push(DisplayItem::StrokePath {
            path: PathData::rect(0.0, 0.0, 40.0, 40.0),
            stroke,
            transform: Affine2::IDENTITY,
            opacity: 1.0,
        });
        list.push(DisplayItem::EndGroup);

        let svg = to_svg_string_export(&list, 40.0, 40.0);

        assert!(svg.contains("<linearGradient"));
        assert!(svg.contains("<radialGradient"));
        assert!(svg.contains(r#"spreadMethod="reflect""#));
        assert!(svg.contains(r#"stop-opacity="0.5""#));
        assert!(svg.contains(r#"fill-rule="evenodd""#));
        assert!(svg.contains(r#"clip-rule="evenodd""#));
        assert!(svg.contains(r#"stroke-linejoin="miter-clip""#));
        assert!(svg.contains(r#"stroke-miterlimit="7""#));
        assert!(svg.contains(r#"stroke-dasharray="4 2""#));
        assert!(svg.contains(r#"stroke-dashoffset="1.5""#));
        assert!(svg.contains(r#"<g opacity="0.4" clip-path="url(#strek-clip-"#));
        assert!(svg.ends_with("</g></svg>"));
    }

    #[test]
    fn equivalent_gradients_share_definitions_and_references() {
        let linear = Paint::LinearGradient(LinearGradient {
            start: Vec2::ZERO,
            end: Vec2::new(40.0, 0.0),
            transform: Affine2::IDENTITY,
            spread: SpreadMethod::Pad,
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: [1.0, 0.0, 0.0, 1.0],
                },
                GradientStop {
                    offset: 1.0,
                    color: [0.0, 0.0, 1.0, 1.0],
                },
            ],
        });
        let radial = Paint::RadialGradient(RadialGradient {
            center: Vec2::new(20.0, 20.0),
            focal: Vec2::new(18.0, 18.0),
            radius: 20.0,
            transform: Affine2::IDENTITY,
            spread: SpreadMethod::Repeat,
            stops: vec![GradientStop {
                offset: 0.5,
                color: [0.0, 1.0, 0.0, 0.75],
            }],
        });

        let mut list = DisplayList::new();
        for y in [0.0, 50.0] {
            list.push(DisplayItem::FillPath {
                path: PathData::rect(0.0, y, 40.0, 40.0),
                paint: linear.clone(),
                fill_rule: FillRule::NonZero,
                transform: Affine2::IDENTITY,
                opacity: 1.0,
            });
        }
        for y in [100.0, 150.0] {
            list.push(DisplayItem::StrokePath {
                path: PathData::rect(0.0, y, 40.0, 40.0),
                stroke: Stroke::new(2.0, radial.clone()),
                transform: Affine2::IDENTITY,
                opacity: 1.0,
            });
        }

        let svg = to_svg_string_export(&list, 40.0, 190.0);

        assert_eq!(svg.matches("<linearGradient ").count(), 1);
        assert_eq!(svg.matches("<radialGradient ").count(), 1);
        assert_eq!(
            svg.matches(r#"fill="url(#strek-linear-gradient-0)""#)
                .count(),
            2
        );
        assert_eq!(
            svg.matches(r#"stroke="url(#strek-radial-gradient-1)""#)
                .count(),
            2
        );
    }

    #[test]
    fn equivalent_clip_paths_share_a_definition_and_reference() {
        let clip = ClipPath {
            children: vec![ClipNode::Shape(ClipShape {
                path: PathData::circle(20.0, 20.0, 15.0),
                transform: Affine2::IDENTITY,
                fill_rule: FillRule::EvenOdd,
            })],
            clip_path: None,
        };
        let mut list = DisplayList::new();
        for x in [0.0, 50.0] {
            list.push(DisplayItem::BeginGroup {
                opacity: 1.0,
                clip_path: Some(clip.clone()),
            });
            list.push(DisplayItem::FillPath {
                path: PathData::rect(x, 0.0, 40.0, 40.0),
                paint: Paint::black(),
                fill_rule: FillRule::NonZero,
                transform: Affine2::IDENTITY,
                opacity: 1.0,
            });
            list.push(DisplayItem::EndGroup);
        }

        let svg = to_svg_string_export(&list, 90.0, 40.0);

        assert_eq!(svg.matches("<clipPath ").count(), 1);
        assert_eq!(svg.matches(r#"clip-path="url(#strek-clip-0)""#).count(), 2);
    }

    #[test]
    fn distinct_gradients_and_clip_paths_keep_separate_definitions() {
        let gradient = |end_x| {
            Paint::LinearGradient(LinearGradient {
                start: Vec2::ZERO,
                end: Vec2::new(end_x, 0.0),
                transform: Affine2::IDENTITY,
                spread: SpreadMethod::Pad,
                stops: vec![GradientStop {
                    offset: 0.0,
                    color: [1.0, 0.0, 0.0, 1.0],
                }],
            })
        };
        let clip = |radius| ClipPath {
            children: vec![ClipNode::Shape(ClipShape {
                path: PathData::circle(20.0, 20.0, radius),
                transform: Affine2::IDENTITY,
                fill_rule: FillRule::NonZero,
            })],
            clip_path: None,
        };

        let mut list = DisplayList::new();
        for (paint, clip_path) in [(gradient(40.0), clip(15.0)), (gradient(80.0), clip(10.0))] {
            list.push(DisplayItem::BeginGroup {
                opacity: 1.0,
                clip_path: Some(clip_path),
            });
            list.push(DisplayItem::FillPath {
                path: PathData::rect(0.0, 0.0, 40.0, 40.0),
                paint,
                fill_rule: FillRule::NonZero,
                transform: Affine2::IDENTITY,
                opacity: 1.0,
            });
            list.push(DisplayItem::EndGroup);
        }

        let svg = to_svg_string_export(&list, 40.0, 40.0);

        assert_eq!(svg.matches("<linearGradient ").count(), 2);
        assert_eq!(svg.matches("<clipPath ").count(), 2);
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
            fill_rule: FillRule::NonZero,
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
            fill_rule: FillRule::NonZero,
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
