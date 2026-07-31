//! Strict SVG-to-document conversion for editable vector imports.

use std::error::Error;
use std::fmt;

use editor_core::{
    Document, DocumentValidationError, Node, NodeId, Paint, PathCmd, PathData, Stroke, Style,
};
use glam::{Affine2, Mat2, Vec2};
use resvg::{tiny_skia, usvg};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const MAX_XML_NODES: u32 = 200_000;
const MAX_SVG_NESTING_DEPTH: usize = 128;
const MAX_GEOMETRY_ATTRIBUTE_BYTES: usize = 512 * 1024;
const MAX_TOTAL_GEOMETRY_BYTES: usize = 2 * 1024 * 1024;
const MAX_DOCUMENT_NODES: usize = 100_000;
const MAX_PATH_SEGMENTS: usize = 100_000;
const MAX_TOTAL_PATH_SEGMENTS: usize = 250_000;

/// Failure while converting an SVG into Strek's editable document model.
#[derive(Debug)]
pub(crate) enum SvgImportError {
    Xml(roxmltree::Error),
    InvalidRoot,
    UnsupportedElement {
        element: String,
        position: roxmltree::TextPos,
    },
    UnsupportedFeature {
        feature: String,
        element: String,
        position: Option<roxmltree::TextPos>,
    },
    Parse(usvg::Error),
    Complexity {
        resource: &'static str,
        limit: usize,
    },
    InvalidGeometry,
    Empty,
    InvalidDocument(DocumentValidationError),
    InternalStructure,
}

impl fmt::Display for SvgImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(source) => write!(formatter, "could not parse SVG XML: {source}"),
            Self::InvalidRoot => write!(formatter, "the file must have an <svg> root element"),
            Self::UnsupportedElement { element, position } => write!(
                formatter,
                "unsupported SVG element <{element}> at {position}; convert it to paths before importing"
            ),
            Self::UnsupportedFeature {
                feature,
                element,
                position,
            } => {
                write!(formatter, "unsupported SVG feature {feature} on {element}")?;
                if let Some(position) = position {
                    write!(formatter, " at {position}")?;
                }
                Ok(())
            }
            Self::Parse(source) => write!(formatter, "could not interpret SVG geometry: {source}"),
            Self::Complexity { resource, limit } => {
                write!(formatter, "the SVG exceeds the supported {resource} limit ({limit})")
            }
            Self::InvalidGeometry => write!(
                formatter,
                "the SVG contains non-finite or non-invertible geometry"
            ),
            Self::Empty => write!(formatter, "the SVG contains no supported geometry"),
            Self::InvalidDocument(source) => {
                write!(formatter, "the imported SVG exceeds document limits: {source}")
            }
            Self::InternalStructure => {
                write!(formatter, "could not construct the imported document tree")
            }
        }
    }
}

impl Error for SvgImportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Xml(source) => Some(source),
            Self::Parse(source) => Some(source),
            Self::InvalidDocument(source) => Some(source),
            Self::InvalidRoot
            | Self::UnsupportedElement { .. }
            | Self::UnsupportedFeature { .. }
            | Self::Complexity { .. }
            | Self::InvalidGeometry
            | Self::Empty
            | Self::InternalStructure => None,
        }
    }
}

/// Import the supported editable SVG subset into a new document.
pub(crate) fn import_svg(svg: &str, document_name: &str) -> Result<Document, SvgImportError> {
    let xml = roxmltree::Document::parse_with_options(
        svg,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_XML_NODES,
        },
    )
    .map_err(SvgImportError::Xml)?;
    validate_source(&xml)?;

    let tree =
        usvg::Tree::from_xmltree(&xml, &usvg::Options::default()).map_err(SvgImportError::Parse)?;
    validate_group(tree.root(), 0)?;

    let mut document = Document::new();
    let root_group = Node::group(non_empty_name(document_name, || "Imported SVG".to_owned()))
        .with_transform(import_transform(tree.root().transform())?);
    let import_root = document
        .add_child(document.root, root_group)
        .ok_or(SvgImportError::InternalStructure)?;
    let mut state = ImportState {
        // Include the native document root and the imported SVG root group.
        nodes: 2,
        ..ImportState::default()
    };
    append_children(
        &mut document,
        import_root,
        tree.root(),
        tree.root().abs_transform(),
        0,
        &mut state,
    )?;

    if state.paths == 0 {
        return Err(SvgImportError::Empty);
    }
    document
        .to_validated_saved()
        .map_err(SvgImportError::InvalidDocument)?;
    Ok(document)
}

