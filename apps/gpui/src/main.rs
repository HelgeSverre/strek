//! GPUI-based desktop application for the vector editor.
//!
//! Uses Zed's GPUI framework for high-performance GPU-accelerated rendering.

mod assets;
mod canvas;
mod layer_panel;
mod status_bar;
mod text_input;
mod toolbar;

use editor_core::{Editor, EditorAction};
use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Application, Bounds, ClipboardItem, Context,
    FocusHandle, Focusable, KeyBinding, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent, Window,
    WindowBounds, WindowOptions,
};

actions!(
    vector_editor,
    [
        Undo,
        Redo,
        SelectAll,
        DeselectAll,
        Backspace,
        Delete,
        Duplicate,
        Group,
        Ungroup,
        BringToFront,
        SendToBack,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ZoomToFit,
        ZoomToSelection,
        ToggleLayerPanel,
        ToggleMainMenu,
        ToggleZoomMenu,
        SelectTool,
        FrameTool,
        RectangleTool,
        EllipseTool,
        PenTool,
        TextTool,
        EditVector,
        FinishEditing,
        JoinPaths,
        SplitPath,
        ReversePath,
        TogglePathClosed,
        TextSmaller,
        TextLarger,
        AlignTextLeft,
        AlignTextCenter,
        AlignTextRight,
        ToggleFrameBackground,
        TextLeft,
        TextRight,
        TextSelectLeft,
        TextSelectRight,
        TextHome,
        TextEnd,
        Copy,
        Cut,
        Paste,
        Enter,
        Escape,
    ]
);

/// Main application state as a GPUI Entity.
struct VectorEditor {
    editor: Editor,
    focus_handle: FocusHandle,
    show_layer_panel: bool,
    open_menu: Option<toolbar::MenuKind>,
    current_cursor: gpui::CursorStyle,
    canvas_input_bounds: Option<Bounds<gpui::Pixels>>,
    text_image_cache: canvas::TextImageCache,
    did_focus: bool,
}

