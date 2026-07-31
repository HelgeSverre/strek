//! Stdio MCP server backed by the running Strek automation socket.

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
    phase: PointerPhase,
    #[schemars(description = "Canvas-local horizontal position in pixels")]
    x: f32,
    #[schemars(description = "Canvas-local vertical position in pixels")]
    y: f32,
    #[serde(default)]
    button: Option<PointerButton>,
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
    target: UiTarget,
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
    async fn get_state(&self) -> CallToolResult {
        response_result(AutomationRequest::State).await
    }

    #[tool(description = "Run an enabled Strek editor command by ID without using the keyboard")]
    async fn perform_action(
        &self,
        Parameters(ActionParams { id }): Parameters<ActionParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::Action { id }).await
    }

    #[tool(
        description = "Send a canvas-local pointer event without moving the system cursor. Use down, move, then up to draw or drag."
    )]
    async fn pointer(&self, Parameters(params): Parameters<PointerParams>) -> CallToolResult {
        response_result(AutomationRequest::Pointer {
            phase: params.phase,
            x: params.x,
            y: params.y,
            button: params.button.unwrap_or_default(),
            modifiers: AutomationModifiers {
                shift: params.shift,
                control: params.control,
                alt: params.alt,
                command: params.command,
            },
        })
        .await
    }

    #[tool(description = "Insert text into the active Strek text editing session")]
    async fn insert_text(
        &self,
        Parameters(TextParams { text }): Parameters<TextParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::Text { text }).await
    }

    #[tool(description = "Show or hide Strek panels and overlays without clicking the UI")]
    async fn set_ui(
        &self,
        Parameters(UiParams { target, visible }): Parameters<UiParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::SetUi { target, visible }).await
    }

    #[tool(
        description = "Activate Strek and capture its complete window as a PNG image without moving the system cursor"
    )]
    async fn screenshot(&self) -> CallToolResult {
        match tokio::task::spawn_blocking(automation::capture_screenshot).await {
            Ok(Ok(png)) => {
                let data = base64::engine::general_purpose::STANDARD.encode(png);
                CallToolResult::success(vec![ContentBlock::image(data, "image/png")])
            }
            Ok(Err(error)) => tool_error(error),
            Err(error) => tool_error(format!("screenshot task failed: {error}")),
        }
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

async fn response_result(request: AutomationRequest) -> CallToolResult {
    match tokio::task::spawn_blocking(move || automation::request(request)).await {
        Ok(Ok(response)) => match serde_json::to_value(&response) {
            Ok(value) if response.ok => CallToolResult::structured(value),
            Ok(value) => CallToolResult::structured_error(value),
            Err(error) => tool_error(format!("could not serialize Strek response: {error}")),
        },
        Ok(Err(error)) => tool_error(error),
        Err(error) => tool_error(format!("automation task failed: {error}")),
    }
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
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
