//! Strict SVG-to-document conversion for editable vector imports.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use editor_core::{
    ClipNode, ClipPath, ClipShape, Document, DocumentValidationError, FillRule, GradientStop,
    LineCap, LineJoin, LinearGradient, Node, NodeId, Paint, PaintOrder, PathCmd, PathData,
    RadialGradient, SpreadMethod, Stroke, Style,
};
use glam::{Affine2, Mat2, Vec2};
use resvg::{tiny_skia, usvg};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const MAX_XML_NODES: u32 = 200_000;
const MAX_SVG_NESTING_DEPTH: usize = 128;
const MAX_GEOMETRY_ATTRIBUTE_BYTES: usize = 512 * 1024;
const MAX_TOTAL_GEOMETRY_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_TEXT_BYTES: usize = 512 * 1024;
const MAX_SOURCE_LIST_VALUES: usize = 4_096;
const MAX_TOTAL_SOURCE_LIST_VALUES: usize = 250_000;
const MAX_PRE_NORMALIZATION_WORK: usize = 250_000;
const TEXT_OUTLINE_WORK_PER_CHARACTER: usize = 32;
const MAX_DOCUMENT_NODES: usize = 100_000;
const MAX_PATH_SEGMENTS: usize = 100_000;
const MAX_TOTAL_PATH_SEGMENTS: usize = 250_000;
const MAX_GRADIENT_STOPS: usize = 4_096;
const MAX_TOTAL_GRADIENT_STOPS: usize = 250_000;
const MAX_STROKE_DASH_VALUES: usize = 4_096;
const MAX_TOTAL_STROKE_DASH_VALUES: usize = 250_000;

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
    MalformedCss {
        detail: &'static str,
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
            Self::MalformedCss { detail } => write!(formatter, "malformed SVG CSS: {detail}"),
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
            | Self::MalformedCss { .. }
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

    let options = import_options();
    let tree = usvg::Tree::from_xmltree(&xml, &options).map_err(SvgImportError::Parse)?;
    validate_group(tree.root(), 0)?;

    let mut document = Document::new();
    let size = tree.size();
    let mut root_frame = Node::frame(
        non_empty_name(document_name, || "Imported SVG".to_owned()),
        size.width(),
        size.height(),
    );
    root_frame
        .frame_data_mut()
        .ok_or(SvgImportError::InternalStructure)?
        .background = None;
    let import_root = document
        .add_child(document.root, root_frame)
        .ok_or(SvgImportError::InternalStructure)?;
    let mut state = ImportState {
        // Include the native document root and imported frame. Clip geometry is counted below.
        nodes: 2,
        ..ImportState::default()
    };
    let mut content_group = Node::group("SVG content")
        .with_transform(import_transform(tree.root().transform())?)
        .with_style(import_group_style(tree.root()));
    content_group.clip_path = tree
        .root()
        .clip_path()
        .map(|clip| import_clip_path(clip, 0, &mut state))
        .transpose()?;
    let content_root = document
        .add_child(import_root, content_group)
        .ok_or(SvgImportError::InternalStructure)?;
    increment_import_node(&mut state)?;
    append_children(
        &mut document,
        content_root,
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

fn import_options() -> usvg::Options<'static> {
    usvg::Options {
        fontdb: crate::typography::system_font_database(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    }
}

fn validate_source(document: &roxmltree::Document<'_>) -> Result<(), SvgImportError> {
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || !matches!(root.tag_name().namespace(), None | Some(SVG_NAMESPACE))
    {
        return Err(SvgImportError::InvalidRoot);
    }
    validate_metadata_observable_descendants(document)?;

    let mut stack = vec![(root, 0_usize, SourceTextContext::General)];
    let mut elements = Vec::new();
    let mut ids = HashMap::new();
    let mut geometry_bytes = 0_usize;
    let mut text_bytes = 0_usize;
    let mut source_lists = SourceListBudget::default();
    let mut css_sources = CssSources::default();
    let mut source_text = SourceTextStats::default();
    while let Some((element, depth, text_context)) = stack.pop() {
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
                    | "metadata"
                    | "style"
                    | "a"
                    | "switch"
                    | "use"
                    | "symbol"
                    | "marker"
                    | "linearGradient"
                    | "radialGradient"
                    | "stop"
                    | "clipPath"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "text"
                    | "tspan"
                    | "textPath"
                    | "title"
                    | "desc"
            );
        if !supported {
            return Err(SvgImportError::UnsupportedElement {
                element: name.to_owned(),
                position: document.text_pos_at(element.range().start),
            });
        }

        elements.push(element);
        if let Some(attribute) = element
            .attributes()
            .find(|attribute| attribute.name() == "id" && attribute.namespace().is_none())
        {
            let id = attribute.value().trim();
            if !id.is_empty() && ids.insert(id.to_owned(), element).is_some() {
                return Err(source_feature_error(
                    document,
                    element,
                    Some(attribute.range().start),
                    format!("duplicate id #{id}"),
                ));
            }
        }

        let inline_style = element
            .attributes()
            .find(|attribute| attribute.name() == "style")
            .map(|attribute| {
                css_without_comments(
                    document,
                    element,
                    attribute.range().start,
                    attribute.value(),
                )
            })
            .transpose()?;
        let style_sheet = (name == "style")
            .then(|| {
                css_without_comments(
                    document,
                    element,
                    element.range().start,
                    element.text().unwrap_or(""),
                )
            })
            .transpose()?;

        if let Some(style_sheet) = style_sheet.as_deref() {
            if has_external_url_reference(style_sheet)? {
                return Err(SvgImportError::UnsupportedFeature {
                    feature: "external resource references".to_owned(),
                    element: "<style>".to_owned(),
                    position: Some(document.text_pos_at(element.range().start)),
                });
            }
            if let Some(feature) = unsupported_style_sheet_feature(style_sheet)? {
                return Err(SvgImportError::UnsupportedFeature {
                    feature: feature.to_owned(),
                    element: "<style>".to_owned(),
                    position: Some(document.text_pos_at(element.range().start)),
                });
            }
            source_lists.charge_style_sheet(style_sheet)?;
        }

        validate_feature_attributes(document, element, inline_style.as_deref())?;

        for attribute in element.attributes() {
            if attribute.name() == "style" {
                source_lists.charge_declarations(
                    inline_style
                        .as_deref()
                        .ok_or(SvgImportError::InternalStructure)?,
                )?;
            } else {
                source_lists.charge_attribute(name, attribute.name(), attribute.value())?;
            }
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

        if let Some(inline_style) = inline_style {
            css_sources.inline_styles.insert(element.id(), inline_style);
        }
        if let Some(style_sheet) = style_sheet {
            css_sources.style_sheets.insert(element.id(), style_sheet);
        }

        if text_context == SourceTextContext::Active {
            let (bytes, characters) = element
                .children()
                .filter(roxmltree::Node::is_text)
                .filter_map(|child| child.text())
                .fold((0_usize, 0_usize), |(bytes, characters), text| {
                    (
                        bytes.saturating_add(text.len()),
                        characters.saturating_add(text.chars().count()),
                    )
                });
            text_bytes = text_bytes.saturating_add(bytes);
            if text_bytes > MAX_TOTAL_TEXT_BYTES {
                return Err(SvgImportError::Complexity {
                    resource: "text bytes",
                    limit: MAX_TOTAL_TEXT_BYTES,
                });
            }
            source_text.characters.insert(element.id(), characters);
        }

        // Metadata is explicitly non-rendering and can contain arbitrary foreign XML.
        if name == "metadata" {
            continue;
        }

        let children = element
            .children()
            .filter(roxmltree::Node::is_element)
            .collect::<Vec<_>>();
        stack.extend(children.into_iter().rev().map(|child| {
            (
                child,
                depth + 1,
                text_context.for_child(name, child.tag_name().name()),
            )
        }));
    }

    let references = validate_local_references(document, &elements, &ids, &css_sources)?;
    validate_reference_cycles(document, &ids, &references)?;
    let expansion = collect_style_expansion_work(
        document,
        root,
        &elements,
        &ids,
        &css_sources,
        &references,
        source_text,
    )?;
    validate_pre_normalization_work(document, root, &references, &expansion)?;
    Ok(())
}

fn validate_metadata_observable_descendants(
    document: &roxmltree::Document<'_>,
) -> Result<(), SvgImportError> {
    let mut stack = vec![(document.root_element(), 0_usize, false)];
    while let Some((element, depth, inside_metadata)) = stack.pop() {
        if depth > MAX_SVG_NESTING_DEPTH {
            return Err(SvgImportError::Complexity {
                resource: "nesting depth",
                limit: MAX_SVG_NESTING_DEPTH,
            });
        }
        if inside_metadata {
            let id = element
                .attributes()
                .find(|attribute| attribute.name() == "id" && attribute.namespace().is_none());
            if element.tag_name().name() == "style" || id.is_some() {
                let (feature, position) = if let Some(id) = id {
                    (
                        "SVG IDs inside <metadata>".to_owned(),
                        Some(id.range().start),
                    )
                } else {
                    (
                        "style sheets inside <metadata>".to_owned(),
                        Some(element.range().start),
                    )
                };
                return Err(source_feature_error(document, element, position, feature));
            }
        }

        let children_inside_metadata = inside_metadata
            || element.tag_name().name() == "metadata"
                && matches!(element.tag_name().namespace(), None | Some(SVG_NAMESPACE));
        let children = element
            .children()
            .filter(roxmltree::Node::is_element)
            .collect::<Vec<_>>();
        stack.extend(
            children
                .into_iter()
                .rev()
                .map(|child| (child, depth + 1, children_inside_metadata)),
        );
    }
    Ok(())
}

#[derive(Default)]
struct CssSources {
    inline_styles: HashMap<roxmltree::NodeId, String>,
    style_sheets: HashMap<roxmltree::NodeId, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceTextContext {
    General,
    Active,
    Suppressed,
}

impl SourceTextContext {
    fn for_child(self, parent: &str, child: &str) -> Self {
        match self {
            Self::General if child == "text" => Self::Active,
            Self::General => Self::General,
            Self::Active
                if matches!(child, "tspan" | "a") || (child == "textPath" && parent == "text") =>
            {
                Self::Active
            }
            Self::Active | Self::Suppressed => Self::Suppressed,
        }
    }
}

#[derive(Default)]
struct SourceTextStats {
    characters: HashMap<roxmltree::NodeId, usize>,
}

fn css_without_comments(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    byte_position: usize,
    css: &str,
) -> Result<String, SvgImportError> {
    let bytes = css.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            output.push(byte);
            index += 1;
            if byte == b'\\' && index < bytes.len() {
                output.push(bytes[index]);
                index += 1;
            } else if byte == delimiter {
                quote = None;
            }
            continue;
        }

        if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
            output.push(byte);
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            let comment_start = index;
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            if index + 1 >= bytes.len() {
                return Err(source_feature_error(
                    document,
                    element,
                    Some(byte_position.saturating_add(comment_start)),
                    "unterminated CSS comment".to_owned(),
                ));
            }
            output.push(b' ');
            index += 2;
            continue;
        }
        output.push(byte);
        index += 1;
    }

    String::from_utf8(output).map_err(|_| SvgImportError::InternalStructure)
}

#[derive(Clone, Copy)]
enum SourceListSyntax {
    Numbers,
    Tokens,
}

#[derive(Default)]
struct SourceListBudget {
    total_values: usize,
}

impl SourceListBudget {
    fn charge_attribute(
        &mut self,
        element: &str,
        attribute: &str,
        value: &str,
    ) -> Result<(), SvgImportError> {
        let syntax = match attribute {
            "stroke-dasharray" | "transform" | "transform-origin" | "gradientTransform"
            | "viewBox" => Some(SourceListSyntax::Numbers),
            "points" if matches!(element, "polyline" | "polygon") => {
                Some(SourceListSyntax::Numbers)
            }
            "x" | "y" | "dx" | "dy" | "rotate"
                if matches!(element, "text" | "tspan" | "textPath") =>
            {
                Some(SourceListSyntax::Numbers)
            }
            "class"
            | "font"
            | "font-family"
            | "font-feature-settings"
            | "font-variation-settings"
            | "font-variant"
            | "font-variant-caps"
            | "font-variant-east-asian"
            | "font-variant-ligatures"
            | "font-variant-numeric"
            | "paint-order"
            | "requiredExtensions"
            | "requiredFeatures"
            | "systemLanguage"
            | "text-decoration" => Some(SourceListSyntax::Tokens),
            _ => None,
        };
        self.charge(value, syntax)
    }

    fn charge_declarations(&mut self, declarations: &str) -> Result<(), SvgImportError> {
        for_each_css_declaration(declarations, |property, value, _| {
            self.charge(value, css_list_syntax(property))
        })
    }

    fn charge_style_sheet(&mut self, style_sheet: &str) -> Result<(), SvgImportError> {
        for_each_css_rule(style_sheet, |_, declarations| {
            self.charge_declarations(declarations)
        })
    }

    fn charge(
        &mut self,
        value: &str,
        syntax: Option<SourceListSyntax>,
    ) -> Result<(), SvgImportError> {
        let Some(syntax) = syntax else {
            return Ok(());
        };
        let values = match syntax {
            SourceListSyntax::Numbers => count_svg_numbers(value, MAX_SOURCE_LIST_VALUES + 1),
            SourceListSyntax::Tokens => count_list_tokens(value, MAX_SOURCE_LIST_VALUES + 1),
        };
        if values > MAX_SOURCE_LIST_VALUES {
            return Err(SvgImportError::Complexity {
                resource: "values in one source list",
                limit: MAX_SOURCE_LIST_VALUES,
            });
        }
        self.total_values = self.total_values.saturating_add(values);
        if self.total_values > MAX_TOTAL_SOURCE_LIST_VALUES {
            return Err(SvgImportError::Complexity {
                resource: "total source list values",
                limit: MAX_TOTAL_SOURCE_LIST_VALUES,
            });
        }
        Ok(())
    }
}

fn css_list_syntax(property: &str) -> Option<SourceListSyntax> {
    match property.trim().to_ascii_lowercase().as_str() {
        "clip" | "dx" | "dy" | "rotate" | "stroke-dasharray" | "transform" | "transform-origin"
        | "x" | "y" => Some(SourceListSyntax::Numbers),
        "font"
        | "font-family"
        | "font-feature-settings"
        | "font-variation-settings"
        | "font-variant"
        | "font-variant-caps"
        | "font-variant-east-asian"
        | "font-variant-ligatures"
        | "font-variant-numeric"
        | "paint-order"
        | "text-decoration" => Some(SourceListSyntax::Tokens),
        _ => None,
    }
}

fn count_list_tokens(value: &str, stop_after: usize) -> usize {
    value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|token| !token.is_empty())
        .take(stop_after)
        .count()
}

fn count_svg_numbers(value: &str, stop_after: usize) -> usize {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut count = 0;
    while index < bytes.len() && count < stop_after {
        let start = index;
        if matches!(bytes[index], b'+' | b'-') {
            index += 1;
        }

        let mut digits = 0;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            digits += 1;
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b'.' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                digits += 1;
                index += 1;
            }
        }
        if digits == 0 {
            index = start + 1;
            continue;
        }

        if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
            let exponent_start = index;
            index += 1;
            if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
                index += 1;
            }
            let exponent_digits_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if exponent_digits_start == index {
                index = exponent_start;
            }
        }
        count += 1;
    }
    count
}

