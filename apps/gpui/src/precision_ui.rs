//! GPUI presentation and bounded overlay geometry for canvas precision aids.
//!
//! This module deliberately owns no editor state. Callers provide immutable
//! snapshots and receive all requested changes through [`PrecisionCallbacks`].
//! Document mutations (guides and grid settings) and workspace mutations
//! (visibility and snapping) remain the responsibility of the integration
//! layer.

use std::rc::Rc;

use glam::Vec2;
use gpui::{
    canvas, div, prelude::*, px, rgb, rgba, App, Bounds, IntoElement, MouseButton, Pixels,
    SharedString, Window,
};

use crate::assets::{icon, Icon};

const SURFACE: u32 = 0x202124;
const SURFACE_RAISED: u32 = 0x292a2e;
const SURFACE_HOVER: u32 = 0x35363b;
const BORDER: u32 = 0x414349;
const TEXT: u32 = 0xf1f3f4;
const TEXT_MUTED: u32 = 0xa8abb2;
const ACCENT: u32 = 0x0c8ce9;
const WARNING: u32 = 0xfbbc04;

pub(crate) const DEFAULT_RULER_THICKNESS: f32 = 22.0;
pub(crate) const MIN_GRID_MINOR_SEPARATION_PX: f32 = 8.0;
pub(crate) const MIN_GRID_MAJOR_SEPARATION_PX: f32 = 24.0;
pub(crate) const MIN_RULER_MAJOR_SEPARATION_PX: f32 = 64.0;
pub(crate) const MAX_GRID_LINES_PER_AXIS: usize = 2_048;
pub(crate) const MAX_RULER_TICKS_PER_AXIS: usize = 2_048;
pub(crate) const MAX_GUIDE_LINES: usize = 10_000;

pub(crate) const MIN_TOLERANCE_PX: f32 = 1.0;
pub(crate) const MAX_TOLERANCE_PX: f32 = 32.0;
pub(crate) const MIN_GRID_SPACING: f32 = 0.1;
pub(crate) const MAX_GRID_SPACING: f32 = 100_000.0;

/// A coarse snapping category exposed by the V1 precision popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SnapTarget {
    Objects,
    Guides,
    Grid,
}

/// A non-exporting precision overlay whose visibility is workspace state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PrecisionOverlay {
    Rulers,
    Grid,
    Guides,
}

/// Workspace-owned snapping settings rendered by the precision controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SnappingSettings {
    pub enabled: bool,
    pub objects: bool,
    pub guides: bool,
    pub grid: bool,
    pub tolerance_px: f32,
}

impl Default for SnappingSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            objects: true,
            guides: true,
            grid: false,
            tolerance_px: 8.0,
        }
    }
}

/// Workspace-owned visibility and guide-interaction settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PrecisionVisibility {
    pub rulers: bool,
    pub grid: bool,
    pub guides: bool,
    pub guides_locked: bool,
}

impl Default for PrecisionVisibility {
    fn default() -> Self {
        Self {
            rulers: false,
            grid: false,
            guides: true,
            guides_locked: false,
        }
    }
}

/// Document-owned settings for the single V1 square grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GridSettings {
    pub spacing: f32,
    pub major_every: u8,
}

impl GridSettings {
    pub(crate) fn is_valid(self) -> bool {
        self.spacing.is_finite() && self.spacing > 0.0 && (1..=10).contains(&self.major_every)
    }
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            spacing: 10.0,
            major_every: 5,
        }
    }
}

/// Immutable state consumed by the toolbar and popover renderers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PrecisionUiSnapshot {
    pub popover_open: bool,
    pub snapping: SnappingSettings,
    pub visibility: PrecisionVisibility,
    pub grid: GridSettings,
    pub guide_count: usize,
}

type ValueCallback<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;
type CommandCallback = Rc<dyn Fn(&mut Window, &mut App)>;

/// Typed callback boundary between precision UI and application/editor state.
///
/// The default callbacks are inert, which makes snapshots easy to render in
/// previews. Production integration should install each relevant handler with
/// the builder methods.
#[derive(Clone)]
pub(crate) struct PrecisionCallbacks {
    popover_open_changed: ValueCallback<bool>,
    snapping_enabled_changed: ValueCallback<bool>,
    snap_target_changed: ValueCallback<(SnapTarget, bool)>,
    tolerance_changed: ValueCallback<f32>,
    overlay_visibility_changed: ValueCallback<(PrecisionOverlay, bool)>,
    guides_locked_changed: ValueCallback<bool>,
    grid_spacing_changed: ValueCallback<f32>,
    grid_major_every_changed: ValueCallback<u8>,
    clear_guides: CommandCallback,
}

impl PrecisionCallbacks {
    pub(crate) fn on_popover_open_changed(
        mut self,
        callback: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.popover_open_changed = Rc::new(callback);
        self
    }

    pub(crate) fn on_snapping_enabled_changed(
        mut self,
        callback: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.snapping_enabled_changed = Rc::new(callback);
        self
    }