impl VectorEditor {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            editor: Editor::with_demo_content(),
            focus_handle: cx.focus_handle(),
            show_layer_panel: true,
            open_menu: None,
            current_cursor: gpui::CursorStyle::Arrow,
            canvas_input_bounds: None,
            text_image_cache: canvas::TextImageCache::default(),
            did_focus: false,
        }
    }

    fn execute_editor_action(&mut self, action: EditorAction, cx: &mut Context<Self>) {
        let menu_closed = self.open_menu.take().is_some();
        let executed = self.editor.execute_action(action);
        if executed {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        if executed || menu_closed {
            cx.notify();
        }
    }

    fn undo(&mut self, _: &Undo, _window: &mut Window, cx: &mut Context<Self>) {
        let menu_closed = self.open_menu.take().is_some();
        if self.editor.undo_in_context() || menu_closed {
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
        let menu_closed = self.open_menu.take().is_some();
        if self.editor.redo_in_context() || menu_closed {
            cx.notify();
        }
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.text_input_snapshot().is_some() {
            self.editor.select_all_text();
            cx.notify();
        } else {
            self.execute_editor_action(EditorAction::SelectAll, cx);
        }
    }

    fn deselect_all(&mut self, _: &DeselectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::DeselectAll, cx);
    }

    fn backspace(&mut self, _: &Backspace, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.delete_text_backward() {
            cx.notify();
        } else if self.editor.interaction_kind() == editor_core::InteractionKind::Pen {
            if self.editor.undo_pen_edit() {
                cx.notify();
            }
        } else if self.editor.interaction_kind() == editor_core::InteractionKind::VectorEditing {
            if self.editor.delete_selected_vector_anchors() {
                cx.notify();
            }
        } else {
            self.execute_editor_action(EditorAction::Delete, cx);
        }
    }

    fn delete(&mut self, _: &Delete, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.delete_text_forward() {
            cx.notify();
        } else if self.editor.interaction_kind() == editor_core::InteractionKind::Pen {
            // Delete has no destructive document-level meaning while a Pen
            // session owns the keyboard; Backspace removes the last anchor.
        } else if self.editor.interaction_kind() == editor_core::InteractionKind::VectorEditing {
            if self.editor.delete_selected_vector_anchors() {
                cx.notify();
            }
        } else {
            self.execute_editor_action(EditorAction::Delete, cx);
        }
    }

    fn duplicate(&mut self, _: &Duplicate, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::Duplicate, cx);
    }

    fn group(&mut self, _: &Group, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::Group, cx);
    }

    fn ungroup(&mut self, _: &Ungroup, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::Ungroup, cx);
    }

    fn bring_to_front(&mut self, _: &BringToFront, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::BringToFront, cx);
    }

    fn send_to_back(&mut self, _: &SendToBack, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::SendToBack, cx);
    }

    fn zoom_in(&mut self, _: &ZoomIn, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ZoomIn, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ZoomOut, cx);
    }

    fn zoom_reset(&mut self, _: &ZoomReset, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ZoomReset, cx);
    }

    fn zoom_to_fit(&mut self, _: &ZoomToFit, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ZoomToFit, cx);
    }

    fn zoom_to_selection(
        &mut self,
        _: &ZoomToSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::ZoomToSelection, cx);
    }

    fn toggle_layer_panel(
        &mut self,
        _: &ToggleLayerPanel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_layer_panel = !self.show_layer_panel;
        self.open_menu = None;
        cx.notify();
    }

    fn select_tool(&mut self, _: &SelectTool, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ToolSelect, cx);
    }

    fn frame_tool(&mut self, _: &FrameTool, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ToolFrame, cx);
    }

    fn rectangle_tool(&mut self, _: &RectangleTool, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ToolRectangle, cx);
    }

    fn ellipse_tool(&mut self, _: &EllipseTool, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ToolEllipse, cx);
    }

    fn pen_tool(&mut self, _: &PenTool, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ToolPen, cx);
    }

    fn text_tool(&mut self, _: &TextTool, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ToolText, cx);
    }

    fn edit_vector(&mut self, _: &EditVector, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::EnterVectorEdit, cx);
    }

    fn finish_editing(&mut self, _: &FinishEditing, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::FinishEditing, cx);
    }

    fn join_paths(&mut self, _: &JoinPaths, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::JoinPaths, cx);
    }

    fn split_path(&mut self, _: &SplitPath, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::SplitPath, cx);
    }

    fn reverse_path(&mut self, _: &ReversePath, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ReversePath, cx);
    }

    fn toggle_path_closed(
        &mut self,
        _: &TogglePathClosed,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::TogglePathClosed, cx);
    }

    fn text_smaller(&mut self, _: &TextSmaller, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.adjust_selected_text_size(-1.0) {
            cx.notify();
        }
    }

    fn text_larger(&mut self, _: &TextLarger, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.adjust_selected_text_size(1.0) {
            cx.notify();
        }
    }

    fn align_text_left(&mut self, _: &AlignTextLeft, _window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editor
            .set_selected_text_alignment(editor_core::TextAlign::Left)
        {
            cx.notify();
        }
    }

    fn align_text_center(
        &mut self,
        _: &AlignTextCenter,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .set_selected_text_alignment(editor_core::TextAlign::Center)
        {
            cx.notify();
        }
    }

    fn align_text_right(
        &mut self,
        _: &AlignTextRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .set_selected_text_alignment(editor_core::TextAlign::Right)
        {
            cx.notify();
        }
    }

    fn toggle_frame_background(
        &mut self,
        _: &ToggleFrameBackground,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.toggle_selected_frame_background() {
            cx.notify();
        }
    }

    fn text_left(&mut self, _: &TextLeft, _window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editor
            .navigate_text(editor_core::TextNavigation::PreviousGrapheme, false)
        {
            cx.notify();
        }
    }

    fn text_right(&mut self, _: &TextRight, _window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editor
            .navigate_text(editor_core::TextNavigation::NextGrapheme, false)
        {
            cx.notify();
        }
    }

    fn text_select_left(
        &mut self,
        _: &TextSelectLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .navigate_text(editor_core::TextNavigation::PreviousGrapheme, true)
        {
            cx.notify();
        }
    }

    fn text_select_right(
        &mut self,
        _: &TextSelectRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .navigate_text(editor_core::TextNavigation::NextGrapheme, true)
        {
            cx.notify();
        }
    }

    fn text_home(&mut self, _: &TextHome, _window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editor
            .navigate_text(editor_core::TextNavigation::LineStart, false)
        {
            cx.notify();
        }
    }

    fn text_end(&mut self, _: &TextEnd, _window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editor
            .navigate_text(editor_core::TextNavigation::LineEnd, false)
        {
            cx.notify();
        }
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.editor.text_input_snapshot() else {
            return;
        };
        if let Some(selected) = snapshot.text.get(snapshot.selection) {
            if !selected.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
            }
        }
    }

    fn cut(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.editor.text_input_snapshot() else {
            return;
        };
        if let Some(selected) = snapshot.text.get(snapshot.selection.clone()) {
            if !selected.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
                self.editor.replace_text(Some(snapshot.selection), "");
                cx.notify();
            }
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if self.editor.replace_text(None, &text) {
                cx.notify();
            }
        }
    }

    fn enter(&mut self, _: &Enter, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.text_input_snapshot().is_some() {
            self.editor.replace_text(None, "\n");
            cx.notify();
        } else if matches!(
            self.editor.interaction_kind(),
            editor_core::InteractionKind::Pen | editor_core::InteractionKind::VectorEditing
        ) {
            self.editor.finish_editing();
            cx.notify();
        } else {
            self.execute_editor_action(EditorAction::EnterVectorEdit, cx);
        }
    }

    fn toggle_main_menu(
        &mut self,
        _: &ToggleMainMenu,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_menu(toolbar::MenuKind::Main, cx);
    }

    fn toggle_zoom_menu(
        &mut self,
        _: &ToggleZoomMenu,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_menu(toolbar::MenuKind::Zoom, cx);
    }

    fn toggle_menu(&mut self, menu: toolbar::MenuKind, cx: &mut Context<Self>) {
        self.open_menu = (self.open_menu != Some(menu)).then_some(menu);
        cx.notify();
    }

    fn close_menu_from_mouse(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_menu.take().is_some() {
            cx.notify();
        }
    }

    fn escape(&mut self, _: &Escape, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open_menu.take().is_some() {
            cx.notify();
        } else {
            let effects = self.editor.handle_event(editor_core::InputEvent::KeyDown {
                key: editor_core::Key::Escape,
                modifiers: editor_core::Modifiers::default(),
            });
            if let Some(cursor) = effects.cursor {
                self.current_cursor = convert_cursor(cursor);
            }
            if effects.redraw {
                cx.notify();
            }
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        let position = canvas_position(event.position);
        let modifiers = convert_modifiers(&event.modifiers);
        let button = convert_mouse_button(event.button);
        let effects = self.editor.handle_pointer_down_with_click_count(
            position,
            button,
            modifiers,
            event.click_count,
        );
        if effects.redraw {
            cx.notify();
        }
        if let Some(cursor) = effects.cursor {
            self.current_cursor = convert_cursor(cursor);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let position = canvas_position(event.position);
        let modifiers = convert_modifiers(&event.modifiers);
        let button = convert_mouse_button(event.button);

        let input = editor_core::InputEvent::PointerUp {
            position,
            button,
            modifiers,
        };

        let effects = self.editor.handle_event(input);
        if effects.redraw {
            cx.notify();
        }
        if let Some(cursor) = effects.cursor {
            self.current_cursor = convert_cursor(cursor);
            cx.notify();
        }
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = canvas_position(event.position);
        let modifiers = convert_modifiers(&event.modifiers);

        let input = editor_core::InputEvent::PointerMove {
            position,
            modifiers,
        };

        let effects = self.editor.handle_event(input);
        if effects.redraw {
            cx.notify();
        }
        if let Some(cursor) = effects.cursor {
            self.current_cursor = convert_cursor(cursor);
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let position = canvas_position(event.position);
        let pixel_delta = event.delta.pixel_delta(window.line_height());
        let delta = glam::Vec2::new(pixel_delta.x.0, pixel_delta.y.0);
        let modifiers = convert_modifiers(&event.modifiers);

        let input = editor_core::InputEvent::Scroll {
            position,
            delta,
            modifiers,
        };

        let effects = self.editor.handle_event(input);
        if effects.redraw {
            cx.notify();
        }
    }

    fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let effects = self
            .editor
            .handle_event(editor_core::InputEvent::ModifiersChanged {
                modifiers: convert_modifiers(&event.modifiers),
            });
        if effects.redraw {
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "space" || self.editor.text_input_snapshot().is_some() {
            return;
        }
        let effects = self.editor.handle_event(editor_core::InputEvent::KeyDown {
            key: editor_core::Key::Space,
            modifiers: convert_modifiers(&event.keystroke.modifiers),
        });
        if let Some(cursor) = effects.cursor {
            self.current_cursor = convert_cursor(cursor);
        }
        if effects.redraw {
            cx.notify();
        }
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "space" {
            return;
        }
        let effects = self.editor.handle_event(editor_core::InputEvent::KeyUp {
            key: editor_core::Key::Space,
            modifiers: convert_modifiers(&event.keystroke.modifiers),
        });
        if let Some(cursor) = effects.cursor {
            self.current_cursor = convert_cursor(cursor);
        }
        if effects.redraw {
            cx.notify();
        }
    }
}

impl Focusable for VectorEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VectorEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.did_focus {
            self.focus_handle.focus(window);
            self.did_focus = true;
        }
        let tool = self.editor.tool;
        let interaction = self.editor.interaction_kind();
        let key_context = if self.editor.text_input_snapshot().is_some() {
            "VectorTextEditor"
        } else {
            "VectorEditor"
        };
        let selection_count = self.editor.selection().len();
        let zoom = self.editor.view().zoom;
        let show_layer_panel = self.show_layer_panel;
        let open_menu = self.open_menu;
        let cursor = self.current_cursor;

        let window_size = window.viewport_size();
        let panel_width = if show_layer_panel {
            layer_panel::WIDTH
        } else {
            0.0
        };
        self.editor.set_viewport_size(glam::Vec2::new(
            (window_size.width.0 - panel_width).max(1.0),
            (window_size.height.0 - toolbar::HEADER_HEIGHT - status_bar::HEIGHT).max(1.0),
        ));

        let text_layouts =
            canvas::shape_text_layouts(self.editor.document().text_items_for_layout(), window);
        self.editor.set_text_layouts(text_layouts);

        // Build display list for canvas
        let display_list = self.editor.build_display_list();

        // Build layer entries for panel
        let layer_entries = self.editor.build_layer_tree();

        div()
            .id("vector-editor")
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::deselect_all))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::group))
            .on_action(cx.listener(Self::ungroup))
            .on_action(cx.listener(Self::bring_to_front))
            .on_action(cx.listener(Self::send_to_back))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::zoom_to_fit))
            .on_action(cx.listener(Self::zoom_to_selection))
            .on_action(cx.listener(Self::toggle_layer_panel))
            .on_action(cx.listener(Self::toggle_main_menu))
            .on_action(cx.listener(Self::toggle_zoom_menu))
            .on_action(cx.listener(Self::select_tool))
            .on_action(cx.listener(Self::frame_tool))
            .on_action(cx.listener(Self::rectangle_tool))
            .on_action(cx.listener(Self::ellipse_tool))
            .on_action(cx.listener(Self::pen_tool))
            .on_action(cx.listener(Self::text_tool))
            .on_action(cx.listener(Self::edit_vector))
            .on_action(cx.listener(Self::finish_editing))
            .on_action(cx.listener(Self::join_paths))
            .on_action(cx.listener(Self::split_path))
            .on_action(cx.listener(Self::reverse_path))
            .on_action(cx.listener(Self::toggle_path_closed))
            .on_action(cx.listener(Self::text_smaller))
            .on_action(cx.listener(Self::text_larger))
            .on_action(cx.listener(Self::align_text_left))
            .on_action(cx.listener(Self::align_text_center))
            .on_action(cx.listener(Self::align_text_right))
            .on_action(cx.listener(Self::toggle_frame_background))
            .on_action(cx.listener(Self::text_left))
            .on_action(cx.listener(Self::text_right))
            .on_action(cx.listener(Self::text_select_left))
            .on_action(cx.listener(Self::text_select_right))
            .on_action(cx.listener(Self::text_home))
            .on_action(cx.listener(Self::text_end))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::escape))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .child(toolbar::render_header(
                tool,
                zoom,
                show_layer_panel,
                self.editor.can_undo_in_context(),
                self.editor.can_redo_in_context(),
                open_menu,
            ))
            // Main content area
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    // Canvas area
                    .child(
                        div()
                            .id("canvas")
                            .relative()
                            .flex_1()
                            .overflow_hidden()
                            .bg(rgb(0x303136))
                            .cursor(cursor)
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
                            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
                            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
                            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                            .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::on_mouse_up))
                            .on_mouse_up_out(MouseButton::Right, cx.listener(Self::on_mouse_up))
                            .on_mouse_move(cx.listener(Self::on_mouse_move))
                            .on_scroll_wheel(cx.listener(Self::on_scroll))
                            .child(canvas::render_canvas(
                                display_list,
                                self.text_image_cache.clone(),
                            ))
                            .child(text_input::canvas_text_input(cx.entity().clone()))
                            .when_some(toolbar::render_context_bar(&self.editor), |canvas, bar| {
                                canvas.child(bar)
                            }),
                    )
                    // Layer panel (conditionally shown)
                    .when(show_layer_panel, |this| {
                        this.child(layer_panel::render_layer_panel(layer_entries, cx))
                    }),
            )
            // Status bar at bottom
            .child(status_bar::render_status_bar(
                tool,
                interaction,
                selection_count,
                zoom,
            ))
            .when_some(open_menu, |root, menu| {
                root.child(toolbar::render_menu(
                    menu,
                    &self.editor,
                    show_layer_panel,
                    window_size.width.0,
                    cx,
                ))
            })
    }
}

