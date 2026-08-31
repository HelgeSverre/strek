# Strek automation

Strek exposes the same editor commands and canvas input used by its native UI
through three cursor-free interfaces:

- `strek automate` for shell scripts
- the bundled `AUTOMATION.applescript` helper
- `strek mcp` for Model Context Protocol clients

All three talk to the running desktop application through a local IPC endpoint.
On Linux and macOS this is a Unix-domain socket. On Windows it is a loopback
TCP endpoint. They do not move the system cursor. The transport protocol itself
is an implementation detail; scripts should use one of the supported interfaces
above.

This document describes the currently implemented protocol. The planned
self-describing, revision-safe protocol is specified in
[`agent-control-plane.md`](agent-control-plane.md); fields or tools described
there must not be treated as implemented until their vertical delivery phase is
complete.

## Requirements

Start the Strek desktop application before issuing automation commands. The CLI
and MCP server are clients of that running process; `strek mcp` does not launch
the GUI.

For unattended automation, launch the desktop window without activating it:

```sh
strek --background
```

Semantic requests explicitly refresh inactive windows, so scripted edits remain
visible without bringing Strek to the foreground. Window screenshots are
currently supported on macOS, where background mode parks a two-pixel edge of
the window on the rightmost display so the system continues rendering its
otherwise off-screen surface.

By default, Strek creates a per-user local endpoint. On Linux and macOS it is a
socket below `XDG_RUNTIME_DIR`, when it is an absolute path, or the system
temporary directory:

```text
<runtime-directory>/strek-<user-id>/automation.sock
```

On Linux and macOS, the directory is restricted to its owner with mode `0700`
and the socket with mode `0600`. On Windows, the default endpoint is a loopback
address in the dynamic port range, derived from the current user. Set
`STREK_AUTOMATION_SOCKET` in both the app and client environments to use another
endpoint. The value is a filesystem path on Linux/macOS and a loopback
`IP:port` address on Windows. For example, when running isolated development
instances:

```sh
STREK_AUTOMATION_SOCKET=/tmp/strek-dev.sock cargo run -p strek
STREK_AUTOMATION_SOCKET=/tmp/strek-dev.sock target/debug/strek automate state
```

```powershell
$env:STREK_AUTOMATION_SOCKET = "127.0.0.1:49153"
cargo run -p strek
target\debug\strek.exe automate state
```

Set `STREK_CONFIG_DIR` to an isolated directory when an unattended run should
not read or update the normal workspace preferences, recent files, or custom
keybindings. The README showcase recorder uses this to produce deterministic
output without changing the user's configuration.

## Recommended workflow

1. Call `capabilities`/`get_capabilities` when discovering a running version,
   then `state`/`get_state`. Prefer `query-document`/`query_document` for
   targeted inspection; use `document`/`get_document` when the task genuinely
   needs the complete tree and document libraries.
2. Use the returned opaque layer IDs with `select`, `layer`, `color`, and
   `property` for semantic edits.
3. Use enabled command IDs from `actions` for editor commands.
4. Read `canvas` and use pointer input only for spatial interactions such as
   drawing, dragging, or editing path anchors.
5. Reinspect the document, export an artifact, or take a screenshot.

Protocol version 2 reports session-local document, workspace, and artwork
revisions. Guarded CLI and MCP calls reject stale state and deduplicate terminal
responses within the running process's bounded 256-response cache. Responses to
successful non-observation requests include a compact revision/effect receipt.

Atomic semantic batches, delta queries, and entity-level receipt effects are not
implemented yet. Entity queries are bounded to 512 layers and make style and
computed geometry opt-in. Evicted sequences fail explicitly instead of being re-executed.
These limitations are explicit so agents do not infer stronger concurrency
guarantees from a successful JSON response.

Successful state-changing requests include the settled post-action state, so a
separate state request is often unnecessary.

## Command-line interface

Running `strek automate` without a subcommand is equivalent to
`strek automate state`.

