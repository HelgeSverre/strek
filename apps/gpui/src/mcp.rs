//! Stdio MCP server backed by the running Strek automation socket.

use std::str::FromStr;

use base64::Engine;
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo},
    schemars::JsonSchema,
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use serde::Deserialize;

use crate::automation::{
    self, AutomationModifiers, AutomationRequest, PointerButton, PointerPhase, UiTarget,
};

#[derive(Debug, Deserialize, JsonSchema)]
struct ActionParams {
    #[schemars(description = "Command ID from get_state, for example tool.rectangle or edit.undo")]
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PointerParams {
    #[schemars(description = "Pointer phase: down, move, or up")]
    phase: String,
    #[schemars(description = "Canvas-local horizontal position in pixels")]
    x: f32,
    #[schemars(description = "Canvas-local vertical position in pixels")]
    y: f32,
    #[serde(default)]
    #[schemars(description = "Pointer button: left, middle, or right; defaults to left")]
    button: Option<String>,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    control: bool,
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    command: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TextParams {
    #[schemars(description = "Text to insert into the active Strek text edit session")]
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UiParams {
    #[schemars(
        description = "UI target: main-menu, command-palette, layers-panel, or design-panel"
    )]
    target: String,
    #[schemars(description = "Whether the target should be visible")]
    visible: bool,
}

#[derive(Debug, Clone)]
struct StrekMcp;

#[tool_router]
impl StrekMcp {
    #[tool(
        description = "Inspect the running Strek editor, window, canvas, selection, and actions"
    )]
    fn get_state(&self) -> Result<String, String> {
        response_json(AutomationRequest::State)
    }

    #[tool(description = "Run an enabled Strek editor command by ID without using the keyboard")]
    fn perform_action(
        &self,
        Parameters(ActionParams { id }): Parameters<ActionParams>,
    ) -> Result<String, String> {
        response_json(AutomationRequest::Action { id })
    }

    #[tool(
        description = "Send a canvas-local pointer event without moving the system cursor. Use down, move, then up to draw or drag."
    )]
    fn pointer(&self, Parameters(params): Parameters<PointerParams>) -> Result<String, String> {
        let phase = PointerPhase::from_str(&params.phase)?;
        let button = params
            .button
            .as_deref()
            .map(PointerButton::from_str)
            .transpose()?
            .unwrap_or_default();
        response_json(AutomationRequest::Pointer {
            phase,
            x: params.x,
            y: params.y,
            button,
            modifiers: AutomationModifiers {
                shift: params.shift,
                control: params.control,
                alt: params.alt,
                command: params.command,
            },
        })
    }

    #[tool(description = "Insert text into the active Strek text editing session")]
    fn insert_text(
        &self,
        Parameters(TextParams { text }): Parameters<TextParams>,
    ) -> Result<String, String> {
        response_json(AutomationRequest::Text { text })
    }

    #[tool(description = "Show or hide Strek panels and overlays without clicking the UI")]
    fn set_ui(
        &self,
        Parameters(UiParams { target, visible }): Parameters<UiParams>,
    ) -> Result<String, String> {
        response_json(AutomationRequest::SetUi {
            target: UiTarget::from_str(&target)?,
            visible,
        })
    }

    #[tool(
        description = "Activate Strek and capture its complete window as a PNG image without moving the system cursor"
    )]
    fn screenshot(&self) -> Result<CallToolResult, String> {
        let png = automation::capture_screenshot()?;
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        Ok(CallToolResult::success(vec![ContentBlock::image(
            data,
            "image/png",
        )]))
    }
}

#[tool_handler]
impl ServerHandler for StrekMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "strek",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Automate a running Strek window without moving the system cursor. Call get_state before sending actions or canvas-local pointer events.",
            )
    }
}

fn response_json(request: AutomationRequest) -> Result<String, String> {
    let response = automation::request(request)?.into_result()?;
    serde_json::to_string_pretty(&response).map_err(|error| error.to_string())
}

pub(crate) fn run() -> Result<(), String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?
        .block_on(async {
            StrekMcp
                .serve(rmcp::transport::stdio())
                .await
                .map_err(|error| error.to_string())?
                .waiting()
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
}
