//! Shared command catalog and user-configurable keyboard shortcuts.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use editor_core::EditorAction;
use gpui::{Action, App, KeyBinding, KeyBindingContextPredicate, Keystroke};
use serde_json::{json, Map, Value};

use crate::{
    AlignObjectsBottom, AlignObjectsCenter, AlignObjectsLeft, AlignObjectsMiddle,
    AlignObjectsRight, AlignObjectsTop, AlignTextCenter, AlignTextLeft, AlignTextRight, Backspace,
    BringForward, BringToFront, Copy, Cut, Delete, DeselectAll, DistributeObjectsHorizontal,
    DistributeObjectsVertical, Duplicate, EditVector, EllipseTool, ExportJpeg, ExportPng,
    ExportSvg, ExportSvgOutlined, ExportWebP, FinishEditing, FrameTool, Group, InvertSelection,
    JoinPaths, LineTool, NewDocument, OpenDocument, OpenKeyboardShortcuts, Paste, PenTool,
    QuitApplication, RectangleTool, Redo, ReversePath, SaveDocument, SaveDocumentAs, SelectAll,
    SelectTool, SendBackward, SendToBack, ShowCommandPalette, SplitPath, TextLarger, TextSmaller,
    TextTool, ToggleDesignPanel, ToggleFrameBackground, ToggleLayerPanel, TogglePathClosed, Undo,
    Ungroup, ZoomIn, ZoomOut, ZoomReset, ZoomResetAll, ZoomToFit, ZoomToSelection,
};

const KEYMAP_VERSION: u64 = 1;
const EDITOR_CONTEXT: &str = "Strek";
const TEXT_CONTEXT: &str = "StrekTextEditor";
const MENU_CONTEXT: &str = "StrekMenu";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AppCommand {
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
    QuitApplication,
    Copy,
    Cut,
    Paste,
    DeleteBackward,
    ToggleLayerPanel,
    ToggleDesignPanel,
    ShowCommandPalette,
    TextSmaller,
    TextLarger,
    AlignTextLeft,
    AlignTextCenter,
    AlignTextRight,
    ToggleFrameBackground,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CommandTarget {
    Editor(EditorAction),
    App(AppCommand),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub default_bindings: &'static [&'static str],
    pub target: CommandTarget,
}

macro_rules! editor_command {
    ($id:literal, $label:literal, $description:literal, $category:literal, [$($binding:literal),* $(,)?], $action:ident) => {
        CommandSpec {
            id: $id,
            label: $label,
            description: $description,
            category: $category,
            default_bindings: &[$($binding),*],
            target: CommandTarget::Editor(EditorAction::$action),
        }
    };
}

macro_rules! app_command {
    ($id:literal, $label:literal, $description:literal, $category:literal, [$($binding:literal),* $(,)?], $action:ident) => {
        CommandSpec {
            id: $id,
            label: $label,
            description: $description,
            category: $category,
            default_bindings: &[$($binding),*],
            target: CommandTarget::App(AppCommand::$action),
        }
    };
}

