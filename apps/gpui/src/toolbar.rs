//! Editor chrome: document bar, floating tool dock, and command menus.

use std::path::{Path, PathBuf};

use editor_core::{Editor, EditorAction, InteractionKind, TextAlign, Tool};
use gpui::{
    anchored, div, point, prelude::*, px, rgb, rgba, Action, AnyElement, AnyView, App, Context,
    Corner, IntoElement, MouseButton, Render, SharedString, Window,
};

use crate::{
    assets::{icon, Icon},
    commands::{AppCommand, CommandTarget, Keymap},
    AlignObjectsBottom, AlignObjectsCenter, AlignObjectsLeft, AlignObjectsMiddle,
    AlignObjectsRight, AlignObjectsTop, AlignTextCenter, AlignTextLeft, AlignTextRight,
    BringForward, BringToFront, Copy, Cut, Delete, DeselectAll, DistributeObjectsHorizontal,
    DistributeObjectsVertical, Duplicate, EditVector, EllipseTool, ExportJpeg, ExportPng,
    ExportSvg, ExportSvgOutlined, ExportWebP, FinishEditing, FrameTool, Group, InvertSelection,
    JoinPaths, LineTool, NewDocument, OpenDocument, OpenKeyboardShortcuts, Paste, PenTool,
    RectangleTool, Redo, ReversePath, SaveDocument, SaveDocumentAs, SelectAll, SelectTool,
    SendBackward, SendToBack, ShowCommandPalette, SplitPath, StartZoomInput, Strek, TextLarger,
    TextSmaller, TextTool, ToggleDesignPanel, ToggleFrameBackground, ToggleLayerPanel,
    ToggleMainMenu, TogglePathClosed, ToggleZoomMenu, Undo, Ungroup, ZoomIn, ZoomOut, ZoomReset,
    ZoomResetAll, ZoomToFit, ZoomToSelection,
};

pub const HEADER_HEIGHT: f32 = 48.0;

const SURFACE: u32 = 0x202124;
const SURFACE_RAISED: u32 = 0x292a2e;
const SURFACE_HOVER: u32 = 0x35363b;
const BORDER: u32 = 0x414349;
const TEXT: u32 = 0xf1f3f4;
const TEXT_MUTED: u32 = 0xa8abb2;
const ACCENT: u32 = 0x0c8ce9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PortableFontFamily {
    System,
    Serif,
    Monospace,
    Other,
}

impl PortableFontFamily {
    pub(crate) fn from_name(family: &str) -> Self {
        match family.trim().to_ascii_lowercase().as_str() {
            "sans-serif" | "system-ui" => Self::System,
            "serif" => Self::Serif,
            "monospace" => Self::Monospace,
            _ => Self::Other,
        }
    }
}

pub(crate) fn font_family_label(family: &str) -> String {
    match PortableFontFamily::from_name(family) {
        PortableFontFamily::System => "System".to_owned(),
        PortableFontFamily::Serif => "Serif".to_owned(),
        PortableFontFamily::Monospace => "Monospace".to_owned(),
        PortableFontFamily::Other => {
            let family = family.trim();
            if family.is_empty() {
                "Unknown".to_owned()
            } else {
                family.to_owned()
            }
        }
    }
}

fn shortcut(keymap: &Keymap, target: CommandTarget) -> Option<String> {
    keymap.shortcut_label(target)
}

fn shortcut_text(keymap: &Keymap, target: CommandTarget) -> String {
    shortcut(keymap, target).unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Main,
    Zoom,
}

pub struct DocumentHeader {
    pub name: String,
    pub location: String,
    pub dirty: bool,
}

#[derive(Clone, Copy)]
pub struct PanelVisibility {
    pub layers: bool,
    pub design: bool,
}

#[derive(Clone, Copy)]
pub struct HistoryAvailability {
    pub undo: bool,
    pub redo: bool,
}

#[derive(Clone, Copy)]
pub struct ZoomInputSnapshot<'a> {
    pub value: &'a str,
    pub invalid: bool,
}

#[derive(Clone, Copy)]
pub struct ZoomState<'a> {
    pub level: f32,
    pub input: Option<ZoomInputSnapshot<'a>>,
}

pub struct MenuState<'a> {
    pub panels: PanelVisibility,
    pub can_paste: bool,
    pub recent_files: &'a [PathBuf],
    pub file_busy: bool,
    pub keymap: &'a Keymap,
}

struct EditorTooltip {
    label: SharedString,
    shortcut: Option<SharedString>,
}

impl Render for EditorTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(8.0))
            .py(px(5.0))
            .bg(rgb(0x17181a))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded(px(5.0))
            .shadow_md()
            .text_size(px(11.0))
            .text_color(rgb(TEXT))
            .child(self.label.clone())
            .when_some(self.shortcut.clone(), |tooltip, shortcut| {
                tooltip.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgb(TEXT_MUTED))
                        .child(shortcut),
                )
            })
    }
}

pub(crate) fn editor_tooltip(
    label: impl Into<SharedString>,
    shortcut: Option<&'static str>,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    editor_tooltip_owned(label, shortcut.map(SharedString::from))
}