    pub(crate) fn on_snap_target_changed(
        mut self,
        callback: impl Fn(SnapTarget, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.snap_target_changed = Rc::new(move |(target, enabled), window, cx| {
            callback(target, enabled, window, cx);
        });
        self
    }

    pub(crate) fn on_tolerance_changed(
        mut self,
        callback: impl Fn(f32, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.tolerance_changed = Rc::new(callback);
        self
    }

    pub(crate) fn on_overlay_visibility_changed(
        mut self,
        callback: impl Fn(PrecisionOverlay, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.overlay_visibility_changed = Rc::new(move |(overlay, visible), window, cx| {
            callback(overlay, visible, window, cx);
        });
        self
    }

    pub(crate) fn on_guides_locked_changed(
        mut self,
        callback: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.guides_locked_changed = Rc::new(callback);
        self
    }

    pub(crate) fn on_grid_spacing_changed(
        mut self,
        callback: impl Fn(f32, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.grid_spacing_changed = Rc::new(callback);
        self
    }

    pub(crate) fn on_grid_major_every_changed(
        mut self,
        callback: impl Fn(u8, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.grid_major_every_changed = Rc::new(callback);
        self
    }

    pub(crate) fn on_clear_guides(
        mut self,
        callback: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.clear_guides = Rc::new(callback);
        self
    }
}

impl Default for PrecisionCallbacks {
    fn default() -> Self {
        Self {
            popover_open_changed: Rc::new(|_, _, _| {}),
            snapping_enabled_changed: Rc::new(|_, _, _| {}),
            snap_target_changed: Rc::new(|_, _, _| {}),
            tolerance_changed: Rc::new(|_, _, _| {}),
            overlay_visibility_changed: Rc::new(|_, _, _| {}),
            guides_locked_changed: Rc::new(|_, _, _| {}),
            grid_spacing_changed: Rc::new(|_, _, _| {}),
            grid_major_every_changed: Rc::new(|_, _, _| {}),
            clear_guides: Rc::new(|_, _| {}),
        }
    }
}

/// Window-relative placement for the precision popover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PrecisionPopoverPlacement {
    pub top: f32,
    pub right: f32,
    pub max_height: f32,
}

/// Render the precision settings popover at a window-relative position.
pub(crate) fn render_precision_popover(
    snapshot: PrecisionUiSnapshot,
    callbacks: PrecisionCallbacks,
    placement: PrecisionPopoverPlacement,
) -> impl IntoElement {
    let snapping = snapshot.snapping;
    let visibility = snapshot.visibility;
    let grid_hidden_warning = snapping.grid && !visibility.grid;
    let guides_hidden_warning = snapping.guides && !visibility.guides;

    div()
        .id("precision-popover")
        .absolute()
        .top(px(placement.top))
        .right(px(placement.right))
        .w(px(252.0))
        .max_h(px(placement.max_height.max(1.0)))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .py(px(7.0))
        .bg(rgb(SURFACE_RAISED))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded(px(8.0))
        .shadow_lg()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(section_label("Snapping"))
        .child(toggle_row(
            "precision-master-snapping",
            "Snapping",
            snapping.enabled,
            None,
            callbacks.snapping_enabled_changed.clone(),
        ))
        .child(toggle_row(
            "precision-snap-objects",
            "Objects",
            snapping.objects,
            None,
            snap_target_callback(&callbacks, SnapTarget::Objects),
        ))
        .child(toggle_row(
            "precision-snap-guides",
            "Guides",
            snapping.guides,
            guides_hidden_warning.then_some("Hidden"),
            snap_target_callback(&callbacks, SnapTarget::Guides),
        ))
        .child(toggle_row(
            "precision-snap-grid",
            "Grid",
            snapping.grid,
            grid_hidden_warning.then_some("Hidden"),
            snap_target_callback(&callbacks, SnapTarget::Grid),
        ))
        .child(stepper_row_f32(
            F32StepperSpec {
                id: "precision-tolerance",
                label: "Tolerance",
                value: snapping.tolerance_px,
                suffix: "px",
                min: MIN_TOLERANCE_PX,
                max: MAX_TOLERANCE_PX,
                step: 1.0,
            },
            callbacks.tolerance_changed.clone(),
        ))
        .child(separator())
        .child(section_label("View"))
        .child(toggle_row(
            "precision-show-rulers",
            "Show rulers",
            visibility.rulers,
            None,
            overlay_callback(&callbacks, PrecisionOverlay::Rulers),
        ))
        .child(toggle_row(
            "precision-show-grid",
            "Show grid",
            visibility.grid,
            None,
            overlay_callback(&callbacks, PrecisionOverlay::Grid),
        ))
        .child(toggle_row(
            "precision-show-guides",
            "Show guides",
            visibility.guides,
            None,
            overlay_callback(&callbacks, PrecisionOverlay::Guides),
        ))
        .child(toggle_row(
            "precision-lock-guides",
            "Lock guides",
            visibility.guides_locked,
            None,
            callbacks.guides_locked_changed.clone(),
        ))
        .child(separator())
        .child(section_label("Grid"))
        .child(stepper_row_f32(
            F32StepperSpec {
                id: "precision-grid-spacing",
                label: "Spacing",
                value: snapshot.grid.spacing,
                suffix: "px",
                min: MIN_GRID_SPACING,
                max: MAX_GRID_SPACING,
                step: grid_spacing_step(snapshot.grid.spacing),
            },
            callbacks.grid_spacing_changed.clone(),
        ))
        .child(stepper_row_u8(
            "precision-grid-major",
            "Major every",
            snapshot.grid.major_every,
            1,
            10,
            callbacks.grid_major_every_changed.clone(),
        ))
        .child(separator())
        .child(clear_guides_button(
            snapshot.guide_count,
            callbacks.clear_guides.clone(),
        ))
}

fn snap_target_callback(callbacks: &PrecisionCallbacks, target: SnapTarget) -> ValueCallback<bool> {
    let callback = callbacks.snap_target_changed.clone();
    Rc::new(move |enabled, window, cx| callback((target, enabled), window, cx))
}

fn overlay_callback(
    callbacks: &PrecisionCallbacks,
    overlay: PrecisionOverlay,
) -> ValueCallback<bool> {
    let callback = callbacks.overlay_visibility_changed.clone();
    Rc::new(move |visible, window, cx| callback((overlay, visible), window, cx))
}

fn section_label(label: &'static str) -> impl IntoElement {
    div()
        .px(px(12.0))
        .pt(px(4.0))
        .pb(px(3.0))
        .text_size(px(9.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(TEXT_MUTED))
        .child(label.to_uppercase())
}

fn toggle_row(
    id: &'static str,
    label: &'static str,
    active: bool,
    status: Option<&'static str>,
    callback: ValueCallback<bool>,
) -> impl IntoElement {
    div()
        .id(id)
        .h(px(29.0))
        .mx(px(5.0))
        .px(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .rounded(px(4.0))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(toggle_indicator(active))
        .child(
            div()
                .flex_1()
                .text_size(px(11.0))
                .text_color(rgb(TEXT))
                .child(label),
        )
        .when_some(status, |row, status| {
            row.child(
                div()
                    .px(px(5.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .bg(rgba(0xfbbc0422))
                    .text_size(px(8.0))
                    .text_color(rgb(WARNING))
                    .child(status),
            )
        })
        .child(
            div()
                .text_size(px(9.0))
                .text_color(rgb(if active { ACCENT } else { TEXT_MUTED }))
                .child(if active { "On" } else { "Off" }),
        )
        .on_click(move |_, window, cx| callback(!active, window, cx))
}

fn toggle_indicator(active: bool) -> impl IntoElement {
    div()
        .w(px(14.0))
        .h(px(14.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .border_1()
        .border_color(rgb(if active { ACCENT } else { BORDER }))
        .bg(if active {
            rgb(ACCENT)
        } else {
            rgba(0x00000000)
        })
        .when(active, |indicator| {
            indicator.child(icon(Icon::Check, 10.0, rgb(0xffffff)))
        })
}

#[derive(Clone, Copy)]
struct F32StepperSpec {
    id: &'static str,
    label: &'static str,
    value: f32,
    suffix: &'static str,
    min: f32,
    max: f32,
    step: f32,
}

fn stepper_row_f32(spec: F32StepperSpec, callback: ValueCallback<f32>) -> impl IntoElement {
    let F32StepperSpec {
        id,
        label,
        value,
        suffix,
        min,
        max,
        step,
    } = spec;
    let value = if value.is_finite() {
        value.clamp(min, max)
    } else {
        min
    };
    let decrement = callback.clone();
    let increment = callback;

    div()
        .id(id)
        .h(px(30.0))
        .mx(px(5.0))
        .px(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .child(
            div()
                .flex_1()
                .text_size(px(11.0))
                .text_color(rgb(TEXT))
                .child(label),
        )
        .child(stepper_button(
            format!("{id}-decrement"),
            Icon::Minus,
            value > min,
            move |window, cx| {
                decrement((value - step).clamp(min, max), window, cx);
            },
        ))
        .child(
            div()
                .min_w(px(54.0))
                .text_align(gpui::TextAlign::Center)
                .text_size(px(10.0))
                .font_family("SFMono-Regular")
                .text_color(rgb(TEXT))
                .child(format!("{} {suffix}", format_number(value))),
        )
        .child(stepper_button(
            format!("{id}-increment"),
            Icon::Plus,
            value < max,
            move |window, cx| {
                increment((value + step).clamp(min, max), window, cx);
            },
        ))
}

fn stepper_row_u8(
    id: &'static str,
    label: &'static str,
    value: u8,
    min: u8,
    max: u8,
    callback: ValueCallback<u8>,
) -> impl IntoElement {
    let value = value.clamp(min, max);
    let decrement = callback.clone();
    let increment = callback;

    div()
        .id(id)
        .h(px(30.0))
        .mx(px(5.0))
        .px(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .child(
            div()
                .flex_1()
                .text_size(px(11.0))
                .text_color(rgb(TEXT))
                .child(label),
        )
        .child(stepper_button(
            format!("{id}-decrement"),
            Icon::Minus,
            value > min,
            move |window, cx| {
                decrement(value.saturating_sub(1).max(min), window, cx);
            },
        ))
        .child(
            div()
                .min_w(px(54.0))
                .text_align(gpui::TextAlign::Center)
                .text_size(px(10.0))
                .font_family("SFMono-Regular")
                .text_color(rgb(TEXT))
                .child(value.to_string()),
        )
        .child(stepper_button(
            format!("{id}-increment"),
            Icon::Plus,
            value < max,
            move |window, cx| {
                increment(value.saturating_add(1).min(max), window, cx);
            },
        ))
}

fn stepper_button(
    id: String,
    icon_kind: Icon,
    enabled: bool,
    callback: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(id))
        .w(px(24.0))
        .h(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(rgb(BORDER))
        .opacity(if enabled { 1.0 } else { 0.35 })
        .child(icon(
            icon_kind,
            12.0,
            rgb(if enabled { TEXT } else { TEXT_MUTED }),
        ))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                .on_click(move |_, window, cx| callback(window, cx))
        })
}

fn clear_guides_button(count: usize, callback: CommandCallback) -> impl IntoElement {
    let enabled = count > 0;
    div()
        .id("precision-clear-guides")
        .h(px(29.0))
        .mx(px(5.0))
        .px(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .rounded(px(4.0))
        .opacity(if enabled { 1.0 } else { 0.35 })
        .text_size(px(11.0))
        .text_color(rgb(TEXT))
        .child(div().flex_1().child("Clear guides"))
        .child(
            div()
                .text_size(px(9.0))
                .text_color(rgb(TEXT_MUTED))
                .child(count.to_string()),
        )
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                .on_click(move |_, window, cx| callback(window, cx))
        })
}

fn separator() -> impl IntoElement {
    div().h(px(1.0)).mx(px(8.0)).my(px(4.0)).bg(rgb(BORDER))
}

pub(crate) fn grid_spacing_step(spacing: f32) -> f32 {
    match spacing.abs() {
        value if value < 2.0 => 0.1,
        value if value < 20.0 => 1.0,
        value if value < 100.0 => 5.0,
        _ => 10.0,
    }
}

fn format_number(value: f32) -> String {
    if (value.round() - value).abs() < 0.000_1 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

/// A finite canvas-local rectangle in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScreenRect {
    pub min: Vec2,
    pub max: Vec2,
}

impl ScreenRect {
    pub(crate) fn from_size(size: Vec2) -> Self {
        Self {
            min: Vec2::ZERO,
            max: size,
        }
    }

    pub(crate) fn width(self) -> f32 {
        self.max.x - self.min.x
    }

    pub(crate) fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    pub(crate) fn is_valid(self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.width() > 0.0 && self.height() > 0.0
    }

    fn inset_top_left(self, amount: f32) -> Self {
        Self {
            min: self.min + Vec2::splat(amount.max(0.0)),
            max: self.max,
        }
    }
}

/// Canvas camera matching `editor_core::View`: screen = world * zoom + pan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverlayView {
    pub pan: Vec2,
    pub zoom: f32,
}

impl OverlayView {
    pub(crate) fn is_valid(self) -> bool {
        self.pan.is_finite() && self.zoom.is_finite() && self.zoom > 0.0
    }
}

/// Horizontal means a constant document Y; vertical means a constant X.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OverlayAxis {
    Horizontal,
    Vertical,
}

/// Minimal guide input needed to generate non-interactive overlay geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GuideOverlay {
    pub axis: OverlayAxis,
    pub position: f32,
    pub active: bool,
}

/// Visual role for a generated screen-space line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayLineKind {
    GridMinor,
    GridMajor,
    Guide,
    ActiveGuide,
    RulerTick,
}

/// A viewport-clipped line in canvas-local screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OverlayLine {
    pub start: Vec2,
    pub end: Vec2,
    pub kind: OverlayLineKind,
}

/// One generated ruler tick and its optional document-coordinate label.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RulerTick {
    pub axis: OverlayAxis,
    pub screen_position: f32,
    pub document_value: f32,
    pub major: bool,
    pub label: Option<String>,
}

/// An adaptive interval expressed as an integer multiple of a base interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AdaptiveInterval {
    pub document_units: f32,
    pub base_multiple: u64,
}

/// Major and minor ruler intervals in document units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RulerIntervals {
    pub major: f32,
    pub minor: f32,
}

/// Owned input for building all non-exporting precision overlay geometry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PrecisionOverlaySnapshot {
    pub viewport: ScreenRect,
    pub view: OverlayView,
    pub visibility: PrecisionVisibility,
    pub grid: GridSettings,
    pub guides: Vec<GuideOverlay>,
    pub ruler_thickness: f32,
}