fn validate_source(document: &roxmltree::Document<'_>) -> Result<(), SvgImportError> {
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || !matches!(root.tag_name().namespace(), None | Some(SVG_NAMESPACE))
    {
        return Err(SvgImportError::InvalidRoot);
    }

    let mut stack = vec![(root, 0_usize)];
    let mut geometry_bytes = 0_usize;
    while let Some((element, depth)) = stack.pop() {
        if depth > MAX_SVG_NESTING_DEPTH {
            return Err(SvgImportError::Complexity {
                resource: "nesting depth",
                limit: MAX_SVG_NESTING_DEPTH,
            });
        }

        let name = element.tag_name().name();
        let supported_namespace =
            matches!(element.tag_name().namespace(), None | Some(SVG_NAMESPACE));
        let supported = supported_namespace
            && matches!(
                name,
                "svg"
                    | "g"
                    | "defs"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "title"
                    | "desc"
            );
        if !supported || (name == "svg" && element != root) {
            return Err(SvgImportError::UnsupportedElement {
                element: if name == "svg" {
                    "nested svg".to_owned()
                } else {
                    name.to_owned()
                },
                position: document.text_pos_at(element.range().start),
            });
        }

        validate_feature_attributes(document, element)?;

        for attribute in element.attributes() {
            if matches!(attribute.name(), "d" | "points") {
                let bytes = attribute.value().len();
                if bytes > MAX_GEOMETRY_ATTRIBUTE_BYTES {
                    return Err(SvgImportError::Complexity {
                        resource: "bytes in one geometry attribute",
                        limit: MAX_GEOMETRY_ATTRIBUTE_BYTES,
                    });
                }
                geometry_bytes = geometry_bytes.saturating_add(bytes);
                if geometry_bytes > MAX_TOTAL_GEOMETRY_BYTES {
                    return Err(SvgImportError::Complexity {
                        resource: "total geometry bytes",
                        limit: MAX_TOTAL_GEOMETRY_BYTES,
                    });
                }
            }
        }

        let children = element
            .children()
            .filter(roxmltree::Node::is_element)
            .collect::<Vec<_>>();
        stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
    }
    Ok(())
}

fn validate_feature_attributes(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
) -> Result<(), SvgImportError> {
    for attribute in element.attributes() {
        let feature = unsupported_presentation_feature(attribute.name(), attribute.value());
        let feature = feature.or_else(|| match attribute.name() {
            "clip-path" => Some("clipping"),
            "mask" => Some("masks"),
            "filter" => Some("filters"),
            "marker" | "marker-start" | "marker-mid" | "marker-end" => Some("markers"),
            "mix-blend-mode" => Some("blend modes"),
            "isolation" => Some("group isolation"),
            "vector-effect" => Some("non-scaling strokes"),
            "style" => unsupported_style_feature(attribute.value()),
            "fill" | "stroke" if references_paint_server(attribute.value()) => {
                Some("paint servers")
            }
            _ => None,
        });
        if let Some(feature) = feature {
            return Err(SvgImportError::UnsupportedFeature {
                feature: feature.to_owned(),
                element: format!("<{}>", element.tag_name().name()),
                position: Some(document.text_pos_at(attribute.range().start)),
            });
        }
    }
    Ok(())
}

fn unsupported_style_feature(style: &str) -> Option<&'static str> {
    style.split(';').find_map(|declaration| {
        let (property, value) = declaration.split_once(':')?;
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        unsupported_presentation_feature(&property, value).or_else(|| match property.as_str() {
            "clip-path" => Some("clipping"),
            "mask" => Some("masks"),
            "filter" => Some("filters"),
            "marker" | "marker-start" | "marker-mid" | "marker-end" => Some("markers"),
            "mix-blend-mode" => Some("blend modes"),
            "isolation" => Some("group isolation"),
            "vector-effect" => Some("non-scaling strokes"),
            "fill" | "stroke" if references_paint_server(value) => Some("paint servers"),
            _ => None,
        })
    })
}