fn editor_tooltip_owned(
    label: impl Into<SharedString>,
    shortcut: Option<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let label = label.into();
    move |_, cx| {
        cx.new(|_| EditorTooltip {
            label: label.clone(),
            shortcut: shortcut.clone(),
        })
        .into()
    }
}

pub fn render_header(
    document: DocumentHeader,
    current_tool: Tool,
    zoom: ZoomState<'_>,
    panels: PanelVisibility,
    history: HistoryAvailability,
    open_menu: Option<MenuKind>,
    keymap: &Keymap,
) -> impl IntoElement {
    div()
        .id("document-bar")
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(8.0))
        .bg(rgb(SURFACE))
        .border_b_1()
        .border_color(rgb(BORDER))
        .child(icon_action_button(
            "main-menu",
            Icon::Menu,
            open_menu == Some(MenuKind::Main),
            true,
            "Main menu",
            None,
            ToggleMainMenu,
        ))
        .child(div().w(px(1.0)).h(px(22.0)).mx(px(2.0)).bg(rgb(BORDER)))
        .child(
            div()
                .min_w(px(176.0))
                .flex()
                .flex_col()
                .justify_center()
                .px(px(4.0))
                .child(
                    div()
                        .text_color(rgb(TEXT))
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(if document.dirty {
                            format!("{} •", document.name)
                        } else {
                            document.name
                        }),
                )
                .child(
                    div()
                        .text_color(rgb(TEXT_MUTED))
                        .text_size(px(10.0))
                        .child(document.location),
                ),
        )
        .child(div().flex_1())
        .child(render_tool_rail(current_tool, keymap))
        .child(div().flex_1())
        .child(icon_action_button(
            "command-palette",
            Icon::Search,
            false,
            true,
            "Command palette",
            shortcut(keymap, CommandTarget::App(AppCommand::ShowCommandPalette)),
            ShowCommandPalette,
        ))
        .child(icon_action_button(
            "undo",
            Icon::Undo,
            false,
            history.undo,
            "Undo",
            shortcut(keymap, CommandTarget::Editor(EditorAction::Undo)),
            Undo,
        ))
        .child(icon_action_button(
            "redo",
            Icon::Redo,
            false,
            history.redo,
            "Redo",
            shortcut(keymap, CommandTarget::Editor(EditorAction::Redo)),
            Redo,
        ))
        .child(div().w(px(1.0)).h(px(22.0)).mx(px(2.0)).bg(rgb(BORDER)))
        .child(zoom_control(
            zoom.level,
            zoom.input,
            open_menu == Some(MenuKind::Zoom),
        ))
        .child(icon_action_button(
            "layers-panel",
            Icon::Layers,
            panels.layers,
            true,
            if panels.layers {
                "Hide Layers panel"
            } else {
                "Show Layers panel"
            },
            shortcut(keymap, CommandTarget::App(AppCommand::ToggleLayerPanel)),
            ToggleLayerPanel,
        ))
        .child(icon_action_button(
            "design-panel",
            Icon::Properties,
            panels.design,
            true,
            if panels.design {
                "Hide Design panel"
            } else {
                "Show Design panel"
            },
            shortcut(keymap, CommandTarget::App(AppCommand::ToggleDesignPanel)),
            ToggleDesignPanel,
        ))
}