const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkerPlacement {
    Start,
    Mid,
    End,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalReferenceKind {
    Use,
    TextPath,
    Gradient,
    Paint,
    ClipPath,
    Marker(MarkerPlacement),
}

impl LocalReferenceKind {
    fn description(self) -> &'static str {
        match self {
            Self::Use => "use",
            Self::TextPath => "text path",
            Self::Gradient => "gradient inheritance",
            Self::Paint => "paint server",
            Self::ClipPath => "clipping path",
            Self::Marker(_) => "marker",
        }
    }

    fn expected_target(self) -> &'static str {
        match self {
            Self::Use => "a renderable SVG element",
            Self::TextPath => "a <path>",
            Self::Gradient | Self::Paint => "a <linearGradient> or <radialGradient>",
            Self::ClipPath => "a <clipPath>",
            Self::Marker(_) => "a <marker>",
        }
    }

    fn accepts(self, element_name: &str) -> bool {
        match self {
            Self::Use => matches!(
                element_name,
                "svg"
                    | "g"
                    | "a"
                    | "switch"
                    | "use"
                    | "symbol"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "text"
            ),
            Self::TextPath => element_name == "path",
            Self::Gradient | Self::Paint => {
                matches!(element_name, "linearGradient" | "radialGradient")
            }
            Self::ClipPath => element_name == "clipPath",
            Self::Marker(_) => element_name == "marker",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LocalReference {
    target: roxmltree::NodeId,
    kind: LocalReferenceKind,
}

#[derive(Default)]
struct LocalReferences {
    by_source: HashMap<roxmltree::NodeId, Vec<LocalReference>>,
    count: usize,
    css_selector_work: usize,
}

impl LocalReferences {
    fn push(
        &mut self,
        source: roxmltree::NodeId,
        reference: LocalReference,
    ) -> Result<(), SvgImportError> {
        self.count = add_normalization_work(self.count, 1)?;
        self.by_source.entry(source).or_default().push(reference);
        Ok(())
    }

    fn charge_css_selectors(&mut self, comparisons: usize) -> Result<(), SvgImportError> {
        self.css_selector_work = add_normalization_work(self.css_selector_work, comparisons)?;
        Ok(())
    }

    fn get(&self, source: &roxmltree::NodeId) -> Option<&[LocalReference]> {
        self.by_source.get(source).map(Vec::as_slice)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct GeometryStats {
    shapes: usize,
    vertices: usize,
    middle_vertices: usize,
}

impl GeometryStats {
    fn add(&mut self, other: Self) {
        self.shapes = self.shapes.saturating_add(other.shapes);
        self.vertices = self.vertices.saturating_add(other.vertices);
        self.middle_vertices = self.middle_vertices.saturating_add(other.middle_vertices);
    }
}

#[derive(Default)]
struct NormalizationState {
    work: HashMap<roxmltree::NodeId, usize>,
    work_visiting: HashSet<roxmltree::NodeId>,
    geometry: HashMap<roxmltree::NodeId, GeometryStats>,
    geometry_visiting: HashSet<roxmltree::NodeId>,
    consumers: HashMap<roxmltree::NodeId, usize>,
    consumers_visiting: HashSet<roxmltree::NodeId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CssSpecificity {
    ids: u8,
    classes: u8,
    elements: u8,
}

enum CssSelectorClass {
    Supported {
        specificity: CssSpecificity,
        work_factor: usize,
    },
    Unsupported,
    Invalid,
}

#[derive(Clone, Copy)]
struct SourceCssElement<'a, 'input>(roxmltree::Node<'a, 'input>);

impl simplecss::Element for SourceCssElement<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(SourceCssElement)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(SourceCssElement)
    }

    fn has_local_name(&self, name: &str) -> bool {
        self.0.tag_name().name() == name
    }

    fn attribute_matches(&self, name: &str, operator: simplecss::AttributeOperator<'_>) -> bool {
        self.0
            .attribute(name)
            .is_some_and(|value| operator.matches(value))
    }

    fn pseudo_class_matches(&self, class: simplecss::PseudoClass<'_>) -> bool {
        matches!(class, simplecss::PseudoClass::FirstChild) && self.prev_sibling_element().is_none()
    }
}

#[derive(Clone, Copy, Debug)]
enum ExpansionPaint {
    None,
    Other,
    Gradient(roxmltree::NodeId),
}

#[derive(Clone, Copy, Debug)]
enum CascadedPaint {
    Inherit,
    Initial,
    Value(ExpansionPaint),
}

#[derive(Clone, Copy, Debug)]
struct DashExpansion {
    values: usize,
}

#[derive(Clone, Copy, Debug)]
enum CascadedDash {
    Inherit,
    Initial,
    Value(Option<DashExpansion>),
}

#[derive(Clone, Copy, Debug)]
struct SpecifiedValue<T> {
    important: bool,
    value: T,
}

#[derive(Clone, Copy, Debug, Default)]
struct SpecifiedExpansionStyle {
    fill: Option<SpecifiedValue<CascadedPaint>>,
    stroke: Option<SpecifiedValue<CascadedPaint>>,
    dash: Option<SpecifiedValue<CascadedDash>>,
}

#[derive(Clone, Copy, Debug)]
struct ComputedExpansionStyle {
    fill: ExpansionPaint,
    stroke: ExpansionPaint,
    dash: Option<DashExpansion>,
}

impl Default for ComputedExpansionStyle {
    fn default() -> Self {
        Self {
            fill: ExpansionPaint::Other,
            stroke: ExpansionPaint::None,
            dash: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ExpansionPropertyValue {
    Fill(CascadedPaint),
    Stroke(CascadedPaint),
    Dash(CascadedDash),
}

#[derive(Clone, Copy, Debug)]
struct StyleExpansionDeclaration {
    value: ExpansionPropertyValue,
    important: bool,
}

#[derive(Clone, Debug)]
struct StyleExpansionRule {
    selector: String,
    specificity: CssSpecificity,
    order: usize,
    declarations: Vec<StyleExpansionDeclaration>,
}

#[derive(Clone, Copy, Debug)]
struct PaintExpansionWork {
    target: roxmltree::NodeId,
    copies: usize,
}

#[derive(Default)]
struct NormalizationExpansion {
    paints: HashMap<roxmltree::NodeId, Vec<PaintExpansionWork>>,
    dash_values: HashMap<roxmltree::NodeId, usize>,
    cascade_work: usize,
    source_text: SourceTextStats,
}

fn source_feature_error(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    byte_position: Option<usize>,
    feature: String,
) -> SvgImportError {
    SvgImportError::UnsupportedFeature {
        feature,
        element: format!("<{}>", element.tag_name().name()),
        position: byte_position.map(|position| document.text_pos_at(position)),
    }
}

fn validate_local_references(
    document: &roxmltree::Document<'_>,
    elements: &[roxmltree::Node<'_, '_>],
    ids: &HashMap<String, roxmltree::Node<'_, '_>>,
    css_sources: &CssSources,
) -> Result<LocalReferences, SvgImportError> {
    let mut references = LocalReferences::default();

    for &element in elements {
        validate_href_reference(document, element, ids, &mut references)?;

        for attribute in element.attributes() {
            match attribute.name() {
                "href" | "id" => continue,
                "style" => {
                    let style = css_sources
                        .inline_styles
                        .get(&element.id())
                        .ok_or(SvgImportError::InternalStructure)?;
                    for_each_css_source_declaration(style, |property, value| {
                        validate_property_references(
                            document,
                            element,
                            attribute.range().start,
                            property,
                            value,
                            ids,
                            &mut references,
                        )
                    })?;
                }
                property => validate_property_references(
                    document,
                    element,
                    attribute.range().start,
                    property,
                    attribute.value(),
                    ids,
                    &mut references,
                )?,
            }
        }

        if element.tag_name().name() == "style" {
            let style_sheet = css_sources
                .style_sheets
                .get(&element.id())
                .ok_or(SvgImportError::InternalStructure)?;
            validate_style_sheet_references(
                document,
                element,
                style_sheet,
                elements,
                ids,
                &mut references,
            )?;
        }
    }

    Ok(references)
}

fn validate_href_reference(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    ids: &HashMap<String, roxmltree::Node<'_, '_>>,
    references: &mut LocalReferences,
) -> Result<(), SvgImportError> {
    let href = element
        .attributes()
        .find(|attribute| attribute.name() == "href" && attribute.namespace().is_none());
    let xlink_href = element.attributes().find(|attribute| {
        attribute.name() == "href" && attribute.namespace() == Some(XLINK_NAMESPACE)
    });

    if href.is_some() && xlink_href.is_some() {
        let position = xlink_href.map(|attribute| attribute.range().start);
        return Err(source_feature_error(
            document,
            element,
            position,
            "simultaneous href and xlink:href attributes".to_owned(),
        ));
    }

    let Some(attribute) = href.or(xlink_href) else {
        return Ok(());
    };
    if element.tag_name().name() == "a" {
        return Ok(());
    }

    let kind = match element.tag_name().name() {
        "use" => LocalReferenceKind::Use,
        "textPath" => LocalReferenceKind::TextPath,
        "linearGradient" | "radialGradient" => LocalReferenceKind::Gradient,
        name => {
            return Err(source_feature_error(
                document,
                element,
                Some(attribute.range().start),
                format!("local href references on <{name}>"),
            ));
        }
    };
    let value = attribute.value().trim();
    let target_id = value.strip_prefix('#').ok_or_else(|| {
        source_feature_error(
            document,
            element,
            Some(attribute.range().start),
            "external resource references".to_owned(),
        )
    })?;
    add_local_reference(
        document,
        element,
        attribute.range().start,
        target_id,
        kind,
        ids,
        references,
    )
}

fn validate_property_references(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    byte_position: usize,
    property: &str,
    value: &str,
    ids: &HashMap<String, roxmltree::Node<'_, '_>>,
    references: &mut LocalReferences,
) -> Result<(), SvgImportError> {
    let mut reference_kind = None;
    for_each_local_url_reference_id(value, |target_id| {
        let kind = if let Some(kind) = reference_kind {
            kind
        } else {
            let property = property.trim().to_ascii_lowercase();
            let kind = reference_kind_for_property(&property).ok_or_else(|| {
                source_feature_error(
                    document,
                    element,
                    Some(byte_position),
                    format!("local URL references in unsupported property {property}"),
                )
            })?;
            reference_kind = Some(kind);
            kind
        };
        add_local_reference(
            document,
            element,
            byte_position,
            target_id,
            kind,
            ids,
            references,
        )
    })
}

fn add_local_reference(
    document: &roxmltree::Document<'_>,
    source: roxmltree::Node<'_, '_>,
    byte_position: usize,
    target_id: &str,
    kind: LocalReferenceKind,
    ids: &HashMap<String, roxmltree::Node<'_, '_>>,
    references: &mut LocalReferences,
) -> Result<(), SvgImportError> {
    let target = resolve_local_target(document, source, byte_position, target_id, kind, ids)?;
    references.push(
        source.id(),
        LocalReference {
            target: target.id(),
            kind,
        },
    )
}

fn resolve_local_target<'a, 'input>(
    document: &roxmltree::Document<'input>,
    source: roxmltree::Node<'a, 'input>,
    byte_position: usize,
    target_id: &str,
    kind: LocalReferenceKind,
    ids: &HashMap<String, roxmltree::Node<'a, 'input>>,
) -> Result<roxmltree::Node<'a, 'input>, SvgImportError> {
    let target = ids.get(target_id).copied().ok_or_else(|| {
        source_feature_error(
            document,
            source,
            Some(byte_position),
            format!("missing local {} target #{target_id}", kind.description()),
        )
    })?;
    let target_name = target.tag_name().name();
    if !kind.accepts(target_name) {
        return Err(source_feature_error(
            document,
            source,
            Some(byte_position),
            format!(
                "local {} target #{target_id} must reference {} (found <{target_name}>)",
                kind.description(),
                kind.expected_target()
            ),
        ));
    }
    Ok(target)
}

fn reference_kind_for_property(property: &str) -> Option<LocalReferenceKind> {
    match property {
        "fill" | "stroke" => Some(LocalReferenceKind::Paint),
        "clip-path" => Some(LocalReferenceKind::ClipPath),
        "marker-start" => Some(LocalReferenceKind::Marker(MarkerPlacement::Start)),
        "marker-mid" => Some(LocalReferenceKind::Marker(MarkerPlacement::Mid)),
        "marker-end" => Some(LocalReferenceKind::Marker(MarkerPlacement::End)),
        "marker" => Some(LocalReferenceKind::Marker(MarkerPlacement::All)),
        _ => None,
    }
}

fn for_each_css_declaration(
    style: &str,
    mut visit: impl FnMut(&str, &str, bool) -> Result<(), SvgImportError>,
) -> Result<(), SvgImportError> {
    for declaration in simplecss::DeclarationTokenizer::from(style) {
        visit(declaration.name, declaration.value, declaration.important)?;
    }
    Ok(())
}

fn for_each_css_source_declaration(
    style: &str,
    mut visit: impl FnMut(&str, &str) -> Result<(), SvgImportError>,
) -> Result<(), SvgImportError> {
    for_each_css_segment(style, b';', |declaration, _| {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            return Ok(());
        }
        let Some(colon) = find_css_top_level_delimiter(declaration, b':')? else {
            return Ok(());
        };
        visit(declaration[..colon].trim(), declaration[colon + 1..].trim())
    })
}

fn for_each_css_rule(
    style_sheet: &str,
    mut visit: impl FnMut(&str, &str) -> Result<(), SvgImportError>,
) -> Result<(), SvgImportError> {
    for_each_css_segment(style_sheet, b'}', |rule, terminated| {
        let rule = rule.trim();
        if rule.is_empty() {
            return Ok(());
        }
        if !terminated {
            return Err(SvgImportError::MalformedCss {
                detail: "unterminated stylesheet rule",
            });
        }
        let Some(open_brace) = find_css_top_level_delimiter(rule, b'{')? else {
            return Err(SvgImportError::MalformedCss {
                detail: "stylesheet rule is missing an opening brace",
            });
        };
        let selectors = rule[..open_brace].trim();
        let declarations = rule[open_brace + 1..].trim();
        if selectors.is_empty() || find_css_top_level_delimiter(declarations, b'{')?.is_some() {
            return Err(SvgImportError::MalformedCss {
                detail: "nested or empty stylesheet rule",
            });
        }
        visit(selectors, declarations)
    })
}

fn for_each_css_segment(
    input: &str,
    delimiter: u8,
    mut visit: impl FnMut(&str, bool) -> Result<(), SvgImportError>,
) -> Result<(), SvgImportError> {
    let bytes = input.as_bytes();
    let mut start = 0;
    let mut index = 0;
    let mut quote = None;
    let mut parentheses = 0_usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2).min(bytes.len());
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                quote = Some(byte);
                index += 1;
            }
            b'\\' => {
                return Err(SvgImportError::MalformedCss {
                    detail: "CSS escapes outside strings are unsupported",
                });
            }
            b'(' => {
                parentheses = parentheses.saturating_add(1);
                index += 1;
            }
            b')' => {
                parentheses = parentheses
                    .checked_sub(1)
                    .ok_or(SvgImportError::MalformedCss {
                        detail: "unbalanced CSS parentheses",
                    })?;
                index += 1;
            }
            _ if byte == delimiter && parentheses == 0 => {
                visit(&input[start..index], true)?;
                index += 1;
                start = index;
            }
            _ => index += 1,
        }
    }
    if quote.is_some() {
        return Err(SvgImportError::MalformedCss {
            detail: "unterminated CSS string",
        });
    }
    if parentheses != 0 {
        return Err(SvgImportError::MalformedCss {
            detail: "unbalanced CSS parentheses",
        });
    }
    visit(&input[start..], false)
}

fn find_css_top_level_delimiter(
    input: &str,
    delimiter: u8,
) -> Result<Option<usize>, SvgImportError> {
    let mut found = None;
    for_each_css_segment(input, delimiter, |_, terminated| {
        if terminated && found.is_none() {
            found = Some(());
        }
        Ok(())
    })?;
    if found.is_none() {
        return Ok(None);
    }

    let mut position = None;
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut quote = None;
    let mut parentheses = 0_usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(quote_delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2).min(bytes.len());
            } else {
                if byte == quote_delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                quote = Some(byte);
                index += 1;
            }
            b'\\' => index += 2,
            b'(' => {
                parentheses += 1;
                index += 1;
            }
            b')' => {
                parentheses -= 1;
                index += 1;
            }
            _ if byte == delimiter && parentheses == 0 => {
                position = Some(index);
                break;
            }
            _ => index += 1,
        }
    }
    Ok(position)
}

fn for_each_local_url_reference_id<'a>(
    value: &'a str,
    mut visit: impl FnMut(&'a str) -> Result<(), SvgImportError>,
) -> Result<(), SvgImportError> {
    for_each_css_url(value, |target| {
        if let Some(id) = target.strip_prefix('#') {
            visit(id)?;
        }
        Ok(())
    })
}

fn for_each_css_url<'a>(
    value: &'a str,
    mut visit: impl FnMut(&'a str) -> Result<(), SvgImportError>,
) -> Result<(), SvgImportError> {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2).min(bytes.len());
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
            index += 1;
            continue;
        }
        if byte == b'\\' {
            index = index.saturating_add(2).min(bytes.len());
            continue;
        }
        if let Some(open_parenthesis) = css_url_function_start(bytes, index) {
            let (target, next) = parse_css_url_function(value, open_parenthesis)?;
            visit(target)?;
            index = next;
            continue;
        }
        index += 1;
    }
    Ok(())
}