/// Fully generated, bounded geometry suitable for GPUI painting or hit-testing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PrecisionOverlayGeometry {
    pub viewport: ScreenRect,
    pub content_viewport: ScreenRect,
    pub ruler_thickness: f32,
    pub grid_lines: Vec<OverlayLine>,
    pub guide_lines: Vec<OverlayLine>,
    pub ruler_ticks: Vec<RulerTick>,
    pub ruler_tick_lines: Vec<OverlayLine>,
}

/// Pick an integer multiple that respects both visual density and a hard cap.
pub(crate) fn adaptive_interval(
    base_document_units: f32,
    zoom: f32,
    minimum_screen_px: f32,
    viewport_span_px: f32,
    max_primitives: usize,
) -> Option<AdaptiveInterval> {
    if !base_document_units.is_finite()
        || base_document_units <= 0.0
        || !zoom.is_finite()
        || zoom <= 0.0
        || !minimum_screen_px.is_finite()
        || minimum_screen_px <= 0.0
        || !viewport_span_px.is_finite()
        || viewport_span_px <= 0.0
        || max_primitives == 0
    {
        return None;
    }

    // A span with both endpoints visible can contain one more primitive than
    // the number of intervals, so reserve one slot for the inclusive endpoint.
    let interval_capacity = max_primitives.saturating_sub(1).max(1);
    let cap_spacing = viewport_span_px / interval_capacity as f32;
    let required_screen_spacing = minimum_screen_px.max(cap_spacing);
    let raw_multiple = required_screen_spacing / (base_document_units * zoom);
    let base_multiple = nice_integer_ceiling(raw_multiple.max(1.0))?;
    let document_units = base_document_units * base_multiple as f32;
    document_units.is_finite().then_some(AdaptiveInterval {
        document_units,
        base_multiple,
    })
}