| Command | Purpose |
| --- | --- |
| `strek automate state` | Inspect the editor, window, canvas, selection, and available actions. |
| `strek automate capabilities` | Discover operation families, parameters, effects, costs, authority classes, limits, and current availability. |
| `strek automate document` | Inspect the document tree, session-stable layer IDs, styles, world bounds, and transforms. |
| `strek automate query-document [--descendants] [--style] [--geometry] [layer-id ...]` | Inspect named layers (or the root) with bounded descendants and opt-in expensive projections. |
| `strek automate new [--discard]` | Create a document, optionally discarding unsaved changes. |
| `strek automate open <absolute-path> [--discard]` | Open a native document or import a supported SVG. |
| `strek automate save <absolute-path>` | Save the current document in Strek's native format. |
| `strek automate export <svg\|svg-outlined\|png\|jpeg\|webp> [absolute-path]` | Export to a path, or omit it to return a bounded inline artifact. |
| `strek automate activate` | Bring the Strek window to the front. |
| `strek automate action <command-id>` | Run an enabled editor command from `state.actions`. |
| `strek automate select <replace\|add\|remove\|toggle> [layer-id ...]` | Change selection using IDs from `document`. |
| `strek automate color <fill\|stroke\|frame-background> <hex\|none>` | Set or remove selection paint directly. |
| `strek automate property <target> <value>` | Set a semantic numeric property directly. |
| `strek automate layer <layer-id> [--name value] [--visible bool] [--locked bool]` | Update layer metadata directly. |
| `strek automate pointer <down\|move\|up> <x> <y> [left\|middle\|right]` | Send a canvas-local pointer event; the button defaults to `left`. |
| `strek automate text <text>` | Insert text into the active text-editing session. |
| `strek automate ui <target> <show\|hide>` | Show or hide a supported panel or overlay. |
| `strek automate screenshot <output.png>` | On macOS, save a complete window screenshot without activating Strek. |

Guard a normal CLI command with the session and revisions returned by `state`:

```sh
strek automate guarded \
  --request-id agent-step-7 \
  --request-sequence 4 \
  --session-id SESSION_ID \
  --document-revision 12 \
  --workspace-revision 9 \
  -- action edit.undo
```

`request_id` requires `session_id` and `request_sequence`; it is limited to 128
ASCII letters, digits, periods, underscores, colons, and hyphens. Use exactly
`state.retry_window.next_sequence` for a new guarded call. The server returns
the originally accepted response when that session/sequence is retried with the
same request ID. A retained retry uses no additional edit or sequence number.
Once a sequence is older than `retained_from`, it fails with `request_expired`
instead of executing again. A session changes when the process restarts or the
document is replaced.

The CLI accepts these UI targets: `main-menu`, `command-palette`,
`layers-panel`, `design-panel`, `fill-color-picker`, `stroke-color-picker`,
`frame-background-color-picker`, `color-library`, and `precision-controls`.
The color-picker targets operate on the current selection. Underscores are
accepted in place of hyphens.

All commands except `screenshot` print an `AutomationResponse` as formatted
JSON and exit nonzero when the request fails:

| Field | Meaning |
| --- | --- |
| `ok` | Whether Strek accepted and completed the request. |
| `protocol_version` | Automation response contract version; currently `2`. |
| `message` | A human-readable result or error. |
| `error` | Stable machine error code and message for rejected requests. |
| `receipt` | Operation, request ID, before/after revisions, and coarse document/workspace effect for a successful mutation. |
| `state` | Current automation state, when available. |
| `document` | Document-tree inspection result for `document`. |
| `document_query` | Bounded entity projection returned by `query-document`; `style` and `geometry` are absent unless requested. |
| `artifact` | Inline export metadata and base64 payload when export has no path. |

`screenshot` prints the output path on success.

### State

`state` is the authoritative description of the automation-visible editor:

