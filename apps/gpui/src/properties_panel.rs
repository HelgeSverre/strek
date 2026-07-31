//! Figma-style design inspector for the current selection.

use editor_core::{
    Direction, Editor, FrameData, Layout, NodeKind, NumericPropertyTarget, Paint, Rect, Style,
    TextAlign, TextData, TransformComponents,
};
use gpui::{
    div, prelude::*, px, rgb, rgba, Action, AnyElement, IntoElement, MouseButton, SharedString,
    WeakEntity, Window,
};

use crate::{
    toolbar::{editor_tooltip, font_family_label, PortableFontFamily},
    AlignTextCenter, AlignTextLeft, AlignTextRight, PropertyFillBlack, PropertyFillBlue,
    PropertyFillGreen, PropertyFillNone, PropertyFillRed, PropertyFillWhite, PropertyMoveDown,
    PropertyMoveLeft, PropertyMoveRight, PropertyMoveUp, PropertyOpacityDown, PropertyOpacityUp,
    PropertyRotateLeft, PropertyRotateRight, PropertyStrokeDown, PropertyStrokeUp,
    PropertyToggleStroke, SetLayoutFree, SetLayoutHorizontal, SetLayoutVertical,
    SetTextFamilyMonospace, SetTextFamilySerif, SetTextFamilySystem, StartFillColorInput,
    StartStrokeColorInput, StepLayoutPaddingDown, StepLayoutPaddingUp, StepLayoutSpacingDown,
    StepLayoutSpacingUp, Strek, TextLarger, TextSmaller, TextWeightDown, TextWeightUp,
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
}

