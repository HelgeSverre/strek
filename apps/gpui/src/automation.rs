//! Local automation protocol shared by the CLI, AppleScript, and MCP server.

#[cfg(target_os = "macos")]
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, mpsc::SyncSender, Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
use core_foundation::base::{CFType, TCFType};
#[cfg(target_os = "macos")]
use core_foundation::dictionary::CFDictionary;
#[cfg(target_os = "macos")]
use core_foundation::number::CFNumber;
#[cfg(target_os = "macos")]
use core_foundation::string::{CFString, CFStringRef};
#[cfg(target_os = "macos")]
use core_graphics::geometry::CGRect;
#[cfg(target_os = "macos")]
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowBounds, kCGWindowLayer,
    kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowNumber,
    kCGWindowOwnerPID,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const SOCKET_ENV: &str = "STREK_AUTOMATION_SOCKET";
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(6);
const CONNECTION_WORKERS: usize = 4;
const MAX_QUEUED_CONNECTIONS: usize = 16;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const EXECUTION_HEARTBEAT: Duration = Duration::from_millis(250);
pub(crate) const AUTOMATION_PROTOCOL_VERSION: u16 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AutomationRequestMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_document_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_workspace_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationCall {
    #[serde(flatten)]
    pub metadata: AutomationRequestMetadata,
    #[serde(flatten)]
    pub request: AutomationRequest,
}