fn unsupported_presentation_feature(property: &str, value: &str) -> Option<&'static str> {
    let value = value.trim().to_ascii_lowercase();
    match property {
        "fill-rule" if value == "evenodd" => Some("even-odd fill rules"),
        "stroke-dasharray" if value != "none" => Some("dashed strokes"),
        "stroke-linecap" if matches!(value.as_str(), "round" | "square") => {
            Some("non-butt stroke caps")
        }
        "stroke-linejoin"
            if matches!(value.as_str(), "round" | "bevel" | "arcs" | "miter-clip") =>
        {
            Some("non-default stroke joins")
        }
        "paint-order" if value.split_whitespace().next() == Some("stroke") => {
            Some("stroke-before-fill paint order")
        }
        "opacity" if !is_full_opacity(&value) => Some("object/group opacity"),
        "fill" | "stroke"
            if value.starts_with("var(")
                || matches!(value.as_str(), "context-fill" | "context-stroke") =>
        {
            Some("context-dependent paints")
        }
        _ => None,
    }
}

fn is_full_opacity(value: &str) -> bool {
    if let Some(percentage) = value.strip_suffix('%') {
        percentage
            .trim()
            .parse::<f32>()
            .is_ok_and(|opacity| (opacity - 100.0).abs() <= f32::EPSILON)
    } else {
        value
            .parse::<f32>()
            .is_ok_and(|opacity| (opacity - 1.0).abs() <= f32::EPSILON)
    }
}

fn references_paint_server(value: &str) -> bool {
    value.trim_start().to_ascii_lowercase().starts_with("url(")
}

fn validate_group(group: &usvg::Group, depth: usize) -> Result<(), SvgImportError> {
    if depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "converted nesting depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
    }
    if (group.opacity().get() - 1.0).abs() > f32::EPSILON {
        return Err(unsupported_tree_feature("object/group opacity", group.id()));
    }
    if group.clip_path().is_some() {
        return Err(unsupported_tree_feature("clipping", group.id()));
    }
    if group.mask().is_some() {
        return Err(unsupported_tree_feature("masks", group.id()));
    }
    if !group.filters().is_empty() {
        return Err(unsupported_tree_feature("filters", group.id()));
    }
    if group.blend_mode() != usvg::BlendMode::Normal {
        return Err(unsupported_tree_feature("blend modes", group.id()));
    }
    if group.isolate() {
        return Err(unsupported_tree_feature("group isolation", group.id()));
    }
    import_transform(group.transform())?;

    for node in group.children() {
        match node {
            usvg::Node::Group(child) => validate_group(child, depth + 1)?,
            usvg::Node::Path(path) => validate_path(path)?,
            usvg::Node::Image(_) => {
                return Err(unsupported_tree_feature("images", node.id()));
            }
            usvg::Node::Text(_) => {
                return Err(unsupported_tree_feature("text", node.id()));
            }
        }
    }
    Ok(())
}

fn validate_path(path: &usvg::Path) -> Result<(), SvgImportError> {
    if path.paint_order() != usvg::PaintOrder::FillAndStroke {
        return Err(unsupported_tree_feature(
            "stroke-before-fill paint order",
            path.id(),
        ));
    }
    if let Some(fill) = path.fill() {
        if fill.rule() != usvg::FillRule::NonZero {
            return Err(unsupported_tree_feature("even-odd fill rules", path.id()));
        }
        validate_paint(fill.paint(), path.id())?;
    }
    if let Some(stroke) = path.stroke() {
        validate_paint(stroke.paint(), path.id())?;
        if stroke.dasharray().is_some() {
            return Err(unsupported_tree_feature("dashed strokes", path.id()));
        }
        if stroke.linecap() != usvg::LineCap::Butt {
            return Err(unsupported_tree_feature("non-butt stroke caps", path.id()));
        }
        if stroke.linejoin() != usvg::LineJoin::Miter
            || (stroke.miterlimit().get() - 4.0).abs() > f32::EPSILON
        {
            return Err(unsupported_tree_feature(
                "non-default stroke joins",
                path.id(),
            ));
        }
    }
    import_transform(path.abs_transform())?;
    Ok(())
}

fn validate_paint(paint: &usvg::Paint, id: &str) -> Result<(), SvgImportError> {
    if matches!(paint, usvg::Paint::Color(_)) {
        Ok(())
    } else {
        Err(unsupported_tree_feature("gradients or patterns", id))
    }
}

fn unsupported_tree_feature(feature: &str, id: &str) -> SvgImportError {
    let target = if id.is_empty() {
        "generated SVG node".to_owned()
    } else {
        format!("SVG node #{id}")
    };
    SvgImportError::UnsupportedFeature {
        feature: feature.to_owned(),
        element: target,
        position: None,
    }
}