pub(crate) const COMMANDS: &[CommandSpec] = &[
    app_command!(
        "file.new",
        "New Document",
        "Create a new document",
        "File",
        ["secondary-n"],
        NewDocument
    ),
    app_command!(
        "file.open",
        "Open Document…",
        "Open a document from disk",
        "File",
        ["secondary-o"],
        OpenDocument
    ),
    app_command!(
        "file.save",
        "Save Document",
        "Save the current document",
        "File",
        ["secondary-s"],
        SaveDocument
    ),
    app_command!(
        "file.save_as",
        "Save Document As…",
        "Save the current document to a new path",
        "File",
        ["secondary-shift-s"],
        SaveDocumentAs
    ),
    app_command!(
        "file.export_svg",
        "Export as SVG…",
        "Export the visible artwork as SVG",
        "File",
        [],
        ExportSvg
    ),
    app_command!(
        "file.export_png",
        "Export as PNG…",
        "Export the visible artwork as PNG",
        "File",
        [],
        ExportPng
    ),
    app_command!(
        "file.export_svg_outlined",
        "Export SVG with Outlined Text…",
        "Export portable SVG with text converted to vector paths",
        "File",
        [],
        ExportSvgOutlined
    ),
    app_command!(
        "file.export_jpeg",
        "Export as JPEG…",
        "Export the visible artwork as an opaque JPEG",
        "File",
        [],
        ExportJpeg
    ),
    app_command!(
        "file.export_webp",
        "Export as WebP…",
        "Export the visible artwork as a lossless WebP image",
        "File",
        [],
        ExportWebP
    ),
    app_command!(
        "preferences.open_keybindings",
        "Open Keyboard Shortcuts",
        "Open the user keybindings JSON file",
        "Preferences",
        [],
        OpenKeyboardShortcuts
    ),
    app_command!(
        "application.quit",
        "Quit Strek",
        "Quit the application",
        "Application",
        ["secondary-q"],
        QuitApplication
    ),
    editor_command!(
        "edit.undo",
        "Undo",
        "Undo the last edit",
        "Edit",
        ["secondary-z"],
        Undo
    ),
    editor_command!(
        "edit.redo",
        "Redo",
        "Redo the last undone edit",
        "Edit",
        ["secondary-shift-z", "secondary-y"],
        Redo
    ),
    app_command!(
        "edit.copy",
        "Copy",
        "Copy the current selection",
        "Edit",
        ["secondary-c"],
        Copy
    ),
    app_command!(
        "edit.cut",
        "Cut",
        "Cut the current selection",
        "Edit",
        ["secondary-x"],
        Cut
    ),
    app_command!(
        "edit.paste",
        "Paste",
        "Paste clipboard contents",
        "Edit",
        ["secondary-v"],
        Paste
    ),
    app_command!(
        "edit.delete_backward",
        "Delete Backward",
        "Delete backward in text or delete selected layers",
        "Edit",
        ["backspace"],
        DeleteBackward
    ),
    editor_command!(
        "edit.delete",
        "Delete",
        "Delete the current selection",
        "Edit",
        ["delete"],
        Delete
    ),
    editor_command!(
        "edit.duplicate",
        "Duplicate",
        "Duplicate the current selection",
        "Edit",
        ["secondary-d"],
        Duplicate
    ),
    editor_command!(
        "selection.select_all",
        "Select All",
        "Select all layers in the current scope",
        "Selection",
        ["secondary-a"],
        SelectAll
    ),
    editor_command!(
        "selection.deselect_all",
        "Deselect All",
        "Clear the current selection",
        "Selection",
        ["secondary-shift-a"],
        DeselectAll
    ),
    editor_command!(
        "selection.invert",
        "Invert Selection",
        "Invert the selection in the current scope",
        "Selection",
        ["secondary-shift-i"],
        InvertSelection
    ),
    editor_command!(
        "arrange.group",
        "Group Selection",
        "Group the selected sibling layers",
        "Arrange",
        ["secondary-g"],
        Group
    ),
    editor_command!(
        "arrange.ungroup",
        "Ungroup Selection",
        "Move children out of selected groups",
        "Arrange",
        ["secondary-shift-g"],
        Ungroup
    ),
    editor_command!(
        "arrange.bring_to_front",
        "Bring to Front",
        "Move selected layers to the front",
        "Arrange",
        ["secondary-shift-]"],
        BringToFront
    ),
    editor_command!(
        "arrange.send_to_back",
        "Send to Back",
        "Move selected layers to the back",
        "Arrange",
        ["secondary-shift-["],
        SendToBack
    ),
    editor_command!(
        "arrange.bring_forward",
        "Bring Forward",
        "Move selected layers forward one step",
        "Arrange",
        ["secondary-]"],
        BringForward
    ),
    editor_command!(
        "arrange.send_backward",
        "Send Backward",
        "Move selected layers backward one step",
        "Arrange",
        ["secondary-["],
        SendBackward
    ),
    editor_command!(
        "arrange.align_left",
        "Align Left",
        "Align selected layers to the left edge",
        "Arrange",
        [],
        AlignLeft
    ),
    editor_command!(
        "arrange.align_center",
        "Align Horizontal Centers",
        "Align selected layers to their horizontal center",
        "Arrange",
        [],
        AlignCenter
    ),
    editor_command!(
        "arrange.align_right",
        "Align Right",
        "Align selected layers to the right edge",
        "Arrange",
        [],
        AlignRight
    ),
    editor_command!(
        "arrange.align_top",
        "Align Top",
        "Align selected layers to the top edge",
        "Arrange",
        [],
        AlignTop
    ),
    editor_command!(
        "arrange.align_middle",
        "Align Vertical Centers",
        "Align selected layers to their vertical center",
        "Arrange",
        [],
        AlignMiddle
    ),
    editor_command!(
        "arrange.align_bottom",
        "Align Bottom",
        "Align selected layers to the bottom edge",
        "Arrange",
        [],
        AlignBottom
    ),
    editor_command!(
        "arrange.distribute_horizontal",
        "Distribute Horizontally",
        "Make horizontal gaps equal",
        "Arrange",
        [],
        DistributeHorizontal
    ),
    editor_command!(
        "arrange.distribute_vertical",
        "Distribute Vertically",
        "Make vertical gaps equal",
        "Arrange",
        [],
        DistributeVertical
    ),
    editor_command!(
        "selection.nudge_up",
        "Nudge Up",
        "Move the selection up by one document unit",
        "Selection",
        ["up"],
        NudgeUp
    ),
    editor_command!(
        "selection.nudge_down",
        "Nudge Down",
        "Move the selection down by one document unit",
        "Selection",
        ["down"],
        NudgeDown
    ),
    editor_command!(
        "selection.nudge_left",
        "Nudge Left",
        "Move the selection left by one document unit",
        "Selection",
        ["left"],
        NudgeLeft
    ),
    editor_command!(
        "selection.nudge_right",
        "Nudge Right",
        "Move the selection right by one document unit",
        "Selection",
        ["right"],
        NudgeRight
    ),
    editor_command!(
        "selection.nudge_up_large",
        "Nudge Up 10",
        "Move the selection up by ten document units",
        "Selection",
        ["shift-up"],
        NudgeUpLarge
    ),
    editor_command!(
        "selection.nudge_down_large",
        "Nudge Down 10",
        "Move the selection down by ten document units",
        "Selection",
        ["shift-down"],
        NudgeDownLarge
    ),
    editor_command!(
        "selection.nudge_left_large",
        "Nudge Left 10",
        "Move the selection left by ten document units",
        "Selection",
        ["shift-left"],
        NudgeLeftLarge
    ),
    editor_command!(
        "selection.nudge_right_large",
        "Nudge Right 10",
        "Move the selection right by ten document units",
        "Selection",
        ["shift-right"],
        NudgeRightLarge
    ),
    editor_command!(
        "view.zoom_in",
        "Zoom In",
        "Increase canvas zoom",
        "View",
        ["secondary-=", "secondary-+"],
        ZoomIn
    ),
    editor_command!(
        "view.zoom_out",
        "Zoom Out",
        "Decrease canvas zoom",
        "View",
        ["secondary--"],
        ZoomOut
    ),
    editor_command!(
        "view.zoom_100",
        "Zoom to 100%",
        "Set canvas zoom to 100%",
        "View",
        ["secondary-1"],
        ZoomReset
    ),
    editor_command!(
        "view.reset",
        "Reset View",
        "Reset canvas pan and zoom",
        "View",
        ["secondary-0"],
        ZoomResetAll
    ),
    editor_command!(
        "view.zoom_to_fit",
        "Zoom to Fit",
        "Fit all visible artwork in the canvas",
        "View",
        ["secondary-shift-1"],
        ZoomToFit
    ),
    editor_command!(
        "view.zoom_to_selection",
        "Zoom to Selection",
        "Fit the selected layers in the canvas",
        "View",
        ["secondary-2"],
        ZoomToSelection
    ),
    app_command!(
        "view.toggle_layers",
        "Toggle Layers Panel",
        "Show or hide the Layers panel",
        "View",
        ["secondary-\\"],
        ToggleLayerPanel
    ),
    app_command!(
        "view.toggle_design",
        "Toggle Design Panel",
        "Show or hide the Design panel",
        "View",
        [],
        ToggleDesignPanel
    ),
    app_command!(
        "view.command_palette",
        "Show Command Palette",
        "Search and run editor commands",
        "View",
        ["secondary-shift-p"],
        ShowCommandPalette
    ),
    editor_command!(
        "tool.select",
        "Select Tool",
        "Select, move, and resize layers",
        "Tools",
        ["v"],
        ToolSelect
    ),
    editor_command!(
        "tool.frame",
        "Frame Tool",
        "Draw frames and adopt enclosed layers",
        "Tools",
        ["f"],
        ToolFrame
    ),
    editor_command!(
        "tool.rectangle",
        "Rectangle Tool",
        "Draw rectangles",
        "Tools",
        ["r"],
        ToolRectangle
    ),
    editor_command!(
        "tool.ellipse",
        "Ellipse Tool",
        "Draw ellipses",
        "Tools",
        ["o"],
        ToolEllipse
    ),
    editor_command!(
        "tool.line",
        "Line Tool",
        "Draw straight line segments",
        "Tools",
        ["l"],
        ToolLine
    ),
    editor_command!(
        "tool.pen",
        "Pen Tool",
        "Draw paths with corners and Bézier curves",
        "Tools",
        ["p"],
        ToolPen
    ),
    editor_command!(
        "tool.text",
        "Text Tool",
        "Create and edit text layers",
        "Tools",
        ["t"],
        ToolText
    ),
    editor_command!(
        "path.edit",
        "Edit Vector",
        "Edit anchors and Bézier handles",
        "Path",
        [],
        EnterVectorEdit
    ),
    editor_command!(
        "path.finish",
        "Finish Editing",
        "Commit the active Pen, Text, or vector-edit session",
        "Path",
        ["secondary-enter"],
        FinishEditing
    ),
    editor_command!(
        "path.join",
        "Join Paths",
        "Join two selected open endpoints",
        "Path",
        ["secondary-j"],
        JoinPaths
    ),
    editor_command!(
        "path.split",
        "Split Path",
        "Split the active path at the selected anchor",
        "Path",
        [],
        SplitPath
    ),
    editor_command!(
        "path.reverse",
        "Reverse Path",
        "Reverse the selected path contours",
        "Path",
        [],
        ReversePath
    ),
    editor_command!(
        "path.toggle_closed",
        "Open or Close Path",
        "Toggle the active contour closure",
        "Path",
        [],
        TogglePathClosed
    ),
    app_command!(
        "text.smaller",
        "Decrease Font Size",
        "Decrease the selected text size",
        "Text",
        [],
        TextSmaller
    ),
    app_command!(
        "text.larger",
        "Increase Font Size",
        "Increase the selected text size",
        "Text",
        [],
        TextLarger
    ),
    app_command!(
        "text.align_left",
        "Align Text Left",
        "Left-align the selected text",
        "Text",
        [],
        AlignTextLeft
    ),
    app_command!(
        "text.align_center",
        "Align Text Center",
        "Center the selected text",
        "Text",
        [],
        AlignTextCenter
    ),
    app_command!(
        "text.align_right",
        "Align Text Right",
        "Right-align the selected text",
        "Text",
        [],
        AlignTextRight
    ),
    app_command!(
        "frame.toggle_background",
        "Toggle Frame Background",
        "Toggle the selected frame background",
        "Frame",
        [],
        ToggleFrameBackground
    ),
];