/// Context-sensitive property strip shown over the canvas.
pub fn render_context_bar(editor: &Editor, keymap: &Keymap) -> Option<AnyElement> {
    if editor.interaction_kind() == InteractionKind::VectorEditing {
        return Some(
            context_panel()
                .child(context_label("Vector editing"))
                .child(context_separator())
                .child(context_text_button(
                    "join",
                    "Join",
                    editor.can_execute(EditorAction::JoinPaths),
                    shortcut(keymap, CommandTarget::Editor(EditorAction::JoinPaths)),
                    JoinPaths,
                ))
                .child(context_text_button(
                    "split",
                    "Split",
                    editor.can_execute(EditorAction::SplitPath),
                    None,
                    SplitPath,
                ))
                .child(context_text_button(
                    "reverse",
                    "Reverse",
                    editor.can_execute(EditorAction::ReversePath),
                    None,
                    ReversePath,
                ))
                .child(context_text_button(
                    "close",
                    "Open / close",
                    editor.can_execute(EditorAction::TogglePathClosed),
                    None,
                    TogglePathClosed,
                ))
                .child(context_primary_button("done", "Done", FinishEditing))
                .into_any_element(),
        );
    }

    let selection_count = editor.selection().len();
    if selection_count == 0 {
        return None;
    }

    let panel = context_panel();
    if selection_count > 1 {
        return Some(
            with_common_selection_actions(
                panel.child(context_label(format!("{selection_count} objects selected"))),
                editor,
                keymap,
            )
            .into_any_element(),
        );
    }

    if let Some(text) = editor.selected_text_data() {
        let family = PortableFontFamily::from_name(&text.font.family);
        return Some(
            with_common_selection_actions(
                panel
                    .child(context_label(format!(
                        "Text · {}",
                        font_family_label(&text.font.family)
                    )))
                    .child(context_toggle_text_button(
                        "family-system",
                        "System",
                        "Use system sans-serif",
                        family == PortableFontFamily::System,
                        crate::SetTextFamilySystem,
                    ))
                    .child(context_toggle_text_button(
                        "family-serif",
                        "Serif",
                        "Use portable serif",
                        family == PortableFontFamily::Serif,
                        crate::SetTextFamilySerif,
                    ))
                    .child(context_toggle_text_button(
                        "family-monospace",
                        "Mono",
                        "Use portable monospace",
                        family == PortableFontFamily::Monospace,
                        crate::SetTextFamilyMonospace,
                    ))
                    .child(context_separator())
                    .child(context_text_button(
                        "size-down",
                        "−",
                        text.font_size > 1.0,
                        None,
                        TextSmaller,
                    ))
                    .child(context_value(format!("{:.0}", text.font_size)))
                    .child(context_text_button(
                        "size-up",
                        "+",
                        text.font_size < 512.0,
                        None,
                        TextLarger,
                    ))
                    .child(context_separator())
                    .child(context_text_button(
                        "weight-down",
                        "−",
                        text.font.weight > 100,
                        None,
                        crate::TextWeightDown,
                    ))
                    .child(context_value(text.font.weight.to_string()))
                    .child(context_text_button(
                        "weight-up",
                        "+",
                        text.font.weight < 900,
                        None,
                        crate::TextWeightUp,
                    ))
                    .child(context_toggle_text_button(
                        "italic",
                        "I",
                        "Toggle italic",
                        text.font.italic,
                        crate::ToggleTextItalic,
                    ))
                    .child(context_separator())
                    .child(context_icon_button(
                        "align-left",
                        Icon::AlignLeft,
                        "Align left",
                        text.align == TextAlign::Left,
                        AlignTextLeft,
                    ))
                    .child(context_icon_button(
                        "align-center",
                        Icon::AlignCenter,
                        "Align center",
                        text.align == TextAlign::Center,
                        AlignTextCenter,
                    ))
                    .child(context_icon_button(
                        "align-right",
                        Icon::AlignRight,
                        "Align right",
                        text.align == TextAlign::Right,
                        AlignTextRight,
                    )),
                editor,
                keymap,
            )
            .into_any_element(),
        );
    }

    if let Some(frame) = editor.selected_frame_data() {
        return Some(
            with_common_selection_actions(
                panel
                    .child(context_label(format!(
                        "Frame  {:.0} × {:.0}",
                        frame.width, frame.height
                    )))
                    .child(context_separator())
                    .child(context_text_button(
                        "frame-background",
                        if frame.background.is_some() {
                            "White"
                        } else {
                            "Transparent"
                        },
                        true,
                        None,
                        ToggleFrameBackground,
                    )),
                editor,
                keymap,
            )
            .into_any_element(),
        );
    }

    if editor.can_execute(EditorAction::EnterVectorEdit) {
        return Some(
            with_common_selection_actions(
                panel
                    .child(context_label("Vector shape"))
                    .child(context_separator())
                    .child(context_text_button(
                        "edit-vector",
                        "Edit anchors",
                        true,
                        Some("Enter".to_owned()),
                        EditVector,
                    )),
                editor,
                keymap,
            )
            .into_any_element(),
        );
    }

    Some(
        with_common_selection_actions(
            panel.child(context_label("1 object selected")),
            editor,
            keymap,
        )
        .into_any_element(),
    )
}