fn canvas_position(position: gpui::Point<gpui::Pixels>) -> glam::Vec2 {
    glam::Vec2::new(position.x.0, position.y.0 - toolbar::HEADER_HEIGHT)
}

fn convert_modifiers(mods: &gpui::Modifiers) -> editor_core::Modifiers {
    editor_core::Modifiers {
        shift: mods.shift,
        ctrl: mods.control,
        alt: mods.alt,
        meta: mods.platform,
    }
}

fn convert_mouse_button(button: MouseButton) -> editor_core::MouseButton {
    match button {
        MouseButton::Left => editor_core::MouseButton::Left,
        MouseButton::Middle => editor_core::MouseButton::Middle,
        MouseButton::Right => editor_core::MouseButton::Right,
        _ => editor_core::MouseButton::Left,
    }
}

fn convert_cursor(cursor: editor_core::Cursor) -> gpui::CursorStyle {
    match cursor {
        editor_core::Cursor::Default => gpui::CursorStyle::Arrow,
        editor_core::Cursor::Pointer => gpui::CursorStyle::PointingHand,
        editor_core::Cursor::Move => gpui::CursorStyle::ClosedHand,
        editor_core::Cursor::Crosshair => gpui::CursorStyle::Crosshair,
        editor_core::Cursor::Text => gpui::CursorStyle::IBeam,
        editor_core::Cursor::Grab => gpui::CursorStyle::OpenHand,
        editor_core::Cursor::Grabbing => gpui::CursorStyle::ClosedHand,
        editor_core::Cursor::ResizeNS => gpui::CursorStyle::ResizeUpDown,
        editor_core::Cursor::ResizeEW => gpui::CursorStyle::ResizeLeftRight,
        editor_core::Cursor::ResizeNESW => gpui::CursorStyle::ResizeUpRightDownLeft,
        editor_core::Cursor::ResizeNWSE => gpui::CursorStyle::ResizeUpLeftDownRight,
    }
}