/// Resolved keybindings after applying the optional per-user override file.
#[derive(Debug, Clone, Default)]
pub(crate) struct Keymap {
    overrides: BTreeMap<String, Vec<String>>,
}

impl Keymap {
    pub(crate) fn load() -> Self {
        let Some(path) = keymap_path() else {
            return Self::default();
        };
        match fs::read_to_string(&path) {
            Ok(json) => match Self::from_json(&json) {
                Ok((keymap, warnings)) => {
                    for warning in warnings {
                        log::warn!("{}: {warning}", path.display());
                    }
                    keymap
                }
                Err(error) => {
                    log::warn!("could not load {}: {error}", path.display());
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("could not read {}: {error}", path.display());
                Self::default()
            }
        }
    }

    fn from_json(json: &str) -> Result<(Self, Vec<String>), String> {
        let value: Value =
            serde_json::from_str(json).map_err(|error| format!("invalid JSON: {error}"))?;
        let object = value
            .as_object()
            .ok_or_else(|| "the keymap root must be an object".to_owned())?;
        if let Some(version_value) = object.get("version") {
            let version = version_value
                .as_u64()
                .ok_or_else(|| "`version` must be a positive integer".to_owned())?;
            if version != KEYMAP_VERSION {
                return Err(format!(
                    "unsupported keymap version {version}; expected {KEYMAP_VERSION}"
                ));
            }
        }
        let bindings = object
            .get("bindings")
            .and_then(Value::as_object)
            .ok_or_else(|| "`bindings` must be an object".to_owned())?;

        let mut overrides = BTreeMap::new();
        let mut warnings = Vec::new();
        for (id, value) in bindings {
            let Some(spec) = command(id) else {
                warnings.push(format!("unknown command `{id}` was ignored"));
                continue;
            };
            let Some(bindings) = parse_binding_value(id, value, &mut warnings) else {
                continue;
            };
            let explicitly_disabled = bindings.is_empty();
            let valid = bindings
                .into_iter()
                .filter(|binding| {
                    if validate_binding(binding).is_ok() {
                        true
                    } else {
                        warnings.push(format!(
                            "invalid binding `{binding}` for `{}` was ignored",
                            spec.id
                        ));
                        false
                    }
                })
                .collect::<Vec<_>>();
            if !explicitly_disabled && valid.is_empty() {
                warnings.push(format!(
                    "all configured bindings for `{}` were invalid; defaults remain active",
                    spec.id
                ));
                continue;
            }
            overrides.insert(id.clone(), valid);
        }

        Ok((Self { overrides }, warnings))
    }

    pub(crate) fn bindings<'a>(&'a self, spec: &'a CommandSpec) -> Vec<&'a str> {
        self.overrides
            .get(spec.id)
            .map(|bindings| bindings.iter().map(String::as_str).collect())
            .unwrap_or_else(|| spec.default_bindings.to_vec())
    }

    pub(crate) fn shortcut_label(&self, target: CommandTarget) -> Option<String> {
        let spec = command_for_target(target)?;
        self.bindings(spec)
            .first()
            .and_then(|binding| binding_label(binding))
    }

    pub(crate) fn ensure_user_file() -> Result<PathBuf, String> {
        let path = keymap_path().ok_or_else(|| "no user configuration directory".to_owned())?;
        if !path.exists() {
            let mut bindings = Map::new();
            for spec in COMMANDS {
                let value = match spec.default_bindings {
                    [] => Value::Array(Vec::new()),
                    [binding] => Value::String((*binding).to_owned()),
                    bindings => Value::Array(
                        bindings
                            .iter()
                            .map(|binding| Value::String((*binding).to_owned()))
                            .collect(),
                    ),
                };
                bindings.insert(spec.id.to_owned(), value);
            }
            let json = serde_json::to_vec_pretty(&json!({
                "_note": "Changes take effect after restarting Strek.",
                "version": KEYMAP_VERSION,
                "bindings": bindings,
            }))
            .map_err(|error| error.to_string())?;
            crate::document_io::write_atomic(&path, &json).map_err(|error| error.to_string())?;
        }
        Ok(path)
    }
}

pub(crate) fn command(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|command| command.id == id)
}