| Field | Meaning |
| --- | --- |
| `process_id` | Process ID of the running desktop application. |
| `protocol_version` | Automation protocol version; currently `2`. |
| `revisions` | Session ID plus monotonic document, workspace, and artwork revisions. |
| `retry_window` | Next guarded request sequence, oldest retained response sequence, and bounded cache capacity. |
| `document` | Current document name. |
| `dirty` | Whether the document has unsaved changes. |
| `tool` | Active tool. |
| `interaction` | Active editor interaction or `idle`. |
| `selection_count` | Number of selected layers. |
| `selected_layers` | Names of selected layers. |
| `zoom` | Current canvas zoom. |
| `pan` | Canvas pan as `{x, y}`. |
| `window` | Screen-space `{x, y, width, height}` for the app window. |
| `canvas` | Window-local canvas `{x, y, width, height}`, or `null` before layout is ready. |
| `layers_panel_visible` | Layers panel visibility. |
| `design_panel_visible` | Design panel visibility. |
| `main_menu_open` | Whether the main menu is open. |
| `command_palette_open` | Whether the command palette is open. |
| `color_picker_open` | Whether a fill or stroke color picker is open. |
| `numeric_property_scrub_active` | Whether a numeric property is being scrubbed. |
| `actions` | Editor command objects containing `id`, `label`, and `enabled`. |

Tool values are `select`, `frame`, `rectangle`, `ellipse`, `line`, `pen`,
`text`, and `vector_edit`. Interaction values are `idle`, `moving`, `resizing`,
`rotating`, `marquee`, `creating_shape`, `creating_frame`, `pen`,
`creating_text`, `text_editing`, `vector_editing`, and `panning`.

Only editor commands appear in `actions`; use the dedicated file and semantic
commands for the other operations. Always use IDs reported by the running
version of Strek. Layer IDs are opaque and stable only for the current loaded
document session; inspect the tree again after `new` or `open`. Typical command
IDs include `tool.rectangle` and `edit.undo`.

Numeric property targets are `world-x`, `world-y`, `opacity`, `stroke-width`,
`text-size`, `layout-spacing`, and `layout-padding`. Opacity uses the model range
from `0` to `1`; the other values use document units. Explicit file paths must
be absolute so the desktop app and its client cannot interpret different
working directories. Inline artifacts are limited to 2 MiB of encoded artifact
bytes; use a destination path for larger exports.

### Capabilities

`capabilities` describes the operation families implemented by the running
binary. Each entry contains a stable `id`, summary, parameter descriptions,
effect, cost and authority classes, whether revision guards apply, exact limits
derived from runtime constants where applicable, and current `enabled` state.
A disabled entry includes `disabled_reason`.

The manifest is bounded metadata and does not replace request validation. Some
families contain several actions with different parameter requirements, so the
handler remains authoritative for a concrete call. Command-specific enablement
continues to live in `state.actions`.

### Examples

Create a rectangle:

```sh
strek automate action tool.rectangle
strek automate pointer down 120 100
strek automate pointer move 360 260
strek automate pointer up 360 260
```

Insert text after entering a text-editing session through the Text tool and a
canvas pointer sequence:

```sh
strek automate text "Hello, Strek"
```

Show a panel and capture the result on macOS:

```sh
strek automate ui layers-panel show
strek automate screenshot /tmp/strek.png
```

Inspect and recolor an existing layer without pointer input:

```sh
strek automate document
strek automate select replace node-0000000100000001
strek automate color fill '#ff3366'
strek automate property opacity 0.8
strek automate export svg /tmp/strek-export.svg
```

The same export flow in Windows PowerShell uses a native Windows path:

```powershell
$exportPath = Join-Path $env:TEMP "strek-export.svg"
strek automate export svg $exportPath
```

Use the ID from the current `document` response; the example ID is only
illustrative. Omitting the export path returns an `artifact` object in JSON.

Pointer coordinates are pixels relative to the canvas, not the screen or
window. Use `canvas.width` and `canvas.height` from state to choose an in-bounds
pointer-down position. The CLI supports pointer buttons but not modifier keys;
use MCP when an event needs modifiers.

Text insertion is accepted only while `interaction` is `text_editing` and no
menu, command palette, context menu, or inline numeric editor is intercepting
input.

## AppleScript (macOS)

The packaged application installs its helper here:

```text
/Applications/Strek.app/Contents/Resources/AUTOMATION.applescript
```