#[derive(Default)]
struct ImportState {
    groups: usize,
    paths: usize,
    nodes: usize,
    total_path_segments: usize,
}

fn append_children(
    document: &mut Document,
    parent: NodeId,
    group: &usvg::Group,
    parent_absolute: tiny_skia::Transform,
    depth: usize,
    state: &mut ImportState,
) -> Result<(), SvgImportError> {
    if depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "converted nesting depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
    }

    for child in group.children() {
        state.nodes += 1;
        if state.nodes > MAX_DOCUMENT_NODES {
            return Err(SvgImportError::Complexity {
                resource: "document nodes",
                limit: MAX_DOCUMENT_NODES,
            });
        }
        match child {
            usvg::Node::Group(child_group) => {
                state.groups += 1;
                let node = Node::group(non_empty_name(child_group.id(), || {
                    format!("Group {}", state.groups)
                }))
                .with_transform(import_transform(child_group.transform())?);
                let child_id = document
                    .add_child(parent, node)
                    .ok_or(SvgImportError::InternalStructure)?;
                append_children(
                    document,
                    child_id,
                    child_group,
                    child_group.abs_transform(),
                    depth + 1,
                    state,
                )?;
            }
            usvg::Node::Path(path) => {
                state.paths += 1;
                let node = Node::shape(
                    non_empty_name(path.id(), || format!("Path {}", state.paths)),
                    import_path_data(path.data(), state)?,
                )
                .with_transform(relative_transform(parent_absolute, path.abs_transform())?)
                .with_style(import_path_style(path)?)
                .with_visible(path.is_visible());
                document
                    .add_child(parent, node)
                    .ok_or(SvgImportError::InternalStructure)?;
            }
            usvg::Node::Image(_) | usvg::Node::Text(_) => {
                return Err(SvgImportError::InternalStructure);
            }
        }
    }
    Ok(())
}

fn import_path_style(path: &usvg::Path) -> Result<Style, SvgImportError> {
    let fill = path.fill().map(import_fill).transpose()?;
    let stroke = path.stroke().map(import_stroke).transpose()?;
    Ok(Style {
        fill,
        stroke,
        opacity: 1.0,
    })
}

fn import_fill(fill: &usvg::Fill) -> Result<Paint, SvgImportError> {
    import_paint(fill.paint(), fill.opacity().get())
}

fn import_stroke(stroke: &usvg::Stroke) -> Result<Stroke, SvgImportError> {
    Ok(Stroke::new(
        stroke.width().get(),
        import_paint(stroke.paint(), stroke.opacity().get())?,
    ))
}

fn import_paint(paint: &usvg::Paint, alpha: f32) -> Result<Paint, SvgImportError> {
    let usvg::Paint::Color(color) = paint else {
        return Err(unsupported_tree_feature("gradients or patterns", ""));
    };
    Ok(Paint::rgba(
        f32::from(color.red) / 255.0,
        f32::from(color.green) / 255.0,
        f32::from(color.blue) / 255.0,
        alpha,
    ))
}

fn import_path_data(
    path: &tiny_skia::Path,
    state: &mut ImportState,
) -> Result<PathData, SvgImportError> {
    let mut commands = Vec::new();
    let mut current = Vec2::ZERO;
    let mut path_segments = 0_usize;

    for segment in path.segments() {
        path_segments += 1;
        if path_segments > MAX_PATH_SEGMENTS {
            return Err(SvgImportError::Complexity {
                resource: "segments in one path",
                limit: MAX_PATH_SEGMENTS,
            });
        }
        state.total_path_segments += 1;
        if state.total_path_segments > MAX_TOTAL_PATH_SEGMENTS {
            return Err(SvgImportError::Complexity {
                resource: "total path segments",
                limit: MAX_TOTAL_PATH_SEGMENTS,
            });
        }
        match segment {
            tiny_skia::PathSegment::MoveTo(point) => {
                current = import_point(point)?;
                commands.push(PathCmd::MoveTo(current));
            }
            tiny_skia::PathSegment::LineTo(point) => {
                current = import_point(point)?;
                commands.push(PathCmd::LineTo(current));
            }
            tiny_skia::PathSegment::QuadTo(control, end) => {
                let control = import_point(control)?;
                let end = import_point(end)?;
                let c1 = current + (control - current) * (2.0 / 3.0);
                let c2 = end + (control - end) * (2.0 / 3.0);
                commands.push(PathCmd::CubicTo { c1, c2, p: end });
                current = end;
            }
            tiny_skia::PathSegment::CubicTo(c1, c2, end) => {
                let end = import_point(end)?;
                commands.push(PathCmd::CubicTo {
                    c1: import_point(c1)?,
                    c2: import_point(c2)?,
                    p: end,
                });
                current = end;
            }
            tiny_skia::PathSegment::Close => commands.push(PathCmd::Close),
        }
    }

    if commands.is_empty() {
        Err(SvgImportError::InvalidGeometry)
    } else {
        Ok(PathData::from_commands(&commands))
    }
}

