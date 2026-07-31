//! GPUI-based desktop application for the vector editor.
//!
//! Uses Zed's GPUI framework for high-performance GPU-accelerated rendering.

mod assets;
mod automation;
mod canvas;
mod command_palette;
mod commands;
mod document_io;
mod export;
mod layer_name_input;
mod layer_panel;
mod mcp;
mod properties_panel;
mod status_bar;
mod text_input;
mod toolbar;

use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use editor_core::{
    Editor, EditorAction, NodeId, NumericPropertyScrubSession, NumericPropertyTarget,
};
use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Application, Bounds, ClipboardItem, Context,
    DragMoveEvent, Entity, FocusHandle, Focusable, KeyBinding, KeyDownEvent, KeyUpEvent, Menu,
    MenuItem, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathPromptOptions, PromptLevel, ScrollWheelEvent, Window, WindowBounds, WindowOptions,
};

const OBJECT_CLIPBOARD_METADATA: &str = "strek-object-clipboard-v1";
const MAX_AUTOMATION_SELECTED_LAYERS: usize = 10_000;
const MAX_AUTOMATION_SELECTED_LAYER_BYTES: usize = 256 * 1024;
static OBJECT_CLIPBOARD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

actions!(
    strek,
    [
        NewDocument,
        OpenDocument,
        SaveDocument,
        SaveDocumentAs,
        ExportSvg,
        ExportSvgOutlined,
        ExportPng,
        ExportJpeg,
        ExportWebP,
        OpenKeyboardShortcuts,
        ShowCommandPalette,
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
        StartZoomInput,
        ToggleLayerPanel,
        ToggleDesignPanel,
        ToggleMainMenu,
        ToggleZoomMenu,
        SelectTool,
        FrameTool,
        RectangleTool,
        EllipseTool,
        LineTool,
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
        SetTextFamilySystem,
        SetTextFamilySerif,
        SetTextFamilyMonospace,
        TextWeightDown,
        TextWeightUp,
        ToggleTextItalic,
        AlignTextLeft,
        AlignTextCenter,
        AlignTextRight,
        ToggleFrameBackground,
        SetLayoutFree,
        SetLayoutHorizontal,
        SetLayoutVertical,
        StepLayoutSpacingDown,
        StepLayoutSpacingUp,
        StepLayoutPaddingDown,
        StepLayoutPaddingUp,
        PropertyMoveLeft,
        PropertyMoveRight,
        PropertyMoveUp,
        PropertyMoveDown,
        PropertyRotateLeft,
        PropertyRotateRight,
        PropertyOpacityDown,
        PropertyOpacityUp,
        PropertyFillNone,
        PropertyFillWhite,
        PropertyFillBlack,
        PropertyFillBlue,
        PropertyFillRed,
        PropertyFillGreen,
        StartFillColorInput,
        StartStrokeColorInput,
        PropertyToggleStroke,
        PropertyStrokeDown,
        PropertyStrokeUp,
        ToggleCreationFill,
        StartCreationFillColorInput,
        ToggleCreationStroke,
        StartCreationStrokeColorInput,
        CreationStrokeDown,
        CreationStrokeUp,
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
        SelectParent,
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
    Exporting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorInputScope {
    Selection,
    Creation,
}

#[derive(Debug, Clone)]
struct PropertyColorInput {
    scope: ColorInputScope,
    target: properties_panel::ColorTarget,
    value: String,
    replace_on_type: bool,
    invalid: bool,
}

#[derive(Debug, Clone)]
struct ZoomInput {
    value: String,
    replace_on_type: bool,
    invalid: bool,
}

#[derive(Debug)]
struct ActiveNumericPropertyScrub {
    session: NumericPropertyScrubSession,
    target: NumericPropertyTarget,
    pointer_origin_x: f32,
}

#[derive(Debug)]
struct ObjectClipboard {
    contents: editor_core::EditorClipboard,
    metadata: String,
}

impl ObjectClipboard {
    fn new(contents: editor_core::EditorClipboard) -> Self {
        let sequence = OBJECT_CLIPBOARD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            contents,
            metadata: format!(
                "{OBJECT_CLIPBOARD_METADATA}:{}:{sequence}",
                std::process::id()
            ),
        }
    }

    fn owns(&self, item: &ClipboardItem) -> bool {
        item.metadata()
            .is_some_and(|metadata| metadata == &self.metadata)
    }
}

/// Main application state as a GPUI Entity.
struct Strek {
    editor: Editor,
    document_path: Option<PathBuf>,
    recent_files: document_io::RecentFiles,
    file_operation: FileOperation,
    allow_window_close: bool,
    object_clipboard: Option<ObjectClipboard>,
    keymap: commands::Keymap,
    recent_commands: Vec<commands::CommandTarget>,
    command_palette: Option<Entity<command_palette::CommandPalette>>,
    property_color_input: Option<PropertyColorInput>,
    zoom_input: Option<ZoomInput>,
    numeric_property_scrub: Option<ActiveNumericPropertyScrub>,
    focus_handle: FocusHandle,
    show_layers_panel: bool,
    show_design_panel: bool,
    layers_panel_width: f32,
    design_panel_width: f32,
    layer_name_input: Option<(NodeId, Entity<layer_name_input::LayerNameInput>)>,
    layer_context_menu: Option<layer_panel::LayerContextMenu>,
    open_menu: Option<toolbar::MenuKind>,
    current_cursor: gpui::CursorStyle,
    canvas_input_bounds: Option<Bounds<gpui::Pixels>>,
    text_image_cache: canvas::TextImageCache,
    did_focus: bool,
}

impl Strek {
    fn new(keymap: commands::Keymap, cx: &mut Context<Self>) -> Self {
        Self {
            editor: Editor::new(),
            document_path: None,
            recent_files: document_io::RecentFiles::load(),
            file_operation: FileOperation::Idle,
            allow_window_close: false,
            object_clipboard: None,
            keymap,
            recent_commands: Vec::new(),
            command_palette: None,
            property_color_input: None,
            zoom_input: None,
            numeric_property_scrub: None,
            focus_handle: cx.focus_handle(),
            show_layers_panel: true,
            show_design_panel: true,
            layers_panel_width: layer_panel::DEFAULT_PANEL_WIDTH,
            design_panel_width: layer_panel::DEFAULT_PANEL_WIDTH,
            layer_name_input: None,
            layer_context_menu: None,
            open_menu: None,
            current_cursor: gpui::CursorStyle::Arrow,
            canvas_input_bounds: None,
            text_image_cache: canvas::TextImageCache::default(),
            did_focus: false,
        }
    }

    fn start_automation(
        &mut self,
        mut requests: tokio::sync::mpsc::UnboundedReceiver<automation::PendingRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |editor, cx| {
            while let Some(pending) = requests.recv().await {
                if editor
                    .update_in(cx, |editor, window, cx| {
                        pending
                            .respond_with(|request| editor.handle_automation(request, window, cx));
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_automation(
        &mut self,
        request: automation::AutomationRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> automation::AutomationResponse {
        use automation::{AutomationRequest, PointerPhase, UiTarget};

        let result = match request {
            AutomationRequest::State => Ok("state read".to_owned()),
            AutomationRequest::Action { id } => {
                let Some(spec) = commands::command(&id) else {
                    return automation::AutomationResponse::error(
                        self.automation_state(window),
                        format!("unknown command `{id}`"),
                    );
                };
                let commands::CommandTarget::Editor(action) = spec.target else {
                    return automation::AutomationResponse::error(
                        self.automation_state(window),
                        format!("`{id}` is not an editor action; use the dedicated UI tools"),
                    );
                };
                if !self.command_is_enabled(spec.target) {
                    Err(format!("command `{id}` is disabled in the current state"))
                } else {
                    let palette_closed = self.command_palette.take().is_some();
                    let menu_closed = self.dismiss_menus();
                    if palette_closed || menu_closed {
                        self.focus_handle.focus(window);
                    }
                    self.execute_automation_editor_action(action, window, cx);
                    if palette_closed || menu_closed {
                        cx.notify();
                    }
                    Ok(format!("performed `{id}`"))
                }
            }
            AutomationRequest::Pointer {
                phase,
                x,
                y,
                button,
                modifiers,
            } => {
                if self.file_operation == FileOperation::Opening {
                    Err(
                        "wait for the document to finish opening before sending canvas input"
                            .to_owned(),
                    )
                } else if self.numeric_property_scrub.is_some() {
                    Err(
                        "finish or cancel the numeric property scrub before sending canvas input"
                            .to_owned(),
                    )
                } else if self.command_palette.is_some()
                    || self.open_menu.is_some()
                    || self.layer_context_menu.is_some()
                {
                    Err("dismiss open menus and overlays before sending canvas input".to_owned())
                } else if !x.is_finite() || !y.is_finite() {
                    Err("pointer coordinates must be finite".to_owned())
                } else if matches!(phase, PointerPhase::Down)
                    && self.canvas_input_bounds.is_none_or(|bounds| {
                        x < 0.0 || y < 0.0 || x >= bounds.size.width.0 || y >= bounds.size.height.0
                    })
                {
                    Err("pointer down must be inside the canvas bounds from get_state".to_owned())
                } else {
                    if matches!(phase, PointerPhase::Down) {
                        self.property_color_input = None;
                        self.zoom_input = None;
                        self.focus_handle.focus(window);
                    }
                    let event = match phase {
                        PointerPhase::Down => editor_core::InputEvent::PointerDown {
                            position: glam::Vec2::new(x, y),
                            button: automation_mouse_button(button),
                            modifiers: automation_modifiers(modifiers),
                        },
                        PointerPhase::Move => editor_core::InputEvent::PointerMove {
                            position: glam::Vec2::new(x, y),
                            modifiers: automation_modifiers(modifiers),
                        },
                        PointerPhase::Up => editor_core::InputEvent::PointerUp {
                            position: glam::Vec2::new(x, y),
                            button: automation_mouse_button(button),
                            modifiers: automation_modifiers(modifiers),
                        },
                    };
                    let effects = self.editor.handle_event(event);
                    if let Some(cursor) = effects.cursor {
                        self.current_cursor = convert_cursor(cursor);
                    }
                    if effects.redraw {
                        cx.notify();
                    }
                    Ok(format!("sent pointer {phase:?}"))
                }
            }
            AutomationRequest::Text { text } => {
                if self.file_operation == FileOperation::Opening {
                    Err("wait for the document to finish opening before sending text".to_owned())
                } else if self.command_palette.is_some()
                    || self.open_menu.is_some()
                    || self.layer_context_menu.is_some()
                    || self.property_color_input.is_some()
                    || self.zoom_input.is_some()
                {
                    Err(
                        "dismiss open menus, overlays, and inline inputs before sending text"
                            .to_owned(),
                    )
                } else if self.editor.text_input_snapshot().is_none() {
                    Err("Strek is not editing text".to_owned())
                } else if self.editor.replace_text(None, &text) {
                    cx.notify();
                    Ok("inserted text".to_owned())
                } else {
                    Err("text did not change".to_owned())
                }
            }
            AutomationRequest::SetUi { target, visible } => {
                let scrub_cancelled = self.cancel_numeric_property_scrub();
                match target {
                    UiTarget::MainMenu => {
                        self.dismiss_menus();
                        if visible {
                            if self.editor.cancel_pointer_interaction() {
                                self.current_cursor = convert_cursor(self.editor.cursor());
                            }
                            self.property_color_input = None;
                            self.zoom_input = None;
                            self.finish_layer_rename(true, cx);
                            self.command_palette = None;
                            self.open_menu = Some(toolbar::MenuKind::Main);
                            self.focus_handle.focus(window);
                        }
                        cx.notify();
                    }
                    UiTarget::CommandPalette => {
                        if self.command_palette.is_some() != visible {
                            self.show_command_palette(&ShowCommandPalette, window, cx);
                        }
                    }
                    UiTarget::LayersPanel => {
                        if self.show_layers_panel != visible {
                            self.toggle_layer_panel(&ToggleLayerPanel, window, cx);
                        }
                    }
                    UiTarget::DesignPanel => {
                        if self.show_design_panel != visible {
                            self.toggle_design_panel(&ToggleDesignPanel, window, cx);
                        }
                    }
                }
                if scrub_cancelled {
                    cx.notify();
                }
                Ok(format!("set {target:?} visibility to {visible}"))
            }
            AutomationRequest::Activate => {
                cx.activate(true);
                window.activate_window();
                self.focus_handle.focus(window);
                Ok("activated Strek".to_owned())
            }
        };

        let state = self.automation_state(window);
        match result {
            Ok(message) => automation::AutomationResponse::success(state, message),
            Err(message) => automation::AutomationResponse::error(state, message),
        }
    }

    fn execute_automation_editor_action(
        &mut self,
        action: EditorAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            EditorAction::Undo => self.undo(&Undo, window, cx),
            EditorAction::Redo => self.redo(&Redo, window, cx),
            EditorAction::SelectAll => self.select_all(&SelectAll, window, cx),
            EditorAction::Delete => self.delete(&Delete, window, cx),
            action => self.execute_editor_action(action, cx),
        }
    }

    fn automation_state(&self, window: &Window) -> automation::AutomationState {
        let bounds = window.bounds();
        let view = self.editor.view();
        let (selected_layers, selected_layers_truncated) =
            bounded_automation_layer_names(self.editor.selection().iter().filter_map(|id| {
                self.editor
                    .document()
                    .get(id)
                    .map(|node| node.name.as_str())
            }));
        let actions = commands::COMMANDS
            .iter()
            .filter_map(|spec| {
                let commands::CommandTarget::Editor(_) = spec.target else {
                    return None;
                };
                Some(automation::AutomationAction {
                    id: spec.id.to_owned(),
                    label: spec.label.to_owned(),
                    enabled: self.command_is_enabled(spec.target),
                })
            })
            .collect();

        automation::AutomationState {
            process_id: std::process::id(),
            document: self.document_name(),
            dirty: self.editor.is_dirty(),
            tool: automation_tool_name(self.editor.tool).to_owned(),
            interaction: automation_interaction_name(self.editor.interaction_kind()).to_owned(),
            selection_count: self.editor.selection().len(),
            selected_layers,
            selected_layers_truncated,
            zoom: view.zoom,
            pan: automation::AutomationPoint {
                x: view.pan.x,
                y: view.pan.y,
            },
            window: automation_bounds(bounds),
            canvas: self.canvas_input_bounds.map(automation_bounds),
            layers_panel_visible: self.show_layers_panel,
            design_panel_visible: self.show_design_panel,
            main_menu_open: self.open_menu == Some(toolbar::MenuKind::Main),
            command_palette_open: self.command_palette.is_some(),
            numeric_property_scrub_active: self.numeric_property_scrub.is_some(),
            actions,
        }
    }

    fn document_name(&self) -> String {
        self.document_path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(|name| {
                name.strip_suffix(".strek.json")
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

    fn selected_object_clipboard_text(&self) -> String {
        let names = self
            .editor
            .selection()
            .iter()
            .filter_map(|id| self.editor.document().get(id))
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>();
        if names.is_empty() {
            "Strek vector objects".to_owned()
        } else {
            names.join("\n")
        }
    }

    fn refresh_canvas_input_bounds(&mut self, window: &Window) {
        self.canvas_input_bounds = Some(canvas_bounds_for_layout(
            window.viewport_size(),
            self.show_layers_panel,
            self.layers_panel_width,
            self.show_design_panel,
            self.design_panel_width,
        ));
    }

    fn update_window_title(&self, window: &mut Window) {
        let dirty_marker = if self.editor.is_dirty() { " •" } else { "" };
        window.set_window_title(&format!("{}{} — Strek", self.document_name(), dirty_marker));
    }

    pub(crate) fn begin_layer_rename(
        &mut self,
        id: NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .layer_name_input
            .as_ref()
            .is_some_and(|(active_id, _)| *active_id == id)
        {
            if let Some((_, input)) = &self.layer_name_input {
                input.read(cx).focus(window);
            }
            return;
        }
        self.finish_layer_rename(true, cx);

        let Some(name) = self.editor.document().get(id).map(|node| node.name.clone()) else {
            return;
        };
        self.editor.select_layer(id, false);

        let input =
            cx.new(|input_cx| layer_name_input::LayerNameInput::new(id, name, window, input_cx));
        cx.subscribe_in(
            &input,
            window,
            |editor, _, event: &layer_name_input::LayerNameEvent, window, cx| {
                let event_id = match event {
                    layer_name_input::LayerNameEvent::Commit { id, name } => {
                        editor.editor.rename_layer(*id, name);
                        *id
                    }
                    layer_name_input::LayerNameEvent::Cancel { id } => *id,
                };
                if editor
                    .layer_name_input
                    .as_ref()
                    .is_some_and(|(active_id, _)| *active_id == event_id)
                {
                    editor.layer_name_input = None;
                }
                editor.focus_handle.focus(window);
                cx.notify();
            },
        )
        .detach();
        self.layer_name_input = Some((id, input.clone()));
        cx.defer_in(window, move |_, window, cx| {
            input.read(cx).focus(window);
            cx.notify();
        });
    }

    fn finish_layer_rename(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some((_, input)) = self.layer_name_input.take() else {
            return;
        };
        let (id, name) = input.update(cx, |input, _| input.finish_from_parent());
        if commit {
            self.editor.rename_layer(id, &name);
        }
    }

    pub(crate) fn drop_layer_on(&mut self, dragged: NodeId, target: NodeId) -> bool {
        if dragged == target {
            return false;
        }

        let Some(target_node) = self.editor.document().get(target) else {
            return false;
        };
        let (after_parent, after_index) = if matches!(
            target_node.kind,
            editor_core::NodeKind::Group | editor_core::NodeKind::Frame(_)
        ) {
            let dragged_is_child = self
                .editor
                .document()
                .get(dragged)
                .is_some_and(|node| node.parent == Some(target));
            (
                target,
                target_node
                    .children
                    .len()
                    .saturating_sub(usize::from(dragged_is_child)),
            )
        } else {
            let Some(parent) = target_node.parent else {
                return false;
            };
            let Some(target_index) = self
                .editor
                .document()
                .get(parent)
                .and_then(|node| node.children.iter().position(|child| *child == target))
            else {
                return false;
            };
            let dragged_precedes_target = self
                .editor
                .document()
                .get(dragged)
                .filter(|node| node.parent == Some(parent))
                .and_then(|_| {
                    self.editor
                        .document()
                        .get(parent)?
                        .children
                        .iter()
                        .position(|child| *child == dragged)
                })
                .is_some_and(|dragged_index| dragged_index < target_index);
            (
                parent,
                target_index.saturating_sub(usize::from(dragged_precedes_target)) + 1,
            )
        };

        self.editor
            .reparent_layer(dragged, after_parent, after_index)
    }

    fn reset_document(&mut self) {
        self.cancel_numeric_property_scrub();
        self.editor = editor_preserving_creation_styles(&self.editor, Editor::new());
        self.document_path = None;
        self.object_clipboard = None;
        self.command_palette = None;
        self.property_color_input = None;
        self.zoom_input = None;
        self.layer_name_input = None;
        self.text_image_cache = canvas::TextImageCache::default();
        self.current_cursor = gpui::CursorStyle::Arrow;
        self.dismiss_menus();
    }

    fn replace_document(&mut self, document: editor_core::Document, path: PathBuf) {
        self.cancel_numeric_property_scrub();
        self.editor =
            editor_preserving_creation_styles(&self.editor, Editor::from_document(document));
        self.document_path = Some(path.clone());
        self.recent_files.record(&path);
        self.object_clipboard = None;
        self.command_palette = None;
        self.property_color_input = None;
        self.zoom_input = None;
        self.layer_name_input = None;
        self.text_image_cache = canvas::TextImageCache::default();
        self.current_cursor = gpui::CursorStyle::Arrow;
        self.dismiss_menus();
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
        self.finish_layer_rename(true, cx);
        self.dismiss_menus();
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
        self.finish_layer_rename(true, cx);
        self.dismiss_menus();
        if self.file_operation == FileOperation::Idle {
            self.begin_save_dialog(None, window, cx);
        }
    }

    fn export_svg(&mut self, _: &ExportSvg, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_export_dialog(export::ExportFormat::Svg, window, cx);
    }

    fn export_svg_outlined(
        &mut self,
        _: &ExportSvgOutlined,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_export_dialog(export::ExportFormat::SvgOutlined, window, cx);
    }

    fn export_png(&mut self, _: &ExportPng, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_export_dialog(export::ExportFormat::Png, window, cx);
    }

    fn export_jpeg(&mut self, _: &ExportJpeg, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_export_dialog(export::ExportFormat::Jpeg, window, cx);
    }

    fn export_webp(&mut self, _: &ExportWebP, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_export_dialog(export::ExportFormat::WebP, window, cx);
    }

    fn open_keyboard_shortcuts(
        &mut self,
        _: &OpenKeyboardShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_menus();
        match commands::Keymap::ensure_user_file() {
            Ok(path) => cx.open_url(&commands::file_url(&path)),
            Err(error) => {
                let prompt = window.prompt(
                    PromptLevel::Critical,
                    "Could not open keyboard shortcuts",
                    Some(&error),
                    &["OK"],
                    cx,
                );
                cx.spawn(async move |_, _| {
                    let _ = prompt.await;
                })
                .detach();
            }
        }
    }

    fn show_command_palette(
        &mut self,
        _: &ShowCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_numeric_property_scrub();
        if self.command_palette.take().is_some() {
            self.focus_handle.focus(window);
            cx.notify();
            return;
        }

        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        self.property_color_input = None;
        self.zoom_input = None;
        self.finish_layer_rename(true, cx);
        self.dismiss_menus();
        let entries = commands::COMMANDS
            .iter()
            .filter(|spec| {
                spec.target
                    != commands::CommandTarget::App(commands::AppCommand::ShowCommandPalette)
            })
            .map(|spec| command_palette::PaletteEntry {
                target: spec.target,
                label: spec.label,
                description: spec.description,
                category: spec.category,
                shortcut: self.keymap.shortcut_label(spec.target),
                enabled: self.command_is_enabled(spec.target),
                recent_rank: self
                    .recent_commands
                    .iter()
                    .position(|target| *target == spec.target),
            })
            .collect();
        let palette = cx.new(|cx| command_palette::CommandPalette::new(entries, cx));
        cx.subscribe_in(
            &palette,
            window,
            |editor, _, event: &command_palette::CommandPaletteEvent, window, cx| {
                editor.command_palette = None;
                editor.focus_handle.focus(window);
                if let command_palette::CommandPaletteEvent::Execute(target) = event {
                    editor.recent_commands.retain(|recent| recent != target);
                    editor.recent_commands.insert(0, *target);
                    editor.recent_commands.truncate(8);
                    window.dispatch_action(commands::action_for(*target), cx);
                }
                cx.notify();
            },
        )
        .detach();
        self.command_palette = Some(palette.clone());
        cx.defer_in(window, move |_, window, cx| {
            palette.read(cx).focus(window);
            cx.notify();
        });
    }

    fn command_is_enabled(&self, target: commands::CommandTarget) -> bool {
        use commands::{AppCommand, CommandTarget};

        if self.file_operation == FileOperation::Opening
            && !matches!(
                target,
                CommandTarget::App(
                    AppCommand::Copy
                        | AppCommand::OpenKeyboardShortcuts
                        | AppCommand::ToggleLayerPanel
                        | AppCommand::ToggleDesignPanel
                        | AppCommand::ShowCommandPalette
                )
            )
        {
            return false;
        }

        match target {
            CommandTarget::Editor(action) => self.editor.can_execute(action),
            CommandTarget::App(
                AppCommand::NewDocument
                | AppCommand::OpenDocument
                | AppCommand::SaveDocument
                | AppCommand::SaveDocumentAs
                | AppCommand::ExportSvg
                | AppCommand::ExportSvgOutlined
                | AppCommand::ExportPng
                | AppCommand::ExportJpeg
                | AppCommand::ExportWebP
                | AppCommand::QuitApplication,
            ) => self.file_operation == FileOperation::Idle,
            CommandTarget::App(AppCommand::Copy | AppCommand::Cut) => {
                self.editor.text_input_snapshot().map_or_else(
                    || !self.editor.selection().is_empty(),
                    |snapshot| !snapshot.selection.is_empty(),
                )
            }
            CommandTarget::App(AppCommand::DeleteBackward) => {
                self.editor.text_input_snapshot().is_some() || !self.editor.selection().is_empty()
            }
            CommandTarget::App(AppCommand::Paste) => {
                self.editor.text_input_snapshot().is_some() || self.object_clipboard.is_some()
            }
            CommandTarget::App(
                AppCommand::TextSmaller
                | AppCommand::TextLarger
                | AppCommand::AlignTextLeft
                | AppCommand::AlignTextCenter
                | AppCommand::AlignTextRight,
            ) => self.editor.selected_text_data().is_some(),
            CommandTarget::App(AppCommand::ToggleFrameBackground) => {
                self.editor.selected_frame_data().is_some()
            }
            CommandTarget::App(
                AppCommand::OpenKeyboardShortcuts
                | AppCommand::ToggleLayerPanel
                | AppCommand::ToggleDesignPanel
                | AppCommand::ShowCommandPalette,
            ) => true,
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

        self.finish_layer_rename(true, cx);
        self.dismiss_menus();
        self.settle_for_document_io();
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
        let opening_revision = self.editor.current_revision();
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
                        Ok(document) if editor.editor.current_revision() == opening_revision => {
                            editor.replace_document(document, path);
                            cx.notify();
                        }
                        Ok(_) => editor.show_error(
                            "Document changed while opening",
                            "Strek kept your current changes and did not replace them with the loaded document. Open the file again when editing has stopped."
                                .to_owned(),
                            window,
                            cx,
                        ),
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

    fn begin_export_dialog(
        &mut self,
        format: export::ExportFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_layer_rename(true, cx);
        self.dismiss_menus();
        if self.file_operation != FileOperation::Idle {
            return;
        }
        self.settle_for_document_io();
        if self.editor.artwork_bounds().is_none() {
            self.show_error(
                "Nothing to export",
                "Add at least one visible layer before exporting.".to_owned(),
                window,
                cx,
            );
            return;
        }

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
                        Ok(Ok(Some(path))) => editor.begin_export_to_path(
                            export::normalize_path(path, format),
                            format,
                            window,
                            cx,
                        ),
                        Ok(Ok(None)) | Err(_) => cx.notify(),
                        Ok(Err(error)) => editor.show_error(
                            "Could not open export dialog",
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

    fn begin_export_to_path(
        &mut self,
        path: PathBuf,
        format: export::ExportFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settle_for_document_io();
        let Some(snapshot) = self.editor.artwork_snapshot() else {
            self.show_error(
                "Nothing to export",
                "The document no longer contains visible artwork.".to_owned(),
                window,
                cx,
            );
            return;
        };

        self.file_operation = FileOperation::Exporting;
        let task = cx
            .background_executor()
            .spawn(async move { export::write_export(&path, format, &snapshot) });
        cx.spawn_in(window, async move |editor, cx| {
            let result = task.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.file_operation = FileOperation::Idle;
                    match result {
                        Ok(()) => cx.notify(),
                        Err(error) => editor.show_error(
                            "Could not export artwork",
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
        self.settle_for_document_io();
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

        self.settle_for_document_io();
        if self.editor.is_dirty() {
            self.request_document_action(PendingDocumentAction::Close, window, cx);
            false
        } else {
            true
        }
    }

    fn dismiss_menus(&mut self) -> bool {
        let main_menu_closed = self.open_menu.take().is_some();
        let layer_menu_closed = self.layer_context_menu.take().is_some();
        main_menu_closed || layer_menu_closed
    }

    fn execute_editor_action(&mut self, action: EditorAction, cx: &mut Context<Self>) {
        self.property_color_input = None;
        self.zoom_input = None;
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let menu_closed = self.dismiss_menus();
        let executed = self.editor.execute_action(action);
        if executed {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        if executed || menu_closed || scrub_cancelled {
            cx.notify();
        }
    }

    fn undo(&mut self, _: &Undo, _window: &mut Window, cx: &mut Context<Self>) {
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let menu_closed = self.dismiss_menus();
        if scrub_cancelled {
            cx.notify();
            return;
        }
        if self.editor.undo_in_context() || menu_closed {
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let menu_closed = self.dismiss_menus();
        if scrub_cancelled {
            cx.notify();
            return;
        }
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
        if self.editor.text_input_snapshot().is_some() {
            if self.editor.delete_text_backward() {
                cx.notify();
            }
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
        if self.editor.text_input_snapshot().is_some() {
            if self.editor.delete_text_forward() {
                cx.notify();
            }
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

    fn start_zoom_input(
        &mut self,
        _: &StartZoomInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        self.property_color_input = None;
        self.dismiss_menus();
        self.zoom_input = Some(ZoomInput {
            value: toolbar::format_zoom_percentage(self.editor.view().zoom),
            replace_on_type: true,
            invalid: false,
        });
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn toggle_layer_panel(
        &mut self,
        _: &ToggleLayerPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_numeric_property_scrub();
        self.property_color_input = None;
        self.zoom_input = None;
        self.finish_layer_rename(true, cx);
        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        self.show_layers_panel = !self.show_layers_panel;
        self.refresh_canvas_input_bounds(window);
        self.dismiss_menus();
        cx.notify();
    }

    fn toggle_design_panel(
        &mut self,
        _: &ToggleDesignPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_numeric_property_scrub();
        self.property_color_input = None;
        self.zoom_input = None;
        self.finish_layer_rename(true, cx);
        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        self.show_design_panel = !self.show_design_panel;
        self.refresh_canvas_input_bounds(window);
        self.dismiss_menus();
        cx.notify();
    }

    fn resize_panel(
        &mut self,
        event: &DragMoveEvent<layer_panel::PanelResizeDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let side = event.drag(cx).side();
        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }

        let viewport_width = window.viewport_size().width.0;
        let pointer_x = event.event.position.x.0;
        let requested_width = match side {
            layer_panel::PanelSide::Layers => pointer_x,
            layer_panel::PanelSide::Design => viewport_width - pointer_x,
        };
        let opposite_width = match side {
            layer_panel::PanelSide::Layers if self.show_design_panel => self.design_panel_width,
            layer_panel::PanelSide::Design if self.show_layers_panel => self.layers_panel_width,
            _ => 0.0,
        };
        let width = clamp_panel_width(requested_width, opposite_width, viewport_width);
        let current_width = match side {
            layer_panel::PanelSide::Layers => &mut self.layers_panel_width,
            layer_panel::PanelSide::Design => &mut self.design_panel_width,
        };
        if (*current_width - width).abs() > f32::EPSILON {
            *current_width = width;
            self.refresh_canvas_input_bounds(window);
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn begin_numeric_property_scrub(
        &mut self,
        target: NumericPropertyTarget,
        pointer_origin_x: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_operation == FileOperation::Opening || !pointer_origin_x.is_finite() {
            return;
        }
        self.cancel_numeric_property_scrub();
        self.finish_layer_rename(true, cx);
        let Some(session) = self.editor.begin_numeric_property_scrub(target) else {
            return;
        };
        self.property_color_input = None;
        self.zoom_input = None;
        self.dismiss_menus();
        self.numeric_property_scrub = Some(ActiveNumericPropertyScrub {
            session,
            target,
            pointer_origin_x,
        });
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn preview_numeric_property_scrub_at(
        &mut self,
        pointer_x: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scrub) = self.numeric_property_scrub.as_ref() else {
            return false;
        };
        if !pointer_x.is_finite() {
            return false;
        }
        let pointer_delta = pointer_x - scrub.pointer_origin_x;
        let delta = properties_panel::numeric_property_delta(scrub.target, pointer_delta);
        let changed = self
            .editor
            .preview_numeric_property_scrub(&scrub.session, delta);
        cx.notify();
        changed
    }

    fn preview_numeric_property_scrub(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button == Some(MouseButton::Left) {
            self.preview_numeric_property_scrub_at(event.position.x.0, cx);
        }
    }

    fn finish_numeric_property_scrub(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        self.preview_numeric_property_scrub_at(event.position.x.0, cx);
        let Some(scrub) = self.numeric_property_scrub.take() else {
            return;
        };
        self.editor.commit_numeric_property_scrub(scrub.session);
        cx.notify();
    }

    fn cancel_numeric_property_scrub(&mut self) -> bool {
        let Some(scrub) = self.numeric_property_scrub.take() else {
            return false;
        };
        self.editor.cancel_numeric_property_scrub(scrub.session);
        true
    }

    fn settle_for_document_io(&mut self) {
        self.cancel_numeric_property_scrub();
        self.editor.settle_for_document_io();
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

    fn line_tool(&mut self, _: &LineTool, _window: &mut Window, cx: &mut Context<Self>) {
        self.execute_editor_action(EditorAction::ToolLine, cx);
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

    fn set_text_family_system(
        &mut self,
        _: &SetTextFamilySystem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.set_selected_text_font_family("sans-serif") {
            cx.notify();
        }
    }

    fn set_text_family_serif(
        &mut self,
        _: &SetTextFamilySerif,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.set_selected_text_font_family("serif") {
            cx.notify();
        }
    }

    fn set_text_family_monospace(
        &mut self,
        _: &SetTextFamilyMonospace,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.set_selected_text_font_family("monospace") {
            cx.notify();
        }
    }

    fn text_weight_down(
        &mut self,
        _: &TextWeightDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_selected_text_weight(-100, cx);
    }

    fn text_weight_up(&mut self, _: &TextWeightUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_selected_text_weight(100, cx);
    }

    fn step_selected_text_weight(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(text) = self.editor.selected_text_data() else {
            return;
        };
        let weight = (i32::from(text.font.weight) + delta).clamp(0, i32::from(u16::MAX));
        if self.editor.set_selected_text_font_weight(weight as u16) {
            cx.notify();
        }
    }

    fn toggle_text_italic(
        &mut self,
        _: &ToggleTextItalic,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.editor.selected_text_data() else {
            return;
        };
        if self.editor.set_selected_text_italic(!text.font.italic) {
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

    fn set_layout_free(&mut self, _: &SetLayoutFree, _window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editor
            .set_selected_group_layout_mode(editor_core::GroupLayoutMode::Free)
        {
            cx.notify();
        }
    }

    fn set_layout_horizontal(
        &mut self,
        _: &SetLayoutHorizontal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .set_selected_group_layout_mode(editor_core::GroupLayoutMode::Horizontal)
        {
            cx.notify();
        }
    }

    fn set_layout_vertical(
        &mut self,
        _: &SetLayoutVertical,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .set_selected_group_layout_mode(editor_core::GroupLayoutMode::Vertical)
        {
            cx.notify();
        }
    }

    fn selected_auto_layout(&self) -> Option<editor_core::AutoLayout> {
        match self.editor.selected_group_layout()? {
            editor_core::Layout::Auto(layout) => Some(layout),
            editor_core::Layout::Free => None,
        }
    }

    fn step_layout_spacing(&mut self, delta: f32, cx: &mut Context<Self>) {
        let Some(layout) = self.selected_auto_layout() else {
            return;
        };
        let spacing = (layout.spacing + delta).max(0.0);
        if self.editor.set_selected_group_spacing(spacing) {
            cx.notify();
        }
    }

    fn step_layout_spacing_down(
        &mut self,
        _: &StepLayoutSpacingDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_layout_spacing(-1.0, cx);
    }

    fn step_layout_spacing_up(
        &mut self,
        _: &StepLayoutSpacingUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_layout_spacing(1.0, cx);
    }

    fn step_layout_padding(&mut self, delta: f32, cx: &mut Context<Self>) {
        let Some(layout) = self.selected_auto_layout() else {
            return;
        };
        let padding = layout.padding;
        let current = (padding.left + padding.top + padding.right + padding.bottom) / 4.0;
        if self
            .editor
            .set_selected_group_uniform_padding((current + delta).max(0.0))
        {
            cx.notify();
        }
    }

    fn step_layout_padding_down(
        &mut self,
        _: &StepLayoutPaddingDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_layout_padding(-1.0, cx);
    }

    fn step_layout_padding_up(
        &mut self,
        _: &StepLayoutPaddingUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_layout_padding(1.0, cx);
    }

    fn property_move_left(
        &mut self,
        _: &PropertyMoveLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.move_selection_by(glam::Vec2::new(-1.0, 0.0)) {
            cx.notify();
        }
    }

    fn property_move_right(
        &mut self,
        _: &PropertyMoveRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.move_selection_by(glam::Vec2::new(1.0, 0.0)) {
            cx.notify();
        }
    }

    fn property_move_up(
        &mut self,
        _: &PropertyMoveUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.move_selection_by(glam::Vec2::new(0.0, -1.0)) {
            cx.notify();
        }
    }

    fn property_move_down(
        &mut self,
        _: &PropertyMoveDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.move_selection_by(glam::Vec2::new(0.0, 1.0)) {
            cx.notify();
        }
    }

    fn property_rotate_left(
        &mut self,
        _: &PropertyRotateLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .rotate_selection_by(-std::f32::consts::PI / 12.0)
        {
            cx.notify();
        }
    }

    fn property_rotate_right(
        &mut self,
        _: &PropertyRotateRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.rotate_selection_by(std::f32::consts::PI / 12.0) {
            cx.notify();
        }
    }

    fn selected_opacity(&self) -> f32 {
        self.editor
            .selection()
            .primary()
            .and_then(|id| self.editor.document().get(id))
            .map(|node| node.style.opacity)
            .unwrap_or(1.0)
    }

    fn property_opacity_down(
        &mut self,
        _: &PropertyOpacityDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let opacity = self.selected_opacity() - 0.1;
        if self.editor.set_selected_opacity(opacity) {
            cx.notify();
        }
    }

    fn property_opacity_up(
        &mut self,
        _: &PropertyOpacityUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let opacity = self.selected_opacity() + 0.1;
        if self.editor.set_selected_opacity(opacity) {
            cx.notify();
        }
    }

    fn start_fill_color_input(
        &mut self,
        _: &StartFillColorInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_property_color_input(
            ColorInputScope::Selection,
            properties_panel::ColorTarget::Fill,
            window,
            cx,
        );
    }

    fn start_stroke_color_input(
        &mut self,
        _: &StartStrokeColorInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_property_color_input(
            ColorInputScope::Selection,
            properties_panel::ColorTarget::Stroke,
            window,
            cx,
        );
    }

    fn start_property_color_input(
        &mut self,
        scope: ColorInputScope,
        target: properties_panel::ColorTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        let Some(style) = color_input_style(&self.editor, scope) else {
            return;
        };
        let paint = match target {
            properties_panel::ColorTarget::Fill => style.fill.as_ref(),
            properties_panel::ColorTarget::Stroke => {
                style.stroke.as_ref().map(|stroke| &stroke.paint)
            }
        }
        .cloned()
        .unwrap_or_else(editor_core::Paint::black);
        self.zoom_input = None;
        self.property_color_input = Some(PropertyColorInput {
            scope,
            target,
            value: properties_panel::format_paint(&paint),
            replace_on_type: true,
            invalid: false,
        });
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn start_creation_fill_color_input(
        &mut self,
        _: &StartCreationFillColorInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_property_color_input(
            ColorInputScope::Creation,
            properties_panel::ColorTarget::Fill,
            window,
            cx,
        );
    }

    fn start_creation_stroke_color_input(
        &mut self,
        _: &StartCreationStrokeColorInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_property_color_input(
            ColorInputScope::Creation,
            properties_panel::ColorTarget::Stroke,
            window,
            cx,
        );
    }

    fn toggle_creation_fill(
        &mut self,
        _: &ToggleCreationFill,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.property_color_input = None;
        if self.editor.toggle_creation_fill() {
            cx.notify();
        }
    }

    fn toggle_creation_stroke(
        &mut self,
        _: &ToggleCreationStroke,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.property_color_input = None;
        if self.editor.toggle_creation_stroke() {
            cx.notify();
        }
    }

    fn creation_stroke_down(
        &mut self,
        _: &CreationStrokeDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.adjust_creation_stroke_width(-1.0) {
            cx.notify();
        }
    }

    fn creation_stroke_up(
        &mut self,
        _: &CreationStrokeUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.adjust_creation_stroke_width(1.0) {
            cx.notify();
        }
    }

    fn set_property_fill(&mut self, fill: Option<editor_core::Paint>, cx: &mut Context<Self>) {
        self.property_color_input = None;
        if self.editor.set_selected_fill(fill) {
            cx.notify();
        }
    }

    fn property_fill_none(
        &mut self,
        _: &PropertyFillNone,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_property_fill(None, cx);
    }

    fn property_fill_white(
        &mut self,
        _: &PropertyFillWhite,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_property_fill(Some(editor_core::Paint::white()), cx);
    }

    fn property_fill_black(
        &mut self,
        _: &PropertyFillBlack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_property_fill(Some(editor_core::Paint::black()), cx);
    }

    fn property_fill_blue(
        &mut self,
        _: &PropertyFillBlue,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_property_fill(Some(editor_core::Paint::rgb(0.047, 0.55, 0.91)), cx);
    }

    fn property_fill_red(
        &mut self,
        _: &PropertyFillRed,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_property_fill(Some(editor_core::Paint::rgb(0.95, 0.28, 0.13)), cx);
    }

    fn property_fill_green(
        &mut self,
        _: &PropertyFillGreen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_property_fill(Some(editor_core::Paint::rgb(0.08, 0.68, 0.36)), cx);
    }

    fn property_toggle_stroke(
        &mut self,
        _: &PropertyToggleStroke,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.property_color_input = None;
        if self.editor.toggle_selected_stroke() {
            cx.notify();
        }
    }

    fn property_stroke_down(
        &mut self,
        _: &PropertyStrokeDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.adjust_selected_stroke_width(-1.0) {
            cx.notify();
        }
    }

    fn property_stroke_up(
        &mut self,
        _: &PropertyStrokeUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.adjust_selected_stroke_width(1.0) {
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
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let menu_closed = self.dismiss_menus();
        if let Some(snapshot) = self.editor.text_input_snapshot() {
            if let Some(selected) = snapshot.text.get(snapshot.selection) {
                if !selected.is_empty() {
                    self.object_clipboard = None;
                    cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
                }
            }
        } else {
            let clipboard_text = self.selected_object_clipboard_text();
            self.object_clipboard = self.editor.copy_selection().map(ObjectClipboard::new);
            if let Some(clipboard) = &self.object_clipboard {
                cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
                    clipboard_text,
                    clipboard.metadata.clone(),
                ));
            }
        }
        if menu_closed || scrub_cancelled {
            cx.notify();
        }
    }

    fn cut(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let menu_closed = self.dismiss_menus();
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
        } else {
            let clipboard_text = self.selected_object_clipboard_text();
            if let Some(clipboard) = self.editor.copy_selection() {
                let clipboard = ObjectClipboard::new(clipboard);
                cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
                    clipboard_text,
                    clipboard.metadata.clone(),
                ));
                self.object_clipboard = Some(clipboard);
                self.editor.delete_selection();
                changed = true;
            }
        }
        if changed || menu_closed || scrub_cancelled {
            cx.notify();
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let menu_closed = self.dismiss_menus();
        let mut changed = false;
        if self.editor.text_input_snapshot().is_some() {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                if self.editor.replace_text(None, &text) {
                    changed = true;
                }
            }
        } else if cx.read_from_clipboard().as_ref().is_some_and(|item| {
            self.object_clipboard
                .as_ref()
                .is_some_and(|clipboard| clipboard.owns(item))
        }) {
            if let Some(mut clipboard) = self.object_clipboard.take() {
                if self.editor.paste_clipboard(&mut clipboard.contents) {
                    changed = true;
                }
                self.object_clipboard = Some(clipboard);
            }
        } else {
            self.object_clipboard = None;
        }
        if changed || menu_closed || scrub_cancelled {
            cx.notify();
        }
    }

    fn enter(&mut self, _: &Enter, _window: &mut Window, cx: &mut Context<Self>) {
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let changed = if self.editor.text_input_snapshot().is_some() {
            self.editor.replace_text(None, "\n")
        } else if matches!(
            self.editor.interaction_kind(),
            editor_core::InteractionKind::Pen | editor_core::InteractionKind::VectorEditing
        ) {
            self.editor.finish_editing()
        } else if self.editor.enter_selection() {
            self.current_cursor = convert_cursor(self.editor.cursor());
            true
        } else {
            false
        };
        if changed || scrub_cancelled {
            cx.notify();
        }
    }

    fn select_parent(&mut self, _: &SelectParent, _window: &mut Window, cx: &mut Context<Self>) {
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let changed = self.editor.select_parent();
        if changed {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        if changed || scrub_cancelled {
            cx.notify();
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
        self.zoom_input = None;
        let was_open = self.open_menu == Some(menu);
        self.dismiss_menus();
        self.open_menu = (!was_open).then_some(menu);
        cx.notify();
    }

    fn close_menu_from_mouse(
        &mut self,
        _: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dismiss_menus() {
            cx.notify();
        }
    }

    fn dismiss_inline_input_from_mouse(
        &mut self,
        _: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.property_color_input.take().is_some() || self.zoom_input.take().is_some() {
            cx.notify();
        }
    }

    fn escape(&mut self, _: &Escape, _window: &mut Window, cx: &mut Context<Self>) {
        if self.cancel_numeric_property_scrub() {
            cx.notify();
            return;
        }
        if self.dismiss_menus() {
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
        if self.file_operation == FileOperation::Opening {
            return;
        }
        self.property_color_input = None;
        self.zoom_input = None;
        self.focus_handle.focus(window);
        let position = canvas_position(
            event.position,
            self.canvas_input_bounds,
            visible_panel_width(self.show_layers_panel, self.layers_panel_width),
        );
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
        if self.file_operation == FileOperation::Opening {
            return;
        }
        let position = canvas_position(
            event.position,
            self.canvas_input_bounds,
            visible_panel_width(self.show_layers_panel, self.layers_panel_width),
        );
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
        if self.file_operation == FileOperation::Opening {
            return;
        }
        let position = canvas_position(
            event.position,
            self.canvas_input_bounds,
            visible_panel_width(self.show_layers_panel, self.layers_panel_width),
        );
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
        if self.file_operation == FileOperation::Opening {
            return;
        }
        let position = canvas_position(
            event.position,
            self.canvas_input_bounds,
            visible_panel_width(self.show_layers_panel, self.layers_panel_width),
        );
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
        if let Some(cursor) = effects.cursor {
            self.current_cursor = convert_cursor(cursor);
            cx.notify();
        }
    }

    fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_operation == FileOperation::Opening {
            return;
        }
        let effects = self
            .editor
            .handle_event(editor_core::InputEvent::ModifiersChanged {
                modifiers: convert_modifiers(&event.modifiers),
            });
        if effects.redraw {
            cx.notify();
        }
    }

    fn handle_property_color_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let primary = if cfg!(target_os = "macos") {
            event.keystroke.modifiers.platform
        } else {
            event.keystroke.modifiers.control
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.property_color_input = None;
                cx.notify();
            }
            "enter" => {
                let Some(input) = self.property_color_input.as_ref() else {
                    return;
                };
                let Some(paint) = properties_panel::parse_hex_paint(&input.value) else {
                    if let Some(input) = &mut self.property_color_input {
                        input.invalid = true;
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                };
                let scope = input.scope;
                let target = input.target;
                self.property_color_input = None;
                let changed = apply_color_input(&mut self.editor, scope, target, paint);
                if changed {
                    self.current_cursor = convert_cursor(self.editor.cursor());
                }
                cx.notify();
            }
            "backspace" | "delete" => {
                if let Some(input) = &mut self.property_color_input {
                    if input.replace_on_type {
                        input.value.clear();
                    } else {
                        input.value.pop();
                    }
                    input.replace_on_type = false;
                    input.invalid = false;
                    cx.notify();
                }
            }
            "a" if primary => {
                if let Some(input) = &mut self.property_color_input {
                    input.replace_on_type = true;
                    input.invalid = false;
                    cx.notify();
                }
            }
            "v" if primary => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    if let Some(input) = &mut self.property_color_input {
                        input.value = sanitize_hex_input(&text);
                        input.replace_on_type = false;
                        input.invalid = false;
                        cx.notify();
                    }
                }
            }
            _ if !primary
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt =>
            {
                let Some(text) = event.keystroke.key_char.as_deref() else {
                    cx.stop_propagation();
                    return;
                };
                if let Some(input) = &mut self.property_color_input {
                    if input.replace_on_type {
                        input.value.clear();
                        input.replace_on_type = false;
                    }
                    append_hex_input(&mut input.value, text);
                    input.invalid = false;
                    cx.notify();
                }
            }
            _ => {}
        }
        cx.stop_propagation();
    }

    fn handle_zoom_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let primary = if cfg!(target_os = "macos") {
            event.keystroke.modifiers.platform
        } else {
            event.keystroke.modifiers.control
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.zoom_input = None;
                cx.notify();
            }
            "enter" => {
                let Some(input) = self.zoom_input.as_ref() else {
                    return;
                };
                let Some(percentage) = parse_zoom_percentage(&input.value) else {
                    if let Some(input) = &mut self.zoom_input {
                        input.invalid = true;
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                };
                self.zoom_input = None;
                self.editor.set_zoom(percentage / 100.0);
                cx.notify();
            }
            "backspace" | "delete" => {
                if let Some(input) = &mut self.zoom_input {
                    if input.replace_on_type {
                        input.value.clear();
                    } else {
                        input.value.pop();
                    }
                    input.replace_on_type = false;
                    input.invalid = false;
                    cx.notify();
                }
            }
            "a" if primary => {
                if let Some(input) = &mut self.zoom_input {
                    input.replace_on_type = true;
                    input.invalid = false;
                    cx.notify();
                }
            }
            "v" if primary => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    if let Some(input) = &mut self.zoom_input {
                        input.value = sanitize_zoom_input(&text);
                        input.replace_on_type = false;
                        input.invalid = false;
                        cx.notify();
                    }
                }
            }
            _ if !primary
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.alt =>
            {
                let Some(text) = event.keystroke.key_char.as_deref() else {
                    cx.stop_propagation();
                    return;
                };
                if let Some(input) = &mut self.zoom_input {
                    if input.replace_on_type {
                        input.value.clear();
                        input.replace_on_type = false;
                    }
                    append_zoom_input(&mut input.value, text);
                    input.invalid = false;
                    cx.notify();
                }
            }
            _ => {}
        }
        cx.stop_propagation();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.file_operation == FileOperation::Opening {
            return;
        }
        if self.zoom_input.is_some() {
            self.handle_zoom_key(event, cx);
            return;
        }
        if self.property_color_input.is_some() {
            self.handle_property_color_key(event, cx);
            return;
        }
        if self.command_palette.is_some()
            || self.open_menu.is_some()
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
        if self.file_operation == FileOperation::Opening {
            return;
        }
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

impl Focusable for Strek {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Strek {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.did_focus {
            self.focus_handle.focus(window);
            self.did_focus = true;
        }
        self.update_window_title(window);
        let tool = self.editor.tool;
        let interaction = self.editor.interaction_kind();
        let key_context = editor_key_context(
            self.file_operation == FileOperation::Opening,
            self.command_palette.is_some()
                || self.layer_name_input.is_some()
                || self.property_color_input.is_some()
                || self.zoom_input.is_some(),
            self.open_menu.is_some() || self.layer_context_menu.is_some(),
            self.editor.text_input_snapshot().is_some(),
        );
        let selection_count = self.editor.selection().len();
        let zoom = self.editor.view().zoom;
        let show_layers_panel = self.show_layers_panel;
        let show_design_panel = self.show_design_panel;
        let layers_panel_width = self.layers_panel_width;
        let design_panel_width = self.design_panel_width;
        let panel_visibility = toolbar::PanelVisibility {
            layers: show_layers_panel,
            design: show_design_panel,
        };
        let layer_name_input = self.layer_name_input.clone();
        let layer_context_menu = self.layer_context_menu;
        let command_palette = self.command_palette.clone();
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
            Some(toolbar::MenuKind::Main) => {
                cx.read_from_clipboard().as_ref().is_some_and(|item| {
                    self.object_clipboard
                        .as_ref()
                        .is_some_and(|clipboard| clipboard.owns(item))
                })
            }
            _ => false,
        };

        let window_size = window.viewport_size();
        self.editor.set_viewport_size(glam::Vec2::new(
            canvas_viewport_width(
                window_size.width.0,
                show_layers_panel,
                layers_panel_width,
                show_design_panel,
                design_panel_width,
            ),
            (window_size.height.0 - toolbar::HEADER_HEIGHT - status_bar::HEIGHT).max(1.0),
        ));

        let text_layouts =
            canvas::shape_text_layouts(self.editor.document().text_items_for_layout(), window);
        self.editor.set_text_layouts(text_layouts);
        let color_input = self
            .property_color_input
            .as_ref()
            .filter(|input| input.scope == ColorInputScope::Selection)
            .map(|input| properties_panel::ColorInputSnapshot {
                target: input.target,
                value: input.value.clone(),
                invalid: input.invalid,
            });
        let creation_color_input = self
            .property_color_input
            .as_ref()
            .filter(|input| input.scope == ColorInputScope::Creation)
            .map(|input| toolbar::CreationColorInputSnapshot {
                target: input.target,
                value: input.value.clone(),
                invalid: input.invalid,
            });
        let properties_snapshot = properties_panel::snapshot(&mut self.editor, color_input);
        let zoom_input = self
            .zoom_input
            .as_ref()
            .map(|input| toolbar::ZoomInputSnapshot {
                value: input.value.as_str(),
                invalid: input.invalid,
            });

        // Build display list for canvas
        let display_list = self.editor.build_display_list();

        // Build layer entries for panel
        let layer_entries = self.editor.build_layer_tree();
        let canvas_child_index = usize::from(show_layers_panel);
        let editor_entity = cx.entity().downgrade();
        let canvas_bounds_entity = editor_entity.clone();

        div()
            .id("strek")
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::new_document))
            .on_action(cx.listener(Self::open_document))
            .on_action(cx.listener(Self::save_document))
            .on_action(cx.listener(Self::save_document_as))
            .on_action(cx.listener(Self::export_svg))
            .on_action(cx.listener(Self::export_svg_outlined))
            .on_action(cx.listener(Self::export_png))
            .on_action(cx.listener(Self::export_jpeg))
            .on_action(cx.listener(Self::export_webp))
            .on_action(cx.listener(Self::open_keyboard_shortcuts))
            .on_action(cx.listener(Self::show_command_palette))
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
            .on_action(cx.listener(Self::start_zoom_input))
            .on_action(cx.listener(Self::toggle_layer_panel))
            .on_action(cx.listener(Self::toggle_design_panel))
            .on_action(cx.listener(Self::toggle_main_menu))
            .on_action(cx.listener(Self::toggle_zoom_menu))
            .on_action(cx.listener(Self::select_tool))
            .on_action(cx.listener(Self::frame_tool))
            .on_action(cx.listener(Self::rectangle_tool))
            .on_action(cx.listener(Self::ellipse_tool))
            .on_action(cx.listener(Self::line_tool))
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
            .on_action(cx.listener(Self::set_text_family_system))
            .on_action(cx.listener(Self::set_text_family_serif))
            .on_action(cx.listener(Self::set_text_family_monospace))
            .on_action(cx.listener(Self::text_weight_down))
            .on_action(cx.listener(Self::text_weight_up))
            .on_action(cx.listener(Self::toggle_text_italic))
            .on_action(cx.listener(Self::align_text_left))
            .on_action(cx.listener(Self::align_text_center))
            .on_action(cx.listener(Self::align_text_right))
            .on_action(cx.listener(Self::toggle_frame_background))
            .on_action(cx.listener(Self::set_layout_free))
            .on_action(cx.listener(Self::set_layout_horizontal))
            .on_action(cx.listener(Self::set_layout_vertical))
            .on_action(cx.listener(Self::step_layout_spacing_down))
            .on_action(cx.listener(Self::step_layout_spacing_up))
            .on_action(cx.listener(Self::step_layout_padding_down))
            .on_action(cx.listener(Self::step_layout_padding_up))
            .on_action(cx.listener(Self::property_move_left))
            .on_action(cx.listener(Self::property_move_right))
            .on_action(cx.listener(Self::property_move_up))
            .on_action(cx.listener(Self::property_move_down))
            .on_action(cx.listener(Self::property_rotate_left))
            .on_action(cx.listener(Self::property_rotate_right))
            .on_action(cx.listener(Self::property_opacity_down))
            .on_action(cx.listener(Self::property_opacity_up))
            .on_action(cx.listener(Self::property_fill_none))
            .on_action(cx.listener(Self::property_fill_white))
            .on_action(cx.listener(Self::property_fill_black))
            .on_action(cx.listener(Self::property_fill_blue))
            .on_action(cx.listener(Self::property_fill_red))
            .on_action(cx.listener(Self::property_fill_green))
            .on_action(cx.listener(Self::start_fill_color_input))
            .on_action(cx.listener(Self::start_stroke_color_input))
            .on_action(cx.listener(Self::property_toggle_stroke))
            .on_action(cx.listener(Self::property_stroke_down))
            .on_action(cx.listener(Self::property_stroke_up))
            .on_action(cx.listener(Self::toggle_creation_fill))
            .on_action(cx.listener(Self::start_creation_fill_color_input))
            .on_action(cx.listener(Self::toggle_creation_stroke))
            .on_action(cx.listener(Self::start_creation_stroke_color_input))
            .on_action(cx.listener(Self::creation_stroke_down))
            .on_action(cx.listener(Self::creation_stroke_up))
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
            .on_action(cx.listener(Self::select_parent))
            .on_action(cx.listener(Self::escape))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_mouse_move(cx.listener(Self::preview_numeric_property_scrub))
            .capture_any_mouse_up(cx.listener(Self::finish_numeric_property_scrub))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(Self::finish_numeric_property_scrub),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::dismiss_inline_input_from_mouse),
            )
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
                toolbar::ZoomState {
                    level: zoom,
                    input: zoom_input,
                },
                panel_visibility,
                toolbar::HistoryAvailability {
                    undo: self.editor.can_undo_in_context(),
                    redo: self.editor.can_redo_in_context(),
                },
                open_menu,
                &self.keymap,
            ))
            // Main content area
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .on_drag_move(cx.listener(Self::resize_panel))
                    .on_children_prepainted(move |children_bounds, _window, cx| {
                        let Some(bounds) = children_bounds.get(canvas_child_index).copied() else {
                            return;
                        };
                        canvas_bounds_entity
                            .update(cx, |editor, _| {
                                editor.canvas_input_bounds = Some(bounds);
                            })
                            .ok();
                    })
                    // Layers panel (conditionally shown)
                    .when(show_layers_panel, |this| {
                        this.child(layer_panel::render_layers_panel(
                            layer_entries,
                            layer_name_input,
                            layers_panel_width,
                            cx,
                        ))
                    })
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
                            .when_some(
                                toolbar::render_context_bar(
                                    &self.editor,
                                    &self.keymap,
                                    creation_color_input,
                                ),
                                |canvas, bar| canvas.child(bar),
                            ),
                    )
                    // Design panel (conditionally shown)
                    .when(show_design_panel, |this| {
                        this.child(layer_panel::render_design_panel(
                            properties_snapshot,
                            design_panel_width,
                            editor_entity.clone(),
                        ))
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
                        panels: panel_visibility,
                        can_paste,
                        recent_files: &recent_files,
                        file_busy,
                        keymap: &self.keymap,
                    },
                    window_size.width.0,
                    cx,
                ))
            })
            .when_some(layer_context_menu, |root, menu| {
                root.child(layer_panel::render_layer_context_menu(
                    menu,
                    &self.editor,
                    &self.keymap,
                    cx,
                ))
            })
            .when_some(command_palette, |root, palette| root.child(palette))
    }
}

fn visible_panel_width(visible: bool, width: f32) -> f32 {
    if visible {
        width
    } else {
        0.0
    }
}

fn bounded_automation_layer_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> (Vec<String>, bool) {
    let mut names = names.into_iter();
    let mut selected = Vec::new();
    let mut remaining_bytes = MAX_AUTOMATION_SELECTED_LAYER_BYTES;
    let mut truncated = false;

    while selected.len() < MAX_AUTOMATION_SELECTED_LAYERS {
        let Some(name) = names.next() else {
            return (selected, truncated);
        };
        if remaining_bytes == 0 {
            truncated = true;
            break;
        }

        let mut end = name.len().min(remaining_bytes);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        selected.push(name[..end].to_owned());
        remaining_bytes -= end;
        if end != name.len() {
            truncated = true;
            break;
        }
    }

    if names.next().is_some() {
        truncated = true;
    }
    (selected, truncated)
}

fn canvas_bounds_for_layout(
    window_size: gpui::Size<gpui::Pixels>,
    show_layers_panel: bool,
    layers_panel_width: f32,
    show_design_panel: bool,
    design_panel_width: f32,
) -> Bounds<gpui::Pixels> {
    Bounds::new(
        gpui::point(
            px(visible_panel_width(show_layers_panel, layers_panel_width)),
            px(toolbar::HEADER_HEIGHT),
        ),
        size(
            px(canvas_viewport_width(
                window_size.width.0,
                show_layers_panel,
                layers_panel_width,
                show_design_panel,
                design_panel_width,
            )),
            px((window_size.height.0 - toolbar::HEADER_HEIGHT - status_bar::HEIGHT).max(1.0)),
        ),
    )
}

fn canvas_position(
    position: gpui::Point<gpui::Pixels>,
    canvas_bounds: Option<gpui::Bounds<gpui::Pixels>>,
    fallback_left: f32,
) -> glam::Vec2 {
    let origin = canvas_bounds.map_or_else(
        || gpui::point(px(fallback_left), px(toolbar::HEADER_HEIGHT)),
        |bounds| bounds.origin,
    );
    glam::Vec2::new(position.x.0 - origin.x.0, position.y.0 - origin.y.0)
}

fn editor_key_context(
    file_opening: bool,
    has_modal: bool,
    has_menu: bool,
    editing_text: bool,
) -> &'static str {
    if file_opening {
        "StrekFileOpening"
    } else if has_modal {
        "StrekModal"
    } else if has_menu {
        "StrekMenu"
    } else if editing_text {
        "StrekTextEditor"
    } else {
        "Strek"
    }
}

fn editor_preserving_creation_styles(current: &Editor, mut replacement: Editor) -> Editor {
    replacement.set_creation_style_defaults(current.creation_style_defaults().clone());
    replacement
}

fn color_input_style(editor: &Editor, scope: ColorInputScope) -> Option<&editor_core::Style> {
    match scope {
        ColorInputScope::Selection => editor
            .selection()
            .primary()
            .and_then(|id| editor.document().get(id))
            .map(|node| &node.style),
        ColorInputScope::Creation => editor.active_creation_style(),
    }
}

fn apply_color_input(
    editor: &mut Editor,
    scope: ColorInputScope,
    target: properties_panel::ColorTarget,
    paint: editor_core::Paint,
) -> bool {
    match (scope, target) {
        (ColorInputScope::Selection, properties_panel::ColorTarget::Fill) => {
            editor.set_selected_fill(Some(paint))
        }
        (ColorInputScope::Selection, properties_panel::ColorTarget::Stroke) => {
            editor.set_selected_stroke_paint(paint)
        }
        (ColorInputScope::Creation, properties_panel::ColorTarget::Fill) => {
            editor.set_creation_fill(Some(paint))
        }
        (ColorInputScope::Creation, properties_panel::ColorTarget::Stroke) => {
            editor.set_creation_stroke_paint(paint)
        }
    }
}

fn sanitize_hex_input(value: &str) -> String {
    let mut sanitized = String::with_capacity(9);
    append_hex_input(&mut sanitized, value);
    sanitized
}

fn append_hex_input(value: &mut String, text: &str) {
    for character in text.chars() {
        if character == '#' && value.is_empty() {
            value.push(character);
        } else if character.is_ascii_hexdigit()
            && value.chars().filter(|character| *character != '#').count() < 8
        {
            value.push(character.to_ascii_uppercase());
        }
    }
}

fn parse_zoom_percentage(value: &str) -> Option<f32> {
    let percentage: f32 = value.trim().trim_end_matches('%').trim().parse().ok()?;
    (percentage.is_finite() && percentage > 0.0).then_some(percentage)
}

fn sanitize_zoom_input(value: &str) -> String {
    let mut sanitized = String::with_capacity(8);
    append_zoom_input(&mut sanitized, value);
    sanitized
}

fn append_zoom_input(value: &mut String, text: &str) {
    for character in text.chars() {
        if character.is_ascii_digit() && value.len() < 8 {
            value.push(character);
        } else if matches!(character, '.' | ',') && !value.contains('.') && value.len() < 8 {
            value.push('.');
        }
    }
}

fn canvas_viewport_width(
    window_width: f32,
    show_layers_panel: bool,
    layers_panel_width: f32,
    show_design_panel: bool,
    design_panel_width: f32,
) -> f32 {
    let panels_width = visible_panel_width(show_layers_panel, layers_panel_width)
        + visible_panel_width(show_design_panel, design_panel_width);
    (window_width - panels_width).max(1.0)
}

fn clamp_panel_width(requested: f32, opposite_width: f32, window_width: f32) -> f32 {
    let available_width = window_width - opposite_width - layer_panel::MIN_CANVAS_WIDTH;
    let maximum_width =
        available_width.clamp(layer_panel::MIN_PANEL_WIDTH, layer_panel::MAX_PANEL_WIDTH);
    requested
        .round()
        .clamp(layer_panel::MIN_PANEL_WIDTH, maximum_width)
}

fn automation_bounds(bounds: Bounds<gpui::Pixels>) -> automation::AutomationBounds {
    automation::AutomationBounds {
        x: bounds.origin.x.0,
        y: bounds.origin.y.0,
        width: bounds.size.width.0,
        height: bounds.size.height.0,
    }
}

fn automation_tool_name(tool: editor_core::Tool) -> &'static str {
    match tool {
        editor_core::Tool::Select => "select",
        editor_core::Tool::Frame => "frame",
        editor_core::Tool::Rectangle => "rectangle",
        editor_core::Tool::Ellipse => "ellipse",
        editor_core::Tool::Line => "line",
        editor_core::Tool::Pen => "pen",
        editor_core::Tool::Text => "text",
        editor_core::Tool::VectorEdit => "vector_edit",
    }
}

fn automation_interaction_name(interaction: editor_core::InteractionKind) -> &'static str {
    match interaction {
        editor_core::InteractionKind::Idle => "idle",
        editor_core::InteractionKind::Moving => "moving",
        editor_core::InteractionKind::Resizing => "resizing",
        editor_core::InteractionKind::Rotating => "rotating",
        editor_core::InteractionKind::Marquee => "marquee",
        editor_core::InteractionKind::CreatingShape => "creating_shape",
        editor_core::InteractionKind::CreatingFrame => "creating_frame",
        editor_core::InteractionKind::Pen => "pen",
        editor_core::InteractionKind::CreatingText => "creating_text",
        editor_core::InteractionKind::TextEditing => "text_editing",
        editor_core::InteractionKind::VectorEditing => "vector_editing",
        editor_core::InteractionKind::Panning => "panning",
    }
}

fn automation_mouse_button(button: automation::PointerButton) -> editor_core::MouseButton {
    match button {
        automation::PointerButton::Left => editor_core::MouseButton::Left,
        automation::PointerButton::Middle => editor_core::MouseButton::Middle,
        automation::PointerButton::Right => editor_core::MouseButton::Right,
    }
}

fn automation_modifiers(modifiers: automation::AutomationModifiers) -> editor_core::Modifiers {
    editor_core::Modifiers {
        shift: modifiers.shift,
        ctrl: modifiers.control,
        alt: modifiers.alt,
        meta: modifiers.command,
    }
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

fn register_keybindings(cx: &mut App, keymap: &commands::Keymap) {
    commands::register_keybindings(cx, keymap);
    cx.bind_keys([
        KeyBinding::new("enter", Enter, Some("Strek")),
        KeyBinding::new("shift-enter", SelectParent, Some("Strek")),
        KeyBinding::new("escape", Escape, Some("Strek")),
        KeyBinding::new("left", TextLeft, Some("StrekTextEditor")),
        KeyBinding::new("right", TextRight, Some("StrekTextEditor")),
        KeyBinding::new("shift-left", TextSelectLeft, Some("StrekTextEditor")),
        KeyBinding::new("shift-right", TextSelectRight, Some("StrekTextEditor")),
        KeyBinding::new("home", TextHome, Some("StrekTextEditor")),
        KeyBinding::new("end", TextEnd, Some("StrekTextEditor")),
        KeyBinding::new("enter", Enter, Some("StrekTextEditor")),
        KeyBinding::new("escape", Escape, Some("StrekTextEditor")),
        KeyBinding::new("escape", Escape, Some("StrekMenu")),
    ]);
}

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("automate") => {
            if let Err(error) = automation::run_cli(args) {
                eprintln!("strek automate: {error}");
                std::process::exit(1);
            }
            return;
        }
        Some("mcp") => {
            if let Err(error) = mcp::run() {
                eprintln!("strek mcp: {error}");
                std::process::exit(1);
            }
            return;
        }
        _ => {}
    }

    env_logger::init();
    let automation_requests = match automation::start_server() {
        Ok(requests) => Some(requests),
        Err(error) => {
            log::warn!(
                "automation unavailable at {}: {error}",
                automation::socket_path_display()
            );
            None
        }
    };

    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            let keymap = commands::Keymap::load();
            register_keybindings(cx, &keymap);
            command_palette::register_keybindings(cx);
            layer_name_input::register_keybindings(cx);
            cx.set_menus(vec![
                Menu {
                    name: "Strek".into(),
                    items: vec![
                        MenuItem::action("Keyboard Shortcuts…", OpenKeyboardShortcuts),
                        MenuItem::separator(),
                        MenuItem::action("Quit Strek", QuitApplication),
                    ],
                },
                Menu {
                    name: "File".into(),
                    items: vec![
                        MenuItem::action("New", NewDocument),
                        MenuItem::action("Open…", OpenDocument),
                        MenuItem::separator(),
                        MenuItem::action("Save", SaveDocument),
                        MenuItem::action("Save As…", SaveDocumentAs),
                        MenuItem::separator(),
                        MenuItem::action("Export SVG…", ExportSvg),
                        MenuItem::action("Export SVG with Outlined Text…", ExportSvgOutlined),
                        MenuItem::action("Export PNG…", ExportPng),
                        MenuItem::action("Export JPEG…", ExportJpeg),
                        MenuItem::action("Export WebP…", ExportWebP),
                    ],
                },
                Menu {
                    name: "View".into(),
                    items: vec![
                        MenuItem::action("Command Palette…", ShowCommandPalette),
                        MenuItem::separator(),
                        MenuItem::action("Toggle Layers Panel", ToggleLayerPanel),
                        MenuItem::action("Toggle Design Panel", ToggleDesignPanel),
                    ],
                },
            ]);

            let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);

            let mut automation_requests = automation_requests;
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                move |window, cx| {
                    let editor = cx.new(|editor_cx| Strek::new(keymap.clone(), editor_cx));
                    if let Some(requests) = automation_requests.take() {
                        editor.update(cx, |editor, cx| {
                            editor.start_automation(requests, window, cx)
                        });
                    }
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn canvas_input_uses_the_rendered_canvas_origin() {
        let bounds = gpui::Bounds::new(
            gpui::point(px(220.0), px(48.0)),
            gpui::size(px(800.0), px(700.0)),
        );
        let position = canvas_position(gpui::point(px(340.0), px(140.0)), Some(bounds), 999.0);

        assert_eq!(position, glam::Vec2::new(120.0, 92.0));
    }

    #[test]
    fn canvas_input_falls_back_to_the_current_layers_width() {
        let position = canvas_position(gpui::point(px(340.0), px(140.0)), None, 220.0);

        assert_eq!(position, glam::Vec2::new(120.0, 92.0));
    }

    #[test]
    fn viewport_width_accounts_for_each_visible_panel() {
        assert_eq!(
            canvas_viewport_width(1280.0, true, 240.0, true, 300.0),
            740.0
        );
        assert_eq!(
            canvas_viewport_width(1280.0, true, 240.0, false, 300.0),
            1040.0
        );
        assert_eq!(
            canvas_viewport_width(1280.0, false, 240.0, true, 300.0),
            980.0
        );
        assert_eq!(
            canvas_viewport_width(1280.0, false, 240.0, false, 300.0),
            1280.0
        );
    }

    #[test]
    fn predicted_canvas_bounds_settle_immediately_after_panel_changes() {
        let bounds =
            canvas_bounds_for_layout(gpui::size(px(1280.0), px(800.0)), true, 240.0, false, 300.0);

        assert_eq!(bounds.origin, gpui::point(px(240.0), px(48.0)));
        assert_eq!(bounds.size.width, px(1040.0));
        assert_eq!(
            bounds.size.height,
            px(800.0 - toolbar::HEADER_HEIGHT - status_bar::HEIGHT)
        );
    }

    #[test]
    fn panel_width_is_bounded_and_preserves_canvas_space() {
        assert_eq!(clamp_panel_width(90.0, 248.0, 1280.0), 180.0);
        assert_eq!(clamp_panel_width(900.0, 248.0, 1280.0), 420.0);
        assert_eq!(clamp_panel_width(420.0, 420.0, 920.0), 180.0);
    }

    #[test]
    fn modal_context_blocks_canvas_shortcuts_while_typing() {
        assert_eq!(
            editor_key_context(true, false, false, false),
            "StrekFileOpening"
        );
        assert_eq!(editor_key_context(false, true, false, false), "StrekModal");
        assert_eq!(editor_key_context(false, true, true, true), "StrekModal");
        assert_eq!(editor_key_context(false, false, true, true), "StrekMenu");
        assert_eq!(
            editor_key_context(false, false, false, true),
            "StrekTextEditor"
        );
        assert_eq!(editor_key_context(false, false, false, false), "Strek");
    }

    #[test]
    fn object_clipboard_requires_its_exact_ownership_token() {
        let clipboard = ObjectClipboard::new(editor_core::EditorClipboard::default());
        let owned =
            ClipboardItem::new_string_with_metadata("Layer".to_owned(), clipboard.metadata.clone());
        let stale = ClipboardItem::new_string_with_metadata(
            "Other layer".to_owned(),
            format!("{OBJECT_CLIPBOARD_METADATA}:other-process:1"),
        );
        let external = ClipboardItem::new_string("Layer".to_owned());

        assert!(clipboard.owns(&owned));
        assert!(!clipboard.owns(&stale));
        assert!(!clipboard.owns(&external));
    }

    #[test]
    fn automation_layer_names_are_bounded_without_splitting_utf8() {
        let oversized = format!("{}é", "a".repeat(MAX_AUTOMATION_SELECTED_LAYER_BYTES - 1));
        let (names, truncated) =
            bounded_automation_layer_names([oversized.as_str(), "not included"]);

        assert!(truncated);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].len(), MAX_AUTOMATION_SELECTED_LAYER_BYTES - 1);
        assert!(names[0].is_char_boundary(names[0].len()));
    }

    #[test]
    fn color_input_keeps_one_prefix_and_eight_hex_digits() {
        assert_eq!(sanitize_hex_input("  #12ab34cd trailing"), "#12AB34CD");
        assert_eq!(sanitize_hex_input("##ff00aa"), "#FF00AA");

        let mut value = "#12".to_owned();
        append_hex_input(&mut value, "g3f");
        assert_eq!(value, "#123F");
    }

    #[test]
    fn creation_color_scope_does_not_mutate_the_lingering_selection() {
        let mut editor = Editor::new();
        let selected = editor
            .document
            .add_child(
                editor.document.root,
                editor_core::Node::shape(
                    "Selected",
                    editor_core::PathData::rect(0.0, 0.0, 10.0, 10.0),
                ),
            )
            .unwrap();
        editor.selection.select(selected);
        assert!(editor.execute_action(EditorAction::ToolRectangle));
        let selected_before = editor.document.get(selected).unwrap().style.clone();

        assert!(apply_color_input(
            &mut editor,
            ColorInputScope::Creation,
            properties_panel::ColorTarget::Fill,
            editor_core::Paint::rgb(0.9, 0.1, 0.2),
        ));

        assert_eq!(
            editor.document.get(selected).unwrap().style,
            selected_before
        );
        assert_eq!(
            editor.active_creation_style().unwrap().fill,
            Some(editor_core::Paint::rgb(0.9, 0.1, 0.2))
        );
        assert!(!editor.history.can_undo());
    }

    #[test]
    fn replacing_a_document_preserves_creation_styles_only_as_session_state() {
        let mut current = Editor::new();
        assert!(current.execute_action(EditorAction::ToolPen));
        assert!(current.set_creation_stroke_paint(editor_core::Paint::rgb(0.2, 0.5, 0.9)));
        let expected = current.creation_style_defaults().clone();

        let document = editor_core::Document::new();
        let root = document.root;
        let mut replacement =
            editor_preserving_creation_styles(&current, Editor::from_document(document));

        assert_eq!(replacement.document.root, root);
        assert_eq!(replacement.creation_style_defaults(), &expected);
        assert!(!replacement.history.can_undo());
        assert!(!replacement.is_dirty());
        assert!(replacement.execute_action(EditorAction::ToolPen));
        assert_eq!(
            replacement
                .active_creation_style()
                .unwrap()
                .stroke
                .as_ref()
                .unwrap()
                .paint,
            editor_core::Paint::rgb(0.2, 0.5, 0.9)
        );
    }

    #[test]
    fn zoom_input_accepts_decimal_percentages_and_ignores_decoration() {
        assert_eq!(sanitize_zoom_input(" 125.5% "), "125.5");
        assert_eq!(sanitize_zoom_input("12,25x"), "12.25");
        assert_eq!(parse_zoom_percentage(" 125.5% "), Some(125.5));
        assert_eq!(parse_zoom_percentage("0"), None);
        assert_eq!(parse_zoom_percentage("nan"), None);
    }
}