Run any CLI automation subcommand through the helper's `run` handler:

```sh
osascript "/Applications/Strek.app/Contents/Resources/AUTOMATION.applescript" state
osascript "/Applications/Strek.app/Contents/Resources/AUTOMATION.applescript" action tool.rectangle
osascript "/Applications/Strek.app/Contents/Resources/AUTOMATION.applescript" pointer down 120 100
osascript "/Applications/Strek.app/Contents/Resources/AUTOMATION.applescript" screenshot /tmp/strek.png
```

The helper also exposes named handlers for use inside AppleScript:

| Handler | Equivalent command |
| --- | --- |
| `strekState()` | `strek automate state` |
| `strekDocument()` | `strek automate document` |
| `strekActivate()` | `strek automate activate` |
| `strekNew(discardChanges)` | `strek automate new [--discard]` |
| `strekOpen(path, discardChanges)` | `strek automate open <path> [--discard]` |
| `strekSave(path)` | `strek automate save <path>` |
| `strekExport(format, path)` | `strek automate export <format> [path]`; pass `missing value` for an inline artifact. |
| `strekAction(commandId)` | `strek automate action <command-id>` |
| `strekSelect(mode, layerIds)` | `strek automate select <mode> <layer-id ...>` |
| `strekColor(target, color)` | `strek automate color <target> <hex\|none>`; pass `missing value` to remove paint. |
| `strekProperty(target, value)` | `strek automate property <target> <value>` |
| `strekLayer(id, name, visible, locked)` | `strek automate layer ...`; pass `missing value` for unchanged fields. |
| `strekPointer(phase, x, y)` | `strek automate pointer <phase> <x> <y>` |
| `strekText(textValue)` | `strek automate text <text>` |
| `strekUi(targetName, isVisible)` | `strek automate ui <target> <show\|hide>` |
| `strekScreenshot(outputPath)` | `strek automate screenshot <output.png>` |

Load the helper once and call its handlers:

```applescript
set strekAutomation to load script POSIX file "/Applications/Strek.app/Contents/Resources/AUTOMATION.applescript"

set stateJSON to strekAutomation's strekState()
strekAutomation's strekAction("tool.rectangle")
strekAutomation's strekPointer("down", 120, 100)
strekAutomation's strekPointer("move", 360, 260)
strekAutomation's strekPointer("up", 360, 260)
set screenshotPath to strekAutomation's strekScreenshot("/tmp/strek.png")
```

All handlers except screenshot return the CLI's JSON as text. The screenshot
handler returns the written path.

The helper resolves the `strek` executable in this order:

1. Its `strekBinary` property, when set.
2. `STREK_AUTOMATION_BINARY`.
3. `strek` on `/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin`.
4. The executable inside the installed Strek application.

For a development build, set the environment variable before invoking
`osascript`:

```sh
STREK_AUTOMATION_BINARY="$PWD/target/debug/strek" \
  osascript "apps/gpui/assets/AUTOMATION.applescript" state
```

`strekPointer` uses the left button and does not expose modifiers. Use the
generic `run` handler from `osascript` for another button, or use MCP for
modifier keys:

```sh
osascript "apps/gpui/assets/AUTOMATION.applescript" pointer down 120 100 right
```

## MCP server

Strek includes a local stdio MCP server. Configure an MCP client to launch the
installed binary with `mcp` as its only argument:

```json
{
  "mcpServers": {
    "strek": {
      "command": "/opt/homebrew/bin/strek",
      "args": ["mcp"]
    }
  }
}
```

Replace the command with the result of `command -v strek`; Intel Homebrew
commonly installs it at `/usr/local/bin/strek`. The MCP client owns the stdio
server subprocess and negotiates the MCP lifecycle. The server writes protocol
messages on standard input/output and does not open a network port.

On Windows, set `command` to the value printed by
`(Get-Command strek).Source`, escaping each backslash when writing JSON, for
example `C:\\Tools\\Strek\\strek.exe`.

The Strek desktop application must still be running. MCP tool calls are bridged
from the stdio server to that application's local automation endpoint.

### Tools