fn register_keybindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-z", Undo, Some("VectorEditor")),
        KeyBinding::new("cmd-shift-z", Redo, Some("VectorEditor")),
        KeyBinding::new("cmd-a", SelectAll, Some("VectorEditor")),
        KeyBinding::new("cmd-shift-a", DeselectAll, Some("VectorEditor")),
        KeyBinding::new("backspace", Backspace, Some("VectorEditor")),
        KeyBinding::new("delete", Delete, Some("VectorEditor")),
        KeyBinding::new("cmd-d", Duplicate, Some("VectorEditor")),
        KeyBinding::new("cmd-g", Group, Some("VectorEditor")),
        KeyBinding::new("cmd-shift-g", Ungroup, Some("VectorEditor")),
        KeyBinding::new("cmd-]", BringToFront, Some("VectorEditor")),
        KeyBinding::new("cmd-[", SendToBack, Some("VectorEditor")),
        KeyBinding::new("cmd-=", ZoomIn, Some("VectorEditor")),
        KeyBinding::new("cmd-+", ZoomIn, Some("VectorEditor")),
        KeyBinding::new("cmd--", ZoomOut, Some("VectorEditor")),
        KeyBinding::new("cmd-0", ZoomReset, Some("VectorEditor")),
        KeyBinding::new("cmd-1", ZoomToFit, Some("VectorEditor")),
        KeyBinding::new("cmd-2", ZoomToSelection, Some("VectorEditor")),
        KeyBinding::new("cmd-\\", ToggleLayerPanel, Some("VectorEditor")),
        KeyBinding::new("v", SelectTool, Some("VectorEditor")),
        KeyBinding::new("f", FrameTool, Some("VectorEditor")),
        KeyBinding::new("r", RectangleTool, Some("VectorEditor")),
        KeyBinding::new("o", EllipseTool, Some("VectorEditor")),
        KeyBinding::new("p", PenTool, Some("VectorEditor")),
        KeyBinding::new("t", TextTool, Some("VectorEditor")),
        KeyBinding::new("cmd-enter", FinishEditing, Some("VectorEditor")),
        KeyBinding::new("cmd-j", JoinPaths, Some("VectorEditor")),
        KeyBinding::new("left", TextLeft, Some("VectorEditor")),
        KeyBinding::new("right", TextRight, Some("VectorEditor")),
        KeyBinding::new("shift-left", TextSelectLeft, Some("VectorEditor")),
        KeyBinding::new("shift-right", TextSelectRight, Some("VectorEditor")),
        KeyBinding::new("home", TextHome, Some("VectorEditor")),
        KeyBinding::new("end", TextEnd, Some("VectorEditor")),
        KeyBinding::new("cmd-c", Copy, Some("VectorEditor")),
        KeyBinding::new("cmd-x", Cut, Some("VectorEditor")),
        KeyBinding::new("cmd-v", Paste, Some("VectorEditor")),
        KeyBinding::new("enter", Enter, Some("VectorEditor")),
        KeyBinding::new("escape", Escape, Some("VectorEditor")),
        KeyBinding::new("cmd-z", Undo, Some("VectorTextEditor")),
        KeyBinding::new("cmd-shift-z", Redo, Some("VectorTextEditor")),
        KeyBinding::new("cmd-a", SelectAll, Some("VectorTextEditor")),
        KeyBinding::new("backspace", Backspace, Some("VectorTextEditor")),
        KeyBinding::new("delete", Delete, Some("VectorTextEditor")),
        KeyBinding::new("left", TextLeft, Some("VectorTextEditor")),
        KeyBinding::new("right", TextRight, Some("VectorTextEditor")),
        KeyBinding::new("shift-left", TextSelectLeft, Some("VectorTextEditor")),
        KeyBinding::new("shift-right", TextSelectRight, Some("VectorTextEditor")),
        KeyBinding::new("home", TextHome, Some("VectorTextEditor")),
        KeyBinding::new("end", TextEnd, Some("VectorTextEditor")),
        KeyBinding::new("cmd-c", Copy, Some("VectorTextEditor")),
        KeyBinding::new("cmd-x", Cut, Some("VectorTextEditor")),
        KeyBinding::new("cmd-v", Paste, Some("VectorTextEditor")),
        KeyBinding::new("cmd-enter", FinishEditing, Some("VectorTextEditor")),
        KeyBinding::new("enter", Enter, Some("VectorTextEditor")),
        KeyBinding::new("escape", Escape, Some("VectorTextEditor")),
    ]);
}

fn main() {
    env_logger::init();

    Application::new()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            register_keybindings(cx);

            let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(VectorEditor::new),
            )
            .unwrap();
            cx.activate(true);
        });
}
