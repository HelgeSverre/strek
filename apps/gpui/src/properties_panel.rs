//! Figma-style design inspector for the current selection.

use editor_core::{
    Direction, Editor, FrameData, Layout, NodeKind, NumericAdjustmentDirection,
    NumericAdjustmentMode, NumericPropertyTarget, Paint, Rect, TextAlign, TextData,
    TransformComponents,
};
use gpui::{
    div, prelude::*, px, rgb, rgba, Action, AnyElement, IntoElement, MouseButton, SharedString,
    WeakEntity, Window,
};

use crate::{
    color_picker::{format_paint, paint_as_rgba, parse_hex_paint},
    toolbar::{editor_tooltip, font_family_label, PortableFontFamily},
    AlignTextCenter, AlignTextLeft, AlignTextRight, PropertyFillBlack, PropertyFillBlue,
    PropertyFillGreen, PropertyFillNone, PropertyFillRed, PropertyFillWhite, PropertyRotateLeft,
    PropertyRotateRight, PropertyToggleStroke, SetLayoutFree, SetLayoutHorizontal,
    SetLayoutVertical, SetTextFamilyMonospace, SetTextFamilySerif, SetTextFamilySystem,
    StartFillColorInput, StartStrokeColorInput, Strek, TextWeightDown, TextWeightUp,
    ToggleFrameBackground, ToggleTextItalic,
};

const SURFACE: u32 = 0x202124;
const SURFACE_RAISED: u32 = 0x292a2e;
const SURFACE_HOVER: u32 = 0x35363b;
const BORDER: u32 = 0x414349;
const TEXT: u32 = 0xf1f3f4;
const MUTED: u32 = 0xa8abb2;
const ACCENT: u32 = 0x0c8ce9;

#[derive(Clone)]
pub(crate) struct PropertiesSnapshot {
    selection_count: usize,
    label: String,
    kind: String,
    bounds: Option<Rect>,
    transform: Option<TransformComponents>,
    group_layout: Option<Layout>,
    style: Option<SelectionStyle>,
    text: Option<TextData>,
    frame: Option<FrameData>,
    color_input: Option<ColorInputSnapshot>,
    numeric_input: Option<NumericInputSnapshot>,
}

#[derive(Clone)]
struct SelectionStyle {
    fill: SelectionValue<Option<Paint>>,
    stroke_enabled: SelectionValue<bool>,
    stroke_paint: SelectionValue<Option<Paint>>,
    stroke_width: SelectionValue<Option<f32>>,
    opacity: SelectionValue<f32>,
}

#[derive(Clone)]
enum SelectionValue<T> {
    Uniform(T),
    Mixed,
}

impl<T> SelectionValue<T> {
    fn uniform(&self) -> Option<&T> {
        match self {
            Self::Uniform(value) => Some(value),
            Self::Mixed => None,
        }
    }