/// Compute readable decimal ruler intervals while respecting the tick cap.
pub(crate) fn adaptive_ruler_intervals(zoom: f32, viewport_span_px: f32) -> Option<RulerIntervals> {
    if !zoom.is_finite() || zoom <= 0.0 || !viewport_span_px.is_finite() || viewport_span_px <= 0.0
    {
        return None;
    }

    let major_cap = (MAX_RULER_TICKS_PER_AXIS / 5).max(1);
    let target_screen_px = MIN_RULER_MAJOR_SEPARATION_PX.max(viewport_span_px / major_cap as f32);
    let major = nice_decimal_ceiling(target_screen_px / zoom)?;
    let minor = major / 5.0;
    (major.is_finite() && minor.is_finite() && minor > 0.0)
        .then_some(RulerIntervals { major, minor })
}

/// Generate square-grid geometry clipped to the supplied viewport.
pub(crate) fn generate_grid_lines(
    viewport: ScreenRect,
    view: OverlayView,
    grid: GridSettings,
) -> Vec<OverlayLine> {
    if !viewport.is_valid() || !view.is_valid() || !grid.is_valid() {
        return Vec::new();
    }

    let widest_span = viewport.width().max(viewport.height());
    let interval_capacity = MAX_GRID_LINES_PER_AXIS.saturating_sub(1).max(1);
    let required_minor_spacing =
        MIN_GRID_MINOR_SEPARATION_PX.max(widest_span / interval_capacity as f32);
    let show_minor = grid.spacing * view.zoom >= required_minor_spacing;

    let (interval, classify_major) = if show_minor {
        (grid.spacing, true)
    } else {
        let major_base = grid.spacing * f32::from(grid.major_every);
        let Some(interval) = adaptive_interval(
            major_base,
            view.zoom,
            MIN_GRID_MAJOR_SEPARATION_PX,
            widest_span,
            MAX_GRID_LINES_PER_AXIS,
        ) else {
            return Vec::new();
        };
        (interval.document_units, false)
    };

    let mut lines = Vec::new();
    append_grid_axis_lines(
        &mut lines,
        OverlayAxis::Vertical,
        viewport,
        view,
        interval,
        grid,
        classify_major,
    );
    append_grid_axis_lines(
        &mut lines,
        OverlayAxis::Horizontal,
        viewport,
        view,
        interval,
        grid,
        classify_major,
    );
    lines
}

