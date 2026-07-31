//! Local automation protocol shared by the CLI, AppleScript, and MCP server.

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc::SyncSender, Arc};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(unix)]
use std::sync::{mpsc, Mutex};
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
#[cfg(unix)]
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const CLIENT_TIMEOUT: Duration = Duration::from_secs(6);
#[cfg(unix)]
const CONNECTION_WORKERS: usize = 4;
#[cfg(unix)]
const MAX_QUEUED_CONNECTIONS: usize = 16;
#[cfg(unix)]
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
#[cfg(unix)]
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
#[cfg(unix)]
const EXECUTION_HEARTBEAT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AutomationRequest {
    State,
    Action {
        id: String,
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
}

impl FromStr for UiTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "main_menu" | "main-menu" => Ok(Self::MainMenu),
            "command_palette" | "command-palette" => Ok(Self::CommandPalette),
            "layers_panel" | "layers-panel" => Ok(Self::LayersPanel),
            "design_panel" | "design-panel" => Ok(Self::DesignPanel),
            _ => Err(format!(
                "unknown UI target `{value}`; use main-menu, command-palette, layers-panel, or design-panel"
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
}

impl AutomationResponse {
    pub(crate) fn success(state: AutomationState, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: Some(message.into()),
            state: Some(state),
        }
    }

    pub(crate) fn error(state: AutomationState, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(message.into()),
            state: Some(state),
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

#[cfg(unix)]
struct QueuedConnection {
    stream: std::os::unix::net::UnixStream,
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        use std::os::unix::net::{UnixListener, UnixStream};

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
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => match connection_sender.try_send(QueuedConnection {
                            stream,
                            deadline: Instant::now() + RESPONSE_TIMEOUT,
                        }) {
                            Ok(()) => {}
                            Err(mpsc::TrySendError::Full(QueuedConnection {
                                mut stream, ..
                            })) => {
                                let _ = stream.set_write_timeout(Some(RESPONSE_TIMEOUT));
                                let _ = write_protocol_error(
                                    &mut stream,
                                    "Strek automation is busy; retry shortly",
                                );
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => return,
                        },
                        Err(error) => log::warn!("automation socket failed: {error}"),
                    }
                }
            })?;
        Ok(receiver)
    }

    #[cfg(not(unix))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Strek automation currently requires a Unix-domain socket",
        ))
    }
}

#[cfg(unix)]
fn serve_connection(
    mut stream: std::os::unix::net::UnixStream,
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

#[cfg(unix)]
fn wait_for_response(
    stream: &mut std::os::unix::net::UnixStream,
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

#[cfg(unix)]
fn closed_before_response() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "Strek closed before processing the automation request",
    )
}

#[cfg(unix)]
fn read_request_line(
    stream: &mut std::os::unix::net::UnixStream,
    deadline: Instant,
) -> io::Result<String> {
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
        },
    )
}

fn write_json_line(mut writer: impl Write, value: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut writer, value).map_err(io::Error::other)?;
    writer.write_all(b"\n")
}

pub(crate) fn request(request: AutomationRequest) -> Result<AutomationResponse, String> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;

        let request_line = serialize_request_line(&request)?;
        let mut stream = UnixStream::connect(socket_path())
            .map_err(|error| format!("could not connect to Strek: {error}"))?;
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

    #[cfg(not(unix))]
    {
        let _ = request;
        Err("Strek automation currently requires a Unix-domain socket".to_owned())
    }
}

#[cfg(unix)]
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

#[cfg(unix)]
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
        "activate" => no_more_args(args, AutomationRequest::Activate),
        "action" => {
            let id = args
                .next()
                .ok_or_else(|| "usage: strek automate action <command-id>".to_owned())?;
            no_more_args(args, AutomationRequest::Action { id })
        }
        "pointer" => {
            let phase = next_value(args, "pointer phase")?.parse()?;
            let x = parse_number(next_value(args, "pointer x")?, "x")?;
            let y = parse_number(next_value(args, "pointer y")?, "y")?;
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
            "unknown automation command `{command}`; use state, action, pointer, text, ui, activate, or screenshot"
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
        .map_err(|_| format!("invalid {name} coordinate `{value}`"))?;
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| format!("{name} coordinate must be finite"))
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

fn socket_path() -> PathBuf {
    env::var_os(SOCKET_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path)
}

#[cfg(unix)]
fn default_socket_path() -> PathBuf {
    let user_id = unsafe { libc::geteuid() };
    let base = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(env::temp_dir);
    base.join(format!("strek-{user_id}"))
        .join("automation.sock")
}

#[cfg(not(unix))]
fn default_socket_path() -> PathBuf {
    env::temp_dir().join("strek-automation.sock")
}

#[cfg(unix)]
fn prepare_default_socket_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

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

pub(crate) fn socket_path_display() -> String {
    socket_path().display().to_string()
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

    #[cfg(unix)]
    #[test]
    fn executing_wait_keeps_the_client_alive_until_response() {
        use std::os::unix::net::UnixStream;

        let (mut client, mut server) = UnixStream::pair().unwrap();
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
            })
            .unwrap();

        assert!(worker.join().unwrap().unwrap().unwrap().ok);
    }

    #[cfg(unix)]
    #[test]
    fn expired_queued_connection_is_rejected_before_enqueue() {
        use std::os::unix::net::UnixStream;

        let (client, server) = UnixStream::pair().expect("socket pair should open");
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

    #[cfg(unix)]
    #[test]
    fn oversized_request_is_rejected_before_connecting() {
        let request = AutomationRequest::Text {
            text: "x".repeat(MAX_REQUEST_BYTES),
        };
        assert!(serialize_request_line(&request)
            .expect_err("oversized request should be rejected")
            .contains("exceeds"));
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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