impl AutomationCall {
    pub(crate) fn legacy(request: AutomationRequest) -> Self {
        Self {
            metadata: AutomationRequestMetadata::default(),
            request,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AutomationRequest {
    State,
    Capabilities,
    Document,
    QueryDocument {
        #[serde(default)]
        ids: Vec<String>,
        #[serde(default)]
        include_descendants: bool,
        #[serde(default)]
        include_style: bool,
        #[serde(default)]
        include_geometry: bool,
    },
    NewDocument {
        #[serde(default)]
        discard_changes: bool,
    },
    OpenDocument {
        path: String,
        #[serde(default)]
        discard_changes: bool,
    },
    SaveDocument {
        path: String,
    },
    Export {
        format: ArtifactFormat,
        #[serde(default)]
        path: Option<String>,
    },
    Action {
        id: String,
    },
    Select {
        ids: Vec<String>,
        #[serde(default)]
        mode: SelectionMode,
    },
    SetColor {
        target: ColorTarget,
        #[serde(default)]
        color: Option<String>,
    },
    SetNumericProperty {
        target: NumericProperty,
        value: f32,
    },
    SetPrecision {
        #[serde(flatten)]
        settings: PrecisionSettingsPatch,
    },
    Guide {
        action: GuideAction,
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        axis: Option<GuideAxis>,
        #[serde(default)]
        position: Option<f32>,
    },
    ColorGroup {
        action: ColorGroupAction,
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        name: Option<String>,
    },
    SavedColor {
        action: SavedColorAction,
        #[serde(default)]
        id: Option<u64>,
        #[serde(default)]
        group_id: Option<u64>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        target: Option<ColorTarget>,
    },
    SetLayerProperties {
        ids: Vec<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        visible: Option<bool>,
        #[serde(default)]
        locked: Option<bool>,
    },
    Pointer {
        phase: PointerPhase,
        x: f32,
        y: f32,
        #[serde(default)]
        button: PointerButton,
        #[serde(default)]
        modifiers: AutomationModifiers,
    },
    Text {
        text: String,
    },
    SetUi {
        target: UiTarget,
        visible: bool,
    },
    Activate,
}

impl AutomationRequest {
    pub(crate) fn kind(&self) -> AutomationOperationKind {
        match self {
            Self::State => AutomationOperationKind::State,
            Self::Capabilities => AutomationOperationKind::Capabilities,
            Self::Document => AutomationOperationKind::Document,
            Self::QueryDocument { .. } => AutomationOperationKind::QueryDocument,
            Self::NewDocument { .. } => AutomationOperationKind::NewDocument,
            Self::OpenDocument { .. } => AutomationOperationKind::OpenDocument,
            Self::SaveDocument { .. } => AutomationOperationKind::SaveDocument,
            Self::Export { .. } => AutomationOperationKind::Export,
            Self::Action { .. } => AutomationOperationKind::Action,
            Self::Select { .. } => AutomationOperationKind::Select,
            Self::SetColor { .. } => AutomationOperationKind::SetColor,
            Self::SetNumericProperty { .. } => AutomationOperationKind::SetNumericProperty,
            Self::SetPrecision { .. } => AutomationOperationKind::SetPrecision,
            Self::Guide { .. } => AutomationOperationKind::Guide,
            Self::ColorGroup { .. } => AutomationOperationKind::ColorGroup,
            Self::SavedColor { .. } => AutomationOperationKind::SavedColor,
            Self::SetLayerProperties { .. } => AutomationOperationKind::SetLayerProperties,
            Self::Pointer { .. } => AutomationOperationKind::Pointer,
            Self::Text { .. } => AutomationOperationKind::Text,
            Self::SetUi { .. } => AutomationOperationKind::SetUi,
            Self::Activate => AutomationOperationKind::Activate,
        }
    }

    pub(crate) fn operation_name(&self) -> &'static str {
        self.kind().spec().id
    }

    pub(crate) fn is_observation(&self) -> bool {
        self.kind().spec().effect == AutomationEffectClass::Read
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum AutomationOperationKind {
    State,
    Capabilities,
    Document,
    QueryDocument,
    NewDocument,
    OpenDocument,
    SaveDocument,
    Export,
    Action,
    Select,
    SetColor,
    SetNumericProperty,
    SetPrecision,
    Guide,
    ColorGroup,
    SavedColor,
    SetLayerProperties,
    Pointer,
    Text,
    SetUi,
    Activate,
}

impl AutomationOperationKind {
    pub(crate) const ALL: [Self; 21] = [
        Self::State,
        Self::Capabilities,
        Self::Document,
        Self::QueryDocument,
        Self::NewDocument,
        Self::OpenDocument,
        Self::SaveDocument,
        Self::Export,
        Self::Action,
        Self::Select,
        Self::SetColor,
        Self::SetNumericProperty,
        Self::SetPrecision,
        Self::Guide,
        Self::ColorGroup,
        Self::SavedColor,
        Self::SetLayerProperties,
        Self::Pointer,
        Self::Text,
        Self::SetUi,
        Self::Activate,
    ];

    pub(crate) fn spec(self) -> &'static AutomationCapabilitySpec {
        &AUTOMATION_CAPABILITY_SPECS[self as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutomationEffectClass {
    Read,
    Document,
    Workspace,
    Filesystem,
    DocumentAndWorkspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutomationCostClass {
    Metadata,
    DocumentTraversal,
    Layout,
    Render,
    Filesystem,
    Platform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutomationAuthorityClass {
    Inspect,
    Edit,
    FilesystemRead,
    FilesystemWrite,
    Platform,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AutomationParameterSpec {
    pub name: &'static str,
    pub required: bool,
    pub value_type: AutomationParameterType,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutomationParameterType {
    String,
    Boolean,
    UnsignedInteger,
    FiniteNumber,
    StringList,
    Object,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AutomationCapabilitySpec {
    pub id: &'static str,
    pub summary: &'static str,
    pub effect: AutomationEffectClass,
    pub cost: AutomationCostClass,
    pub authority: AutomationAuthorityClass,
    pub parameters: &'static [AutomationParameterSpec],
    pub limits: &'static [&'static str],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationCapability {
    pub id: String,
    pub summary: String,
    pub effect: AutomationEffectClass,
    pub cost: AutomationCostClass,
    pub authority: AutomationAuthorityClass,
    pub guarded: bool,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    pub parameters: Vec<AutomationParameter>,
    pub limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationParameter {
    pub name: String,
    pub required: bool,
    pub value_type: AutomationParameterType,
    pub description: String,
}

const NO_PARAMETERS: &[AutomationParameterSpec] = &[];
const NO_LIMITS: &[&str] = &[];
pub(crate) const AUTOMATION_CAPABILITY_SPECS: &[AutomationCapabilitySpec] = &[
    capability(
        "state",
        "Inspect compact editor and workspace state",
        AutomationEffectClass::Read,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Inspect,
        NO_PARAMETERS,
        NO_LIMITS,
    ),
    capability(
        "capabilities",
        "Discover automation operations and current availability",
        AutomationEffectClass::Read,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Inspect,
        NO_PARAMETERS,
        NO_LIMITS,
    ),
    capability(
        "document",
        "Inspect the authored document tree and computed geometry",
        AutomationEffectClass::Read,
        AutomationCostClass::Layout,
        AutomationAuthorityClass::Inspect,
        NO_PARAMETERS,
        NO_LIMITS,
    ),
    capability(
        "query_document",
        "Inspect selected document entities with opt-in descendants, style, and geometry",
        AutomationEffectClass::Read,
        AutomationCostClass::DocumentTraversal,
        AutomationAuthorityClass::Inspect,
        &[
            typed_parameter(
                "ids",
                false,
                AutomationParameterType::StringList,
                "Layer IDs; empty selects the document root",
            ),
            typed_parameter(
                "include_descendants",
                false,
                AutomationParameterType::Boolean,
                "Include descendants of each requested layer",
            ),
            typed_parameter(
                "include_style",
                false,
                AutomationParameterType::Boolean,
                "Include opacity, fill, and stroke",
            ),
            typed_parameter(
                "include_geometry",
                false,
                AutomationParameterType::Boolean,
                "Include computed world bounds and transforms",
            ),
        ],
        NO_LIMITS,
    ),
    capability(
        "new_document",
        "Replace the current document with an empty document",
        AutomationEffectClass::DocumentAndWorkspace,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Edit,
        &[typed_parameter(
            "discard_changes",
            false,
            AutomationParameterType::Boolean,
            "Allow replacement of a dirty document",
        )],
        NO_LIMITS,
    ),
    capability(
        "open_document",
        "Open native JSON or import a supported SVG",
        AutomationEffectClass::DocumentAndWorkspace,
        AutomationCostClass::Filesystem,
        AutomationAuthorityClass::FilesystemRead,
        &[
            parameter("path", true, "Absolute source path"),
            typed_parameter(
                "discard_changes",
                false,
                AutomationParameterType::Boolean,
                "Allow replacement of a dirty document",
            ),
        ],
        NO_LIMITS,
    ),
    capability(
        "save_document",
        "Validate and atomically save native JSON",
        AutomationEffectClass::Filesystem,
        AutomationCostClass::Filesystem,
        AutomationAuthorityClass::FilesystemWrite,
        &[parameter("path", true, "Absolute destination path")],
        NO_LIMITS,
    ),
    capability(
        "export",
        "Export settled visible artwork",
        AutomationEffectClass::Filesystem,
        AutomationCostClass::Render,
        AutomationAuthorityClass::FilesystemWrite,
        &[
            parameter("format", true, "svg, svg_outlined, png, jpeg, or webp"),
            parameter(
                "path",
                false,
                "Absolute destination; omit for inline artifact",
            ),
        ],
        NO_LIMITS,
    ),
    capability(
        "action",
        "Run an enabled semantic editor command",
        AutomationEffectClass::DocumentAndWorkspace,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Edit,
        &[parameter(
            "id",
            true,
            "Enabled command ID returned by state",
        )],
        NO_LIMITS,
    ),
    capability(
        "select",
        "Change selection using session-stable layer IDs",
        AutomationEffectClass::Workspace,
        AutomationCostClass::DocumentTraversal,
        AutomationAuthorityClass::Edit,
        &[
            typed_parameter(
                "ids",
                true,
                AutomationParameterType::StringList,
                "Layer IDs returned by document",
            ),
            parameter("mode", false, "replace, add, remove, or toggle"),
        ],
        NO_LIMITS,
    ),
    capability(
        "set_color",
        "Set or remove selection paint",
        AutomationEffectClass::Document,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Edit,
        &[
            parameter("target", true, "fill, stroke, or frame_background"),
            parameter("color", false, "HEX RGBA; omit to remove paint"),
        ],
        NO_LIMITS,
    ),
    capability(
        "set_numeric_property",
        "Set an absolute semantic numeric property",
        AutomationEffectClass::Document,
        AutomationCostClass::Layout,
        AutomationAuthorityClass::Edit,
        &[
            parameter("target", true, "Numeric property target"),
            typed_parameter(
                "value",
                true,
                AutomationParameterType::FiniteNumber,
                "Finite value in documented model units",
            ),
        ],
        NO_LIMITS,
    ),
    capability(
        "set_precision",
        "Configure precision workspace state and document grid",
        AutomationEffectClass::DocumentAndWorkspace,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Edit,
        &[typed_parameter(
            "settings",
            false,
            AutomationParameterType::Object,
            "Precision settings patch",
        )],
        NO_LIMITS,
    ),
    capability(
        "guide",
        "Add, move, remove, or clear document guides",
        AutomationEffectClass::Document,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Edit,
        &[
            parameter("action", true, "add, move, remove, or clear"),
            typed_parameter(
                "id",
                false,
                AutomationParameterType::UnsignedInteger,
                "Guide ID",
            ),
            parameter("axis", false, "horizontal or vertical"),
            typed_parameter(
                "position",
                false,
                AutomationParameterType::FiniteNumber,
                "Finite document-space coordinate",
            ),
        ],
        NO_LIMITS,
    ),
    capability(
        "color_group",
        "Manage document Color Library groups",
        AutomationEffectClass::Document,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Edit,
        &[
            parameter("action", true, "add, rename, or remove"),
            typed_parameter(
                "id",
                false,
                AutomationParameterType::UnsignedInteger,
                "Color group ID",
            ),
            parameter("name", false, "UTF-8 group name"),
        ],
        NO_LIMITS,
    ),
    capability(
        "saved_color",
        "Manage or apply document Saved Colors",
        AutomationEffectClass::Document,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Edit,
        &[
            parameter("action", true, "add, update, remove, or apply"),
            typed_parameter(
                "id",
                false,
                AutomationParameterType::UnsignedInteger,
                "Saved Color ID",
            ),
            typed_parameter(
                "group_id",
                false,
                AutomationParameterType::UnsignedInteger,
                "Color group ID",
            ),
            parameter("name", false, "UTF-8 color name"),
            parameter("color", false, "HEX RGBA"),
            parameter("target", false, "Paint target when applying"),
        ],
        NO_LIMITS,
    ),
    capability(
        "set_layer_properties",
        "Set layer name, visibility, or locking",
        AutomationEffectClass::Document,
        AutomationCostClass::DocumentTraversal,
        AutomationAuthorityClass::Edit,
        &[
            typed_parameter(
                "ids",
                true,
                AutomationParameterType::StringList,
                "Layer IDs returned by document",
            ),
            parameter("name", false, "Name for exactly one layer"),
            typed_parameter(
                "visible",
                false,
                AutomationParameterType::Boolean,
                "Layer visibility",
            ),
            typed_parameter(
                "locked",
                false,
                AutomationParameterType::Boolean,
                "Layer lock state",
            ),
        ],
        NO_LIMITS,
    ),
    capability(
        "pointer",
        "Send a canvas-local spatial pointer event",
        AutomationEffectClass::DocumentAndWorkspace,
        AutomationCostClass::Layout,
        AutomationAuthorityClass::Edit,
        &[
            parameter("phase", true, "down, move, or up"),
            typed_parameter(
                "x",
                true,
                AutomationParameterType::FiniteNumber,
                "Finite canvas-local pixel coordinate",
            ),
            typed_parameter(
                "y",
                true,
                AutomationParameterType::FiniteNumber,
                "Finite canvas-local pixel coordinate",
            ),
            parameter("button", false, "left, middle, or right"),
            typed_parameter(
                "modifiers",
                false,
                AutomationParameterType::Object,
                "Keyboard modifier state",
            ),
        ],
        NO_LIMITS,
    ),
    capability(
        "text",
        "Insert text into the active semantic text edit",
        AutomationEffectClass::Document,
        AutomationCostClass::Layout,
        AutomationAuthorityClass::Edit,
        &[parameter("text", true, "UTF-8 text")],
        NO_LIMITS,
    ),
    capability(
        "set_ui",
        "Show or hide a supported panel or overlay",
        AutomationEffectClass::Workspace,
        AutomationCostClass::Metadata,
        AutomationAuthorityClass::Edit,
        &[
            parameter("target", true, "Supported UI target"),
            typed_parameter(
                "visible",
                true,
                AutomationParameterType::Boolean,
                "Requested visibility",
            ),
        ],
        NO_LIMITS,
    ),
    capability(
        "activate",
        "Activate and focus the Strek window",
        AutomationEffectClass::Workspace,
        AutomationCostClass::Platform,
        AutomationAuthorityClass::Platform,
        NO_PARAMETERS,
        NO_LIMITS,
    ),
];

const fn capability(
    id: &'static str,
    summary: &'static str,
    effect: AutomationEffectClass,
    cost: AutomationCostClass,
    authority: AutomationAuthorityClass,
    parameters: &'static [AutomationParameterSpec],
    limits: &'static [&'static str],
) -> AutomationCapabilitySpec {
    AutomationCapabilitySpec {
        id,
        summary,
        effect,
        cost,
        authority,
        parameters,
        limits,
    }
}

const fn parameter(
    name: &'static str,
    required: bool,
    description: &'static str,
) -> AutomationParameterSpec {
    AutomationParameterSpec {
        name,
        required,
        value_type: AutomationParameterType::String,
        description,
    }
}

const fn typed_parameter(
    name: &'static str,
    required: bool,
    value_type: AutomationParameterType,
    description: &'static str,
) -> AutomationParameterSpec {
    AutomationParameterSpec {
        name,
        required,
        value_type,
        description,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactFormat {
    Svg,
    SvgOutlined,
    Png,
    Jpeg,
    WebP,
}

impl FromStr for ArtifactFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "svg" => Ok(Self::Svg),
            "svg-outlined" | "svg_outlined" => Ok(Self::SvgOutlined),
            "png" => Ok(Self::Png),
            "jpeg" | "jpg" => Ok(Self::Jpeg),
            "webp" => Ok(Self::WebP),
            _ => Err(format!(
                "unknown export format `{value}`; use svg, svg-outlined, png, jpeg, or webp"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SelectionMode {
    #[default]
    Replace,
    Add,
    Remove,
    Toggle,
}

impl FromStr for SelectionMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "replace" => Ok(Self::Replace),
            "add" => Ok(Self::Add),
            "remove" => Ok(Self::Remove),
            "toggle" => Ok(Self::Toggle),
            _ => Err(format!(
                "unknown selection mode `{value}`; use replace, add, remove, or toggle"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ColorTarget {
    Fill,
    Stroke,
    FrameBackground,
}

impl FromStr for ColorTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "fill" => Ok(Self::Fill),
            "stroke" => Ok(Self::Stroke),
            "frame_background" | "frame-background" => Ok(Self::FrameBackground),
            _ => Err(format!(
                "unknown color target `{value}`; use fill, stroke, or frame-background"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NumericProperty {
    WorldX,
    WorldY,
    Opacity,
    StrokeWidth,
    TextSize,
    LayoutSpacing,
    LayoutPadding,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
pub(crate) struct PrecisionSettingsPatch {
    pub rulers: Option<bool>,
    pub grid_visible: Option<bool>,
    pub guides_visible: Option<bool>,
    pub guides_locked: Option<bool>,
    pub snapping: Option<bool>,
    pub snap_objects: Option<bool>,
    pub snap_guides: Option<bool>,
    pub snap_grid: Option<bool>,
    pub tolerance: Option<f32>,
    pub grid_spacing: Option<f32>,
    pub grid_major_every: Option<u8>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GuideAction {
    Add,
    Move,
    Remove,
    Clear,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GuideAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ColorGroupAction {
    Add,
    Rename,
    Remove,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SavedColorAction {
    Add,
    Update,
    Remove,
    Apply,
}

impl FromStr for NumericProperty {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "x" | "world-x" | "world_x" => Ok(Self::WorldX),
            "y" | "world-y" | "world_y" => Ok(Self::WorldY),
            "opacity" => Ok(Self::Opacity),
            "stroke-width" | "stroke_width" => Ok(Self::StrokeWidth),
            "text-size" | "text_size" => Ok(Self::TextSize),
            "layout-spacing" | "layout_spacing" => Ok(Self::LayoutSpacing),
            "layout-padding" | "layout_padding" => Ok(Self::LayoutPadding),
            _ => Err(format!(
                "unknown numeric property `{value}`; use x, y, opacity, stroke-width, text-size, layout-spacing, or layout-padding"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PointerPhase {
    Down,
    Move,
    Up,
}

impl FromStr for PointerPhase {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "down" => Ok(Self::Down),
            "move" => Ok(Self::Move),
            "up" => Ok(Self::Up),
            _ => Err(format!(
                "unknown pointer phase `{value}`; use down, move, or up"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PointerButton {
    #[default]
    Left,
    Middle,
    Right,
}

impl FromStr for PointerButton {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "left" => Ok(Self::Left),
            "middle" => Ok(Self::Middle),
            "right" => Ok(Self::Right),
            _ => Err(format!(
                "unknown pointer button `{value}`; use left, middle, or right"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct AutomationModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub command: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UiTarget {
    MainMenu,
    CommandPalette,
    LayersPanel,
    DesignPanel,
    FillColorPicker,
    StrokeColorPicker,
    FrameBackgroundColorPicker,
    ColorLibrary,
    PrecisionControls,
}

impl FromStr for UiTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "main_menu" | "main-menu" => Ok(Self::MainMenu),
            "command_palette" | "command-palette" => Ok(Self::CommandPalette),
            "layers_panel" | "layers-panel" => Ok(Self::LayersPanel),
            "design_panel" | "design-panel" => Ok(Self::DesignPanel),
            "fill_color_picker" | "fill-color-picker" => Ok(Self::FillColorPicker),
            "stroke_color_picker" | "stroke-color-picker" => Ok(Self::StrokeColorPicker),
            "frame_background_color_picker" | "frame-background-color-picker" => {
                Ok(Self::FrameBackgroundColorPicker)
            }
            "color_library" | "color-library" => Ok(Self::ColorLibrary),
            "precision_controls" | "precision-controls" => Ok(Self::PrecisionControls),
            _ => Err(format!(
                "unknown UI target `{value}`; use main-menu, command-palette, layers-panel, design-panel, fill-color-picker, stroke-color-picker, frame-background-color-picker, color-library, or precision-controls"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationResponse {
    pub ok: bool,
    pub protocol_version: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AutomationError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<AutomationReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<AutomationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<AutomationDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_query: Option<AutomationDocumentQuery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<AutomationCapability>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<AutomationArtifact>,
}

impl AutomationResponse {
    pub(crate) fn success(state: AutomationState, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            protocol_version: AUTOMATION_PROTOCOL_VERSION,
            message: Some(message.into()),
            error: None,
            receipt: None,
            state: Some(state),
            document: None,
            document_query: None,
            capabilities: None,
            artifact: None,
        }
    }

    pub(crate) fn error(state: AutomationState, message: impl Into<String>) -> Self {
        Self::error_with_code(state, AutomationErrorCode::Rejected, message)
    }

    pub(crate) fn error_with_code(
        state: AutomationState,
        code: AutomationErrorCode,
        message: impl Into<String>,
    ) -> Self {
        let message = message.into();
        Self {
            ok: false,
            protocol_version: AUTOMATION_PROTOCOL_VERSION,
            message: Some(message.clone()),
            error: Some(AutomationError { code, message }),
            receipt: None,
            state: Some(state),
            document: None,
            document_query: None,
            capabilities: None,
            artifact: None,
        }
    }

    pub(crate) fn into_result(self) -> Result<Self, String> {
        if self.ok {
            Ok(self)
        } else {
            Err(self
                .message
                .unwrap_or_else(|| "Strek rejected the automation request".to_owned()))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutomationErrorCode {
    Rejected,
    ProtocolVersionMismatch,
    InvalidRequestId,
    RequestExpired,
    RequestSequenceMismatch,
    SessionMismatch,
    DocumentRevisionMismatch,
    WorkspaceRevisionMismatch,
}

impl AutomationErrorCode {
    pub(crate) fn cacheable(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::DocumentRevisionMismatch | Self::WorkspaceRevisionMismatch
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationError {
    pub code: AutomationErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AutomationEffect {
    DocumentChanged,
    WorkspaceOnly,
    NoOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationReceipt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_sequence: Option<u64>,
    pub operation: String,
    pub before: AutomationRevisions,
    pub after: AutomationRevisions,
    pub effect: AutomationEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AutomationRevisions {
    pub session_id: String,
    pub document: u64,
    pub workspace: u64,
    pub artwork: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationDocument {
    pub root_id: String,
    pub layers: Vec<AutomationLayer>,
    pub grid: AutomationGrid,
    pub guides: Vec<AutomationGuide>,
    pub color_groups: Vec<AutomationColorGroup>,
    pub saved_colors: Vec<AutomationSavedColor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationDocumentQuery {
    pub root_id: String,
    pub layers: Vec<AutomationProjectedLayer>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationProjectedLayer {
    pub id: String,
    pub parent_id: Option<String>,
    pub child_ids: Vec<String>,
    pub name: String,
    pub kind: String,
    pub visible: bool,
    pub locked: bool,
    pub selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<AutomationProjectedStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<AutomationProjectedGeometry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationProjectedStyle {
    pub opacity: f32,
    pub fill: Option<String>,
    pub stroke: Option<AutomationStroke>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationProjectedGeometry {
    pub world_bounds: Option<AutomationBounds>,
    pub world_transform: [f32; 6],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct AutomationGrid {
    pub spacing: f32,
    pub major_every: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct AutomationGuide {
    pub id: u64,
    pub axis: GuideAxis,
    pub position: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationColorGroup {
    pub id: u64,
    pub name: String,
    pub manual_order: u32,
    pub sort: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationSavedColor {
    pub id: u64,
    pub group_id: Option<u64>,
    pub name: Option<String>,
    pub color: String,
    pub manual_order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationLayer {
    pub id: String,
    pub parent_id: Option<String>,
    pub child_ids: Vec<String>,
    pub name: String,
    pub kind: String,
    pub visible: bool,
    pub locked: bool,
    pub selected: bool,
    pub opacity: f32,
    pub fill: Option<String>,
    pub stroke: Option<AutomationStroke>,
    pub world_bounds: Option<AutomationBounds>,
    pub world_transform: [f32; 6],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationStroke {
    pub color: String,
    pub width: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationArtifact {
    pub format: String,
    pub media_type: String,
    pub base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationState {
    #[serde(default)]
    pub process_id: u32,
    pub protocol_version: u16,
    pub revisions: AutomationRevisions,
    pub retry_window: AutomationRetryWindow,
    pub document: String,
    pub dirty: bool,
    pub tool: String,
    pub interaction: String,
    pub selection_count: usize,
    pub selected_layers: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub selected_layers_truncated: bool,
    pub zoom: f32,
    pub pan: AutomationPoint,
    pub window: AutomationBounds,
    pub canvas: Option<AutomationBounds>,
    pub layers_panel_visible: bool,
    pub design_panel_visible: bool,
    pub main_menu_open: bool,
    pub command_palette_open: bool,
    #[serde(default)]
    pub color_picker_open: bool,
    #[serde(default)]
    pub color_library_open: bool,
    #[serde(default)]
    pub precision_controls_open: bool,
    #[serde(default)]
    pub rulers_visible: bool,
    #[serde(default)]
    pub grid_visible: bool,
    #[serde(default)]
    pub guides_visible: bool,
    #[serde(default)]
    pub numeric_property_scrub_active: bool,
    pub actions: Vec<AutomationAction>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AutomationRetryWindow {
    pub next_sequence: u64,
    pub retained_from: u64,
    pub capacity: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct AutomationPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct AutomationBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationAction {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

pub(crate) struct PendingRequest {
    call: AutomationCall,
    responder: SyncSender<AutomationResponse>,
    deadline: Instant,
    lifecycle: Arc<RequestLifecycle>,
}

const REQUEST_PENDING: u8 = 0;
const REQUEST_EXECUTING: u8 = 1;
const REQUEST_CANCELLED: u8 = 2;
const REQUEST_COMPLETED: u8 = 3;

struct RequestLifecycle(AtomicU8);

impl RequestLifecycle {
    fn pending() -> Self {
        Self(AtomicU8::new(REQUEST_PENDING))
    }

    fn begin_execution(&self) -> bool {
        self.0
            .compare_exchange(
                REQUEST_PENDING,
                REQUEST_EXECUTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn cancel_pending(&self) -> bool {
        self.0
            .compare_exchange(
                REQUEST_PENDING,
                REQUEST_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn state(&self) -> u8 {
        self.0.load(Ordering::Acquire)
    }

    fn complete(&self) {
        self.0.store(REQUEST_COMPLETED, Ordering::Release);
    }
}

// Platform-specific IPC adapters. The automation protocol below uses only the
// shared stream/listener surface exposed by these modules.
#[cfg(unix)]
mod transport {
    use std::env;
    use std::fs;
    use std::io;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    use super::SOCKET_ENV;

    pub(super) type Stream = UnixStream;

    pub(super) struct Listener(UnixListener);

    pub(super) fn listen() -> io::Result<Listener> {
        let path = socket_path();
        if env::var_os(SOCKET_ENV).is_none() {
            prepare_default_socket_directory(&path)?;
        }
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ));
            }
            if UnixStream::connect(&path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "another Strek automation server is running",
                ));
            }
            fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(Listener(listener))
    }

    pub(super) fn connect() -> io::Result<Stream> {
        UnixStream::connect(socket_path())
    }

    pub(super) fn endpoint_display() -> String {
        socket_path().display().to_string()
    }

    impl Listener {
        pub(super) fn accept(&self) -> io::Result<Stream> {
            self.0.accept().map(|(stream, _)| stream)
        }
    }

    #[cfg(test)]
    pub(super) fn pair_for_tests() -> io::Result<(Stream, Stream)> {
        UnixStream::pair()
    }

    fn socket_path() -> PathBuf {
        env::var_os(SOCKET_ENV)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(default_socket_path)
    }

    fn default_socket_path() -> PathBuf {
        let user_id = unsafe { libc::geteuid() };
        let base = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(env::temp_dir);
        base.join(format!("strek-{user_id}"))
            .join("automation.sock")
    }

    fn prepare_default_socket_directory(path: &Path) -> io::Result<()> {
        let directory = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "automation socket has no parent directory",
            )
        })?;
        match fs::create_dir(directory) {
            Ok(()) => fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }

        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} is not a directory", directory.display()),
            ));
        }
        let user_id = unsafe { libc::geteuid() };
        if metadata.uid() != user_id {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} is owned by another user", directory.display()),
            ));
        }
        if metadata.mode() & 0o077 != 0 {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn default_socket_path_is_scoped_to_the_effective_user() {
            let path = default_socket_path();
            let expected_directory = format!("strek-{}", unsafe { libc::geteuid() });
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some("automation.sock")
            );
            assert_eq!(
                path.parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str()),
                Some(expected_directory.as_str())
            );
        }
    }
}

#[cfg(windows)]
mod transport {
    use std::env;
    use std::io;
    use std::net::{SocketAddr, TcpListener, TcpStream};

    use super::SOCKET_ENV;

    pub(super) type Stream = TcpStream;

    pub(super) struct Listener(TcpListener);

    pub(super) fn listen() -> io::Result<Listener> {
        Ok(Listener(TcpListener::bind(tcp_address()?)?))
    }

    pub(super) fn connect() -> io::Result<Stream> {
        TcpStream::connect(tcp_address()?)
    }

    pub(super) fn endpoint_display() -> String {
        tcp_address()
            .map(|address| address.to_string())
            .unwrap_or_else(|error| format!("invalid endpoint: {error}"))
    }

    impl Listener {
        pub(super) fn accept(&self) -> io::Result<Stream> {
            self.0.accept().map(|(stream, _)| stream)
        }
    }

    #[cfg(test)]
    pub(super) fn pair_for_tests() -> io::Result<(Stream, Stream)> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let client = TcpStream::connect(address)?;
        let (server, _) = listener.accept()?;
        Ok((client, server))
    }

    fn tcp_address() -> io::Result<SocketAddr> {
        let endpoint = env::var(SOCKET_ENV)
            .ok()
            .filter(|endpoint| !endpoint.is_empty())
            .unwrap_or_else(|| default_tcp_address().to_string());
        parse_tcp_address(&endpoint)
    }

    fn parse_tcp_address(endpoint: &str) -> io::Result<SocketAddr> {
        endpoint
            .parse::<SocketAddr>()
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid {SOCKET_ENV} endpoint `{endpoint}`: {error}"),
                )
            })
            .and_then(|address| {
                if address.ip().is_loopback() {
                    Ok(address)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("{SOCKET_ENV} must use a loopback address, got `{address}`"),
                    ))
                }
            })
    }

    fn default_tcp_address() -> SocketAddr {
        let identity = env::var_os("USERNAME")
            .or_else(|| env::var_os("USERPROFILE"))
            .unwrap_or_else(|| "strek".into());
        let hash = identity
            .to_string_lossy()
            .bytes()
            .fold(2_166_136_261_u32, |hash, byte| {
                (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
            });
        let port = 49_152 + (hash % 16_384) as u16;
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn default_tcp_endpoint_is_loopback_and_uses_a_user_scoped_port() {
            let address = default_tcp_address();

            assert!(address.ip().is_loopback());
            assert!((49_152..65_536).contains(&u32::from(address.port())));
        }

        #[test]
        fn tcp_endpoint_rejects_non_loopback_addresses() {
            assert!(parse_tcp_address("192.0.2.1:49153").is_err());
            assert_eq!(parse_tcp_address("127.0.0.1:49153").unwrap().port(), 49_153);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod transport {
    use std::io::{self, Read, Write};
    use std::time::Duration;

    pub(super) struct Stream;
    pub(super) struct Listener;

    pub(super) fn listen() -> io::Result<Listener> {
        Err(unsupported())
    }

    pub(super) fn connect() -> io::Result<Stream> {
        Err(unsupported())
    }

    pub(super) fn endpoint_display() -> String {
        "unsupported".to_owned()
    }

    #[cfg(test)]
    pub(super) fn pair_for_tests() -> io::Result<(Stream, Stream)> {
        Err(unsupported())
    }

    impl Listener {
        pub(super) fn accept(&self) -> io::Result<Stream> {
            Err(unsupported())
        }
    }

    impl Read for Stream {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(unsupported())
        }
    }

    impl Write for Stream {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(unsupported())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(unsupported())
        }
    }

    impl Stream {
        pub(super) fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Err(unsupported())
        }

        pub(super) fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Err(unsupported())
        }
    }

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Strek automation is unavailable on this platform",
        )
    }
}

struct QueuedConnection {
    stream: transport::Stream,
    deadline: Instant,
}

impl PendingRequest {
    pub(crate) fn begin(self) -> Option<(AutomationCall, AutomationResponder)> {
        if Instant::now() >= self.deadline {
            self.lifecycle.cancel_pending();
            return None;
        }
        if !self.lifecycle.begin_execution() {
            return None;
        }

        let responder = AutomationResponder {
            sender: self.responder,
            lifecycle: self.lifecycle,
        };
        Some((self.call, responder))
    }

    #[cfg(test)]
    fn respond_with(
        self,
        make_response: impl FnOnce(AutomationRequest) -> AutomationResponse,
    ) -> bool {
        let Some((call, responder)) = self.begin() else {
            return false;
        };
        responder.respond(make_response(call.request))
    }
}

pub(crate) struct AutomationResponder {
    sender: SyncSender<AutomationResponse>,
    lifecycle: Arc<RequestLifecycle>,
}

impl AutomationResponder {
    pub(crate) fn respond(self, response: AutomationResponse) -> bool {
        let sent = self.sender.send(response).is_ok();
        self.lifecycle.complete();
        sent
    }
}

#[cfg(test)]
pub(crate) fn pending_request_for_test(
    request: AutomationRequest,
) -> (PendingRequest, mpsc::Receiver<AutomationResponse>) {
    pending_call_for_test(AutomationCall::legacy(request))
}

#[cfg(test)]
pub(crate) fn pending_call_for_test(
    call: AutomationCall,
) -> (PendingRequest, mpsc::Receiver<AutomationResponse>) {
    let (responder, response) = mpsc::sync_channel(1);
    let lifecycle = Arc::new(RequestLifecycle::pending());
    (
        PendingRequest {
            call,
            responder,
            deadline: Instant::now() + RESPONSE_TIMEOUT,
            lifecycle,
        },
        response,
    )
}

pub(crate) fn start_server() -> io::Result<tokio::sync::mpsc::UnboundedReceiver<PendingRequest>> {
    let listener = transport::listen()?;
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let (connection_sender, connection_receiver) =
        mpsc::sync_channel::<QueuedConnection>(MAX_QUEUED_CONNECTIONS);
    let connection_receiver = Arc::new(Mutex::new(connection_receiver));

    for index in 0..CONNECTION_WORKERS {
        let connection_receiver = Arc::clone(&connection_receiver);
        let sender = sender.clone();
        std::thread::Builder::new()
            .name(format!("strek-automation-{index}"))
            .spawn(move || loop {
                let stream = {
                    let Ok(receiver) = connection_receiver.lock() else {
                        return;
                    };
                    receiver.recv()
                };
                let Ok(connection) = stream else {
                    return;
                };
                if let Err(error) =
                    serve_connection(connection.stream, &sender, connection.deadline)
                {
                    log::warn!("automation request failed: {error}");
                }
            })?;
    }

    std::thread::Builder::new()
        .name("strek-automation-accept".to_owned())
        .spawn(move || loop {
            match listener.accept() {
                Ok(stream) => match connection_sender.try_send(QueuedConnection {
                    stream,
                    deadline: Instant::now() + RESPONSE_TIMEOUT,
                }) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(QueuedConnection { mut stream, .. })) => {
                        let _ = stream.set_write_timeout(Some(RESPONSE_TIMEOUT));
                        let _ = write_protocol_error(
                            &mut stream,
                            "Strek automation is busy; retry shortly",
                        );
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => return,
                },
                Err(error) => log::warn!("automation endpoint failed: {error}"),
            }
        })?;
    Ok(receiver)
}

fn serve_connection(
    mut stream: transport::Stream,
    sender: &tokio::sync::mpsc::UnboundedSender<PendingRequest>,
    deadline: Instant,
) -> io::Result<()> {
    stream.set_write_timeout(Some(RESPONSE_TIMEOUT))?;
    if Instant::now() >= deadline {
        write_protocol_error(
            &mut stream,
            "Strek did not process the automation request in time",
        )?;
        return Ok(());
    }
    let request_json = match read_request_line(&mut stream, deadline) {
        Ok(request_json) => request_json,
        Err(error) => {
            write_protocol_error(&mut stream, format!("invalid automation request: {error}"))?;
            return Ok(());
        }
    };
    let call = match serde_json::from_str(&request_json) {
        Ok(call) => call,
        Err(error) => {
            write_protocol_error(&mut stream, format!("invalid automation request: {error}"))?;
            return Ok(());
        }
    };
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    let lifecycle = Arc::new(RequestLifecycle::pending());
    sender
        .send(PendingRequest {
            call,
            responder: response_sender,
            deadline,
            lifecycle: Arc::clone(&lifecycle),
        })
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Strek window closed"))?;
    let response = match wait_for_response(&mut stream, response_receiver, &lifecycle, deadline)? {
        Some(response) => response,
        None => {
            write_protocol_error(
                &mut stream,
                "Strek did not process the automation request in time",
            )?;
            return Ok(());
        }
    };
    write_json_line(&mut stream, &response)
}

fn wait_for_response(
    stream: &mut transport::Stream,
    response_receiver: mpsc::Receiver<AutomationResponse>,
    lifecycle: &RequestLifecycle,
    deadline: Instant,
) -> io::Result<Option<AutomationResponse>> {
    loop {
        match lifecycle.state() {
            REQUEST_PENDING => {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    if lifecycle.cancel_pending() {
                        return Ok(None);
                    }
                    continue;
                };
                match response_receiver.recv_timeout(remaining.min(EXECUTION_HEARTBEAT)) {
                    Ok(response) => return Ok(Some(response)),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(closed_before_response());
                    }
                }
            }
            REQUEST_EXECUTING | REQUEST_COMPLETED => {
                // A space is valid leading JSON whitespace and also keeps the
                // client read deadline alive while a UI mutation finishes.
                stream.write_all(b" ")?;
                match response_receiver.recv_timeout(EXECUTION_HEARTBEAT) {
                    Ok(response) => return Ok(Some(response)),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(closed_before_response());
                    }
                }
            }
            REQUEST_CANCELLED => return Ok(None),
            _ => return Err(io::Error::other("invalid automation request lifecycle")),
        }
    }
}

fn closed_before_response() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "Strek closed before processing the automation request",
    )
}

fn read_request_line(stream: &mut transport::Stream, deadline: Instant) -> io::Result<String> {
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];

    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "request deadline exceeded"))?;
        stream.set_read_timeout(Some(remaining))?;

        let available = MAX_REQUEST_BYTES + 1 - bytes.len();
        if available == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            ));
        }
        let chunk_len = chunk.len().min(available);
        let count = stream.read(&mut chunk[..chunk_len])?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended before a newline",
            ));
        }

        if let Some(newline) = chunk[..count].iter().position(|byte| *byte == b'\n') {
            bytes.extend_from_slice(&chunk[..=newline]);
            if bytes.len() > MAX_REQUEST_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
                ));
            }
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            ));
        }
    }

    String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.utf8_error()))
}

fn write_protocol_error(writer: &mut impl Write, message: impl Into<String>) -> io::Result<()> {
    let message = message.into();
    write_json_line(
        writer,
        &AutomationResponse {
            ok: false,
            protocol_version: AUTOMATION_PROTOCOL_VERSION,
            message: Some(message.clone()),
            error: Some(AutomationError {
                code: AutomationErrorCode::Rejected,
                message,
            }),
            receipt: None,
            state: None,
            document: None,
            document_query: None,
            capabilities: None,
            artifact: None,
        },
    )
}

fn write_json_line(mut writer: impl Write, response: &AutomationResponse) -> io::Result<()> {
    writer.write_all(&serialize_response_line(response)?)
}

fn serialize_response_line(response: &AutomationResponse) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(response).map_err(io::Error::other)?;
    bytes.push(b'\n');
    if bytes.len() <= MAX_RESPONSE_BYTES {
        return Ok(bytes);
    }

    let fallback = AutomationResponse {
        ok: response.ok,
        protocol_version: AUTOMATION_PROTOCOL_VERSION,
        message: Some(if response.ok {
            "request completed, but its response payload exceeded the automation response limit"
                .to_owned()
        } else {
            "request was rejected, but its details exceeded the automation response limit"
                .to_owned()
        }),
        error: response.error.clone(),
        receipt: response.receipt.clone(),
        state: None,
        document: None,
        document_query: None,
        capabilities: None,
        artifact: None,
    };
    let mut bytes = serde_json::to_vec(&fallback).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn request(request: AutomationRequest) -> Result<AutomationResponse, String> {
    request_call(AutomationCall::legacy(request))
}

pub(crate) fn request_call(call: AutomationCall) -> Result<AutomationResponse, String> {
    let request_line = serialize_call_line(&call)?;
    let mut stream =
        transport::connect().map_err(|error| format!("could not connect to Strek: {error}"))?;
    stream
        .set_read_timeout(Some(CLIENT_TIMEOUT))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(CLIENT_TIMEOUT))
        .map_err(|error| error.to_string())?;
    stream
        .write_all(&request_line)
        .map_err(|error| error.to_string())?;
    let response_json = read_response_line(stream).map_err(|error| error.to_string())?;
    serde_json::from_str(&response_json)
        .map_err(|error| format!("invalid response from Strek: {error}"))
}

fn read_response_line(reader: impl Read) -> io::Result<String> {
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::with_capacity(4096);
    reader
        .by_ref()
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("automation response exceeds {MAX_RESPONSE_BYTES} bytes"),
        ));
    }
    if !bytes.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "automation response ended before a newline",
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.utf8_error()))
}

#[cfg(test)]
fn serialize_request_line(request: &AutomationRequest) -> Result<Vec<u8>, String> {
    serialize_call_line(&AutomationCall::legacy(request.clone()))
}

fn serialize_call_line(call: &AutomationCall) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(call).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(format!(
            "automation request exceeds {MAX_REQUEST_BYTES} bytes"
        ));
    }
    Ok(bytes)
}

pub(crate) fn run_cli(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let command = args.next().unwrap_or_else(|| "state".to_owned());
    if command == "guarded" {
        let call = parse_guarded_call(&mut args)?;
        let response = request_call(call)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?
        );
        return response.into_result().map(|_| ());
    }
    if command == "screenshot" {
        let path = args
            .next()
            .ok_or_else(|| "usage: strek automate screenshot <output.png>".to_owned())?;
        if args.next().is_some() {
            return Err("usage: strek automate screenshot <output.png>".to_owned());
        }
        let png = capture_screenshot()?;
        fs::write(&path, png).map_err(|error| format!("could not write {path}: {error}"))?;
        println!("{path}");
        return Ok(());
    }

    let automation_request = parse_request(command, &mut args)?;
    let response = request(automation_request)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?
    );
    if response.ok {
        Ok(())
    } else {
        Err(response
            .message
            .unwrap_or_else(|| "automation request failed".to_owned()))
    }
}

fn parse_guarded_call(args: &mut impl Iterator<Item = String>) -> Result<AutomationCall, String> {
    let mut metadata = AutomationRequestMetadata {
        protocol_version: Some(AUTOMATION_PROTOCOL_VERSION),
        ..AutomationRequestMetadata::default()
    };
    let command = loop {
        let argument = args.next().ok_or_else(|| {
            "usage: strek automate guarded [guards] -- <command> [arguments]".to_owned()
        })?;
        match argument.as_str() {
            "--" => {
                break args
                    .next()
                    .ok_or_else(|| "guarded automation requires a command after `--`".to_owned())?
            }
            "--request-id" => metadata.request_id = Some(required_arg(args, "request id")?),
            "--request-sequence" => {
                metadata.request_sequence = Some(parse_u64_arg(args, "request sequence")?)
            }
            "--session-id" => metadata.session_id = Some(required_arg(args, "session id")?),
            "--document-revision" => {
                metadata.if_document_revision = Some(parse_u64_arg(args, "document revision")?)
            }
            "--workspace-revision" => {
                metadata.if_workspace_revision = Some(parse_u64_arg(args, "workspace revision")?)
            }
            _ => return Err(format!("unknown guarded automation option `{argument}`")),
        }
    };
    let request = parse_request(command, args)?;
    Ok(AutomationCall { metadata, request })
}

fn required_arg(
    args: &mut impl Iterator<Item = String>,
    description: &str,
) -> Result<String, String> {
    args.next().ok_or_else(|| format!("missing {description}"))
}

fn parse_u64_arg(
    args: &mut impl Iterator<Item = String>,
    description: &str,
) -> Result<u64, String> {
    let value = required_arg(args, description)?;
    value
        .parse()
        .map_err(|_| format!("{description} must be an unsigned integer"))
}

fn parse_request(
    command: String,
    args: &mut impl Iterator<Item = String>,
) -> Result<AutomationRequest, String> {
    match command.as_str() {
        "state" => no_more_args(args, AutomationRequest::State),
        "capabilities" => no_more_args(args, AutomationRequest::Capabilities),
        "document" => no_more_args(args, AutomationRequest::Document),
        "query-document" => {
            let mut ids = Vec::new();
            let mut include_descendants = false;
            let mut include_style = false;
            let mut include_geometry = false;
            for argument in args {
                match argument.as_str() {
                    "--descendants" => include_descendants = true,
                    "--style" => include_style = true,
                    "--geometry" => include_geometry = true,
                    option if option.starts_with("--") => {
                        return Err(format!("unknown query-document option `{option}`"));
                    }
                    _ => ids.push(argument),
                }
            }
            Ok(AutomationRequest::QueryDocument {
                ids,
                include_descendants,
                include_style,
                include_geometry,
            })
        }
        "new" => {
            let discard_changes = match args.next().as_deref() {
                None => false,
                Some("--discard") => true,
                Some(argument) => return Err(format!("unknown new option `{argument}`")),
            };
            no_more_args(
                args,
                AutomationRequest::NewDocument { discard_changes },
            )
        }
        "open" => {
            let path = next_value(args, "document path")?;
            let discard_changes = match args.next().as_deref() {
                None => false,
                Some("--discard") => true,
                Some(argument) => return Err(format!("unknown open option `{argument}`")),
            };
            no_more_args(
                args,
                AutomationRequest::OpenDocument {
                    path,
                    discard_changes,
                },
            )
        }
        "save" => {
            let path = next_value(args, "document path")?;
            no_more_args(args, AutomationRequest::SaveDocument { path })
        }
        "export" => {
            let format = next_value(args, "export format")?.parse::<ArtifactFormat>()?;
            let path = args.next();
            no_more_args(args, AutomationRequest::Export { format, path })
        }
        "activate" => no_more_args(args, AutomationRequest::Activate),
        "action" => {
            let id = args
                .next()
                .ok_or_else(|| "usage: strek automate action <command-id>".to_owned())?;
            no_more_args(args, AutomationRequest::Action { id })
        }
        "select" => {
            let mode = next_value(args, "selection mode")?.parse::<SelectionMode>()?;
            let ids = args.by_ref().collect::<Vec<_>>();
            Ok(AutomationRequest::Select { ids, mode })
        }
        "color" => {
            let target = next_value(args, "color target")?.parse::<ColorTarget>()?;
            let value = next_value(args, "HEX color or none")?;
            let color = (!value.eq_ignore_ascii_case("none")).then_some(value);
            no_more_args(args, AutomationRequest::SetColor { target, color })
        }
        "property" => {
            let target = next_value(args, "property target")?.parse::<NumericProperty>()?;
            let value = parse_number(next_value(args, "property value")?, "property value")?;
            no_more_args(
                args,
                AutomationRequest::SetNumericProperty { target, value },
            )
        }
        "layer" => {
            let id = next_value(args, "layer ID")?;
            let mut name = None;
            let mut visible = None;
            let mut locked = None;
            while let Some(option) = args.next() {
                match option.as_str() {
                    "--name" => name = Some(next_value(args, "layer name")?),
                    "--visible" => {
                        visible = Some(parse_bool(next_value(args, "visible")?, "visible")?);
                    }
                    "--locked" => {
                        locked = Some(parse_bool(next_value(args, "locked")?, "locked")?);
                    }
                    _ => return Err(format!("unknown layer option `{option}`")),
                }
            }
            Ok(AutomationRequest::SetLayerProperties {
                ids: vec![id],
                name,
                visible,
                locked,
            })
        }
        "pointer" => {
            let phase = next_value(args, "pointer phase")?.parse()?;
            let x = parse_number(next_value(args, "pointer x")?, "x coordinate")?;
            let y = parse_number(next_value(args, "pointer y")?, "y coordinate")?;
            let button = args
                .next()
                .map(|button| button.parse())
                .transpose()?
                .unwrap_or_default();
            no_more_args(
                args,
                AutomationRequest::Pointer {
                    phase,
                    x,
                    y,
                    button,
                    modifiers: AutomationModifiers::default(),
                },
            )
        }
        "text" => {
            let text = args.collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                Err("usage: strek automate text <text>".to_owned())
            } else {
                Ok(AutomationRequest::Text { text })
            }
        }
        "ui" => {
            let target = next_value(args, "UI target")?.parse()?;
            let visible = match next_value(args, "show or hide")?.as_str() {
                "show" => true,
                "hide" => false,
                value => return Err(format!("unknown UI visibility `{value}`; use show or hide")),
            };
            no_more_args(args, AutomationRequest::SetUi { target, visible })
        }
        _ => Err(format!(
            "unknown automation command `{command}`; use state, document, new, open, save, export, action, select, color, property, layer, pointer, text, ui, activate, or screenshot"
        )),
    }
}

fn no_more_args(
    args: &mut impl Iterator<Item = String>,
    request: AutomationRequest,
) -> Result<AutomationRequest, String> {
    if let Some(argument) = args.next() {
        Err(format!("unexpected argument `{argument}`"))
    } else {
        Ok(request)
    }
}

fn next_value(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("missing {name}"))
}

fn parse_number(value: String, name: &str) -> Result<f32, String> {
    let value = value
        .parse::<f32>()
        .map_err(|_| format!("invalid {name} `{value}`"))?;
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| format!("{name} must be finite"))
}