/// Generate ruler ticks for both axes, bounded by the supplied viewport.
pub(crate) fn generate_ruler_ticks(viewport: ScreenRect, view: OverlayView) -> Vec<RulerTick> {
    if !viewport.is_valid() || !view.is_valid() {
        return Vec::new();
    }

    let mut ticks = Vec::new();
    append_ruler_axis_ticks(&mut ticks, OverlayAxis::Vertical, viewport, view);
    append_ruler_axis_ticks(&mut ticks, OverlayAxis::Horizontal, viewport, view);
    ticks
}

/// Generate horizontal/vertical guide lines, preserving source order by index.
pub(crate) fn generate_guide_lines(
    viewport: ScreenRect,
    view: OverlayView,
    guides: &[GuideOverlay],
) -> Vec<OverlayLine> {
    if !viewport.is_valid() || !view.is_valid() {
        return Vec::new();
    }

    guides
        .iter()
        .take(MAX_GUIDE_LINES)
        .filter(|guide| guide.position.is_finite())
        .filter_map(|guide| {
            let position = guide
                .position
                .mul_add(view.zoom, axis_pan(view, guide.axis));
            axis_position_in_viewport(viewport, guide.axis, position).then(|| {
                line_at_axis(
                    viewport,
                    guide.axis,
                    position,
                    if guide.active {
                        OverlayLineKind::ActiveGuide
                    } else {
                        OverlayLineKind::Guide
                    },
                )
            })
        })
        .collect()
}

/// Build all requested overlay geometry with rulers reserving top/left chrome.
pub(crate) fn build_precision_overlay_geometry(
    snapshot: &PrecisionOverlaySnapshot,
) -> PrecisionOverlayGeometry {
    let available = snapshot.viewport.width().min(snapshot.viewport.height());
    let preferred_thickness =
        if snapshot.ruler_thickness.is_finite() && snapshot.ruler_thickness > 0.0 {
            snapshot.ruler_thickness
        } else {
            DEFAULT_RULER_THICKNESS
        };
    let ruler_thickness = if available.is_finite() && available > 1.0 {
        preferred_thickness.clamp(1.0, available - 1.0)
    } else {
        0.0
    };
    let rulers_renderable = snapshot.visibility.rulers && ruler_thickness > 0.0;
    let content_viewport = if rulers_renderable {
        snapshot.viewport.inset_top_left(ruler_thickness)
    } else {
        snapshot.viewport
    };

    let grid_lines = if snapshot.visibility.grid {
        generate_grid_lines(content_viewport, snapshot.view, snapshot.grid)
    } else {
        Vec::new()
    };
    let guide_lines = if snapshot.visibility.guides {
        generate_guide_lines(content_viewport, snapshot.view, &snapshot.guides)
    } else {
        Vec::new()
    };
    let ruler_ticks = if rulers_renderable {
        generate_ruler_ticks(content_viewport, snapshot.view)
    } else {
        Vec::new()
    };
    let ruler_tick_lines = ruler_ticks
        .iter()
        .map(|tick| ruler_tick_line(snapshot.viewport, ruler_thickness, tick))
        .collect();

    PrecisionOverlayGeometry {
        viewport: snapshot.viewport,
        content_viewport,
        ruler_thickness,
        grid_lines,
        guide_lines,
        ruler_ticks,
        ruler_tick_lines,
    }
}

/// Render the grid as the bottommost non-exporting canvas layer.
///
/// Callers must insert this before the artwork canvas. Guides, rulers, and
/// interaction chrome belong in [`render_precision_overlay`] above artwork.
pub(crate) fn render_precision_grid(geometry: PrecisionOverlayGeometry) -> impl IntoElement {
    div().id("precision-grid").absolute().inset_0().child(
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                paint_overlay_lines(&geometry.grid_lines, bounds, window);
            },
        )
        .size_full(),
    )
}