fn context_panel() -> gpui::Stateful<gpui::Div> {
    div()
        .id("context-properties")
        .absolute()
        .top(px(10.0))
        .left(px(10.0))
        .h(px(34.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .px(px(5.0))
        .bg(rgb(SURFACE_RAISED))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded(px(7.0))
        .shadow_sm()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn context_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px(px(6.0))
        .text_color(rgb(TEXT_MUTED))
        .text_size(px(10.0))
        .child(label.into())
}

fn context_value(value: impl Into<SharedString>) -> impl IntoElement {
    div()
        .min_w(px(34.0))
        .px(px(3.0))
        .text_color(rgb(TEXT))
        .text_size(px(10.0))
        .text_align(gpui::TextAlign::Center)
        .child(value.into())
}

fn context_separator() -> impl IntoElement {
    div().w(px(1.0)).h(px(18.0)).mx(px(2.0)).bg(rgb(BORDER))
}

fn with_common_selection_actions(
    panel: gpui::Stateful<gpui::Div>,
    editor: &Editor,
    keymap: &Keymap,
) -> gpui::Stateful<gpui::Div> {
    let selection_count = editor.selection().len();
    let can_align = editor.can_execute(EditorAction::AlignLeft);
    let can_distribute = editor.can_execute(EditorAction::DistributeHorizontal);
    let can_group = editor.can_execute(EditorAction::Group);
    let can_ungroup = editor.can_execute(EditorAction::Ungroup);

    panel
        .when(can_align, |panel| {
            panel
                .child(context_separator())
                .child(context_icon_button(
                    "object-align-left",
                    Icon::ObjectAlignLeft,
                    "Align left",
                    false,
                    AlignObjectsLeft,
                ))
                .child(context_icon_button(
                    "object-align-center",
                    Icon::ObjectAlignCenter,
                    "Align horizontal centers",
                    false,
                    AlignObjectsCenter,
                ))
                .child(context_icon_button(
                    "object-align-right",
                    Icon::ObjectAlignRight,
                    "Align right",
                    false,
                    AlignObjectsRight,
                ))
                .child(context_icon_button(
                    "object-align-top",
                    Icon::ObjectAlignTop,
                    "Align top",
                    false,
                    AlignObjectsTop,
                ))
                .child(context_icon_button(
                    "object-align-middle",
                    Icon::ObjectAlignMiddle,
                    "Align vertical centers",
                    false,
                    AlignObjectsMiddle,
                ))
                .child(context_icon_button(
                    "object-align-bottom",
                    Icon::ObjectAlignBottom,
                    "Align bottom",
                    false,
                    AlignObjectsBottom,
                ))
        })
        .when(can_distribute, |panel| {
            panel
                .child(context_separator())
                .child(context_icon_button(
                    "distribute-horizontal",
                    Icon::DistributeHorizontal,
                    "Distribute horizontally",
                    false,
                    DistributeObjectsHorizontal,
                ))
                .child(context_icon_button(
                    "distribute-vertical",
                    Icon::DistributeVertical,
                    "Distribute vertically",
                    false,
                    DistributeObjectsVertical,
                ))
        })
        .child(context_separator())
        .child(context_text_button(
            "duplicate",
            "Duplicate",
            editor.can_execute(EditorAction::Duplicate),
            shortcut(keymap, CommandTarget::Editor(EditorAction::Duplicate)),
            Duplicate,
        ))
        .when(selection_count > 1, |panel| {
            panel.child(context_text_button(
                "group",
                "Group",
                can_group,
                shortcut(keymap, CommandTarget::Editor(EditorAction::Group)),
                Group,
            ))
        })
        .when(can_ungroup, |panel| {
            panel.child(context_text_button(
                "ungroup",
                "Ungroup",
                true,
                shortcut(keymap, CommandTarget::Editor(EditorAction::Ungroup)),
                Ungroup,
            ))
        })
        .child(context_text_button(
            "fit-selection",
            "Fit",
            editor.can_execute(EditorAction::ZoomToSelection),
            shortcut(keymap, CommandTarget::Editor(EditorAction::ZoomToSelection)),
            ZoomToSelection,
        ))
        .child(context_text_button(
            "delete",
            "Delete",
            editor.can_execute(EditorAction::Delete),
            shortcut(keymap, CommandTarget::Editor(EditorAction::Delete)),
            Delete,
        ))
}

fn context_text_button<A: Action + Clone>(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    shortcut: Option<String>,
    action: A,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("context-{id}")))
        .h(px(25.0))
        .min_w(px(25.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .text_color(rgb(TEXT))
        .text_size(px(10.0))
        .opacity(if enabled { 1.0 } else { 0.38 })
        .child(label)
        .tooltip(editor_tooltip_owned(
            label,
            shortcut.map(SharedString::from),
        ))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(action.clone()), cx);
                })
        })
}

fn context_toggle_text_button<A: Action + Clone>(
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    active: bool,
    action: A,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("context-{id}")))
        .h(px(25.0))
        .min_w(px(25.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(if active {
            rgb(ACCENT)
        } else {
            rgba(0x00000000)
        })
        .text_color(rgb(if active { 0xffffff } else { TEXT }))
        .text_size(px(10.0))
        .font_weight(if active {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .cursor_pointer()
        .hover(|style| style.bg(rgb(if active { ACCENT } else { SURFACE_HOVER })))
        .child(label)
        .tooltip(editor_tooltip(tooltip, None))
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn context_primary_button<A: Action + Clone>(
    id: &'static str,
    label: &'static str,
    action: A,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("context-{id}")))
        .h(px(25.0))
        .min_w(px(25.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(rgb(ACCENT))
        .text_color(rgb(0xffffff))
        .text_size(px(10.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x179bf0)))
        .tooltip(editor_tooltip(label, Some("Enter")))
        .child(label)
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn context_icon_button<A: Action + Clone>(
    id: &'static str,
    icon_kind: Icon,
    label: &'static str,
    active: bool,
    action: A,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("context-{id}")))
        .w(px(27.0))
        .h(px(25.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(if active {
            rgb(ACCENT)
        } else {
            rgba(0x00000000)
        })
        .cursor_pointer()
        .hover(|style| style.bg(rgb(if active { ACCENT } else { SURFACE_HOVER })))
        .child(icon(icon_kind, 15.0, rgb(TEXT)))
        .tooltip(editor_tooltip(label, None))
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn render_tool_rail(current_tool: Tool, keymap: &Keymap) -> impl IntoElement {
    div()
        .id("tool-rail")
        .h(px(38.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(1.0))
        .px(px(4.0))
        .bg(rgb(SURFACE_RAISED))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded(px(8.0))
        .shadow_sm()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(tool_button(
            "select",
            Icon::Pointer,
            "Select",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolSelect)),
            current_tool == Tool::Select,
            true,
            SelectTool,
        ))
        .child(separator())
        .child(tool_button(
            "frame",
            Icon::Frame,
            "Frame",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolFrame)),
            current_tool == Tool::Frame,
            true,
            FrameTool,
        ))
        .child(tool_button(
            "rectangle",
            Icon::Rectangle,
            "Rectangle",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolRectangle)),
            current_tool == Tool::Rectangle,
            true,
            RectangleTool,
        ))
        .child(tool_button(
            "ellipse",
            Icon::Ellipse,
            "Ellipse",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolEllipse)),
            current_tool == Tool::Ellipse,
            true,
            EllipseTool,
        ))
        .child(tool_button(
            "line",
            Icon::Line,
            "Line",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolLine)),
            current_tool == Tool::Line,
            true,
            LineTool,
        ))
        .child(separator())
        .child(tool_button(
            "pen",
            Icon::Pen,
            "Pen",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolPen)),
            matches!(current_tool, Tool::Pen | Tool::VectorEdit),
            true,
            PenTool,
        ))
        .child(tool_button(
            "text",
            Icon::Text,
            "Text",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolText)),
            current_tool == Tool::Text,
            true,
            TextTool,
        ))
}