#[derive(Clone)]
enum SelectionStyle {
    Uniform(Style),
    Mixed,
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

pub(crate) fn numeric_property_delta(target: NumericPropertyTarget, pointer_delta: f32) -> f32 {
    let scale = match target {
        NumericPropertyTarget::Opacity => 0.01,
        NumericPropertyTarget::StrokeWidth => 0.1,
        NumericPropertyTarget::WorldX
        | NumericPropertyTarget::WorldY
        | NumericPropertyTarget::TextSize
        | NumericPropertyTarget::AutoLayoutSpacing
        | NumericPropertyTarget::AutoLayoutPadding => 1.0,
    };
    pointer_delta * scale
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
    let style = selected.first().and_then(|first| {
        let style = editor.document().get(*first)?.style.clone();
        Some(
            if selected.iter().skip(1).all(|id| {
                editor
                    .document()
                    .get(*id)
                    .is_some_and(|node| node.style == style)
            }) {
                SelectionStyle::Uniform(style)
            } else {
                SelectionStyle::Mixed
            },
        )
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
    }
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

    let (style, mixed_style) = match snapshot.style.clone() {
        Some(SelectionStyle::Uniform(style)) => (Some(style), false),
        Some(SelectionStyle::Mixed) => (None, true),
        None => (None, false),
    };
    let fill = style.as_ref().and_then(|style| style.fill.as_ref());
    let stroke_paint = style
        .as_ref()
        .and_then(|style| style.stroke.as_ref())
        .map(|stroke| &stroke.paint);
    let stroke_width = style
        .as_ref()
        .and_then(|style| style.stroke.as_ref())
        .map(|stroke| stroke.width);
    let opacity = style.as_ref().map(|style| style.opacity);
    let transform = snapshot.transform;
    let group_layout = snapshot.group_layout.clone();

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
                    .child(value_stepper(
                        "X",
                        format_number(position.x),
                        "Move left",
                        "Move right",
                        PropertyMoveLeft,
                        PropertyMoveRight,
                        Some((NumericPropertyTarget::WorldX, &editor_entity)),
                    ))
                    .child(value_stepper(
                        "Y",
                        format_number(position.y),
                        "Move up",
                        "Move down",
                        PropertyMoveUp,
                        PropertyMoveDown,
                        Some((NumericPropertyTarget::WorldY, &editor_entity)),
                    ))
                    .child(readonly_pair(
                        "W",
                        format_number(bounds.width()),
                        "H",
                        format_number(bounds.height()),
                    ))
                    .child(value_stepper(
                        "Rotate",
                        transform
                            .map(|components| format_degrees(components.rotation_radians))
                            .unwrap_or_else(|| "Mixed".to_owned()),
                        "Rotate counter-clockwise",
                        "Rotate clockwise",
                        PropertyRotateLeft,
                        PropertyRotateRight,
                        None,
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
            panel.child(layout_section(layout, &editor_entity))
        })
        .child(
            section("Appearance")
                .child(value_stepper(
                    "Opacity",
                    opacity
                        .map(|opacity| format!("{:.0}%", opacity * 100.0))
                        .unwrap_or_else(|| "Mixed".to_owned()),
                    "Decrease opacity",
                    "Increase opacity",
                    PropertyOpacityDown,
                    PropertyOpacityUp,
                    Some((NumericPropertyTarget::Opacity, &editor_entity)),
                ))
                .child(property_label("Fill"))
                .child(color_value_editor(
                    "fill-color",
                    fill,
                    mixed_style,
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
                            !mixed_style && fill.is_none(),
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
                            if mixed_style {
                                "Mixed"
                            } else if stroke_width.is_some() {
                                "On"
                            } else {
                                "Off"
                            },
                            "Toggle stroke",
                            PropertyToggleStroke,
                        ))
                        .child(div().flex_1())
                        .child(property_button(
                            "stroke-down",
                            "−",
                            "Decrease stroke width",
                            PropertyStrokeDown,
                        ))
                        .child(numeric_value(
                            stroke_width.map(format_number).unwrap_or_else(|| {
                                if mixed_style { "Mixed" } else { "—" }.to_owned()
                            }),
                            stroke_width
                                .map(|_| (NumericPropertyTarget::StrokeWidth, &editor_entity)),
                        ))
                        .child(property_button(
                            "stroke-up",
                            "+",
                            "Increase stroke width",
                            PropertyStrokeUp,
                        )),
                )
                .when(mixed_style || stroke_width.is_some(), |appearance| {
                    appearance.child(color_value_editor(
                        "stroke-color",
                        stroke_paint,
                        mixed_style,
                        snapshot
                            .color_input
                            .as_ref()
                            .filter(|input| input.target == ColorTarget::Stroke),
                        "Edit stroke color",
                        StartStrokeColorInput,
                    ))
                }),
        )
        .when_some(snapshot.text, |panel, text| {
            panel.child(
                section("Typography")
                    .child(font_family_controls(&text))
                    .child(value_stepper(
                        "Size",
                        format_number(text.font_size),
                        "Decrease font size",
                        "Increase font size",
                        TextSmaller,
                        TextLarger,
                        Some((NumericPropertyTarget::TextSize, &editor_entity)),
                    ))
                    .child(value_stepper(
                        "Weight",
                        text.font.weight.to_string(),
                        "Decrease font weight",
                        "Increase font weight",
                        TextWeightDown,
                        TextWeightUp,
                        None,
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

fn layout_section(layout: Layout, editor_entity: &WeakEntity<Strek>) -> gpui::Div {
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
                .child(value_stepper(
                    "Spacing",
                    format_number(layout.spacing),
                    "Decrease layout spacing",
                    "Increase layout spacing",
                    StepLayoutSpacingDown,
                    StepLayoutSpacingUp,
                    Some((NumericPropertyTarget::AutoLayoutSpacing, editor_entity)),
                ))
                .child(value_stepper(
                    "Padding",
                    padding
                        .map(format_number)
                        .unwrap_or_else(|| "Mixed".to_owned()),
                    "Decrease uniform padding",
                    "Increase uniform padding",
                    StepLayoutPaddingDown,
                    StepLayoutPaddingUp,
                    Some((NumericPropertyTarget::AutoLayoutPadding, editor_entity)),
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

fn value_stepper<A: Action + Clone, B: Action + Clone>(
    label: &'static str,
    value: String,
    decrease_tooltip: &'static str,
    increase_tooltip: &'static str,
    decrease: A,
    increase: B,
    scrub: Option<(NumericPropertyTarget, &WeakEntity<Strek>)>,
) -> impl IntoElement {
    div()
        .h(px(34.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .px(px(10.0))
        .child(row_label(label))
        .child(property_button("decrease", "−", decrease_tooltip, decrease))
        .child(numeric_value(value, scrub))
        .child(property_button("increase", "+", increase_tooltip, increase))
}

fn numeric_value(
    value: String,
    scrub: Option<(NumericPropertyTarget, &WeakEntity<Strek>)>,
) -> AnyElement {
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
            "Drag horizontally to adjust; Escape cancels",
            None,
        ))
        .on_mouse_down(MouseButton::Left, {
            let editor_entity = editor_entity.clone();
            move |event, window, cx| {
                let pointer_x = event.position.x.0;
                editor_entity
                    .update(cx, |editor, cx| {
                        editor.begin_numeric_property_scrub(target, pointer_x, window, cx);
                    })
                    .ok();
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

pub(crate) fn format_paint(paint: &Paint) -> String {
    let Paint::Solid([red, green, blue, alpha]) = paint;
    let channels =
        [red, green, blue, alpha].map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    if channels[3] == u8::MAX {
        format!("#{:02X}{:02X}{:02X}", channels[0], channels[1], channels[2])
    } else {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            channels[0], channels[1], channels[2], channels[3]
        )
    }
}

pub(crate) fn parse_hex_paint(value: &str) -> Option<Paint> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    let expanded;
    let value = match value.len() {
        3 | 4 => {
            expanded = value
                .chars()
                .flat_map(|character| [character, character])
                .collect::<String>();
            expanded.as_str()
        }
        6 | 8 => value,
        _ => return None,
    };
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |start| u8::from_str_radix(&value[start..start + 2], 16).ok();
    let red = channel(0)?;
    let green = channel(2)?;
    let blue = channel(4)?;
    let alpha = if value.len() == 8 {
        channel(6)?
    } else {
        u8::MAX
    };
    let normalized = |channel: u8| f32::from(channel) / 255.0;
    Some(Paint::rgba(
        normalized(red),
        normalized(green),
        normalized(blue),
        normalized(alpha),
    ))
}

pub(crate) fn paint_as_rgba(paint: &Paint) -> u32 {
    let Paint::Solid(channels) = paint;
    let [red, green, blue, alpha] =
        channels.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u32);
    (red << 24) | (green << 16) | (blue << 8) | alpha
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
        format_degrees, format_paint, format_percent, numeric_property_delta, parse_hex_paint,
        snapshot, uniform_padding, SelectionStyle,
    };
    use editor_core::{
        AlignCross, AlignMain, AutoLayout, Direction, Editor, Layout, Node, NumericPropertyTarget,
        Paint, PathData,
    };
    use glam::{Affine2, Vec2};

    #[test]
    fn numeric_scrub_pixels_use_property_appropriate_units() {
        assert_eq!(
            numeric_property_delta(NumericPropertyTarget::WorldX, 12.0),
            12.0
        );
        assert_eq!(
            numeric_property_delta(NumericPropertyTarget::Opacity, 12.0),
            0.12
        );
        assert_eq!(
            numeric_property_delta(NumericPropertyTarget::StrokeWidth, 12.0),
            1.2
        );
    }

    #[test]
    fn hex_colors_support_short_rgb_and_alpha_forms() {
        assert_eq!(
            parse_hex_paint("#0c8"),
            Some(Paint::rgba(0.0, 0.8, 8.0 / 15.0, 1.0))
        );
        assert_eq!(
            parse_hex_paint("33669980"),
            Some(Paint::rgba(
                0x33 as f32 / 255.0,
                0x66 as f32 / 255.0,
                0x99 as f32 / 255.0,
                0x80 as f32 / 255.0,
            ))
        );
        assert!(parse_hex_paint("#12xz89").is_none());
        assert!(parse_hex_paint("#12345").is_none());
    }

    #[test]
    fn paint_format_omits_opaque_alpha_and_preserves_transparency() {
        assert_eq!(format_paint(&Paint::rgb(1.0, 0.5, 0.0)), "#FF8000");
        assert_eq!(format_paint(&Paint::rgba(0.2, 0.4, 0.6, 0.5)), "#33669980");
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

        let typography = snapshot(&mut editor, None).text.unwrap().font;

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

        assert!(matches!(
            snapshot(&mut editor, None).style,
            Some(SelectionStyle::Mixed)
        ));

        editor.document.get_mut(blue).unwrap().style =
            editor.document.get(red).unwrap().style.clone();
        assert!(matches!(
            snapshot(&mut editor, None).style,
            Some(SelectionStyle::Uniform(_))
        ));
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

        let snapshot = snapshot(&mut editor, None);

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
