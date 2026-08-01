//! Modeless, in-window Color Library utility panel.
//!
//! This module deliberately owns presentation state only. Document mutations are
//! reported as [`ColorLibraryPanelEvent`] values so the application root can route
//! them through `editor_core` commands and history.

use std::{cell::Cell, collections::HashSet, ops::Range, rc::Rc};

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, AnyElement, App, Bounds,
    ClipboardItem, Context, CursorStyle, DragMoveEvent, Element, ElementId, ElementInputHandler,
    Entity, EntityId, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId,
    IntoElement, KeyBinding, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, Render, ShapedLine, SharedString, Style, TextRun,
    UTF16Selection, UnderlineStyle, Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::assets::{icon, Icon};

const SURFACE: u32 = 0x202124;
const SURFACE_RAISED: u32 = 0x292a2e;
const SURFACE_HOVER: u32 = 0x35363b;
const SURFACE_INPUT: u32 = 0x17181a;
const BORDER: u32 = 0x414349;
const TEXT: u32 = 0xf1f3f4;
const MUTED: u32 = 0xa8abb2;
const ACCENT: u32 = 0x0c8ce9;
const DANGER: u32 = 0xf24822;

const WINDOW_MARGIN: f32 = 8.0;
const DEFAULT_WIDTH: f32 = 520.0;
const DEFAULT_HEIGHT: f32 = 460.0;
const MIN_WIDTH: f32 = 360.0;
const MIN_HEIGHT: f32 = 220.0;
const MAX_WIDTH: f32 = 760.0;
const MAX_HEIGHT: f32 = 820.0;

/// App-local identity used to map a core color-group ID into the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ColorLibraryGroupId(pub u64);

/// App-local identity used to map a core Saved Color ID into the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ColorLibraryColorId(pub u64);

/// A real group or the virtual Ungrouped section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ColorLibrarySectionId {
    Ungrouped,
    Group(ColorLibraryGroupId),
}

impl ColorLibrarySectionId {
    fn key(self) -> String {
        match self {
            Self::Ungrouped => "ungrouped".to_owned(),
            Self::Group(id) => format!("group-{}", id.0),
        }
    }

    fn group(self) -> Option<ColorLibraryGroupId> {
        match self {
            Self::Ungrouped => None,
            Self::Group(id) => Some(id),
        }
    }
}

/// Authored ordering mode for one Color Library section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ColorLibrarySortMode {
    #[default]
    Manual,
    Name,
    HueAndShades,
    Lightness,
    Chroma,
    Brightness,
}

impl ColorLibrarySortMode {
    const ALL: [Self; 6] = [
        Self::Manual,
        Self::Name,
        Self::HueAndShades,
        Self::Lightness,
        Self::Chroma,
        Self::Brightness,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::Name => "Name",
            Self::HueAndShades => "Hue & shades",
            Self::Lightness => "Lightness",
            Self::Chroma => "Chroma",
            Self::Brightness => "Brightness",
        }
    }
}

/// Direction paired with a section's sort mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ColorLibrarySortDirection {
    #[default]
    Ascending,
    Descending,
}

impl ColorLibrarySortDirection {
    fn icon(self) -> Icon {
        match self {
            Self::Ascending => Icon::ArrowUp,
            Self::Descending => Icon::ArrowDown,
        }
    }

    fn reversed(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// Complete sort selection for one section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ColorLibrarySort {
    pub mode: ColorLibrarySortMode,
    pub direction: ColorLibrarySortDirection,
}

/// One Saved Color row prepared for presentation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColorLibraryRow {
    pub id: ColorLibraryColorId,
    pub name: Option<SharedString>,
    pub rgba: [f32; 4],
    /// Canonical display/edit value supplied by the application, normally HEX.
    pub value: SharedString,
    /// Zero-based authored order. The panel presents it as one-based.
    pub manual_order: usize,
}

impl ColorLibraryRow {
    fn fallback_name(&self) -> SharedString {
        self.name.clone().unwrap_or_else(|| self.value.clone())
    }
}

/// One collapsible table section. Sections must already be in authored order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColorLibrarySection {
    pub id: ColorLibrarySectionId,
    pub name: SharedString,
    pub sort: ColorLibrarySort,
    /// Rows must already reflect the section's active core-defined sort.
    pub rows: Vec<ColorLibraryRow>,
}

/// Read-only panel input. The core remains authoritative for all document data.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ColorLibrarySnapshot {
    pub sections: Vec<ColorLibrarySection>,
}

/// Workspace-owned placement restored independently from the document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ColorLibraryPanelGeometry {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for ColorLibraryPanelGeometry {
    fn default() -> Self {
        Self {
            left: 72.0,
            top: 72.0,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        }
    }
}

impl ColorLibraryPanelGeometry {
    /// Clamp restored or dragged geometry so the panel remains reachable.
    pub(crate) fn clamped(self, viewport: gpui::Size<Pixels>) -> Self {
        let available_width = (viewport.width.0 - WINDOW_MARGIN * 2.0).max(1.0);
        let available_height = (viewport.height.0 - WINDOW_MARGIN * 2.0).max(1.0);
        let min_width = MIN_WIDTH.min(available_width);
        let min_height = MIN_HEIGHT.min(available_height);
        let width = self.width.clamp(min_width, MAX_WIDTH.min(available_width));
        let height = self
            .height
            .clamp(min_height, MAX_HEIGHT.min(available_height));
        let max_left = (viewport.width.0 - WINDOW_MARGIN - width).max(WINDOW_MARGIN);
        let max_top = (viewport.height.0 - WINDOW_MARGIN - height).max(WINDOW_MARGIN);

        Self {
            left: self.left.clamp(WINDOW_MARGIN, max_left),
            top: self.top.clamp(WINDOW_MARGIN, max_top),
            width,
            height,
        }
    }
}

/// Workspace state the application may persist between launches.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ColorLibraryPanelPresentation {
    pub geometry: ColorLibraryPanelGeometry,
    pub collapsed_sections: HashSet<ColorLibrarySectionId>,
}