pub fn render_menu(
    kind: MenuKind,
    editor: &Editor,
    state: MenuState<'_>,
    viewport_width: f32,
    cx: &mut Context<Strek>,
) -> impl IntoElement {
    let (anchor, corner, panel): (_, _, AnyElement) = match kind {
        MenuKind::Main => (
            point(px(8.0), px(HEADER_HEIGHT + 6.0)),
            Corner::TopLeft,
            render_main_menu(
                editor,
                state.panels,
                state.can_paste,
                state.recent_files,
                state.file_busy,
                state.keymap,
                cx,
            )
            .into_any_element(),
        ),
        MenuKind::Zoom => (
            point(px(viewport_width - 44.0), px(HEADER_HEIGHT + 6.0)),
            Corner::TopRight,
            render_zoom_menu(editor, state.keymap).into_any_element(),
        ),
    };

    div()
        .id("menu-scrim")
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(MouseButton::Left, cx.listener(Strek::close_menu_from_mouse))
        .child(
            anchored()
                .anchor(corner)
                .position(anchor)
                .snap_to_window()
                .child(
                    div()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(panel),
                ),
        )
}

fn render_main_menu(
    editor: &Editor,
    panels: PanelVisibility,
    can_paste: bool,
    recent_files: &[PathBuf],
    file_busy: bool,
    keymap: &Keymap,
    cx: &mut Context<Strek>,
) -> impl IntoElement {
    let can_copy_or_cut = editor.text_input_snapshot().map_or_else(
        || !editor.selection().is_empty(),
        |snapshot| !snapshot.selection.is_empty(),
    );
    let recent_items = recent_files
        .iter()
        .enumerate()
        .map(|(index, path)| recent_file_item(index, path, cx).into_any_element())
        .collect::<Vec<_>>();

    menu_panel("Main menu", 272.0)
        .child(menu_section("File"))
        .child(menu_item(
            "New",
            shortcut_text(keymap, CommandTarget::App(AppCommand::NewDocument)),
            !file_busy,
            NewDocument,
        ))
        .child(menu_item(
            "Open…",
            shortcut_text(keymap, CommandTarget::App(AppCommand::OpenDocument)),
            !file_busy,
            OpenDocument,
        ))
        .child(menu_item(
            "Save",
            shortcut_text(keymap, CommandTarget::App(AppCommand::SaveDocument)),
            !file_busy,
            SaveDocument,
        ))
        .child(menu_item(
            "Save as…",
            shortcut_text(keymap, CommandTarget::App(AppCommand::SaveDocumentAs)),
            !file_busy,
            SaveDocumentAs,
        ))
        .child(menu_item("Export SVG…", "", !file_busy, ExportSvg))
        .child(menu_item(
            "Export outlined SVG…",
            "",
            !file_busy,
            ExportSvgOutlined,
        ))
        .child(menu_item("Export PNG…", "", !file_busy, ExportPng))
        .child(menu_item("Export JPEG…", "", !file_busy, ExportJpeg))
        .child(menu_item("Export WebP…", "", !file_busy, ExportWebP))
        .when(!recent_items.is_empty(), |menu| {
            menu.child(menu_section("Recent")).children(recent_items)
        })
        .child(menu_separator())
        .child(menu_section("Tools"))
        .child(menu_item(
            "Select",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolSelect)),
            editor.can_execute(EditorAction::ToolSelect),
            SelectTool,
        ))
        .child(menu_item(
            "Frame",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolFrame)),
            editor.can_execute(EditorAction::ToolFrame),
            FrameTool,
        ))
        .child(menu_item(
            "Rectangle",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolRectangle)),
            editor.can_execute(EditorAction::ToolRectangle),
            RectangleTool,
        ))
        .child(menu_item(
            "Ellipse",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolEllipse)),
            editor.can_execute(EditorAction::ToolEllipse),
            EllipseTool,
        ))
        .child(menu_item(
            "Line",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolLine)),
            editor.can_execute(EditorAction::ToolLine),
            LineTool,
        ))
        .child(menu_item(
            "Pen",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolPen)),
            editor.can_execute(EditorAction::ToolPen),
            PenTool,
        ))
        .child(menu_item(
            "Text",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ToolText)),
            editor.can_execute(EditorAction::ToolText),
            TextTool,
        ))
        .child(menu_separator())
        .child(menu_section("Edit"))
        .child(menu_item(
            "Undo",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::Undo)),
            editor.can_undo_in_context(),
            Undo,
        ))
        .child(menu_item(
            "Redo",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::Redo)),
            editor.can_redo_in_context(),
            Redo,
        ))
        .child(menu_item(
            "Cut",
            shortcut_text(keymap, CommandTarget::App(AppCommand::Cut)),
            can_copy_or_cut,
            Cut,
        ))
        .child(menu_item(
            "Copy",
            shortcut_text(keymap, CommandTarget::App(AppCommand::Copy)),
            can_copy_or_cut,
            Copy,
        ))
        .child(menu_item(
            "Paste",
            shortcut_text(keymap, CommandTarget::App(AppCommand::Paste)),
            can_paste,
            Paste,
        ))
        .child(menu_separator())
        .child(menu_section("Selection"))
        .child(menu_item(
            "Select all",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::SelectAll)),
            editor.can_execute(EditorAction::SelectAll),
            SelectAll,
        ))
        .child(menu_item(
            "Deselect all",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::DeselectAll)),
            editor.can_execute(EditorAction::DeselectAll),
            DeselectAll,
        ))
        .child(menu_item(
            "Invert selection",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::InvertSelection)),
            editor.can_execute(EditorAction::InvertSelection),
            InvertSelection,
        ))
        .child(menu_separator())
        .child(menu_section("Object"))
        .child(menu_item(
            "Duplicate",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::Duplicate)),
            editor.can_execute(EditorAction::Duplicate),
            Duplicate,
        ))
        .child(menu_item(
            "Delete",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::Delete)),
            editor.can_execute(EditorAction::Delete),
            Delete,
        ))
        .child(menu_item(
            "Group",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::Group)),
            editor.can_execute(EditorAction::Group),
            Group,
        ))
        .child(menu_item(
            "Ungroup",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::Ungroup)),
            editor.can_execute(EditorAction::Ungroup),
            Ungroup,
        ))
        .child(menu_item(
            "Bring forward",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::BringForward)),
            editor.can_execute(EditorAction::BringForward),
            BringForward,
        ))
        .child(menu_item(
            "Send backward",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::SendBackward)),
            editor.can_execute(EditorAction::SendBackward),
            SendBackward,
        ))
        .child(menu_item(
            "Bring to front",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::BringToFront)),
            editor.can_execute(EditorAction::BringToFront),
            BringToFront,
        ))
        .child(menu_item(
            "Send to back",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::SendToBack)),
            editor.can_execute(EditorAction::SendToBack),
            SendToBack,
        ))
        .child(menu_separator())
        .child(menu_section("Align"))
        .child(menu_item(
            "Align left",
            "",
            editor.can_execute(EditorAction::AlignLeft),
            AlignObjectsLeft,
        ))
        .child(menu_item(
            "Align horizontal centers",
            "",
            editor.can_execute(EditorAction::AlignCenter),
            AlignObjectsCenter,
        ))
        .child(menu_item(
            "Align right",
            "",
            editor.can_execute(EditorAction::AlignRight),
            AlignObjectsRight,
        ))
        .child(menu_item(
            "Align top",
            "",
            editor.can_execute(EditorAction::AlignTop),
            AlignObjectsTop,
        ))
        .child(menu_item(
            "Align vertical centers",
            "",
            editor.can_execute(EditorAction::AlignMiddle),
            AlignObjectsMiddle,
        ))
        .child(menu_item(
            "Align bottom",
            "",
            editor.can_execute(EditorAction::AlignBottom),
            AlignObjectsBottom,
        ))
        .child(menu_item(
            "Distribute horizontally",
            "",
            editor.can_execute(EditorAction::DistributeHorizontal),
            DistributeObjectsHorizontal,
        ))
        .child(menu_item(
            "Distribute vertically",
            "",
            editor.can_execute(EditorAction::DistributeVertical),
            DistributeObjectsVertical,
        ))
        .child(menu_separator())
        .child(menu_section("Path"))
        .child(menu_item(
            "Finish editing",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::FinishEditing)),
            editor.can_execute(EditorAction::FinishEditing),
            FinishEditing,
        ))
        .child(menu_item(
            "Join endpoints",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::JoinPaths)),
            editor.can_execute(EditorAction::JoinPaths),
            JoinPaths,
        ))
        .child(menu_item(
            "Split at anchor",
            "",
            editor.can_execute(EditorAction::SplitPath),
            SplitPath,
        ))
        .child(menu_item(
            "Reverse contour",
            "",
            editor.can_execute(EditorAction::ReversePath),
            ReversePath,
        ))
        .child(menu_item(
            "Open or close contour",
            "",
            editor.can_execute(EditorAction::TogglePathClosed),
            TogglePathClosed,
        ))
        .child(menu_separator())
        .child(menu_section("View"))
        .child(menu_item(
            if panels.layers {
                "Hide Layers panel"
            } else {
                "Show Layers panel"
            },
            shortcut_text(keymap, CommandTarget::App(AppCommand::ToggleLayerPanel)),
            true,
            ToggleLayerPanel,
        ))
        .child(menu_item(
            if panels.design {
                "Hide Design panel"
            } else {
                "Show Design panel"
            },
            shortcut_text(keymap, CommandTarget::App(AppCommand::ToggleDesignPanel)),
            true,
            ToggleDesignPanel,
        ))
        .child(menu_item(
            "Command palette…",
            shortcut_text(keymap, CommandTarget::App(AppCommand::ShowCommandPalette)),
            true,
            ShowCommandPalette,
        ))
        .child(menu_separator())
        .child(menu_section("Preferences"))
        .child(menu_item(
            "Keyboard shortcuts…",
            "",
            true,
            OpenKeyboardShortcuts,
        ))
}