pub(crate) fn command_for_target(target: CommandTarget) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|command| command.target == target)
}

pub(crate) fn action_for(target: CommandTarget) -> Box<dyn Action> {
    match target {
        CommandTarget::Editor(action) => match action {
            EditorAction::SelectAll => Box::new(SelectAll),
            EditorAction::DeselectAll => Box::new(DeselectAll),
            EditorAction::InvertSelection => Box::new(InvertSelection),
            EditorAction::Undo => Box::new(Undo),
            EditorAction::Redo => Box::new(Redo),
            EditorAction::Delete => Box::new(Delete),
            EditorAction::Duplicate => Box::new(Duplicate),
            EditorAction::Group => Box::new(Group),
            EditorAction::Ungroup => Box::new(Ungroup),
            EditorAction::BringToFront => Box::new(BringToFront),
            EditorAction::SendToBack => Box::new(SendToBack),
            EditorAction::BringForward => Box::new(BringForward),
            EditorAction::SendBackward => Box::new(SendBackward),
            EditorAction::AlignLeft => Box::new(AlignObjectsLeft),
            EditorAction::AlignCenter => Box::new(AlignObjectsCenter),
            EditorAction::AlignRight => Box::new(AlignObjectsRight),
            EditorAction::AlignTop => Box::new(AlignObjectsTop),
            EditorAction::AlignMiddle => Box::new(AlignObjectsMiddle),
            EditorAction::AlignBottom => Box::new(AlignObjectsBottom),
            EditorAction::DistributeHorizontal => Box::new(DistributeObjectsHorizontal),
            EditorAction::DistributeVertical => Box::new(DistributeObjectsVertical),
            EditorAction::NudgeUp => Box::new(crate::NudgeUp),
            EditorAction::NudgeDown => Box::new(crate::NudgeDown),
            EditorAction::NudgeLeft => Box::new(crate::NudgeLeft),
            EditorAction::NudgeRight => Box::new(crate::NudgeRight),
            EditorAction::NudgeUpLarge => Box::new(crate::NudgeUpLarge),
            EditorAction::NudgeDownLarge => Box::new(crate::NudgeDownLarge),
            EditorAction::NudgeLeftLarge => Box::new(crate::NudgeLeftLarge),
            EditorAction::NudgeRightLarge => Box::new(crate::NudgeRightLarge),
            EditorAction::ZoomIn => Box::new(ZoomIn),
            EditorAction::ZoomOut => Box::new(ZoomOut),
            EditorAction::ZoomReset => Box::new(ZoomReset),
            EditorAction::ZoomResetAll => Box::new(ZoomResetAll),
            EditorAction::ZoomToFit => Box::new(ZoomToFit),
            EditorAction::ZoomToSelection => Box::new(ZoomToSelection),
            EditorAction::ToolSelect => Box::new(SelectTool),
            EditorAction::ToolFrame => Box::new(FrameTool),
            EditorAction::ToolRectangle => Box::new(RectangleTool),
            EditorAction::ToolEllipse => Box::new(EllipseTool),
            EditorAction::ToolLine => Box::new(LineTool),
            EditorAction::ToolPen => Box::new(PenTool),
            EditorAction::ToolText => Box::new(TextTool),
            EditorAction::EnterVectorEdit => Box::new(EditVector),
            EditorAction::FinishEditing => Box::new(FinishEditing),
            EditorAction::JoinPaths => Box::new(JoinPaths),
            EditorAction::SplitPath => Box::new(SplitPath),
            EditorAction::ReversePath => Box::new(ReversePath),
            EditorAction::TogglePathClosed => Box::new(TogglePathClosed),
        },
        CommandTarget::App(action) => match action {
            AppCommand::NewDocument => Box::new(NewDocument),
            AppCommand::OpenDocument => Box::new(OpenDocument),
            AppCommand::SaveDocument => Box::new(SaveDocument),
            AppCommand::SaveDocumentAs => Box::new(SaveDocumentAs),
            AppCommand::ExportSvg => Box::new(ExportSvg),
            AppCommand::ExportSvgOutlined => Box::new(ExportSvgOutlined),
            AppCommand::ExportPng => Box::new(ExportPng),
            AppCommand::ExportJpeg => Box::new(ExportJpeg),
            AppCommand::ExportWebP => Box::new(ExportWebP),
            AppCommand::OpenKeyboardShortcuts => Box::new(OpenKeyboardShortcuts),
            AppCommand::QuitApplication => Box::new(QuitApplication),
            AppCommand::Copy => Box::new(Copy),
            AppCommand::Cut => Box::new(Cut),
            AppCommand::Paste => Box::new(Paste),
            AppCommand::DeleteBackward => Box::new(Backspace),
            AppCommand::ToggleLayerPanel => Box::new(ToggleLayerPanel),
            AppCommand::ToggleDesignPanel => Box::new(ToggleDesignPanel),
            AppCommand::ShowCommandPalette => Box::new(ShowCommandPalette),
            AppCommand::TextSmaller => Box::new(TextSmaller),
            AppCommand::TextLarger => Box::new(TextLarger),
            AppCommand::AlignTextLeft => Box::new(AlignTextLeft),
            AppCommand::AlignTextCenter => Box::new(AlignTextCenter),
            AppCommand::AlignTextRight => Box::new(AlignTextRight),
            AppCommand::ToggleFrameBackground => Box::new(ToggleFrameBackground),
        },
    }
}

