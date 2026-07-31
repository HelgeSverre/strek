//! Editor chrome: document bar, floating tool dock, and command menus.

use editor_core::{Editor, EditorAction, InteractionKind, TextAlign, Tool};
use gpui::{
    anchored, div, point, prelude::*, px, rgb, rgba, Action, AnyElement, App, Context, Corner,
    IntoElement, MouseButton, SharedString, Window,
};

use crate::{
    assets::{icon, Icon},
    AlignTextCenter, AlignTextLeft, AlignTextRight, BringForward, BringToFront, Copy, Cut, Delete,
    DeselectAll, Duplicate, EditVector, EllipseTool, FinishEditing, FrameTool, Group,
    InvertSelection, JoinPaths, Paste, PenTool, RectangleTool, Redo, ReversePath, SelectAll,
    SelectTool, SendBackward, SendToBack, SplitPath, TextLarger, TextSmaller, TextTool,
    ToggleFrameBackground, ToggleLayerPanel, ToggleMainMenu, TogglePathClosed, ToggleZoomMenu,
    Undo, Ungroup, VectorEditor, ZoomIn, ZoomOut, ZoomReset, ZoomResetAll, ZoomToFit,
    ZoomToSelection,
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
pub enum MenuKind {
    Main,
    Zoom,
}

pub fn render_header(
    current_tool: Tool,
    zoom: f32,
    show_layer_panel: bool,
    can_undo: bool,
    can_redo: bool,
    open_menu: Option<MenuKind>,
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
                        .child("Untitled canvas"),
                )
                .child(
                    div()
                        .text_color(rgb(TEXT_MUTED))
                        .text_size(px(10.0))
                        .child("Local draft"),
                ),
        )
        .child(div().flex_1())
        .child(render_tool_rail(current_tool))
        .child(div().flex_1())
        .child(icon_action_button(
            "undo",
            Icon::Undo,
            false,
            can_undo,
            Undo,
        ))
        .child(icon_action_button(
            "redo",
            Icon::Redo,
            false,
            can_redo,
            Redo,
        ))
        .child(div().w(px(1.0)).h(px(22.0)).mx(px(2.0)).bg(rgb(BORDER)))
        .child(zoom_button(zoom, open_menu == Some(MenuKind::Zoom)))
        .child(icon_action_button(
            "layers-panel",
            Icon::PanelRight,
            show_layer_panel,
            true,
            ToggleLayerPanel,
        ))
}

/// Context-sensitive property strip shown over the canvas.
pub fn render_context_bar(editor: &Editor) -> Option<AnyElement> {
    let panel = context_panel();
    if let Some(text) = editor.selected_text_data() {
        return Some(
            panel
                .child(context_label(text.font.family))
                .child(context_separator())
                .child(context_text_button("smaller", "−", TextSmaller))
                .child(
                    div()
                        .min_w(px(38.0))
                        .text_color(rgb(TEXT))
                        .text_size(px(11.0))
                        .text_align(gpui::TextAlign::Center)
                        .child(format!("{:.0}", text.font_size)),
                )
                .child(context_text_button("larger", "+", TextLarger))
                .child(context_separator())
                .child(context_icon_button(
                    "align-left",
                    Icon::AlignLeft,
                    text.align == TextAlign::Left,
                    AlignTextLeft,
                ))
                .child(context_icon_button(
                    "align-center",
                    Icon::AlignCenter,
                    text.align == TextAlign::Center,
                    AlignTextCenter,
                ))
                .child(context_icon_button(
                    "align-right",
                    Icon::AlignRight,
                    text.align == TextAlign::Right,
                    AlignTextRight,
                ))
                .into_any_element(),
        );
    }

    if let Some(frame) = editor.selected_frame_data() {
        return Some(
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
                    ToggleFrameBackground,
                ))
                .into_any_element(),
        );
    }

    if editor.interaction_kind() == InteractionKind::VectorEditing {
        return Some(
            panel
                .child(context_label("Vector"))
                .child(context_separator())
                .child(context_text_button("join", "Join", JoinPaths))
                .child(context_text_button("split", "Split", SplitPath))
                .child(context_text_button("reverse", "Reverse", ReversePath))
                .child(context_text_button(
                    "close",
                    "Open / close",
                    TogglePathClosed,
                ))
                .child(context_text_button("done", "Done", FinishEditing))
                .into_any_element(),
        );
    }

    if editor.can_execute(EditorAction::EnterVectorEdit) {
        return Some(
            panel
                .child(context_label("Vector shape"))
                .child(context_separator())
                .child(context_text_button(
                    "edit-vector",
                    "Edit anchors",
                    EditVector,
                ))
                .into_any_element(),
        );
    }

    None
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