fn recent_file_item(index: usize, path: &Path, cx: &mut Context<Strek>) -> impl IntoElement {
    let path = path.to_path_buf();
    let label = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .to_owned();

    div()
        .id(SharedString::from(format!("recent-file-{index}")))
        .h(px(29.0))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(9.0))
        .rounded(px(4.0))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_color(rgb(TEXT))
                .text_size(px(11.0))
                .child(label),
        )
        .child(
            div()
                .max_w(px(116.0))
                .overflow_hidden()
                .text_color(rgb(TEXT_MUTED))
                .text_size(px(9.0))
                .child(
                    path.parent()
                        .map(|parent| parent.display().to_string())
                        .unwrap_or_default(),
                ),
        )
        .on_click(cx.listener(move |editor, _, window, cx| {
            editor.open_recent_document(path.clone(), window, cx);
        }))
}

fn render_zoom_menu(editor: &Editor, keymap: &Keymap) -> impl IntoElement {
    menu_panel("Zoom", 204.0)
        .child(menu_item(
            "Zoom in",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ZoomIn)),
            editor.can_execute(EditorAction::ZoomIn),
            ZoomIn,
        ))
        .child(menu_item(
            "Zoom out",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ZoomOut)),
            editor.can_execute(EditorAction::ZoomOut),
            ZoomOut,
        ))
        .child(menu_item(
            "Actual size",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ZoomReset)),
            editor.can_execute(EditorAction::ZoomReset),
            ZoomReset,
        ))
        .child(menu_item(
            "Reset view",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ZoomResetAll)),
            editor.can_execute(EditorAction::ZoomResetAll),
            ZoomResetAll,
        ))
        .child(menu_separator())
        .child(menu_item(
            "Fit canvas",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ZoomToFit)),
            editor.can_execute(EditorAction::ZoomToFit),
            ZoomToFit,
        ))
        .child(menu_item(
            "Fit selection",
            shortcut_text(keymap, CommandTarget::Editor(EditorAction::ZoomToSelection)),
            editor.can_execute(EditorAction::ZoomToSelection),
            ZoomToSelection,
        ))
}