/// Render guides and rulers as non-exporting chrome above artwork.
pub(crate) fn render_precision_overlay(geometry: PrecisionOverlayGeometry) -> impl IntoElement {
    let ruler_visible = geometry.content_viewport != geometry.viewport;
    let ruler_labels = geometry
        .ruler_ticks
        .iter()
        .filter_map(render_ruler_label)
        .collect::<Vec<_>>();
    let painted_geometry = geometry.clone();

    div()
        .id("precision-overlay")
        .absolute()
        .inset_0()
        .when(ruler_visible, |overlay| {
            overlay
                .child(ruler_background(&geometry, OverlayAxis::Horizontal))
                .child(ruler_background(&geometry, OverlayAxis::Vertical))
        })
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    paint_overlay_lines(&painted_geometry.guide_lines, bounds, window);
                    paint_overlay_lines(&painted_geometry.ruler_tick_lines, bounds, window);
                },
            )
            .size_full(),
        )
        .children(ruler_labels)
}

fn append_grid_axis_lines(
    lines: &mut Vec<OverlayLine>,
    axis: OverlayAxis,
    viewport: ScreenRect,
    view: OverlayView,
    interval: f32,
    grid: GridSettings,
    classify_major: bool,
) {
    let (screen_min, screen_max) = axis_screen_range(viewport, axis);
    let samples = axis_samples(
        screen_min,
        screen_max,
        axis_pan(view, axis),
        view.zoom,
        interval,
        MAX_GRID_LINES_PER_AXIS,
    );
    for sample in samples {
        let kind = if classify_major
            && sample.index.rem_euclid(f64::from(grid.major_every)).abs() < 0.25
        {
            OverlayLineKind::GridMajor
        } else if classify_major {
            OverlayLineKind::GridMinor
        } else {
            OverlayLineKind::GridMajor
        };
        lines.push(line_at_axis(viewport, axis, sample.screen, kind));
    }
}

fn append_ruler_axis_ticks(
    ticks: &mut Vec<RulerTick>,
    axis: OverlayAxis,
    viewport: ScreenRect,
    view: OverlayView,
) {
    let (screen_min, screen_max) = axis_screen_range(viewport, axis);
    let Some(intervals) = adaptive_ruler_intervals(view.zoom, screen_max - screen_min) else {
        return;
    };
    let samples = axis_samples(
        screen_min,
        screen_max,
        axis_pan(view, axis),
        view.zoom,
        intervals.minor,
        MAX_RULER_TICKS_PER_AXIS,
    );
    let major_ratio = f64::from(intervals.major / intervals.minor);

    ticks.extend(samples.into_iter().map(|sample| {
        let major = sample.index.rem_euclid(major_ratio).abs() < 0.25;
        let document_value = (sample.index * f64::from(intervals.minor)) as f32;
        RulerTick {
            axis,
            screen_position: sample.screen,
            document_value,
            major,
            label: major.then(|| format_document_coordinate(document_value)),
        }
    }));
}

#[derive(Clone, Copy, Debug)]
struct AxisSample {
    index: f64,
    screen: f32,
}

fn axis_samples(
    screen_min: f32,
    screen_max: f32,
    pan: f32,
    zoom: f32,
    document_interval: f32,
    max_count: usize,
) -> Vec<AxisSample> {
    if max_count == 0
        || !screen_min.is_finite()
        || !screen_max.is_finite()
        || screen_max <= screen_min
        || !pan.is_finite()
        || !zoom.is_finite()
        || zoom <= 0.0
        || !document_interval.is_finite()
        || document_interval <= 0.0
    {
        return Vec::new();
    }

    let screen_min = f64::from(screen_min);
    let screen_max = f64::from(screen_max);
    let pan = f64::from(pan);
    let screen_interval = f64::from(document_interval) * f64::from(zoom);
    let first_index = ((screen_min - pan) / screen_interval).ceil();
    let last_index = ((screen_max - pan) / screen_interval).floor();
    if !first_index.is_finite() || !last_index.is_finite() || last_index < first_index {
        return Vec::new();
    }

    let count = ((last_index - first_index + 1.0).min(max_count as f64)) as usize;
    (0..count)
        .filter_map(|offset| {
            let index = first_index + offset as f64;
            let screen = index.mul_add(screen_interval, pan);
            (screen.is_finite() && screen >= screen_min - 0.5 && screen <= screen_max + 0.5)
                .then_some(AxisSample {
                    index,
                    screen: screen as f32,
                })
        })
        .collect()
}

fn nice_integer_ceiling(value: f32) -> Option<u64> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    if value <= 1.0 {
        return Some(1);
    }

    let exponent = value.log10().floor();
    let magnitude = 10_f32.powf(exponent);
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return None;
    }
    let normalized = value / magnitude;
    let nice = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    let result = nice * magnitude;
    (result.is_finite() && result <= u64::MAX as f32).then(|| result.ceil() as u64)
}

fn nice_decimal_ceiling(value: f32) -> Option<f32> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let exponent = value.log10().floor();
    let magnitude = 10_f32.powf(exponent);
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return None;
    }
    let normalized = value / magnitude;
    let nice = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    let result = nice * magnitude;
    (result.is_finite() && result > 0.0).then_some(result)
}

fn axis_pan(view: OverlayView, axis: OverlayAxis) -> f32 {
    match axis {
        OverlayAxis::Horizontal => view.pan.y,
        OverlayAxis::Vertical => view.pan.x,
    }
}

fn axis_screen_range(viewport: ScreenRect, axis: OverlayAxis) -> (f32, f32) {
    match axis {
        OverlayAxis::Horizontal => (viewport.min.y, viewport.max.y),
        OverlayAxis::Vertical => (viewport.min.x, viewport.max.x),
    }
}