fn css_url_function_start(bytes: &[u8], index: usize) -> Option<usize> {
    let name_end = index.checked_add(3)?;
    if name_end > bytes.len()
        || !bytes[index..name_end].eq_ignore_ascii_case(b"url")
        || index
            .checked_sub(1)
            .is_some_and(|previous| is_css_identifier_byte(bytes[previous]))
        || bytes
            .get(name_end)
            .is_some_and(|byte| is_css_identifier_byte(*byte))
    {
        return None;
    }
    let mut cursor = name_end;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'(')).then_some(cursor)
}

fn is_css_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-') || !byte.is_ascii()
}

fn parse_css_url_function(
    value: &str,
    open_parenthesis: usize,
) -> Result<(&str, usize), SvgImportError> {
    let bytes = value.as_bytes();
    let mut index = open_parenthesis + 1;
    let content_start = index;
    let mut quote = None;
    let mut depth = 1_usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2).min(bytes.len());
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                quote = Some(byte);
                index += 1;
            }
            b'\\' => {
                return Err(SvgImportError::MalformedCss {
                    detail: "CSS escapes outside strings are unsupported",
                });
            }
            b'(' => {
                depth = depth.saturating_add(1);
                index += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    let target = parse_css_url_target(&value[content_start..index])?;
                    return Ok((target, index + 1));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    Err(SvgImportError::MalformedCss {
        detail: "unterminated CSS url() function",
    })
}

fn parse_css_url_target(content: &str) -> Result<&str, SvgImportError> {
    let content = content.trim();
    let Some(delimiter) = content
        .as_bytes()
        .first()
        .copied()
        .filter(|byte| matches!(byte, b'\'' | b'"'))
    else {
        return Ok(content);
    };
    let bytes = content.as_bytes();
    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2).min(bytes.len()),
            byte if byte == delimiter => {
                if !content[index + 1..].trim().is_empty() {
                    return Err(SvgImportError::MalformedCss {
                        detail: "unexpected text after quoted CSS url",
                    });
                }
                return Ok(&content[1..index]);
            }
            _ => index += 1,
        }
    }
    Err(SvgImportError::MalformedCss {
        detail: "unterminated quoted CSS url",
    })
}

fn validate_style_sheet_references<'a, 'input>(
    document: &roxmltree::Document<'input>,
    style_element: roxmltree::Node<'a, 'input>,
    style_sheet: &str,
    elements: &[roxmltree::Node<'a, 'input>],
    ids: &HashMap<String, roxmltree::Node<'a, 'input>>,
    references: &mut LocalReferences,
) -> Result<(), SvgImportError> {
    let mut selector_candidate_work = None;
    for_each_css_rule(style_sheet, |selectors, declarations| {
        for_each_css_source_declaration(declarations, |property, value| {
            let mut has_local_target = false;
            for_each_local_url_reference_id(value, |_| {
                has_local_target = true;
                Ok(())
            })?;
            if !has_local_target {
                return Ok(());
            }

            let property = property.trim().to_ascii_lowercase();
            let kind = reference_kind_for_property(&property).ok_or_else(|| {
                source_feature_error(
                    document,
                    style_element,
                    Some(style_element.range().start),
                    format!("local URL references in unsupported CSS property {property}"),
                )
            })?;
            let selector_work_factor = css_selector_list_work_factor(selectors).map_err(|()| {
                source_feature_error(
                    document,
                    style_element,
                    Some(style_element.range().start),
                    "unsupported CSS selector with local resource references".to_owned(),
                )
            })?;
            let candidate_work = match selector_candidate_work {
                Some(work) => work,
                None => {
                    let work = css_selector_candidate_work(elements)?;
                    selector_candidate_work = Some(work);
                    work
                }
            };
            references.charge_css_selectors(multiply_normalization_work(
                multiply_normalization_work(candidate_work, selectors.len().max(1))?,
                selector_work_factor,
            )?)?;
            let matching_sources = elements
                .iter()
                .copied()
                .filter(|element| reference_property_applies(*element, kind))
                .map(|element| {
                    css_selector_list_matches(element, selectors).map(|matched| (element, matched))
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|()| {
                    source_feature_error(
                        document,
                        style_element,
                        Some(style_element.range().start),
                        "complex CSS selectors with local resource references".to_owned(),
                    )
                })?;

            for_each_local_url_reference_id(value, |target_id| {
                let target = resolve_local_target(
                    document,
                    style_element,
                    style_element.range().start,
                    target_id,
                    kind,
                    ids,
                )?;
                for (source, matched) in &matching_sources {
                    if *matched {
                        references.push(
                            source.id(),
                            LocalReference {
                                target: target.id(),
                                kind,
                            },
                        )?;
                    }
                }
                Ok(())
            })
        })
    })
}

fn reference_property_applies(element: roxmltree::Node<'_, '_>, kind: LocalReferenceKind) -> bool {
    let name = element.tag_name().name();
    match kind {
        LocalReferenceKind::Paint => matches!(
            name,
            "svg"
                | "g"
                | "a"
                | "switch"
                | "use"
                | "symbol"
                | "marker"
                | "path"
                | "rect"
                | "circle"
                | "ellipse"
                | "line"
                | "polyline"
                | "polygon"
                | "text"
                | "tspan"
                | "textPath"
        ),
        LocalReferenceKind::ClipPath => matches!(
            name,
            "svg"
                | "g"
                | "a"
                | "switch"
                | "use"
                | "symbol"
                | "marker"
                | "clipPath"
                | "path"
                | "rect"
                | "circle"
                | "ellipse"
                | "line"
                | "polyline"
                | "polygon"
                | "text"
        ),
        LocalReferenceKind::Marker(_) => matches!(
            name,
            "svg"
                | "g"
                | "a"
                | "switch"
                | "use"
                | "symbol"
                | "marker"
                | "path"
                | "rect"
                | "circle"
                | "ellipse"
                | "line"
                | "polyline"
                | "polygon"
        ),
        LocalReferenceKind::Use | LocalReferenceKind::TextPath | LocalReferenceKind::Gradient => {
            false
        }
    }
}

fn css_selector_list_matches(
    element: roxmltree::Node<'_, '_>,
    selectors: &str,
) -> Result<bool, ()> {
    let mut matched = false;
    for selector in selectors.split(',') {
        matched |= css_selector_matches(element, selector.trim())?;
    }
    Ok(matched)
}

fn css_selector_list_work_factor(selectors: &str) -> Result<usize, ()> {
    selectors.split(',').try_fold(1_usize, |factor, selector| {
        let CssSelectorClass::Supported { work_factor, .. } =
            classify_css_selector(selector.trim())
        else {
            return Err(());
        };
        Ok(factor.max(work_factor))
    })
}

fn css_selector_candidate_work(
    elements: &[roxmltree::Node<'_, '_>],
) -> Result<usize, SvgImportError> {
    elements.iter().try_fold(0_usize, |work, element| {
        let attribute_work = element.attributes().fold(0_usize, |sum, attribute| {
            let inspected_value_bytes = usize::from(matches!(attribute.name(), "class" | "id"))
                .saturating_mul(attribute.value().len());
            sum.saturating_add(1).saturating_add(inspected_value_bytes)
        });
        add_normalization_work(
            work,
            attribute_work
                .saturating_add(element.tag_name().name().len())
                .saturating_add(1),
        )
    })
}

fn css_selector_matches(element: roxmltree::Node<'_, '_>, selector: &str) -> Result<bool, ()> {
    simplecss::Selector::parse(selector)
        .map(|selector| selector.matches(&SourceCssElement(element)))
        .ok_or(())
}

fn collect_style_expansion_work<'a, 'input>(
    document: &roxmltree::Document<'input>,
    root: roxmltree::Node<'a, 'input>,
    elements: &[roxmltree::Node<'a, 'input>],
    ids: &HashMap<String, roxmltree::Node<'a, 'input>>,
    css_sources: &CssSources,
    references: &LocalReferences,
    source_text: SourceTextStats,
) -> Result<NormalizationExpansion, SvgImportError> {
    let mut specified = HashMap::<roxmltree::NodeId, SpecifiedExpansionStyle>::new();
    let mut cascade_work = 0_usize;
    let selector_candidate_work = css_selector_candidate_work(elements)?;

    for &element in elements {
        for attribute in element.attributes().filter(|attribute| {
            attribute.namespace().is_none()
                && matches!(attribute.name(), "fill" | "stroke" | "stroke-dasharray")
        }) {
            cascade_work = add_normalization_work(cascade_work, 1)?;
            let value = parse_expansion_property(attribute.name(), attribute.value(), ids)?
                .ok_or(SvgImportError::InternalStructure)?;
            apply_specified_expansion_value(
                specified.entry(element.id()).or_default(),
                value,
                false,
            );
        }
    }

    let mut rules = Vec::<StyleExpansionRule>::new();
    let mut rule_order = 0_usize;
    for &style_element in elements.iter().filter(|element| {
        element.tag_name().name() == "style"
            && matches!(element.attribute("type"), None | Some("text/css"))
    }) {
        let style_sheet = css_sources
            .style_sheets
            .get(&style_element.id())
            .ok_or(SvgImportError::InternalStructure)?;
        cascade_work = add_normalization_work(cascade_work, style_sheet.len())?;
        for_each_css_rule(style_sheet, |selectors, declarations| {
            rule_order = rule_order.saturating_add(1);
            let mut relevant = Vec::new();
            let mut declaration_work = 0_usize;
            for_each_css_declaration(declarations, |property, value, important| {
                declaration_work = add_normalization_work(
                    declaration_work,
                    property.len().saturating_add(value.len()).max(1),
                )?;
                if let Some(value) = parse_expansion_property(property, value, ids)? {
                    add_normalization_work(relevant.len(), 1)?;
                    relevant.push(StyleExpansionDeclaration { value, important });
                }
                Ok(())
            })?;
            // simplecss removes rules without valid declarations, so usvg never matches them.
            if declaration_work == 0 {
                return Ok(());
            }

            for_each_css_segment(selectors, b',', |selector, _| {
                let selector = selector.trim();
                let (specificity, work_factor) = match classify_css_selector(selector) {
                    CssSelectorClass::Supported {
                        specificity,
                        work_factor,
                    } => (specificity, work_factor),
                    CssSelectorClass::Invalid => return Ok(()),
                    CssSelectorClass::Unsupported => {
                        return Err(source_feature_error(
                            document,
                            style_element,
                            Some(style_element.range().start),
                            "CSS selector is outside bounded type/class/ID compounds with at most one descendant combinator".to_owned(),
                        ));
                    }
                };
                cascade_work = add_normalization_work(
                    cascade_work,
                    multiply_normalization_work(
                        multiply_normalization_work(
                            selector_candidate_work,
                            selector.len().max(1),
                        )?,
                        work_factor,
                    )?,
                )?;
                let matching_elements = elements.iter().try_fold(0_usize, |count, element| {
                    css_selector_matches(*element, selector)
                        .map_err(|()| SvgImportError::InternalStructure)
                        .map(|matched| count + usize::from(matched))
                })?;
                cascade_work = add_normalization_work(
                    cascade_work,
                    multiply_normalization_work(matching_elements, declaration_work)?,
                )?;
                if relevant.is_empty() {
                    return Ok(());
                }
                add_normalization_work(rules.len(), 1)?;
                rules.push(StyleExpansionRule {
                    selector: selector.to_owned(),
                    specificity,
                    order: rule_order,
                    declarations: relevant.clone(),
                });
                Ok(())
            })?;
            Ok(())
        })?;
    }

    rules.sort_by_key(|rule| (rule.specificity, rule.order));
    for rule in rules {
        for &element in elements {
            if !css_selector_matches(element, &rule.selector)
                .map_err(|()| SvgImportError::InternalStructure)?
            {
                continue;
            }
            let style = specified.entry(element.id()).or_default();
            for declaration in &rule.declarations {
                apply_specified_expansion_value(style, declaration.value, declaration.important);
            }
        }
    }

    for &element in elements {
        let Some(inline_style) = css_sources.inline_styles.get(&element.id()) else {
            continue;
        };
        cascade_work = add_normalization_work(cascade_work, inline_style.len())?;
        for_each_css_declaration(inline_style, |property, value, important| {
            let Some(value) = parse_expansion_property(property, value, ids)? else {
                return Ok(());
            };
            cascade_work = add_normalization_work(cascade_work, 1)?;
            apply_specified_expansion_value(
                specified.entry(element.id()).or_default(),
                value,
                important,
            );
            Ok(())
        })?;
    }

    let mut expansion = NormalizationExpansion {
        cascade_work,
        source_text,
        ..NormalizationExpansion::default()
    };
    let mut geometry_state = NormalizationState::default();
    collect_computed_expansion_work(
        document,
        root,
        ComputedExpansionStyle::default(),
        &specified,
        references,
        &mut geometry_state,
        &mut expansion,
    )?;
    Ok(expansion)
}

fn parse_expansion_property<'a, 'input>(
    property: &str,
    value: &str,
    ids: &HashMap<String, roxmltree::Node<'a, 'input>>,
) -> Result<Option<ExpansionPropertyValue>, SvgImportError> {
    match property.trim() {
        "fill" => Ok(Some(ExpansionPropertyValue::Fill(parse_cascaded_paint(
            value, ids,
        )?))),
        "stroke" => Ok(Some(ExpansionPropertyValue::Stroke(parse_cascaded_paint(
            value, ids,
        )?))),
        "stroke-dasharray" => Ok(Some(ExpansionPropertyValue::Dash(parse_cascaded_dash(
            value,
        )))),
        _ => Ok(None),
    }
}

fn parse_cascaded_paint<'a, 'input>(
    value: &str,
    ids: &HashMap<String, roxmltree::Node<'a, 'input>>,
) -> Result<CascadedPaint, SvgImportError> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("inherit")
        || trimmed.eq_ignore_ascii_case("unset")
        || trimmed.eq_ignore_ascii_case("revert")
        || trimmed.eq_ignore_ascii_case("revert-layer")
    {
        return Ok(CascadedPaint::Inherit);
    }
    if trimmed.eq_ignore_ascii_case("initial") {
        return Ok(CascadedPaint::Initial);
    }
    if trimmed.eq_ignore_ascii_case("none") {
        return Ok(CascadedPaint::Value(ExpansionPaint::None));
    }

    let mut gradient = None;
    for_each_local_url_reference_id(trimmed, |target_id| {
        if gradient.is_none() {
            gradient = ids.get(target_id).map(|target| target.id());
        }
        Ok(())
    })?;
    Ok(CascadedPaint::Value(
        gradient.map_or(ExpansionPaint::Other, |target| {
            ExpansionPaint::Gradient(target)
        }),
    ))
}

fn parse_cascaded_dash(value: &str) -> CascadedDash {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("inherit")
        || trimmed.eq_ignore_ascii_case("unset")
        || trimmed.eq_ignore_ascii_case("revert")
        || trimmed.eq_ignore_ascii_case("revert-layer")
    {
        return CascadedDash::Inherit;
    }
    if trimmed.eq_ignore_ascii_case("initial") {
        return CascadedDash::Initial;
    }
    if trimmed.eq_ignore_ascii_case("none") {
        return CascadedDash::Value(None);
    }
    CascadedDash::Value(Some(DashExpansion {
        values: count_svg_numbers(trimmed, MAX_SOURCE_LIST_VALUES + 1),
    }))
}

fn apply_specified_expansion_value(
    style: &mut SpecifiedExpansionStyle,
    value: ExpansionPropertyValue,
    important: bool,
) {
    match value {
        ExpansionPropertyValue::Fill(value) => {
            apply_cascade_value(&mut style.fill, value, important)
        }
        ExpansionPropertyValue::Stroke(value) => {
            apply_cascade_value(&mut style.stroke, value, important);
        }
        ExpansionPropertyValue::Dash(value) => {
            apply_cascade_value(&mut style.dash, value, important)
        }
    }
}

fn apply_cascade_value<T: Copy>(slot: &mut Option<SpecifiedValue<T>>, value: T, important: bool) {
    // usvg 0.45.1 retains the first important declaration it inserts. Normal
    // declarations keep replacing each other until an important value locks the slot.
    if slot.is_none_or(|current| !current.important) {
        *slot = Some(SpecifiedValue { important, value });
    }
}