fn icon_action_button<A: Action + Clone>(
    id: &'static str,
    icon_kind: Icon,
    active: bool,
    enabled: bool,
    label: &'static str,
    shortcut: Option<String>,
    action: A,
) -> impl IntoElement {
    let icon_color = if enabled { TEXT } else { TEXT_MUTED };

    div()
        .id(SharedString::from(format!("header-{id}")))
        .w(px(30.0))
        .h(px(30.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(if active {
            rgb(ACCENT)
        } else {
            rgba(0x00000000)
        })
        .opacity(if enabled { 1.0 } else { 0.35 })
        .child(icon(icon_kind, 17.0, rgb(icon_color)))
        .tooltip(editor_tooltip_owned(
            label,
            shortcut.map(SharedString::from),
        ))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(if active { ACCENT } else { SURFACE_HOVER })))
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(action.clone()), cx);
                })
        })
}

fn zoom_control(
    zoom: f32,
    input: Option<ZoomInputSnapshot<'_>>,
    menu_active: bool,
) -> impl IntoElement {
    div()
        .id("header-zoom-control")
        .h(px(30.0))
        .flex()
        .flex_row()
        .items_center()
        .rounded(px(6.0))
        .bg(if menu_active || input.is_some() {
            rgb(SURFACE_HOVER)
        } else {
            rgba(0x00000000)
        })
        .child(zoom_icon_button(
            "header-zoom-out",
            Icon::Minus,
            "Zoom out",
            ZoomOut,
        ))
        .child(
            div()
                .id("header-zoom-value")
                .h_full()
                .min_w(px(52.0))
                .px(px(5.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(rgb(if input.is_some_and(|input| input.invalid) {
                    0xff8a80
                } else {
                    TEXT
                }))
                .text_size(px(11.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                .child(input.map_or_else(
                    || format_zoom_label(zoom),
                    |input| format!("{}%", input.value),
                ))
                .tooltip(editor_tooltip("Enter zoom percentage", None))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(StartZoomInput), cx);
                }),
        )
        .child(
            div()
                .id("header-zoom-menu")
                .w(px(22.0))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                .child(icon(Icon::ChevronDown, 12.0, rgb(TEXT_MUTED)))
                .tooltip(editor_tooltip("Zoom options", None))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(ToggleZoomMenu), cx);
                }),
        )
        .child(zoom_icon_button(
            "header-zoom-in",
            Icon::Plus,
            "Zoom in",
            ZoomIn,
        ))
}