/// Semantic requests emitted by the panel. Document-changing requests should be
/// executed through the core so validation, history, and automation stay aligned.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ColorLibraryPanelEvent {
    Dismiss,
    QuickAdd,
    AddGroup,
    RenameGroup {
        group: ColorLibraryGroupId,
        name: String,
    },
    RemoveGroup {
        group: ColorLibraryGroupId,
    },
    SetSort {
        section: ColorLibrarySectionId,
        sort: ColorLibrarySort,
    },
    EditSwatch {
        color: ColorLibraryColorId,
    },
    RenameColor {
        color: ColorLibraryColorId,
        name: Option<String>,
    },
    SetColorValue {
        color: ColorLibraryColorId,
        value: String,
    },
    SetColorOrder {
        color: ColorLibraryColorId,
        one_based_order: usize,
    },
    DuplicateColor {
        color: ColorLibraryColorId,
    },
    RemoveColor {
        color: ColorLibraryColorId,
    },
    MoveColorBefore {
        color: ColorLibraryColorId,
        section: ColorLibrarySectionId,
        before: Option<ColorLibraryColorId>,
    },
    MoveGroupBefore {
        group: ColorLibraryGroupId,
        before: ColorLibraryGroupId,
    },
    PresentationChanged(ColorLibraryPanelPresentation),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditTarget {
    GroupName(ColorLibraryGroupId),
    ColorName(ColorLibraryColorId),
    ColorValue(ColorLibraryColorId),
    ColorOrder(ColorLibraryColorId),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PanelFocus {
    #[default]
    QuickAdd,
    AddGroup,
    Close,
    Collapse(ColorLibrarySectionId),
    GroupName(ColorLibraryGroupId),
    Sort(ColorLibrarySectionId),
    RemoveGroup(ColorLibraryGroupId),
    Swatch(ColorLibraryColorId),
    Order(ColorLibraryColorId),
    Name(ColorLibraryColorId),
    Value(ColorLibraryColorId),
    Duplicate(ColorLibraryColorId),
    Remove(ColorLibraryColorId),
}

/// Stateful GPUI view for the modeless Color Library panel.
pub(crate) struct ColorLibraryPanel {
    snapshot: ColorLibrarySnapshot,
    geometry: ColorLibraryPanelGeometry,
    collapsed_sections: HashSet<ColorLibrarySectionId>,
    search_query: String,
    search_input: Entity<PanelTextInput>,
    editing: Option<(EditTarget, Entity<PanelTextInput>)>,
    sort_menu: Option<ColorLibrarySectionId>,
    focus_handle: FocusHandle,
    keyboard_focus: PanelFocus,
}

impl ColorLibraryPanel {
    pub(crate) fn new(
        snapshot: ColorLibrarySnapshot,
        presentation: ColorLibraryPanelPresentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|input_cx| {
            PanelTextInput::new(
                "color-library-search-input",
                "",
                "Search name, group, or value",
                false,
                window,
                input_cx,
            )
        });
        cx.subscribe_in(
            &search_input,
            window,
            |panel, _, event: &PanelTextInputEvent, window, cx| {
                match event {
                    PanelTextInputEvent::Changed(value)
                    | PanelTextInputEvent::Commit(value)
                    | PanelTextInputEvent::Blur(value) => {
                        panel.search_query.clone_from(value);
                    }
                    PanelTextInputEvent::Cancel => {
                        panel.search_query.clear();
                        panel.search_input.update(cx, |input, cx| input.clear(cx));
                        panel.focus_handle.focus(window);
                    }
                }
                cx.notify();
            },
        )
        .detach();

        let geometry = presentation.geometry.clamped(window.viewport_size());
        Self {
            snapshot,
            geometry,
            collapsed_sections: presentation.collapsed_sections,
            search_query: String::new(),
            search_input,
            editing: None,
            sort_menu: None,
            focus_handle: cx.focus_handle(),
            keyboard_focus: PanelFocus::default(),
        }
    }

    pub(crate) fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    pub(crate) fn presentation(&self) -> ColorLibraryPanelPresentation {
        ColorLibraryPanelPresentation {
            geometry: self.geometry,
            collapsed_sections: self.collapsed_sections.clone(),
        }
    }

    /// Replace core-owned data after a semantic command completes.
    pub(crate) fn set_snapshot(&mut self, snapshot: ColorLibrarySnapshot, cx: &mut Context<Self>) {
        self.snapshot = snapshot;
        if self
            .editing
            .as_ref()
            .is_some_and(|(target, _)| !self.contains_edit_target(*target))
        {
            self.editing = None;
        }
        self.normalize_keyboard_focus();
        cx.notify();
    }

    fn contains_edit_target(&self, target: EditTarget) -> bool {
        match target {
            EditTarget::GroupName(group) => self
                .snapshot
                .sections
                .iter()
                .any(|section| section.id == ColorLibrarySectionId::Group(group)),
            EditTarget::ColorName(color)
            | EditTarget::ColorValue(color)
            | EditTarget::ColorOrder(color) => self
                .snapshot
                .sections
                .iter()
                .any(|section| section.rows.iter().any(|row| row.id == color)),
        }
    }

    fn visible_sections(&self) -> Vec<ColorLibrarySection> {
        self.snapshot
            .sections
            .iter()
            .filter_map(|section| filtered_section(section, &self.search_query))
            .collect()
    }

    fn focus_targets(&self) -> Vec<PanelFocus> {
        let mut targets = vec![
            PanelFocus::QuickAdd,
            PanelFocus::AddGroup,
            PanelFocus::Close,
        ];
        for section in self.visible_sections() {
            targets.push(PanelFocus::Collapse(section.id));
            if let Some(group) = section.id.group() {
                targets.push(PanelFocus::GroupName(group));
            }
            targets.push(PanelFocus::Sort(section.id));
            if let Some(group) = section.id.group() {
                targets.push(PanelFocus::RemoveGroup(group));
            }
            if !self.collapsed_sections.contains(&section.id) {
                for row in section.rows {
                    targets.extend([
                        PanelFocus::Swatch(row.id),
                        PanelFocus::Order(row.id),
                        PanelFocus::Name(row.id),
                        PanelFocus::Value(row.id),
                        PanelFocus::Duplicate(row.id),
                        PanelFocus::Remove(row.id),
                    ]);
                }
            }
        }
        targets
    }

    fn normalize_keyboard_focus(&mut self) {
        let targets = self.focus_targets();
        if !targets.contains(&self.keyboard_focus) {
            self.keyboard_focus = targets.first().copied().unwrap_or_default();
        }
    }

    fn focus_relative(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.focus_targets();
        if targets.is_empty() {
            return;
        }
        let current = targets
            .iter()
            .position(|target| *target == self.keyboard_focus)
            .unwrap_or_default();
        let next = if reverse {
            current.checked_sub(1).unwrap_or(targets.len() - 1)
        } else {
            (current + 1) % targets.len()
        };
        self.keyboard_focus = targets[next];
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "tab" => self.focus_relative(event.keystroke.modifiers.shift, window, cx),
            "down" if self.editing.is_none() => self.focus_relative(false, window, cx),
            "up" if self.editing.is_none() => self.focus_relative(true, window, cx),
            "enter" | "space" if self.editing.is_none() => self.activate_keyboard_focus(window, cx),
            "escape" if self.sort_menu.take().is_some() => cx.notify(),
            "escape" if self.editing.is_none() => cx.emit(ColorLibraryPanelEvent::Dismiss),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn activate_keyboard_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.keyboard_focus {
            PanelFocus::QuickAdd => cx.emit(ColorLibraryPanelEvent::QuickAdd),
            PanelFocus::AddGroup => cx.emit(ColorLibraryPanelEvent::AddGroup),
            PanelFocus::Close => cx.emit(ColorLibraryPanelEvent::Dismiss),
            PanelFocus::Collapse(section) => self.toggle_collapsed(section, cx),
            PanelFocus::GroupName(group) => {
                if let Some(section) = self
                    .snapshot
                    .sections
                    .iter()
                    .find(|section| section.id == ColorLibrarySectionId::Group(group))
                {
                    self.begin_edit(
                        EditTarget::GroupName(group),
                        section.name.to_string(),
                        window,
                        cx,
                    );
                }
            }
            PanelFocus::Sort(section) => self.toggle_sort_menu(section, cx),
            PanelFocus::RemoveGroup(group) => {
                cx.emit(ColorLibraryPanelEvent::RemoveGroup { group })
            }
            PanelFocus::Swatch(color) => cx.emit(ColorLibraryPanelEvent::EditSwatch { color }),
            PanelFocus::Order(color) => {
                if let Some(row) = self.row(color) {
                    self.begin_edit(
                        EditTarget::ColorOrder(color),
                        (row.manual_order + 1).to_string(),
                        window,
                        cx,
                    );
                }
            }
            PanelFocus::Name(color) => {
                if let Some(row) = self.row(color) {
                    self.begin_edit(
                        EditTarget::ColorName(color),
                        row.name
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                        window,
                        cx,
                    );
                }
            }
            PanelFocus::Value(color) => {
                if let Some(row) = self.row(color) {
                    self.begin_edit(
                        EditTarget::ColorValue(color),
                        row.value.to_string(),
                        window,
                        cx,
                    );
                }
            }
            PanelFocus::Duplicate(color) => {
                cx.emit(ColorLibraryPanelEvent::DuplicateColor { color })
            }
            PanelFocus::Remove(color) => cx.emit(ColorLibraryPanelEvent::RemoveColor { color }),
        }
    }

    fn row(&self, id: ColorLibraryColorId) -> Option<&ColorLibraryRow> {
        self.snapshot
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.id == id)
    }

    fn begin_edit(
        &mut self,
        target: EditTarget,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|input_cx| {
            PanelTextInput::new(
                "color-library-inline-input",
                value,
                "",
                true,
                window,
                input_cx,
            )
        });
        cx.subscribe_in(
            &input,
            window,
            |panel, input, event: &PanelTextInputEvent, window, cx| match event {
                PanelTextInputEvent::Commit(value) | PanelTextInputEvent::Blur(value) => {
                    panel.commit_edit(input.entity_id(), value, window, cx);
                }
                PanelTextInputEvent::Cancel => {
                    panel.cancel_edit(input.entity_id(), window, cx);
                }
                PanelTextInputEvent::Changed(_) => {}
            },
        )
        .detach();
        self.editing = Some((target, input.clone()));
        cx.defer_in(window, move |_, window, cx| {
            input.read(cx).focus(window);
        });
        cx.notify();
    }

    fn commit_edit(
        &mut self,
        input_id: EntityId,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, input)) = self
            .editing
            .as_ref()
            .filter(|(_, input)| input.entity_id() == input_id)
            .cloned()
        else {
            return;
        };

        let event = match target {
            EditTarget::GroupName(group) => {
                let name = value.trim();
                if name.is_empty() {
                    input.update(cx, |input, cx| input.set_invalid(true, cx));
                    return;
                }
                ColorLibraryPanelEvent::RenameGroup {
                    group,
                    name: name.to_owned(),
                }
            }
            EditTarget::ColorName(color) => ColorLibraryPanelEvent::RenameColor {
                color,
                name: non_empty_trimmed(value),
            },
            EditTarget::ColorValue(color) => {
                let value = value.trim();
                if !is_valid_color_value(value) {
                    input.update(cx, |input, cx| input.set_invalid(true, cx));
                    return;
                }
                ColorLibraryPanelEvent::SetColorValue {
                    color,
                    value: value.to_owned(),
                }
            }
            EditTarget::ColorOrder(color) => {
                let Ok(one_based_order) = value.trim().parse::<usize>() else {
                    input.update(cx, |input, cx| input.set_invalid(true, cx));
                    return;
                };
                if one_based_order == 0 {
                    input.update(cx, |input, cx| input.set_invalid(true, cx));
                    return;
                }
                ColorLibraryPanelEvent::SetColorOrder {
                    color,
                    one_based_order,
                }
            }
        };

        self.editing = None;
        self.focus_handle.focus(window);
        cx.emit(event);
        cx.notify();
    }

    fn cancel_edit(&mut self, input_id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editing
            .as_ref()
            .is_some_and(|(_, input)| input.entity_id() == input_id)
        {
            self.editing = None;
            self.focus_handle.focus(window);
            cx.notify();
        }
    }

    fn toggle_collapsed(&mut self, section: ColorLibrarySectionId, cx: &mut Context<Self>) {
        if !self.collapsed_sections.remove(&section) {
            self.collapsed_sections.insert(section);
        }
        self.normalize_keyboard_focus();
        self.emit_presentation(cx);
    }

    fn toggle_sort_menu(&mut self, section: ColorLibrarySectionId, cx: &mut Context<Self>) {
        self.sort_menu = (self.sort_menu != Some(section)).then_some(section);
        cx.notify();
    }

    fn choose_sort(
        &mut self,
        section: ColorLibrarySectionId,
        sort: ColorLibrarySort,
        cx: &mut Context<Self>,
    ) {
        self.sort_menu = None;
        cx.emit(ColorLibraryPanelEvent::SetSort { section, sort });
        cx.notify();
    }

    fn set_geometry(
        &mut self,
        geometry: ColorLibraryPanelGeometry,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let geometry = geometry.clamped(window.viewport_size());
        if geometry != self.geometry {
            self.geometry = geometry;
            self.emit_presentation(cx);
        }
    }

    fn emit_presentation(&self, cx: &mut Context<Self>) {
        cx.emit(ColorLibraryPanelEvent::PresentationChanged(
            self.presentation(),
        ));
        cx.notify();
    }

    fn move_panel(
        &mut self,
        event: &DragMoveEvent<PanelMoveDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let grab_offset = event.drag(cx).grab_offset.get();
        self.set_geometry(
            ColorLibraryPanelGeometry {
                left: event.event.position.x.0 - grab_offset.x.0,
                top: event.event.position.y.0 - grab_offset.y.0,
                ..self.geometry
            },
            window,
            cx,
        );
        cx.stop_propagation();
    }

    fn resize_panel(
        &mut self,
        event: &DragMoveEvent<PanelResizeDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_geometry(
            ColorLibraryPanelGeometry {
                width: event.event.position.x.0 - self.geometry.left,
                height: event.event.position.y.0 - self.geometry.top,
                ..self.geometry
            },
            window,
            cx,
        );
        cx.stop_propagation();
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let drag = PanelMoveDrag::default();
        let drag_for_constructor = drag.clone();
        div()
            .id("color-library-title-bar")
            .h(px(38.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .border_b_1()
            .border_color(rgb(BORDER))
            .cursor_move()
            .on_drag(drag, move |_, offset, _, cx| {
                drag_for_constructor.grab_offset.set(offset);
                cx.new(|_| EmptyDragPreview)
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT))
                    .child("Color Library"),
            )
            .child(self.header_button(
                "color-library-quick-add",
                "Save color",
                PanelFocus::QuickAdd,
                ColorLibraryPanelEvent::QuickAdd,
                cx,
            ))
            .child(self.header_button(
                "color-library-add-group",
                "Add group",
                PanelFocus::AddGroup,
                ColorLibraryPanelEvent::AddGroup,
                cx,
            ))
            .child(self.header_icon_button(
                "color-library-close",
                Icon::Close,
                PanelFocus::Close,
                ColorLibraryPanelEvent::Dismiss,
                cx,
            ))
    }

    fn header_button(
        &self,
        id: &'static str,
        label: &'static str,
        focus: PanelFocus,
        event: ColorLibraryPanelEvent,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        small_button(id, label, self.keyboard_focus == focus).on_click(cx.listener(
            move |panel, _, window, cx| {
                panel.keyboard_focus = focus;
                panel.focus_handle.focus(window);
                cx.emit(event.clone());
                cx.stop_propagation();
                cx.notify();
            },
        ))
    }

    fn header_icon_button(
        &self,
        id: &'static str,
        icon_kind: Icon,
        focus: PanelFocus,
        event: ColorLibraryPanelEvent,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        small_icon_button(id, icon_kind, self.keyboard_focus == focus).on_click(cx.listener(
            move |panel, _, window, cx| {
                panel.keyboard_focus = focus;
                panel.focus_handle.focus(window);
                cx.emit(event.clone());
                cx.stop_propagation();
                cx.notify();
            },
        ))
    }

    fn render_search(&self) -> impl IntoElement {
        div()
            .h(px(34.0))
            .flex_none()
            .px(px(10.0))
            .py(px(5.0))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(self.search_input.clone())
    }

    fn render_section(
        &self,
        section: ColorLibrarySection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let collapsed = self.collapsed_sections.contains(&section.id);
        let section_id = section.id;
        let section_key = section_id.key();
        let mut container = div()
            .id(SharedString::from(format!(
                "color-library-section-{section_key}"
            )))
            .relative()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(rgb(BORDER));

        if let Some(target_group) = section_id.group() {
            container = container
                .drag_over::<ColorGroupDrag>(move |style, dragged, _, _| {
                    if dragged.group == target_group {
                        style
                    } else {
                        style.border_1().border_color(rgb(ACCENT))
                    }
                })
                .on_drop(cx.listener(move |_, dragged: &ColorGroupDrag, _, cx| {
                    if dragged.group != target_group {
                        cx.emit(ColorLibraryPanelEvent::MoveGroupBefore {
                            group: dragged.group,
                            before: target_group,
                        });
                    }
                    cx.stop_propagation();
                }));
        }

        let header = self.render_section_header(&section, collapsed, cx);
        container = container.child(header);
        if collapsed {
            return container;
        }

        if section.rows.is_empty() {
            return container.child(
                div()
                    .h(px(44.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(10.0))
                    .text_color(rgb(MUTED))
                    .child("No saved colors"),
            );
        }

        container
            .child(table_header())
            .children(
                section
                    .rows
                    .into_iter()
                    .map(|row| self.render_row(section_id, row, cx).into_any_element()),
            )
            .on_drop(cx.listener(move |_, dragged: &ColorRowDrag, _, cx| {
                cx.emit(ColorLibraryPanelEvent::MoveColorBefore {
                    color: dragged.color,
                    section: section_id,
                    before: None,
                });
                cx.stop_propagation();
            }))
    }

    fn render_section_header(
        &self,
        section: &ColorLibrarySection,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let section_id = section.id;
        let group_id = section_id.group();
        let sort = section.sort;
        let editing_name =
            group_id.and_then(|group| self.inline_input(EditTarget::GroupName(group)));
        let group_name = section.name.clone();
        let group_drag = group_id.map(|group| ColorGroupDrag { group });
        let group_name_element: AnyElement = if let Some(input) = editing_name {
            div().flex_1().child(input).into_any_element()
        } else if let Some(group) = group_id {
            div()
                .id(SharedString::from(format!("group-name-{}", group.0)))
                .flex_1()
                .min_w(px(80.0))
                .overflow_hidden()
                .cursor_text()
                .text_size(px(11.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .when(
                    self.keyboard_focus == PanelFocus::GroupName(group),
                    |name| name.border_b_1().border_color(rgb(ACCENT)),
                )
                .child(group_name)
                .on_click(cx.listener(move |panel, _, window, cx| {
                    panel.keyboard_focus = PanelFocus::GroupName(group);
                    panel.activate_keyboard_focus(window, cx);
                    cx.stop_propagation();
                }))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w(px(80.0))
                .overflow_hidden()
                .text_size(px(11.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child(group_name)
                .into_any_element()
        };

        div()
            .relative()
            .min_h(px(34.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .px(px(8.0))
            .bg(rgb(SURFACE_RAISED))
            .child(
                small_icon_button(
                    SharedString::from(format!("collapse-{}", section_id.key())),
                    if collapsed {
                        Icon::ChevronRight
                    } else {
                        Icon::ChevronDown
                    },
                    self.keyboard_focus == PanelFocus::Collapse(section_id),
                )
                .on_click(cx.listener(move |panel, _, window, cx| {
                    panel.keyboard_focus = PanelFocus::Collapse(section_id);
                    panel.focus_handle.focus(window);
                    panel.toggle_collapsed(section_id, cx);
                    cx.stop_propagation();
                })),
            )
            .when_some(group_drag, |header, drag| {
                header.child(
                    div()
                        .id(SharedString::from(format!("group-drag-{}", drag.group.0)))
                        .w(px(18.0))
                        .h(px(24.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_move()
                        .child(icon(Icon::GripVertical, 14.0, rgb(MUTED)))
                        .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragPreview)),
                )
            })
            .child(group_name_element)
            .child(
                sort_button(
                    SharedString::from(format!("sort-{}", section_id.key())),
                    sort.mode.label(),
                    sort.direction.icon(),
                    self.keyboard_focus == PanelFocus::Sort(section_id),
                )
                .on_click(cx.listener(move |panel, _, window, cx| {
                    panel.keyboard_focus = PanelFocus::Sort(section_id);
                    panel.focus_handle.focus(window);
                    panel.toggle_sort_menu(section_id, cx);
                    cx.stop_propagation();
                })),
            )
            .when_some(group_id, |header, group| {
                header.child(
                    small_icon_button(
                        SharedString::from(format!("remove-group-{}", group.0)),
                        Icon::Close,
                        self.keyboard_focus == PanelFocus::RemoveGroup(group),
                    )
                    .on_click(cx.listener(move |panel, _, window, cx| {
                        panel.keyboard_focus = PanelFocus::RemoveGroup(group);
                        panel.focus_handle.focus(window);
                        cx.emit(ColorLibraryPanelEvent::RemoveGroup { group });
                        cx.stop_propagation();
                        cx.notify();
                    })),
                )
            })
            .when(self.sort_menu == Some(section_id), |header| {
                header.child(self.render_sort_menu(section_id, sort, cx))
            })
    }

    fn render_sort_menu(
        &self,
        section: ColorLibrarySectionId,
        current: ColorLibrarySort,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("sort-menu-{}", section.key())))
            .absolute()
            .top(px(31.0))
            .right(px(8.0))
            .w(px(168.0))
            .p(px(5.0))
            .flex()
            .flex_col()
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .shadow_lg()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(ColorLibrarySortMode::ALL.into_iter().map(|mode| {
                let sort = ColorLibrarySort {
                    mode,
                    direction: current.direction,
                };
                menu_item(mode.label(), mode == current.mode).on_click(cx.listener(
                    move |panel, _, _, cx| {
                        panel.choose_sort(section, sort, cx);
                        cx.stop_propagation();
                    },
                ))
            }))
            .child(div().h(px(1.0)).my(px(4.0)).bg(rgb(BORDER)))
            .child(menu_item("Reverse order", false).on_click(cx.listener(
                move |panel, _, _, cx| {
                    panel.choose_sort(
                        section,
                        ColorLibrarySort {
                            direction: current.direction.reversed(),
                            ..current
                        },
                        cx,
                    );
                    cx.stop_propagation();
                },
            )))
    }

    fn render_row(
        &self,
        section: ColorLibrarySectionId,
        row: ColorLibraryRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let color = row.id;
        let order_input = self.inline_input(EditTarget::ColorOrder(color));
        let name_input = self.inline_input(EditTarget::ColorName(color));
        let value_input = self.inline_input(EditTarget::ColorValue(color));
        let name = row.fallback_name();
        let value = row.value.clone();
        let one_based_order = row.manual_order + 1;

        div()
            .id(SharedString::from(format!("saved-color-{}", color.0)))
            .h(px(34.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .px(px(7.0))
            .border_t_1()
            .border_color(rgb(0x34363b))
            .hover(|style| style.bg(rgb(0x25262a)))
            .drag_over::<ColorRowDrag>(move |style, dragged, _, _| {
                if dragged.color == color {
                    style
                } else {
                    style.border_t_2().border_color(rgb(ACCENT))
                }
            })
            .on_drop(cx.listener(move |_, dragged: &ColorRowDrag, _, cx| {
                if dragged.color != color {
                    cx.emit(ColorLibraryPanelEvent::MoveColorBefore {
                        color: dragged.color,
                        section,
                        before: Some(color),
                    });
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .id(SharedString::from(format!("saved-color-drag-{}", color.0)))
                    .w(px(18.0))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_move()
                    .child(icon(Icon::GripVertical, 14.0, rgb(MUTED)))
                    .on_drag(ColorRowDrag { color }, |_, _, _, cx| {
                        cx.new(|_| EmptyDragPreview)
                    }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("saved-color-order-{}", color.0)))
                    .w(px(40.0))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.0))
                    .text_size(px(10.0))
                    .text_color(rgb(MUTED))
                    .when(self.keyboard_focus == PanelFocus::Order(color), |cell| {
                        cell.border_1().border_color(rgb(ACCENT))
                    })
                    .when_some(order_input.clone(), |cell, input| cell.child(input))
                    .when(order_input.is_none(), |cell| {
                        cell.child(one_based_order.to_string())
                            .cursor_text()
                            .on_click(cx.listener(move |panel, _, window, cx| {
                                panel.keyboard_focus = PanelFocus::Order(color);
                                panel.activate_keyboard_focus(window, cx);
                                cx.stop_propagation();
                            }))
                    }),
            )
            .child(
                div()
                    .id(SharedString::from(format!(
                        "saved-color-swatch-{}",
                        color.0
                    )))
                    .w(px(30.0))
                    .h(px(24.0))
                    .p(px(2.0))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(rgb(if self.keyboard_focus == PanelFocus::Swatch(color) {
                        ACCENT
                    } else {
                        BORDER
                    }))
                    .cursor_pointer()
                    .hover(|style| style.border_color(rgb(ACCENT)))
                    .child(
                        div()
                            .size_full()
                            .rounded(px(2.0))
                            .bg(rgba(pack_rgba(row.rgba))),
                    )
                    .on_click(cx.listener(move |panel, _, window, cx| {
                        panel.keyboard_focus = PanelFocus::Swatch(color);
                        panel.focus_handle.focus(window);
                        cx.emit(ColorLibraryPanelEvent::EditSwatch { color });
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .child(
                editable_cell(
                    SharedString::from(format!("saved-color-name-{}", color.0)),
                    1.0,
                    name,
                    name_input,
                    self.keyboard_focus == PanelFocus::Name(color),
                )
                .on_click(cx.listener(move |panel, _, window, cx| {
                    panel.keyboard_focus = PanelFocus::Name(color);
                    panel.activate_keyboard_focus(window, cx);
                    cx.stop_propagation();
                })),
            )
            .child(
                editable_cell(
                    SharedString::from(format!("saved-color-value-{}", color.0)),
                    0.72,
                    value,
                    value_input,
                    self.keyboard_focus == PanelFocus::Value(color),
                )
                .font_family("SFMono-Regular")
                .on_click(cx.listener(move |panel, _, window, cx| {
                    panel.keyboard_focus = PanelFocus::Value(color);
                    panel.activate_keyboard_focus(window, cx);
                    cx.stop_propagation();
                })),
            )
            .child(
                small_icon_button(
                    SharedString::from(format!("saved-color-duplicate-{}", color.0)),
                    Icon::Copy,
                    self.keyboard_focus == PanelFocus::Duplicate(color),
                )
                .on_click(cx.listener(move |panel, _, window, cx| {
                    panel.keyboard_focus = PanelFocus::Duplicate(color);
                    panel.focus_handle.focus(window);
                    cx.emit(ColorLibraryPanelEvent::DuplicateColor { color });
                    cx.stop_propagation();
                    cx.notify();
                })),
            )
            .child(
                small_icon_button(
                    SharedString::from(format!("saved-color-remove-{}", color.0)),
                    Icon::Close,
                    self.keyboard_focus == PanelFocus::Remove(color),
                )
                .on_click(cx.listener(move |panel, _, window, cx| {
                    panel.keyboard_focus = PanelFocus::Remove(color);
                    panel.focus_handle.focus(window);
                    cx.emit(ColorLibraryPanelEvent::RemoveColor { color });
                    cx.stop_propagation();
                    cx.notify();
                })),
            )
    }

    fn inline_input(&self, target: EditTarget) -> Option<Entity<PanelTextInput>> {
        self.editing
            .as_ref()
            .filter(|(active, _)| *active == target)
            .map(|(_, input)| input.clone())
    }
}

impl EventEmitter<ColorLibraryPanelEvent> for ColorLibraryPanel {}

impl Focusable for ColorLibraryPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ColorLibraryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clamped = self.geometry.clamped(window.viewport_size());
        if clamped != self.geometry {
            self.geometry = clamped;
        }
        let sections = self.visible_sections();
        let empty = sections.is_empty();

        div()
            .id("color-library-panel")
            .absolute()
            .left(px(self.geometry.left))
            .top(px(self.geometry.top))
            .w(px(self.geometry.width))
            .h(px(self.geometry.height))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .shadow_lg()
            .occlude()
            .key_context("ColorLibraryPanel")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_drag_move::<PanelMoveDrag>(cx.listener(Self::move_panel))
            .on_drag_move::<PanelResizeDrag>(cx.listener(Self::resize_panel))
            .child(self.render_header(cx))
            .child(self.render_search())
            .child(
                div()
                    .id("color-library-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .when(empty, |list| {
                        list.child(
                            div()
                                .h_full()
                                .min_h(px(120.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(11.0))
                                .text_color(rgb(MUTED))
                                .child(if self.search_query.trim().is_empty() {
                                    "No saved colors yet"
                                } else {
                                    "No matching saved colors"
                                }),
                        )
                    })
                    .children(
                        sections
                            .into_iter()
                            .map(|section| self.render_section(section, cx).into_any_element()),
                    ),
            )
            .child(
                div()
                    .id("color-library-resize-handle")
                    .absolute()
                    .right_0()
                    .bottom_0()
                    .w(px(14.0))
                    .h(px(14.0))
                    .cursor_nwse_resize()
                    .occlude()
                    .child(
                        div()
                            .absolute()
                            .right(px(3.0))
                            .bottom(px(3.0))
                            .w(px(7.0))
                            .h(px(7.0))
                            .border_r_1()
                            .border_b_1()
                            .border_color(rgb(MUTED)),
                    )
                    .on_drag(PanelResizeDrag, |_, _, _, cx| cx.new(|_| EmptyDragPreview))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
            )
    }
}

#[derive(Clone, Default)]
struct PanelMoveDrag {
    grab_offset: Rc<Cell<Point<Pixels>>>,
}

#[derive(Clone, Copy)]
struct PanelResizeDrag;

#[derive(Clone, Copy)]
struct ColorRowDrag {
    color: ColorLibraryColorId,
}

#[derive(Clone, Copy)]
struct ColorGroupDrag {
    group: ColorLibraryGroupId,
}

struct EmptyDragPreview;

impl Render for EmptyDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

fn table_header() -> impl IntoElement {
    div()
        .h(px(24.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .px(px(7.0))
        .bg(rgb(0x25262a))
        .text_size(px(9.0))
        .text_color(rgb(MUTED))
        .child(div().w(px(18.0)))
        .child(
            div()
                .w(px(40.0))
                .text_align(gpui::TextAlign::Center)
                .child("Order"),
        )
        .child(div().w(px(30.0)).child("Color"))
        .child(div().flex_1().child("Name"))
        .child(div().flex_basis(relative(0.72)).child("Value"))
        .child(
            div()
                .w(px(53.0))
                .text_align(gpui::TextAlign::Center)
                .child("Actions"),
        )
}

fn editable_cell(
    id: SharedString,
    flex_basis: f32,
    display: SharedString,
    input: Option<Entity<PanelTextInput>>,
    focused: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(24.0))
        .flex_basis(relative(flex_basis))
        .min_w(px(72.0))
        .px(px(4.0))
        .flex()
        .items_center()
        .overflow_hidden()
        .rounded(px(3.0))
        .border_1()
        .border_color(rgb(if focused { ACCENT } else { 0x000000 }))
        .text_size(px(10.0))
        .text_color(rgb(TEXT))
        .cursor_text()
        .when_some(input.clone(), |cell, input| cell.p_0().child(input))
        .when(input.is_none(), |cell| cell.child(display))
}

fn small_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    focused: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(24.0))
        .min_w(px(24.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(rgb(if focused { ACCENT } else { BORDER }))
        .bg(rgb(SURFACE_RAISED))
        .text_size(px(10.0))
        .text_color(rgb(TEXT))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(label.into())
}

fn small_icon_button(
    id: impl Into<ElementId>,
    icon_kind: Icon,
    focused: bool,
) -> gpui::Stateful<gpui::Div> {
    small_button(id, "", focused).child(icon(icon_kind, 13.0, rgb(TEXT)))
}

fn sort_button(
    id: impl Into<ElementId>,
    label: &'static str,
    direction: Icon,
    focused: bool,
) -> gpui::Stateful<gpui::Div> {
    small_button(id, label, focused)
        .gap(px(4.0))
        .child(icon(direction, 12.0, rgb(MUTED)))
}

fn menu_item(label: &'static str, active: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(format!(
            "color-library-sort-{}",
            label.replace([' ', '&'], "-").to_ascii_lowercase()
        )))
        .h(px(26.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .rounded(px(4.0))
        .bg(if active {
            rgba(0x0c8ce92e)
        } else {
            rgba(0x00000000)
        })
        .text_size(px(10.0))
        .text_color(rgb(if active { TEXT } else { MUTED }))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(label)
}

fn filtered_section(section: &ColorLibrarySection, query: &str) -> Option<ColorLibrarySection> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(section.clone());
    }
    if section.name.to_lowercase().contains(&query) {
        return Some(section.clone());
    }

    let rows = section
        .rows
        .iter()
        .filter(|row| {
            row.name
                .as_ref()
                .is_some_and(|name| name.to_lowercase().contains(&query))
                || row.value.to_lowercase().contains(&query)
        })
        .cloned()
        .collect::<Vec<_>>();
    (!rows.is_empty()).then(|| ColorLibrarySection {
        rows,
        ..section.clone()
    })
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn pack_rgba(channels: [f32; 4]) -> u32 {
    channels
        .into_iter()
        .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u32)
        .fold(0, |packed, channel| (packed << 8) | channel)
}

fn is_valid_color_value(value: &str) -> bool {
    is_valid_hex(value) || is_valid_rgb_function(value)
}

fn is_valid_hex(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_valid_rgb_function(value: &str) -> bool {
    let value = value.trim();
    let (body, has_alpha) = if let Some(body) = value
        .strip_prefix("rgba(")
        .and_then(|body| body.strip_suffix(')'))
    {
        (body, true)
    } else if let Some(body) = value
        .strip_prefix("rgb(")
        .and_then(|body| body.strip_suffix(')'))
    {
        (body, false)
    } else {
        return false;
    };

    let components = body.split(',').map(str::trim).collect::<Vec<_>>();
    let expected = if has_alpha { 4 } else { 3 };
    if components.len() != expected {
        return false;
    }
    if !components[..3]
        .iter()
        .all(|component| parse_rgb_component(component))
    {
        return false;
    }
    !has_alpha || parse_alpha_component(components[3])
}

fn parse_rgb_component(value: &str) -> bool {
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .parse::<f32>()
            .is_ok_and(|value| value.is_finite() && (0.0..=100.0).contains(&value));
    }
    value
        .parse::<u16>()
        .is_ok_and(|value| value <= u16::from(u8::MAX))
}

fn parse_alpha_component(value: &str) -> bool {
    if let Some(percent) = value.strip_suffix('%') {
        return percent
            .parse::<f32>()
            .is_ok_and(|value| value.is_finite() && (0.0..=100.0).contains(&value));
    }
    value
        .parse::<f32>()
        .is_ok_and(|value| value.is_finite() && (0.0..=1.0).contains(&value))
}

actions!(
    color_library_input,
    [
        InputBackspace,
        InputDelete,
        InputLeft,
        InputRight,
        InputSelectLeft,
        InputSelectRight,
        InputSelectAll,
        InputHome,
        InputEnd,
        InputCopy,
        InputCut,
        InputPaste,
        InputCommit,
        InputCancel,
    ]
);

#[derive(Clone, Debug)]
enum PanelTextInputEvent {
    Changed(String),
    Commit(String),
    Cancel,
    Blur(String),
}

struct PanelTextInput {
    element_id: SharedString,
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    transient: bool,
    finished: bool,
    invalid: bool,
}

impl PanelTextInput {
    fn new(
        element_id: impl Into<SharedString>,
        content: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        transient: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        cx.on_focus_out(&focus_handle, window, |input, _, _, cx| {
            if !input.finished {
                cx.emit(PanelTextInputEvent::Blur(input.content.to_string()));
            }
        })
        .detach();
        let content = content.into();
        let end = content.len();
        Self {
            element_id: element_id.into(),
            focus_handle,
            content,
            placeholder: placeholder.into(),
            selected_range: 0..end,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            transient,
            finished: false,
            invalid: false,
        }
    }

    fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.content = "".into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.invalid = false;
        cx.emit(PanelTextInputEvent::Changed(String::new()));
        cx.notify();
    }

    fn set_invalid(&mut self, invalid: bool, cx: &mut Context<Self>) {
        self.invalid = invalid;
        cx.notify();
    }

    fn finish(&mut self, commit: bool, cx: &mut Context<Self>) {
        if self.finished {
            return;
        }
        if self.transient {
            self.finished = true;
        }
        if commit {
            cx.emit(PanelTextInputEvent::Commit(self.content.to_string()));
        } else {
            cx.emit(PanelTextInputEvent::Cancel);
        }
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.marked_range = None;
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) else {
            return self.content.len();
        };
        line.closest_index_for_x(position.x - bounds.left())
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        utf8_offset_from_utf16(&self.content, offset)
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for character in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += character.len_utf8();
            utf16_offset += character.len_utf16();
        }
        utf16_offset
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn left(&mut self, _: &InputLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &InputRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &InputSelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &InputSelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &InputSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &InputHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &InputEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &InputBackspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &InputDelete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn copy(&mut self, _: &InputCopy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected) = self.content.get(self.selected_range.clone()) {
            if !selected.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
            }
        }
    }

    fn cut(&mut self, _: &InputCut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&InputCopy, window, cx);
        self.replace_text_in_range(None, "", window, cx);
    }

    fn paste(&mut self, _: &InputPaste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace(['\r', '\n'], " "), window, cx);
        }
    }

    fn commit(&mut self, _: &InputCommit, _: &mut Window, cx: &mut Context<Self>) {
        self.finish(true, cx);
    }

    fn cancel(&mut self, _: &InputCancel, _: &mut Window, cx: &mut Context<Self>) {
        self.finish(false, cx);
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
        cx.stop_propagation();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }
}

impl EventEmitter<PanelTextInputEvent> for PanelTextInput {}

impl EntityInputHandler for PanelTextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        self.content.get(range).map(ToOwned::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let replacement = text.replace(['\r', '\n'], " ");
        let mut content = self.content.to_string();
        content.replace_range(range.clone(), &replacement);
        let cursor = range.start + replacement.len();
        self.content = content.into();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.invalid = false;
        cx.emit(PanelTextInputEvent::Changed(self.content.to_string()));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let replacement = text.replace(['\r', '\n'], " ");
        let mut content = self.content.to_string();
        content.replace_range(range.clone(), &replacement);
        let marked = range.start..range.start + replacement.len();
        self.content = content.into();
        self.marked_range = Some(marked.clone());
        self.selected_range = selected_utf16
            .map(|selection| {
                range.start + utf8_offset_from_utf16(&replacement, selection.start)
                    ..range.start + utf8_offset_from_utf16(&replacement, selection.end)
            })
            .unwrap_or(marked.end..marked.end);
        self.selection_reversed = false;
        self.invalid = false;
        cx.emit(PanelTextInputEvent::Changed(self.content.to_string()));
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let line = self.last_layout.as_ref()?;
        Some(self.offset_to_utf16(line.closest_index_for_x(position.x - bounds.left())))
    }
}