fn classify_css_selector(selector: &str) -> CssSelectorClass {
    let Some(parsed) = simplecss::Selector::parse(selector) else {
        return CssSelectorClass::Invalid;
    };
    let mut descendant_combinators = 0_usize;
    for token in simplecss::SelectorTokenizer::from(selector) {
        match token {
            Err(_) => return CssSelectorClass::Invalid,
            Ok(simplecss::SelectorToken::DescendantCombinator) => {
                descendant_combinators += 1;
                if descendant_combinators > 1 {
                    return CssSelectorClass::Unsupported;
                }
            }
            Ok(
                simplecss::SelectorToken::AttributeSelector(_, _)
                | simplecss::SelectorToken::PseudoClass(_)
                | simplecss::SelectorToken::LangPseudoClass(_)
                | simplecss::SelectorToken::ChildCombinator
                | simplecss::SelectorToken::AdjacentCombinator,
            ) => return CssSelectorClass::Unsupported,
            Ok(
                simplecss::SelectorToken::UniversalSelector
                | simplecss::SelectorToken::TypeSelector(_)
                | simplecss::SelectorToken::ClassSelector(_)
                | simplecss::SelectorToken::IdSelector(_),
            ) => {}
        }
    }
    let [ids, classes, elements] = parsed.specificity();
    CssSelectorClass::Supported {
        specificity: CssSpecificity {
            ids,
            classes,
            elements,
        },
        work_factor: if descendant_combinators == 0 {
            1
        } else {
            MAX_SVG_NESTING_DEPTH.saturating_add(1)
        },
    }
}

fn collect_computed_expansion_work(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    inherited: ComputedExpansionStyle,
    specified: &HashMap<roxmltree::NodeId, SpecifiedExpansionStyle>,
    references: &LocalReferences,
    geometry_state: &mut NormalizationState,
    expansion: &mut NormalizationExpansion,
) -> Result<(), SvgImportError> {
    let current = computed_expansion_style(
        inherited,
        specified.get(&element.id()).copied().unwrap_or_default(),
    );
    let copies = graphical_consumer_copies(
        document,
        element,
        references,
        &expansion.source_text,
        geometry_state,
    )?;
    if copies != 0 {
        for paint in [current.fill, current.stroke] {
            if let ExpansionPaint::Gradient(target) = paint {
                if gradient_uses_object_bounding_box(document, target, references)? {
                    expansion
                        .paints
                        .entry(element.id())
                        .or_default()
                        .push(PaintExpansionWork { target, copies });
                }
            }
        }
        if !matches!(current.stroke, ExpansionPaint::None) {
            if let Some(dash) = current.dash {
                let work = multiply_normalization_work(dash.values, copies)?;
                expansion.dash_values.insert(element.id(), work);
            }
        }
    }

    if element.tag_name().name() != "metadata" {
        for child in element.children().filter(roxmltree::Node::is_element) {
            collect_computed_expansion_work(
                document,
                child,
                current,
                specified,
                references,
                geometry_state,
                expansion,
            )?;
        }
    }
    Ok(())
}

fn computed_expansion_style(
    inherited: ComputedExpansionStyle,
    specified: SpecifiedExpansionStyle,
) -> ComputedExpansionStyle {
    let fill = specified
        .fill
        .map_or(inherited.fill, |specified| match specified.value {
            CascadedPaint::Inherit => inherited.fill,
            CascadedPaint::Initial => ExpansionPaint::Other,
            CascadedPaint::Value(value) => value,
        });
    let stroke = specified
        .stroke
        .map_or(inherited.stroke, |specified| match specified.value {
            CascadedPaint::Inherit => inherited.stroke,
            CascadedPaint::Initial => ExpansionPaint::None,
            CascadedPaint::Value(value) => value,
        });
    let dash = specified
        .dash
        .map_or(inherited.dash, |specified| match specified.value {
            CascadedDash::Inherit => inherited.dash,
            CascadedDash::Initial => None,
            CascadedDash::Value(value) => value,
        });
    ComputedExpansionStyle { fill, stroke, dash }
}

fn graphical_consumer_copies(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    references: &LocalReferences,
    source_text: &SourceTextStats,
    geometry_state: &mut NormalizationState,
) -> Result<usize, SvgImportError> {
    if let Some(&characters) = source_text.characters.get(&element.id()) {
        return Ok(characters);
    }
    Ok(match element.tag_name().name() {
        "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => 1,
        "use" => estimate_graphical_consumers(
            document,
            element,
            references,
            source_text,
            geometry_state,
        )?
        .max(1),
        _ => 0,
    })
}

fn estimate_graphical_consumers(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    references: &LocalReferences,
    source_text: &SourceTextStats,
    state: &mut NormalizationState,
) -> Result<usize, SvgImportError> {
    if let Some(&consumers) = state.consumers.get(&element.id()) {
        return Ok(consumers);
    }
    if !state.consumers_visiting.insert(element.id()) {
        return Err(source_feature_error(
            document,
            element,
            Some(element.range().start),
            "cyclic local resource references".to_owned(),
        ));
    }

    let mut consumers = if let Some(&characters) = source_text.characters.get(&element.id()) {
        characters
    } else {
        match element.tag_name().name() {
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => 1,
            _ => 0,
        }
    };
    if element.tag_name().name() != "metadata" {
        for child in element.children().filter(roxmltree::Node::is_element) {
            consumers = add_normalization_work(
                consumers,
                estimate_graphical_consumers(document, child, references, source_text, state)?,
            )?;
        }
    }
    if let Some(element_references) = references.get(&element.id()) {
        for reference in element_references
            .iter()
            .filter(|reference| reference.kind == LocalReferenceKind::Use)
        {
            let target = document
                .get_node(reference.target)
                .ok_or(SvgImportError::InternalStructure)?;
            consumers = add_normalization_work(
                consumers,
                estimate_graphical_consumers(document, target, references, source_text, state)?,
            )?;
        }
    }

    state.consumers_visiting.remove(&element.id());
    state.consumers.insert(element.id(), consumers);
    Ok(consumers)
}

fn gradient_uses_object_bounding_box(
    document: &roxmltree::Document<'_>,
    target: roxmltree::NodeId,
    references: &LocalReferences,
) -> Result<bool, SvgImportError> {
    let mut current = target;
    for _ in 0..=MAX_SVG_NESTING_DEPTH {
        let gradient = document
            .get_node(current)
            .ok_or(SvgImportError::InternalStructure)?;
        if let Some(units) = gradient.attribute("gradientUnits") {
            return Ok(!units.trim().eq_ignore_ascii_case("userSpaceOnUse"));
        }
        let next = references.get(&current).and_then(|references| {
            references
                .iter()
                .find(|reference| reference.kind == LocalReferenceKind::Gradient)
                .map(|reference| reference.target)
        });
        let Some(next) = next else {
            return Ok(true);
        };
        current = next;
    }
    Err(SvgImportError::Complexity {
        resource: "pre-normalization reference depth",
        limit: MAX_SVG_NESTING_DEPTH,
    })
}

fn validate_reference_cycles(
    document: &roxmltree::Document<'_>,
    ids: &HashMap<String, roxmltree::Node<'_, '_>>,
    references: &LocalReferences,
) -> Result<(), SvgImportError> {
    let id_nodes = ids
        .values()
        .map(roxmltree::Node::id)
        .collect::<HashSet<_>>();
    let mut dependencies = HashMap::<roxmltree::NodeId, Vec<roxmltree::NodeId>>::new();
    let mut dependency_count = 0_usize;
    for (&source_id, source_references) in &references.by_source {
        let Some(source) = document.get_node(source_id) else {
            return Err(SvgImportError::InternalStructure);
        };
        for ancestor in source.ancestors().filter(roxmltree::Node::is_element) {
            if !id_nodes.contains(&ancestor.id()) {
                continue;
            }
            dependency_count = add_normalization_work(dependency_count, source_references.len())?;
            dependencies
                .entry(ancestor.id())
                .or_default()
                .extend(source_references.iter().map(|reference| reference.target));
        }
    }

    const VISITING: u8 = 1;
    const VISITED: u8 = 2;
    let mut states = HashMap::<roxmltree::NodeId, u8>::new();
    for &start in &id_nodes {
        if states.get(&start) == Some(&VISITED) {
            continue;
        }
        states.insert(start, VISITING);
        let mut stack = vec![(start, 0_usize)];
        while let Some((node, edge_index)) = stack.last().copied() {
            let next = dependencies
                .get(&node)
                .and_then(|targets| targets.get(edge_index))
                .copied();
            let Some(next) = next else {
                stack.pop();
                states.insert(node, VISITED);
                continue;
            };
            stack.last_mut().ok_or(SvgImportError::InternalStructure)?.1 += 1;
            match states.get(&next) {
                Some(&VISITING) => {
                    let target = document
                        .get_node(next)
                        .ok_or(SvgImportError::InternalStructure)?;
                    let target_id = target.attribute("id").unwrap_or("");
                    return Err(source_feature_error(
                        document,
                        target,
                        Some(target.range().start),
                        format!("cyclic local resource references involving #{target_id}"),
                    ));
                }
                Some(&VISITED) => {}
                _ => {
                    states.insert(next, VISITING);
                    stack.push((next, 0));
                }
            }
        }
    }
    Ok(())
}

fn validate_pre_normalization_work(
    document: &roxmltree::Document<'_>,
    root: roxmltree::Node<'_, '_>,
    references: &LocalReferences,
    expansion: &NormalizationExpansion,
) -> Result<(), SvgImportError> {
    let mut state = NormalizationState::default();
    let tree_work =
        estimate_normalization_work(document, root, references, expansion, &mut state, 0)?;
    add_normalization_work(tree_work, expansion.cascade_work)?;
    Ok(())
}

fn estimate_normalization_work(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    references: &LocalReferences,
    expansion: &NormalizationExpansion,
    state: &mut NormalizationState,
    reference_depth: usize,
) -> Result<usize, SvgImportError> {
    if let Some(&work) = state.work.get(&element.id()) {
        return Ok(work);
    }
    if reference_depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "pre-normalization reference depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
    }
    if !state.work_visiting.insert(element.id()) {
        return Err(source_feature_error(
            document,
            element,
            Some(element.range().start),
            "cyclic local resource references".to_owned(),
        ));
    }

    let mut work = 1_usize;
    if let Some(&characters) = expansion.source_text.characters.get(&element.id()) {
        work = add_normalization_work(
            work,
            multiply_normalization_work(characters, TEXT_OUTLINE_WORK_PER_CHARACTER)?,
        )?;
    }

    if element.tag_name().name() != "metadata" {
        for child in element.children().filter(roxmltree::Node::is_element) {
            let child_work = estimate_normalization_work(
                document,
                child,
                references,
                expansion,
                state,
                reference_depth,
            )?;
            work = add_normalization_work(work, child_work)?;
        }
    }

    if let Some(element_references) = references.get(&element.id()) {
        for reference in element_references {
            let target = document
                .get_node(reference.target)
                .ok_or(SvgImportError::InternalStructure)?;
            if reference.kind == LocalReferenceKind::Paint
                && gradient_uses_object_bounding_box(document, reference.target, references)?
            {
                continue;
            }
            let target_work = estimate_normalization_work(
                document,
                target,
                references,
                expansion,
                state,
                reference_depth + 1,
            )?;
            let multiplier = match reference.kind {
                LocalReferenceKind::Marker(placement) => {
                    marker_placement_count(document, element, placement, references, state)?
                }
                LocalReferenceKind::Use
                | LocalReferenceKind::TextPath
                | LocalReferenceKind::Gradient
                | LocalReferenceKind::Paint
                | LocalReferenceKind::ClipPath => 1,
            };
            work = add_normalization_work(
                work,
                multiply_normalization_work(target_work, multiplier)?,
            )?;
        }
    }

    if let Some(paints) = expansion.paints.get(&element.id()) {
        for paint in paints {
            let target = document
                .get_node(paint.target)
                .ok_or(SvgImportError::InternalStructure)?;
            let target_work = estimate_normalization_work(
                document,
                target,
                references,
                expansion,
                state,
                reference_depth + 1,
            )?;
            work = add_normalization_work(
                work,
                multiply_normalization_work(target_work, paint.copies)?,
            )?;
        }
    }
    if let Some(&dash_values) = expansion.dash_values.get(&element.id()) {
        work = add_normalization_work(work, dash_values)?;
    }

    state.work_visiting.remove(&element.id());
    state.work.insert(element.id(), work);
    Ok(work)
}

fn marker_placement_count(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    placement: MarkerPlacement,
    references: &LocalReferences,
    state: &mut NormalizationState,
) -> Result<usize, SvgImportError> {
    let stats = estimate_geometry_stats(document, element, references, state)?;
    Ok(match placement {
        MarkerPlacement::Start | MarkerPlacement::End => stats.shapes,
        MarkerPlacement::Mid => stats.middle_vertices,
        MarkerPlacement::All => stats.vertices,
    })
}

fn estimate_geometry_stats(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    references: &LocalReferences,
    state: &mut NormalizationState,
) -> Result<GeometryStats, SvgImportError> {
    if let Some(&stats) = state.geometry.get(&element.id()) {
        return Ok(stats);
    }
    if !state.geometry_visiting.insert(element.id()) {
        return Err(source_feature_error(
            document,
            element,
            Some(element.range().start),
            "cyclic local resource references".to_owned(),
        ));
    }

    let vertices = geometry_vertex_upper_bound(element);
    let mut stats = if vertices == 0 {
        GeometryStats::default()
    } else {
        GeometryStats {
            shapes: 1,
            vertices,
            middle_vertices: vertices.saturating_sub(2),
        }
    };
    if element.tag_name().name() != "metadata" {
        for child in element.children().filter(roxmltree::Node::is_element) {
            stats.add(estimate_geometry_stats(document, child, references, state)?);
        }
    }
    if let Some(element_references) = references.get(&element.id()) {
        for reference in element_references
            .iter()
            .filter(|reference| reference.kind == LocalReferenceKind::Use)
        {
            let target = document
                .get_node(reference.target)
                .ok_or(SvgImportError::InternalStructure)?;
            stats.add(estimate_geometry_stats(
                document, target, references, state,
            )?);
        }
    }

    state.geometry_visiting.remove(&element.id());
    state.geometry.insert(element.id(), stats);
    Ok(stats)
}

fn geometry_vertex_upper_bound(element: roxmltree::Node<'_, '_>) -> usize {
    match element.tag_name().name() {
        "path" => element
            .attribute("d")
            .map(str::trim)
            .filter(|data| !data.is_empty())
            .map_or(0, str::len),
        "polyline" | "polygon" => element
            .attribute("points")
            .map(str::trim)
            .filter(|points| !points.is_empty())
            .map_or(0, str::len),
        "line" => 2,
        "rect" | "circle" | "ellipse" => 4,
        _ => 0,
    }
}

fn add_normalization_work(left: usize, right: usize) -> Result<usize, SvgImportError> {
    left.checked_add(right)
        .filter(|&work| work <= MAX_PRE_NORMALIZATION_WORK)
        .ok_or(SvgImportError::Complexity {
            resource: "pre-normalization expansion work",
            limit: MAX_PRE_NORMALIZATION_WORK,
        })
}

fn multiply_normalization_work(left: usize, right: usize) -> Result<usize, SvgImportError> {
    left.checked_mul(right)
        .filter(|&work| work <= MAX_PRE_NORMALIZATION_WORK)
        .ok_or(SvgImportError::Complexity {
            resource: "pre-normalization expansion work",
            limit: MAX_PRE_NORMALIZATION_WORK,
        })
}