| Tool | Parameters | Result |
| --- | --- | --- |
| `get_state` | None | Structured automation response containing current state. |
| `get_capabilities` | None | Runtime operation manifest with current availability and limits. |
| `guarded_call` | request ID/sequence, session ID, optional revision guards, and an automation request object | Execute any semantic request with stale-write rejection and bounded retry safety. |
| `get_document` | None | Document tree plus grid, guides, Color Library groups, and Saved Colors with stable IDs. |
| `query_document` | optional IDs and `include_descendants`, `include_style`, `include_geometry` | Bounded projection of named layers; empty IDs inspect only the root. |
| `new_document` | optional `discard_changes` | Creates a new document. |
| `open_document` | absolute `path`, optional `discard_changes` | Opens native JSON or imports supported SVG. |
| `save_document` | absolute `path` | Saves native JSON. |
| `export_artwork` | `format`, optional absolute `path` | Writes an export or directly returns SVG text/raster image content. |
| `perform_action` | `id` | Runs an enabled editor command ID from `get_state`. |
| `select_layers` | `ids`, optional `mode` | Replaces, adds, removes, or toggles selection by layer ID. |
| `set_color` | `target`, optional `color` | Sets fill/stroke HEX color; omit color to remove paint. |
| `set_numeric_property` | `target`, `value` | Sets an absolute semantic numeric property. |
| `set_precision` | optional ruler/grid/guide visibility, locking, snapping targets, tolerance, and grid fields | Updates workspace precision controls and document grid settings. |
| `guide` | `action`, optional `id`, `axis`, `position` | Adds, moves, removes, or clears persistent guides. |
| `color_group` | `action`, optional `id`, `name` | Adds, renames, or removes a Color Library group. |
| `saved_color` | `action`, optional `id`, `group_id`, `name`, `color`, `target` | Adds, updates, removes, or applies a Saved Color. |
| `set_layer_properties` | `ids`, optional `name`, `visible`, `locked` | Updates layer metadata; name requires exactly one ID. |
| `pointer` | `phase`, `x`, `y`, optional `button`, `shift`, `control`, `alt`, `command` | Sends a canvas-local pointer event and returns current state. |
| `insert_text` | `text` | Inserts text into the active text-editing session and returns current state. |
| `set_ui` | `target`, `visible` | Shows or hides a supported UI target and returns current state. |
| `activate` | None | Brings Strek to the front. |
| `screenshot` | None | Returns the complete window as `image/png` without activating Strek. |

MCP pointer phases are `down`, `move`, and `up`; buttons are `left`, `middle`,
and `right`. All modifier flags default to `false`. MCP UI targets use
`main_menu`, `command_palette`, `layers_panel`, `design_panel`,
`fill_color_picker`, `stroke_color_picker`, `color_library`, and
`precision_controls`.

A typical semantic editing sequence is:

```text
get_state {}
get_document {}
select_layers {"mode":"replace","ids":["node-0000000100000001"]}
set_color {"target":"fill","color":"#ff3366"}
set_numeric_property {"target":"opacity","value":0.8}
export_artwork {"format":"svg"}
```

Endpoint-backed tools return structured errors when Strek rejects a request.
Transport failures and screenshot failures are MCP tool errors.

## Behavior and troubleshooting

- `could not connect to Strek` means the desktop application is not running or
  the client and app use different `STREK_AUTOMATION_SOCKET` values.
- A disabled action is rejected. Refresh state and choose an action whose
  `enabled` field is `true`.
- Pointer-down events must use finite, canvas-local coordinates inside the
  current canvas bounds.
- Canvas input is rejected while a menu, command palette, context menu, inline
  numeric editor, or property scrub is active. Close it before sending pointer
  events.
- Screenshots do not activate Strek. On macOS, the first capture may request
  Screen Recording permission; denial is returned as an error.
- Window screenshots are currently supported only on macOS.
- The default endpoint permits one running Strek automation server per user. Use
  distinct `STREK_AUTOMATION_SOCKET` values for concurrent development
  instances; use paths on Linux/macOS and loopback `host:port` addresses on
  Windows.