pub(crate) fn register_keybindings(cx: &mut App, keymap: &Keymap) {
    for spec in COMMANDS {
        for context in contexts_for(spec.target) {
            let predicate = KeyBindingContextPredicate::parse(context)
                .expect("static keybinding context should parse");
            for binding in keymap.bindings(spec) {
                match KeyBinding::load(
                    binding,
                    action_for(spec.target),
                    Some(Rc::new(predicate.clone())),
                    None,
                ) {
                    Ok(binding) => cx.bind_keys([binding]),
                    Err(error) => log::warn!(
                        "invalid keybinding `{binding}` for `{}` was ignored: {error}",
                        spec.id
                    ),
                }
            }
        }
    }
}

fn contexts_for(target: CommandTarget) -> &'static [&'static str] {
    match target {
        CommandTarget::App(
            AppCommand::NewDocument
            | AppCommand::OpenDocument
            | AppCommand::SaveDocument
            | AppCommand::SaveDocumentAs
            | AppCommand::QuitApplication
            | AppCommand::OpenKeyboardShortcuts
            | AppCommand::ShowCommandPalette,
        ) => &[EDITOR_CONTEXT, TEXT_CONTEXT, MENU_CONTEXT],
        CommandTarget::App(
            AppCommand::Copy | AppCommand::Cut | AppCommand::Paste | AppCommand::DeleteBackward,
        ) => &[EDITOR_CONTEXT, TEXT_CONTEXT],
        CommandTarget::Editor(
            EditorAction::Undo
            | EditorAction::Redo
            | EditorAction::SelectAll
            | EditorAction::Delete
            | EditorAction::FinishEditing,
        ) => &[EDITOR_CONTEXT, TEXT_CONTEXT],
        _ => &[EDITOR_CONTEXT],
    }
}

