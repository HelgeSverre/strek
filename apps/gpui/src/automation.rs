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
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const EXECUTION_HEARTBEAT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AutomationRequest {
    State,
    Document,
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
}

impl FromStr for ColorTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "fill" => Ok(Self::Fill),
            "stroke" => Ok(Self::Stroke),
            _ => Err(format!(
                "unknown color target `{value}`; use fill or stroke"
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
            "color_library" | "color-library" => Ok(Self::ColorLibrary),
            "precision_controls" | "precision-controls" => Ok(Self::PrecisionControls),
            _ => Err(format!(
                "unknown UI target `{value}`; use main-menu, command-palette, layers-panel, design-panel, fill-color-picker, stroke-color-picker, color-library, or precision-controls"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<AutomationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<AutomationDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<AutomationArtifact>,
}

impl AutomationResponse {
    pub(crate) fn success(state: AutomationState, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: Some(message.into()),
            state: Some(state),
            document: None,
            artifact: None,
        }
    }

    pub(crate) fn error(state: AutomationState, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(message.into()),
            state: Some(state),
            document: None,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutomationDocument {
    pub root_id: String,
    pub layers: Vec<AutomationLayer>,
    pub grid: AutomationGrid,
    pub guides: Vec<AutomationGuide>,
    pub color_groups: Vec<AutomationColorGroup>,
    pub saved_colors: Vec<AutomationSavedColor>,
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
    request: AutomationRequest,
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
    pub(crate) fn respond_with(
        self,
        make_response: impl FnOnce(AutomationRequest) -> AutomationResponse,
    ) -> bool {
        if Instant::now() >= self.deadline {
            self.lifecycle.cancel_pending();
            return false;
        }
        if !self.lifecycle.begin_execution() {
            return false;
        }
        let response = make_response(self.request);
        let sent = self.responder.send(response).is_ok();
        self.lifecycle.complete();
        sent
    }
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
    let request = match serde_json::from_str(&request_json) {
        Ok(request) => request,
        Err(error) => {
            write_protocol_error(&mut stream, format!("invalid automation request: {error}"))?;
            return Ok(());
        }
    };
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    let lifecycle = Arc::new(RequestLifecycle::pending());
    sender
        .send(PendingRequest {
            request,
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
    write_json_line(
        writer,
        &AutomationResponse {
            ok: false,
            message: Some(message.into()),
            state: None,
            document: None,
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
        message: Some(if response.ok {
            "request completed, but its response payload exceeded the automation response limit"
                .to_owned()
        } else {
            "request was rejected, but its details exceeded the automation response limit"
                .to_owned()
        }),
        state: None,
        document: None,
        artifact: None,
    };
    let mut bytes = serde_json::to_vec(&fallback).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn request(request: AutomationRequest) -> Result<AutomationResponse, String> {
    let request_line = serialize_request_line(&request)?;
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

fn serialize_request_line(request: &AutomationRequest) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(request).map_err(|error| error.to_string())?;
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

fn parse_request(
    command: String,
    args: &mut impl Iterator<Item = String>,
) -> Result<AutomationRequest, String> {
    match command.as_str() {
        "state" => no_more_args(args, AutomationRequest::State),
        "document" => no_more_args(args, AutomationRequest::Document),
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
    let response = request(AutomationRequest::Activate)?.into_result()?;
    let state = response
        .state
        .ok_or_else(|| "Strek did not return window state".to_owned())?;
    std::thread::sleep(Duration::from_millis(75));
    capture_window(state.process_id, state.window)
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

    use super::*;

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

        let mut to_path = ["svg", "/tmp/logo.svg"].into_iter().map(str::to_owned);
        assert!(matches!(
            parse_request("export".to_owned(), &mut to_path),
            Ok(AutomationRequest::Export {
                format: ArtifactFormat::Svg,
                path: Some(path)
            }) if path == "/tmp/logo.svg"
        ));
    }

    #[test]
    fn expired_pending_request_never_executes() {
        let (responder, response) = std::sync::mpsc::sync_channel(1);
        let executed = Cell::new(false);
        let pending = PendingRequest {
            request: AutomationRequest::State,
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
            request: AutomationRequest::State,
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
                    message: None,
                    state: None,
                    document: None,
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
                message: None,
                state: None,
                document: None,
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
            message: Some("x".repeat(MAX_RESPONSE_BYTES)),
            state: None,
            document: None,
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