fn validate_feature_attributes(
    document: &roxmltree::Document<'_>,
    element: roxmltree::Node<'_, '_>,
    inline_style: Option<&str>,
) -> Result<(), SvgImportError> {
    for attribute in element.attributes() {
        let value = if attribute.name() == "style" {
            inline_style.ok_or(SvgImportError::InternalStructure)?
        } else {
            attribute.value()
        };
        let is_link_destination = element.tag_name().name() == "a" && attribute.name() == "href";
        let has_external_href = attribute.name() == "href"
            && !is_link_destination
            && !value.trim().is_empty()
            && !value.trim().starts_with('#');
        if has_external_href || has_external_url_reference(value)? {
            return Err(SvgImportError::UnsupportedFeature {
                feature: "external resource references".to_owned(),
                element: format!("<{}>", element.tag_name().name()),
                position: Some(document.text_pos_at(attribute.range().start)),
            });
        }

        let feature = unsupported_presentation_feature(attribute.name(), value);
        let feature = if feature.is_some() {
            feature
        } else {
            match attribute.name() {
                "mask" => Some("masks"),
                "filter" => Some("filters"),
                "mix-blend-mode" => Some("blend modes"),
                "isolation" => Some("group isolation"),
                "vector-effect" => Some("non-scaling strokes"),
                "fr" if element.tag_name().name() == "radialGradient" => {
                    Some("radial gradient focal radii")
                }
                "style" => unsupported_style_feature(value)?,
                _ => None,
            }
        };
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

fn has_external_url_reference(value: &str) -> Result<bool, SvgImportError> {
    let mut external = false;
    for_each_css_url(value, |target| {
        external |= !target.is_empty() && !target.starts_with('#');
        Ok(())
    })?;
    Ok(external)
}

fn unsupported_style_sheet_feature(
    style_sheet: &str,
) -> Result<Option<&'static str>, SvgImportError> {
    let mut has_at_rule = false;
    for_each_css_segment(style_sheet, b'@', |_, terminated| {
        has_at_rule |= terminated;
        Ok(())
    })?;
    if has_at_rule {
        return Ok(Some("CSS at-rules"));
    }

    let mut feature = None;
    for_each_css_rule(style_sheet, |_, declarations| {
        if feature.is_none() {
            feature = unsupported_style_feature(declarations)?;
        }
        Ok(())
    })?;
    Ok(feature)
}

fn unsupported_style_feature(style: &str) -> Result<Option<&'static str>, SvgImportError> {
    let mut feature = None;
    for_each_css_declaration(style, |property, value, _| {
        if feature.is_some() {
            return Ok(());
        }
        let property = property.trim().to_ascii_lowercase();
        let value = value
            .trim()
            .strip_suffix("!important")
            .unwrap_or(value.trim())
            .trim();
        feature = unsupported_presentation_feature(&property, value).or(match property.as_str() {
            "mask" => Some("masks"),
            "filter" => Some("filters"),
            "mix-blend-mode" => Some("blend modes"),
            "isolation" => Some("group isolation"),
            "vector-effect" => Some("non-scaling strokes"),
            _ => None,
        });
        Ok(())
    })?;
    Ok(feature)
}

fn unsupported_presentation_feature(property: &str, value: &str) -> Option<&'static str> {
    let value = value.trim().to_ascii_lowercase();
    match property {
        "stroke-linejoin" if value == "arcs" => Some("arcs stroke joins"),
        "color-interpolation" if !matches!(value.as_str(), "auto" | "srgb") => {
            Some("non-sRGB color interpolation")
        }
        "fill" | "stroke"
            if value.starts_with("var(")
                || matches!(value.as_str(), "context-fill" | "context-stroke") =>
        {
            Some("context-dependent paints")
        }
        _ => None,
    }
}

fn validate_group(group: &usvg::Group, depth: usize) -> Result<(), SvgImportError> {
    if depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "converted nesting depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
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
    if let Some(clip_path) = group.clip_path() {
        validate_clip_path(clip_path, depth + 1)?;
    }

    for node in group.children() {
        match node {
            usvg::Node::Group(child) => validate_group(child, depth + 1)?,
            usvg::Node::Path(path) => validate_path(path)?,
            usvg::Node::Image(_) => {
                return Err(unsupported_tree_feature("images", node.id()));
            }
            usvg::Node::Text(text) => validate_group(text.flattened(), depth + 1)?,
        }
    }
    Ok(())
}

fn validate_path(path: &usvg::Path) -> Result<(), SvgImportError> {
    if let Some(fill) = path.fill() {
        validate_paint(fill.paint(), path.id())?;
    }
    if let Some(stroke) = path.stroke() {
        validate_paint(stroke.paint(), path.id())?;
        if stroke
            .dasharray()
            .is_some_and(|dash_array| dash_array.len() > MAX_STROKE_DASH_VALUES)
        {
            return Err(SvgImportError::Complexity {
                resource: "values in one stroke dash pattern",
                limit: MAX_STROKE_DASH_VALUES,
            });
        }
    }
    import_transform(path.abs_transform())?;
    Ok(())
}

fn validate_paint(paint: &usvg::Paint, id: &str) -> Result<(), SvgImportError> {
    match paint {
        usvg::Paint::Color(_) => Ok(()),
        usvg::Paint::LinearGradient(gradient) => {
            validate_gradient(gradient.transform(), gradient.stops().len(), id)
        }
        usvg::Paint::RadialGradient(gradient) => {
            validate_gradient(gradient.transform(), gradient.stops().len(), id)
        }
        usvg::Paint::Pattern(_) => Err(unsupported_tree_feature("patterns", id)),
    }
}

fn validate_gradient(
    transform: tiny_skia::Transform,
    stop_count: usize,
    id: &str,
) -> Result<(), SvgImportError> {
    import_transform(transform)?;
    if stop_count > MAX_GRADIENT_STOPS {
        return Err(SvgImportError::Complexity {
            resource: "stops in one gradient",
            limit: MAX_GRADIENT_STOPS,
        });
    }
    if stop_count < 2 {
        return Err(unsupported_tree_feature(
            "gradients with fewer than two stops",
            id,
        ));
    }
    Ok(())
}

fn validate_clip_path(clip_path: &usvg::ClipPath, depth: usize) -> Result<(), SvgImportError> {
    if depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "converted nesting depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
    }
    import_transform(clip_path.transform())?;
    validate_clip_group(clip_path.root(), depth)?;
    if let Some(linked) = clip_path.clip_path() {
        validate_clip_path(linked, depth + 1)?;
    }
    Ok(())
}

fn validate_clip_group(group: &usvg::Group, depth: usize) -> Result<(), SvgImportError> {
    if depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "converted nesting depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
    }
    if group.mask().is_some() || !group.filters().is_empty() {
        return Err(unsupported_tree_feature(
            "masks or filters in clipping paths",
            group.id(),
        ));
    }
    if group.blend_mode() != usvg::BlendMode::Normal || group.isolate() {
        return Err(unsupported_tree_feature(
            "compositing in clipping paths",
            group.id(),
        ));
    }
    import_transform(group.transform())?;
    if let Some(clip_path) = group.clip_path() {
        validate_clip_path(clip_path, depth + 1)?;
    }

    for node in group.children() {
        match node {
            usvg::Node::Group(child) => validate_clip_group(child, depth + 1)?,
            usvg::Node::Path(path) => {
                import_transform(path.abs_transform())?;
            }
            usvg::Node::Text(text) => validate_clip_group(text.flattened(), depth + 1)?,
            usvg::Node::Image(_) => {
                return Err(unsupported_tree_feature(
                    "images in clipping paths",
                    node.id(),
                ));
            }
        }
    }
    Ok(())
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
    total_gradient_stops: usize,
    total_stroke_dash_values: usize,
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
                let mut node = Node::group(non_empty_name(child_group.id(), || {
                    format!("Group {}", state.groups)
                }))
                .with_transform(import_transform(child_group.transform())?)
                .with_style(import_group_style(child_group));
                node.clip_path = child_group
                    .clip_path()
                    .map(|clip| import_clip_path(clip, depth + 1, state))
                    .transpose()?;
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
                .with_style(import_path_style(path, state)?)
                .with_visible(path.is_visible());
                document
                    .add_child(parent, node)
                    .ok_or(SvgImportError::InternalStructure)?;
            }
            usvg::Node::Text(text) => {
                let flattened = text.flattened();
                let mut node = Node::group(non_empty_name(text.id(), || {
                    format!("Text outlines {}", state.groups + 1)
                }))
                .with_transform(
                    relative_transform(parent_absolute, text.abs_transform())?
                        * import_transform(flattened.transform())?,
                )
                .with_style(import_group_style(flattened));
                node.clip_path = flattened
                    .clip_path()
                    .map(|clip| import_clip_path(clip, depth + 1, state))
                    .transpose()?;
                state.groups += 1;
                let text_group = document
                    .add_child(parent, node)
                    .ok_or(SvgImportError::InternalStructure)?;
                append_local_children(document, text_group, flattened, depth + 1, state)?;
            }
            usvg::Node::Image(_) => {
                return Err(SvgImportError::InternalStructure);
            }
        }
    }
    Ok(())
}

/// Append a text node's flattened subtree.
///
/// Unlike nodes in the main tree, flattened glyph nodes carry transforms relative to the
/// flattened text group. Their `abs_transform` values are not relative to the main SVG root.
fn append_local_children(
    document: &mut Document,
    parent: NodeId,
    group: &usvg::Group,
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
                let mut node = Node::group(non_empty_name(child_group.id(), || {
                    format!("Group {}", state.groups)
                }))
                .with_transform(import_transform(child_group.transform())?)
                .with_style(import_group_style(child_group));
                node.clip_path = child_group
                    .clip_path()
                    .map(|clip| import_clip_path(clip, depth + 1, state))
                    .transpose()?;
                let child_id = document
                    .add_child(parent, node)
                    .ok_or(SvgImportError::InternalStructure)?;
                append_local_children(document, child_id, child_group, depth + 1, state)?;
            }
            usvg::Node::Path(path) => {
                state.paths += 1;
                let node = Node::shape(
                    non_empty_name(path.id(), || format!("Path {}", state.paths)),
                    import_path_data(path.data(), state)?,
                )
                .with_style(import_path_style(path, state)?)
                .with_visible(path.is_visible());
                document
                    .add_child(parent, node)
                    .ok_or(SvgImportError::InternalStructure)?;
            }
            usvg::Node::Image(_) => {
                return Err(unsupported_tree_feature("bitmap glyphs", child.id()));
            }
            usvg::Node::Text(_) => {
                return Err(unsupported_tree_feature(
                    "nested flattened text",
                    child.id(),
                ));
            }
        }
    }
    Ok(())
}

fn import_clip_path(
    clip_path: &usvg::ClipPath,
    depth: usize,
    state: &mut ImportState,
) -> Result<ClipPath, SvgImportError> {
    if depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "converted nesting depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
    }

    Ok(ClipPath {
        transform: import_transform(clip_path.transform())?,
        children: import_clip_children(
            clip_path.root(),
            clip_path.root().abs_transform(),
            depth,
            state,
        )?,
        // A linked clipPath remains in the coordinate system of the artwork node that owns
        // this clip, rather than inheriting this clipPath's transform.
        clip_path: clip_path
            .clip_path()
            .map(|linked| import_clip_path(linked, depth + 1, state).map(Box::new))
            .transpose()?,
    })
}

fn import_clip_children(
    group: &usvg::Group,
    parent_absolute: tiny_skia::Transform,
    depth: usize,
    state: &mut ImportState,
) -> Result<Vec<ClipNode>, SvgImportError> {
    if depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "converted nesting depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
    }

    let mut children = Vec::new();
    for child in group.children() {
        match child {
            usvg::Node::Group(child_group) => {
                increment_import_node(state)?;
                children.push(ClipNode::Group(ClipPath {
                    transform: import_transform(child_group.transform())?,
                    children: import_clip_children(
                        child_group,
                        child_group.abs_transform(),
                        depth + 1,
                        state,
                    )?,
                    clip_path: child_group
                        .clip_path()
                        .map(|clip| import_clip_path(clip, depth + 1, state).map(Box::new))
                        .transpose()?,
                }));
            }
            usvg::Node::Path(path) if path.is_visible() => {
                increment_import_node(state)?;
                children.push(ClipNode::Shape(ClipShape {
                    path: import_path_data(path.data(), state)?,
                    transform: relative_transform(parent_absolute, path.abs_transform())?,
                    fill_rule: import_fill_rule(path.fill().map(usvg::Fill::rule)),
                }));
            }
            usvg::Node::Text(text) => {
                increment_import_node(state)?;
                let flattened = text.flattened();
                children.push(ClipNode::Group(ClipPath {
                    transform: relative_transform(parent_absolute, text.abs_transform())?
                        * import_transform(flattened.transform())?,
                    children: import_local_clip_children(flattened, depth + 1, state)?,
                    clip_path: flattened
                        .clip_path()
                        .map(|clip| import_clip_path(clip, depth + 1, state).map(Box::new))
                        .transpose()?,
                }));
            }
            usvg::Node::Path(_) => {}
            usvg::Node::Image(_) => {
                return Err(unsupported_tree_feature(
                    "images in clipping paths",
                    child.id(),
                ));
            }
        }
    }
    Ok(children)
}

fn import_local_clip_children(
    group: &usvg::Group,
    depth: usize,
    state: &mut ImportState,
) -> Result<Vec<ClipNode>, SvgImportError> {
    if depth > MAX_SVG_NESTING_DEPTH {
        return Err(SvgImportError::Complexity {
            resource: "converted nesting depth",
            limit: MAX_SVG_NESTING_DEPTH,
        });
    }

    let mut children = Vec::new();
    for child in group.children() {
        match child {
            usvg::Node::Group(child_group) => {
                increment_import_node(state)?;
                children.push(ClipNode::Group(ClipPath {
                    transform: import_transform(child_group.transform())?,
                    children: import_local_clip_children(child_group, depth + 1, state)?,
                    clip_path: child_group
                        .clip_path()
                        .map(|clip| import_clip_path(clip, depth + 1, state).map(Box::new))
                        .transpose()?,
                }));
            }
            usvg::Node::Path(path) if path.is_visible() => {
                increment_import_node(state)?;
                children.push(ClipNode::Shape(ClipShape {
                    path: import_path_data(path.data(), state)?,
                    transform: Affine2::IDENTITY,
                    fill_rule: import_fill_rule(path.fill().map(usvg::Fill::rule)),
                }));
            }
            usvg::Node::Path(_) => {}
            usvg::Node::Image(_) => {
                return Err(unsupported_tree_feature("bitmap glyphs", child.id()));
            }
            usvg::Node::Text(_) => {
                return Err(unsupported_tree_feature(
                    "nested flattened text",
                    child.id(),
                ));
            }
        }
    }
    Ok(children)
}

fn increment_import_node(state: &mut ImportState) -> Result<(), SvgImportError> {
    state.nodes += 1;
    if state.nodes > MAX_DOCUMENT_NODES {
        Err(SvgImportError::Complexity {
            resource: "document nodes",
            limit: MAX_DOCUMENT_NODES,
        })
    } else {
        Ok(())
    }
}

fn import_fill_rule(rule: Option<usvg::FillRule>) -> FillRule {
    match rule {
        Some(usvg::FillRule::EvenOdd) => FillRule::EvenOdd,
        Some(usvg::FillRule::NonZero) | None => FillRule::NonZero,
    }
}

fn import_group_style(group: &usvg::Group) -> Style {
    Style {
        opacity: group.opacity().get(),
        ..Style::default()
    }
}

fn import_path_style(path: &usvg::Path, state: &mut ImportState) -> Result<Style, SvgImportError> {
    let fill = path
        .fill()
        .map(|fill| import_fill(fill, state))
        .transpose()?;
    let stroke = path
        .stroke()
        .map(|stroke| import_stroke(stroke, state))
        .transpose()?;
    Ok(Style {
        fill,
        stroke,
        opacity: 1.0,
        fill_rule: import_fill_rule(path.fill().map(usvg::Fill::rule)),
        paint_order: match path.paint_order() {
            usvg::PaintOrder::FillAndStroke => PaintOrder::FillAndStroke,
            usvg::PaintOrder::StrokeAndFill => PaintOrder::StrokeAndFill,
        },
    })
}

fn import_fill(fill: &usvg::Fill, state: &mut ImportState) -> Result<Paint, SvgImportError> {
    import_paint(fill.paint(), fill.opacity().get(), state)
}

fn import_stroke(stroke: &usvg::Stroke, state: &mut ImportState) -> Result<Stroke, SvgImportError> {
    let dash_array = stroke.dasharray().unwrap_or_default();
    state.total_stroke_dash_values = state
        .total_stroke_dash_values
        .saturating_add(dash_array.len());
    if state.total_stroke_dash_values > MAX_TOTAL_STROKE_DASH_VALUES {
        return Err(SvgImportError::Complexity {
            resource: "total imported stroke dash values",
            limit: MAX_TOTAL_STROKE_DASH_VALUES,
        });
    }

    Ok(Stroke {
        width: stroke.width().get(),
        paint: import_paint(stroke.paint(), stroke.opacity().get(), state)?,
        line_cap: match stroke.linecap() {
            usvg::LineCap::Butt => LineCap::Butt,
            usvg::LineCap::Round => LineCap::Round,
            usvg::LineCap::Square => LineCap::Square,
        },
        line_join: match stroke.linejoin() {
            usvg::LineJoin::Miter => LineJoin::Miter,
            usvg::LineJoin::MiterClip => LineJoin::MiterClip,
            usvg::LineJoin::Round => LineJoin::Round,
            usvg::LineJoin::Bevel => LineJoin::Bevel,
        },
        miter_limit: stroke.miterlimit().get(),
        dash_array: dash_array.to_vec(),
        dash_offset: stroke.dashoffset(),
    })
}

