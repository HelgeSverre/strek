//! GPUI-based desktop application for the vector editor.
//!
//! Uses Zed's GPUI framework for high-performance GPU-accelerated rendering.

mod assets;
mod automation;
mod automation_controller;
mod automation_io;
mod canvas;
mod color_library_panel;
mod color_picker;
mod command_palette;
mod commands;
mod document_io;
mod export;
mod layer_name_input;
mod layer_panel;
mod mcp;
mod precision_ui;
mod properties_panel;
mod status_bar;
mod svg_import;
mod text_input;
mod toolbar;
mod typography;
mod workspace_preferences;

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use editor_core::{
    Editor, EditorAction, NodeId, NodeKind, NumericAdjustmentDirection, NumericAdjustmentMode,
    NumericPropertyScrubSession, NumericPropertyTarget,
};
use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Application, Bounds, ClipboardItem, Context,
    DispatchPhase, DragMoveEvent, Entity, FocusHandle, Focusable, Image, ImageFormat, KeyBinding,
    KeyDownEvent, KeyUpEvent, Menu, MenuItem, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathPromptOptions, PromptLevel, ScrollWheelEvent, Window,
    WindowBounds, WindowOptions,
};

const OBJECT_CLIPBOARD_METADATA: &str = "strek-object-clipboard-v1";
const MAX_AUTOMATION_SELECTED_LAYERS: usize = 10_000;
const MAX_AUTOMATION_SELECTED_LAYER_BYTES: usize = 256 * 1024;
const MAX_AUTOMATION_DOCUMENT_LAYERS: usize = 10_000;
const MAX_AUTOMATION_QUERY_LAYERS: usize = 512;
const MAX_AUTOMATION_DOCUMENT_BYTES: usize = 3 * 1024 * 1024;
const MAX_INLINE_AUTOMATION_ARTIFACT_BYTES: usize = 2 * 1024 * 1024;
const WORKSPACE_PREFERENCES_DEBOUNCE: Duration = Duration::from_millis(250);
const RECENT_FILES_DEBOUNCE: Duration = Duration::from_millis(50);
const ARTWORK_RASTER_DEBOUNCE: Duration = Duration::from_millis(50);
const AUTOMATION_DEDUPLICATION_LIMIT: usize = 256;
static OBJECT_CLIPBOARD_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static ARTWORK_CLIPBOARD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
        CopyAsSvg,
        CopyAsPng,
        CopyAsWebP,
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
        ToggleColorLibrary,
        TogglePrecisionMenu,
        ToggleRulers,
        ToggleGrid,
        ToggleGuides,
        ToggleGuideLock,
        ToggleSnapping,
        ToggleSnapObjects,
        ToggleSnapGuides,
        ToggleSnapGrid,
        DecreaseSnapTolerance,
        IncreaseSnapTolerance,
        DecreaseGridSpacing,
        IncreaseGridSpacing,
        DecreaseGridMajorInterval,
        IncreaseGridMajorInterval,
        ClearGuides,
        SaveCurrentColor,
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
        StartFrameBackgroundColorInput,
        SetLayoutFree,
        SetLayoutHorizontal,
        SetLayoutVertical,
        PropertyRotateLeft,
        PropertyRotateRight,
        PropertyFillNone,
        PropertyFillWhite,
        PropertyFillBlack,
        PropertyFillBlue,
        PropertyFillRed,
        PropertyFillGreen,
        StartFillColorInput,
        StartStrokeColorInput,
        PropertyToggleStroke,
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

#[derive(Debug, Clone, Default)]
enum DocumentOrigin {
    #[default]
    Untitled,
    Starter,
    Native(PathBuf),
    ImportedSvg(PathBuf),
}

impl DocumentOrigin {
    fn source_path(&self) -> Option<&Path> {
        match self {
            Self::Untitled | Self::Starter => None,
            Self::Native(path) | Self::ImportedSvg(path) => Some(path),
        }
    }

    fn native_path(&self) -> Option<&Path> {
        match self {
            Self::Native(path) => Some(path),
            Self::Untitled | Self::Starter | Self::ImportedSvg(_) => None,
        }
    }

    fn import_source_path(&self) -> Option<&Path> {
        match self {
            Self::ImportedSvg(path) => Some(path),
            Self::Untitled | Self::Starter | Self::Native(_) => None,
        }
    }

    fn requires_native_save(&self) -> bool {
        matches!(self, Self::ImportedSvg(_))
    }
}

#[derive(Debug, Clone, Copy)]
enum ClipboardArtworkFormat {
    Svg,
    Png,
    WebP,
}

impl ClipboardArtworkFormat {
    fn is_supported(self) -> bool {
        match self {
            Self::Svg | Self::Png => cfg!(any(target_os = "macos", target_os = "windows")),
            Self::WebP => cfg!(target_os = "macos"),
        }
    }