struct PanelTextElement {
    input: Entity<PanelTextInput>,
}

struct PanelTextPrepaint {
    line: ShapedLine,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for PanelTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PanelTextElement {
    type RequestLayoutState = ();
    type PrepaintState = PanelTextPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = if input.content.is_empty() {
            input.placeholder.clone()
        } else {
            input.content.clone()
        };
        let style = window.text_style();
        let color = if input.content.is_empty() {
            rgb(MUTED).into()
        } else {
            style.color
        };
        let run = TextRun {
            len: content.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = &input.marked_range {
            vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(style.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: content.len().saturating_sub(marked.end),
                    ..run.clone()
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>()
        } else {
            vec![run]
        };
        let line = window
            .text_system()
            .shape_line(content, style.font_size.to_pixels(window.rem_size()), &runs)
            .expect("Color Library text shaping should succeed");
        let cursor = input.cursor_offset();
        let selection = input.selected_range.clone();
        let selection_quad = (!selection.is_empty() && !input.content.is_empty()).then(|| {
            fill(
                Bounds::from_corners(
                    point(
                        bounds.left() + line.x_for_index(selection.start),
                        bounds.top(),
                    ),
                    point(
                        bounds.left() + line.x_for_index(selection.end),
                        bounds.bottom(),
                    ),
                ),
                rgba(0x0c8ce966),
            )
        });
        let cursor_quad = (selection.is_empty() && !input.content.is_empty()).then(|| {
            fill(
                Bounds::new(
                    point(
                        bounds.left() + line.x_for_index(cursor),
                        bounds.top() + px(2.0),
                    ),
                    size(px(1.0), (bounds.bottom() - bounds.top()) - px(4.0)),
                ),
                rgb(0xffffff),
            )
        });
        PanelTextPrepaint {
            line,
            cursor: cursor_quad,
            selection: selection_quad,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = state.selection.take() {
            window.paint_quad(selection);
        }
        state
            .line
            .paint(bounds.origin, window.line_height(), window, cx)
            .expect("Color Library text paint should succeed");
        if focus_handle.is_focused(window) {
            if let Some(cursor) = state.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        self.input.update(cx, |input, _| {
            input.last_layout = Some(state.line.clone());
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for PanelTextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(self.element_id.clone())
            .w_full()
            .h(px(24.0))
            .px(px(5.0))
            .flex()
            .items_center()
            .overflow_hidden()
            .key_context("ColorLibraryInput")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .bg(rgb(SURFACE_INPUT))
            .border_1()
            .border_color(rgb(if self.invalid { DANGER } else { ACCENT }))
            .rounded(px(3.0))
            .text_size(px(10.0))
            .text_color(rgb(TEXT))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::commit))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(PanelTextElement {
                input: cx.entity().clone(),
            })
    }
}

impl Focusable for PanelTextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Register key bindings for search and inline Color Library text editors.
/// The app should call this next to `layer_name_input::register_keybindings`.
pub(crate) fn register_keybindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", InputBackspace, Some("ColorLibraryInput")),
        KeyBinding::new("delete", InputDelete, Some("ColorLibraryInput")),
        KeyBinding::new("left", InputLeft, Some("ColorLibraryInput")),
        KeyBinding::new("right", InputRight, Some("ColorLibraryInput")),
        KeyBinding::new("shift-left", InputSelectLeft, Some("ColorLibraryInput")),
        KeyBinding::new("shift-right", InputSelectRight, Some("ColorLibraryInput")),
        KeyBinding::new("home", InputHome, Some("ColorLibraryInput")),
        KeyBinding::new("end", InputEnd, Some("ColorLibraryInput")),
        KeyBinding::new("secondary-a", InputSelectAll, Some("ColorLibraryInput")),
        KeyBinding::new("secondary-c", InputCopy, Some("ColorLibraryInput")),
        KeyBinding::new("secondary-x", InputCut, Some("ColorLibraryInput")),
        KeyBinding::new("secondary-v", InputPaste, Some("ColorLibraryInput")),
        KeyBinding::new("enter", InputCommit, Some("ColorLibraryInput")),
        KeyBinding::new("escape", InputCancel, Some("ColorLibraryInput")),
    ]);
}

fn utf8_offset_from_utf16(text: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for character in text.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += character.len_utf16();
        utf8_offset += character.len_utf8();
    }
    utf8_offset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, name: Option<&str>, value: &str) -> ColorLibraryRow {
        ColorLibraryRow {
            id: ColorLibraryColorId(id),
            name: name.map(|name| SharedString::from(name.to_owned())),
            rgba: [0.0, 0.0, 0.0, 1.0],
            value: value.to_owned().into(),
            manual_order: id as usize,
        }
    }

    fn section() -> ColorLibrarySection {
        ColorLibrarySection {
            id: ColorLibrarySectionId::Group(ColorLibraryGroupId(7)),
            name: "Brand".into(),
            sort: ColorLibrarySort::default(),
            rows: vec![
                row(1, Some("Ocean blue"), "#0C8CE9"),
                row(2, None, "#FFFFFF"),
            ],
        }
    }

    #[test]
    fn filtering_matches_group_name_color_name_and_value() {
        assert_eq!(filtered_section(&section(), "brand").unwrap().rows.len(), 2);
        assert_eq!(filtered_section(&section(), "ocean").unwrap().rows.len(), 1);
        assert_eq!(
            filtered_section(&section(), "ffffff").unwrap().rows.len(),
            1
        );
        assert!(filtered_section(&section(), "missing").is_none());
    }

    #[test]
    fn color_value_validation_accepts_scoped_formats() {
        for value in [
            "#fff",
            "#ffff",
            "#0C8CE9",
            "#0C8CE980",
            "rgb(12, 140, 233)",
            "rgb(10%, 20%, 30%)",
            "rgba(12, 140, 233, 0.5)",
            "rgba(12, 140, 233, 50%)",
        ] {
            assert!(is_valid_color_value(value), "expected {value} to be valid");
        }
    }

    #[test]
    fn color_value_validation_rejects_out_of_range_or_partial_values() {
        for value in [
            "fff",
            "#12",
            "#gggggg",
            "rgb(256, 0, 0)",
            "rgb(0, 0)",
            "rgba(0, 0, 0, 1.1)",
            "rgba(0, 0, 0)",
        ] {
            assert!(
                !is_valid_color_value(value),
                "expected {value} to be invalid"
            );
        }
    }

    #[test]
    fn restored_geometry_is_kept_inside_small_viewports() {
        let geometry = ColorLibraryPanelGeometry {
            left: 900.0,
            top: 800.0,
            width: 900.0,
            height: 900.0,
        }
        .clamped(gpui::size(px(320.0), px(180.0)));

        assert!(geometry.left >= WINDOW_MARGIN);
        assert!(geometry.top >= WINDOW_MARGIN);
        assert!(geometry.left + geometry.width <= 320.0 - WINDOW_MARGIN + f32::EPSILON);
        assert!(geometry.top + geometry.height <= 180.0 - WINDOW_MARGIN + f32::EPSILON);
    }

    #[test]
    fn rgba_packing_uses_gpui_channel_order() {
        assert_eq!(pack_rgba([1.0, 0.5, 0.0, 0.25]), 0xff800040);
    }

    #[test]
    fn utf16_conversion_handles_multibyte_text() {
        let text = "A🙂é";
        assert_eq!(utf8_offset_from_utf16(text, 0), 0);
        assert_eq!(utf8_offset_from_utf16(text, 1), 1);
        assert_eq!(utf8_offset_from_utf16(text, 3), 5);
        assert_eq!(utf8_offset_from_utf16(text, 4), 7);
    }
}
