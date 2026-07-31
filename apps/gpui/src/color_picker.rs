//! Reusable solid-color picker state, color-model conversions, and GPUI chrome.

use std::f32::consts::TAU;
use std::sync::{Arc, OnceLock};

use editor_core::Paint;
use gpui::{
    div, img, linear_color_stop, linear_gradient, prelude::*, px, relative, rgb, rgba, AnyElement,
    Bounds, DragMoveEvent, IntoElement, MouseButton, Pixels, Point, Render, RenderImage,
    SharedString, WeakEntity, Window,
};
use image::{Frame, Rgba as ImageRgba, RgbaImage};
use smallvec::smallvec;

use crate::Strek;

const SURFACE: u32 = 0x202124;
const SURFACE_RAISED: u32 = 0x292a2e;
const SURFACE_HOVER: u32 = 0x35363b;
const BORDER: u32 = 0x414349;
const TEXT: u32 = 0xf1f3f4;
const MUTED: u32 = 0xa8abb2;
const ACCENT: u32 = 0x0c8ce9;
const WHEEL_SIZE: u32 = 168;
const OKLCH_MAX_CHROMA: f32 = 0.4;

const PRESETS: [(&str, u32); 8] = [
    ("White", 0xffffff),
    ("Black", 0x17181a),
    ("Blue", 0x0c8ce9),
    ("Cyan", 0x24c8db),
    ("Green", 0x14ae5c),
    ("Yellow", 0xf2c94c),
    ("Red", 0xf24822),
    ("Purple", 0x9747ff),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorModel {
    Hsv,
    Hsl,
    Rgb,
    Oklch,
}

impl ColorModel {
    const ALL: [Self; 4] = [Self::Hsv, Self::Hsl, Self::Rgb, Self::Oklch];

    fn label(self) -> &'static str {
        match self {
            Self::Hsv => "HSV",
            Self::Hsl => "HSL",
            Self::Rgb => "RGB",
            Self::Oklch => "OKLCH",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorPickerControl {
    Wheel,
    Channel(usize),
    Alpha,
}

#[derive(Clone)]
struct ColorPickerDrag;

impl Render for ColorPickerDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ColorPickerPlacement {
    pub top: f32,
    pub right: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct ColorPickerSnapshot {
    pub title: SharedString,
    paint: Paint,
    model: ColorModel,
    hsv: [f32; 3],
    hsl: [f32; 3],
    oklch: [f32; 3],
    hex_value: String,
    invalid: bool,
}

#[derive(Debug, Clone, Default)]
struct ControlBounds {
    wheel: Option<Bounds<Pixels>>,
    channels: [Option<Bounds<Pixels>>; 3],
    alpha: Option<Bounds<Pixels>>,
}

/// Mutable state shared by every fill and stroke color-editing surface.
#[derive(Debug, Clone)]
pub(crate) struct ColorPickerState {
    paint: Paint,
    model: ColorModel,
    hsv: [f32; 3],
    hsl: [f32; 3],
    oklch: [f32; 3],
    hex_value: String,
    replace_on_type: bool,
    invalid: bool,
    bounds: ControlBounds,
}

impl ColorPickerState {
    pub(crate) fn new(paint: Paint) -> Self {
        let mut picker = Self {
            hex_value: format_paint(&paint),
            paint,
            model: ColorModel::Hsv,
            hsv: [0.0; 3],
            hsl: [0.0; 3],
            oklch: [0.0; 3],
            replace_on_type: true,
            invalid: false,
            bounds: ControlBounds::default(),
        };
        picker.refresh_channels_from_paint();
        picker
    }

    pub(crate) fn snapshot(&self, title: impl Into<SharedString>) -> ColorPickerSnapshot {
        ColorPickerSnapshot {
            title: title.into(),
            paint: self.paint.clone(),
            model: self.model,
            hsv: self.hsv,
            hsl: self.hsl,
            oklch: self.oklch,
            hex_value: self.hex_value.clone(),
            invalid: self.invalid,
        }
    }

    pub(crate) fn hex_value(&self) -> &str {
        &self.hex_value
    }

    pub(crate) fn invalid(&self) -> bool {
        self.invalid
    }

    pub(crate) fn set_model(&mut self, model: ColorModel) {
        self.model = model;
    }

    pub(crate) fn set_paint(&mut self, paint: Paint) {
        self.paint = paint;
        self.refresh_channels_from_paint();
        self.sync_hex_from_paint();
    }

    pub(crate) fn set_control_bounds(
        &mut self,
        control: ColorPickerControl,
        bounds: Bounds<Pixels>,
    ) {
        match control {
            ColorPickerControl::Wheel => self.bounds.wheel = Some(bounds),
            ColorPickerControl::Channel(index) if index < self.bounds.channels.len() => {
                self.bounds.channels[index] = Some(bounds);
            }
            ColorPickerControl::Channel(_) => {}
            ColorPickerControl::Alpha => self.bounds.alpha = Some(bounds),
        }
    }

    pub(crate) fn update_from_point(
        &mut self,
        control: ColorPickerControl,
        point: Point<Pixels>,
        drag_bounds: Option<Bounds<Pixels>>,
    ) -> bool {
        let bounds = drag_bounds.or_else(|| match control {
            ColorPickerControl::Wheel => self.bounds.wheel,
            ColorPickerControl::Channel(index) => {
                self.bounds.channels.get(index).copied().flatten()
            }
            ColorPickerControl::Alpha => self.bounds.alpha,
        });
        let Some(bounds) = bounds.filter(|bounds| {
            bounds.size.width.0 > f32::EPSILON && bounds.size.height.0 > f32::EPSILON
        }) else {
            return false;
        };

        let before = self.paint.clone();
        match control {
            ColorPickerControl::Wheel => self.update_wheel(point, bounds),
            ColorPickerControl::Channel(index) => {
                self.update_channel(index, horizontal_fraction(point, bounds));
            }
            ColorPickerControl::Alpha => {
                let mut rgba = paint_channels(&self.paint);
                rgba[3] = horizontal_fraction(point, bounds);
                self.paint = Paint::Solid(rgba);
            }
        }
        self.sync_hex_from_paint();
        self.paint != before
    }

    pub(crate) fn backspace(&mut self) {
        if self.replace_on_type {
            self.hex_value.clear();
        } else {
            self.hex_value.pop();
        }
        self.replace_on_type = false;
        self.invalid = false;
        self.sync_paint_from_valid_hex();
    }

    pub(crate) fn select_hex(&mut self) {
        self.replace_on_type = true;
        self.invalid = false;
    }

    pub(crate) fn replace_hex(&mut self, value: &str) {
        self.hex_value = sanitize_hex_input(value);
        self.replace_on_type = false;
        self.invalid = false;
        self.sync_paint_from_valid_hex();
    }

    pub(crate) fn append_hex(&mut self, text: &str) {
        if self.replace_on_type {
            self.hex_value.clear();
            self.replace_on_type = false;
        }
        append_hex_input(&mut self.hex_value, text);
        self.invalid = false;
        self.sync_paint_from_valid_hex();
    }

    pub(crate) fn resolve_hex(&mut self) -> Option<Paint> {
        let paint = parse_hex_paint(&self.hex_value);
        self.invalid = paint.is_none();
        if let Some(paint) = &paint {
            self.paint = paint.clone();
            self.refresh_channels_from_paint();
            self.hex_value = format_paint(paint);
        }
        paint
    }

    fn update_wheel(&mut self, point: Point<Pixels>, bounds: Bounds<Pixels>) {
        let center_x = bounds.origin.x.0 + bounds.size.width.0 * 0.5;
        let center_y = bounds.origin.y.0 + bounds.size.height.0 * 0.5;
        let radius = bounds.size.width.0.min(bounds.size.height.0) * 0.5;
        let dx = (point.x.0 - center_x) / radius;
        let dy = (point.y.0 - center_y) / radius;
        let saturation = dx.hypot(dy).clamp(0.0, 1.0);
        let hue = normalize_degrees(dy.atan2(dx).to_degrees());
        let rgba = paint_channels(&self.paint);
        self.hsv[0] = hue;
        self.hsv[1] = saturation;
        let rgb = hsv_to_rgb(self.hsv[0], self.hsv[1], self.hsv[2]);
        self.paint = Paint::rgba(rgb[0], rgb[1], rgb[2], rgba[3]);
        self.refresh_hsl_and_oklch();
    }

    fn update_channel(&mut self, index: usize, fraction: f32) {
        if index >= 3 {
            return;
        }
        let rgba = paint_channels(&self.paint);
        let alpha = rgba[3];
        let rgb = [rgba[0], rgba[1], rgba[2]];
        let rgb = match self.model {
            ColorModel::Hsv => {
                self.hsv[index] = if index == 0 {
                    fraction * 360.0
                } else {
                    fraction
                };
                hsv_to_rgb(self.hsv[0], self.hsv[1], self.hsv[2])
            }
            ColorModel::Hsl => {
                self.hsl[index] = if index == 0 {
                    fraction * 360.0
                } else {
                    fraction
                };
                hsl_to_rgb(self.hsl[0], self.hsl[1], self.hsl[2])
            }
            ColorModel::Rgb => {
                let mut rgb = rgb;
                rgb[index] = fraction;
                rgb
            }
            ColorModel::Oklch => {
                self.oklch[index] = match index {
                    0 => fraction,
                    1 => fraction * OKLCH_MAX_CHROMA,
                    2 => fraction * 360.0,
                    _ => unreachable!(),
                };
                oklch_to_srgb_gamut_mapped(self.oklch[0], self.oklch[1], self.oklch[2])
            }
        };
        self.paint = Paint::rgba(rgb[0], rgb[1], rgb[2], alpha);
        match self.model {
            ColorModel::Hsv => self.refresh_hsl_and_oklch(),
            ColorModel::Hsl => self.refresh_hsv_and_oklch(),
            ColorModel::Rgb => self.refresh_channels_from_paint(),
            ColorModel::Oklch => self.refresh_hsv_and_hsl(),
        }
    }

    fn sync_hex_from_paint(&mut self) {
        self.hex_value = format_paint(&self.paint);
        self.replace_on_type = true;
        self.invalid = false;
    }

    fn sync_paint_from_valid_hex(&mut self) {
        if let Some(paint) = parse_hex_paint(&self.hex_value) {
            self.paint = paint;
            self.refresh_channels_from_paint();
        }
    }

    fn refresh_channels_from_paint(&mut self) {
        self.refresh_hsv_and_hsl();
        self.refresh_oklch();
    }

    fn refresh_hsv_and_hsl(&mut self) {
        let [red, green, blue, _] = paint_channels(&self.paint);
        self.hsv = tuple_to_array(rgb_to_hsv([red, green, blue]));
        self.hsl = tuple_to_array(rgb_to_hsl([red, green, blue]));
    }

    fn refresh_oklch(&mut self) {
        let [red, green, blue, _] = paint_channels(&self.paint);
        self.oklch = tuple_to_array(rgb_to_oklch([red, green, blue]));
    }

    fn refresh_hsl_and_oklch(&mut self) {
        let [red, green, blue, _] = paint_channels(&self.paint);
        self.hsl = tuple_to_array(rgb_to_hsl([red, green, blue]));
        self.oklch = tuple_to_array(rgb_to_oklch([red, green, blue]));
    }

    fn refresh_hsv_and_oklch(&mut self) {
        let [red, green, blue, _] = paint_channels(&self.paint);
        self.hsv = tuple_to_array(rgb_to_hsv([red, green, blue]));
        self.oklch = tuple_to_array(rgb_to_oklch([red, green, blue]));
    }
}

pub(crate) fn render(
    snapshot: ColorPickerSnapshot,
    placement: ColorPickerPlacement,
    editor_entity: WeakEntity<Strek>,
) -> AnyElement {
    let rgba_channels = paint_channels(&snapshot.paint);
    let rgb_channels = [rgba_channels[0], rgba_channels[1], rgba_channels[2]];
    let marker_x = 0.5 + snapshot.hsv[0].to_radians().cos() * snapshot.hsv[1] * 0.5;
    let marker_y = 0.5 + snapshot.hsv[0].to_radians().sin() * snapshot.hsv[1] * 0.5;
    let channels = channel_views(
        snapshot.model,
        rgb_channels,
        snapshot.hsv,
        snapshot.hsl,
        snapshot.oklch,
    );
    let opaque = Paint::rgba(rgb_channels[0], rgb_channels[1], rgb_channels[2], 1.0);
    let transparent = Paint::rgba(rgb_channels[0], rgb_channels[1], rgb_channels[2], 0.0);

    div()
        .id("color-picker")
        .absolute()
        .top(px(placement.top))
        .right(px(placement.right))
        .w(px(286.0))
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(SURFACE))
        .shadow_lg()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(TEXT))
                        .child(snapshot.title),
                )
                .child(
                    div()
                        .w(px(28.0))
                        .h(px(22.0))
                        .rounded(px(4.0))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgba(paint_as_rgba(&snapshot.paint))),
                ),
        )
        .child(div().flex().justify_center().child(wheel_control(
            marker_x,
            marker_y,
            editor_entity.clone(),
        )))
        .child(
            div()
                .flex()
                .gap(px(4.0))
                .children(ColorModel::ALL.into_iter().map(|model| {
                    model_button(model, snapshot.model, editor_entity.clone()).into_any_element()
                })),
        )
        .children(channels.into_iter().enumerate().map(|(index, channel)| {
            channel_slider(index, channel, editor_entity.clone()).into_any_element()
        }))
        .child(channel_slider(
            3,
            ChannelView {
                label: "A",
                value: format!("{:.0}%", rgba_channels[3] * 100.0),
                fraction: rgba_channels[3],
            },
            editor_entity.clone(),
        ))
        .child(div().h(px(8.0)).rounded(px(4.0)).bg(linear_gradient(
            90.0,
            linear_color_stop(rgba(paint_as_rgba(&transparent)), 0.0),
            linear_color_stop(rgba(paint_as_rgba(&opaque)), 1.0),
        )))
        .child(
            div()
                .flex()
                .gap(px(5.0))
                .children(PRESETS.into_iter().map(|(name, color)| {
                    preset_swatch(name, color, editor_entity.clone()).into_any_element()
                })),
        )
        .child(
            div()
                .h(px(30.0))
                .px(px(7.0))
                .flex()
                .items_center()
                .gap(px(7.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(rgb(if snapshot.invalid { 0xf24822 } else { BORDER }))
                .bg(rgb(SURFACE_RAISED))
                .child(div().text_size(px(9.0)).text_color(rgb(MUTED)).child("HEX"))
                .child(
                    div()
                        .flex_1()
                        .font_family("SFMono-Regular")
                        .text_size(px(10.0))
                        .text_color(rgb(if snapshot.invalid { 0xfca58f } else { TEXT }))
                        .child(snapshot.hex_value),
                )
                .child(
                    div()
                        .text_size(px(9.0))
                        .text_color(rgb(MUTED))
                        .child("Type, then Enter"),
                ),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(6.0))
                .child(picker_button(
                    "color-picker-cancel",
                    "Cancel",
                    false,
                    editor_entity.clone(),
                ))
                .child(picker_button(
                    "color-picker-apply",
                    "Apply",
                    true,
                    editor_entity,
                )),
        )
        .into_any_element()
}

