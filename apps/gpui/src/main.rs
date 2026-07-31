//! GPUI-based desktop application for the vector editor.
//!
//! Uses Zed's GPUI framework for high-performance GPU-accelerated rendering.

mod assets;
mod canvas;
mod document_io;
mod layer_panel;
mod status_bar;
mod text_input;
mod toolbar;

use std::env;
use std::path::{Path, PathBuf};

use editor_core::{Editor, EditorAction};
use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Application, Bounds, ClipboardItem, Context,
    FocusHandle, Focusable, KeyBinding, KeyDownEvent, KeyUpEvent, Menu, MenuItem,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathPromptOptions, PromptLevel, ScrollWheelEvent, Window, WindowBounds, WindowOptions,
};

actions!(
    vector_editor,
    [
        NewDocument,
        OpenDocument,
        SaveDocument,
        SaveDocumentAs,
        QuitApplication,
        Undo,
        Redo,
        SelectAll,
        DeselectAll,
        InvertSelection,
        Backspace,
        Delete,
        Duplicate,
        Group,
        Ungroup,
        BringToFront,
        SendToBack,
        BringForward,
        SendBackward,
        AlignObjectsLeft,
        AlignObjectsCenter,
        AlignObjectsRight,
        AlignObjectsTop,
        AlignObjectsMiddle,
        AlignObjectsBottom,
        DistributeObjectsHorizontal,
        DistributeObjectsVertical,
        NudgeUp,
        NudgeDown,
        NudgeLeft,
        NudgeRight,
        NudgeUpLarge,
        NudgeDownLarge,
        NudgeLeftLarge,
        NudgeRightLarge,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ZoomResetAll,
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

#[derive(Debug, Clone)]
enum PendingDocumentAction {
    New,
    OpenPicker,
    OpenPath(PathBuf),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FileOperation {
    #[default]
    Idle,
    Prompting,
    Opening,
    Saving,
}

/// Main application state as a GPUI Entity.
struct VectorEditor {
    editor: Editor,
    document_path: Option<PathBuf>,
    recent_files: document_io::RecentFiles,
    file_operation: FileOperation,
    allow_window_close: bool,
    object_clipboard: Option<editor_core::EditorClipboard>,
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
            editor: Editor::new(),
            document_path: None,
            recent_files: document_io::RecentFiles::load(),
            file_operation: FileOperation::Idle,
            allow_window_close: false,
            object_clipboard: None,
            focus_handle: cx.focus_handle(),
            show_layer_panel: true,
            open_menu: None,
            current_cursor: gpui::CursorStyle::Arrow,
            canvas_input_bounds: None,
            text_image_cache: canvas::TextImageCache::default(),
            did_focus: false,
        }
    }

    fn document_name(&self) -> String {
        self.document_path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(|name| {
                name.strip_suffix(".vector.json")
                    .or_else(|| name.strip_suffix(".json"))
                    .unwrap_or(name)
                    .to_owned()
            })
            .unwrap_or_else(|| "Untitled".to_owned())
    }

    fn document_location(&self) -> String {
        self.document_path
            .as_deref()
            .and_then(Path::parent)
            .map(|parent| parent.display().to_string())
            .unwrap_or_else(|| "Unsaved document".to_owned())
    }

    fn update_window_title(&self, window: &mut Window) {
        let dirty_marker = if self.editor.is_dirty() { " •" } else { "" };
        window.set_window_title(&format!(
            "{}{} — Vector Editor",
            self.document_name(),
            dirty_marker
        ));
    }

    fn reset_document(&mut self) {
        self.editor = Editor::new();
        self.document_path = None;
        self.object_clipboard = None;
        self.text_image_cache = canvas::TextImageCache::default();
        self.current_cursor = gpui::CursorStyle::Arrow;
        self.open_menu = None;
    }

    fn replace_document(&mut self, document: editor_core::Document, path: PathBuf) {
        self.editor = Editor::from_document(document);
        self.document_path = Some(path.clone());
        self.recent_files.record(&path);
        self.object_clipboard = None;
        self.text_image_cache = canvas::TextImageCache::default();
        self.current_cursor = gpui::CursorStyle::Arrow;
        self.open_menu = None;
    }

    fn new_document(&mut self, _: &NewDocument, window: &mut Window, cx: &mut Context<Self>) {
        self.request_document_action(PendingDocumentAction::New, window, cx);
    }

    fn open_document(&mut self, _: &OpenDocument, window: &mut Window, cx: &mut Context<Self>) {
        self.request_document_action(PendingDocumentAction::OpenPicker, window, cx);
    }

    fn open_recent_document(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.request_document_action(PendingDocumentAction::OpenPath(path), window, cx);
    }

    fn save_document(&mut self, _: &SaveDocument, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        if self.file_operation != FileOperation::Idle {
            return;
        }
        if let Some(path) = self.document_path.clone() {
            self.begin_save_to_path(path, None, window, cx);
        } else {
            self.begin_save_dialog(None, window, cx);
        }
    }

    fn save_document_as(
        &mut self,
        _: &SaveDocumentAs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_menu = None;
        if self.file_operation == FileOperation::Idle {
            self.begin_save_dialog(None, window, cx);
        }
    }

    fn quit_application(
        &mut self,
        _: &QuitApplication,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_document_action(PendingDocumentAction::Close, window, cx);
    }

    fn request_document_action(
        &mut self,
        action: PendingDocumentAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_operation != FileOperation::Idle {
            return;
        }

        self.open_menu = None;
        self.editor.settle_for_document_io();
        if !self.editor.is_dirty() {
            self.perform_document_action(action, window, cx);
            return;
        }

        self.file_operation = FileOperation::Prompting;
        let document_name = self.document_name();
        let prompt = window.prompt(
            PromptLevel::Warning,
            &format!("Save changes to “{document_name}”?"),
            Some("Your changes will be lost if you continue without saving."),
            &["Save", "Discard", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |editor, cx| {
            let answer = prompt.await.unwrap_or(2);
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.file_operation = FileOperation::Idle;
                    match answer {
                        0 => {
                            if let Some(path) = editor.document_path.clone() {
                                editor.begin_save_to_path(path, Some(action), window, cx);
                            } else {
                                editor.begin_save_dialog(Some(action), window, cx);
                            }
                        }
                        1 => editor.perform_document_action(action, window, cx),
                        _ => cx.notify(),
                    }
                })
                .ok();
        })
        .detach();
    }

    fn perform_document_action(
        &mut self,
        action: PendingDocumentAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            PendingDocumentAction::New => {
                self.reset_document();
                cx.notify();
            }
            PendingDocumentAction::OpenPicker => self.begin_open_dialog(window, cx),
            PendingDocumentAction::OpenPath(path) => self.begin_open_path(path, window, cx),
            PendingDocumentAction::Close => {
                self.allow_window_close = true;
                window.remove_window();
            }
        }
    }

    fn begin_open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_operation = FileOperation::Prompting;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
        });
        cx.spawn_in(window, async move |editor, cx| {
            let result = prompt.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.file_operation = FileOperation::Idle;
                    match result {
                        Ok(Ok(Some(paths))) => {
                            if let Some(path) = paths.into_iter().next() {
                                editor.begin_open_path(path, window, cx);
                            }
                        }
                        Ok(Ok(None)) | Err(_) => cx.notify(),
                        Ok(Err(error)) => {
                            editor.show_error(
                                "Could not open file picker",
                                error.to_string(),
                                window,
                                cx,
                            );
                        }
                    }
                })
                .ok();
        })
        .detach();
    }

    fn begin_open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.file_operation = FileOperation::Opening;
        let load_path = path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { document_io::read_document(&load_path) });
        cx.spawn_in(window, async move |editor, cx| {
            let result = task.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.file_operation = FileOperation::Idle;
                    match result {
                        Ok(document) => {
                            editor.replace_document(document, path);
                            cx.notify();
                        }
                        Err(error) => {
                            if !path.is_file() {
                                editor.recent_files.remove(&path);
                            }
                            editor.show_error(
                                "Could not open document",
                                error.to_string(),
                                window,
                                cx,
                            );
                        }
                    }
                })
                .ok();
        })
        .detach();
        self.update_window_title(window);
        cx.notify();
    }

    fn begin_save_dialog(
        &mut self,
        resume: Option<PendingDocumentAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_operation = FileOperation::Prompting;
        let directory = self
            .document_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let prompt = cx.prompt_for_new_path(&directory);
        cx.spawn_in(window, async move |editor, cx| {
            let result = prompt.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.file_operation = FileOperation::Idle;
                    match result {
                        Ok(Ok(Some(path))) => editor.begin_save_to_path(
                            document_io::normalize_document_path(path),
                            resume,
                            window,
                            cx,
                        ),
                        Ok(Ok(None)) | Err(_) => cx.notify(),
                        Ok(Err(error)) => editor.show_error(
                            "Could not open save dialog",
                            error.to_string(),
                            window,
                            cx,
                        ),
                    }
                })
                .ok();
        })
        .detach();
        self.update_window_title(window);
        cx.notify();
    }

    fn begin_save_to_path(
        &mut self,
        path: PathBuf,
        resume: Option<PendingDocumentAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor.settle_for_document_io();
        let json = match document_io::serialize_document(self.editor.document()) {
            Ok(json) => json,
            Err(error) => {
                self.show_error("Could not prepare document", error.to_string(), window, cx);
                return;
            }
        };
        let revision = self.editor.current_revision();
        self.file_operation = FileOperation::Saving;
        let save_path = path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { document_io::write_document(&save_path, &json) });
        cx.spawn_in(window, async move |editor, cx| {
            let result = task.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.file_operation = FileOperation::Idle;
                    match result {
                        Ok(()) => {
                            editor.editor.mark_revision_saved(revision);
                            editor.document_path = Some(path.clone());
                            editor.recent_files.record(&path);
                            if let Some(action) = resume {
                                if editor.editor.is_dirty() {
                                    editor.request_document_action(action, window, cx);
                                } else {
                                    editor.perform_document_action(action, window, cx);
                                }
                            } else {
                                cx.notify();
                            }
                        }
                        Err(error) => editor.show_error(
                            "Could not save document",
                            error.to_string(),
                            window,
                            cx,
                        ),
                    }
                })
                .ok();
        })
        .detach();
        self.update_window_title(window);
        cx.notify();
    }

    fn show_error(
        &mut self,
        title: &str,
        detail: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_operation = FileOperation::Idle;
        let prompt = window.prompt(PromptLevel::Critical, title, Some(&detail), &["OK"], cx);
        cx.spawn(async move |_, _| {
            let _ = prompt.await;
        })
        .detach();
        cx.notify();
    }

    fn should_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.allow_window_close {
            return true;
        }
        if self.file_operation != FileOperation::Idle {
            return false;
        }

        self.editor.settle_for_document_io();
        if self.editor.is_dirty() {
            self.request_document_action(PendingDocumentAction::Close, window, cx);
            false
        } else {
            true
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

    fn invert_selection(
        &mut self,
        _: &InvertSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::InvertSelection, cx);
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

    fn bring_forward(&mut self, _: &BringForward, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::BringForward, cx);
    }

    fn send_backward(&mut self, _: &SendBackward, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::SendBackward, cx);
    }

    fn align_objects_left(
        &mut self,
        _: &AlignObjectsLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::AlignLeft, cx);
    }

    fn align_objects_center(
        &mut self,
        _: &AlignObjectsCenter,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::AlignCenter, cx);
    }

    fn align_objects_right(
        &mut self,
        _: &AlignObjectsRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::AlignRight, cx);
    }

    fn align_objects_top(
        &mut self,
        _: &AlignObjectsTop,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::AlignTop, cx);
    }

    fn align_objects_middle(
        &mut self,
        _: &AlignObjectsMiddle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::AlignMiddle, cx);
    }

    fn align_objects_bottom(
        &mut self,
        _: &AlignObjectsBottom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::AlignBottom, cx);
    }

    fn distribute_objects_horizontal(
        &mut self,
        _: &DistributeObjectsHorizontal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::DistributeHorizontal, cx);
    }

    fn distribute_objects_vertical(
        &mut self,
        _: &DistributeObjectsVertical,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute_editor_action(EditorAction::DistributeVertical, cx);
    }

    fn nudge_up(&mut self, _: &NudgeUp, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open_menu.is_none() {
            self.execute_editor_action(EditorAction::NudgeUp, cx);
        }
    }

    fn nudge_down(&mut self, _: &NudgeDown, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open_menu.is_none() {
            self.execute_editor_action(EditorAction::NudgeDown, cx);
        }
    }

    fn nudge_left(&mut self, _: &NudgeLeft, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open_menu.is_none() {
            self.execute_editor_action(EditorAction::NudgeLeft, cx);
        }
    }

    fn nudge_right(&mut self, _: &NudgeRight, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open_menu.is_none() {
            self.execute_editor_action(EditorAction::NudgeRight, cx);
        }
    }

    fn nudge_up_large(&mut self, _: &NudgeUpLarge, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open_menu.is_none() {
            self.execute_editor_action(EditorAction::NudgeUpLarge, cx);
        }
    }

    fn nudge_down_large(
        &mut self,
        _: &NudgeDownLarge,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_menu.is_none() {
            self.execute_editor_action(EditorAction::NudgeDownLarge, cx);
        }
    }

    fn nudge_left_large(
        &mut self,
        _: &NudgeLeftLarge,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_menu.is_none() {
            self.execute_editor_action(EditorAction::NudgeLeftLarge, cx);
        }
    }

    fn nudge_right_large(
        &mut self,
        _: &NudgeRightLarge,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_menu.is_none() {
            self.execute_editor_action(EditorAction::NudgeRightLarge, cx);
        }
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

    fn zoom_reset_all(&mut self, _: &ZoomResetAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ZoomResetAll, cx);
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
        let menu_closed = self.open_menu.take().is_some();
        if let Some(snapshot) = self.editor.text_input_snapshot() {
            if let Some(selected) = snapshot.text.get(snapshot.selection) {
                if !selected.is_empty() {
                    self.object_clipboard = None;
                    cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
                }
            }
        } else {
            self.object_clipboard = self.editor.copy_selection();
            if self.object_clipboard.is_some() {
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            }
        }
        if menu_closed {
            cx.notify();
        }
    }

    fn cut(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        let menu_closed = self.open_menu.take().is_some();
        let mut changed = false;
        if let Some(snapshot) = self.editor.text_input_snapshot() {
            if let Some(selected) = snapshot.text.get(snapshot.selection.clone()) {
                if !selected.is_empty() {
                    self.object_clipboard = None;
                    cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
                    self.editor.replace_text(Some(snapshot.selection), "");
                    changed = true;
                }
            }
        } else if let Some(clipboard) = self.editor.copy_selection() {
            self.object_clipboard = Some(clipboard);
            cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            self.editor.delete_selection();
            changed = true;
        }
        if changed || menu_closed {
            cx.notify();
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let menu_closed = self.open_menu.take().is_some();
        let mut changed = false;
        if self.editor.text_input_snapshot().is_some() {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                if self.editor.replace_text(None, &text) {
                    changed = true;
                }
            }
        } else if let Some(mut clipboard) = self.object_clipboard.take() {
            if self.editor.paste_clipboard(&mut clipboard) {
                changed = true;
            }
            self.object_clipboard = Some(clipboard);
        }
        if changed || menu_closed {
            cx.notify();
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
        if self.open_menu.is_some()
            || event.keystroke.key != "space"
            || self.editor.text_input_snapshot().is_some()
        {
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
        self.update_window_title(window);
        let tool = self.editor.tool;
        let interaction = self.editor.interaction_kind();
        let key_context = if self.open_menu.is_some() {
            "VectorMenu"
        } else if self.editor.text_input_snapshot().is_some() {
            "VectorTextEditor"
        } else {
            "VectorEditor"
        };
        let selection_count = self.editor.selection().len();
        let zoom = self.editor.view().zoom;
        let show_layer_panel = self.show_layer_panel;
        let open_menu = self.open_menu;
        let cursor = self.current_cursor;
        let document_name = self.document_name();
        let document_location = self.document_location();
        let recent_files = self.recent_files.paths().to_vec();
        let file_busy = self.file_operation != FileOperation::Idle;
        let can_paste = match open_menu {
            Some(toolbar::MenuKind::Main) if self.editor.text_input_snapshot().is_some() => cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|text| !text.is_empty()),
            Some(toolbar::MenuKind::Main) => self.object_clipboard.is_some(),
            _ => false,
        };

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
            .on_action(cx.listener(Self::new_document))
            .on_action(cx.listener(Self::open_document))
            .on_action(cx.listener(Self::save_document))
            .on_action(cx.listener(Self::save_document_as))
            .on_action(cx.listener(Self::quit_application))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::deselect_all))
            .on_action(cx.listener(Self::invert_selection))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::group))
            .on_action(cx.listener(Self::ungroup))
            .on_action(cx.listener(Self::bring_to_front))
            .on_action(cx.listener(Self::send_to_back))
            .on_action(cx.listener(Self::bring_forward))
            .on_action(cx.listener(Self::send_backward))
            .on_action(cx.listener(Self::align_objects_left))
            .on_action(cx.listener(Self::align_objects_center))
            .on_action(cx.listener(Self::align_objects_right))
            .on_action(cx.listener(Self::align_objects_top))
            .on_action(cx.listener(Self::align_objects_middle))
            .on_action(cx.listener(Self::align_objects_bottom))
            .on_action(cx.listener(Self::distribute_objects_horizontal))
            .on_action(cx.listener(Self::distribute_objects_vertical))
            .on_action(cx.listener(Self::nudge_up))
            .on_action(cx.listener(Self::nudge_down))
            .on_action(cx.listener(Self::nudge_left))
            .on_action(cx.listener(Self::nudge_right))
            .on_action(cx.listener(Self::nudge_up_large))
            .on_action(cx.listener(Self::nudge_down_large))
            .on_action(cx.listener(Self::nudge_left_large))
            .on_action(cx.listener(Self::nudge_right_large))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::zoom_reset_all))
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
                toolbar::DocumentHeader {
                    name: document_name,
                    location: document_location,
                    dirty: self.editor.is_dirty(),
                },
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
                    toolbar::MenuState {
                        show_layer_panel,
                        can_paste,
                        recent_files: &recent_files,
                        file_busy,
                    },
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
        KeyBinding::new("secondary-n", NewDocument, Some("VectorEditor")),
        KeyBinding::new("secondary-o", OpenDocument, Some("VectorEditor")),
        KeyBinding::new("secondary-s", SaveDocument, Some("VectorEditor")),
        KeyBinding::new("secondary-shift-s", SaveDocumentAs, Some("VectorEditor")),
        KeyBinding::new("secondary-q", QuitApplication, Some("VectorEditor")),
        KeyBinding::new("secondary-z", Undo, Some("VectorEditor")),
        KeyBinding::new("secondary-shift-z", Redo, Some("VectorEditor")),
        KeyBinding::new("secondary-y", Redo, Some("VectorEditor")),
        KeyBinding::new("secondary-a", SelectAll, Some("VectorEditor")),
        KeyBinding::new("secondary-shift-a", DeselectAll, Some("VectorEditor")),
        KeyBinding::new("secondary-shift-i", InvertSelection, Some("VectorEditor")),
        KeyBinding::new("backspace", Backspace, Some("VectorEditor")),
        KeyBinding::new("delete", Delete, Some("VectorEditor")),
        KeyBinding::new("secondary-d", Duplicate, Some("VectorEditor")),
        KeyBinding::new("secondary-g", Group, Some("VectorEditor")),
        KeyBinding::new("secondary-shift-g", Ungroup, Some("VectorEditor")),
        KeyBinding::new("secondary-]", BringForward, Some("VectorEditor")),
        KeyBinding::new("secondary-[", SendBackward, Some("VectorEditor")),
        KeyBinding::new("secondary-shift-]", BringToFront, Some("VectorEditor")),
        KeyBinding::new("secondary-shift-[", SendToBack, Some("VectorEditor")),
        KeyBinding::new("secondary-=", ZoomIn, Some("VectorEditor")),
        KeyBinding::new("secondary-+", ZoomIn, Some("VectorEditor")),
        KeyBinding::new("secondary--", ZoomOut, Some("VectorEditor")),
        KeyBinding::new("secondary-0", ZoomResetAll, Some("VectorEditor")),
        KeyBinding::new("secondary-1", ZoomReset, Some("VectorEditor")),
        KeyBinding::new("secondary-shift-1", ZoomToFit, Some("VectorEditor")),
        KeyBinding::new("secondary-2", ZoomToSelection, Some("VectorEditor")),
        KeyBinding::new("secondary-\\", ToggleLayerPanel, Some("VectorEditor")),
        KeyBinding::new("v", SelectTool, Some("VectorEditor")),
        KeyBinding::new("f", FrameTool, Some("VectorEditor")),
        KeyBinding::new("r", RectangleTool, Some("VectorEditor")),
        KeyBinding::new("o", EllipseTool, Some("VectorEditor")),
        KeyBinding::new("p", PenTool, Some("VectorEditor")),
        KeyBinding::new("t", TextTool, Some("VectorEditor")),
        KeyBinding::new("secondary-enter", FinishEditing, Some("VectorEditor")),
        KeyBinding::new("secondary-j", JoinPaths, Some("VectorEditor")),
        KeyBinding::new("up", NudgeUp, Some("VectorEditor")),
        KeyBinding::new("down", NudgeDown, Some("VectorEditor")),
        KeyBinding::new("left", NudgeLeft, Some("VectorEditor")),
        KeyBinding::new("right", NudgeRight, Some("VectorEditor")),
        KeyBinding::new("shift-up", NudgeUpLarge, Some("VectorEditor")),
        KeyBinding::new("shift-down", NudgeDownLarge, Some("VectorEditor")),
        KeyBinding::new("shift-left", NudgeLeftLarge, Some("VectorEditor")),
        KeyBinding::new("shift-right", NudgeRightLarge, Some("VectorEditor")),
        KeyBinding::new("secondary-c", Copy, Some("VectorEditor")),
        KeyBinding::new("secondary-x", Cut, Some("VectorEditor")),
        KeyBinding::new("secondary-v", Paste, Some("VectorEditor")),
        KeyBinding::new("enter", Enter, Some("VectorEditor")),
        KeyBinding::new("escape", Escape, Some("VectorEditor")),
        KeyBinding::new("secondary-n", NewDocument, Some("VectorTextEditor")),
        KeyBinding::new("secondary-o", OpenDocument, Some("VectorTextEditor")),
        KeyBinding::new("secondary-s", SaveDocument, Some("VectorTextEditor")),
        KeyBinding::new(
            "secondary-shift-s",
            SaveDocumentAs,
            Some("VectorTextEditor"),
        ),
        KeyBinding::new("secondary-q", QuitApplication, Some("VectorTextEditor")),
        KeyBinding::new("secondary-z", Undo, Some("VectorTextEditor")),
        KeyBinding::new("secondary-shift-z", Redo, Some("VectorTextEditor")),
        KeyBinding::new("secondary-y", Redo, Some("VectorTextEditor")),
        KeyBinding::new("secondary-a", SelectAll, Some("VectorTextEditor")),
        KeyBinding::new("backspace", Backspace, Some("VectorTextEditor")),
        KeyBinding::new("delete", Delete, Some("VectorTextEditor")),
        KeyBinding::new("left", TextLeft, Some("VectorTextEditor")),
        KeyBinding::new("right", TextRight, Some("VectorTextEditor")),
        KeyBinding::new("shift-left", TextSelectLeft, Some("VectorTextEditor")),
        KeyBinding::new("shift-right", TextSelectRight, Some("VectorTextEditor")),
        KeyBinding::new("home", TextHome, Some("VectorTextEditor")),
        KeyBinding::new("end", TextEnd, Some("VectorTextEditor")),
        KeyBinding::new("secondary-c", Copy, Some("VectorTextEditor")),
        KeyBinding::new("secondary-x", Cut, Some("VectorTextEditor")),
        KeyBinding::new("secondary-v", Paste, Some("VectorTextEditor")),
        KeyBinding::new("secondary-enter", FinishEditing, Some("VectorTextEditor")),
        KeyBinding::new("enter", Enter, Some("VectorTextEditor")),
        KeyBinding::new("escape", Escape, Some("VectorTextEditor")),
        KeyBinding::new("secondary-n", NewDocument, Some("VectorMenu")),
        KeyBinding::new("secondary-o", OpenDocument, Some("VectorMenu")),
        KeyBinding::new("secondary-s", SaveDocument, Some("VectorMenu")),
        KeyBinding::new("secondary-shift-s", SaveDocumentAs, Some("VectorMenu")),
        KeyBinding::new("secondary-q", QuitApplication, Some("VectorMenu")),
        KeyBinding::new("escape", Escape, Some("VectorMenu")),
    ]);

    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("ctrl-g", Group, Some("VectorEditor")),
        KeyBinding::new("ctrl-shift-g", Ungroup, Some("VectorEditor")),
    ]);
}

fn main() {
    env_logger::init();

    Application::new()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            register_keybindings(cx);
            cx.set_menus(vec![
                Menu {
                    name: "Vector Editor".into(),
                    items: vec![MenuItem::action("Quit Vector Editor", QuitApplication)],
                },
                Menu {
                    name: "File".into(),
                    items: vec![
                        MenuItem::action("New", NewDocument),
                        MenuItem::action("Open…", OpenDocument),
                        MenuItem::separator(),
                        MenuItem::action("Save", SaveDocument),
                        MenuItem::action("Save As…", SaveDocumentAs),
                    ],
                },
            ]);

            let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let editor = cx.new(VectorEditor::new);
                    let weak_editor = editor.downgrade();
                    window.on_window_should_close(cx, move |window, cx| {
                        weak_editor
                            .update(cx, |editor, cx| editor.should_close_window(window, cx))
                            .unwrap_or(true)
                    });
                    editor
                },
            )
            .unwrap();
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}