fn parse_bool(value: String, name: &str) -> Result<bool, String> {
    match value.as_str() {
        "true" | "show" | "on" => Ok(true),
        "false" | "hide" | "off" => Ok(false),
        _ => Err(format!("{name} must be true or false")),
    }
}

pub(crate) fn capture_screenshot() -> Result<Vec<u8>, String> {
    let response = request(screenshot_state_request())?.into_result()?;
    let state = response
        .state
        .ok_or_else(|| "Strek did not return window state".to_owned())?;
    std::thread::sleep(Duration::from_millis(75));
    capture_window(state.process_id, state.window)
}

fn screenshot_state_request() -> AutomationRequest {
    AutomationRequest::State
}

#[cfg(target_os = "macos")]
fn capture_window(process_id: u32, legacy_bounds: AutomationBounds) -> Result<Vec<u8>, String> {
    if process_id == 0 {
        return capture_bounds(legacy_bounds);
    }
    let window_id = window_id_for_process(process_id, legacy_bounds)
        .ok_or_else(|| "could not find Strek's visible macOS window".to_owned())?;
    capture_with_arguments(&["-x".to_owned(), "-o".to_owned(), format!("-l{window_id}")])
}

#[cfg(target_os = "macos")]
fn window_id_for_process(process_id: u32, expected_bounds: AutomationBounds) -> Option<u32> {
    let windows = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    )?;
    let mut best_match = None;
    for raw_window in windows.iter() {
        let Some(window) =
            unsafe { CFType::wrap_under_get_rule(*raw_window as _) }.downcast::<CFDictionary>()
        else {
            continue;
        };
        let Some(owner_pid) = dictionary_number(&window, unsafe { kCGWindowOwnerPID }) else {
            continue;
        };
        let Some(layer) = dictionary_number(&window, unsafe { kCGWindowLayer }) else {
            continue;
        };
        if owner_pid != i64::from(process_id) || layer != 0 {
            continue;
        }
        let Some(window_id) = dictionary_number(&window, unsafe { kCGWindowNumber })
            .and_then(|number| u32::try_from(number).ok())
        else {
            continue;
        };
        let Some(bounds) = dictionary_bounds(&window, unsafe { kCGWindowBounds }) else {
            continue;
        };
        let score = window_bounds_distance(bounds, expected_bounds);
        if best_match.is_none_or(|(_, best_score)| score < best_score) {
            best_match = Some((window_id, score));
        }
    }
    best_match.map(|(window_id, _)| window_id)
}