fn context_separator() -> impl IntoElement {
    div().w(px(1.0)).h(px(18.0)).mx(px(2.0)).bg(rgb(BORDER))
}

fn context_text_button<A: Action + Clone>(
    id: &'static str,
    label: &'static str,
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
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .child(label)
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn context_icon_button<A: Action + Clone>(
    id: &'static str,
    icon_kind: Icon,
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
        .on_click(move |_, window, cx| {
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

fn render_tool_rail(current_tool: Tool) -> impl IntoElement {
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
            current_tool == Tool::Select,
            true,
            SelectTool,
        ))
        .child(separator())
        .child(tool_button(
            "frame",
            Icon::Frame,
            current_tool == Tool::Frame,
            true,
            FrameTool,
        ))
        .child(tool_button(
            "rectangle",
            Icon::Rectangle,
            current_tool == Tool::Rectangle,
            true,
            RectangleTool,
        ))
        .child(tool_button(
            "ellipse",
            Icon::Ellipse,
            current_tool == Tool::Ellipse,
            true,
            EllipseTool,
        ))
        .child(separator())
        .child(tool_button(
            "pen",
            Icon::Pen,
            matches!(current_tool, Tool::Pen | Tool::VectorEdit),
            true,
            PenTool,
        ))
        .child(tool_button(
            "text",
            Icon::Text,
            current_tool == Tool::Text,
            true,
            TextTool,
        ))
}

pub fn render_menu(
    kind: MenuKind,
    editor: &Editor,
    show_layer_panel: bool,
    can_paste: bool,
    viewport_width: f32,
    cx: &mut Context<VectorEditor>,
) -> impl IntoElement {
    let (anchor, corner, panel): (_, _, AnyElement) = match kind {
        MenuKind::Main => (
            point(px(8.0), px(HEADER_HEIGHT + 6.0)),
            Corner::TopLeft,
            render_main_menu(editor, show_layer_panel, can_paste).into_any_element(),
        ),
        MenuKind::Zoom => (
            point(px(viewport_width - 44.0), px(HEADER_HEIGHT + 6.0)),
            Corner::TopRight,
            render_zoom_menu(editor).into_any_element(),
        ),
    };

    div()
        .id("menu-scrim")
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(VectorEditor::close_menu_from_mouse),
        )
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

fn render_main_menu(editor: &Editor, show_layer_panel: bool, can_paste: bool) -> impl IntoElement {
    let can_copy_or_cut = editor.text_input_snapshot().map_or_else(
        || !editor.selection().is_empty(),
        |snapshot| !snapshot.selection.is_empty(),
    );

    menu_panel("Main menu", 272.0)
        .child(menu_section("Tools"))
        .child(menu_item(
            "Select",
            "V",
            editor.can_execute(EditorAction::ToolSelect),
            SelectTool,
        ))
        .child(menu_item(
            "Frame",
            "F",
            editor.can_execute(EditorAction::ToolFrame),
            FrameTool,
        ))
        .child(menu_item(
            "Rectangle",
            "R",
            editor.can_execute(EditorAction::ToolRectangle),
            RectangleTool,
        ))
        .child(menu_item(
            "Ellipse",
            "O",
            editor.can_execute(EditorAction::ToolEllipse),
            EllipseTool,
        ))
        .child(menu_item(
            "Pen",
            "P",
            editor.can_execute(EditorAction::ToolPen),
            PenTool,
        ))
        .child(menu_item(
            "Text",
            "T",
            editor.can_execute(EditorAction::ToolText),
            TextTool,
        ))
        .child(menu_separator())
        .child(menu_section("Edit"))
        .child(menu_item("Undo", "⌘Z", editor.can_undo_in_context(), Undo))
        .child(menu_item("Redo", "⇧⌘Z", editor.can_redo_in_context(), Redo))
        .child(menu_item("Cut", "⌘X", can_copy_or_cut, Cut))
        .child(menu_item("Copy", "⌘C", can_copy_or_cut, Copy))
        .child(menu_item("Paste", "⌘V", can_paste, Paste))
        .child(menu_separator())
        .child(menu_section("Selection"))
        .child(menu_item(
            "Select all",
            "⌘A",
            editor.can_execute(EditorAction::SelectAll),
            SelectAll,
        ))
        .child(menu_item(
            "Deselect all",
            "⇧⌘A",
            editor.can_execute(EditorAction::DeselectAll),
            DeselectAll,
        ))
        .child(menu_item(
            "Invert selection",
            "⇧⌘I",
            editor.can_execute(EditorAction::InvertSelection),
            InvertSelection,
        ))
        .child(menu_separator())
        .child(menu_section("Object"))
        .child(menu_item(
            "Duplicate",
            "⌘D",
            editor.can_execute(EditorAction::Duplicate),
            Duplicate,
        ))
        .child(menu_item(
            "Delete",
            "⌫",
            editor.can_execute(EditorAction::Delete),
            Delete,
        ))
        .child(menu_item(
            "Group",
            "⌘G",
            editor.can_execute(EditorAction::Group),
            Group,
        ))
        .child(menu_item(
            "Ungroup",
            "⇧⌘G",
            editor.can_execute(EditorAction::Ungroup),
            Ungroup,
        ))
        .child(menu_item(
            "Bring forward",
            "⌘]",
            editor.can_execute(EditorAction::BringForward),
            BringForward,
        ))
        .child(menu_item(
            "Send backward",
            "⌘[",
            editor.can_execute(EditorAction::SendBackward),
            SendBackward,
        ))
        .child(menu_item(
            "Bring to front",
            "⇧⌘]",
            editor.can_execute(EditorAction::BringToFront),
            BringToFront,
        ))
        .child(menu_item(
            "Send to back",
            "⇧⌘[",
            editor.can_execute(EditorAction::SendToBack),
            SendToBack,
        ))
        .child(menu_separator())
        .child(menu_section("Path"))
        .child(menu_item(
            "Finish editing",
            "⌘↩",
            editor.can_execute(EditorAction::FinishEditing),
            FinishEditing,
        ))
        .child(menu_item(
            "Join endpoints",
            "⌘J",
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
            if show_layer_panel {
                "Hide layers panel"
            } else {
                "Show layers panel"
            },
            "⌘\\",
            true,
            ToggleLayerPanel,
        ))
}

fn render_zoom_menu(editor: &Editor) -> impl IntoElement {
    menu_panel("Zoom", 204.0)
        .child(menu_item(
            "Zoom in",
            "⌘+",
            editor.can_execute(EditorAction::ZoomIn),
            ZoomIn,
        ))
        .child(menu_item(
            "Zoom out",
            "⌘−",
            editor.can_execute(EditorAction::ZoomOut),
            ZoomOut,
        ))
        .child(menu_item(
            "Actual size",
            "⌘1",
            editor.can_execute(EditorAction::ZoomReset),
            ZoomReset,
        ))
        .child(menu_item(
            "Reset view",
            "⌘0",
            editor.can_execute(EditorAction::ZoomResetAll),
            ZoomResetAll,
        ))
        .child(menu_separator())
        .child(menu_item(
            "Fit canvas",
            "⇧⌘1",
            editor.can_execute(EditorAction::ZoomToFit),
            ZoomToFit,
        ))
        .child(menu_item(
            "Fit selection",
            "⌘2",
            editor.can_execute(EditorAction::ZoomToSelection),
            ZoomToSelection,
        ))
}

fn icon_action_button<A: Action + Clone>(
    id: &'static str,
    icon_kind: Icon,
    active: bool,
    enabled: bool,
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
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(if active { ACCENT } else { SURFACE_HOVER })))
                .on_click(move |_, window, cx| {
                    window.dispatch_action(Box::new(action.clone()), cx);
                })
        })
}

fn zoom_button(zoom: f32, active: bool) -> impl IntoElement {
    div()
        .id("header-zoom")
        .h(px(30.0))
        .min_w(px(72.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(2.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .bg(if active {
            rgb(SURFACE_HOVER)
        } else {
            rgba(0x00000000)
        })
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
        .text_color(rgb(TEXT))
        .text_size(px(11.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(format!("{:.0}%", zoom * 100.0))
        .child(icon(Icon::ChevronDown, 13.0, rgb(TEXT_MUTED)))
        .on_click(|_, window, cx| {
            window.dispatch_action(Box::new(ToggleZoomMenu), cx);
        })
}

fn tool_button<A: Action + Clone>(
    id: &'static str,
    icon_kind: Icon,
    active: bool,
    enabled: bool,
    action: A,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("tool-{id}")))
        .relative()
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
        .when(active, |button| {
            button.child(
                div()
                    .absolute()
                    .bottom(px(2.0))
                    .left(px(13.0))
                    .w(px(10.0))
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(rgb(0xffffff)),
            )
        })
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
    shortcut: &'static str,
    enabled: bool,
    action: A,
) -> impl IntoElement {
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