fn import_paint(
    paint: &usvg::Paint,
    alpha: f32,
    state: &mut ImportState,
) -> Result<Paint, SvgImportError> {
    match paint {
        usvg::Paint::Color(color) => Ok(Paint::rgba(
            f32::from(color.red) / 255.0,
            f32::from(color.green) / 255.0,
            f32::from(color.blue) / 255.0,
            alpha,
        )),
        usvg::Paint::LinearGradient(gradient) => {
            track_gradient_stops(gradient.stops().len(), state)?;
            Ok(Paint::LinearGradient(LinearGradient {
                start: Vec2::new(gradient.x1(), gradient.y1()),
                end: Vec2::new(gradient.x2(), gradient.y2()),
                transform: import_transform(gradient.transform())?,
                spread: import_spread_method(gradient.spread_method()),
                stops: import_gradient_stops(gradient.stops(), alpha),
            }))
        }
        usvg::Paint::RadialGradient(gradient) => {
            track_gradient_stops(gradient.stops().len(), state)?;
            Ok(Paint::RadialGradient(RadialGradient {
                center: Vec2::new(gradient.cx(), gradient.cy()),
                focal: Vec2::new(gradient.fx(), gradient.fy()),
                radius: gradient.r().get(),
                transform: import_transform(gradient.transform())?,
                spread: import_spread_method(gradient.spread_method()),
                stops: import_gradient_stops(gradient.stops(), alpha),
            }))
        }
        usvg::Paint::Pattern(_) => Err(unsupported_tree_feature("patterns", "")),
    }
}

fn track_gradient_stops(stop_count: usize, state: &mut ImportState) -> Result<(), SvgImportError> {
    state.total_gradient_stops = state.total_gradient_stops.saturating_add(stop_count);
    if state.total_gradient_stops > MAX_TOTAL_GRADIENT_STOPS {
        Err(SvgImportError::Complexity {
            resource: "total imported gradient stops",
            limit: MAX_TOTAL_GRADIENT_STOPS,
        })
    } else {
        Ok(())
    }
}

fn import_gradient_stops(stops: &[usvg::Stop], alpha: f32) -> Vec<GradientStop> {
    stops
        .iter()
        .map(|stop| GradientStop {
            offset: stop.offset().get(),
            color: [
                f32::from(stop.color().red) / 255.0,
                f32::from(stop.color().green) / 255.0,
                f32::from(stop.color().blue) / 255.0,
                stop.opacity().get() * alpha,
            ],
        })
        .collect()
}