fn wheel_control(
    marker_x: f32,
    marker_y: f32,
    editor_entity: WeakEntity<Strek>,
) -> impl IntoElement {
    let control = ColorPickerControl::Wheel;
    interactive_control(
        "color-picker-wheel",
        control,
        editor_entity,
        div()
            .relative()
            .w(px(WHEEL_SIZE as f32))
            .h(px(WHEEL_SIZE as f32))
            .rounded_full()
            .overflow_hidden()
            .cursor_crosshair()
            .child(
                img(color_wheel_image())
                    .w(px(WHEEL_SIZE as f32))
                    .h(px(WHEEL_SIZE as f32)),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(marker_x))
                    .top(relative(marker_y))
                    .ml(px(-5.0))
                    .mt(px(-5.0))
                    .w(px(10.0))
                    .h(px(10.0))
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(0xffffff))
                    .shadow_sm(),
            ),
    )
}

fn model_button(
    model: ColorModel,
    active: ColorModel,
    editor_entity: WeakEntity<Strek>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!(
            "color-model-{}",
            model.label().to_ascii_lowercase()
        )))
        .flex_1()
        .h(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(rgb(if model == active { ACCENT } else { BORDER }))
        .bg(if model == active {
            rgba(0x0c8ce92e)
        } else {
            rgba(0x00000000)
        })
        .text_size(px(9.0))
        .text_color(rgb(if model == active { 0xd8efff } else { MUTED }))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(model.label())
        .on_click(move |_, _, cx| {
            editor_entity
                .update(cx, |editor, cx| editor.set_color_picker_model(model, cx))
                .ok();
            cx.stop_propagation();
        })
}