pub(crate) fn format_zoom_percentage(zoom: f32) -> String {
    let percentage = zoom * 100.0;
    if percentage.fract().abs() < 0.01 {
        format!("{percentage:.0}")
    } else {
        format!("{percentage:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn format_zoom_label(zoom: f32) -> String {
    format!("{}%", format_zoom_percentage(zoom))
}

fn zoom_icon_button<A: Action + Clone>(
    id: &'static str,
    icon_kind: Icon,
    label: &'static str,
    action: A,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(26.0))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(icon(icon_kind, 13.0, rgb(TEXT_MUTED)))
        .tooltip(editor_tooltip(label, None))
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn tool_button<A: Action + Clone>(
    id: &'static str,
    icon_kind: Icon,
    label: &'static str,
    shortcut: String,
    active: bool,
    enabled: bool,
    action: A,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("tool-{id}")))
        .w(px(36.0))
        .h(px(34.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(if active {
            rgb(ACCENT)
        } else {
            rgba(0x00000000)
        })
        .opacity(if enabled { 1.0 } else { 0.32 })
        .child(icon(
            icon_kind,
            18.0,
            rgb(if active { 0xffffff } else { TEXT }),
        ))
        .tooltip(editor_tooltip_owned(
            label,
            (!shortcut.is_empty()).then(|| SharedString::from(shortcut)),
        ))
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(if active { ACCENT } else { SURFACE_HOVER })))
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(action.clone()), cx);
                })
        })
}

fn separator() -> impl IntoElement {
    div().w(px(1.0)).h(px(24.0)).mx(px(3.0)).bg(rgb(BORDER))
}

fn menu_panel(title: &'static str, width: f32) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(format!(
            "{}-menu-panel",
            title.to_lowercase().replace(' ', "-")
        )))
        .w(px(width))
        .max_h(px(700.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .py(px(6.0))
        .bg(rgb(SURFACE_RAISED))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded(px(8.0))
        .shadow_lg()
        .child(
            div()
                .px(px(12.0))
                .pt(px(3.0))
                .pb(px(6.0))
                .text_color(rgb(TEXT))
                .text_size(px(12.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
}

fn menu_section(label: &'static str) -> impl IntoElement {
    div()
        .px(px(12.0))
        .pt(px(6.0))
        .pb(px(3.0))
        .text_size(px(9.0))
        .text_color(rgb(TEXT_MUTED))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(label.to_uppercase())
}

fn menu_item<A: Action + Clone>(
    label: &'static str,
    shortcut: impl Into<SharedString>,
    enabled: bool,
    action: A,
) -> impl IntoElement {
    let shortcut = shortcut.into();
    div()
        .id(SharedString::from(format!(
            "menu-item-{}",
            label.replace(' ', "-").to_lowercase()
        )))
        .h(px(28.0))
        .mx(px(5.0))
        .px(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .rounded(px(4.0))
        .opacity(if enabled { 1.0 } else { 0.35 })
        .text_size(px(11.0))
        .text_color(rgb(TEXT))
        .child(div().flex_1().child(label))
        .child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(TEXT_MUTED))
                .child(shortcut),
        )
        .when(enabled, |item| {
            item.cursor_pointer()
                .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                .on_click(move |_, window: &mut Window, cx: &mut App| {
                    window.dispatch_action(Box::new(action.clone()), cx);
                })
        })
}

fn menu_separator() -> impl IntoElement {
    div().h(px(1.0)).mx(px(8.0)).my(px(4.0)).bg(rgb(BORDER))
}

#[cfg(test)]
mod tests {
    use super::{font_family_label, format_zoom_label, format_zoom_percentage, PortableFontFamily};

    #[test]
    fn zoom_labels_preserve_decimal_percentages() {
        assert_eq!(format_zoom_percentage(1.0), "100");
        assert_eq!(format_zoom_percentage(1.255), "125.5");
        assert_eq!(format_zoom_label(1.255), "125.5%");
    }

    #[test]
    fn portable_font_families_have_stable_active_states_and_labels() {
        assert_eq!(
            PortableFontFamily::from_name("system-ui"),
            PortableFontFamily::System
        );
        assert_eq!(
            PortableFontFamily::from_name(" sans-serif "),
            PortableFontFamily::System
        );
        assert_eq!(
            PortableFontFamily::from_name("SERIF"),
            PortableFontFamily::Serif
        );
        assert_eq!(
            PortableFontFamily::from_name("monospace"),
            PortableFontFamily::Monospace
        );
        assert_eq!(font_family_label("sans-serif"), "System");
        assert_eq!(font_family_label("Custom Sans"), "Custom Sans");
        assert_eq!(font_family_label("  "), "Unknown");
    }
}