    fn export_format(self) -> export::ExportFormat {
        match self {
            Self::Svg => export::ExportFormat::Svg,
            Self::Png => export::ExportFormat::Png,
            Self::WebP => export::ExportFormat::WebP,
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Self::Svg => ImageFormat::Svg,
            Self::Png => ImageFormat::Png,
            Self::WebP => ImageFormat::Webp,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FileOperation {
    #[default]
    Idle,
    Prompting,
    Opening,
    Saving,
    Exporting,
    Copying,
}

enum AutomationDispatch {
    Immediate(Box<automation::AutomationResponse>),
    Background(gpui::Task<Result<automation_io::AutomationIoSuccess, String>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateWriteCloseAction {
    CloseNow,
    WaitForFlush,
    StartFlush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorInputScope {
    Selection,
    Creation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPolicy {
    Preserve,
    Request,
}

impl FocusPolicy {
    fn requests_focus(self) -> bool {
        self == Self::Request
    }

    fn apply(self, focus_handle: &FocusHandle, window: &mut Window) {
        if self.requests_focus() {
            focus_handle.focus(window);
        }
    }
}

#[derive(Debug, Clone)]
struct PropertyColorInput {
    scope: ColorInputScope,
    target: properties_panel::ColorTarget,
    picker: color_picker::ColorPickerState,
    save_feedback: Option<color_picker::SavedColorFeedback>,
}

impl PropertyColorInput {
    fn matches(&self, scope: ColorInputScope, target: properties_panel::ColorTarget) -> bool {
        self.scope == scope && self.target == target
    }
}

#[derive(Debug, Clone, Copy)]
struct QuickAddLibraryColorResult {
    id: editor_core::SavedColorId,
    group_id: Option<editor_core::ColorGroupId>,
    already_existed: bool,
}

#[derive(Debug, Clone)]
struct ZoomInput {
    value: String,
    replace_on_type: bool,
    invalid: bool,
}

#[derive(Debug, Clone)]
struct NumericPropertyInput {
    target: NumericPropertyTarget,
    value: String,
    replace_on_type: bool,
    invalid: bool,
}

#[derive(Debug, Clone)]
struct GuidePositionInput {
    id: editor_core::GuideId,
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

struct CachedArtworkSnapshot {
    key: canvas::ArtworkCacheKey,
    snapshot: Option<Arc<editor_core::ArtworkSnapshot>>,
    requires_raster: bool,
}

type ArtworkRasterIdentity = canvas::ArtworkRasterIdentity;

#[derive(Debug, Clone, Copy, PartialEq)]
struct PendingArtworkRaster {
    identity: ArtworkRasterIdentity,
    interactive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtworkRasterRequestAction {
    Ready,
    Wait,
    Start,
    Replace,
}

#[derive(Debug, Default)]
struct ArtworkRasterScheduler {
    desired: Option<ArtworkRasterIdentity>,
    pending: Option<PendingArtworkRaster>,
    failed: Option<ArtworkRasterIdentity>,
}

impl ArtworkRasterScheduler {
    fn request(
        &mut self,
        identity: ArtworkRasterIdentity,
        interactive: bool,
        cached: bool,
    ) -> ArtworkRasterRequestAction {
        self.desired = Some(identity);
        if cached {
            self.pending = None;
            self.failed = None;
            return ArtworkRasterRequestAction::Ready;
        }
        if self.failed == Some(identity) {
            return ArtworkRasterRequestAction::Wait;
        }
        if let Some(pending) = self.pending {
            if pending.identity == identity {
                return ArtworkRasterRequestAction::Wait;
            }
            if pending.interactive {
                return ArtworkRasterRequestAction::Wait;
            }
            self.pending = Some(PendingArtworkRaster {
                identity,
                interactive,
            });
            self.failed = None;
            return ArtworkRasterRequestAction::Replace;
        }

        self.pending = Some(PendingArtworkRaster {
            identity,
            interactive,
        });
        self.failed = None;
        ArtworkRasterRequestAction::Start
    }

    fn complete(&mut self, identity: ArtworkRasterIdentity, succeeded: bool) -> bool {
        if self.pending.map(|pending| pending.identity) != Some(identity) {
            return false;
        }
        self.pending = None;
        if self.desired != Some(identity) {
            return false;
        }
        self.failed = (!succeeded).then_some(identity);
        true
    }

    fn fail_desired(&mut self, identity: ArtworkRasterIdentity) -> bool {
        if self.desired != Some(identity) {
            return false;
        }
        self.pending = None;
        self.failed = Some(identity);
        true
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Main application state as a GPUI Entity.
struct Strek {
    editor: Editor,
    document_origin: DocumentOrigin,
    recent_files: document_io::RecentFiles,
    recent_files_persist_task: Option<gpui::Task<()>>,
    state_write_flush_in_progress: bool,
    file_operation: FileOperation,
    allow_window_close: bool,
    object_clipboard: Option<ObjectClipboard>,
    keymap: commands::Keymap,
    recent_commands: Vec<commands::CommandTarget>,
    command_palette: Option<Entity<command_palette::CommandPalette>>,
    property_color_input: Option<PropertyColorInput>,
    zoom_input: Option<ZoomInput>,
    numeric_property_input: Option<NumericPropertyInput>,
    numeric_property_scrub: Option<ActiveNumericPropertyScrub>,
    focus_handle: FocusHandle,
    show_layers_panel: bool,
    show_design_panel: bool,
    workspace_preferences: workspace_preferences::WorkspacePreferences,
    workspace_preferences_persist_task: Option<gpui::Task<()>>,
    precision_menu_open: bool,
    color_library_open: bool,
    color_library_panel: Option<Entity<color_library_panel::ColorLibraryPanel>>,
    active_guide: Option<editor_core::GuideId>,
    guide_drag: Option<GuideDrag>,
    guide_position_input: Option<GuidePositionInput>,
    layers_panel_width: f32,
    design_panel_width: f32,
    layer_name_input: Option<(NodeId, Entity<layer_name_input::LayerNameInput>)>,
    layer_context_menu: Option<layer_panel::LayerContextMenu>,
    open_menu: Option<toolbar::MenuKind>,
    current_cursor: gpui::CursorStyle,
    canvas_input_bounds: Option<Bounds<gpui::Pixels>>,
    text_image_cache: canvas::TextImageCache,
    artwork_image_cache: canvas::ArtworkImageCache,
    artwork_snapshot_cache: Option<CachedArtworkSnapshot>,
    artwork_raster_task: Option<gpui::Task<()>>,
    artwork_raster_scheduler: ArtworkRasterScheduler,
    document_epoch: u64,
    automation_session_nonce: [u8; 16],
    automation_document_revision: Cell<u64>,
    automation_document_identity: Cell<(u64, u64)>,
    automation_workspace_revision: Cell<u64>,
    automation_workspace_fingerprint: Cell<u64>,
    automation_artwork_revision: Cell<u64>,
    automation_artwork_identity: Cell<(u64, u64, u64)>,
    automation_request_epoch: Cell<u64>,
    automation_next_request_sequence: Cell<u64>,
    automation_responses: HashMap<String, CachedAutomationResponse>,
    automation_response_order: VecDeque<String>,
    fit_artwork_on_first_layout: bool,
    did_focus: bool,
}

#[derive(Debug, Clone)]
struct CachedAutomationResponse {
    session_id: String,
    sequence: u64,
    request_id: String,
    response: automation::AutomationResponse,
}

impl Strek {
    fn new(keymap: commands::Keymap, cx: &mut Context<Self>) -> Self {
        let workspace_preferences = workspace_preferences::WorkspacePreferences::load();
        let mut editor = Editor::with_demo_content();
        editor
            .set_snap_settings(editor_core::SnapSettings {
                enabled: workspace_preferences.snapping_enabled,
                objects: workspace_preferences.snap_to_objects,
                guides: workspace_preferences.snap_to_guides,
                grid: workspace_preferences.snap_to_grid,
                tolerance: workspace_preferences.snap_tolerance,
            })
            .expect("sanitized workspace snapping preferences must be valid");
        let mut automation_session_nonce = [0_u8; 16];
        getrandom::fill(&mut automation_session_nonce)
            .expect("the operating system must provide automation session randomness");
        Self {
            editor,
            document_origin: DocumentOrigin::Starter,
            recent_files: document_io::RecentFiles::load(),
            recent_files_persist_task: None,
            state_write_flush_in_progress: false,
            file_operation: FileOperation::Idle,
            allow_window_close: false,
            object_clipboard: None,
            keymap,
            recent_commands: Vec::new(),
            command_palette: None,
            property_color_input: None,
            zoom_input: None,
            numeric_property_input: None,
            numeric_property_scrub: None,
            focus_handle: cx.focus_handle(),
            show_layers_panel: true,
            show_design_panel: true,
            workspace_preferences,
            workspace_preferences_persist_task: None,
            precision_menu_open: false,
            color_library_open: false,
            color_library_panel: None,
            active_guide: None,
            guide_drag: None,
            guide_position_input: None,
            layers_panel_width: layer_panel::DEFAULT_PANEL_WIDTH,
            design_panel_width: layer_panel::DEFAULT_PANEL_WIDTH,
            layer_name_input: None,
            layer_context_menu: None,
            open_menu: None,
            current_cursor: gpui::CursorStyle::Arrow,
            canvas_input_bounds: None,
            text_image_cache: canvas::TextImageCache::default(),
            artwork_image_cache: canvas::ArtworkImageCache::default(),
            artwork_snapshot_cache: None,
            artwork_raster_task: None,
            artwork_raster_scheduler: ArtworkRasterScheduler::default(),
            document_epoch: 0,
            automation_session_nonce,
            automation_document_revision: Cell::new(0),
            automation_document_identity: Cell::new((0, 0)),
            automation_workspace_revision: Cell::new(0),
            automation_workspace_fingerprint: Cell::new(0),
            automation_artwork_revision: Cell::new(0),
            automation_artwork_identity: Cell::new((0, 0, 0)),
            automation_request_epoch: Cell::new(0),
            automation_next_request_sequence: Cell::new(0),
            automation_responses: HashMap::new(),
            automation_response_order: VecDeque::new(),
            fit_artwork_on_first_layout: true,
            did_focus: false,
        }
    }

    fn artwork_cache_key(&self) -> canvas::ArtworkCacheKey {
        canvas::ArtworkCacheKey {
            document_epoch: self.document_epoch,
            history_revision: self.editor.current_revision(),
            transient_revision: self.editor.transient_artwork_revision(),
        }
    }

    fn artwork_snapshot_for_render(
        &mut self,
    ) -> (
        canvas::ArtworkCacheKey,
        Option<Arc<editor_core::ArtworkSnapshot>>,
        bool,
    ) {
        let key = self.artwork_cache_key();
        let stale = self
            .artwork_snapshot_cache
            .as_ref()
            .is_none_or(|cached| cached.key != key);
        if stale {
            let snapshot = self.editor.artwork_snapshot().map(Arc::new);
            let requires_raster = snapshot.as_ref().is_some_and(|snapshot| {
                canvas::requires_rasterized_artwork(&snapshot.display_list)
            });
            self.artwork_snapshot_cache = Some(CachedArtworkSnapshot {
                key,
                snapshot,
                requires_raster,
            });
        }
        let cached = self
            .artwork_snapshot_cache
            .as_ref()
            .expect("the artwork snapshot cache was populated");
        (cached.key, cached.snapshot.clone(), cached.requires_raster)
    }

    fn ensure_artwork_raster(
        &mut self,
        request: canvas::ArtworkRasterRequest,
        interactive: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let identity = request.identity();
        let action = self.artwork_raster_scheduler.request(
            identity,
            interactive,
            self.artwork_image_cache.contains(&request),
        );
        match action {
            ArtworkRasterRequestAction::Ready => {
                self.artwork_raster_task.take();
                return;
            }
            ArtworkRasterRequestAction::Wait => return,
            ArtworkRasterRequestAction::Replace => {
                self.artwork_raster_task.take();
            }
            ArtworkRasterRequestAction::Start => {}
        }

        let background = cx.background_executor().clone();
        self.artwork_raster_task = Some(cx.spawn_in(window, async move |editor, cx| {
            if !interactive {
                gpui::Timer::after(ARTWORK_RASTER_DEBOUNCE).await;
            }
            let prepared = background
                .spawn(async move { canvas::prepare_artwork_raster(request) })
                .await;
            editor
                .update_in(cx, |editor, window, cx| {
                    if !editor
                        .artwork_raster_scheduler
                        .complete(identity, prepared.is_some())
                    {
                        cx.notify();
                        return;
                    }
                    if let Some(prepared) = prepared {
                        editor.artwork_image_cache.install(prepared);
                    } else {
                        editor.show_render_error(
                            "Could not render advanced artwork",
                            "The bounded canvas rasterizer could not prepare this artwork. The document is unchanged and can still be exported as SVG."
                                .to_owned(),
                            window,
                            cx,
                        );
                    }
                    cx.notify();
                })
                .ok();
        }));
    }

    fn clear_document_render_cache(&mut self) {
        self.text_image_cache.clear();
        self.artwork_image_cache.clear();
        self.document_epoch = self.document_epoch.wrapping_add(1);
        self.artwork_snapshot_cache = None;
        self.artwork_raster_scheduler.clear();
        self.artwork_raster_task.take();
    }

    fn document_name(&self) -> String {
        if matches!(self.document_origin, DocumentOrigin::Starter) {
            return "Strek Showcase".to_owned();
        }
        self.source_document_path()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy())
            .map(|name| {
                let lowercase = name.to_ascii_lowercase();
                for suffix in [".strek.json", ".json", ".svg"] {
                    if lowercase.ends_with(suffix) {
                        return name[..name.len() - suffix.len()].to_owned();
                    }
                }
                name.into_owned()
            })
            .unwrap_or_else(|| "Untitled".to_owned())
    }

    fn document_location(&self) -> String {
        self.source_document_path()
            .and_then(Path::parent)
            .map(|parent| parent.display().to_string())
            .unwrap_or_else(|| "Unsaved document".to_owned())
    }

    fn source_document_path(&self) -> Option<&Path> {
        self.document_origin.source_path()
    }

    fn document_is_dirty(&self) -> bool {
        self.document_origin.requires_native_save() || self.editor.is_dirty()
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
        let dirty_marker = if self.document_is_dirty() { " •" } else { "" };
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
        self.sync_snap_settings();
        self.document_origin = DocumentOrigin::Untitled;
        self.object_clipboard = None;
        self.command_palette = None;
        self.property_color_input = None;
        self.zoom_input = None;
        self.numeric_property_input = None;
        self.active_guide = None;
        self.guide_drag = None;
        self.guide_position_input = None;
        self.color_library_open = false;
        self.color_library_panel = None;
        self.layer_name_input = None;
        self.clear_document_render_cache();
        self.fit_artwork_on_first_layout = false;
        self.current_cursor = gpui::CursorStyle::Arrow;
        self.dismiss_menus();
    }

    fn replace_document(
        &mut self,
        document: editor_core::Document,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.install_document(document);
        self.document_origin = DocumentOrigin::Native(path.clone());
        self.record_recent_file(&path, cx);
    }

    fn replace_imported_document(
        &mut self,
        document: editor_core::Document,
        source: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.install_document(document);
        self.document_origin = DocumentOrigin::ImportedSvg(source.clone());
        self.record_recent_file(&source, cx);
    }

    fn install_document(&mut self, document: editor_core::Document) {
        self.cancel_numeric_property_scrub();
        self.editor =
            editor_preserving_creation_styles(&self.editor, Editor::from_document(document));
        self.sync_snap_settings();
        self.object_clipboard = None;
        self.command_palette = None;
        self.property_color_input = None;
        self.zoom_input = None;
        self.numeric_property_input = None;
        self.active_guide = None;
        self.guide_drag = None;
        self.guide_position_input = None;
        self.color_library_open = false;
        self.color_library_panel = None;
        self.layer_name_input = None;
        self.clear_document_render_cache();
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
        if let Some(path) = self.document_origin.native_path().map(Path::to_path_buf) {
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

    fn copy_as_svg(&mut self, _: &CopyAsSvg, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_copy_artwork(ClipboardArtworkFormat::Svg, window, cx);
    }

    fn copy_as_png(&mut self, _: &CopyAsPng, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_copy_artwork(ClipboardArtworkFormat::Png, window, cx);
    }

    fn copy_as_webp(&mut self, _: &CopyAsWebP, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_copy_artwork(ClipboardArtworkFormat::WebP, window, cx);
    }

    fn open_keyboard_shortcuts(
        &mut self,
        _: &OpenKeyboardShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_menus();
        match commands::Keymap::ensure_user_file().and_then(|path| commands::file_url(&path)) {
            Ok(url) => cx.open_url(&url),
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
        let visible = self.command_palette.is_none();
        self.set_command_palette_visible(visible, FocusPolicy::Request, window, cx);
    }

    fn set_command_palette_visible(
        &mut self,
        visible: bool,
        focus_policy: FocusPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_numeric_property_scrub();
        if !visible {
            if self.command_palette.take().is_some() {
                focus_policy.apply(&self.focus_handle, window);
                cx.notify();
            }
            return;
        }
        if let Some(palette) = &self.command_palette {
            if focus_policy.requests_focus() {
                palette.read(cx).focus(window);
            }
            return;
        }

        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        self.clear_property_color_input(cx);
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
        if focus_policy.requests_focus() {
            cx.defer_in(window, move |_, window, cx| {
                palette.read(cx).focus(window);
                cx.notify();
            });
        } else {
            cx.notify();
        }
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
                        | AppCommand::ToggleColorLibrary
                        | AppCommand::TogglePrecisionMenu
                        | AppCommand::ToggleRulers
                        | AppCommand::ToggleGrid
                        | AppCommand::ToggleGuides
                        | AppCommand::ToggleGuideLock
                        | AppCommand::ToggleSnapping
                        | AppCommand::ToggleSnapObjects
                        | AppCommand::ToggleSnapGuides
                        | AppCommand::ToggleSnapGrid
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
            CommandTarget::App(AppCommand::CopyAsSvg) => {
                self.can_copy_artwork(ClipboardArtworkFormat::Svg)
            }
            CommandTarget::App(AppCommand::CopyAsPng) => {
                self.can_copy_artwork(ClipboardArtworkFormat::Png)
            }
            CommandTarget::App(AppCommand::CopyAsWebP) => {
                self.can_copy_artwork(ClipboardArtworkFormat::WebP)
            }
            CommandTarget::App(AppCommand::Copy | AppCommand::Cut) => {
                self.file_operation != FileOperation::Copying
                    && self.editor.text_input_snapshot().map_or_else(
                        || !self.editor.selection().is_empty(),
                        |snapshot| !snapshot.selection.is_empty(),
                    )
            }
            CommandTarget::App(AppCommand::DeleteBackward) => {
                self.editor.text_input_snapshot().is_some() || !self.editor.selection().is_empty()
            }
            CommandTarget::App(AppCommand::Paste) => {
                self.file_operation != FileOperation::Copying
                    && (self.editor.text_input_snapshot().is_some()
                        || self.object_clipboard.is_some())
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
            CommandTarget::App(AppCommand::ClearGuides) => {
                !self.editor.document().guides.is_empty()
            }
            CommandTarget::App(AppCommand::SaveCurrentColor) => {
                self.library_color_source().is_some()
            }
            CommandTarget::App(
                AppCommand::OpenKeyboardShortcuts
                | AppCommand::ToggleLayerPanel
                | AppCommand::ToggleDesignPanel
                | AppCommand::ToggleColorLibrary
                | AppCommand::TogglePrecisionMenu
                | AppCommand::ToggleRulers
                | AppCommand::ToggleGrid
                | AppCommand::ToggleGuides
                | AppCommand::ToggleGuideLock
                | AppCommand::ToggleSnapping
                | AppCommand::ToggleSnapObjects
                | AppCommand::ToggleSnapGuides
                | AppCommand::ToggleSnapGrid
                | AppCommand::DecreaseSnapTolerance
                | AppCommand::IncreaseSnapTolerance
                | AppCommand::DecreaseGridSpacing
                | AppCommand::IncreaseGridSpacing
                | AppCommand::DecreaseGridMajorInterval
                | AppCommand::IncreaseGridMajorInterval
                | AppCommand::ShowCommandPalette,
            ) => true,
        }
    }

    fn can_copy_artwork(&self, format: ClipboardArtworkFormat) -> bool {
        self.file_operation == FileOperation::Idle
            && format.is_supported()
            && self.has_visible_artwork()
    }

    fn has_visible_artwork(&self) -> bool {
        document_has_visible_artwork(self.editor.document())
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
        if !self.document_is_dirty() {
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
                            if let Some(path) =
                                editor.document_origin.native_path().map(Path::to_path_buf)
                            {
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
                            match document {
                                document_io::OpenedDocument::Native(document) => {
                                    editor.replace_document(document, path, cx);
                                }
                                document_io::OpenedDocument::ImportedSvg(document) => {
                                    editor.replace_imported_document(document, path, cx);
                                }
                            }
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
                            if !document_io::path_is_file(&path) {
                                editor.remove_recent_file(&path, cx);
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
            .source_document_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let prompt = cx.prompt_for_new_path(&directory);
        let import_source = self
            .document_origin
            .import_source_path()
            .map(Path::to_path_buf);
        cx.spawn_in(window, async move |editor, cx| {
            let result = prompt.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.file_operation = FileOperation::Idle;
                    match result {
                        Ok(Ok(Some(path))) => {
                            let path = if let Some(source) = import_source.as_deref() {
                                document_io::normalize_imported_document_path(path, source)
                            } else {
                                Ok(document_io::normalize_document_path(path))
                            };
                            match path {
                                Ok(path) => {
                                    editor.begin_save_to_path(path, resume, window, cx);
                                }
                                Err(error) => editor.show_error(
                                    "Could not save document",
                                    error.to_string(),
                                    window,
                                    cx,
                                ),
                            }
                        }
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
            .source_document_path()
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

    fn begin_copy_artwork(
        &mut self,
        format: ClipboardArtworkFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_layer_rename(true, cx);
        self.dismiss_menus();
        if self.file_operation != FileOperation::Idle {
            return;
        }
        if !format.is_supported() {
            self.show_error(
                "Clipboard format unavailable",
                "This clipboard format is not supported on the current platform.".to_owned(),
                window,
                cx,
            );
            return;
        }
        self.settle_for_document_io();
        let Some(snapshot) = self.editor.artwork_snapshot() else {
            self.show_error(
                "Nothing to copy",
                "Add at least one visible layer before copying artwork.".to_owned(),
                window,
                cx,
            );
            return;
        };

        self.file_operation = FileOperation::Copying;
        let clipboard_before = cx.read_from_clipboard();
        let task = cx
            .background_executor()
            .spawn(async move { export::encode_artwork(format.export_format(), &snapshot) });
        cx.spawn_in(window, async move |editor, cx| {
            let result = task.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.file_operation = FileOperation::Idle;
                    match result {
                        Ok(bytes) => {
                            if cx.read_from_clipboard() != clipboard_before {
                                cx.notify();
                                return;
                            }
                            let sequence =
                                ARTWORK_CLIPBOARD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                            let image = Image {
                                format: format.image_format(),
                                bytes,
                                id: (u64::from(std::process::id()) << 32) | sequence,
                            };
                            editor.object_clipboard = None;
                            cx.write_to_clipboard(ClipboardItem::new_image(&image));
                            cx.notify();
                        }
                        Err(error) => editor.show_error(
                            "Could not copy artwork",
                            error.to_string(),
                            window,
                            cx,
                        ),
                    }
                })
                .ok();
        })
        .detach();
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
                            editor.document_origin = DocumentOrigin::Native(path.clone());
                            editor.record_recent_file(&path, cx);
                            if let Some(action) = resume {
                                if editor.document_is_dirty() {
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

    fn show_render_error(
        &mut self,
        title: &str,
        detail: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = window.prompt(PromptLevel::Critical, title, Some(&detail), &["OK"], cx);
        cx.spawn(async move |_, _| {
            let _ = prompt.await;
        })
        .detach();
        cx.notify();
    }

    fn should_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.allow_window_close {
            return !self.flush_pending_state_writes_before_close(window, cx);
        }
        if self.file_operation != FileOperation::Idle {
            return false;
        }

        self.settle_for_document_io();
        if self.document_is_dirty() {
            self.request_document_action(PendingDocumentAction::Close, window, cx);
            false
        } else {
            !self.flush_pending_state_writes_before_close(window, cx)
        }
    }

    fn flush_pending_state_writes_before_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match state_write_close_action(
            self.state_write_flush_in_progress,
            self.workspace_preferences_persist_task.is_some(),
            self.recent_files_persist_task.is_some(),
        ) {
            StateWriteCloseAction::CloseNow => false,
            StateWriteCloseAction::WaitForFlush => true,
            StateWriteCloseAction::StartFlush => {
                self.state_write_flush_in_progress = true;
                let preferences_task = self.workspace_preferences_persist_task.take();
                let recent_files_task = self.recent_files_persist_task.take();
                let preferences = preferences_task
                    .as_ref()
                    .map(|_| self.workspace_preferences.clone());
                let recent_files = recent_files_task
                    .as_ref()
                    .map(|_| self.recent_files.clone());
                cx.spawn_in(window, async move |editor, cx| {
                    if let Some(task) = preferences_task {
                        task.await;
                    }
                    if let Some(task) = recent_files_task {
                        task.await;
                    }
                    cx.background_executor()
                        .spawn(async move {
                            if let Some(preferences) = preferences {
                                preferences.persist();
                            }
                            if let Some(recent_files) = recent_files {
                                recent_files.persist();
                            }
                        })
                        .await;
                    editor
                        .update_in(cx, |editor, window, _| {
                            editor.state_write_flush_in_progress = false;
                            editor.allow_window_close = true;
                            window.remove_window();
                        })
                        .ok();
                })
                .detach();
                true
            }
        }
    }

    fn dismiss_menus(&mut self) -> bool {
        let main_menu_closed = self.open_menu.take().is_some();
        let layer_menu_closed = self.layer_context_menu.take().is_some();
        main_menu_closed || layer_menu_closed
    }

    fn execute_editor_action(&mut self, action: EditorAction, cx: &mut Context<Self>) {
        let picker_dismissed = self.property_color_input.take().is_some();
        let zoom_dismissed = self.zoom_input.take().is_some();
        let numeric_input_dismissed = self.numeric_property_input.take().is_some();
        let scrub_cancelled = self.cancel_numeric_property_scrub();
        let menu_closed = self.dismiss_menus();
        let executed = self.editor.execute_action(action);
        self.refresh_color_library_panel(cx);
        if executed {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        if executed
            || menu_closed
            || scrub_cancelled
            || picker_dismissed
            || zoom_dismissed
            || numeric_input_dismissed
        {
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
            self.refresh_color_library_panel(cx);
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
            self.refresh_color_library_panel(cx);
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
        if let Some(id) = self.active_guide.take() {
            if self.editor.remove_guide(id).is_ok() {
                cx.notify();
            }
        } else if self.editor.text_input_snapshot().is_some() {
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
        if let Some(id) = self.active_guide.take() {
            if self.editor.remove_guide(id).is_ok() {
                cx.notify();
            }
        } else if self.editor.text_input_snapshot().is_some() {
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
        self.clear_property_color_input(cx);
        self.numeric_property_input = None;
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
        self.clear_property_color_input(cx);
        self.zoom_input = None;
        self.numeric_property_input = None;
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
        self.clear_property_color_input(cx);
        self.zoom_input = None;
        self.numeric_property_input = None;
        self.finish_layer_rename(true, cx);
        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        self.show_design_panel = !self.show_design_panel;
        self.refresh_canvas_input_bounds(window);
        self.dismiss_menus();
        cx.notify();
    }

    fn toggle_color_library(
        &mut self,
        _: &ToggleColorLibrary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.color_library_open {
            self.color_library_open = false;
            self.color_library_panel = None;
        } else {
            self.open_color_library(FocusPolicy::Request, window, cx);
        }
        self.precision_menu_open = false;
        self.dismiss_menus();
        cx.notify();
    }

    fn open_color_library(
        &mut self,
        focus_policy: FocusPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = &self.color_library_panel {
            self.color_library_open = true;
            if focus_policy.requests_focus() {
                panel.read(cx).focus(window);
            }
            return;
        }

        let snapshot = self.color_library_snapshot();
        let presentation = color_library_presentation(&self.workspace_preferences);
        let panel = cx.new(|panel_cx| {
            color_library_panel::ColorLibraryPanel::new(snapshot, presentation, window, panel_cx)
        });
        cx.subscribe_in(
            &panel,
            window,
            |editor, panel, event: &color_library_panel::ColorLibraryPanelEvent, window, cx| {
                editor.handle_color_library_panel_event(panel.clone(), event.clone(), window, cx);
            },
        )
        .detach();
        if focus_policy.requests_focus() {
            panel.read(cx).focus(window);
        }
        self.color_library_panel = Some(panel);
        self.color_library_open = true;
    }

    fn handle_color_library_panel_event(
        &mut self,
        panel: Entity<color_library_panel::ColorLibraryPanel>,
        event: color_library_panel::ColorLibraryPanelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use color_library_panel::{ColorLibraryPanelEvent as Event, ColorLibrarySectionId};

        let result: Result<(), String> = match event {
            Event::Dismiss => {
                self.color_library_open = false;
                self.color_library_panel = None;
                Ok(())
            }
            Event::QuickAdd { group } => group.map(color_group_id).transpose().and_then(|group| {
                self.workspace_preferences.last_color_group = group.map(|id| id.get().to_string());
                self.quick_add_library_color(group).map(|_| ())
            }),
            Event::AddGroup => {
                let number = self.editor.document().color_library.groups.len() + 1;
                self.editor
                    .add_color_group(&format!("Group {number}"))
                    .map(|id| {
                        self.workspace_preferences.last_color_group = Some(id.get().to_string());
                    })
                    .map_err(|error| error.to_string())
            }
            Event::RenameGroup { group, name } => color_group_id(group).and_then(|id| {
                self.editor
                    .rename_color_group(id, &name)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }),
            Event::RemoveGroup { group } => color_group_id(group).and_then(|id| {
                self.editor
                    .remove_color_group(id, false)
                    .map_err(|error| error.to_string())
            }),
            Event::SetSort { section, sort } => match section {
                ColorLibrarySectionId::Ungrouped => Ok(()),
                ColorLibrarySectionId::Group(group) => color_group_id(group).and_then(|id| {
                    self.editor
                        .set_color_group_sort(
                            id,
                            core_color_sort_mode(sort.mode),
                            core_color_sort_direction(sort.direction),
                        )
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }),
            },
            Event::EditSwatch { color } => self.apply_library_color(color),
            Event::RenameColor { color, name } => self.update_library_color(color, name, None),
            Event::SetColorValue { color, value } => {
                let paint = color_picker::parse_saved_color_paint(&value).ok_or_else(|| {
                    "color must be a 3, 4, 6, or 8 digit hexadecimal value".to_owned()
                });
                paint.and_then(|paint| {
                    self.update_library_color(color, None, Some(rgba_color_from_paint(&paint)?))
                })
            }
            Event::SetColorOrder {
                color,
                one_based_order,
            } => saved_color_id(color).and_then(|id| {
                let group = self
                    .editor
                    .document()
                    .color_library
                    .color(id)
                    .ok_or_else(|| "saved color no longer exists".to_owned())?
                    .group_id;
                self.editor
                    .move_saved_color(id, group, one_based_order.saturating_sub(1))
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }),
            Event::DuplicateColor { color } => saved_color_id(color).and_then(|id| {
                self.editor
                    .duplicate_saved_color(id)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }),
            Event::RemoveColor { color } => saved_color_id(color).and_then(|id| {
                self.editor
                    .remove_saved_color(id)
                    .map_err(|error| error.to_string())
            }),
            Event::MoveColorBefore {
                color,
                section,
                before,
            } => saved_color_id(color).and_then(|id| {
                let group = color_section_group(section)?;
                let before = before.map(saved_color_id).transpose()?;
                let index = before
                    .and_then(|before| {
                        self.editor
                            .document()
                            .color_library
                            .colors_in_group(group)
                            .iter()
                            .position(|candidate| candidate.id == before)
                    })
                    .unwrap_or_else(|| {
                        self.editor
                            .document()
                            .color_library
                            .colors_in_group(group)
                            .len()
                    });
                self.editor
                    .move_saved_color(id, group, index)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }),
            Event::MoveColorToSection { color, section } => saved_color_id(color).and_then(|id| {
                let group = color_section_group(section)?;
                let index = self
                    .editor
                    .document()
                    .color_library
                    .colors_in_group(group)
                    .len();
                self.editor
                    .move_saved_color(id, group, index)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }),
            Event::MoveGroupBefore { group, before } => color_group_id(group).and_then(|id| {
                let before = color_group_id(before)?;
                let index = self
                    .editor
                    .document()
                    .color_library
                    .groups
                    .iter()
                    .position(|candidate| candidate.id == before)
                    .ok_or_else(|| "target color group no longer exists".to_owned())?;
                self.editor
                    .reorder_color_group(id, index)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }),
            Event::MoveGroupBy { group, direction } => color_group_id(group).and_then(|id| {
                let groups = &self.editor.document().color_library.groups;
                let current = groups
                    .iter()
                    .position(|candidate| candidate.id == id)
                    .ok_or_else(|| "color group no longer exists".to_owned())?;
                let target = match direction {
                    color_library_panel::MoveDirection::Up => current.saturating_sub(1),
                    color_library_panel::MoveDirection::Down => {
                        (current + 1).min(groups.len().saturating_sub(1))
                    }
                };
                self.editor
                    .reorder_color_group(id, target)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }),
            Event::PresentationChanged(presentation) => {
                self.store_color_library_presentation(&presentation);
                Ok(())
            }
        };

        if let Err(error) = result {
            log::warn!("Color Library action failed: {error}");
        }
        if self.color_library_open {
            let snapshot = self.color_library_snapshot();
            panel.update(cx, |panel, cx| panel.set_snapshot(snapshot, cx));
        }
        self.schedule_workspace_preferences_persist(cx);
        cx.notify();
    }

    fn library_color_source(&self) -> Option<editor_core::Paint> {
        let picker = self
            .property_color_input
            .as_ref()
            .map(|input| input.picker.paint().clone());
        let selection = self
            .editor
            .selection()
            .primary()
            .and_then(|id| self.editor.document().get(id))
            .and_then(|node| node.style.fill.clone());
        let creation = self
            .editor
            .active_creation_style()
            .and_then(|style| style.fill.clone());
        first_library_color_source(picker, selection, creation)
    }

    fn preferred_color_group(&self) -> Option<editor_core::ColorGroupId> {
        self.workspace_preferences
            .last_color_group
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(editor_core::ColorGroupId::from_opaque)
            .filter(|id| self.editor.document().color_library.group(*id).is_some())
    }

    fn color_library_snapshot(&self) -> color_library_panel::ColorLibrarySnapshot {
        build_color_library_snapshot(
            self.editor.document(),
            self.library_color_source().is_some(),
            self.preferred_color_group(),
        )
    }

    fn quick_add_library_color(
        &mut self,
        group: Option<editor_core::ColorGroupId>,
    ) -> Result<QuickAddLibraryColorResult, String> {
        let paint = self.library_color_source().ok_or_else(|| {
            "select a filled layer or choose a picker color before saving".to_owned()
        })?;
        let rgba = rgba_color_from_paint(&paint)?;
        if let Some(existing) = self.editor.document().color_library.find_exact(rgba) {
            return Ok(QuickAddLibraryColorResult {
                id: existing.id,
                group_id: existing.group_id,
                already_existed: true,
            });
        }
        self.editor
            .add_saved_color(group, None, rgba)
            .map(|id| QuickAddLibraryColorResult {
                id,
                group_id: group,
                already_existed: false,
            })
            .map_err(|error| error.to_string())
    }

    fn update_library_color(
        &mut self,
        color: color_library_panel::ColorLibraryColorId,
        name: Option<String>,
        rgba: Option<editor_core::RgbaColor>,
    ) -> Result<(), String> {
        let id = saved_color_id(color)?;
        let current = self
            .editor
            .document()
            .color_library
            .color(id)
            .cloned()
            .ok_or_else(|| "saved color no longer exists".to_owned())?;
        let name = if rgba.is_some() && name.is_none() {
            current.name.as_deref()
        } else {
            name.as_deref()
        };
        self.editor
            .update_saved_color(id, name, rgba.unwrap_or(current.rgba))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn apply_library_color(
        &mut self,
        color: color_library_panel::ColorLibraryColorId,
    ) -> Result<(), String> {
        let id = saved_color_id(color)?;
        let rgba = self
            .editor
            .document()
            .color_library
            .color(id)
            .ok_or_else(|| "saved color no longer exists".to_owned())?
            .rgba
            .components();
        let paint = editor_core::Paint::Solid(rgba);
        if let Some(input) = self.property_color_input.as_mut() {
            input.picker.set_paint(paint.clone());
            apply_color_input(&mut self.editor, input.scope, input.target, paint);
        } else if !self.editor.selection().is_empty() {
            self.editor.set_selected_fill(Some(paint));
        } else if self.editor.active_creation_style().is_some() {
            self.editor.set_creation_fill(Some(paint));
        }
        Ok(())
    }

    fn store_color_library_presentation(
        &mut self,
        presentation: &color_library_panel::ColorLibraryPanelPresentation,
    ) {
        self.workspace_preferences.color_library_panel =
            workspace_preferences::FloatingPanelPreferences {
                x: presentation.geometry.left,
                y: presentation.geometry.top,
                width: presentation.geometry.width,
                height: presentation.geometry.height,
            };
        self.workspace_preferences.collapsed_color_groups = presentation
            .collapsed_sections
            .iter()
            .map(|section| match section {
                color_library_panel::ColorLibrarySectionId::Ungrouped => "ungrouped".to_owned(),
                color_library_panel::ColorLibrarySectionId::Group(group) => {
                    format!("group-{}", group.0)
                }
            })
            .collect();
    }

    fn refresh_color_library_panel(&self, cx: &mut Context<Self>) {
        let Some(panel) = &self.color_library_panel else {
            return;
        };
        let snapshot = self.color_library_snapshot();
        panel.update(cx, |panel, cx| panel.set_snapshot(snapshot, cx));
    }

    fn toggle_precision_menu(
        &mut self,
        _: &TogglePrecisionMenu,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.precision_menu_open = !self.precision_menu_open;
        self.dismiss_menus();
        cx.notify();
    }

    fn toggle_rulers(&mut self, _: &ToggleRulers, _window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_preferences.show_rulers = !self.workspace_preferences.show_rulers;
        self.persist_workspace_preferences(cx);
    }

    fn toggle_grid(&mut self, _: &ToggleGrid, _window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_preferences.show_grid = !self.workspace_preferences.show_grid;
        self.persist_workspace_preferences(cx);
    }

    fn toggle_guides(&mut self, _: &ToggleGuides, _window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_preferences.show_guides = !self.workspace_preferences.show_guides;
        if !self.workspace_preferences.show_guides {
            self.active_guide = None;
            self.guide_drag = None;
            self.guide_position_input = None;
        }
        self.persist_workspace_preferences(cx);
    }

    fn toggle_guide_lock(
        &mut self,
        _: &ToggleGuideLock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_preferences.guides_locked = !self.workspace_preferences.guides_locked;
        if self.workspace_preferences.guides_locked {
            self.active_guide = None;
            self.guide_drag = None;
            self.guide_position_input = None;
        }
        self.persist_workspace_preferences(cx);
    }

    fn toggle_snapping(
        &mut self,
        _: &ToggleSnapping,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_preferences.snapping_enabled = !self.workspace_preferences.snapping_enabled;
        self.persist_workspace_preferences(cx);
    }

    fn toggle_snap_objects(
        &mut self,
        _: &ToggleSnapObjects,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_preferences.snap_to_objects = !self.workspace_preferences.snap_to_objects;
        self.persist_workspace_preferences(cx);
    }

    fn toggle_snap_guides(
        &mut self,
        _: &ToggleSnapGuides,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_preferences.snap_to_guides = !self.workspace_preferences.snap_to_guides;
        self.persist_workspace_preferences(cx);
    }

    fn decrease_snap_tolerance(
        &mut self,
        _: &DecreaseSnapTolerance,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_snap_tolerance(-1.0, cx);
    }

    fn increase_snap_tolerance(
        &mut self,
        _: &IncreaseSnapTolerance,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_snap_tolerance(1.0, cx);
    }

    fn adjust_snap_tolerance(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.workspace_preferences.snap_tolerance =
            (self.workspace_preferences.snap_tolerance + delta).clamp(
                precision_ui::MIN_TOLERANCE_PX,
                precision_ui::MAX_TOLERANCE_PX,
            );
        self.persist_workspace_preferences(cx);
    }

    fn decrease_grid_spacing(
        &mut self,
        _: &DecreaseGridSpacing,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_grid_spacing(false, cx);
    }

    fn increase_grid_spacing(
        &mut self,
        _: &IncreaseGridSpacing,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_grid_spacing(true, cx);
    }

    fn adjust_grid_spacing(&mut self, increase: bool, cx: &mut Context<Self>) {
        let current = self.editor.document().grid;
        let step = precision_ui::grid_spacing_step(current.spacing);
        let spacing = if increase {
            current.spacing + step
        } else {
            current.spacing - step
        }
        .clamp(
            precision_ui::MIN_GRID_SPACING,
            precision_ui::MAX_GRID_SPACING,
        );
        if let Ok(settings) = editor_core::GridSettings::new(spacing, current.major_every) {
            if self.editor.set_grid_settings(settings).unwrap_or(false) {
                cx.notify();
            }
        }
    }

    fn decrease_grid_major_interval(
        &mut self,
        _: &DecreaseGridMajorInterval,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_grid_major_interval(false, cx);
    }

    fn increase_grid_major_interval(
        &mut self,
        _: &IncreaseGridMajorInterval,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.adjust_grid_major_interval(true, cx);
    }

    fn adjust_grid_major_interval(&mut self, increase: bool, cx: &mut Context<Self>) {
        let current = self.editor.document().grid;
        let major_every = if increase {
            current.major_every.saturating_add(1)
        } else {
            current.major_every.saturating_sub(1)
        }
        .clamp(1, 10);
        if let Ok(settings) = editor_core::GridSettings::new(current.spacing, major_every) {
            if self.editor.set_grid_settings(settings).unwrap_or(false) {
                cx.notify();
            }
        }
    }

    fn toggle_snap_grid(
        &mut self,
        _: &ToggleSnapGrid,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_preferences.snap_to_grid = !self.workspace_preferences.snap_to_grid;
        self.persist_workspace_preferences(cx);
    }

    fn clear_guides(&mut self, _: &ClearGuides, _window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.clear_guides() {
            self.active_guide = None;
            self.guide_drag = None;
            self.guide_position_input = None;
            cx.notify();
        }
    }

    fn save_current_color(
        &mut self,
        _: &SaveCurrentColor,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quick_add_current_picker_color(cx);
    }

    fn persist_workspace_preferences(&mut self, cx: &mut Context<Self>) {
        self.sync_snap_settings();
        self.schedule_workspace_preferences_persist(cx);
        cx.notify();
    }

    fn schedule_workspace_preferences_persist(&mut self, cx: &mut Context<Self>) {
        let preferences = self.workspace_preferences.clone();
        self.workspace_preferences_persist_task =
            Some(cx.background_executor().spawn(async move {
                gpui::Timer::after(WORKSPACE_PREFERENCES_DEBOUNCE).await;
                preferences.persist();
            }));
    }

    fn record_recent_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.recent_files.record(path);
        self.schedule_recent_files_persist(cx);
    }

    fn remove_recent_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.recent_files.remove(path);
        self.schedule_recent_files_persist(cx);
    }

    fn schedule_recent_files_persist(&mut self, cx: &mut Context<Self>) {
        let recent_files = self.recent_files.clone();
        self.recent_files_persist_task = Some(cx.background_executor().spawn(async move {
            gpui::Timer::after(RECENT_FILES_DEBOUNCE).await;
            recent_files.persist();
        }));
    }

    fn sync_snap_settings(&mut self) {
        self.editor
            .set_snap_settings(editor_core::SnapSettings {
                enabled: self.workspace_preferences.snapping_enabled,
                objects: self.workspace_preferences.snap_to_objects,
                guides: self.workspace_preferences.snap_to_guides,
                grid: self.workspace_preferences.snap_to_grid,
                tolerance: self.workspace_preferences.snap_tolerance,
            })
            .expect("sanitized workspace snapping preferences must be valid");
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
        self.clear_property_color_input(cx);
        self.zoom_input = None;
        self.numeric_property_input = None;
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
        mode: NumericAdjustmentMode,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scrub) = self.numeric_property_scrub.as_ref() else {
            return false;
        };
        if !pointer_x.is_finite() {
            return false;
        }
        let pointer_delta = pointer_x - scrub.pointer_origin_x;
        let delta = properties_panel::numeric_property_delta(scrub.target, pointer_delta, mode);
        let changed =
            self.editor
                .preview_numeric_property_scrub_with_mode(&scrub.session, delta, mode);
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
            let mode = if event.modifiers.shift {
                NumericAdjustmentMode::Coarse
            } else {
                NumericAdjustmentMode::Fine
            };
            self.preview_numeric_property_scrub_at(event.position.x.0, mode, cx);
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
        let mode = if event.modifiers.shift {
            NumericAdjustmentMode::Coarse
        } else {
            NumericAdjustmentMode::Fine
        };
        self.preview_numeric_property_scrub_at(event.position.x.0, mode, cx);
        let Some(scrub) = self.numeric_property_scrub.take() else {
            return;
        };
        self.editor.commit_numeric_property_scrub(scrub.session);
        cx.notify();
    }

    fn adjust_numeric_property(
        &mut self,
        target: NumericPropertyTarget,
        direction: NumericAdjustmentDirection,
        mode: NumericAdjustmentMode,
        cx: &mut Context<Self>,
    ) {
        if self.file_operation == FileOperation::Opening {
            return;
        }
        self.cancel_numeric_property_scrub();
        self.finish_layer_rename(true, cx);
        let Some(session) = self.editor.begin_numeric_property_scrub(target) else {
            return;
        };
        let delta = target.step(mode) * direction.multiplier();
        let changed = self
            .editor
            .preview_numeric_property_scrub_with_mode(&session, delta, mode);
        let committed = self.editor.commit_numeric_property_scrub(session);
        if changed || committed {
            cx.notify();
        }
    }

    fn start_numeric_property_input(
        &mut self,
        target: NumericPropertyTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_operation == FileOperation::Opening {
            return;
        }
        self.cancel_numeric_property_scrub();
        self.finish_layer_rename(true, cx);
        let Some(value) = self.editor.numeric_property_value(target) else {
            return;
        };
        self.clear_property_color_input(cx);
        self.zoom_input = None;
        self.dismiss_menus();
        self.numeric_property_input = Some(NumericPropertyInput {
            target,
            value: format_numeric_property_input(target, value),
            replace_on_type: true,
            invalid: false,
        });
        self.focus_handle.focus(window);
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
        self.numeric_property_input = None;
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

    fn start_frame_background_color_input(
        &mut self,
        _: &StartFrameBackgroundColorInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_property_color_input(
            ColorInputScope::Selection,
            properties_panel::ColorTarget::FrameBackground,
            FocusPolicy::Request,
            window,
            cx,
        );
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

    fn start_fill_color_input(
        &mut self,
        _: &StartFillColorInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_property_color_input(
            ColorInputScope::Selection,
            properties_panel::ColorTarget::Fill,
            FocusPolicy::Request,
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
            FocusPolicy::Request,
            window,
            cx,
        );
    }

    fn start_property_color_input(
        &mut self,
        scope: ColorInputScope,
        target: properties_panel::ColorTarget,
        focus_policy: FocusPolicy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(paint) = color_input_paint(&self.editor, scope, target) else {
            return false;
        };
        if self.editor.cancel_pointer_interaction() {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        self.zoom_input = None;
        self.numeric_property_input = None;
        self.property_color_input = Some(PropertyColorInput {
            scope,
            target,
            picker: color_picker::ColorPickerState::new(paint),
            save_feedback: None,
        });
        self.refresh_color_library_panel(cx);
        focus_policy.apply(&self.focus_handle, window);
        cx.notify();
        true
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
            FocusPolicy::Request,
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
            FocusPolicy::Request,
            window,
            cx,
        );
    }

    pub(crate) fn set_color_picker_model(
        &mut self,
        model: color_picker::ColorModel,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = &mut self.property_color_input {
            input.picker.set_model(model);
            cx.notify();
        }
    }

    pub(crate) fn select_color_picker_preset(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(input) = &mut self.property_color_input {
            input.picker.select_preset(index);
            cx.notify();
        }
    }

    pub(crate) fn select_color_picker_saved_color(
        &mut self,
        id: u64,
        keyboard_index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = editor_core::SavedColorId::from_opaque(id) else {
            return false;
        };
        let Some(rgba) = self
            .editor
            .document()
            .color_library
            .color(id)
            .map(|color| color.rgba.components())
        else {
            return false;
        };
        if let Some(input) = &mut self.property_color_input {
            input.picker.set_paint(editor_core::Paint::Solid(rgba));
            input.picker.focus_saved_color(keyboard_index);
            cx.notify();
            true
        } else {
            false
        }
    }

    pub(crate) fn quick_add_current_picker_color(&mut self, cx: &mut Context<Self>) {
        match self.quick_add_library_color(self.preferred_color_group()) {
            Ok(result) => {
                let library = &self.editor.document().color_library;
                let value = library
                    .color(result.id)
                    .map(|color| color.rgba.hex_label())
                    .unwrap_or_else(|| "saved color".to_owned());
                let group_name = result
                    .group_id
                    .and_then(|id| library.group(id))
                    .map_or_else(|| "Ungrouped".to_owned(), |group| group.name.clone());
                if let Some(input) = &mut self.property_color_input {
                    input.picker.reveal_saved_color(&value);
                    input.save_feedback = Some(color_picker::SavedColorFeedback {
                        id: result.id.get(),
                        message: if result.already_existed {
                            format!("Already saved in {group_name}").into()
                        } else {
                            format!("Saved to {group_name}").into()
                        },
                    });
                }
            }
            Err(error) => log::warn!("could not save picker color: {error}"),
        }
        self.refresh_color_library_panel(cx);
        cx.notify();
    }

    pub(crate) fn rename_picker_saved_color(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = editor_core::SavedColorId::from_opaque(id) else {
            return;
        };
        self.open_color_library(FocusPolicy::Request, window, cx);
        let Some(panel) = self.color_library_panel.clone() else {
            return;
        };
        panel.update(cx, |panel, cx| {
            panel.begin_color_rename(
                color_library_panel::ColorLibraryColorId(id.get()),
                window,
                cx,
            );
        });
        self.precision_menu_open = false;
        cx.notify();
    }

    pub(crate) fn show_color_library_from_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_color_library(FocusPolicy::Request, window, cx);
        self.precision_menu_open = false;
        cx.notify();
    }

    pub(crate) fn focus_color_picker_hex_input(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = &mut self.property_color_input {
            input.picker.focus_hex_input();
            cx.notify();
        }
    }

    pub(crate) fn focus_color_picker_saved_search(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = &mut self.property_color_input {
            input.picker.focus_saved_search();
            cx.notify();
        }
    }

    pub(crate) fn set_color_picker_control_bounds(
        &mut self,
        control: color_picker::ColorPickerControl,
        bounds: Bounds<gpui::Pixels>,
    ) {
        if let Some(input) = &mut self.property_color_input {
            input.picker.set_control_bounds(control, bounds);
        }
    }

    pub(crate) fn update_color_picker_from_point(
        &mut self,
        control: color_picker::ColorPickerControl,
        point: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self
            .property_color_input
            .as_mut()
            .is_some_and(|input| input.picker.update_from_point(control, point))
        {
            cx.notify();
        }
    }

    pub(crate) fn commit_color_picker(&mut self, cx: &mut Context<Self>) {
        let Some(input) = &mut self.property_color_input else {
            return;
        };
        let Some(paint) = input.picker.resolve_hex() else {
            cx.notify();
            return;
        };
        let scope = input.scope;
        let target = input.target;
        self.property_color_input = None;
        if apply_color_input(&mut self.editor, scope, target, paint) {
            self.current_cursor = convert_cursor(self.editor.cursor());
        }
        self.refresh_color_library_panel(cx);
        cx.notify();
    }

    pub(crate) fn dismiss_color_picker(&mut self, cx: &mut Context<Self>) {
        if self.clear_property_color_input(cx) {
            cx.notify();
        }
    }

    fn clear_property_color_input(&mut self, cx: &mut Context<Self>) -> bool {
        let dismissed = self.property_color_input.take().is_some();
        if dismissed {
            self.refresh_color_library_panel(cx);
        }
        dismissed
    }

    fn toggle_creation_fill(
        &mut self,
        _: &ToggleCreationFill,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picker_dismissed = self.property_color_input.take().is_some();
        let changed = self.editor.toggle_creation_fill();
        if picker_dismissed || changed {
            self.refresh_color_library_panel(cx);
            cx.notify();
        }
    }

    fn toggle_creation_stroke(
        &mut self,
        _: &ToggleCreationStroke,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picker_dismissed = self.property_color_input.take().is_some();
        let changed = self.editor.toggle_creation_stroke();
        if picker_dismissed || changed {
            self.refresh_color_library_panel(cx);
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
        let picker_dismissed = self.property_color_input.take().is_some();
        let changed = self.editor.set_selected_fill(fill);
        if picker_dismissed || changed {
            self.refresh_color_library_panel(cx);
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
        let picker_dismissed = self.property_color_input.take().is_some();
        let changed = self.editor.toggle_selected_stroke();
        if picker_dismissed || changed {
            self.refresh_color_library_panel(cx);
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
        if self.file_operation == FileOperation::Copying {
            return;
        }
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
        if self.file_operation == FileOperation::Copying {
            return;
        }
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
        if self.file_operation == FileOperation::Copying {
            return;
        }
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
        let color_picker_dismissed = self.property_color_input.take().is_some();
        let zoom_dismissed = self.zoom_input.take().is_some();
        let numeric_property_dismissed = self.numeric_property_input.take().is_some();
        let guide_position_dismissed = self.guide_position_input.take().is_some();
        let input_dismissed = color_picker_dismissed
            || zoom_dismissed
            || numeric_property_dismissed
            || guide_position_dismissed;
        if color_picker_dismissed {
            self.refresh_color_library_panel(cx);
        }
        if input_dismissed {
            cx.notify();
        }
    }

    fn escape(&mut self, _: &Escape, _window: &mut Window, cx: &mut Context<Self>) {
        if self.guide_position_input.take().is_some() {
            cx.notify();
            return;
        }
        if self.guide_drag.take().is_some() {
            cx.notify();
            return;
        }
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

    fn begin_guide_drag(
        &mut self,
        position: glam::Vec2,
        duplicate: bool,
        click_count: usize,
    ) -> bool {
        if !self.workspace_preferences.show_guides || self.workspace_preferences.guides_locked {
            return false;
        }

        let ruler = precision_ui::DEFAULT_RULER_THICKNESS;
        let axis = if self.workspace_preferences.show_rulers && position.y <= ruler {
            Some(editor_core::GuideAxis::Horizontal)
        } else if self.workspace_preferences.show_rulers && position.x <= ruler {
            Some(editor_core::GuideAxis::Vertical)
        } else {
            None
        };
        let world = self.editor.view().to_world(position);
        if let Some(axis) = axis {
            self.editor.cancel_pointer_interaction();
            self.guide_drag = Some(GuideDrag {
                source: None,
                axis,
                position: guide_axis_value(axis, world),
                duplicate: false,
            });
            self.active_guide = None;
            return true;
        }

        const GUIDE_HIT_TOLERANCE_PX: f32 = 5.0;
        let view = *self.editor.view();
        let hit = self
            .editor
            .document()
            .guides
            .iter()
            .filter_map(|guide| {
                let screen = match guide.axis {
                    editor_core::GuideAxis::Horizontal => {
                        view.to_screen(glam::Vec2::new(0.0, guide.position)).y
                    }
                    editor_core::GuideAxis::Vertical => {
                        view.to_screen(glam::Vec2::new(guide.position, 0.0)).x
                    }
                };
                let pointer = match guide.axis {
                    editor_core::GuideAxis::Horizontal => position.y,
                    editor_core::GuideAxis::Vertical => position.x,
                };
                let distance = (screen - pointer).abs();
                (distance <= GUIDE_HIT_TOLERANCE_PX).then_some((distance, *guide))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, guide)| guide);

        let Some(guide) = hit else {
            return false;
        };
        self.editor.cancel_pointer_interaction();
        self.active_guide = Some(guide.id);
        if click_count >= 2 {
            self.guide_position_input = Some(GuidePositionInput {
                id: guide.id,
                value: format!("{}", guide.position),
                replace_on_type: true,
                invalid: false,
            });
            self.guide_drag = None;
            return true;
        }
        self.guide_drag = Some(GuideDrag {
            source: Some(guide.id),
            axis: guide.axis,
            position: guide.position,
            duplicate,
        });
        true
    }

    fn update_guide_drag(&mut self, position: glam::Vec2) -> bool {
        let Some(drag) = self.guide_drag.as_mut() else {
            return false;
        };
        let world = self.editor.view().to_world(position);
        drag.position = guide_axis_value(drag.axis, world);
        true
    }

    fn finish_guide_drag(&mut self, position: glam::Vec2) -> bool {
        let Some(drag) = self.guide_drag.take() else {
            return false;
        };
        let ruler = precision_ui::DEFAULT_RULER_THICKNESS;
        let returned_to_ruler = self.workspace_preferences.show_rulers
            && match drag.axis {
                editor_core::GuideAxis::Horizontal => position.y <= ruler,
                editor_core::GuideAxis::Vertical => position.x <= ruler,
            };

        if returned_to_ruler {
            if let Some(id) = drag.source {
                if self.editor.remove_guide(id).is_ok() {
                    self.active_guide = None;
                }
            }
            return true;
        }

        self.active_guide = match (drag.source, drag.duplicate) {
            (Some(_), true) | (None, _) => self.editor.add_guide(drag.axis, drag.position).ok(),
            (Some(id), false) => {
                let _ = self.editor.move_guide(id, drag.position);
                Some(id)
            }
        };
        true
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
        let picker_dismissed = self.clear_property_color_input(cx);
        self.zoom_input = None;
        self.focus_handle.focus(window);
        let position = canvas_position(
            event.position,
            self.canvas_input_bounds,
            visible_panel_width(self.show_layers_panel, self.layers_panel_width),
        );
        if event.button == MouseButton::Left
            && self.begin_guide_drag(position, event.modifiers.alt, event.click_count)
        {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        self.active_guide = None;
        let modifiers = convert_modifiers(&event.modifiers);
        let button = convert_mouse_button(event.button);
        let effects = self.editor.handle_pointer_down_with_click_count(
            position,
            button,
            modifiers,
            event.click_count,
        );
        if effects.redraw {
            self.refresh_color_library_panel(cx);
        }
        if effects.redraw || picker_dismissed {
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
        if event.button == MouseButton::Left && self.finish_guide_drag(position) {
            cx.notify();
            return;
        }
        let modifiers = convert_modifiers(&event.modifiers);
        let button = convert_mouse_button(event.button);

        let input = editor_core::InputEvent::PointerUp {
            position,
            button,
            modifiers,
        };

        let effects = self.editor.handle_event(input);
        if effects.redraw {
            self.refresh_color_library_panel(cx);
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
        if self.update_guide_drag(position) {
            cx.notify();
            return;
        }
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

    fn on_canvas_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Middle-button drags are routed by a window-level listener so panning
        // remains live after the pointer leaves the canvas hitbox.
        if event.pressed_button != Some(MouseButton::Middle) {
            self.on_mouse_move(event, window, cx);
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
        let saved_color_ids = self.visible_picker_saved_color_ids();
        let primary = if cfg!(target_os = "macos") {
            event.keystroke.modifiers.platform
        } else {
            event.keystroke.modifiers.control
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.dismiss_color_picker(cx);
            }
            "tab" => {
                if let Some(input) = &mut self.property_color_input {
                    input
                        .picker
                        .focus_next(event.keystroke.modifiers.shift, saved_color_ids.len());
                    cx.notify();
                }
            }
            "left" | "right" | "up" | "down" => {
                if self.property_color_input.as_mut().is_some_and(|input| {
                    input.picker.handle_arrow(
                        &event.keystroke.key,
                        event.keystroke.modifiers.shift,
                        saved_color_ids.len(),
                    )
                }) {
                    cx.notify();
                }
            }
            "enter" | "space" => {
                let action = self
                    .property_color_input
                    .as_mut()
                    .map(|input| input.picker.activate_focused(&saved_color_ids));
                if let Some(action) = action {
                    self.handle_color_picker_keyboard_action(action, cx);
                }
            }
            "backspace" | "delete" => {
                if let Some(input) = &mut self.property_color_input {
                    if input.picker.accepts_text_input() {
                        input.picker.backspace();
                        cx.notify();
                    } else if input.picker.accepts_saved_search_input() {
                        input.picker.backspace_saved_search();
                        cx.notify();
                    }
                }
            }
            "a" if primary => {
                if let Some(input) = &mut self.property_color_input {
                    if input.picker.accepts_text_input() {
                        input.picker.select_hex();
                        cx.notify();
                    } else if input.picker.accepts_saved_search_input() {
                        input.picker.clear_saved_search();
                        cx.notify();
                    }
                }
            }
            "v" if primary => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    if let Some(input) = &mut self.property_color_input {
                        if input.picker.accepts_text_input() {
                            input.picker.replace_hex(&text);
                            cx.notify();
                        } else if input.picker.accepts_saved_search_input() {
                            input.picker.replace_saved_search(&text);
                            cx.notify();
                        }
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
                    if input.picker.accepts_text_input() {
                        input.picker.append_hex(text);
                        cx.notify();
                    } else if input.picker.accepts_saved_search_input() {
                        input.picker.append_saved_search(text);
                        cx.notify();
                    }
                }
            }
            _ => {}
        }
        cx.stop_propagation();
    }

    fn visible_picker_saved_color_ids(&self) -> Vec<u64> {
        let Some(input) = self.property_color_input.as_ref() else {
            return Vec::new();
        };
        let current = rgba_color_from_paint(input.picker.paint()).ok();
        build_picker_saved_color_groups(
            &self.editor.document().color_library,
            input.picker.saved_search(),
            current,
        )
        .into_iter()
        .flat_map(|group| group.colors.into_iter().map(|color| color.id))
        .collect()
    }

    fn handle_color_picker_keyboard_action(
        &mut self,
        action: color_picker::ColorPickerKeyboardAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            color_picker::ColorPickerKeyboardAction::Updated => cx.notify(),
            color_picker::ColorPickerKeyboardAction::ApplySavedColor { id, index } => {
                if self.select_color_picker_saved_color(id, index, cx) {
                    self.commit_color_picker(cx);
                }
            }
            color_picker::ColorPickerKeyboardAction::Commit => self.commit_color_picker(cx),
            color_picker::ColorPickerKeyboardAction::Dismiss => self.dismiss_color_picker(cx),
        }
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

    fn handle_numeric_property_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let primary = if cfg!(target_os = "macos") {
            event.keystroke.modifiers.platform
        } else {
            event.keystroke.modifiers.control
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.numeric_property_input = None;
                cx.notify();
            }
            "enter" => {
                let Some(input) = self.numeric_property_input.as_ref() else {
                    return;
                };
                let Some(value) = parse_numeric_property_input(input.target, &input.value) else {
                    if let Some(input) = &mut self.numeric_property_input {
                        input.invalid = true;
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                };
                let target = input.target;
                if !numeric_property_input_value_is_valid(target, value) {
                    if let Some(input) = &mut self.numeric_property_input {
                        input.invalid = true;
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                self.numeric_property_input = None;
                self.editor.set_numeric_property(target, value);
                cx.notify();
            }
            "backspace" | "delete" => {
                if let Some(input) = &mut self.numeric_property_input {
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
                if let Some(input) = &mut self.numeric_property_input {
                    input.replace_on_type = true;
                    input.invalid = false;
                    cx.notify();
                }
            }
            "v" if primary => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    if let Some(input) = &mut self.numeric_property_input {
                        input.value = sanitize_numeric_property_input(&text);
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
                if let Some(input) = &mut self.numeric_property_input {
                    if input.replace_on_type {
                        input.value.clear();
                        input.replace_on_type = false;
                    }
                    append_numeric_property_input(&mut input.value, text);
                    input.invalid = false;
                    cx.notify();
                }
            }
            _ => {}
        }
        cx.stop_propagation();
    }

    fn handle_guide_position_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let primary = if cfg!(target_os = "macos") {
            event.keystroke.modifiers.platform
        } else {
            event.keystroke.modifiers.control
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.guide_position_input = None;
                cx.notify();
            }
            "enter" => {
                let Some(input) = self.guide_position_input.as_ref() else {
                    return;
                };
                let Ok(position) = input.value.trim().parse::<f32>() else {
                    if let Some(input) = &mut self.guide_position_input {
                        input.invalid = true;
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                };
                let id = input.id;
                match self.editor.move_guide(id, position) {
                    Ok(_) => {
                        self.guide_position_input = None;
                        self.active_guide = Some(id);
                    }
                    Err(_) => {
                        if let Some(input) = &mut self.guide_position_input {
                            input.invalid = true;
                        }
                    }
                }
                cx.notify();
            }
            "backspace" | "delete" => {
                if let Some(input) = &mut self.guide_position_input {
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
                if let Some(input) = &mut self.guide_position_input {
                    input.replace_on_type = true;
                    input.invalid = false;
                    cx.notify();
                }
            }
            "v" if primary => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    if let Some(input) = &mut self.guide_position_input {
                        input.value = sanitize_numeric_property_input(&text);
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
                if let Some(input) = &mut self.guide_position_input {
                    if input.replace_on_type {
                        input.value.clear();
                        input.replace_on_type = false;
                    }
                    for character in text.chars().filter(|character| {
                        character.is_ascii_digit() || matches!(character, '.' | '-' | '+')
                    }) {
                        input.value.push(character);
                    }
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
        if self.guide_position_input.is_some() {
            self.handle_guide_position_key(event, cx);
            return;
        }
        if self.numeric_property_input.is_some() {
            self.handle_numeric_property_key(event, cx);
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
        if matches!(
            event.keystroke.key.as_str(),
            "left" | "right" | "up" | "down"
        ) {
            self.editor.finish_nudge_sequence();
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

#[derive(Debug, Clone, Copy)]
struct GuideDrag {
    source: Option<editor_core::GuideId>,
    axis: editor_core::GuideAxis,
    position: f32,
    duplicate: bool,
}

impl Focusable for Strek {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn document_has_visible_artwork(document: &editor_core::Document) -> bool {
    document.descendants(document.root).any(|id| {
        if !document.is_effectively_visible(id) {
            return false;
        }
        let Some(node) = document.get(id) else {
            return false;
        };
        document.local_bounds(id).is_some_and(|bounds| {
            bounds.min.is_finite()
                && bounds.max.is_finite()
                && (!bounds.is_empty()
                    || matches!(&node.kind, NodeKind::Shape(_))
                        && node.style.stroke.as_ref().is_some_and(|stroke| {
                            stroke.width > 0.0 && (bounds.width() > 0.0 || bounds.height() > 0.0)
                        }))
        })
    })
}

impl Render for Strek {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.did_focus {
            self.focus_handle.focus(window);
            self.did_focus = true;
        }
        self.update_window_title(window);
        if let Some(identity) = self.artwork_image_cache.take_paint_failure() {
            if self.artwork_raster_scheduler.fail_desired(identity) {
                self.artwork_raster_task.take();
                self.show_render_error(
                    "Could not display advanced artwork",
                    "GPUI could not upload the prepared canvas image. The last successfully painted image is retained when available, the document is unchanged, and SVG export remains available."
                        .to_owned(),
                    window,
                    cx,
                );
            }
        }
        let tool = self.editor.tool;
        let interaction = self.editor.interaction_kind();
        let key_context = editor_key_context(
            self.file_operation == FileOperation::Opening,
            self.command_palette.is_some()
                || self.layer_name_input.is_some()
                || self.property_color_input.is_some()
                || self.zoom_input.is_some()
                || self.numeric_property_input.is_some()
                || self.guide_position_input.is_some(),
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
        let color_library_panel = self.color_library_panel.clone();
        let guide_position_input = self.guide_position_input.clone();
        let open_menu = self.open_menu;
        let cursor = self.current_cursor;
        let document_name = self.document_name();
        let document_location = self.document_location();
        let recent_files = self.recent_files.paths().to_vec();
        let file_busy = self.file_operation != FileOperation::Idle;
        let can_copy_artwork = self.has_visible_artwork();
        let can_copy_svg =
            !file_busy && can_copy_artwork && ClipboardArtworkFormat::Svg.is_supported();
        let can_copy_png =
            !file_busy && can_copy_artwork && ClipboardArtworkFormat::Png.is_supported();
        let can_copy_webp =
            !file_busy && can_copy_artwork && ClipboardArtworkFormat::WebP.is_supported();
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
        let canvas_viewport = glam::Vec2::new(
            canvas_viewport_width(
                window_size.width.0,
                show_layers_panel,
                layers_panel_width,
                show_design_panel,
                design_panel_width,
            ),
            (window_size.height.0 - toolbar::HEADER_HEIGHT - status_bar::HEIGHT).max(1.0),
        );
        self.editor.set_viewport_size(canvas_viewport);
        if self.fit_artwork_on_first_layout {
            self.editor.zoom_to_fit();
            self.fit_artwork_on_first_layout = false;
        }

        let precision_visibility = precision_ui::PrecisionVisibility {
            rulers: self.workspace_preferences.show_rulers,
            grid: self.workspace_preferences.show_grid,
            guides: self.workspace_preferences.show_guides,
            guides_locked: self.workspace_preferences.guides_locked,
        };
        let document_grid = self.editor.document().grid;
        let precision_grid = precision_ui::GridSettings {
            spacing: document_grid.spacing,
            major_every: document_grid.major_every,
        };
        let precision_snapshot = precision_ui::PrecisionUiSnapshot {
            popover_open: self.precision_menu_open,
            snapping: precision_ui::SnappingSettings {
                enabled: self.workspace_preferences.snapping_enabled,
                objects: self.workspace_preferences.snap_to_objects,
                guides: self.workspace_preferences.snap_to_guides,
                grid: self.workspace_preferences.snap_to_grid,
                tolerance_px: self.workspace_preferences.snap_tolerance,
            },
            visibility: precision_visibility,
            grid: precision_grid,
            guide_count: self.editor.document().guides.len(),
        };
        let mut precision_guides = self
            .editor
            .document()
            .guides
            .iter()
            .map(|guide| {
                let position = self.guide_drag.map_or(guide.position, |drag| {
                    if drag.source == Some(guide.id) && !drag.duplicate {
                        drag.position
                    } else {
                        guide.position
                    }
                });
                precision_ui::GuideOverlay {
                    axis: match guide.axis {
                        editor_core::GuideAxis::Horizontal => precision_ui::OverlayAxis::Horizontal,
                        editor_core::GuideAxis::Vertical => precision_ui::OverlayAxis::Vertical,
                    },
                    position,
                    active: self.active_guide == Some(guide.id),
                }
            })
            .collect::<Vec<_>>();
        if let Some(drag) = self
            .guide_drag
            .filter(|drag| drag.source.is_none() || drag.duplicate)
        {
            precision_guides.push(precision_ui::GuideOverlay {
                axis: match drag.axis {
                    editor_core::GuideAxis::Horizontal => precision_ui::OverlayAxis::Horizontal,
                    editor_core::GuideAxis::Vertical => precision_ui::OverlayAxis::Vertical,
                },
                position: drag.position,
                active: true,
            });
        }
        let precision_overlay = precision_ui::build_precision_overlay_geometry(
            &precision_ui::PrecisionOverlaySnapshot {
                viewport: precision_ui::ScreenRect::from_size(canvas_viewport),
                view: precision_ui::OverlayView {
                    pan: self.editor.view().pan,
                    zoom: self.editor.view().zoom,
                },
                visibility: precision_visibility,
                grid: precision_grid,
                guides: precision_guides,
                ruler_thickness: precision_ui::DEFAULT_RULER_THICKNESS,
            },
        );

        let text_layouts =
            canvas::shape_text_layouts(self.editor.document().text_items_for_layout(), window);
        self.editor.set_text_layouts(text_layouts);
        let color_input = self
            .property_color_input
            .as_ref()
            .filter(|input| input.scope == ColorInputScope::Selection)
            .map(|input| properties_panel::ColorInputSnapshot {
                target: input.target,
                value: input.picker.hex_value().to_owned(),
                invalid: input.picker.invalid(),
            });
        let creation_color_input = self
            .property_color_input
            .as_ref()
            .filter(|input| input.scope == ColorInputScope::Creation)
            .map(|input| toolbar::CreationColorInputSnapshot {
                target: input.target,
                value: input.picker.hex_value().to_owned(),
                invalid: input.picker.invalid(),
            });
        let color_library = &self.editor.document().color_library;
        let has_picker_saved_colors = !color_library.colors.is_empty();
        let picker_saved_query = self
            .property_color_input
            .as_ref()
            .map_or("", |input| input.picker.saved_search())
            .trim()
            .to_lowercase();
        let current_picker_rgba = self
            .property_color_input
            .as_ref()
            .and_then(|input| rgba_color_from_paint(input.picker.paint()).ok());
        let picker_saved_color_groups = build_picker_saved_color_groups(
            color_library,
            &picker_saved_query,
            current_picker_rgba,
        );
        let color_picker = self.property_color_input.as_ref().map(|input| {
            let title = match (input.scope, input.target) {
                (ColorInputScope::Selection, properties_panel::ColorTarget::Fill) => "Fill color",
                (ColorInputScope::Selection, properties_panel::ColorTarget::Stroke) => {
                    "Stroke color"
                }
                (ColorInputScope::Selection, properties_panel::ColorTarget::FrameBackground) => {
                    "Frame background"
                }
                (ColorInputScope::Creation, properties_panel::ColorTarget::Fill) => {
                    "New shape fill"
                }
                (ColorInputScope::Creation, properties_panel::ColorTarget::Stroke) => {
                    "New shape stroke"
                }
                (ColorInputScope::Creation, properties_panel::ColorTarget::FrameBackground) => {
                    "Frame background"
                }
            };
            input
                .picker
                .snapshot(title)
                .with_saved_colors(picker_saved_color_groups, has_picker_saved_colors)
                .with_save_feedback(input.save_feedback.clone())
        });
        let numeric_input = self.numeric_property_input.as_ref().map(|input| {
            properties_panel::NumericInputSnapshot {
                target: input.target,
                value: input.value.clone(),
                invalid: input.invalid,
            }
        });
        let properties_snapshot =
            properties_panel::snapshot(&mut self.editor, color_input, numeric_input);
        let view = *self.editor.view();
        let (artwork_key, world_snapshot, requires_raster) = self.artwork_snapshot_for_render();
        let display_list = if requires_raster {
            self.editor.build_overlay_display_list()
        } else {
            self.editor.build_display_list()
        };
        let interactive_raster = self.editor.has_active_artwork_gesture()
            || self.numeric_property_scrub.is_some()
            || !self.artwork_image_cache.has_artwork_for_key(artwork_key);
        let artwork_snapshot = if requires_raster {
            world_snapshot.map(|snapshot| {
                if let Some(request) = canvas::artwork_raster_request(
                    artwork_key,
                    Arc::clone(&snapshot),
                    view,
                    window.scale_factor(),
                    interactive_raster,
                ) {
                    self.ensure_artwork_raster(request, interactive_raster, window, cx);
                }
                artwork_key
            })
        } else {
            self.artwork_raster_scheduler.clear();
            self.artwork_raster_task.take();
            None
        };
        let zoom_input = self
            .zoom_input
            .as_ref()
            .map(|input| toolbar::ZoomInputSnapshot {
                value: input.value.as_str(),
                invalid: input.invalid,
            });

        // Build layer entries for panel
        let layer_entries = self.editor.build_layer_tree();
        let canvas_child_index = usize::from(show_layers_panel);
        let editor_entity = cx.entity().downgrade();
        let middle_pan_entity = editor_entity.clone();
        let middle_pan_handler: canvas::WindowMouseMoveHandler =
            Rc::new(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.pressed_button == Some(MouseButton::Middle)
                {
                    middle_pan_entity
                        .update(cx, |editor, cx| editor.on_mouse_move(event, window, cx))
                        .ok();
                }
            });
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
            .when(can_copy_svg, |root| {
                root.on_action(cx.listener(Self::copy_as_svg))
            })
            .when(can_copy_png, |root| {
                root.on_action(cx.listener(Self::copy_as_png))
            })
            .when(can_copy_webp, |root| {
                root.on_action(cx.listener(Self::copy_as_webp))
            })
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
            .on_action(cx.listener(Self::toggle_color_library))
            .on_action(cx.listener(Self::toggle_precision_menu))
            .on_action(cx.listener(Self::toggle_rulers))
            .on_action(cx.listener(Self::toggle_grid))
            .on_action(cx.listener(Self::toggle_guides))
            .on_action(cx.listener(Self::toggle_guide_lock))
            .on_action(cx.listener(Self::toggle_snapping))
            .on_action(cx.listener(Self::toggle_snap_objects))
            .on_action(cx.listener(Self::toggle_snap_guides))
            .on_action(cx.listener(Self::toggle_snap_grid))
            .on_action(cx.listener(Self::decrease_snap_tolerance))
            .on_action(cx.listener(Self::increase_snap_tolerance))
            .on_action(cx.listener(Self::decrease_grid_spacing))
            .on_action(cx.listener(Self::increase_grid_spacing))
            .on_action(cx.listener(Self::decrease_grid_major_interval))
            .on_action(cx.listener(Self::increase_grid_major_interval))
            .on_action(cx.listener(Self::clear_guides))
            .on_action(cx.listener(Self::save_current_color))
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
            .on_action(cx.listener(Self::start_frame_background_color_input))
            .on_action(cx.listener(Self::set_layout_free))
            .on_action(cx.listener(Self::set_layout_horizontal))
            .on_action(cx.listener(Self::set_layout_vertical))
            .on_action(cx.listener(Self::property_rotate_left))
            .on_action(cx.listener(Self::property_rotate_right))
            .on_action(cx.listener(Self::property_fill_none))
            .on_action(cx.listener(Self::property_fill_white))
            .on_action(cx.listener(Self::property_fill_black))
            .on_action(cx.listener(Self::property_fill_blue))
            .on_action(cx.listener(Self::property_fill_red))
            .on_action(cx.listener(Self::property_fill_green))
            .on_action(cx.listener(Self::start_fill_color_input))
            .on_action(cx.listener(Self::start_stroke_color_input))
            .on_action(cx.listener(Self::property_toggle_stroke))
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
                    dirty: self.document_is_dirty(),
                },
                tool,
                toolbar::ZoomState {
                    level: zoom,
                    input: zoom_input,
                },
                toolbar::HeaderState {
                    panels: panel_visibility,
                    history: toolbar::HistoryAvailability {
                        undo: self.editor.can_undo_in_context(),
                        redo: self.editor.can_redo_in_context(),
                    },
                    utilities: toolbar::UtilityState {
                        snapping: self.workspace_preferences.snapping_enabled,
                        precision_open: self.precision_menu_open,
                        color_library_open: self.color_library_open,
                    },
                    open_menu,
                },
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
                            .debug_selector(|| "canvas".to_owned())
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
                            .on_mouse_move(cx.listener(Self::on_canvas_mouse_move))
                            .on_scroll_wheel(cx.listener(Self::on_scroll))
                            // Keep the grid beneath artwork. Guides and rulers are
                            // rendered by the foreground precision overlay below.
                            .child(precision_ui::render_precision_grid(
                                precision_overlay.clone(),
                            ))
                            .child(canvas::render_canvas(
                                display_list,
                                artwork_snapshot,
                                view,
                                self.text_image_cache.clone(),
                                self.artwork_image_cache.clone(),
                                middle_pan_handler,
                            ))
                            .child(precision_ui::render_precision_overlay(precision_overlay))
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
                        utilities: toolbar::UtilityState {
                            snapping: self.workspace_preferences.snapping_enabled,
                            precision_open: self.precision_menu_open,
                            color_library_open: self.color_library_open,
                        },
                        show_rulers: self.workspace_preferences.show_rulers,
                        show_grid: self.workspace_preferences.show_grid,
                        show_guides: self.workspace_preferences.show_guides,
                        guides_locked: self.workspace_preferences.guides_locked,
                        can_clear_guides: !self.editor.document().guides.is_empty(),
                        can_paste,
                        recent_files: &recent_files,
                        file_busy,
                        can_copy_artwork,
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
            .when(self.precision_menu_open, |root| {
                root.child(precision_ui::render_precision_popover(
                    precision_snapshot,
                    precision_callbacks(editor_entity.clone()),
                    precision_ui::PrecisionPopoverPlacement {
                        top: toolbar::HEADER_HEIGHT + 8.0,
                        right: visible_panel_width(show_design_panel, design_panel_width) + 44.0,
                        max_height: (window_size.height.0 - toolbar::HEADER_HEIGHT - 16.0).max(1.0),
                    },
                ))
            })
            .when_some(guide_position_input, |root, input| {
                root.child(render_guide_position_input(input))
            })
            .when_some(color_picker, |root, snapshot| {
                root.child(color_picker::render(
                    snapshot,
                    color_picker::ColorPickerPlacement {
                        top: toolbar::HEADER_HEIGHT + 12.0,
                        right: visible_panel_width(show_design_panel, design_panel_width) + 12.0,
                        viewport_width: window_size.width.0,
                        max_height: (window_size.height.0 - 16.0).max(1.0),
                    },
                    editor_entity.clone(),
                ))
            })
            .when_some(command_palette, |root, palette| root.child(palette))
            .when_some(color_library_panel, |root, panel| root.child(panel))
    }
}

fn precision_callbacks(editor_entity: gpui::WeakEntity<Strek>) -> precision_ui::PrecisionCallbacks {
    let popover_entity = editor_entity.clone();
    let snapping_entity = editor_entity.clone();
    let target_entity = editor_entity.clone();
    let tolerance_entity = editor_entity.clone();
    let overlay_entity = editor_entity.clone();
    let lock_entity = editor_entity.clone();
    let spacing_entity = editor_entity.clone();
    let major_entity = editor_entity.clone();

    precision_ui::PrecisionCallbacks::default()
        .on_popover_open_changed(move |open, _, cx| {
            popover_entity
                .update(cx, |editor, cx| {
                    editor.precision_menu_open = open;
                    cx.notify();
                })
                .ok();
        })
        .on_snapping_enabled_changed(move |enabled, _, cx| {
            snapping_entity
                .update(cx, |editor, cx| {
                    editor.workspace_preferences.snapping_enabled = enabled;
                    editor.persist_workspace_preferences(cx);
                })
                .ok();
        })
        .on_snap_target_changed(move |target, enabled, _, cx| {
            target_entity
                .update(cx, |editor, cx| {
                    match target {
                        precision_ui::SnapTarget::Objects => {
                            editor.workspace_preferences.snap_to_objects = enabled;
                        }
                        precision_ui::SnapTarget::Guides => {
                            editor.workspace_preferences.snap_to_guides = enabled;
                        }
                        precision_ui::SnapTarget::Grid => {
                            editor.workspace_preferences.snap_to_grid = enabled;
                        }
                    }
                    editor.persist_workspace_preferences(cx);
                })
                .ok();
        })
        .on_tolerance_changed(move |tolerance, _, cx| {
            tolerance_entity
                .update(cx, |editor, cx| {
                    editor.workspace_preferences.snap_tolerance = tolerance.clamp(1.0, 32.0);
                    editor.persist_workspace_preferences(cx);
                })
                .ok();
        })
        .on_overlay_visibility_changed(move |overlay, visible, _, cx| {
            overlay_entity
                .update(cx, |editor, cx| {
                    match overlay {
                        precision_ui::PrecisionOverlay::Rulers => {
                            editor.workspace_preferences.show_rulers = visible;
                        }
                        precision_ui::PrecisionOverlay::Grid => {
                            editor.workspace_preferences.show_grid = visible;
                        }
                        precision_ui::PrecisionOverlay::Guides => {
                            editor.workspace_preferences.show_guides = visible;
                            if !visible {
                                editor.active_guide = None;
                                editor.guide_drag = None;
                                editor.guide_position_input = None;
                            }
                        }
                    }
                    editor.persist_workspace_preferences(cx);
                })
                .ok();
        })
        .on_guides_locked_changed(move |locked, _, cx| {
            lock_entity
                .update(cx, |editor, cx| {
                    editor.workspace_preferences.guides_locked = locked;
                    if locked {
                        editor.active_guide = None;
                        editor.guide_drag = None;
                        editor.guide_position_input = None;
                    }
                    editor.persist_workspace_preferences(cx);
                })
                .ok();
        })
        .on_grid_spacing_changed(move |spacing, _, cx| {
            spacing_entity
                .update(cx, |editor, cx| {
                    let current = editor.editor.document().grid;
                    if let Ok(settings) =
                        editor_core::GridSettings::new(spacing, current.major_every)
                    {
                        let _ = editor.editor.set_grid_settings(settings);
                        cx.notify();
                    }
                })
                .ok();
        })
        .on_grid_major_every_changed(move |major_every, _, cx| {
            major_entity
                .update(cx, |editor, cx| {
                    let current = editor.editor.document().grid;
                    if let Ok(settings) =
                        editor_core::GridSettings::new(current.spacing, major_every)
                    {
                        let _ = editor.editor.set_grid_settings(settings);
                        cx.notify();
                    }
                })
                .ok();
        })
        .on_clear_guides(move |_, cx| {
            editor_entity
                .update(cx, |editor, cx| {
                    if editor.editor.clear_guides() {
                        cx.notify();
                    }
                })
                .ok();
        })
}

fn render_guide_position_input(input: GuidePositionInput) -> impl IntoElement {
    div()
        .id("guide-position-input")
        .absolute()
        .top(px(toolbar::HEADER_HEIGHT + 10.0))
        .left(px(36.0))
        .w(px(190.0))
        .p(px(8.0))
        .flex()
        .items_center()
        .gap(px(7.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(rgb(if input.invalid { 0xf24822 } else { 0x0c8ce9 }))
        .bg(rgb(0x202124))
        .shadow_lg()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(px(9.0))
                .text_color(rgb(0xa8abb2))
                .child("Guide"),
        )
        .child(
            div()
                .flex_1()
                .h(px(26.0))
                .px(px(7.0))
                .flex()
                .items_center()
                .rounded(px(4.0))
                .bg(rgb(0x17181a))
                .font_family(crate::typography::MONOSPACE_FONT_FAMILY)
                .text_size(px(10.0))
                .text_color(rgb(if input.invalid { 0xfca58f } else { 0xf1f3f4 }))
                .child(input.value),
        )
        .child(
            div()
                .text_size(px(9.0))
                .text_color(rgb(0xa8abb2))
                .child("px"),
        )
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

fn guide_axis_value(axis: editor_core::GuideAxis, position: glam::Vec2) -> f32 {
    match axis {
        editor_core::GuideAxis::Horizontal => position.y,
        editor_core::GuideAxis::Vertical => position.x,
    }
}

fn first_library_color_source(
    picker: Option<editor_core::Paint>,
    selection: Option<editor_core::Paint>,
    creation: Option<editor_core::Paint>,
) -> Option<editor_core::Paint> {
    picker.or(selection).or(creation)
}

fn build_color_library_snapshot(
    document: &editor_core::Document,
    quick_add_enabled: bool,
    quick_add_group: Option<editor_core::ColorGroupId>,
) -> color_library_panel::ColorLibrarySnapshot {
    use color_library_panel::{
        ColorLibraryColorId, ColorLibraryGroupId, ColorLibraryRow, ColorLibrarySection,
        ColorLibrarySectionId, ColorLibrarySnapshot, ColorLibrarySort,
    };

    let library = &document.color_library;
    let rows = |group_id| {
        library
            .colors_in_group(group_id)
            .into_iter()
            .map(|color| ColorLibraryRow {
                id: ColorLibraryColorId(color.id.get()),
                name: color.name.clone().map(Into::into),
                rgba: color.rgba.components(),
                value: color.rgba.hex_label().into(),
                manual_order: color.manual_order as usize,
            })
            .collect()
    };
    let mut sections = vec![ColorLibrarySection {
        id: ColorLibrarySectionId::Ungrouped,
        name: "Ungrouped".into(),
        sort: ColorLibrarySort::default(),
        rows: rows(None),
    }];
    let mut groups = library.groups.iter().collect::<Vec<_>>();
    groups.sort_by_key(|group| (group.manual_order, group.id));
    sections.extend(groups.into_iter().map(|group| ColorLibrarySection {
        id: ColorLibrarySectionId::Group(ColorLibraryGroupId(group.id.get())),
        name: group.name.clone().into(),
        sort: ColorLibrarySort {
            mode: panel_color_sort_mode(group.sort_mode),
            direction: panel_color_sort_direction(group.sort_direction),
        },
        rows: rows(Some(group.id)),
    }));
    let quick_add_destination = quick_add_group
        .and_then(|id| library.group(id))
        .map_or_else(|| "Ungrouped".into(), |group| group.name.clone().into());
    ColorLibrarySnapshot {
        sections,
        quick_add_enabled,
        quick_add_group: quick_add_group.map(|id| ColorLibraryGroupId(id.get())),
        quick_add_destination,
    }
}

fn color_library_presentation(
    preferences: &workspace_preferences::WorkspacePreferences,
) -> color_library_panel::ColorLibraryPanelPresentation {
    let panel = preferences.color_library_panel;
    color_library_panel::ColorLibraryPanelPresentation {
        geometry: color_library_panel::ColorLibraryPanelGeometry {
            left: panel.x,
            top: panel.y,
            width: panel.width,
            height: panel.height,
        },
        collapsed_sections: preferences
            .collapsed_color_groups
            .iter()
            .filter_map(|value| {
                if value == "ungrouped" {
                    Some(color_library_panel::ColorLibrarySectionId::Ungrouped)
                } else {
                    value
                        .strip_prefix("group-")?
                        .parse::<u64>()
                        .ok()
                        .map(color_library_panel::ColorLibraryGroupId)
                        .map(color_library_panel::ColorLibrarySectionId::Group)
                }
            })
            .collect(),
    }
}

fn color_group_id(
    id: color_library_panel::ColorLibraryGroupId,
) -> Result<editor_core::ColorGroupId, String> {
    editor_core::ColorGroupId::from_opaque(id.0).ok_or_else(|| "invalid color group ID".to_owned())
}

fn saved_color_id(
    id: color_library_panel::ColorLibraryColorId,
) -> Result<editor_core::SavedColorId, String> {
    editor_core::SavedColorId::from_opaque(id.0).ok_or_else(|| "invalid saved color ID".to_owned())
}

fn color_section_group(
    section: color_library_panel::ColorLibrarySectionId,
) -> Result<Option<editor_core::ColorGroupId>, String> {
    match section {
        color_library_panel::ColorLibrarySectionId::Ungrouped => Ok(None),
        color_library_panel::ColorLibrarySectionId::Group(id) => color_group_id(id).map(Some),
    }
}

fn panel_color_sort_mode(
    mode: editor_core::ColorSortMode,
) -> color_library_panel::ColorLibrarySortMode {
    match mode {
        editor_core::ColorSortMode::Manual => color_library_panel::ColorLibrarySortMode::Manual,
        editor_core::ColorSortMode::Name => color_library_panel::ColorLibrarySortMode::Name,
        editor_core::ColorSortMode::HueAndShades => {
            color_library_panel::ColorLibrarySortMode::HueAndShades
        }
        editor_core::ColorSortMode::Lightness => {
            color_library_panel::ColorLibrarySortMode::Lightness
        }
        editor_core::ColorSortMode::Chroma => color_library_panel::ColorLibrarySortMode::Chroma,
        editor_core::ColorSortMode::Brightness => {
            color_library_panel::ColorLibrarySortMode::Brightness
        }
    }
}

fn core_color_sort_mode(
    mode: color_library_panel::ColorLibrarySortMode,
) -> editor_core::ColorSortMode {
    match mode {
        color_library_panel::ColorLibrarySortMode::Manual => editor_core::ColorSortMode::Manual,
        color_library_panel::ColorLibrarySortMode::Name => editor_core::ColorSortMode::Name,
        color_library_panel::ColorLibrarySortMode::HueAndShades => {
            editor_core::ColorSortMode::HueAndShades
        }
        color_library_panel::ColorLibrarySortMode::Lightness => {
            editor_core::ColorSortMode::Lightness
        }
        color_library_panel::ColorLibrarySortMode::Chroma => editor_core::ColorSortMode::Chroma,
        color_library_panel::ColorLibrarySortMode::Brightness => {
            editor_core::ColorSortMode::Brightness
        }
    }
}

fn panel_color_sort_direction(
    direction: editor_core::SortDirection,
) -> color_library_panel::ColorLibrarySortDirection {
    match direction {
        editor_core::SortDirection::Ascending => {
            color_library_panel::ColorLibrarySortDirection::Ascending
        }
        editor_core::SortDirection::Descending => {
            color_library_panel::ColorLibrarySortDirection::Descending
        }
    }
}

fn core_color_sort_direction(
    direction: color_library_panel::ColorLibrarySortDirection,
) -> editor_core::SortDirection {
    match direction {
        color_library_panel::ColorLibrarySortDirection::Ascending => {
            editor_core::SortDirection::Ascending
        }
        color_library_panel::ColorLibrarySortDirection::Descending => {
            editor_core::SortDirection::Descending
        }
    }
}

fn rgba_color_from_paint(paint: &editor_core::Paint) -> Result<editor_core::RgbaColor, String> {
    let Some(rgba) = paint.solid_color() else {
        return Err("gradient paints cannot be stored as a solid saved color".to_owned());
    };
    editor_core::RgbaColor::from_array(rgba).map_err(|error| error.to_string())
}

fn build_picker_saved_color_groups(
    library: &editor_core::ColorLibrary,
    query: &str,
    current: Option<editor_core::RgbaColor>,
) -> Vec<color_picker::SavedColorGroupOption> {
    const MAX_PICKER_COLORS: usize = 128;

    let query = query.trim().to_lowercase();
    let mut keyboard_index = 0;
    let mut groups = Vec::new();
    for group in &library.groups {
        append_picker_saved_color_group(
            &mut groups,
            library,
            Some(group.id),
            &group.name,
            &query,
            current,
            &mut keyboard_index,
            MAX_PICKER_COLORS,
        );
    }
    append_picker_saved_color_group(
        &mut groups,
        library,
        None,
        "Ungrouped",
        &query,
        current,
        &mut keyboard_index,
        MAX_PICKER_COLORS,
    );
    groups
}

#[allow(clippy::too_many_arguments)]
fn append_picker_saved_color_group(
    groups: &mut Vec<color_picker::SavedColorGroupOption>,
    library: &editor_core::ColorLibrary,
    group_id: Option<editor_core::ColorGroupId>,
    group_name: &str,
    query: &str,
    current: Option<editor_core::RgbaColor>,
    keyboard_index: &mut usize,
    limit: usize,
) {
    if *keyboard_index >= limit {
        return;
    }
    let group_matches = !query.is_empty() && group_name.to_lowercase().contains(query);
    let mut colors = Vec::new();
    for color in library.colors_in_group(group_id) {
        if *keyboard_index >= limit {
            break;
        }
        let value = color.rgba.hex_label();
        let rgb_value = rgb_search_label(color.rgba.components());
        let color_matches = query.is_empty()
            || group_matches
            || value.to_lowercase().contains(query)
            || rgb_value.contains(query)
            || color
                .name
                .as_ref()
                .is_some_and(|name| name.to_lowercase().contains(query));
        if !color_matches {
            continue;
        }
        colors.push(color_picker::SavedColorOption {
            id: color.id.get(),
            name: color.name.clone().map(Into::into),
            rgba: color.rgba.components(),
            value: value.into(),
            keyboard_index: *keyboard_index,
            exact_match: current == Some(color.rgba),
        });
        *keyboard_index += 1;
    }
    if !colors.is_empty() {
        groups.push(color_picker::SavedColorGroupOption {
            name: group_name.to_owned().into(),
            colors,
        });
    }
}

fn rgb_search_label(rgba: [f32; 4]) -> String {
    let channels = rgba.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!(
        "rgb({}, {}, {}) rgba({}, {}, {}, {})",
        channels[0], channels[1], channels[2], channels[0], channels[1], channels[2], channels[3]
    )
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

fn state_write_close_action(
    flush_in_progress: bool,
    workspace_preferences_pending: bool,
    recent_files_pending: bool,
) -> StateWriteCloseAction {
    if flush_in_progress {
        StateWriteCloseAction::WaitForFlush
    } else if workspace_preferences_pending || recent_files_pending {
        StateWriteCloseAction::StartFlush
    } else {
        StateWriteCloseAction::CloseNow
    }
}

fn editor_preserving_creation_styles(current: &Editor, mut replacement: Editor) -> Editor {
    replacement.set_creation_style_defaults(current.creation_style_defaults().clone());
    replacement
}

fn color_input_paint(
    editor: &Editor,
    scope: ColorInputScope,
    target: properties_panel::ColorTarget,
) -> Option<editor_core::Paint> {
    let paint = match (scope, target) {
        (ColorInputScope::Selection, properties_panel::ColorTarget::Fill) => editor
            .selection()
            .primary()
            .and_then(|id| editor.document().get(id))
            .map(|node| {
                node.style
                    .fill
                    .clone()
                    .unwrap_or_else(editor_core::Paint::black)
            }),
        (ColorInputScope::Selection, properties_panel::ColorTarget::Stroke) => editor
            .selection()
            .primary()
            .and_then(|id| editor.document().get(id))
            .map(|node| {
                node.style
                    .stroke
                    .as_ref()
                    .map(|stroke| stroke.paint.clone())
                    .unwrap_or_else(editor_core::Paint::black)
            }),
        (ColorInputScope::Selection, properties_panel::ColorTarget::FrameBackground) => editor
            .selected_frame_data()
            .map(|frame| frame.background.unwrap_or_else(editor_core::Paint::white)),
        (ColorInputScope::Creation, properties_panel::ColorTarget::Fill) => editor
            .active_creation_style()
            .map(|style| style.fill.clone().unwrap_or_else(editor_core::Paint::black)),
        (ColorInputScope::Creation, properties_panel::ColorTarget::Stroke) => {
            editor.active_creation_style().map(|style| {
                style
                    .stroke
                    .as_ref()
                    .map(|stroke| stroke.paint.clone())
                    .unwrap_or_else(editor_core::Paint::black)
            })
        }
        (ColorInputScope::Creation, properties_panel::ColorTarget::FrameBackground) => None,
    };
    paint.filter(|paint| matches!(paint, editor_core::Paint::Solid(_)))
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
        (ColorInputScope::Selection, properties_panel::ColorTarget::FrameBackground) => {
            editor.set_selected_frame_background(Some(paint))
        }
        (ColorInputScope::Creation, properties_panel::ColorTarget::Fill) => {
            editor.set_creation_fill(Some(paint))
        }
        (ColorInputScope::Creation, properties_panel::ColorTarget::Stroke) => {
            editor.set_creation_stroke_paint(paint)
        }
        (ColorInputScope::Creation, properties_panel::ColorTarget::FrameBackground) => false,
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

fn format_numeric_property_input(target: NumericPropertyTarget, value: f32) -> String {
    let displayed = match target {
        NumericPropertyTarget::Opacity => value * 100.0,
        _ => value,
    };
    properties_panel::format_number(displayed)
}

fn parse_numeric_property_input(target: NumericPropertyTarget, value: &str) -> Option<f32> {
    let parsed: f32 = value
        .trim()
        .trim_end_matches('%')
        .trim()
        .replace(',', ".")
        .parse()
        .ok()?;
    let value = match target {
        NumericPropertyTarget::Opacity => parsed / 100.0,
        _ => parsed,
    };
    value.is_finite().then_some(value)
}

fn numeric_property_input_value_is_valid(target: NumericPropertyTarget, value: f32) -> bool {
    value.is_finite()
        && (!matches!(
            target,
            NumericPropertyTarget::AutoLayoutSpacing | NumericPropertyTarget::AutoLayoutPadding
        ) || value >= 0.0)
}

fn sanitize_numeric_property_input(value: &str) -> String {
    let mut sanitized = String::with_capacity(16);
    append_numeric_property_input(&mut sanitized, value);
    sanitized
}

fn append_numeric_property_input(value: &mut String, text: &str) {
    for character in text.chars() {
        if character.is_ascii_digit() && value.len() < 16 {
            value.push(character);
        } else if matches!(character, '.' | ',') && !value.contains('.') && value.len() < 16 {
            value.push('.');
        } else if character == '-' && value.is_empty() {
            value.push(character);
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

fn automation_node_id(id: NodeId) -> String {
    format!("node-{:016x}", id.to_opaque())
}

fn automation_absolute_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    path.is_absolute()
        .then_some(path)
        .ok_or_else(|| format!("automation paths must be absolute; received `{value}`"))
}

fn parse_automation_node_id(
    document: &editor_core::Document,
    value: &str,
) -> Result<NodeId, String> {
    let encoded = value
        .strip_prefix("node-")
        .ok_or_else(|| format!("invalid layer ID `{value}`"))?;
    let opaque =
        u64::from_str_radix(encoded, 16).map_err(|_| format!("invalid layer ID `{value}`"))?;
    let id = NodeId::from_opaque(opaque);
    document
        .get(id)
        .filter(|node| !node.deleted)
        .map(|_| id)
        .ok_or_else(|| format!("layer `{value}` does not exist in the current document"))
}

fn automation_node_kind(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Group => "group",
        NodeKind::Frame(_) => "frame",
        NodeKind::Shape(_) => "shape",
        NodeKind::Text(_) => "text",
    }
}

fn automation_numeric_property(property: automation::NumericProperty) -> NumericPropertyTarget {
    match property {
        automation::NumericProperty::WorldX => NumericPropertyTarget::WorldX,
        automation::NumericProperty::WorldY => NumericPropertyTarget::WorldY,
        automation::NumericProperty::Opacity => NumericPropertyTarget::Opacity,
        automation::NumericProperty::StrokeWidth => NumericPropertyTarget::StrokeWidth,
        automation::NumericProperty::TextSize => NumericPropertyTarget::TextSize,
        automation::NumericProperty::LayoutSpacing => NumericPropertyTarget::AutoLayoutSpacing,
        automation::NumericProperty::LayoutPadding => NumericPropertyTarget::AutoLayoutPadding,
    }
}

fn automation_guide_axis(axis: automation::GuideAxis) -> editor_core::GuideAxis {
    match axis {
        automation::GuideAxis::Horizontal => editor_core::GuideAxis::Horizontal,
        automation::GuideAxis::Vertical => editor_core::GuideAxis::Vertical,
    }
}

fn automation_color_group_id(id: Option<u64>) -> Result<editor_core::ColorGroupId, String> {
    id.and_then(editor_core::ColorGroupId::from_opaque)
        .ok_or_else(|| "a valid color group id is required".to_owned())
}

fn automation_saved_color_id(id: Option<u64>) -> Result<editor_core::SavedColorId, String> {
    id.and_then(editor_core::SavedColorId::from_opaque)
        .ok_or_else(|| "a valid saved color id is required".to_owned())
}

fn automation_rgba_color(value: Option<&str>) -> Result<editor_core::RgbaColor, String> {
    let value = value.ok_or_else(|| "a hexadecimal color value is required".to_owned())?;
    let paint = color_picker::parse_hex_paint(value)
        .ok_or_else(|| "color must be a 3, 4, 6, or 8 digit hexadecimal value".to_owned())?;
    rgba_color_from_paint(&paint)
}

fn automation_color_sort_name(mode: editor_core::ColorSortMode) -> &'static str {
    match mode {
        editor_core::ColorSortMode::Manual => "manual",
        editor_core::ColorSortMode::Name => "name",
        editor_core::ColorSortMode::HueAndShades => "hue_and_shades",
        editor_core::ColorSortMode::Lightness => "lightness",
        editor_core::ColorSortMode::Chroma => "chroma",
        editor_core::ColorSortMode::Brightness => "brightness",
    }
}

fn automation_export_format(format: automation::ArtifactFormat) -> export::ExportFormat {
    match format {
        automation::ArtifactFormat::Svg => export::ExportFormat::Svg,
        automation::ArtifactFormat::SvgOutlined => export::ExportFormat::SvgOutlined,
        automation::ArtifactFormat::Png => export::ExportFormat::Png,
        automation::ArtifactFormat::Jpeg => export::ExportFormat::Jpeg,
        automation::ArtifactFormat::WebP => export::ExportFormat::WebP,
    }
}

fn automation_artifact_name(format: automation::ArtifactFormat) -> &'static str {
    match format {
        automation::ArtifactFormat::Svg => "svg",
        automation::ArtifactFormat::SvgOutlined => "svg_outlined",
        automation::ArtifactFormat::Png => "png",
        automation::ArtifactFormat::Jpeg => "jpeg",
        automation::ArtifactFormat::WebP => "webp",
    }
}

fn automation_artifact_media_type(format: automation::ArtifactFormat) -> &'static str {
    match format {
        automation::ArtifactFormat::Svg | automation::ArtifactFormat::SvgOutlined => {
            "image/svg+xml"
        }
        automation::ArtifactFormat::Png => "image/png",
        automation::ArtifactFormat::Jpeg => "image/jpeg",
        automation::ArtifactFormat::WebP => "image/webp",
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
    let launch_in_background = match args.next().as_deref() {
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
        Some("--background") => true,
        _ => false,
    };

    env_logger::init();
    let automation_requests = match automation::start_server() {
        Ok(requests) => Some(requests),
        Err(error) => {
            log::warn!(
                "automation unavailable at {}: {error}",
                automation::endpoint_display()
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
            color_library_panel::register_keybindings(cx);
            layer_name_input::register_keybindings(cx);
            let mut file_menu_items = vec![
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
            ];
            if cfg!(any(target_os = "macos", target_os = "windows")) {
                file_menu_items.push(MenuItem::separator());
                file_menu_items.push(MenuItem::action("Copy as SVG", CopyAsSvg));
                file_menu_items.push(MenuItem::action("Copy as PNG", CopyAsPng));
            }
            if cfg!(target_os = "macos") {
                file_menu_items.push(MenuItem::action("Copy as WebP", CopyAsWebP));
            }

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
                    items: file_menu_items,
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

            let window_size = size(px(1280.0), px(800.0));
            let bounds = if launch_in_background {
                background_window_bounds(window_size, cx)
            } else {
                Bounds::centered(None, window_size, cx)
            };

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
            if !launch_in_background {
                cx.activate(true);
            }
        });
}

#[cfg(target_os = "macos")]
fn background_window_bounds(
    window_size: gpui::Size<gpui::Pixels>,
    cx: &App,
) -> Bounds<gpui::Pixels> {
    let Some(display) = cx
        .displays()
        .into_iter()
        .max_by(|left, right| left.bounds().right().0.total_cmp(&right.bounds().right().0))
    else {
        return Bounds::centered(None, window_size, cx);
    };
    parked_background_bounds(window_size, display.bounds())
}

#[cfg(target_os = "macos")]
fn parked_background_bounds(
    window_size: gpui::Size<gpui::Pixels>,
    display: Bounds<gpui::Pixels>,
) -> Bounds<gpui::Pixels> {
    const VISIBLE_EDGE: f32 = 2.0;

    Bounds {
        origin: gpui::point(
            display.origin.x + display.size.width - px(VISIBLE_EDGE),
            display.origin.y,
        ),
        size: window_size,
    }
}

#[cfg(not(target_os = "macos"))]
fn background_window_bounds(
    window_size: gpui::Size<gpui::Pixels>,
    cx: &App,
) -> Bounds<gpui::Pixels> {
    Bounds::centered(None, window_size, cx)
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn background_window_leaves_only_a_two_pixel_edge_on_screen() {
        let display = Bounds {
            origin: gpui::point(px(-1920.0), px(25.0)),
            size: size(px(1920.0), px(1080.0)),
        };
        let bounds = parked_background_bounds(size(px(1280.0), px(800.0)), display);

        assert_eq!(bounds.origin.x, display.right() - px(2.0));
        assert_eq!(bounds.origin.y, display.origin.y);
        assert_eq!(bounds.size, size(px(1280.0), px(800.0)));
    }

    #[test]
    fn imported_svg_origin_requires_a_new_native_save_destination() {
        let path = std::env::temp_dir().join("logo.svg");
        let origin = DocumentOrigin::ImportedSvg(path.clone());

        assert_eq!(origin.source_path(), Some(path.as_path()));
        assert_eq!(origin.native_path(), None);
        assert!(origin.requires_native_save());
    }

    #[test]
    fn starter_origin_is_unsaved_but_discardable_until_edited() {
        let origin = DocumentOrigin::Starter;

        assert_eq!(origin.source_path(), None);
        assert_eq!(origin.native_path(), None);
        assert_eq!(origin.import_source_path(), None);
        assert!(!origin.requires_native_save());
    }

    #[test]
    fn artwork_availability_requires_effectively_visible_renderable_geometry() {
        let mut document = editor_core::Document::new();
        assert!(!document_has_visible_artwork(&document));

        document
            .add_child(
                document.root,
                editor_core::Node::shape(
                    "Hidden rectangle",
                    editor_core::PathData::rect(0.0, 0.0, 10.0, 10.0),
                )
                .with_visible(false),
            )
            .unwrap();
        assert!(!document_has_visible_artwork(&document));

        document
            .add_child(
                document.root,
                editor_core::Node::shape(
                    "Horizontal line",
                    editor_core::PathData::from_commands(&[
                        editor_core::PathCmd::MoveTo(glam::Vec2::ZERO),
                        editor_core::PathCmd::LineTo(glam::Vec2::new(10.0, 0.0)),
                    ]),
                )
                .with_style(editor_core::Style {
                    fill: None,
                    stroke: Some(editor_core::Stroke::new(
                        1.0,
                        editor_core::Paint::rgb(0.0, 0.0, 0.0),
                    )),
                    opacity: 1.0,
                    ..editor_core::Style::default()
                }),
            )
            .unwrap();
        assert!(document_has_visible_artwork(&document));
    }

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
    fn color_picker_identity_includes_its_scope_and_target() {
        let input = PropertyColorInput {
            scope: ColorInputScope::Creation,
            target: properties_panel::ColorTarget::Fill,
            picker: color_picker::ColorPickerState::new(editor_core::Paint::black()),
            save_feedback: None,
        };

        assert!(input.matches(
            ColorInputScope::Creation,
            properties_panel::ColorTarget::Fill
        ));
        assert!(!input.matches(
            ColorInputScope::Selection,
            properties_panel::ColorTarget::Fill
        ));
        assert!(!input.matches(
            ColorInputScope::Creation,
            properties_panel::ColorTarget::Stroke
        ));
    }

    #[gpui::test]
    fn saved_color_keyboard_action_commits_the_active_target(cx: &mut gpui::TestAppContext) {
        let (strek, cx) = cx.add_window_view(|_, cx| Strek::new(commands::Keymap::default(), cx));
        strek.update(cx, |strek, cx| {
            let root = strek.editor.document.root;
            let shape = strek
                .editor
                .document
                .add_child(
                    root,
                    editor_core::Node::shape(
                        "Rectangle",
                        editor_core::PathData::rect(0.0, 0.0, 20.0, 20.0),
                    ),
                )
                .unwrap();
            strek.editor.set_layer_selection([shape]);
            let rgba = editor_core::RgbaColor::new(0.2, 0.4, 0.8, 1.0).unwrap();
            let saved = strek
                .editor
                .add_saved_color(None, Some("Brand blue"), rgba)
                .unwrap();
            strek.property_color_input = Some(PropertyColorInput {
                scope: ColorInputScope::Selection,
                target: properties_panel::ColorTarget::Fill,
                picker: color_picker::ColorPickerState::new(editor_core::Paint::black()),
                save_feedback: None,
            });

            strek.handle_color_picker_keyboard_action(
                color_picker::ColorPickerKeyboardAction::ApplySavedColor {
                    id: saved.get(),
                    index: 0,
                },
                cx,
            );

            assert_eq!(
                strek.editor.document.get(shape).unwrap().style.fill,
                Some(editor_core::Paint::Solid(rgba.components()))
            );
            assert!(strek.property_color_input.is_none());
        });
    }

    #[gpui::test]
    fn middle_button_pan_updates_before_mouse_up_outside_canvas(cx: &mut gpui::TestAppContext) {
        let (strek, cx) = cx.add_window_view(|_, cx| Strek::new(commands::Keymap::default(), cx));
        cx.run_until_parked();
        let canvas = cx
            .debug_bounds("canvas")
            .expect("canvas should be rendered");
        let start = gpui::point(
            canvas.origin.x + canvas.size.width / 2.0,
            canvas.origin.y + canvas.size.height / 2.0,
        );
        let end = gpui::point(canvas.origin.x - px(10.0), start.y);
        let start_pan = cx.read(|cx| strek.read(cx).editor.view().pan);

        cx.simulate_mouse_down(start, MouseButton::Middle, gpui::Modifiers::default());
        cx.simulate_mouse_move(end, MouseButton::Middle, gpui::Modifiers::default());

        assert_eq!(
            cx.read(|cx| strek.read(cx).editor.view().pan),
            start_pan + glam::Vec2::new(end.x.0 - start.x.0, end.y.0 - start.y.0)
        );
        assert_eq!(
            cx.read(|cx| strek.read(cx).editor.interaction_kind()),
            editor_core::InteractionKind::Panning
        );
    }

    fn raster_identity(revision: u64) -> ArtworkRasterIdentity {
        (
            canvas::ArtworkCacheKey {
                document_epoch: 1,
                history_revision: revision,
                transient_revision: 0,
            },
            editor_core::Rect::new(glam::Vec2::ZERO, glam::Vec2::splat(100.0)),
            256,
            256,
        )
    }

    #[test]
    fn raster_scheduler_rejects_stale_completion_after_cached_undo() {
        let mut scheduler = ArtworkRasterScheduler::default();
        let a = raster_identity(1);
        let b = raster_identity(2);

        assert_eq!(
            scheduler.request(a, false, false),
            ArtworkRasterRequestAction::Start
        );
        assert!(scheduler.complete(a, true));
        assert_eq!(
            scheduler.request(b, false, false),
            ArtworkRasterRequestAction::Start
        );
        assert_eq!(
            scheduler.request(a, false, true),
            ArtworkRasterRequestAction::Ready
        );
        assert!(!scheduler.complete(b, true));
    }

    #[test]
    fn raster_scheduler_replaces_settled_work_but_coalesces_interactive_work() {
        let mut scheduler = ArtworkRasterScheduler::default();
        let settled = raster_identity(1);
        let interactive = raster_identity(2);
        let latest = raster_identity(3);

        assert_eq!(
            scheduler.request(settled, false, false),
            ArtworkRasterRequestAction::Start
        );
        assert_eq!(
            scheduler.request(interactive, true, false),
            ArtworkRasterRequestAction::Replace
        );
        assert_eq!(
            scheduler.request(latest, true, false),
            ArtworkRasterRequestAction::Wait
        );
        assert!(!scheduler.complete(interactive, false));
        assert_eq!(scheduler.failed, None);
        assert_eq!(scheduler.desired, Some(latest));
        assert_eq!(
            scheduler.request(latest, true, false),
            ArtworkRasterRequestAction::Start
        );
    }

    #[test]
    fn raster_scheduler_marks_a_rejected_upload_as_failed() {
        let mut scheduler = ArtworkRasterScheduler::default();
        let identity = raster_identity(1);

        assert_eq!(
            scheduler.request(identity, false, false),
            ArtworkRasterRequestAction::Start
        );
        assert!(scheduler.complete(identity, true));
        assert_eq!(
            scheduler.request(identity, false, true),
            ArtworkRasterRequestAction::Ready
        );
        assert!(scheduler.fail_desired(identity));
        assert_eq!(scheduler.failed, Some(identity));
        assert_eq!(
            scheduler.request(identity, false, false),
            ArtworkRasterRequestAction::Wait
        );
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

    #[test]
    fn numeric_property_input_uses_display_units_and_signed_decimals() {
        assert_eq!(
            format_numeric_property_input(NumericPropertyTarget::Opacity, 0.5),
            "50"
        );
        assert_eq!(
            parse_numeric_property_input(NumericPropertyTarget::Opacity, "50%"),
            Some(0.5)
        );
        assert_eq!(
            parse_numeric_property_input(NumericPropertyTarget::WorldX, "-12,5"),
            Some(-12.5)
        );
        assert_eq!(sanitize_numeric_property_input(" -12,5px "), "-12.5");
        assert!(!numeric_property_input_value_is_valid(
            NumericPropertyTarget::AutoLayoutSpacing,
            -1.0
        ));
        assert_eq!(
            parse_numeric_property_input(NumericPropertyTarget::TextSize, "NaN"),
            None
        );
    }

    #[test]
    fn automation_file_paths_are_unambiguous() {
        let absolute = std::env::temp_dir().join("logo.svg");
        assert_eq!(
            automation_absolute_path(&absolute.to_string_lossy()).unwrap(),
            absolute
        );
        assert!(automation_absolute_path("logo.svg").is_err());

        #[cfg(target_os = "windows")]
        for path in [r"C:\Designs\logo.svg", r"\\server\share\logo.svg"] {
            assert_eq!(automation_absolute_path(path).unwrap(), PathBuf::from(path));
        }
    }

    #[test]
    fn automation_focus_is_opt_in() {
        assert!(!FocusPolicy::Preserve.requests_focus());
        assert!(FocusPolicy::Request.requests_focus());
    }

    #[test]
    fn window_close_waits_for_pending_state_writes() {
        assert_eq!(
            state_write_close_action(false, false, false),
            StateWriteCloseAction::CloseNow
        );
        assert_eq!(
            state_write_close_action(false, true, false),
            StateWriteCloseAction::StartFlush
        );
        assert_eq!(
            state_write_close_action(false, false, true),
            StateWriteCloseAction::StartFlush
        );
        assert_eq!(
            state_write_close_action(true, false, false),
            StateWriteCloseAction::WaitForFlush
        );
    }

    #[test]
    fn color_library_source_never_falls_back_to_an_invented_color() {
        assert_eq!(first_library_color_source(None, None, None), None);

        let picker = editor_core::Paint::rgb(0.9, 0.1, 0.2);
        let selected = editor_core::Paint::rgb(0.1, 0.9, 0.2);
        assert_eq!(
            first_library_color_source(Some(picker.clone()), Some(selected), None),
            Some(picker)
        );
    }

    #[test]
    fn color_library_snapshot_names_the_explicit_quick_add_target() {
        let mut editor = Editor::new();
        let group = editor.add_color_group("Brand").unwrap();

        let snapshot = build_color_library_snapshot(editor.document(), true, Some(group));

        assert!(snapshot.quick_add_enabled);
        assert_eq!(
            snapshot.quick_add_group,
            Some(color_library_panel::ColorLibraryGroupId(group.get()))
        );
        assert_eq!(snapshot.quick_add_destination.to_string(), "Brand");
    }

    #[test]
    fn picker_saved_colors_search_groups_hex_and_rgb_and_mark_exact_rgba() {
        let mut editor = Editor::new();
        let group = editor.add_color_group("Brand blues").unwrap();
        let ocean = editor
            .add_saved_color(
                Some(group),
                Some("Ocean"),
                editor_core::RgbaColor::new(12.0 / 255.0, 140.0 / 255.0, 233.0 / 255.0, 0.5)
                    .unwrap(),
            )
            .unwrap();
        editor
            .add_saved_color(
                None,
                Some("Paper"),
                editor_core::RgbaColor::new(1.0, 1.0, 1.0, 1.0).unwrap(),
            )
            .unwrap();
        let current = editor.document().color_library.color(ocean).unwrap().rgba;

        let by_group = build_picker_saved_color_groups(
            &editor.document().color_library,
            "brand",
            Some(current),
        );
        assert_eq!(by_group.len(), 1);
        assert_eq!(by_group[0].name.to_string(), "Brand blues");
        assert!(by_group[0].colors[0].exact_match);

        let by_rgb = build_picker_saved_color_groups(
            &editor.document().color_library,
            "rgb(12, 140, 233)",
            None,
        );
        assert_eq!(by_rgb[0].colors[0].id, ocean.get());

        let by_hex =
            build_picker_saved_color_groups(&editor.document().color_library, "0c8ce980", None);
        assert_eq!(by_hex[0].colors[0].id, ocean.get());
    }
}