#[derive(Debug)]
struct ChannelView {
    label: &'static str,
    value: String,
    fraction: f32,
}

fn channel_slider(
    index: usize,
    channel: ChannelView,
    editor_entity: WeakEntity<Strek>,
) -> impl IntoElement {
    let control = if index == 3 {
        ColorPickerControl::Alpha
    } else {
        ColorPickerControl::Channel(index)
    };
    let track = div()
        .relative()
        .flex_1()
        .h(px(8.0))
        .rounded(px(4.0))
        .bg(rgb(0x17181a))
        .cursor_col_resize()
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(relative(channel.fraction.clamp(0.0, 1.0)))
                .rounded(px(4.0))
                .bg(rgb(ACCENT)),
        )
        .child(
            div()
                .absolute()
                .left(relative(channel.fraction.clamp(0.0, 1.0)))
                .top(px(-2.0))
                .ml(px(-5.0))
                .w(px(10.0))
                .h(px(12.0))
                .rounded(px(3.0))
                .border_1()
                .border_color(rgb(0xffffff))
                .bg(rgb(SURFACE_RAISED)),
        );

    div()
        .h(px(22.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .child(
            div()
                .w(px(14.0))
                .text_size(px(9.0))
                .text_color(rgb(MUTED))
                .child(channel.label),
        )
        .child(interactive_control(
            SharedString::from(format!("color-channel-{index}")),
            control,
            editor_entity,
            track,
        ))
        .child(
            div()
                .w(px(50.0))
                .font_family("SFMono-Regular")
                .text_size(px(9.0))
                .text_color(rgb(TEXT))
                .text_align(gpui::TextAlign::Right)
                .child(channel.value),
        )
}

fn interactive_control(
    id: impl Into<gpui::ElementId>,
    control: ColorPickerControl,
    editor_entity: WeakEntity<Strek>,
    content: impl IntoElement,
) -> impl IntoElement {
    let bounds_entity = editor_entity.clone();
    let down_entity = editor_entity.clone();
    let drag_entity = editor_entity;
    div()
        .flex_1()
        .child(content)
        .on_children_prepainted(move |bounds, _, cx| {
            let Some(bounds) = bounds.first().copied() else {
                return;
            };
            bounds_entity
                .update(cx, |editor, _| {
                    editor.set_color_picker_control_bounds(control, bounds);
                })
                .ok();
        })
        .id(id)
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            down_entity
                .update(cx, |editor, cx| {
                    editor.update_color_picker_from_point(control, event.position, None, cx);
                })
                .ok();
            cx.stop_propagation();
        })
        .on_drag(ColorPickerDrag, |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_drag_move::<ColorPickerDrag>(move |event: &DragMoveEvent<ColorPickerDrag>, _, cx| {
            drag_entity
                .update(cx, |editor, cx| {
                    editor.update_color_picker_from_point(
                        control,
                        event.event.position,
                        Some(event.bounds),
                        cx,
                    );
                })
                .ok();
            cx.stop_propagation();
        })
}

