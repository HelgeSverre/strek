//! Local automation protocol shared by the CLI, AppleScript, and MCP server.

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(unix)]
use std::sync::{mpsc, Arc, Mutex};
#[cfg(target_os = "macos")]
use std::time::{SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const SOCKET_ENV: &str = "STREK_AUTOMATION_SOCKET";
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const CONNECTION_WORKERS: usize = 4;
#[cfg(unix)]
const MAX_QUEUED_CONNECTIONS: usize = 16;
#[cfg(unix)]
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

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
    pub document: String,
    pub dirty: bool,
    pub tool: String,
    pub interaction: String,
    pub selection_count: usize,
    pub selected_layers: Vec<String>,
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
}

impl PendingRequest {
    pub(crate) fn respond_with(
        self,
        make_response: impl FnOnce(AutomationRequest) -> AutomationResponse,
    ) -> bool {
        if Instant::now() >= self.deadline {
            return false;
        }
        let response = make_response(self.request);
        self.responder.send(response).is_ok()
    }
}

pub(crate) fn start_server() -> io::Result<tokio::sync::mpsc::UnboundedReceiver<PendingRequest>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        use std::os::unix::net::{UnixListener, UnixStream};

        let path = socket_path();
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
        let (connection_sender, connection_receiver) = mpsc::sync_channel(MAX_QUEUED_CONNECTIONS);
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
                    let Ok(stream) = stream else {
                        return;
                    };
                    if let Err(error) = serve_connection(stream, &sender) {
                        log::warn!("automation request failed: {error}");
                    }
                })?;
        }

        std::thread::Builder::new()
            .name("strek-automation-accept".to_owned())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => match connection_sender.try_send(stream) {
                            Ok(()) => {}
                            Err(mpsc::TrySendError::Full(mut stream)) => {
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
) -> io::Result<()> {
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    stream.set_write_timeout(Some(RESPONSE_TIMEOUT))?;
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
    sender
        .send(PendingRequest {
            request,
            responder: response_sender,
            deadline,
        })
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Strek window closed"))?;
    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
        write_protocol_error(
            &mut stream,
            "Strek did not process the automation request in time",
        )?;
        return Ok(());
    };
    let response = match response_receiver.recv_timeout(remaining) {
        Ok(response) => response,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            write_protocol_error(
                &mut stream,
                "Strek did not process the automation request in time",
            )?;
            return Ok(());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Strek closed before processing the automation request",
            ));
        }
    };
    write_json_line(&mut stream, &response)
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

        let mut stream = UnixStream::connect(socket_path())
            .map_err(|error| format!("could not connect to Strek: {error}"))?;
        stream
            .set_read_timeout(Some(RESPONSE_TIMEOUT))
            .map_err(|error| error.to_string())?;
        write_json_line(&mut stream, &request).map_err(|error| error.to_string())?;
        let mut response_json = String::new();
        BufReader::new(stream)
            .read_line(&mut response_json)
            .map_err(|error| error.to_string())?;
        serde_json::from_str(&response_json)
            .map_err(|error| format!("invalid response from Strek: {error}"))
    }

    #[cfg(not(unix))]
    {
        let _ = request;
        Err("Strek automation currently requires a Unix-domain socket".to_owned())
    }
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
    let bounds = response
        .state
        .ok_or_else(|| "Strek did not return window bounds".to_owned())?
        .window;
    std::thread::sleep(Duration::from_millis(75));
    capture_bounds(bounds)
}

#[cfg(target_os = "macos")]
fn capture_bounds(bounds: AutomationBounds) -> Result<Vec<u8>, String> {
    let values = [bounds.x, bounds.y, bounds.width, bounds.height];
    if values.iter().any(|value| !value.is_finite()) || bounds.width <= 0.0 || bounds.height <= 0.0
    {
        return Err("Strek returned invalid window bounds".to_owned());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path = env::temp_dir().join(format!("strek-{}-{stamp}.png", std::process::id()));
    let region = format!(
        "{},{},{},{}",
        bounds.x.round() as i32,
        bounds.y.round() as i32,
        bounds.width.round() as u32,
        bounds.height.round() as u32
    );
    let output = Command::new("/usr/sbin/screencapture")
        .args(["-x", "-R", &region])
        .arg(&path)
        .output()
        .map_err(|error| format!("could not run screencapture: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let png = fs::read(&path).map_err(|error| format!("could not read screenshot: {error}"))?;
    let _ = fs::remove_file(path);
    Ok(png)
}

#[cfg(not(target_os = "macos"))]
fn capture_bounds(_bounds: AutomationBounds) -> Result<Vec<u8>, String> {
    Err("window screenshots are currently supported on macOS".to_owned())
}

fn socket_path() -> PathBuf {
    env::var_os(SOCKET_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("strek-automation.sock"))
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
}