#[cfg(target_os = "macos")]
fn dictionary_number(dictionary: &CFDictionary, key: CFStringRef) -> Option<i64> {
    let key = unsafe { CFString::wrap_under_get_rule(key) };
    let value = dictionary.find(key.as_CFTypeRef())?;
    let value = unsafe { CFType::wrap_under_get_rule(*value as _) };
    value.downcast::<CFNumber>()?.to_i64()
}

#[cfg(target_os = "macos")]
fn dictionary_bounds(dictionary: &CFDictionary, key: CFStringRef) -> Option<CGRect> {
    let key = unsafe { CFString::wrap_under_get_rule(key) };
    let value = dictionary.find(key.as_CFTypeRef())?;
    let value = unsafe { CFType::wrap_under_get_rule(*value as _) };
    CGRect::from_dict_representation(&value.downcast::<CFDictionary>()?)
}

#[cfg(target_os = "macos")]
fn window_bounds_distance(bounds: CGRect, expected: AutomationBounds) -> f64 {
    (bounds.origin.x - f64::from(expected.x)).abs()
        + (bounds.origin.y - f64::from(expected.y)).abs()
        + (bounds.size.width - f64::from(expected.width)).abs()
        + (bounds.size.height - f64::from(expected.height)).abs()
}

#[cfg(target_os = "macos")]
fn capture_bounds(bounds: AutomationBounds) -> Result<Vec<u8>, String> {
    let values = [bounds.x, bounds.y, bounds.width, bounds.height];
    if values.iter().any(|value| !value.is_finite()) || bounds.width <= 0.0 || bounds.height <= 0.0
    {
        return Err("Strek returned invalid window bounds".to_owned());
    }
    let region = format!(
        "{},{},{},{}",
        bounds.x.round() as i32,
        bounds.y.round() as i32,
        bounds.width.round() as u32,
        bounds.height.round() as u32
    );
    capture_with_arguments(&["-x".to_owned(), "-R".to_owned(), region])
}

