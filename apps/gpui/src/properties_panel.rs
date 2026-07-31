//! Figma-style design inspector for the current selection.

use editor_core::{Editor, FrameData, NodeKind, Paint, Rect, Style, TextData};
use gpui::{div, prelude::*, px, rgb, rgba, Action, AnyElement, IntoElement, SharedString, Window};

use crate::{
    toolbar::editor_tooltip, AlignTextCenter, AlignTextLeft, AlignTextRight, PropertyFillBlack,
    PropertyFillBlue, PropertyFillGreen, PropertyFillNone, PropertyFillRed, PropertyFillWhite,
    PropertyMoveDown, PropertyMoveLeft, PropertyMoveRight, PropertyMoveUp, PropertyOpacityDown,
    PropertyOpacityUp, PropertyRotateLeft, PropertyRotateRight, PropertyStrokeDown,
    PropertyStrokeUp, PropertyToggleStroke, StartFillColorInput, StartStrokeColorInput, TextLarger,
    TextSmaller, ToggleFrameBackground,
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
    style: Option<Style>,
    text: Option<TextData>,
    frame: Option<FrameData>,
    color_input: Option<ColorInputSnapshot>,
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
        selected
            .iter()
            .skip(1)
            .all(|id| {
                editor
                    .document()
                    .get(*id)
                    .is_some_and(|node| node.style == style)
            })
            .then_some(style)
    });
    let text = (selection_count == 1)
        .then(|| editor.selected_text_data())
        .flatten();
    let frame = (selection_count == 1)
        .then(|| editor.selected_frame_data())
        .flatten();

    PropertiesSnapshot {
        selection_count,
        label,
        kind,
        bounds,
        style,
        text,
        frame,
        color_input,
    }
}

pub(crate) fn render(snapshot: PropertiesSnapshot) -> AnyElement {
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
                    .text_color(rgb(0x777a82))
                    .child("Select a layer to edit its design properties."),
            )
            .into_any_element();
    }

    let style = snapshot.style.clone();
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
            panel.child(
                section("Transform")
                    .child(value_stepper(
                        "X",
                        format_number(bounds.min.x),
                        "Move left",
                        "Move right",
                        PropertyMoveLeft,
                        PropertyMoveRight,
                    ))
                    .child(value_stepper(
                        "Y",
                        format_number(bounds.min.y),
                        "Move up",
                        "Move down",
                        PropertyMoveUp,
                        PropertyMoveDown,
                    ))
                    .child(readonly_pair(
                        "W",
                        format_number(bounds.width()),
                        "H",
                        format_number(bounds.height()),
                    ))
                    .child(value_stepper(
                        "Rotate",
                        "15° step".to_owned(),
                        "Rotate counter-clockwise",
                        "Rotate clockwise",
                        PropertyRotateLeft,
                        PropertyRotateRight,
                    )),
            )
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
                ))
                .child(property_label("Fill"))
                .child(color_value_editor(
                    "fill-color",
                    fill,
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
                            fill.is_none(),
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
                        .child(div().w(px(62.0)).text_size(px(10.0)).child("Stroke"))
                        .child(property_button(
                            "stroke-toggle",
                            if stroke_width.is_some() { "On" } else { "Off" },
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
                        .child(
                            div()
                                .w(px(38.0))
                                .text_align(gpui::TextAlign::Center)
                                .text_size(px(10.0))
                                .text_color(rgb(TEXT))
                                .child(
                                    stroke_width
                                        .map(format_number)
                                        .unwrap_or_else(|| "—".to_owned()),
                                ),
                        )
                        .child(property_button(
                            "stroke-up",
                            "+",
                            "Increase stroke width",
                            PropertyStrokeUp,
                        )),
                )
                .when(stroke_width.is_some(), |appearance| {
                    appearance.child(color_value_editor(
                        "stroke-color",
                        stroke_paint,
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
                    .child(value_stepper(
                        "Size",
                        format_number(text.font_size),
                        "Decrease font size",
                        "Increase font size",
                        TextSmaller,
                        TextLarger,
                    ))
                    .child(
                        div()
                            .h(px(36.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(10.0))
                            .child(div().w(px(62.0)).text_size(px(10.0)).child("Align"))
                            .child(property_button(
                                "text-left",
                                "Left",
                                "Align text left",
                                AlignTextLeft,
                            ))
                            .child(property_button(
                                "text-center",
                                "Center",
                                "Align text center",
                                AlignTextCenter,
                            ))
                            .child(property_button(
                                "text-right",
                                "Right",
                                "Align text right",
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
) -> impl IntoElement {
    div()
        .h(px(34.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .px(px(10.0))
        .child(div().w(px(62.0)).text_size(px(10.0)).child(label))
        .child(property_button("decrease", "−", decrease_tooltip, decrease))
        .child(
            div()
                .flex_1()
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(SURFACE_RAISED))
                .rounded(px(4.0))
                .text_size(px(10.0))
                .text_color(rgb(TEXT))
                .child(value),
        )
        .child(property_button("increase", "+", increase_tooltip, increase))
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
    active: Option<&ColorInputSnapshot>,
    tooltip: &'static str,
    action: A,
) -> impl IntoElement {
    let active_paint = active.and_then(|input| parse_hex_paint(&input.value));
    let preview = active_paint.as_ref().or(paint);
    let value = active
        .map(|input| input.value.clone())
        .or_else(|| paint.map(format_paint))
        .unwrap_or_else(|| "None".to_owned());
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

fn paint_as_rgba(paint: &Paint) -> u32 {
    let Paint::Solid(channels) = paint;
    let [red, green, blue, alpha] =
        channels.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u32);
    (red << 24) | (green << 16) | (blue << 8) | alpha
}

fn format_number(value: f32) -> String {
    if (value.round() - value).abs() < 0.01 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::{format_paint, parse_hex_paint};
    use editor_core::Paint;

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
}
