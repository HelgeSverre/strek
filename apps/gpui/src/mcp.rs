//! Stdio MCP server backed by the running Strek automation endpoint.

use base64::Engine;
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo},
    schemars::JsonSchema,
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use serde::Deserialize;

use crate::automation::{
    self, ArtifactFormat, AutomationModifiers, AutomationRequest, ColorTarget, NumericProperty,
    PointerButton, PointerPhase, SelectionMode, UiTarget,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct SelectParams {
    #[schemars(description = "Opaque layer IDs returned by get_document")]
    ids: Vec<String>,
    #[serde(default)]
    mode: SelectionMode,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ColorParams {
    target: ColorTarget,
    #[serde(default)]
    #[schemars(description = "HEX color, including optional alpha; omit to remove the paint")]
    color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NumericPropertyParams {
    target: NumericProperty,
    value: f32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LayerPropertiesParams {
    #[schemars(description = "Opaque layer IDs returned by get_document")]
    ids: Vec<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    visible: Option<bool>,
    #[serde(default)]
    locked: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NewDocumentParams {
    #[serde(default)]
    #[schemars(description = "Discard unsaved changes instead of rejecting the request")]
    discard_changes: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OpenDocumentParams {
    #[schemars(description = "Native document or supported SVG path")]
    path: String,
    #[serde(default)]
    #[schemars(description = "Discard unsaved changes instead of rejecting the request")]
    discard_changes: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SaveDocumentParams {
    #[schemars(description = "Native document destination path")]
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExportParams {
    format: ArtifactFormat,
    #[serde(default)]
    #[schemars(description = "Optional destination path; omit to return the artifact directly")]
    path: Option<String>,
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

    #[tool(
        description = "Inspect the current document tree with stable layer IDs, styles, bounds, and transforms"
    )]
    async fn get_document(&self) -> CallToolResult {
        response_result(AutomationRequest::Document).await
    }

    #[tool(description = "Create a new Strek document")]
    async fn new_document(
        &self,
        Parameters(NewDocumentParams { discard_changes }): Parameters<NewDocumentParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::NewDocument { discard_changes }).await
    }

    #[tool(description = "Open a native Strek document or import a supported SVG from a path")]
    async fn open_document(
        &self,
        Parameters(OpenDocumentParams {
            path,
            discard_changes,
        }): Parameters<OpenDocumentParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::OpenDocument {
            path,
            discard_changes,
        })
        .await
    }

    #[tool(description = "Save the current document to an explicit native path")]
    async fn save_document(
        &self,
        Parameters(SaveDocumentParams { path }): Parameters<SaveDocumentParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::SaveDocument { path }).await
    }

    #[tool(
        description = "Export visible artwork to a path, or omit path to return a bounded SVG, PNG, JPEG, or WebP artifact directly"
    )]
    async fn export_artwork(
        &self,
        Parameters(ExportParams { format, path }): Parameters<ExportParams>,
    ) -> CallToolResult {
        artifact_result(AutomationRequest::Export { format, path }).await
    }

    #[tool(description = "Run an enabled Strek editor command by ID without using the keyboard")]
    async fn perform_action(
        &self,
        Parameters(ActionParams { id }): Parameters<ActionParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::Action { id }).await
    }

    #[tool(description = "Select layers by opaque IDs returned from get_document")]
    async fn select_layers(
        &self,
        Parameters(SelectParams { ids, mode }): Parameters<SelectParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::Select { ids, mode }).await
    }

    #[tool(description = "Set or remove the current selection's fill or stroke color")]
    async fn set_color(
        &self,
        Parameters(ColorParams { target, color }): Parameters<ColorParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::SetColor { target, color }).await
    }

    #[tool(description = "Set an absolute numeric property on the current selection")]
    async fn set_numeric_property(
        &self,
        Parameters(NumericPropertyParams { target, value }): Parameters<NumericPropertyParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::SetNumericProperty { target, value }).await
    }

    #[tool(description = "Rename a layer or set visibility and locking by stable layer ID")]
    async fn set_layer_properties(
        &self,
        Parameters(LayerPropertiesParams {
            ids,
            name,
            visible,
            locked,
        }): Parameters<LayerPropertiesParams>,
    ) -> CallToolResult {
        response_result(AutomationRequest::SetLayerProperties {
            ids,
            name,
            visible,
            locked,
        })
        .await
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

    #[tool(description = "Activate the running Strek application window")]
    async fn activate(&self) -> CallToolResult {
        response_result(AutomationRequest::Activate).await
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
                "Automate a running Strek window without moving the system cursor. Call get_state first and get_document when editing existing artwork. Prefer stable layer IDs and semantic selection, color, property, file, and export tools; use canvas-local pointer events for spatial drawing or direct manipulation.",
            )
    }
}

async fn response_result(request: AutomationRequest) -> CallToolResult {
    match automation_response(request).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(value) if response.ok => CallToolResult::structured(value),
            Ok(value) => CallToolResult::structured_error(value),
            Err(error) => tool_error(format!("could not serialize Strek response: {error}")),
        },
        Err(error) => tool_error(error),
    }
}

async fn artifact_result(request: AutomationRequest) -> CallToolResult {
    let response = match automation_response(request).await {
        Ok(response) => response,
        Err(error) => return tool_error(error),
    };
    if !response.ok || response.artifact.is_none() {
        return match serde_json::to_value(&response) {
            Ok(value) if response.ok => CallToolResult::structured(value),
            Ok(value) => CallToolResult::structured_error(value),
            Err(error) => tool_error(format!("could not serialize Strek response: {error}")),
        };
    }
    let artifact = response
        .artifact
        .expect("the artifact presence was checked above");
    if artifact.media_type == "image/svg+xml" {
        return match base64::engine::general_purpose::STANDARD
            .decode(artifact.base64)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
        {
            Some(svg) => CallToolResult::success(vec![ContentBlock::text(svg)]),
            None => tool_error("Strek returned invalid UTF-8 SVG data"),
        };
    }
    CallToolResult::success(vec![ContentBlock::image(
        artifact.base64,
        artifact.media_type,
    )])
}

async fn automation_response(
    request: AutomationRequest,
) -> Result<automation::AutomationResponse, String> {
    tokio::task::spawn_blocking(move || automation::request(request))
        .await
        .map_err(|error| format!("automation task failed: {error}"))?
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