fn axis_position_in_viewport(viewport: ScreenRect, axis: OverlayAxis, position: f32) -> bool {
    let (min, max) = axis_screen_range(viewport, axis);
    position.is_finite() && position >= min && position <= max
}

fn line_at_axis(
    viewport: ScreenRect,
    axis: OverlayAxis,
    position: f32,
    kind: OverlayLineKind,
) -> OverlayLine {
    match axis {
        OverlayAxis::Horizontal => OverlayLine {
            start: Vec2::new(viewport.min.x, position),
            end: Vec2::new(viewport.max.x, position),
            kind,
        },
        OverlayAxis::Vertical => OverlayLine {
            start: Vec2::new(position, viewport.min.y),
            end: Vec2::new(position, viewport.max.y),
            kind,
        },
    }
}

fn ruler_tick_line(viewport: ScreenRect, thickness: f32, tick: &RulerTick) -> OverlayLine {
    let tick_length = if tick.major { 8.0 } else { 4.0 };
    match tick.axis {
        OverlayAxis::Vertical => OverlayLine {
            start: Vec2::new(
                tick.screen_position,
                viewport.min.y + thickness - tick_length,
            ),
            end: Vec2::new(tick.screen_position, viewport.min.y + thickness),
            kind: OverlayLineKind::RulerTick,
        },
        OverlayAxis::Horizontal => OverlayLine {
            start: Vec2::new(
                viewport.min.x + thickness - tick_length,
                tick.screen_position,
            ),
            end: Vec2::new(viewport.min.x + thickness, tick.screen_position),
            kind: OverlayLineKind::RulerTick,
        },
    }
}

fn format_document_coordinate(value: f32) -> String {
    let normalized = if value.abs() < 0.000_1 { 0.0 } else { value };
    if normalized.abs() >= 100_000.0 || (normalized != 0.0 && normalized.abs() < 0.01) {
        format!("{normalized:.0e}")
    } else {
        format_number(normalized)
    }
}

fn ruler_background(geometry: &PrecisionOverlayGeometry, axis: OverlayAxis) -> impl IntoElement {
    let viewport = geometry.viewport;
    let thickness = geometry.ruler_thickness;
    match axis {
        OverlayAxis::Horizontal => div()
            .absolute()
            .left(px(viewport.min.x))
            .top(px(viewport.min.y))
            .w(px(viewport.width()))
            .h(px(thickness))
            .bg(rgb(SURFACE))
            .border_b_1()
            .border_color(rgb(BORDER))
            .into_any_element(),
        OverlayAxis::Vertical => div()
            .absolute()
            .left(px(viewport.min.x))
            .top(px(viewport.min.y))
            .w(px(thickness))
            .h(px(viewport.height()))
            .bg(rgb(SURFACE))
            .border_r_1()
            .border_color(rgb(BORDER))
            .into_any_element(),
    }
}

fn render_ruler_label(tick: &RulerTick) -> Option<gpui::AnyElement> {
    let label = tick.label.clone()?;
    let element = match tick.axis {
        OverlayAxis::Vertical => div()
            .absolute()
            .left(px(tick.screen_position + 2.0))
            .top(px(1.0))
            .text_size(px(8.0))
            .text_color(rgb(TEXT_MUTED))
            .child(SharedString::from(label)),
        OverlayAxis::Horizontal => div()
            .absolute()
            .left(px(2.0))
            .top(px(tick.screen_position + 1.0))
            .text_size(px(8.0))
            .text_color(rgb(TEXT_MUTED))
            .child(SharedString::from(label)),
    };
    Some(element.into_any_element())
}

fn paint_overlay_lines(lines: &[OverlayLine], bounds: Bounds<Pixels>, window: &mut Window) {
    for line in lines {
        paint_overlay_line(bounds, window, *line);
    }
}