fn preset_swatch(
    name: &'static str,
    color: u32,
    editor_entity: WeakEntity<Strek>,
) -> impl IntoElement {
    let red = ((color >> 16) & 0xff) as f32 / 255.0;
    let green = ((color >> 8) & 0xff) as f32 / 255.0;
    let blue = (color & 0xff) as f32 / 255.0;
    div()
        .id(SharedString::from(format!(
            "color-preset-{}",
            name.to_ascii_lowercase()
        )))
        .w(px(26.0))
        .h(px(22.0))
        .p(px(2.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(rgb(BORDER))
        .cursor_pointer()
        .hover(|style| style.border_color(rgb(ACCENT)))
        .child(div().size_full().rounded(px(2.0)).bg(rgb(color)))
        .on_click(move |_, _, cx| {
            editor_entity
                .update(cx, |editor, cx| {
                    editor.set_color_picker_paint(Paint::rgb(red, green, blue), cx);
                })
                .ok();
            cx.stop_propagation();
        })
}

fn picker_button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    editor_entity: WeakEntity<Strek>,
) -> impl IntoElement {
    div()
        .id(id)
        .h(px(27.0))
        .min_w(px(58.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(rgb(if primary { ACCENT } else { BORDER }))
        .bg(if primary {
            rgba(0x0c8ce92e)
        } else {
            rgba(0x00000000)
        })
        .text_size(px(10.0))
        .text_color(rgb(if primary { 0xd8efff } else { TEXT }))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(label)
        .on_click(move |_, _, cx| {
            editor_entity
                .update(cx, |editor, cx| {
                    if primary {
                        editor.commit_color_picker(cx);
                    } else {
                        editor.dismiss_color_picker(cx);
                    }
                })
                .ok();
            cx.stop_propagation();
        })
}

fn color_wheel_image() -> Arc<RenderImage> {
    static IMAGE: OnceLock<Arc<RenderImage>> = OnceLock::new();
    Arc::clone(IMAGE.get_or_init(|| {
        let radius = (WHEEL_SIZE as f32 - 1.0) * 0.5;
        let center = radius;
        let image = RgbaImage::from_fn(WHEEL_SIZE, WHEEL_SIZE, |x, y| {
            let dx = (x as f32 - center) / radius;
            let dy = (y as f32 - center) / radius;
            let saturation = dx.hypot(dy);
            if saturation > 1.0 {
                return ImageRgba([0, 0, 0, 0]);
            }
            let hue = normalize_degrees(dy.atan2(dx).to_degrees());
            let rgb = hsv_to_rgb(hue, saturation, 1.0);
            let alpha =
                ((1.0 - ((saturation - 0.975) / 0.025).clamp(0.0, 1.0)) * 255.0).round() as u8;
            ImageRgba([
                (rgb[0] * 255.0).round() as u8,
                (rgb[1] * 255.0).round() as u8,
                (rgb[2] * 255.0).round() as u8,
                alpha,
            ])
        });
        Arc::new(RenderImage::new(smallvec![Frame::new(image)]))
    }))
}

fn channel_views(
    model: ColorModel,
    rgb: [f32; 3],
    hsv: [f32; 3],
    hsl: [f32; 3],
    oklch: [f32; 3],
) -> [ChannelView; 3] {
    match model {
        ColorModel::Hsv => [
            hue_channel("H", hsv[0]),
            percent_channel("S", hsv[1]),
            percent_channel("V", hsv[2]),
        ],
        ColorModel::Hsl => [
            hue_channel("H", hsl[0]),
            percent_channel("S", hsl[1]),
            percent_channel("L", hsl[2]),
        ],
        ColorModel::Rgb => [
            byte_channel("R", rgb[0]),
            byte_channel("G", rgb[1]),
            byte_channel("B", rgb[2]),
        ],
        ColorModel::Oklch => [
            percent_channel("L", oklch[0]),
            ChannelView {
                label: "C",
                value: format!("{:.3}", oklch[1]),
                fraction: oklch[1] / OKLCH_MAX_CHROMA,
            },
            hue_channel("H", oklch[2]),
        ],
    }
}

fn hue_channel(label: &'static str, hue: f32) -> ChannelView {
    ChannelView {
        label,
        value: format!("{hue:.0}°"),
        fraction: normalize_degrees(hue) / 360.0,
    }
}

fn percent_channel(label: &'static str, value: f32) -> ChannelView {
    ChannelView {
        label,
        value: format!("{:.0}%", value * 100.0),
        fraction: value,
    }
}

fn byte_channel(label: &'static str, value: f32) -> ChannelView {
    ChannelView {
        label,
        value: ((value.clamp(0.0, 1.0) * 255.0).round() as u8).to_string(),
        fraction: value,
    }
}

fn horizontal_fraction(point: Point<Pixels>, bounds: Bounds<Pixels>) -> f32 {
    ((point.x.0 - bounds.origin.x.0) / bounds.size.width.0).clamp(0.0, 1.0)
}

pub(crate) fn format_paint(paint: &Paint) -> String {
    let channels =
        paint_channels(paint).map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
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
    let [red, green, blue, alpha] =
        paint_channels(paint).map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u32);
    (red << 24) | (green << 16) | (blue << 8) | alpha
}

fn sanitize_hex_input(value: &str) -> String {
    let mut sanitized = String::new();
    append_hex_input(&mut sanitized, value);
    sanitized
}

fn append_hex_input(value: &mut String, text: &str) {
    for character in text.chars() {
        if character == '#' && value.is_empty() {
            value.push(character);
        } else if character.is_ascii_hexdigit() && value.trim_start_matches('#').chars().count() < 8
        {
            value.push(character.to_ascii_uppercase());
        }
    }
}

fn paint_channels(paint: &Paint) -> [f32; 4] {
    let Paint::Solid(channels) = paint;
    *channels
}

fn tuple_to_array((first, second, third): (f32, f32, f32)) -> [f32; 3] {
    [first, second, third]
}

fn normalize_degrees(degrees: f32) -> f32 {
    degrees.rem_euclid(360.0)
}

fn rgb_to_hsv(rgb: [f32; 3]) -> (f32, f32, f32) {
    let [red, green, blue] = rgb.map(|channel| channel.clamp(0.0, 1.0));
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let delta = max - min;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if max == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if max == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    let saturation = if max <= f32::EPSILON {
        0.0
    } else {
        delta / max
    };
    (normalize_degrees(hue), saturation, max)
}

fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let saturation = saturation.clamp(0.0, 1.0);
    let value = value.clamp(0.0, 1.0);
    let chroma = value * saturation;
    let hue_sector = normalize_degrees(hue) / 60.0;
    let x = chroma * (1.0 - (hue_sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match hue_sector as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = value - chroma;
    [red + offset, green + offset, blue + offset]
}

fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let [red, green, blue] = rgb.map(|channel| channel.clamp(0.0, 1.0));
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let delta = max - min;
    let lightness = (max + min) * 0.5;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if max == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if max == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    let saturation = if delta <= f32::EPSILON {
        0.0
    } else {
        delta / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    (normalize_degrees(hue), saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let hue_sector = normalize_degrees(hue) / 60.0;
    let x = chroma * (1.0 - (hue_sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match hue_sector as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = lightness - chroma * 0.5;
    [red + offset, green + offset, blue + offset]
}

fn rgb_to_oklch(rgb: [f32; 3]) -> (f32, f32, f32) {
    let [red, green, blue] = rgb.map(srgb_to_linear);
    let l = (0.412_221_46 * red + 0.536_332_55 * green + 0.051_445_995 * blue).cbrt();
    let m = (0.211_903_5 * red + 0.680_699_5 * green + 0.107_396_96 * blue).cbrt();
    let s = (0.088_302_46 * red + 0.281_718_85 * green + 0.629_978_7 * blue).cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    let chroma = a.hypot(b);
    let hue = if chroma <= 0.000_001 {
        0.0
    } else {
        normalize_degrees(b.atan2(a) * 360.0 / TAU)
    };
    (lightness.clamp(0.0, 1.0), chroma, hue)
}

fn oklch_to_srgb_gamut_mapped(lightness: f32, chroma: f32, hue: f32) -> [f32; 3] {
    let lightness = lightness.clamp(0.0, 1.0);
    let chroma = chroma.clamp(0.0, OKLCH_MAX_CHROMA);
    let candidate = oklch_to_srgb(lightness, chroma, hue);
    if candidate
        .into_iter()
        .all(|channel| (0.0..=1.0).contains(&channel))
    {
        return candidate;
    }

    let mut low = 0.0;
    let mut high = chroma;
    for _ in 0..18 {
        let middle = (low + high) * 0.5;
        let candidate = oklch_to_srgb(lightness, middle, hue);
        if candidate
            .into_iter()
            .all(|channel| (0.0..=1.0).contains(&channel))
        {
            low = middle;
        } else {
            high = middle;
        }
    }
    oklch_to_srgb(lightness, low, hue).map(|channel| channel.clamp(0.0, 1.0))
}

fn oklch_to_srgb(lightness: f32, chroma: f32, hue: f32) -> [f32; 3] {
    let radians = normalize_degrees(hue).to_radians();
    let a = chroma * radians.cos();
    let b = chroma * radians.sin();
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        linear_to_srgb(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        linear_to_srgb(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        linear_to_srgb(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    ]
}

fn srgb_to_linear(channel: f32) -> f32 {
    let channel = channel.clamp(0.0, 1.0);
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(channel: f32) -> f32 {
    if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rgb_close(actual: [f32; 3], expected: [f32; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 0.000_1,
                "{actual} != {expected}"
            );
        }
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
    fn hsv_and_hsl_round_trip_primary_and_arbitrary_colors() {
        for rgb in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.2, 0.45, 0.8],
            [0.4, 0.4, 0.4],
        ] {
            let (hue, saturation, value) = rgb_to_hsv(rgb);
            assert_rgb_close(hsv_to_rgb(hue, saturation, value), rgb);
            let (hue, saturation, lightness) = rgb_to_hsl(rgb);
            assert_rgb_close(hsl_to_rgb(hue, saturation, lightness), rgb);
        }
    }

    #[test]
    fn oklch_round_trips_srgb_and_reduces_out_of_gamut_chroma() {
        for rgb in [[1.0, 0.0, 0.0], [0.2, 0.45, 0.8], [0.4, 0.4, 0.4]] {
            let (lightness, chroma, hue) = rgb_to_oklch(rgb);
            assert_rgb_close(oklch_to_srgb_gamut_mapped(lightness, chroma, hue), rgb);
        }

        let mapped = oklch_to_srgb_gamut_mapped(0.7, 0.4, 140.0);
        assert!(mapped
            .into_iter()
            .all(|channel| (0.0..=1.0).contains(&channel)));
        let (lightness, _, hue) = rgb_to_oklch(mapped);
        assert!((lightness - 0.7).abs() < 0.01);
        assert!((normalize_degrees(hue - 140.0)).min(normalize_degrees(140.0 - hue)) < 1.0);
    }

    #[test]
    fn picker_text_edits_keep_draft_color_in_sync() {
        let mut picker = ColorPickerState::new(Paint::black());
        picker.replace_hex("#33669980");

        assert_eq!(
            picker.paint,
            Paint::rgba(0.2, 0.4, 0.6, 0x80 as f32 / 255.0)
        );
        assert!(!picker.invalid());

        picker.select_hex();
        picker.append_hex("f80");
        assert_eq!(picker.paint, Paint::rgb(1.0, 8.0 / 15.0, 0.0));
        assert_eq!(picker.resolve_hex(), Some(Paint::rgb(1.0, 8.0 / 15.0, 0.0)));
    }

    #[test]
    fn picker_preserves_hue_while_editing_achromatic_colors() {
        let mut picker = ColorPickerState::new(Paint::black());
        picker.update_channel(0, 120.0 / 360.0);
        picker.update_channel(1, 1.0);
        picker.update_channel(2, 1.0);

        let [red, green, blue, _] = paint_channels(&picker.paint);
        assert_rgb_close([red, green, blue], [0.0, 1.0, 0.0]);
    }
}