fn import_spread_method(method: usvg::SpreadMethod) -> SpreadMethod {
    match method {
        usvg::SpreadMethod::Pad => SpreadMethod::Pad,
        usvg::SpreadMethod::Reflect => SpreadMethod::Reflect,
        usvg::SpreadMethod::Repeat => SpreadMethod::Repeat,
    }
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
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(SvgImportError::InvalidGeometry);
    }
    let inverse = parent.inverse();
    if !inverse.matrix2.is_finite() || !inverse.translation.is_finite() {
        return Err(SvgImportError::InvalidGeometry);
    }
    let relative = inverse * import_transform(absolute)?;
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
    fn imports_nested_geometry_under_a_small_but_invertible_transform() {
        let mut document = import_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <g transform="scale(0.0001)">
                    <path id="small" d="M0 0L10000 0L10000 10000Z"/>
                </g>
            </svg>"#,
            "small-scale.svg",
        )
        .unwrap();

        let path_id = document
            .descendants(document.root)
            .find(|&id| document.get(id).is_some_and(|node| node.name == "small"))
            .unwrap();
        let determinant = document.world_transform(path_id).matrix2.determinant();
        assert!(determinant.is_finite());
        assert!(determinant > 0.0 && determinant < f32::EPSILON);
    }

    #[test]
    fn rejects_unsupported_elements_with_source_position() {
        let error = import_svg(
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><image href=\"image.png\"/></svg>",
            "image.svg",
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("unsupported SVG element <image>"));
        assert!(message.contains("1:"));
    }

    #[test]
    fn preserves_the_svg_viewport_as_a_transparent_frame() {
        let mut document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"
                         viewBox="0 0 12 8">
                <rect x="5" y="3" width="2" height="2" fill="#f00"/>
            </svg>"##,
            "whitespace.svg",
        )
        .unwrap();

        let frame_id = document.get(document.root).unwrap().children[0];
        let frame = document.get(frame_id).unwrap().frame_data().unwrap();
        assert!((frame.width - 120.0).abs() < 0.001);
        assert!((frame.height - 80.0).abs() < 0.001);
        assert_eq!(frame.background, None);

        let bounds = document.world_bounds(frame_id).unwrap();
        assert!((bounds.width() - 120.0).abs() < 0.001);
        assert!((bounds.height() - 80.0).abs() < 0.001);
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
    fn imports_css_links_switches_and_reused_symbols_after_normalization() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
                <style>.accent { fill: #123456; }</style>
                <metadata><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"/></metadata>
                <defs>
                    <symbol id="tile" viewBox="0 0 10 10" overflow="visible">
                        <rect id="reused" width="10" height="10" class="accent"/>
                    </symbol>
                </defs>
                <a href="https://example.invalid">
                    <use href="#tile" width="10" height="10"/>
                </a>
                <switch>
                    <path id="chosen" d="M20 0h10v10H20z" class="accent"/>
                </switch>
            </svg>"##,
            "normalized.svg",
        )
        .unwrap();

        let shapes = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .filter(|node| matches!(node.kind, NodeKind::Shape(_)))
            .collect::<Vec<_>>();
        assert_eq!(shapes.len(), 2);
        assert!(shapes.iter().all(|node| node.style.fill
            == Some(Paint::rgba(
                0x12 as f32 / 255.0,
                0x34 as f32 / 255.0,
                0x56 as f32 / 255.0,
                1.0
            ))));
    }

    #[test]
    fn imports_nested_svg_and_markers_when_the_normalized_tree_is_supported() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="30">
                <defs>
                    <marker id="arrow" markerWidth="4" markerHeight="4" refX="4" refY="2"
                            orient="auto" overflow="visible">
                        <path d="M0 0L4 2L0 4z" fill="#f00"/>
                    </marker>
                </defs>
                <svg x="2" y="2" width="10" height="10" viewBox="0 0 10 10" overflow="visible">
                    <rect id="nested" width="10" height="10" fill="#0f0"/>
                </svg>
                <path id="marked" d="M20 5L40 5" fill="none" stroke="#000"
                      marker-end="url(#arrow)"/>
            </svg>"##,
            "structural.svg",
        )
        .unwrap();

        let shape_count = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .filter(|node| matches!(node.kind, NodeKind::Shape(_)))
            .count();
        assert_eq!(shape_count, 3);
    }

    #[test]
    fn imports_text_as_editable_outline_paths() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40">
                <text id="label" x="5" y="25" font-family="sans-serif" font-size="20"
                      fill="#2468ac">Hi</text>
            </svg>"##,
            "text.svg",
        )
        .unwrap();

        let nodes = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .collect::<Vec<_>>();
        assert!(nodes.iter().any(|node| node.name == "label"));
        assert!(nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Shape(_))));
        assert!(!nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Text(_))));
    }

    #[test]
    fn rejects_excessive_text_before_font_shaping() {
        let content = "A".repeat(MAX_TOTAL_TEXT_BYTES + 1);
        for text in [
            format!("<text>{content}</text>"),
            format!("<text><a>{content}</a></text>"),
        ] {
            let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{text}</svg>");
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "text bytes",
                    limit: MAX_TOTAL_TEXT_BYTES,
                }
            ));
        }
    }

    #[test]
    fn ignores_text_nodes_that_usvg_does_not_shape() {
        let content = "A".repeat(MAX_TOTAL_TEXT_BYTES + 1);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <a>{content}</a>
                <rect width="1" height="1"/>
            </svg>"#
        );

        source_preflight(&svg).unwrap();
    }

    #[test]
    fn imports_gradients_fill_rules_paint_order_and_advanced_strokes() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60">
                <defs>
                    <linearGradient id="linear" x1="0" y1="0" x2="1" y2="1"
                                    spreadMethod="reflect" gradientTransform="rotate(10)">
                        <stop offset="0" stop-color="#f00" stop-opacity="0.5"/>
                        <stop offset="1" stop-color="#00f"/>
                    </linearGradient>
                    <radialGradient id="radial" cx="0.5" cy="0.5" r="0.5"
                                    fx="0.25" fy="0.25" spreadMethod="repeat">
                        <stop offset="0" stop-color="#fff"/>
                        <stop offset="1" stop-color="#000"/>
                    </radialGradient>
                </defs>
                <path id="styled" d="M5 5h40v40H5zM15 15v20h20V15z"
                      fill="url(#linear)" fill-opacity="0.8" fill-rule="evenodd"
                      stroke="url(#radial)" stroke-width="3" stroke-opacity="0.6"
                      stroke-linecap="square" stroke-linejoin="bevel" stroke-miterlimit="7"
                      stroke-dasharray="2 3" stroke-dashoffset="1" paint-order="stroke fill"/>
                <path id="round" d="M55 10L90 10" fill="none" stroke="#000"
                      stroke-linecap="round" stroke-linejoin="round"/>
                <path id="miter-clip" d="M55 30L70 45L90 30" fill="none" stroke="#000"
                      stroke-linejoin="miter-clip"/>
            </svg>"##,
            "advanced.svg",
        )
        .unwrap();

        let styled = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .find(|node| node.name == "styled")
            .unwrap();
        assert_eq!(styled.style.fill_rule, FillRule::EvenOdd);
        assert_eq!(styled.style.paint_order, PaintOrder::StrokeAndFill);
        let Paint::LinearGradient(linear) = styled.style.fill.as_ref().unwrap() else {
            panic!("expected a linear gradient fill");
        };
        assert_eq!(linear.spread, SpreadMethod::Reflect);
        assert_eq!(linear.stops.len(), 2);
        assert!((linear.stops[0].color[3] - 0.4).abs() < 0.001);

        let stroke = styled.style.stroke.as_ref().unwrap();
        assert_eq!(stroke.line_cap, LineCap::Square);
        assert_eq!(stroke.line_join, LineJoin::Bevel);
        assert!((stroke.miter_limit - 7.0).abs() < 0.001);
        assert_eq!(stroke.dash_array, vec![2.0, 3.0]);
        assert!((stroke.dash_offset - 1.0).abs() < 0.001);
        let Paint::RadialGradient(radial) = &stroke.paint else {
            panic!("expected a radial gradient stroke");
        };
        assert_eq!(radial.spread, SpreadMethod::Repeat);
        assert!((radial.stops[0].color[3] - 0.6).abs() < 0.001);

        let round = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .find(|node| node.name == "round")
            .unwrap();
        assert_eq!(
            round.style.stroke.as_ref().unwrap().line_cap,
            LineCap::Round
        );
        assert_eq!(
            round.style.stroke.as_ref().unwrap().line_join,
            LineJoin::Round
        );

        let miter_clip = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .find(|node| node.name == "miter-clip")
            .unwrap();
        assert_eq!(
            miter_clip.style.stroke.as_ref().unwrap().line_join,
            LineJoin::MiterClip
        );
    }

    #[test]
    fn imports_object_opacity_without_flattening_paint_opacity() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <g id="outer" opacity="0.5">
                    <rect id="paint" width="10" height="10" opacity="0.4"
                          fill="#f00" fill-opacity="0.25"
                          stroke="#00f" stroke-opacity="0.75"/>
                </g>
            </svg>"##,
            "opacity.svg",
        )
        .unwrap();

        let nodes = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .collect::<Vec<_>>();
        let outer = nodes.iter().find(|node| node.name == "outer").unwrap();
        assert!((outer.style.opacity - 0.5).abs() < 0.001);
        let object_group = nodes
            .iter()
            .find(|node| {
                matches!(node.kind, NodeKind::Group) && (node.style.opacity - 0.4).abs() < 0.001
            })
            .unwrap();
        assert!((object_group.style.opacity - 0.4).abs() < 0.001);

        let paint = nodes.iter().find(|node| node.name == "paint").unwrap();
        assert_eq!(paint.style.opacity, 1.0);
        assert_eq!(paint.style.fill, Some(Paint::rgba(1.0, 0.0, 0.0, 0.25)));
        assert_eq!(
            paint.style.stroke.as_ref().unwrap().paint,
            Paint::rgba(0.0, 0.0, 1.0, 0.75)
        );
    }

    #[test]
    fn imports_recursive_and_transformed_clipping_paths() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="80">
                <defs>
                    <clipPath id="inner" transform="translate(2 3)">
                        <path d="M0 0h45v35H0zM10 10h20v15H10z" clip-rule="evenodd"/>
                    </clipPath>
                    <clipPath id="outer" clip-path="url(#inner)">
                        <rect width="50" height="40" transform="translate(5 7)"/>
                    </clipPath>
                </defs>
                <g id="clipped" clip-path="url(#outer)">
                    <rect width="80" height="60" fill="#f00"/>
                </g>
            </svg>"##,
            "clip.svg",
        )
        .unwrap();

        let clipped = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .find(|node| node.name == "clipped")
            .unwrap();
        let clip = clipped.clip_path.as_ref().unwrap();
        assert_eq!(clip.children.len(), 1);
        let ClipNode::Group(group) = &clip.children[0] else {
            panic!("expected the transformed clip group");
        };
        assert!((group.transform.translation.x - 5.0).abs() < 0.001);
        assert!((group.transform.translation.y - 7.0).abs() < 0.001);

        let linked = clip.clip_path.as_ref().unwrap();
        assert!((linked.transform.translation.x - 2.0).abs() < 0.001);
        assert!((linked.transform.translation.y - 3.0).abs() < 0.001);
        let ClipNode::Shape(shape) = &linked.children[0] else {
            panic!("expected the linked clip shape");
        };
        assert_eq!(shape.fill_rule, FillRule::EvenOdd);
    }

    #[test]
    fn imported_open_clip_subpaths_are_implicitly_closed_for_hit_testing() {
        let mut document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <defs><clipPath id="clip"><path d="M0 0 L10 0 L10 10"/></clipPath></defs>
                <rect id="target" width="10" height="10" fill="red" clip-path="url(#clip)"/>
            </svg>"##,
            "open-clip.svg",
        )
        .unwrap();

        let target = document
            .descendants(document.root)
            .find(|id| document.get(*id).is_some_and(|node| node.name == "target"))
            .unwrap();
        assert_eq!(
            document.hit_test_with_tolerance(Vec2::new(8.0, 2.0), false, 0.0),
            Some(target)
        );
        assert_ne!(
            document.hit_test_with_tolerance(Vec2::new(2.0, 8.0), false, 0.0),
            Some(target)
        );
    }

    #[test]
    fn advanced_import_survives_the_display_list_and_svg_renderer() {
        let mut document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
                <defs>
                    <linearGradient id="paint" x1="0" y1="0" x2="40" y2="0"
                                    gradientUnits="userSpaceOnUse">
                        <stop offset="0" stop-color="#f00"/>
                        <stop offset="1" stop-color="#00f"/>
                    </linearGradient>
                    <clipPath id="clip"><circle cx="20" cy="10" r="9"/></clipPath>
                </defs>
                <g opacity="0.6" clip-path="url(#clip)">
                    <path d="M0 0h40v20H0zM14 4v12h12V4z" fill="url(#paint)"
                          fill-rule="evenodd" stroke="#000" stroke-width="2"
                          stroke-linecap="round" stroke-dasharray="3 2"/>
                </g>
            </svg>"##,
            "pipeline.svg",
        )
        .unwrap();

        let display_list = document.build_display_list(&editor_core::View::default());
        let svg = render_svg::to_svg_string_export(&display_list, 40.0, 20.0);
        assert!(svg.contains("<linearGradient"));
        assert!(svg.contains(r#"fill-rule="evenodd""#));
        assert!(svg.contains(r#"clip-path="url(#strek-clip-"#));
        assert!(svg.contains(r#"stroke-dasharray="3 2""#));

        let tree = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()).unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(40, 20).unwrap();
        resvg::render(
            &tree,
            tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        let clipped_out = pixmap.pixel(1, 1).unwrap();
        let hole = pixmap.pixel(20, 10).unwrap();
        let red_side = pixmap.pixel(12, 10).unwrap();
        let blue_side = pixmap.pixel(28, 10).unwrap();
        assert_eq!(clipped_out.alpha(), 0);
        assert_eq!(hole.alpha(), 0);
        assert!((145..=160).contains(&red_side.alpha()));
        assert!((145..=160).contains(&blue_side.alpha()));
        assert!(red_side.red() > red_side.blue());
        assert!(blue_side.blue() > blue_side.red());
    }

    #[test]
    fn rejects_external_resource_references_before_normalization() {
        for svg in [
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <rect width="10" height="10" fill="#f00"/>
                <use href="other.svg#shape"/>
            </svg>"##,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <style>rect { fill: url(https://example.invalid/paint.svg#gradient); }</style>
                <rect width="10" height="10"/>
            </svg>"##,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <rect width="10" height="10" clip-path="url('clips.svg#clip')"/>
            </svg>"##,
        ] {
            let error = import_svg(svg, "external.svg").unwrap_err();
            assert!(error.to_string().contains("external resource references"));
        }
    }

    #[test]
    fn rejects_duplicate_and_ambiguous_reference_ids_before_normalization() {
        let duplicate = source_preflight_error(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <path id="shape" d="M0 0h1v1z"/>
                <path id="shape" d="M2 2h1v1z"/>
            </svg>"##,
        );
        assert!(duplicate.to_string().contains("duplicate id #shape"));

        let dual_href = source_preflight_error(
            r##"<svg xmlns="http://www.w3.org/2000/svg"
                        xmlns:xlink="http://www.w3.org/1999/xlink">
                <defs>
                    <path id="modern" d="M0 0h1v1z"/>
                    <path id="legacy" d="M2 2h1v1z"/>
                </defs>
                <use href="#modern" xlink:href="#legacy"/>
            </svg>"##,
        );
        assert!(dual_href
            .to_string()
            .contains("simultaneous href and xlink:href"));
    }

    #[test]
    fn rejects_styles_and_svg_ids_hidden_inside_metadata() {
        for (svg, expected) in [
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <metadata><style>path:not(:not(:not(.x))) { fill: red; }</style></metadata>
                    <path d="M0 0h1v1z"/>
                </svg>"##,
                "style sheets inside <metadata>",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <metadata><g id="shape"><image href="/tmp/private.png"/></g></metadata>
                    <path id="shape" d="M0 0h1v1z"/>
                    <use href="#shape"/>
                </svg>"##,
                "SVG IDs inside <metadata>",
            ),
        ] {
            assert!(source_preflight_error(svg).to_string().contains(expected));
        }
    }

    #[test]
    fn metadata_descendant_preflight_enforces_nesting_in_linear_walk() {
        let nested = format!(
            "{}{}",
            "<foreign>".repeat(MAX_SVG_NESTING_DEPTH + 1),
            "</foreign>".repeat(MAX_SVG_NESTING_DEPTH + 1)
        );
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><metadata>{nested}</metadata></svg>"#
        );
        assert!(matches!(
            source_preflight_error(&svg),
            SvgImportError::Complexity {
                resource: "nesting depth",
                limit: MAX_SVG_NESTING_DEPTH,
            }
        ));
    }

    #[test]
    fn normalization_options_deny_embedded_and_external_images() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">
            <image width="1" height="1"
                href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADElEQVR42mP8/58BAQEDAQDJ/pLvAAAAAElFTkSuQmCC"/>
        </svg>"#;
        let tree = usvg::Tree::from_str(svg, &import_options()).unwrap();
        assert!(tree.root().children().is_empty());
    }

    #[test]
    fn rejects_missing_and_wrong_type_local_references_before_normalization() {
        let cases = [
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg"><use href="#missing"/></svg>"##,
                "missing local use target #missing",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <defs><linearGradient id="target"><stop/><stop offset="1"/></linearGradient></defs>
                    <use href="#target"/>
                </svg>"##,
                "local use target #target must reference a renderable SVG element",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <rect id="target" width="1" height="1"/>
                    <text><textPath href="#target">x</textPath></text>
                </svg>"##,
                "local text path target #target must reference a <path>",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <path id="target" d="M0 0h1v1z"/>
                    <linearGradient id="paint" href="#target"><stop/><stop offset="1"/></linearGradient>
                </svg>"##,
                "local gradient inheritance target #target must reference",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <defs><clipPath id="target"><rect width="1" height="1"/></clipPath></defs>
                    <rect width="1" height="1" fill="url(#target)"/>
                </svg>"##,
                "local paint server target #target must reference",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <defs><linearGradient id="target"><stop/><stop offset="1"/></linearGradient></defs>
                    <rect width="1" height="1" clip-path="url(#target)"/>
                </svg>"##,
                "local clipping path target #target must reference a <clipPath>",
            ),
            (
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <defs><clipPath id="target"><rect width="1" height="1"/></clipPath></defs>
                    <path d="M0 0L1 1" marker-end="url(#target)"/>
                </svg>"##,
                "local marker target #target must reference a <marker>",
            ),
        ];

        for (svg, expected) in cases {
            let error = source_preflight_error(svg);
            assert!(
                error.to_string().contains(expected),
                "expected {expected:?}, got {error}"
            );
        }
    }

    #[test]
    fn rejects_local_reference_cycles_before_normalization() {
        for svg in [
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs>
                    <g id="a"><use href="#b"/></g>
                    <g id="b"><use href="#a"/></g>
                </defs>
                <use href="#a"/>
            </svg>"##,
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs>
                    <linearGradient id="a" href="#b"><stop/><stop offset="1"/></linearGradient>
                    <linearGradient id="b" href="#a"><stop/><stop offset="1"/></linearGradient>
                </defs>
                <rect width="1" height="1" fill="url(#a)"/>
            </svg>"##,
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs>
                    <clipPath id="a" clip-path="url(#b)"><rect width="1" height="1"/></clipPath>
                    <clipPath id="b" clip-path="url(#a)"><rect width="1" height="1"/></clipPath>
                </defs>
                <rect width="1" height="1" clip-path="url(#a)"/>
            </svg>"##,
        ] {
            let error = source_preflight_error(svg);
            assert!(error
                .to_string()
                .contains("cyclic local resource references"));
        }
    }

    #[test]
    fn accepts_typed_local_references_in_simple_css_rules() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <style>.paint { fill: url(#gradient); }</style>
                <defs>
                    <linearGradient id="gradient">
                        <stop stop-color="#f00"/><stop offset="1" stop-color="#00f"/>
                    </linearGradient>
                </defs>
                <rect class="paint" width="10" height="10"/>
            </svg>"##,
            "css-resource.svg",
        )
        .unwrap();

        assert!(document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .any(|node| matches!(node.style.fill, Some(Paint::LinearGradient(_)))));
    }

    #[test]
    fn bounds_dash_lists_in_attributes_inline_styles_and_style_sheets_before_normalization() {
        let allowed = std::iter::repeat_n("1", MAX_SOURCE_LIST_VALUES)
            .collect::<Vec<_>>()
            .join(" ");
        let allowed_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L1 1" stroke="black" stroke-dasharray="{allowed}"/></svg>"#
        );
        source_preflight(&allowed_svg).unwrap();

        let excessive = format!("{}-1", "0-1 ".repeat(MAX_SOURCE_LIST_VALUES / 2));
        for svg in [
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L1 1" stroke-dasharray="{excessive}"/></svg>"#
            ),
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L1 1" style="stroke-dasharray: {excessive}"/></svg>"#
            ),
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><style>path {{ stroke-dasharray: {excessive}; }}</style><path d="M0 0L1 1"/></svg>"#
            ),
        ] {
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "values in one source list",
                    limit: MAX_SOURCE_LIST_VALUES,
                }
            ));
        }
    }

    #[test]
    fn bounds_inherited_object_bounding_box_gradient_expansion_before_normalization() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        for styled_group in [
            format!(r#"<g fill="url(#paint)">{rects}</g>"#),
            format!(r#"<g style="fill: url(#paint)">{rects}</g>"#),
            format!(
                r#"<style>.painted {{ fill: url(#paint); }}</style><g class="painted">{rects}</g>"#
            ),
        ] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                    {styled_group}
                </svg>"##
            );
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn inherited_user_space_gradient_remains_shared_across_consumers() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs>
                    <linearGradient id="base" gradientUnits="userSpaceOnUse">{stops}</linearGradient>
                    <linearGradient id="paint" href="#base"/>
                </defs>
                <g fill="url(#paint)">{rects}</g>
            </svg>"##
        );

        source_preflight(&svg).unwrap();
    }

    #[test]
    fn descendant_paint_overrides_do_not_inherit_gradient_expansion() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1" fill="red"/>"#.repeat(245);
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                <g fill="url(#paint)">{rects}</g>
            </svg>"##
        );

        source_preflight(&svg).unwrap();
    }

    #[test]
    fn bounds_inherited_dash_array_expansion_before_normalization() {
        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        let paths = r#"<path d="M0 0h1"/>"#.repeat(62);
        for styled_group in [
            format!(r#"<g stroke="black" stroke-dasharray="{dash_values}">{paths}</g>"#),
            format!(r#"<g style="stroke: black; stroke-dasharray: {dash_values}">{paths}</g>"#),
            format!(
                r#"<style>.dashed {{ stroke: black; stroke-dasharray: {dash_values}; }}</style><g class="dashed">{paths}</g>"#
            ),
        ] {
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{styled_group}</svg>"#);
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn css_important_inherited_dash_cannot_be_hidden_by_inline_resets() {
        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        let paths = r#"<path d="M0 0h1"/>"#.repeat(62);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <style>.dashed {{ stroke: black ! important; stroke-dasharray: {dash_values} ! important; }}</style>
                <g class="dashed" style="stroke: none; stroke-dasharray: none">{paths}</g>
            </svg>"#
        );

        assert!(matches!(
            source_preflight_error(&svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));
    }

    #[test]
    fn first_inline_important_resource_declaration_remains_bounded() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        let gradient_svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                <g style="fill: url(#paint) !important; fill: red !important">{rects}</g>
            </svg>"##
        );
        assert!(matches!(
            source_preflight_error(&gradient_svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));

        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        let paths = r#"<path d="M0 0h1"/>"#.repeat(62);
        let dash_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <g style="stroke: black; stroke-dasharray: {dash_values} !important; stroke-dasharray: none !important">{paths}</g>
            </svg>"#
        );
        assert!(matches!(
            source_preflight_error(&dash_svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));
    }

    #[test]
    fn malformed_bang_stops_inline_and_stylesheet_declaration_blocks() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        for styled_group in [
            format!(r#"<g fill="url(#paint)" style="stroke: red !foo; fill: red">{rects}</g>"#),
            format!(
                r#"<style>.victim {{ stroke: red !foo; fill: red; }}</style><g class="victim" fill="url(#paint)">{rects}</g>"#
            ),
        ] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                    {styled_group}
                </svg>"##
            );
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }

        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        let paths = r#"<path d="M0 0h1"/>"#.repeat(62);
        for styled_group in [
            format!(
                r#"<g stroke="black" stroke-dasharray="{dash_values}" style="fill: red !foo; stroke: none">{paths}</g>"#
            ),
            format!(
                r#"<style>.victim {{ fill: red !foo; stroke: none; }}</style><g class="victim" stroke="black" stroke-dasharray="{dash_values}">{paths}</g>"#
            ),
        ] {
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{styled_group}</svg>"#);
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn locked_declaration_tokenizer_stops_after_invalid_declarations() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        for declarations in [
            "fill: ; fill: red",
            ": invalid; fill: red",
            "color: outer(inner()); fill: red",
        ] {
            for styled_group in [
                format!(r#"<g fill="url(#paint)" style="{declarations}">{rects}</g>"#),
                format!(
                    r#"<style>.victim {{ {declarations}; }}</style><g class="victim" fill="url(#paint)">{rects}</g>"#
                ),
            ] {
                let svg = format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg">
                        <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                        {styled_group}
                    </svg>"##
                );
                assert!(matches!(
                    source_preflight_error(&svg),
                    SvgImportError::Complexity {
                        resource: "pre-normalization expansion work",
                        limit: MAX_PRE_NORMALIZATION_WORK,
                    }
                ));
            }
        }

        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        let paths = r#"<path d="M0 0h1"/>"#.repeat(62);
        for styled_group in [
            format!(
                r#"<g stroke="black" stroke-dasharray="{dash_values}" style="stroke: ; stroke: none">{paths}</g>"#
            ),
            format!(
                r#"<style>.victim {{ stroke: ; stroke: none; }}</style><g class="victim" stroke="black" stroke-dasharray="{dash_values}">{paths}</g>"#
            ),
        ] {
            let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{styled_group}</svg>"#);
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn locked_declaration_tokenizer_keeps_valid_following_declarations() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        for declarations in ["color: red; fill: red", "*fill: red"] {
            for styled_group in [
                format!(r#"<g fill="url(#paint)" style="{declarations}">{rects}</g>"#),
                format!(
                    r#"<style>.victim {{ {declarations}; }}</style><g class="victim" fill="url(#paint)">{rects}</g>"#
                ),
            ] {
                let svg = format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg">
                        <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                        {styled_group}
                    </svg>"##
                );
                source_preflight(&svg).unwrap();
            }
        }

        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        let paths = r#"<path d="M0 0h1"/>"#.repeat(62);
        for declarations in ["color: red; stroke: none", "*stroke: none"] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg">
                    <g stroke="black" stroke-dasharray="{dash_values}" style="{declarations}">{paths}</g>
                </svg>"#
            );
            source_preflight(&svg).unwrap();
        }
    }

    #[test]
    fn descendant_selector_inheritance_is_charged_before_resource_expansion() {
        let stops = "<stop/>".repeat(1_024);
        let paths = r#"<path d="M0 0h1"/>"#.repeat(245);
        let gradient_svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <style>path {{ fill: red; }} g path {{ fill: inherit; }}</style>
                <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                <g fill="url(#paint)">{paths}</g>
            </svg>"##
        );
        assert!(matches!(
            source_preflight_error(&gradient_svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));

        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        let paths = r#"<path d="M0 0h1"/>"#.repeat(62);
        let dash_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <style>path {{ stroke: none; }} g path {{ stroke: inherit; }}</style>
                <g stroke="black" stroke-dasharray="{dash_values}">{paths}</g>
            </svg>"#
        );
        assert!(matches!(
            source_preflight_error(&dash_svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));
    }

    #[test]
    fn bounds_selector_bytes_times_candidate_elements_before_matching() {
        let selector = ".a".repeat(2_000);
        let paths = r#"<path class="a" d="M0 0h1"/>"#.repeat(62);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <style>{selector} {{ stroke-dasharray: none; }}</style>
                {paths}
            </svg>"#
        );

        assert!(matches!(
            source_preflight_error(&svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));
    }

    #[test]
    fn bounds_deep_descendant_selectors_with_harmless_declarations() {
        let group_count = 64;
        let selector = std::iter::repeat_n("*", group_count + 3)
            .collect::<Vec<_>>()
            .join(" ");
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <style>{selector} {{ opacity: 1; }}</style>
                {}<path d="M0 0h1"/>{}
            </svg>"#,
            "<g>".repeat(group_count),
            "</g>".repeat(group_count),
        );

        assert!(source_preflight_error(&svg)
            .to_string()
            .contains("outside bounded type/class/ID compounds"));
    }

    #[test]
    fn rejects_recursive_backtracking_descendant_selectors_before_matching() {
        let selector = format!(
            ".missing {}",
            std::iter::repeat_n("*", 24).collect::<Vec<_>>().join(" ")
        );
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <style>{selector} {{ fill: red; }}</style>
                {}<path d="M0 0h1"/>{}
            </svg>"#,
            "<g>".repeat(24),
            "</g>".repeat(24),
        );
        for error in [
            source_preflight_error(&svg),
            import_svg(&svg, "backtracking.svg").unwrap_err(),
        ] {
            assert!(error
                .to_string()
                .contains("outside bounded type/class/ID compounds"));
        }
    }

    #[test]
    fn imports_one_bounded_descendant_selector() {
        let document = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
                <style>
                    g path.target { fill: #123456; stroke: #abcdef; }
                </style>
                <g><path id="target" class="target" d="M0 0h10v10H0z"/></g>
            </svg>"##,
            "selectors.svg",
        )
        .unwrap();
        let target = document
            .descendants(document.root)
            .filter_map(|id| document.get(id))
            .find(|node| node.name == "target")
            .unwrap();
        assert_eq!(
            target.style.fill,
            Some(Paint::rgb(
                0x12 as f32 / 255.0,
                0x34 as f32 / 255.0,
                0x56 as f32 / 255.0,
            ))
        );
        assert_eq!(
            target.style.stroke.as_ref().map(|stroke| &stroke.paint),
            Some(&Paint::rgb(
                0xab as f32 / 255.0,
                0xcd as f32 / 255.0,
                0xef as f32 / 255.0,
            ))
        );
    }

    #[test]
    fn rejects_attribute_pseudo_child_and_adjacent_selectors_before_matching() {
        for selector in [
            "[foo~=never]",
            "path:first-child",
            "g > path",
            "path + path",
        ] {
            let huge_attribute = "x".repeat(512 * 1024);
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg">
                    <style>{selector} {{ opacity: 1; }}</style>
                    <path foo="{huge_attribute}" d="M0 0h1"/>
                </svg>"#
            );
            assert!(source_preflight_error(&svg)
                .to_string()
                .contains("outside bounded type/class/ID compounds"));
        }
    }

    #[test]
    fn selector_accounting_does_not_charge_uninspected_geometry_bytes() {
        let whitespace = " ".repeat(300 * 1024);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <style>.target {{ fill: red; }}</style>
                <path class="target" d="M0 0{whitespace}L1 1"/>
            </svg>"#
        );
        source_preflight(&svg).unwrap();
    }

    #[test]
    fn bounds_harmless_stylesheet_selector_and_declaration_work() {
        let paths = r#"<path class="a" d="M0 0h1"/>"#.repeat(62);
        let repeated_selector = ".a".repeat(2_000);
        let repeated_declarations = "opacity: 1;".repeat(512);
        for svg in [
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg">
                    <style>{repeated_selector} {{ opacity: 1; }}</style>
                    {paths}
                </svg>"#
            ),
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg">
                    <style>.a {{ {repeated_declarations} }}</style>
                    {paths}
                </svg>"#
            ),
        ] {
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn locked_selector_tokenizer_rejects_invalid_identifiers() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        for (selector, identity) in [
            (".1a", r#"class="1a""#),
            ("#1a", r#"id="1a""#),
            (".-1", r#"class="-1""#),
            (".--x", r#"class="--x""#),
        ] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <style>{selector} {{ fill: red; }}</style>
                    <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                    <g {identity} fill="url(#paint)">{rects}</g>
                </svg>"##
            );
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn locked_selector_tokenizer_accepts_valid_identifier_boundaries() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        for (selector, identity) in [
            ("._a", r#"class="_a""#),
            (".-a", r#"class="-a""#),
            ("#_a", r#"id="_a""#),
            ("#-a", r#"id="-a""#),
        ] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <style>{selector} {{ fill: red; }}</style>
                    <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                    <g {identity} fill="url(#paint)">{rects}</g>
                </svg>"##
            );
            source_preflight(&svg).unwrap();
        }
    }

    #[test]
    fn stylesheet_important_order_matches_locked_usvg_cascade() {
        let stops = "<stop/>".repeat(1_024);
        let rects = r#"<rect width="1" height="1"/>"#.repeat(245);
        for styling in [
            r#"<style>
                    #target { fill: red !important; }
                    .painted { fill: url(#paint) !important; }
                </style>
                <g id="target" class="painted">{RECTS}</g>"#
                .replace("{RECTS}", &rects),
            r#"<style>
                    .painted { fill: url(#paint) !important; }
                    .painted { fill: red !important; }
                </style>
                <g class="painted">{RECTS}</g>"#
                .replace("{RECTS}", &rects),
            r#"<style>.painted { fill: url(#paint) !important; }</style>
                <g class="painted" style="fill: red !important">{RECTS}</g>"#
                .replace("{RECTS}", &rects),
        ] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                    {styling}
                </svg>"##
            );
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn use_expansion_counts_referenced_text_consumers() {
        let stops = "<stop/>".repeat(1_024);
        let labels = "<text>x</text>".repeat(245);
        let gradient_svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs>
                    <linearGradient id="paint">{stops}</linearGradient>
                    <g id="labels">{labels}</g>
                </defs>
                <use href="#labels" fill="url(#paint)"/>
            </svg>"##
        );
        assert!(matches!(
            source_preflight_error(&gradient_svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));

        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        for referenced_text in [
            "<text><tspan>x</tspan></text>".repeat(62),
            r##"<text><textPath href="#baseline">x</textPath></text>"##.repeat(62),
        ] {
            let dash_svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg">
                    <defs>
                        <path id="baseline" d="M0 0h10"/>
                        <g id="labels">{referenced_text}</g>
                    </defs>
                    <use href="#labels" stroke="black" stroke-dasharray="{dash_values}"/>
                </svg>"##
            );
            assert!(matches!(
                source_preflight_error(&dash_svg),
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn css_comments_cannot_hide_source_list_limits_before_normalization() {
        let excessive = std::iter::repeat_n("1", MAX_SOURCE_LIST_VALUES + 1)
            .collect::<Vec<_>>()
            .join(" ");
        for svg in [
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><path style="/* ; */ stroke-dasharray: {excessive}"/></svg>"#
            ),
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><style>path {{ /* }} */ stroke-dasharray: {excessive}; }}</style><path/></svg>"#
            ),
        ] {
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "values in one source list",
                    limit: MAX_SOURCE_LIST_VALUES,
                }
            ));
        }
    }

    #[test]
    fn css_comments_cannot_hide_resource_references_or_unsupported_features() {
        let missing = source_preflight_error(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <rect style="/* ; */ fill: url(#missing)"/>
            </svg>"##,
        );
        assert!(missing
            .to_string()
            .contains("missing local paint server target #missing"));

        let wrong_type = source_preflight_error(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <style>rect { /* } */ fill: url(#clip); }</style>
                <defs><clipPath id="clip"><rect width="1" height="1"/></clipPath></defs>
                <rect width="1" height="1"/>
            </svg>"##,
        );
        assert!(wrong_type
            .to_string()
            .contains("local paint server target #clip must reference"));

        let cycle = source_preflight_error(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs>
                    <clipPath id="a" style="/* ; */ clip-path: url(#b)"><rect width="1" height="1"/></clipPath>
                    <clipPath id="b" style="clip-path: url(#a)"><rect width="1" height="1"/></clipPath>
                </defs>
                <rect width="1" height="1" clip-path="url(#a)"/>
            </svg>"##,
        );
        assert!(cycle
            .to_string()
            .contains("cyclic local resource references"));

        let external = source_preflight_error(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect style="/* ; */ fill: url(https://example.invalid/paint.svg)"/></svg>"#,
        );
        assert!(external
            .to_string()
            .contains("external resource references"));

        let unsupported = source_preflight_error(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect style="/* ; */ mix-blend-mode: multiply"/></svg>"#,
        );
        assert!(unsupported.to_string().contains("blend modes"));
    }

    #[test]
    fn rejects_unterminated_css_comments_but_preserves_comment_text_in_strings() {
        source_preflight(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect style="font-family: &quot;A/* } ; */B&quot;; fill: red"/></svg>"#,
        )
        .unwrap();

        for svg in [
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect style="fill: red; /*"/></svg>"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><style>rect { fill: red; /*</style><rect/></svg>"#,
        ] {
            assert!(source_preflight_error(svg)
                .to_string()
                .contains("unterminated CSS comment"));
        }
    }

    #[test]
    fn quoted_css_urls_with_delimiters_cannot_bypass_resource_preflight() {
        let missing = source_preflight_error(
            r###"<svg xmlns="http://www.w3.org/2000/svg">
                <rect style='fill: url("#missing;)")'/>
            </svg>"###,
        );
        assert!(missing
            .to_string()
            .contains("missing local paint server target #missing;)"));

        let wrong_type = source_preflight_error(
            r###"<svg xmlns="http://www.w3.org/2000/svg">
                <defs><clipPath id="clip;)"><rect width="1" height="1"/></clipPath></defs>
                <rect style='fill: url("#clip;)")'/>
            </svg>"###,
        );
        assert!(
            wrong_type
                .to_string()
                .contains("local paint server target #clip;) must reference"),
            "got {wrong_type}"
        );

        let external = source_preflight_error(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect style='fill: url("https://example.invalid/a;b)")'/></svg>"#,
        );
        assert!(external
            .to_string()
            .contains("external resource references"));

        let cycle = source_preflight_error(
            r###"<svg xmlns="http://www.w3.org/2000/svg">
                <defs>
                    <clipPath id="a;)" style='clip-path: url("#b;)")'><rect width="1" height="1"/></clipPath>
                    <clipPath id="b;)" style='clip-path: url("#a;)")'><rect width="1" height="1"/></clipPath>
                </defs>
                <rect clip-path='url("#a;)")'/>
            </svg>"###,
        );
        assert!(cycle
            .to_string()
            .contains("cyclic local resource references"));
    }

    #[test]
    fn css_delimiters_and_url_text_inside_strings_remain_opaque() {
        source_preflight(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <style>path { font-family: "literal ; } ) url(https://example.invalid)"; stroke-dasharray: 1 2; }</style>
                <path style='font-family: "inline ; } ) url(https://example.invalid)"; stroke-dasharray: 3 4'/>
            </svg>"#,
        )
        .unwrap();
    }

    #[test]
    fn quoted_css_delimiters_cannot_hide_oversized_lists() {
        let excessive = std::iter::repeat_n("1", MAX_SOURCE_LIST_VALUES + 1)
            .collect::<Vec<_>>()
            .join(" ");
        for svg in [
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><path style='font-family: "semi;colon"; stroke-dasharray: {excessive}'/></svg>"#
            ),
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><style>path {{ font-family: "brace }} paren )"; stroke-dasharray: {excessive}; }}</style><path/></svg>"#
            ),
        ] {
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "values in one source list",
                    limit: MAX_SOURCE_LIST_VALUES,
                }
            ));
        }
    }

    #[test]
    fn bounds_every_text_position_list_before_normalization() {
        let values = std::iter::repeat_n("0", MAX_SOURCE_LIST_VALUES + 1)
            .collect::<Vec<_>>()
            .join(",");
        for attribute in ["x", "y", "dx", "dy", "rotate"] {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg"><text {attribute}="{values}">x</text></svg>"#
            );
            assert!(matches!(
                source_preflight_error(&svg),
                SvgImportError::Complexity {
                    resource: "values in one source list",
                    limit: MAX_SOURCE_LIST_VALUES,
                }
            ));
        }
    }

    #[test]
    fn bounds_point_lists_and_aggregate_source_list_values_before_normalization() {
        let excessive_points = std::iter::repeat_n("0", MAX_SOURCE_LIST_VALUES + 1)
            .collect::<Vec<_>>()
            .join(" ");
        let points_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><polyline points="{excessive_points}"/></svg>"#
        );
        assert!(matches!(
            source_preflight_error(&points_svg),
            SvgImportError::Complexity {
                resource: "values in one source list",
                limit: MAX_SOURCE_LIST_VALUES,
            }
        ));

        let values = std::iter::repeat_n("1", MAX_SOURCE_LIST_VALUES)
            .collect::<Vec<_>>()
            .join(" ");
        let paths = format!(r#"<path stroke-dasharray="{values}"/>"#)
            .repeat(MAX_TOTAL_SOURCE_LIST_VALUES / MAX_SOURCE_LIST_VALUES + 1);
        let aggregate_svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{paths}</svg>"#);
        assert!(matches!(
            source_preflight_error(&aggregate_svg),
            SvgImportError::Complexity {
                resource: "total source list values",
                limit: MAX_TOTAL_SOURCE_LIST_VALUES,
            }
        ));
    }

    #[test]
    fn rejects_repeated_use_expansion_before_normalization() {
        let mut definitions = String::from(r#"<g id="level0"><path d="M0 0h1v1z"/></g>"#);
        for level in 1..=6 {
            let uses = format!(r##"<use href="#level{}"/>"##, level - 1).repeat(8);
            definitions.push_str(&format!(r#"<g id="level{level}">{uses}</g>"#));
        }
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg"><defs>{definitions}</defs><use href="#level6"/></svg>"##
        );

        let error = source_preflight_error(&svg);
        assert!(matches!(
            error,
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));
    }

    #[test]
    fn rejects_marker_expansion_before_normalization() {
        let marker_paths = r#"<path d="M0 0h1v1z"/>"#.repeat(16);
        let marked_path = format!("M0 0{}", "L1 1".repeat(4_000));
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs><marker id="marker">{marker_paths}</marker></defs>
                <g marker-mid="url(#marker)"><path d="{marked_path}"/></g>
            </svg>"##
        );

        let error = source_preflight_error(&svg);
        assert!(matches!(
            error,
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));
    }

    #[test]
    fn rejects_text_outline_expansion_before_normalization() {
        let text = "A".repeat(MAX_PRE_NORMALIZATION_WORK / TEXT_OUTLINE_WORK_PER_CHARACTER + 1);
        for content in [text.clone(), format!("<a>{text}</a>")] {
            let svg =
                format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><text>{content}</text></svg>");

            let error = source_preflight_error(&svg);
            assert!(matches!(
                error,
                SvgImportError::Complexity {
                    resource: "pre-normalization expansion work",
                    limit: MAX_PRE_NORMALIZATION_WORK,
                }
            ));
        }
    }

    #[test]
    fn link_wrapped_text_counts_inherited_paint_expansion() {
        let stops = "<stop/>".repeat(1_024);
        let links = "<a>x</a>".repeat(245);
        let gradient_svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg">
                <defs><linearGradient id="paint">{stops}</linearGradient></defs>
                <text fill="url(#paint)">{links}</text>
            </svg>"##
        );
        assert!(matches!(
            source_preflight_error(&gradient_svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));

        let dash_values = std::iter::repeat_n("1", 4_096)
            .collect::<Vec<_>>()
            .join(" ");
        let links = "<a>x</a>".repeat(62);
        let dash_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg">
                <text stroke="black" stroke-dasharray="{dash_values}">{links}</text>
            </svg>"#
        );
        assert!(matches!(
            source_preflight_error(&dash_svg),
            SvgImportError::Complexity {
                resource: "pre-normalization expansion work",
                limit: MAX_PRE_NORMALIZATION_WORK,
            }
        ));
    }

    #[test]
    fn still_rejects_patterns_after_normalization() {
        let error = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <defs><pattern id="p" width="2" height="2"><rect width="1" height="1"/></pattern></defs>
                <rect width="10" height="10" fill="url(#p)"/>
            </svg>"##,
            "pattern.svg",
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("unsupported SVG element <pattern>"));
    }

    #[test]
    fn rejects_svg2_features_that_usvg_would_drop() {
        let arcs = import_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <style>path { stroke-linejoin: arcs; }</style>
                <path d="M0 0L5 10L10 0" fill="none" stroke="black"/>
            </svg>"#,
            "arcs.svg",
        )
        .unwrap_err();
        assert!(arcs.to_string().contains("arcs stroke joins"));

        let focal_radius = import_svg(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
                <defs><radialGradient id="g" fr="10%"><stop/><stop offset="1"/></radialGradient></defs>
                <rect width="10" height="10" fill="url(#g)"/>
            </svg>"##,
            "focal-radius.svg",
        )
        .unwrap_err();
        assert!(focal_radius
            .to_string()
            .contains("radial gradient focal radii"));
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

    fn source_preflight(svg: &str) -> Result<(), SvgImportError> {
        let document = roxmltree::Document::parse_with_options(
            svg,
            roxmltree::ParsingOptions {
                allow_dtd: false,
                nodes_limit: MAX_XML_NODES,
            },
        )
        .unwrap();
        validate_source(&document)
    }

    fn source_preflight_error(svg: &str) -> SvgImportError {
        source_preflight(svg).unwrap_err()
    }
}