fn keymap_path() -> Option<PathBuf> {
    crate::document_io::app_config_directory().map(|directory| directory.join("keybindings.json"))
}

fn parse_binding_value(id: &str, value: &Value, warnings: &mut Vec<String>) -> Option<Vec<String>> {
    if value.is_null() {
        return Some(Vec::new());
    }
    if let Some(binding) = value.as_str() {
        return Some(vec![binding.to_owned()]);
    }
    let Some(values) = value.as_array() else {
        warnings.push(format!(
            "binding `{id}` must be a string, array of strings, or null"
        ));
        return None;
    };
    let mut bindings = Vec::with_capacity(values.len());
    for value in values {
        let Some(binding) = value.as_str() else {
            warnings.push(format!("non-string binding for `{id}` was ignored"));
            continue;
        };
        bindings.push(binding.to_owned());
    }
    Some(bindings)
}

fn validate_binding(binding: &str) -> Result<(), String> {
    if binding.trim().is_empty() {
        return Err("binding is empty".to_owned());
    }
    for keystroke in binding.split_whitespace() {
        Keystroke::parse(keystroke).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn binding_label(binding: &str) -> Option<String> {
    binding
        .split_whitespace()
        .map(|keystroke| Keystroke::parse(keystroke).ok().map(|key| key.to_string()))
        .collect::<Option<Vec<_>>>()
        .map(|keystrokes| keystrokes.join(" "))
}

pub(crate) fn file_url(path: &std::path::Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(path.len() + 7);
    if cfg!(target_os = "windows") && !path.starts_with('/') {
        encoded.push_str("file:///");
    } else {
        encoded.push_str("file://");
    }
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~' | b':') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_overrides_replace_defaults_and_null_disables_a_command() {
        let (keymap, warnings) = Keymap::from_json(
            r#"{
                "version": 1,
                "bindings": {
                    "tool.rectangle": "g",
                    "edit.redo": null
                }
            }"#,
        )
        .unwrap();

        assert!(warnings.is_empty());
        assert_eq!(
            keymap.bindings(command("tool.rectangle").unwrap()),
            vec!["g"]
        );
        assert!(keymap.bindings(command("edit.redo").unwrap()).is_empty());
        assert_eq!(keymap.bindings(command("tool.ellipse").unwrap()), vec!["o"]);
    }

    #[test]
    fn malformed_entries_do_not_discard_valid_overrides() {
        let (keymap, warnings) = Keymap::from_json(
            r#"{
                "bindings": {
                    "tool.line": "shift-l",
                    "tool.ellipse": 42,
                    "missing.command": "m"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            keymap.bindings(command("tool.line").unwrap()),
            vec!["shift-l"]
        );
        assert_eq!(keymap.bindings(command("tool.ellipse").unwrap()), vec!["o"]);
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn every_core_action_has_one_unique_command_entry() {
        let mut ids = std::collections::HashSet::new();
        let mut targets = std::collections::HashSet::new();
        for command in COMMANDS {
            assert!(
                ids.insert(command.id),
                "duplicate command id {}",
                command.id
            );
            assert!(
                targets.insert(command.target),
                "duplicate command target {}",
                command.id
            );
        }
        for action in EditorAction::all() {
            assert!(
                command_for_target(CommandTarget::Editor(*action)).is_some(),
                "missing command for {action:?}"
            );
        }
    }

    #[test]
    fn entirely_invalid_override_keeps_the_default_binding() {
        let (keymap, warnings) = Keymap::from_json(
            r#"{
                "version": 1,
                "bindings": {
                    "tool.rectangle": "secondary-not-a-key"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            keymap.bindings(command("tool.rectangle").unwrap()),
            vec!["r"]
        );
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn shortcut_labels_use_platform_symbols() {
        let keymap = Keymap::default();
        let label = keymap
            .shortcut_label(CommandTarget::Editor(EditorAction::ZoomToFit))
            .unwrap();

        #[cfg(target_os = "macos")]
        assert_eq!(label, "⌘⇧1");
        #[cfg(not(target_os = "macos"))]
        assert!(!label.is_empty());
    }

    #[test]
    fn file_urls_escape_spaces_and_non_ascii_bytes() {
        let url = file_url(std::path::Path::new("/tmp/My Keys/ø.json"));

        assert!(url.starts_with("file:///tmp/My%20Keys/"));
        assert!(url.ends_with("%C3%B8.json"));
    }
}