fn import_point(point: tiny_skia::Point) -> Result<Vec2, SvgImportError> {
    let point = Vec2::new(point.x, point.y);
    point
        .is_finite()
        .then_some(point)
        .ok_or(SvgImportError::InvalidGeometry)
}

fn relative_transform(
    parent_absolute: tiny_skia::Transform,
    absolute: tiny_skia::Transform,
) -> Result<Affine2, SvgImportError> {
    let parent = import_transform(parent_absolute)?;
    let determinant = parent.matrix2.determinant();
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return Err(SvgImportError::InvalidGeometry);
    }
    let relative = parent.inverse() * import_transform(absolute)?;
    (relative.matrix2.is_finite() && relative.translation.is_finite())
        .then_some(relative)
        .ok_or(SvgImportError::InvalidGeometry)
}

fn import_transform(transform: tiny_skia::Transform) -> Result<Affine2, SvgImportError> {
    if !transform.is_finite() {
        return Err(SvgImportError::InvalidGeometry);
    }
    Ok(Affine2::from_mat2_translation(
        Mat2::from_cols_array(&[transform.sx, transform.ky, transform.kx, transform.sy]),
        Vec2::new(transform.tx, transform.ty),
    ))
}

fn non_empty_name(candidate: &str, fallback: impl FnOnce() -> String) -> String {
    if candidate.trim().is_empty() {
        fallback()
    } else {
        candidate.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_core::NodeKind;

    #[test]
    fn imports_editable_shapes_solid_styles_and_group_transforms() {
        let mut document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="80" viewBox="0 0 100 80">
                <g id="logo" transform="translate(10 20)">
                    <rect id="box" x="1" y="2" width="30" height="20"
                          fill="#ff0000" fill-opacity="0.25"
                          stroke="#0000ff" stroke-opacity="0.75" stroke-width="2"/>
                    <path id="curve" d="M 0 0 Q 10 20 20 0" fill="none" stroke="#00ff00"/>
                </g>
            </svg>"##,
            "logo.svg",
        )
        .unwrap();

        let imported = document.get(document.root).unwrap().children[0];
        assert_eq!(document.get(imported).unwrap().name, "logo.svg");
        let shapes = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .filter(|node| matches!(node.kind, NodeKind::Shape(_)))
            .collect::<Vec<_>>();
        assert_eq!(shapes.len(), 2);

        let box_shape = shapes.iter().find(|node| node.name == "box").unwrap();
        assert_eq!(box_shape.style.fill, Some(Paint::rgba(1.0, 0.0, 0.0, 0.25)));
        assert_eq!(
            box_shape.style.stroke,
            Some(Stroke::new(2.0, Paint::rgba(0.0, 0.0, 1.0, 0.75)))
        );

        let box_id = document
            .descendants(document.root)
            .find(|id| document.get(*id).is_some_and(|node| node.name == "box"))
            .unwrap();
        let world = document.world_transform(box_id);
        assert!((world.translation.x - 10.0).abs() < 0.001);
        assert!((world.translation.y - 20.0).abs() < 0.001);
    }

    #[test]
    fn rejects_unsupported_elements_with_source_position() {
        let error = import_svg(
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>Hello</text></svg>",
            "text.svg",
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("unsupported SVG element <text>"));
        assert!(message.contains("1:"));
    }

    #[test]
    fn applies_the_root_view_box_transform() {
        let mut document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 10 10">
                <rect id="box" width="10" height="10" fill="#f00"/>
            </svg>"##,
            "scaled.svg",
        )
        .unwrap();
        let box_id = document
            .descendants(document.root)
            .find(|id| document.get(*id).is_some_and(|node| node.name == "box"))
            .unwrap();
        let bounds = document.world_bounds(box_id).unwrap();

        assert!((bounds.width() - 20.0).abs() < 0.001);
        assert!((bounds.height() - 20.0).abs() < 0.001);
    }

    #[test]
    fn converts_each_supported_geometry_element_to_a_path() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <path id="path" d="M0 0L5 5" stroke="#000"/>
                <rect id="rect" width="10" height="10"/>
                <circle id="circle" cx="20" cy="20" r="5"/>
                <ellipse id="ellipse" cx="35" cy="20" rx="8" ry="5"/>
                <line id="line" x1="0" y1="30" x2="10" y2="30" stroke="#000"/>
                <polyline id="polyline" points="20,30 25,35 30,30" fill="none" stroke="#000"/>
                <polygon id="polygon" points="40,30 45,35 50,30"/>
            </svg>"##,
            "geometry.svg",
        )
        .unwrap();
        let names = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .filter(|node| matches!(node.kind, NodeKind::Shape(_)))
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>();

        for expected in [
            "path", "rect", "circle", "ellipse", "line", "polyline", "polygon",
        ] {
            assert!(names.contains(&expected), "missing imported {expected}");
        }
    }

    #[test]
    fn rejects_gradients_and_advanced_strokes() {
        let gradient = import_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <defs><linearGradient id="g"/></defs>
                <rect width="10" height="10" fill="url(#g)"/>
            </svg>"#,
            "gradient.svg",
        )
        .unwrap_err();
        assert!(gradient.to_string().contains("linearGradient"));

        let dashed = import_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <path d="M0 0L10 10" fill="none" stroke="black" stroke-dasharray="2 2"/>
            </svg>"#,
            "dashed.svg",
        )
        .unwrap_err();
        let message = dashed.to_string();
        assert!(message.contains("dashed strokes"));
        assert!(message.contains("2:"));
    }

    #[test]
    fn rejects_object_opacity_but_preserves_paint_opacity() {
        let error = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <g opacity="0.5"><rect width="10" height="10" fill="#f00"/></g>
            </svg>"##,
            "opacity.svg",
        )
        .unwrap_err();

        assert!(error.to_string().contains("object/group opacity"));
    }

    #[test]
    fn rejects_excessive_nesting_before_geometry_conversion() {
        let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\">");
        svg.push_str(&"<g>".repeat(MAX_SVG_NESTING_DEPTH + 1));
        svg.push_str("<rect width=\"1\" height=\"1\"/>");
        svg.push_str(&"</g>".repeat(MAX_SVG_NESTING_DEPTH + 1));
        svg.push_str("</svg>");

        let error = import_svg(&svg, "deep.svg").unwrap_err();

        assert!(matches!(
            error,
            SvgImportError::Complexity {
                resource: "nesting depth",
                limit: MAX_SVG_NESTING_DEPTH,
            }
        ));
    }

    #[test]
    fn rejects_oversized_geometry_attributes_before_usvg_parsing() {
        let path_data = "M0 0 ".repeat(MAX_GEOMETRY_ATTRIBUTE_BYTES / 5 + 1);
        let svg =
            format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><path d=\"{path_data}\"/></svg>");

        let error = import_svg(&svg, "large.svg").unwrap_err();

        assert!(matches!(
            error,
            SvgImportError::Complexity {
                resource: "bytes in one geometry attribute",
                limit: MAX_GEOMETRY_ATTRIBUTE_BYTES,
            }
        ));
    }

    #[test]
    fn rejects_excessive_segments_during_native_path_conversion() {
        let mut builder = tiny_skia::PathBuilder::new();
        builder.move_to(0.0, 0.0);
        for x in 0..MAX_PATH_SEGMENTS {
            builder.line_to(x as f32, 1.0);
        }
        let path = builder.finish().unwrap();
        let mut state = ImportState::default();

        let error = import_path_data(&path, &mut state).unwrap_err();

        assert!(matches!(
            error,
            SvgImportError::Complexity {
                resource: "segments in one path",
                limit: MAX_PATH_SEGMENTS,
            }
        ));
    }

    #[test]
    fn rejects_documents_without_supported_geometry() {
        let error = import_svg(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><title>Empty</title></svg>",
            "empty.svg",
        )
        .unwrap_err();
        assert!(matches!(error, SvgImportError::Empty));
    }
}