fn paint_overlay_line(bounds: Bounds<Pixels>, window: &mut Window, line: OverlayLine) {
    let min = line.start.min(line.end);
    let max = line.start.max(line.end);
    let max = Vec2::new(max.x.max(min.x + 1.0), max.y.max(min.y + 1.0));
    let color = match line.kind {
        OverlayLineKind::GridMinor => gpui::Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.07,
        },
        OverlayLineKind::GridMajor => gpui::Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.14,
        },
        OverlayLineKind::Guide => gpui::Rgba {
            r: 0.2,
            g: 0.65,
            b: 1.0,
            a: 0.85,
        },
        OverlayLineKind::ActiveGuide => gpui::Rgba {
            r: 1.0,
            g: 0.2,
            b: 0.55,
            a: 1.0,
        },
        OverlayLineKind::RulerTick => gpui::Rgba {
            r: 0.66,
            g: 0.68,
            b: 0.72,
            a: 0.75,
        },
    };
    window.paint_quad(gpui::PaintQuad {
        bounds: Bounds::from_corners(
            gpui::point(bounds.origin.x + px(min.x), bounds.origin.y + px(min.y)),
            gpui::point(bounds.origin.x + px(max.x), bounds.origin.y + px(max.y)),
        ),
        corner_radii: gpui::Corners::all(px(0.0)),
        background: gpui::Background::from(color),
        border_widths: gpui::Edges::all(px(0.0)),
        border_color: gpui::transparent_black(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport(width: f32, height: f32) -> ScreenRect {
        ScreenRect::from_size(Vec2::new(width, height))
    }

    #[test]
    fn adaptive_interval_uses_nice_integer_multiples() {
        let interval = adaptive_interval(10.0, 0.1, 24.0, 1_000.0, 2_048).expect("valid interval");

        assert_eq!(interval.base_multiple, 50);
        assert_eq!(interval.document_units, 500.0);
        assert!(interval.document_units * 0.1 >= 24.0);
    }

    #[test]
    fn adaptive_interval_rejects_invalid_inputs() {
        assert_eq!(adaptive_interval(0.0, 1.0, 8.0, 100.0, 10), None);
        assert_eq!(adaptive_interval(10.0, f32::NAN, 8.0, 100.0, 10), None);
        assert_eq!(adaptive_interval(10.0, 1.0, 8.0, 100.0, 0), None);
    }

    #[test]
    fn ruler_intervals_are_readable_and_zoom_adaptive() {
        let zoomed_out = adaptive_ruler_intervals(0.1, 1_200.0).expect("valid interval");
        let zoomed_in = adaptive_ruler_intervals(4.0, 1_200.0).expect("valid interval");

        assert!(zoomed_out.major > zoomed_in.major);
        assert!(zoomed_out.major * 0.1 >= MIN_RULER_MAJOR_SEPARATION_PX);
        assert!(zoomed_in.major * 4.0 >= MIN_RULER_MAJOR_SEPARATION_PX);
        assert_eq!(zoomed_out.minor, zoomed_out.major / 5.0);
    }

    #[test]
    fn dense_grid_generation_is_bounded_and_clipped() {
        let viewport = viewport(100_000.0, 80_000.0);
        let lines = generate_grid_lines(
            viewport,
            OverlayView {
                pan: Vec2::new(-12_345.0, 98_765.0),
                zoom: 0.000_1,
            },
            GridSettings {
                spacing: 0.1,
                major_every: 5,
            },
        );

        assert!(lines.len() <= MAX_GRID_LINES_PER_AXIS * 2);
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|line| {
            line.start.x >= viewport.min.x
                && line.start.y >= viewport.min.y
                && line.end.x <= viewport.max.x
                && line.end.y <= viewport.max.y
        }));
        assert!(lines
            .iter()
            .all(|line| line.kind == OverlayLineKind::GridMajor));
    }

    #[test]
    fn visible_minor_grid_marks_major_intervals() {
        let lines = generate_grid_lines(
            viewport(120.0, 120.0),
            OverlayView {
                pan: Vec2::ZERO,
                zoom: 2.0,
            },
            GridSettings {
                spacing: 10.0,
                major_every: 5,
            },
        );

        assert!(lines
            .iter()
            .any(|line| line.kind == OverlayLineKind::GridMinor));
        assert!(lines
            .iter()
            .any(|line| line.kind == OverlayLineKind::GridMajor));
    }

    #[test]
    fn ruler_tick_generation_is_bounded() {
        let ticks = generate_ruler_ticks(
            viewport(1_000_000.0, 800_000.0),
            OverlayView {
                pan: Vec2::new(-50_000.0, 40_000.0),
                zoom: 0.01,
            },
        );

        assert!(ticks.len() <= MAX_RULER_TICKS_PER_AXIS * 2);
        assert!(ticks.iter().any(|tick| tick.major && tick.label.is_some()));
        assert!(ticks.iter().all(|tick| match tick.axis {
            OverlayAxis::Vertical => (0.0..=1_000_000.0).contains(&tick.screen_position),
            OverlayAxis::Horizontal => (0.0..=800_000.0).contains(&tick.screen_position),
        }));
    }

    #[test]
    fn guide_generation_filters_invalid_and_offscreen_guides() {
        let lines = generate_guide_lines(
            viewport(200.0, 100.0),
            OverlayView {
                pan: Vec2::new(10.0, 20.0),
                zoom: 2.0,
            },
            &[
                GuideOverlay {
                    axis: OverlayAxis::Vertical,
                    position: 25.0,
                    active: false,
                },
                GuideOverlay {
                    axis: OverlayAxis::Horizontal,
                    position: 10.0,
                    active: true,
                },
                GuideOverlay {
                    axis: OverlayAxis::Vertical,
                    position: 500.0,
                    active: false,
                },
                GuideOverlay {
                    axis: OverlayAxis::Horizontal,
                    position: f32::NAN,
                    active: false,
                },
            ],
        );

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start.x, 60.0);
        assert_eq!(lines[0].kind, OverlayLineKind::Guide);
        assert_eq!(lines[1].start.y, 40.0);
        assert_eq!(lines[1].kind, OverlayLineKind::ActiveGuide);
    }

    #[test]
    fn rulers_reserve_overlay_space_without_changing_camera_coordinates() {
        let geometry = build_precision_overlay_geometry(&PrecisionOverlaySnapshot {
            viewport: viewport(800.0, 600.0),
            view: OverlayView {
                pan: Vec2::ZERO,
                zoom: 1.0,
            },
            visibility: PrecisionVisibility {
                rulers: true,
                grid: true,
                guides: true,
                guides_locked: false,
            },
            grid: GridSettings::default(),
            guides: vec![GuideOverlay {
                axis: OverlayAxis::Vertical,
                position: 100.0,
                active: false,
            }],
            ruler_thickness: DEFAULT_RULER_THICKNESS,
        });

        assert_eq!(
            geometry.content_viewport.min,
            Vec2::splat(DEFAULT_RULER_THICKNESS)
        );
        assert!(geometry.grid_lines.iter().all(|line| {
            line.start.x >= DEFAULT_RULER_THICKNESS && line.start.y >= DEFAULT_RULER_THICKNESS
        }));
        assert_eq!(geometry.guide_lines[0].start.x, 100.0);
    }
}