#[cfg(target_os = "macos")]
fn capture_with_arguments(arguments: &[String]) -> Result<Vec<u8>, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let screenshot = TemporaryScreenshot(
        env::temp_dir().join(format!("strek-{}-{stamp}.png", std::process::id())),
    );
    let output = Command::new("/usr/sbin/screencapture")
        .args(arguments)
        .arg(&screenshot.0)
        .output()
        .map_err(|error| format!("could not run screencapture: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            "macOS could not capture Strek; allow Screen Recording access and try again".to_owned()
        } else {
            detail
        });
    }
    fs::read(&screenshot.0).map_err(|error| format!("could not read screenshot: {error}"))
}

#[cfg(not(target_os = "macos"))]
fn capture_window(_process_id: u32, _bounds: AutomationBounds) -> Result<Vec<u8>, String> {
    Err("window screenshots are currently supported on macOS".to_owned())
}

#[cfg(target_os = "macos")]
struct TemporaryScreenshot(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for TemporaryScreenshot {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(crate) fn endpoint_display() -> String {
    transport::endpoint_display()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn capability_registry_is_complete_unique_and_index_aligned() {
        assert_eq!(
            AUTOMATION_CAPABILITY_SPECS.len(),
            AutomationOperationKind::ALL.len()
        );
        let mut ids = HashSet::new();
        for (kind, spec) in AutomationOperationKind::ALL
            .into_iter()
            .zip(AUTOMATION_CAPABILITY_SPECS)
        {
            assert_eq!(kind.spec().id, spec.id);
            assert!(ids.insert(spec.id), "duplicate capability `{}`", spec.id);
            let mut parameters = HashSet::new();
            for parameter in spec.parameters {
                assert!(
                    parameters.insert(parameter.name),
                    "duplicate parameter `{}` on `{}`",
                    parameter.name,
                    spec.id
                );
                assert!(!parameter.description.is_empty());
            }
        }
    }

    #[test]
    fn legacy_request_shape_deserializes_as_an_automation_call() {
        let call: AutomationCall = serde_json::from_str(r#"{"type":"state"}"#).unwrap();
        assert!(matches!(call.request, AutomationRequest::State));
        assert_eq!(call.metadata.protocol_version, None);
    }

    #[test]
    fn capabilities_use_the_legacy_wire_shape() {
        let line = serialize_request_line(&AutomationRequest::Capabilities).unwrap();
        assert_eq!(line, b"{\"type\":\"capabilities\"}\n");
    }

    #[test]
    fn projected_document_cli_parses_explicit_cost_options() {
        let mut args = ["--descendants", "--style", "--geometry", "node:7"]
            .into_iter()
            .map(str::to_owned);
        let request = parse_request("query-document".to_owned(), &mut args).unwrap();
        assert!(matches!(
            request,
            AutomationRequest::QueryDocument {
                ids,
                include_descendants: true,
                include_style: true,
                include_geometry: true,
            } if ids == ["node:7"]
        ));
    }

    #[test]
    fn guarded_cli_call_carries_revisions_and_request_identity() {
        let mut args = [
            "--request-id",
            "agent-step-7",
            "--request-sequence",
            "4",
            "--session-id",
            "session-1",
            "--document-revision",
            "12",
            "--workspace-revision",
            "9",
            "--",
            "action",
            "edit.undo",
        ]
        .into_iter()
        .map(str::to_owned);
        let call = parse_guarded_call(&mut args).unwrap();

        assert_eq!(call.metadata.request_id.as_deref(), Some("agent-step-7"));
        assert_eq!(call.metadata.request_sequence, Some(4));
        assert_eq!(call.metadata.session_id.as_deref(), Some("session-1"));
        assert_eq!(call.metadata.if_document_revision, Some(12));
        assert_eq!(call.metadata.if_workspace_revision, Some(9));
        assert!(matches!(
            call.request,
            AutomationRequest::Action { id } if id == "edit.undo"
        ));
    }

    #[test]
    fn cli_pointer_request_rejects_non_finite_coordinates() {
        let mut args = ["down", "NaN", "12"].into_iter().map(str::to_owned);
        let error = parse_request("pointer".to_owned(), &mut args).unwrap_err();
        assert_eq!(error, "x coordinate must be finite");
    }

    #[test]
    fn cli_ui_names_accept_shell_friendly_hyphens() {
        let mut args = ["command-palette", "show"].into_iter().map(str::to_owned);
        assert!(matches!(
            parse_request("ui".to_owned(), &mut args),
            Ok(AutomationRequest::SetUi {
                target: UiTarget::CommandPalette,
                visible: true
            })
        ));

        let mut args = ["fill-color-picker", "show"].into_iter().map(str::to_owned);
        assert!(matches!(
            parse_request("ui".to_owned(), &mut args),
            Ok(AutomationRequest::SetUi {
                target: UiTarget::FillColorPicker,
                visible: true
            })
        ));

        let mut args = ["frame-background-color-picker", "show"]
            .into_iter()
            .map(str::to_owned);
        assert!(matches!(
            parse_request("ui".to_owned(), &mut args),
            Ok(AutomationRequest::SetUi {
                target: UiTarget::FrameBackgroundColorPicker,
                visible: true
            })
        ));
    }

    #[test]
    fn cli_parses_document_and_semantic_editing_requests() {
        let mut no_args = std::iter::empty();
        assert!(matches!(
            parse_request("document".to_owned(), &mut no_args),
            Ok(AutomationRequest::Document)
        ));

        let mut args = ["add", "node-0000000000000001"]
            .into_iter()
            .map(str::to_owned);
        assert!(matches!(
            parse_request("select".to_owned(), &mut args),
            Ok(AutomationRequest::Select {
                mode: SelectionMode::Add,
                ids
            }) if ids == ["node-0000000000000001"]
        ));

        let mut args = ["fill", "#336699cc"].into_iter().map(str::to_owned);
        assert!(matches!(
            parse_request("color".to_owned(), &mut args),
            Ok(AutomationRequest::SetColor {
                target: ColorTarget::Fill,
                color: Some(color)
            }) if color == "#336699cc"
        ));

        let mut args = ["frame-background", "#336699cc"]
            .into_iter()
            .map(str::to_owned);
        assert!(matches!(
            parse_request("color".to_owned(), &mut args),
            Ok(AutomationRequest::SetColor {
                target: ColorTarget::FrameBackground,
                color: Some(color)
            }) if color == "#336699cc"
        ));

        let mut args = ["opacity", "0.5"].into_iter().map(str::to_owned);
        assert!(matches!(
            parse_request("property".to_owned(), &mut args),
            Ok(AutomationRequest::SetNumericProperty {
                target: NumericProperty::Opacity,
                value: 0.5
            })
        ));
    }

    #[test]
    fn cli_export_supports_inline_and_explicit_path_results() {
        let mut inline = ["png"].into_iter().map(str::to_owned);
        assert!(matches!(
            parse_request("export".to_owned(), &mut inline),
            Ok(AutomationRequest::Export {
                format: ArtifactFormat::Png,
                path: None
            })
        ));

        let export_path = std::env::temp_dir().join("logo.svg");
        let export_path = export_path.to_string_lossy().into_owned();
        let mut to_path = ["svg".to_owned(), export_path.clone()].into_iter();
        assert!(matches!(
            parse_request("export".to_owned(), &mut to_path),
            Ok(AutomationRequest::Export {
                format: ArtifactFormat::Svg,
                path: Some(path)
            }) if path == export_path
        ));
    }

    #[test]
    fn screenshots_read_window_state_without_activating_the_application() {
        assert!(matches!(
            screenshot_state_request(),
            AutomationRequest::State
        ));
    }

    #[test]
    fn expired_pending_request_never_executes() {
        let (responder, response) = std::sync::mpsc::sync_channel(1);
        let executed = Cell::new(false);
        let pending = PendingRequest {
            call: AutomationCall::legacy(AutomationRequest::State),
            responder,
            deadline: Instant::now()
                .checked_sub(Duration::from_secs(1))
                .expect("one second before now should be representable"),
            lifecycle: Arc::new(RequestLifecycle::pending()),
        };

        assert!(!pending.respond_with(|_| {
            executed.set(true);
            panic!("an expired request must not execute")
        }));
        assert!(!executed.get());
        assert!(matches!(
            response.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn executing_request_cannot_be_cancelled_as_timed_out() {
        let (responder, response) = std::sync::mpsc::sync_channel(1);
        let lifecycle = Arc::new(RequestLifecycle::pending());
        let pending = PendingRequest {
            call: AutomationCall::legacy(AutomationRequest::State),
            responder,
            deadline: Instant::now() + Duration::from_secs(1),
            lifecycle: Arc::clone(&lifecycle),
        };
        let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(0);
        let (finish_sender, finish_receiver) = std::sync::mpsc::sync_channel(0);

        let worker = std::thread::spawn(move || {
            pending.respond_with(|_| {
                started_sender.send(()).unwrap();
                finish_receiver.recv().unwrap();
                AutomationResponse {
                    ok: true,
                    protocol_version: AUTOMATION_PROTOCOL_VERSION,
                    message: None,
                    error: None,
                    receipt: None,
                    state: None,
                    document: None,
                    document_query: None,
                    capabilities: None,
                    artifact: None,
                }
            })
        });
        started_receiver.recv().unwrap();

        assert!(!lifecycle.cancel_pending());
        assert_eq!(lifecycle.state(), REQUEST_EXECUTING);
        finish_sender.send(()).unwrap();
        assert!(worker.join().unwrap());
        assert!(response.recv().unwrap().ok);
        assert_eq!(lifecycle.state(), REQUEST_COMPLETED);
    }

    #[test]
    fn executing_wait_keeps_the_client_alive_until_response() {
        let (mut client, mut server) = transport::pair_for_tests().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let (response_sender, response_receiver) = std::sync::mpsc::sync_channel(1);
        let lifecycle = Arc::new(RequestLifecycle::pending());
        assert!(lifecycle.begin_execution());
        let worker_lifecycle = Arc::clone(&lifecycle);
        let worker = std::thread::spawn(move || {
            wait_for_response(
                &mut server,
                response_receiver,
                &worker_lifecycle,
                Instant::now(),
            )
        });

        let mut heartbeat = [0_u8; 1];
        client.read_exact(&mut heartbeat).unwrap();
        assert_eq!(heartbeat, *b" ");
        response_sender
            .send(AutomationResponse {
                ok: true,
                protocol_version: AUTOMATION_PROTOCOL_VERSION,
                message: None,
                error: None,
                receipt: None,
                state: None,
                document: None,
                document_query: None,
                capabilities: None,
                artifact: None,
            })
            .unwrap();

        assert!(worker.join().unwrap().unwrap().unwrap().ok);
    }

    #[test]
    fn expired_queued_connection_is_rejected_before_enqueue() {
        let (client, server) = transport::pair_for_tests().unwrap();
        let (sender, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let deadline = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("one second before now should be representable");

        serve_connection(server, &sender, deadline).expect("expired connection should be rejected");

        let mut response_json = String::new();
        BufReader::new(client)
            .read_line(&mut response_json)
            .expect("protocol error should be readable");
        let response: AutomationResponse =
            serde_json::from_str(&response_json).expect("protocol error should be JSON");
        assert!(!response.ok);
        assert!(response
            .message
            .is_some_and(|message| message.contains("in time")));
        assert!(matches!(
            requests.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn oversized_request_is_rejected_before_connecting() {
        let request = AutomationRequest::Text {
            text: "x".repeat(MAX_REQUEST_BYTES),
        };
        assert!(serialize_request_line(&request)
            .expect_err("oversized request should be rejected")
            .contains("exceeds"));
    }

    #[test]
    fn response_reader_requires_a_bounded_newline_terminated_message() {
        let valid = read_response_line(io::Cursor::new(b" {\"ok\":true}\n")).unwrap();
        assert_eq!(valid, " {\"ok\":true}\n");

        let oversized = vec![b'x'; MAX_RESPONSE_BYTES + 1];
        assert_eq!(
            read_response_line(io::Cursor::new(oversized))
                .expect_err("oversized response should fail")
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            read_response_line(io::Cursor::new(b"{\"ok\":true}"))
                .expect_err("unterminated response should fail")
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn response_writer_preserves_outcome_when_state_is_too_large() {
        let response = AutomationResponse {
            ok: true,
            protocol_version: AUTOMATION_PROTOCOL_VERSION,
            message: Some("x".repeat(MAX_RESPONSE_BYTES)),
            error: None,
            receipt: None,
            state: None,
            document: None,
            document_query: None,
            capabilities: None,
            artifact: None,
        };

        let line = serialize_response_line(&response).unwrap();
        assert!(line.len() <= MAX_RESPONSE_BYTES);
        assert!(line.ends_with(b"\n"));
        let fallback: AutomationResponse = serde_json::from_slice(&line).unwrap();
        assert!(fallback.ok);
        assert!(fallback.state.is_none());
        assert!(fallback
            .message
            .unwrap()
            .contains("exceeded the automation response limit"));
    }
}