    fn is_mixed(&self) -> bool {
        matches!(self, Self::Mixed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorTarget {
    Fill,
    Stroke,
}

#[derive(Debug, Clone)]
pub(crate) struct ColorInputSnapshot {
    pub target: ColorTarget,
    pub value: String,
    pub invalid: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct NumericInputSnapshot {
    pub target: NumericPropertyTarget,
    pub value: String,
    pub invalid: bool,
}

pub(crate) fn numeric_property_delta(
    target: NumericPropertyTarget,
    pointer_delta: f32,
    mode: NumericAdjustmentMode,
) -> f32 {
    pointer_delta * target.step(mode)
}

fn numeric_property_element_id(target: NumericPropertyTarget) -> &'static str {
    match target {
        NumericPropertyTarget::WorldX => "property-scrub-world-x",
        NumericPropertyTarget::WorldY => "property-scrub-world-y",
        NumericPropertyTarget::Opacity => "property-scrub-opacity",
        NumericPropertyTarget::StrokeWidth => "property-scrub-stroke-width",
        NumericPropertyTarget::TextSize => "property-scrub-text-size",
        NumericPropertyTarget::AutoLayoutSpacing => "property-scrub-layout-spacing",
        NumericPropertyTarget::AutoLayoutPadding => "property-scrub-layout-padding",
    }
}

pub(crate) fn snapshot(
    editor: &mut Editor,
    color_input: Option<ColorInputSnapshot>,
    numeric_input: Option<NumericInputSnapshot>,
) -> PropertiesSnapshot {
    let bounds = editor.selection_world_bounds();
    let selected = editor.selection().iter().collect::<Vec<_>>();
    let selection_count = selected.len();
    let primary = editor.selection().primary();
    let label = if selection_count == 1 {
        primary
            .and_then(|id| editor.document().get(id))
            .map(|node| node.name.clone())
            .unwrap_or_else(|| "Selection".to_owned())
    } else {
        format!("{selection_count} layers")
    };
    let kind = if selection_count == 1 {
        primary
            .and_then(|id| editor.document().get(id))
            .map(|node| match node.kind {
                NodeKind::Group => "Group",
                NodeKind::Frame(_) => "Frame",
                NodeKind::Shape(_) => "Vector",
                NodeKind::Text(_) => "Text",
            })
            .unwrap_or("Selection")
            .to_owned()
    } else {
        "Multiple selection".to_owned()
    };
    let style = selected.first().and_then(|_| {
        let styles = selected
            .iter()
            .filter_map(|id| editor.document().get(*id).map(|node| &node.style))
            .collect::<Vec<_>>();
        Some(SelectionStyle {
            fill: uniform_selection_value(styles.iter().map(|style| style.fill.clone()))?,
            stroke_enabled: uniform_selection_value(
                styles.iter().map(|style| style.stroke.is_some()),
            )?,
            stroke_paint: uniform_selection_value(
                styles
                    .iter()
                    .map(|style| style.stroke.as_ref().map(|stroke| stroke.paint.clone())),
            )?,
            stroke_width: uniform_selection_value(
                styles
                    .iter()
                    .map(|style| style.stroke.as_ref().map(|stroke| stroke.width)),
            )?,
            opacity: uniform_selection_value(styles.iter().map(|style| style.opacity))?,
        })
    });
    let text = (selection_count == 1)
        .then(|| editor.selected_text_data())
        .flatten();
    let frame = (selection_count == 1)
        .then(|| editor.selected_frame_data())
        .flatten();
    let transform = (selection_count == 1)
        .then(|| editor.selected_transform_components())
        .flatten();
    let group_layout = (selection_count == 1)
        .then(|| editor.selected_group_layout())
        .flatten();

    PropertiesSnapshot {
        selection_count,
        label,
        kind,
        bounds,
        transform,
        group_layout,
        style,
        text,
        frame,
        color_input,
        numeric_input,
    }
}

fn uniform_selection_value<T: Clone + PartialEq>(
    mut values: impl Iterator<Item = T>,
) -> Option<SelectionValue<T>> {
    let first = values.next()?;
    Some(if values.all(|value| value == first) {
        SelectionValue::Uniform(first)
    } else {
        SelectionValue::Mixed
    })
}

pub(crate) fn render(snapshot: PropertiesSnapshot, editor_entity: WeakEntity<Strek>) -> AnyElement {
    if snapshot.selection_count == 0 {
        return div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .px(px(24.0))
            .text_align(gpui::TextAlign::Center)
            .text_color(rgb(MUTED))
            .text_size(px(11.0))
            .child("Nothing selected")
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(0x8f929a))
                    .child("Select a layer to edit its design properties."),
            )
            .into_any_element();
    }

    let style = snapshot.style.as_ref();
    let fill = style
        .and_then(|style| style.fill.uniform())
        .and_then(Option::as_ref);
    let fill_mixed = style.is_some_and(|style| style.fill.is_mixed());
    let stroke_enabled = style
        .and_then(|style| style.stroke_enabled.uniform())
        .copied();
    let stroke_presence_mixed = style.is_some_and(|style| style.stroke_enabled.is_mixed());
    let stroke_paint = style
        .and_then(|style| style.stroke_paint.uniform())
        .and_then(Option::as_ref);
    let stroke_color_mixed =
        stroke_presence_mixed || style.is_some_and(|style| style.stroke_paint.is_mixed());
    let stroke_width = style
        .and_then(|style| style.stroke_width.uniform())
        .copied()
        .flatten();
    let stroke_width_mixed =
        stroke_presence_mixed || style.is_some_and(|style| style.stroke_width.is_mixed());
    let opacity = style.and_then(|style| style.opacity.uniform()).copied();
    let transform = snapshot.transform;
    let group_layout = snapshot.group_layout.clone();
    let numeric_input = snapshot.numeric_input.as_ref();

    div()
        .id("properties-panel")
        .flex_1()
        .overflow_y_scroll()
        .bg(rgb(SURFACE))
        .child(
            div()
                .px(px(12.0))
                .py(px(10.0))
                .border_b_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT))
                        .child(snapshot.label),
                )
                .child(
                    div()
                        .mt(px(2.0))
                        .text_size(px(9.0))
                        .text_color(rgb(MUTED))
                        .child(snapshot.kind),
                ),
        )
        .when_some(snapshot.bounds, |panel, bounds| {
            let position = transform
                .map(|components| components.translation)
                .unwrap_or(bounds.min);
            panel.child(
                section("Transform")
                    .child(numeric_stepper(
                        "X",
                        format_number(position.x),
                        "Move left",
                        "Move right",
                        NumericPropertyTarget::WorldX,
                        &editor_entity,
                        numeric_input,
                    ))
                    .child(numeric_stepper(
                        "Y",
                        format_number(position.y),
                        "Move up",
                        "Move down",
                        NumericPropertyTarget::WorldY,
                        &editor_entity,
                        numeric_input,
                    ))
                    .child(readonly_pair(
                        "W",
                        format_number(bounds.width()),
                        "H",
                        format_number(bounds.height()),
                    ))
                    .child(action_stepper(
                        "Rotate",
                        transform
                            .map(|components| format_degrees(components.rotation_radians))
                            .unwrap_or_else(|| "Mixed".to_owned()),
                        "Rotate counter-clockwise",
                        "Rotate clockwise",
                        PropertyRotateLeft,
                        PropertyRotateRight,
                    ))
                    .when_some(transform, |transform_section, components| {
                        transform_section
                            .child(readonly_pair(
                                "SX",
                                format_percent(components.scale.x),
                                "SY",
                                format_percent(components.scale.y),
                            ))
                            .child(div().px(px(10.0)).pb(px(8.0)).child(readonly_value(
                                "Skew",
                                format_degrees(components.skew_radians),
                            )))
                    }),
            )
        })
        .when_some(group_layout, |panel, layout| {
            panel.child(layout_section(layout, &editor_entity, numeric_input))
        })
        .child(
            section("Appearance")
                .child(numeric_stepper(
                    "Opacity",
                    opacity
                        .map(|opacity| format!("{:.0}%", opacity * 100.0))
                        .unwrap_or_else(|| "Mixed".to_owned()),
                    "Decrease opacity",
                    "Increase opacity",
                    NumericPropertyTarget::Opacity,
                    &editor_entity,
                    numeric_input,
                ))
                .child(property_label("Fill"))
                .child(color_value_editor(
                    "fill-color",
                    fill,
                    fill_mixed,
                    snapshot
                        .color_input
                        .as_ref()
                        .filter(|input| input.target == ColorTarget::Fill),
                    "Edit fill color",
                    StartFillColorInput,
                ))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(7.0))
                        .px(px(10.0))
                        .pb(px(10.0))
                        .child(color_swatch(
                            "fill-none",
                            None,
                            !fill_mixed && fill.is_none(),
                            "Remove fill",
                            PropertyFillNone,
                        ))
                        .child(color_swatch(
                            "fill-white",
                            Some(0xffffff),
                            paint_matches(fill, [1.0, 1.0, 1.0, 1.0]),
                            "White fill",
                            PropertyFillWhite,
                        ))
                        .child(color_swatch(
                            "fill-black",
                            Some(0x17181a),
                            paint_matches(fill, [0.0, 0.0, 0.0, 1.0]),
                            "Black fill",
                            PropertyFillBlack,
                        ))
                        .child(color_swatch(
                            "fill-blue",
                            Some(0x0c8ce9),
                            paint_matches(fill, [0.047, 0.55, 0.91, 1.0]),
                            "Blue fill",
                            PropertyFillBlue,
                        ))
                        .child(color_swatch(
                            "fill-red",
                            Some(0xf24822),
                            paint_matches(fill, [0.95, 0.28, 0.13, 1.0]),
                            "Red fill",
                            PropertyFillRed,
                        ))
                        .child(color_swatch(
                            "fill-green",
                            Some(0x14ae5c),
                            paint_matches(fill, [0.08, 0.68, 0.36, 1.0]),
                            "Green fill",
                            PropertyFillGreen,
                        )),
                )
                .child(
                    div()
                        .h(px(34.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(10.0))
                        .child(row_label("Stroke"))
                        .child(property_button(
                            "stroke-toggle",
                            if stroke_presence_mixed {
                                "Mixed"
                            } else if stroke_enabled == Some(true) {
                                "On"
                            } else {
                                "Off"
                            },
                            "Toggle stroke",
                            PropertyToggleStroke,
                        ))
                        .child(div().flex_1())
                        .child(numeric_property_button(
                            "stroke-down",
                            "−",
                            "Decrease stroke width",
                            NumericPropertyTarget::StrokeWidth,
                            NumericAdjustmentDirection::Decrease,
                            &editor_entity,
                        ))
                        .child(numeric_value(
                            if stroke_width_mixed {
                                "Mixed".to_owned()
                            } else {
                                stroke_width
                                    .map(format_number)
                                    .unwrap_or_else(|| "—".to_owned())
                            },
                            (!stroke_width_mixed)
                                .then_some(stroke_width)
                                .flatten()
                                .map(|_| (NumericPropertyTarget::StrokeWidth, &editor_entity)),
                            numeric_input
                                .filter(|input| input.target == NumericPropertyTarget::StrokeWidth),
                        ))
                        .child(numeric_property_button(
                            "stroke-up",
                            "+",
                            "Increase stroke width",
                            NumericPropertyTarget::StrokeWidth,
                            NumericAdjustmentDirection::Increase,
                            &editor_entity,
                        )),
                )
                .when(
                    stroke_presence_mixed || stroke_enabled == Some(true),
                    |appearance| {
                        appearance.child(color_value_editor(
                            "stroke-color",
                            stroke_paint,
                            stroke_color_mixed,
                            snapshot
                                .color_input
                                .as_ref()
                                .filter(|input| input.target == ColorTarget::Stroke),
                            "Edit stroke color",
                            StartStrokeColorInput,
                        ))
                    },
                ),
        )
        .when_some(snapshot.text, |panel, text| {
            panel.child(
                section("Typography")
                    .child(font_family_controls(&text))
                    .child(numeric_stepper(
                        "Size",
                        format_number(text.font_size),
                        "Decrease font size",
                        "Increase font size",
                        NumericPropertyTarget::TextSize,
                        &editor_entity,
                        numeric_input,
                    ))
                    .child(action_stepper(
                        "Weight",
                        text.font.weight.to_string(),
                        "Decrease font weight",
                        "Increase font weight",
                        TextWeightDown,
                        TextWeightUp,
                    ))
                    .child(
                        div()
                            .h(px(34.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(10.0))
                            .child(row_label("Style"))
                            .child(property_choice_button(
                                "text-italic",
                                "Italic",
                                "Toggle italic",
                                text.font.italic,
                                ToggleTextItalic,
                            )),
                    )
                    .child(
                        div()
                            .h(px(36.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(10.0))
                            .child(row_label("Align"))
                            .child(property_choice_button(
                                "text-left",
                                "Left",
                                "Align text left",
                                text.align == TextAlign::Left,
                                AlignTextLeft,
                            ))
                            .child(property_choice_button(
                                "text-center",
                                "Center",
                                "Align text center",
                                text.align == TextAlign::Center,
                                AlignTextCenter,
                            ))
                            .child(property_choice_button(
                                "text-right",
                                "Right",
                                "Align text right",
                                text.align == TextAlign::Right,
                                AlignTextRight,
                            )),
                    ),
            )
        })
        .when_some(snapshot.frame, |panel, frame| {
            panel.child(
                section("Frame")
                    .child(readonly_pair(
                        "W",
                        format_number(frame.width),
                        "H",
                        format_number(frame.height),
                    ))
                    .child(div().px(px(10.0)).pb(px(10.0)).child(property_button(
                        "frame-background",
                        if frame.background.is_some() {
                            "White background"
                        } else {
                            "Transparent background"
                        },
                        "Toggle frame background",
                        ToggleFrameBackground,
                    ))),
            )
        })
        .into_any_element()
}

fn layout_section(
    layout: Layout,
    editor_entity: &WeakEntity<Strek>,
    numeric_input: Option<&NumericInputSnapshot>,
) -> gpui::Div {
    let (mode, auto_layout) = match layout {
        Layout::Free => (None, None),
        Layout::Auto(layout) => (Some(layout.direction), Some(layout)),
    };

    section("Auto layout")
        .child(
            div()
                .h(px(36.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .px(px(10.0))
                .child(row_label("Mode"))
                .child(property_choice_button(
                    "layout-free",
                    "Free",
                    "Disable auto layout",
                    mode.is_none(),
                    SetLayoutFree,
                ))
                .child(property_choice_button(
                    "layout-horizontal",
                    "H",
                    "Horizontal auto layout",
                    mode == Some(Direction::Horizontal),
                    SetLayoutHorizontal,
                ))
                .child(property_choice_button(
                    "layout-vertical",
                    "V",
                    "Vertical auto layout",
                    mode == Some(Direction::Vertical),
                    SetLayoutVertical,
                )),
        )
        .when_some(auto_layout, |layout_panel, layout| {
            let padding = uniform_padding(&layout);
            layout_panel
                .child(numeric_stepper(
                    "Spacing",
                    format_number(layout.spacing),
                    "Decrease layout spacing",
                    "Increase layout spacing",
                    NumericPropertyTarget::AutoLayoutSpacing,
                    editor_entity,
                    numeric_input,
                ))
                .child(numeric_stepper(
                    "Padding",
                    padding
                        .map(format_number)
                        .unwrap_or_else(|| "Mixed".to_owned()),
                    "Decrease uniform padding",
                    "Increase uniform padding",
                    NumericPropertyTarget::AutoLayoutPadding,
                    editor_entity,
                    numeric_input,
                ))
        })
}

fn font_family_controls(text: &TextData) -> gpui::Div {
    let family = PortableFontFamily::from_name(&text.font.family);

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(34.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .px(px(10.0))
                .child(row_label("Family"))
                .child(
                    div()
                        .h(px(26.0))
                        .flex_1()
                        .flex()
                        .items_center()
                        .px(px(8.0))
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(SURFACE_RAISED))
                        .text_size(px(10.0))
                        .text_color(rgb(TEXT))
                        .child(font_family_label(&text.font.family)),
                ),
        )
        .child(
            div()
                .h(px(32.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .px(px(10.0))
                .child(div().w(px(62.0)))
                .child(property_choice_button(
                    "text-family-system",
                    "System",
                    "Use system sans-serif",
                    family == PortableFontFamily::System,
                    SetTextFamilySystem,
                ))
                .child(property_choice_button(
                    "text-family-serif",
                    "Serif",
                    "Use portable serif",
                    family == PortableFontFamily::Serif,
                    SetTextFamilySerif,
                ))
                .child(property_choice_button(
                    "text-family-monospace",
                    "Mono",
                    "Use portable monospace",
                    family == PortableFontFamily::Monospace,
                    SetTextFamilyMonospace,
                )),
        )
}

fn uniform_padding(layout: &editor_core::AutoLayout) -> Option<f32> {
    let padding = layout.padding;
    [padding.top, padding.right, padding.bottom]
        .into_iter()
        .all(|edge| (edge - padding.left).abs() <= f32::EPSILON)
        .then_some(padding.left)
}

fn section(title: &'static str) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .border_b_1()
        .border_color(rgb(BORDER))
        .child(
            div()
                .h(px(34.0))
                .flex()
                .items_center()
                .px(px(10.0))
                .text_size(px(10.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child(title),
        )
}

fn property_label(label: &'static str) -> impl IntoElement {
    div()
        .px(px(10.0))
        .pb(px(7.0))
        .text_size(px(10.0))
        .text_color(rgb(MUTED))
        .child(label)
}

fn row_label(label: &'static str) -> impl IntoElement {
    div()
        .w(px(62.0))
        .text_size(px(10.0))
        .text_color(rgb(MUTED))
        .child(label)
}

fn readonly_pair(
    first_label: &'static str,
    first_value: String,
    second_label: &'static str,
    second_value: String,
) -> impl IntoElement {
    div()
        .h(px(34.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(10.0))
        .child(readonly_value(first_label, first_value))
        .child(readonly_value(second_label, second_value))
}

fn readonly_value(label: &'static str, value: String) -> impl IntoElement {
    div()
        .flex_1()
        .h(px(26.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(7.0))
        .bg(rgb(SURFACE_RAISED))
        .rounded(px(4.0))
        .text_size(px(10.0))
        .child(div().text_color(rgb(MUTED)).child(label))
        .child(div().flex_1().text_color(rgb(TEXT)).child(value))
}

fn action_stepper<A: Action + Clone, B: Action + Clone>(
    label: &'static str,
    value: String,
    decrease_tooltip: &'static str,
    increase_tooltip: &'static str,
    decrease: A,
    increase: B,
) -> impl IntoElement {
    stepper_row(
        label,
        property_button("decrease", "−", decrease_tooltip, decrease).into_any_element(),
        numeric_value(value, None, None),
        property_button("increase", "+", increase_tooltip, increase).into_any_element(),
    )
}

fn numeric_stepper(
    label: &'static str,
    value: String,
    decrease_tooltip: &'static str,
    increase_tooltip: &'static str,
    target: NumericPropertyTarget,
    editor_entity: &WeakEntity<Strek>,
    numeric_input: Option<&NumericInputSnapshot>,
) -> impl IntoElement {
    stepper_row(
        label,
        numeric_property_button(
            "decrease",
            "−",
            decrease_tooltip,
            target,
            NumericAdjustmentDirection::Decrease,
            editor_entity,
        )
        .into_any_element(),
        numeric_value(
            value,
            Some((target, editor_entity)),
            numeric_input.filter(|input| input.target == target),
        ),
        numeric_property_button(
            "increase",
            "+",
            increase_tooltip,
            target,
            NumericAdjustmentDirection::Increase,
            editor_entity,
        )
        .into_any_element(),
    )
}

fn stepper_row(
    label: &'static str,
    decrease: AnyElement,
    value: AnyElement,
    increase: AnyElement,
) -> impl IntoElement {
    div()
        .h(px(34.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .px(px(10.0))
        .child(row_label(label))
        .child(decrease)
        .child(value)
        .child(increase)
}

fn numeric_value(
    value: String,
    scrub: Option<(NumericPropertyTarget, &WeakEntity<Strek>)>,
    input: Option<&NumericInputSnapshot>,
) -> AnyElement {
    if let Some(input) = input {
        return div()
            .id(numeric_property_element_id(input.target))
            .flex_1()
            .h(px(26.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(SURFACE_RAISED))
            .border_1()
            .border_color(rgb(if input.invalid { 0xf24822 } else { ACCENT }))
            .rounded(px(4.0))
            .text_size(px(10.0))
            .text_color(rgb(TEXT))
            .cursor_text()
            .child(input.value.clone())
            .tooltip(editor_tooltip(
                "Type a value and press Enter; Escape cancels",
                None,
            ))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .into_any_element();
    }

    let value = div()
        .flex_1()
        .h(px(26.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(SURFACE_RAISED))
        .rounded(px(4.0))
        .text_size(px(10.0))
        .text_color(rgb(TEXT))
        .child(value);

    let Some((target, editor_entity)) = scrub else {
        return value.into_any_element();
    };
    let editor_entity = editor_entity.clone();
    value
        .id(numeric_property_element_id(target))
        .cursor_col_resize()
        .tooltip(editor_tooltip(
            "Drag horizontally to adjust; double-click to type; hold Shift for coarse clean numbers; Escape cancels",
            None,
        ))
        .on_mouse_down(MouseButton::Left, {
            let editor_entity = editor_entity.clone();
            move |event, window, cx| {
                if event.click_count >= 2 {
                    editor_entity
                        .update(cx, |editor, cx| {
                            editor.start_numeric_property_input(target, window, cx);
                        })
                        .ok();
                } else {
                    let pointer_x = event.position.x.0;
                    editor_entity
                        .update(cx, |editor, cx| {
                            editor.begin_numeric_property_scrub(target, pointer_x, window, cx);
                        })
                        .ok();
                }
                cx.stop_propagation();
            }
        })
        .into_any_element()
}

fn property_button<A: Action + Clone>(
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    action: A,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!(
            "property-{id}-{}",
            tooltip.replace(' ', "-").to_lowercase()
        )))
        .h(px(26.0))
        .min_w(px(26.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(rgb(SURFACE_RAISED))
        .text_size(px(10.0))
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(label)
        .tooltip(editor_tooltip(tooltip, None))
        .on_click(move |_, window: &mut Window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn numeric_property_button(
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    target: NumericPropertyTarget,
    direction: NumericAdjustmentDirection,
    editor_entity: &WeakEntity<Strek>,
) -> impl IntoElement {
    let editor_entity = editor_entity.clone();
    div()
        .id(SharedString::from(format!(
            "property-{id}-{}",
            tooltip.replace(' ', "-").to_lowercase()
        )))
        .h(px(26.0))
        .min_w(px(26.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(rgb(SURFACE_RAISED))
        .text_size(px(10.0))
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(label)
        .tooltip(editor_tooltip(tooltip, None))
        .on_click(move |event, _window: &mut Window, cx| {
            let mode = if event.modifiers().shift {
                NumericAdjustmentMode::Coarse
            } else {
                NumericAdjustmentMode::Fine
            };
            editor_entity
                .update(cx, |editor, cx| {
                    editor.adjust_numeric_property(target, direction, mode, cx);
                })
                .ok();
        })
}

fn property_choice_button<A: Action + Clone>(
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    active: bool,
    action: A,
) -> impl IntoElement {
    div()
        .id(id)
        .h(px(26.0))
        .min_w(px(26.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(rgb(if active { ACCENT } else { BORDER }))
        .bg(if active {
            rgba(0x0c8ce92e)
        } else {
            rgba(0x00000000)
        })
        .text_size(px(9.0))
        .text_color(rgb(if active { 0xd8efff } else { TEXT }))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(label)
        .tooltip(editor_tooltip(tooltip, None))
        .on_click(move |_, window: &mut Window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn color_swatch<A: Action + Clone>(
    id: &'static str,
    color: Option<u32>,
    active: bool,
    tooltip: &'static str,
    action: A,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(24.0))
        .h(px(24.0))
        .p(px(2.0))
        .border_1()
        .border_color(rgb(if active { ACCENT } else { BORDER }))
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|style| style.border_color(rgb(ACCENT)))
        .tooltip(editor_tooltip(tooltip, None))
        .child(
            div()
                .w_full()
                .h_full()
                .rounded(px(3.0))
                .bg(color.map(rgb).unwrap_or_else(|| rgba(0x00000000)))
                .when(color.is_none(), |swatch| {
                    swatch
                        .border_1()
                        .border_color(rgb(0xf24822))
                        .child(div().w_full().h(px(1.0)).mt(px(8.0)).bg(rgb(0xf24822)))
                }),
        )
        .on_click(move |_, window: &mut Window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn color_value_editor<A: Action + Clone>(
    id: &'static str,
    paint: Option<&Paint>,
    mixed: bool,
    active: Option<&ColorInputSnapshot>,
    tooltip: &'static str,
    action: A,
) -> impl IntoElement {
    let active_paint = active.and_then(|input| parse_hex_paint(&input.value));
    let preview = active_paint.as_ref().or(if mixed { None } else { paint });
    let value = active.map(|input| input.value.clone()).unwrap_or_else(|| {
        if mixed {
            "Mixed".to_owned()
        } else {
            paint.map(format_paint).unwrap_or_else(|| "None".to_owned())
        }
    });
    let border = if active.is_some_and(|input| input.invalid) {
        0xf24822
    } else if active.is_some() {
        ACCENT
    } else {
        BORDER
    };

    div()
        .id(id)
        .h(px(34.0))
        .mx(px(10.0))
        .mb(px(7.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(SURFACE_RAISED))
        .cursor_text()
        .hover(|style| style.border_color(rgb(ACCENT)))
        .tooltip(editor_tooltip(tooltip, None))
        .child(
            div()
                .w(px(20.0))
                .h(px(20.0))
                .rounded(px(3.0))
                .border_1()
                .border_color(rgb(BORDER))
                .bg(preview
                    .map(paint_as_rgba)
                    .map(rgba)
                    .unwrap_or_else(|| rgba(0x00000000))),
        )
        .child(
            div()
                .flex_1()
                .font_family("SFMono-Regular")
                .text_size(px(10.0))
                .text_color(rgb(if active.is_some_and(|input| input.invalid) {
                    0xfca58f
                } else {
                    TEXT
                }))
                .child(value),
        )
        .when(active.is_some(), |field| {
            field.child(
                div()
                    .text_size(px(9.0))
                    .text_color(rgb(MUTED))
                    .child("Enter ↵"),
            )
        })
        .on_click(move |_, window: &mut Window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn paint_matches(paint: Option<&Paint>, expected: [f32; 4]) -> bool {
    matches!(paint, Some(Paint::Solid(color)) if *color == expected)
}

pub(crate) fn format_number(value: f32) -> String {
    if (value.round() - value).abs() < 0.01 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn format_degrees(radians: f32) -> String {
    format!("{}°", format_number(radians.to_degrees()))
}

fn format_percent(scale: f32) -> String {
    format!("{}%", format_number(scale * 100.0))
}

#[cfg(test)]
mod tests {
    use super::{
        format_degrees, format_percent, numeric_property_delta, snapshot, uniform_padding,
        SelectionValue,
    };
    use editor_core::{
        AlignCross, AlignMain, AutoLayout, Direction, Editor, Layout, Node, NumericAdjustmentMode,
        NumericPropertyTarget, Paint, PathData,
    };
    use glam::{Affine2, Vec2};

    #[test]
    fn numeric_scrub_pixels_use_property_appropriate_units() {
        let expected = [
            (
                NumericPropertyTarget::WorldX,
                NumericAdjustmentMode::Fine,
                12.0,
            ),
            (
                NumericPropertyTarget::WorldY,
                NumericAdjustmentMode::Fine,
                12.0,
            ),
            (
                NumericPropertyTarget::Opacity,
                NumericAdjustmentMode::Fine,
                0.12,
            ),
            (
                NumericPropertyTarget::StrokeWidth,
                NumericAdjustmentMode::Fine,
                1.2,
            ),
            (
                NumericPropertyTarget::TextSize,
                NumericAdjustmentMode::Fine,
                12.0,
            ),
            (
                NumericPropertyTarget::AutoLayoutSpacing,
                NumericAdjustmentMode::Fine,
                12.0,
            ),
            (
                NumericPropertyTarget::AutoLayoutPadding,
                NumericAdjustmentMode::Fine,
                12.0,
            ),
            (
                NumericPropertyTarget::WorldX,
                NumericAdjustmentMode::Coarse,
                120.0,
            ),
            (
                NumericPropertyTarget::WorldY,
                NumericAdjustmentMode::Coarse,
                120.0,
            ),
            (
                NumericPropertyTarget::Opacity,
                NumericAdjustmentMode::Coarse,
                1.2,
            ),
            (
                NumericPropertyTarget::StrokeWidth,
                NumericAdjustmentMode::Coarse,
                12.0,
            ),
            (
                NumericPropertyTarget::TextSize,
                NumericAdjustmentMode::Coarse,
                120.0,
            ),
            (
                NumericPropertyTarget::AutoLayoutSpacing,
                NumericAdjustmentMode::Coarse,
                120.0,
            ),
            (
                NumericPropertyTarget::AutoLayoutPadding,
                NumericAdjustmentMode::Coarse,
                120.0,
            ),
        ];

        for (target, mode, expected_delta) in expected {
            assert_eq!(numeric_property_delta(target, 12.0, mode), expected_delta);
        }
    }

    #[test]
    fn snapshot_preserves_current_text_typography() {
        let mut editor = Editor::new();
        let text = editor
            .document
            .add_child(editor.document.root, Node::text("Label", "Hello"))
            .unwrap();
        editor.selection.select(text);
        assert!(editor.set_selected_text_font_family("monospace"));
        assert!(editor.set_selected_text_font_weight(650));
        assert!(editor.set_selected_text_italic(true));

        let typography = snapshot(&mut editor, None, None).text.unwrap().font;

        assert_eq!(typography.family, "monospace");
        assert_eq!(typography.weight, 700);
        assert!(typography.italic);
    }

    #[test]
    fn snapshot_distinguishes_mixed_style_from_no_fill() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let red = editor
            .document
            .add_child(
                root,
                Node::shape("Red", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_style(editor_core::Style::fill(Paint::rgb(1.0, 0.0, 0.0))),
            )
            .unwrap();
        let blue = editor
            .document
            .add_child(
                root,
                Node::shape("Blue", PathData::rect(20.0, 0.0, 10.0, 10.0))
                    .with_style(editor_core::Style::fill(Paint::rgb(0.0, 0.0, 1.0))),
            )
            .unwrap();
        editor.selection.select(red);
        editor.selection.add(blue);

        assert!(snapshot(&mut editor, None, None)
            .style
            .unwrap()
            .fill
            .is_mixed());

        editor.document.get_mut(blue).unwrap().style =
            editor.document.get(red).unwrap().style.clone();
        assert!(matches!(
            snapshot(&mut editor, None, None).style.unwrap().fill,
            SelectionValue::Uniform(Some(_))
        ));
    }

    #[test]
    fn snapshot_tracks_mixed_state_per_appearance_property() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let first = editor
            .document
            .add_child(
                root,
                Node::shape("First", PathData::rect(0.0, 0.0, 10.0, 10.0))
                    .with_style(editor_core::Style::fill(Paint::rgb(1.0, 0.0, 0.0))),
            )
            .unwrap();
        let second = editor
            .document
            .add_child(
                root,
                Node::shape("Second", PathData::rect(20.0, 0.0, 10.0, 10.0))
                    .with_style(editor_core::Style::fill(Paint::rgb(1.0, 0.0, 0.0))),
            )
            .unwrap();
        editor.document.get_mut(second).unwrap().style.opacity = 0.5;
        editor.selection.select(first);
        editor.selection.add(second);

        let style = snapshot(&mut editor, None, None).style.unwrap();
        assert!(matches!(style.fill, SelectionValue::Uniform(Some(_))));
        assert!(matches!(
            style.stroke_enabled,
            SelectionValue::Uniform(false)
        ));
        assert!(matches!(style.stroke_width, SelectionValue::Uniform(None)));
        assert!(style.opacity.is_mixed());
    }

    #[test]
    fn snapshot_exposes_single_group_layout_and_real_world_transform() {
        let mut editor = Editor::new();
        let root = editor.document.root;
        let layout = AutoLayout::vertical()
            .with_spacing(12.0)
            .with_uniform_padding(8.0)
            .with_align_main(AlignMain::Center)
            .with_align_cross(AlignCross::End);
        let group = editor
            .document
            .add_child(
                root,
                Node::group("Stack")
                    .with_layout(Layout::Auto(layout.clone()))
                    .with_transform(Affine2::from_scale_angle_translation(
                        Vec2::new(2.0, 3.0),
                        0.25,
                        Vec2::new(40.0, 20.0),
                    )),
            )
            .unwrap();
        editor
            .document
            .add_child(
                group,
                Node::shape("Child", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        editor.selection.select(group);

        let snapshot = snapshot(&mut editor, None, None);

        let Some(Layout::Auto(snapshot_layout)) = snapshot.group_layout else {
            panic!("single group should expose auto-layout properties");
        };
        assert_eq!(snapshot_layout.direction, Direction::Vertical);
        assert_eq!(snapshot_layout.spacing, 12.0);
        assert_eq!(snapshot_layout.padding, layout.padding);
        assert_eq!(snapshot_layout.align_main, AlignMain::Center);
        assert_eq!(snapshot_layout.align_cross, AlignCross::End);
        let transform = snapshot.transform.unwrap();
        assert!((transform.rotation_radians - 0.25).abs() < 0.0001);
        assert!((transform.scale.x - 2.0).abs() < 0.0001);
        assert!((transform.scale.y - 3.0).abs() < 0.0001);
        assert_eq!(transform.translation, Vec2::new(40.0, 20.0));
    }

    #[test]
    fn inspector_formatting_distinguishes_uniform_padding_and_real_components() {
        let uniform = AutoLayout::horizontal().with_uniform_padding(6.0);
        assert_eq!(uniform_padding(&uniform), Some(6.0));

        let mut mixed = uniform;
        mixed.padding.right = 7.0;
        assert_eq!(uniform_padding(&mixed), None);
        assert_eq!(format_degrees(0.5), "28.6°");
        assert_eq!(format_percent(-1.25), "-125%");
    }
}
