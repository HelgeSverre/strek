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

## Requirements

Start the Strek desktop application before issuing automation commands. The CLI
and MCP server are clients of that running process; `strek mcp` does not launch
the GUI.

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

## Recommended workflow

1. Call `state`/`get_state`, then `document`/`get_document` when the task
   concerns existing artwork.
2. Use the returned opaque layer IDs with `select`, `layer`, `color`, and
   `property` for semantic edits.
3. Use enabled command IDs from `actions` for editor commands.
4. Read `canvas` and use pointer input only for spatial interactions such as
   drawing, dragging, or editing path anchors.
5. Reinspect the document, export an artifact, or take a screenshot.

Successful state-changing requests include the settled post-action state, so a
separate state request is often unnecessary.

## Command-line interface

Running `strek automate` without a subcommand is equivalent to
`strek automate state`.

| Command | Purpose |
| --- | --- |
| `strek automate state` | Inspect the editor, window, canvas, selection, and available actions. |
| `strek automate document` | Inspect the document tree, session-stable layer IDs, styles, world bounds, and transforms. |
| `strek automate new [--discard]` | Create a document, optionally discarding unsaved changes. |
| `strek automate open <absolute-path> [--discard]` | Open a native document or import a supported SVG. |
| `strek automate save <absolute-path>` | Save the current document in Strek's native format. |
| `strek automate export <svg\|svg-outlined\|png\|jpeg\|webp> [absolute-path]` | Export to a path, or omit it to return a bounded inline artifact. |
| `strek automate activate` | Bring the Strek window to the front. |
| `strek automate action <command-id>` | Run an enabled editor command from `state.actions`. |
| `strek automate select <replace\|add\|remove\|toggle> [layer-id ...]` | Change selection using IDs from `document`. |
| `strek automate color <fill\|stroke> <hex\|none>` | Set or remove selection paint directly. |
| `strek automate property <target> <value>` | Set a semantic numeric property directly. |
| `strek automate layer <layer-id> [--name value] [--visible bool] [--locked bool]` | Update layer metadata directly. |
| `strek automate pointer <down\|move\|up> <x> <y> [left\|middle\|right]` | Send a canvas-local pointer event; the button defaults to `left`. |
| `strek automate text <text>` | Insert text into the active text-editing session. |
| `strek automate ui <target> <show\|hide>` | Show or hide a supported panel or overlay. |
| `strek automate screenshot <output.png>` | Activate Strek and save a complete window screenshot. |

The CLI accepts these UI targets: `main-menu`, `command-palette`,
`layers-panel`, `design-panel`, `fill-color-picker`, and
`stroke-color-picker`. The color-picker targets operate on the current
selection. Underscores are accepted in place of hyphens.

All commands except `screenshot` print an `AutomationResponse` as formatted
JSON and exit nonzero when the request fails:

| Field | Meaning |
| --- | --- |
| `ok` | Whether Strek accepted and completed the request. |
| `message` | A human-readable result or error. |
| `state` | Current automation state, when available. |
| `document` | Document-tree inspection result for `document`. |
| `artifact` | Inline export metadata and base64 payload when export has no path. |

`screenshot` prints the output path on success.

### State

`state` is the authoritative description of the automation-visible editor:

| Field | Meaning |
| --- | --- |
| `process_id` | Process ID of the running desktop application. |
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

Show a panel and capture the result:

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

The Strek desktop application must still be running. MCP tool calls are bridged
from the stdio server to that application's local automation endpoint.

### Tools

| Tool | Parameters | Result |
| --- | --- | --- |
| `get_state` | None | Structured automation response containing current state. |
| `get_document` | None | Document tree with session-stable layer IDs, styles, world bounds, and transforms. |
| `new_document` | optional `discard_changes` | Creates a new document. |
| `open_document` | absolute `path`, optional `discard_changes` | Opens native JSON or imports supported SVG. |
| `save_document` | absolute `path` | Saves native JSON. |
| `export_artwork` | `format`, optional absolute `path` | Writes an export or directly returns SVG text/raster image content. |
| `perform_action` | `id` | Runs an enabled editor command ID from `get_state`. |
| `select_layers` | `ids`, optional `mode` | Replaces, adds, removes, or toggles selection by layer ID. |
| `set_color` | `target`, optional `color` | Sets fill/stroke HEX color; omit color to remove paint. |
| `set_numeric_property` | `target`, `value` | Sets an absolute semantic numeric property. |
| `set_layer_properties` | `ids`, optional `name`, `visible`, `locked` | Updates layer metadata; name requires exactly one ID. |
| `pointer` | `phase`, `x`, `y`, optional `button`, `shift`, `control`, `alt`, `command` | Sends a canvas-local pointer event and returns current state. |
| `insert_text` | `text` | Inserts text into the active text-editing session and returns current state. |
| `set_ui` | `target`, `visible` | Shows or hides a supported UI target and returns current state. |
| `activate` | None | Brings Strek to the front. |
| `screenshot` | None | Activates Strek and returns the complete window as `image/png`. |

MCP pointer phases are `down`, `move`, and `up`; buttons are `left`, `middle`,
and `right`. All modifier flags default to `false`. MCP UI targets use
`main_menu`, `command_palette`, `layers_panel`, `design_panel`,
`fill_color_picker`, and `stroke_color_picker`.

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
- Screenshots activate the Strek window first. On macOS, the first capture may
  request Screen Recording permission; denial is returned as an error.
- Window screenshots are currently supported only on macOS.
- The default endpoint permits one running Strek automation server per user. Use
  distinct `STREK_AUTOMATION_SOCKET` values for concurrent development
  instances; use paths on Linux/macOS and loopback `host:port` addresses on
  Windows.
