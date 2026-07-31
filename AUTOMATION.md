# Strek automation

Strek exposes the same editor commands and canvas input used by its native UI
through three cursor-free interfaces:

- `strek automate` for shell scripts
- the bundled `Strek Automation.applescript` helper
- `strek mcp` for Model Context Protocol clients

All three talk to the running desktop application through a local Unix-domain
socket. They do not move the system cursor. The socket protocol itself is an
implementation detail; scripts should use one of the supported interfaces
above.

## Requirements

Start the Strek desktop application before issuing automation commands. The CLI
and MCP server are clients of that running process; `strek mcp` does not launch
the GUI.

By default, Strek creates a per-user socket below `XDG_RUNTIME_DIR`, when it is
an absolute path, or the system temporary directory:

```text
<runtime-directory>/strek-<user-id>/automation.sock
```

The directory is restricted to its owner with mode `0700` and the socket with
mode `0600`. Set `STREK_AUTOMATION_SOCKET` in both the app and client
environments to use another path, for example when running isolated development
instances:

```sh
STREK_AUTOMATION_SOCKET=/tmp/strek-dev.sock cargo run -p strek
STREK_AUTOMATION_SOCKET=/tmp/strek-dev.sock target/debug/strek automate state
```

## Recommended workflow

1. Call `state` or `get_state`.
2. Read `canvas` before choosing canvas-local coordinates.
3. Select an enabled command ID from `actions` instead of hard-coding labels.
4. Send an action or a `down`, `move`, `up` pointer sequence.
5. Inspect the returned state or take a screenshot.

Successful state-changing requests include the settled post-action state, so a
separate state request is often unnecessary.

## Command-line interface

Running `strek automate` without a subcommand is equivalent to
`strek automate state`.

| Command | Purpose |
| --- | --- |
| `strek automate state` | Inspect the editor, window, canvas, selection, and available actions. |
| `strek automate activate` | Bring the Strek window to the front. |
| `strek automate action <command-id>` | Run an enabled editor command from `state.actions`. |
| `strek automate pointer <down\|move\|up> <x> <y> [left\|middle\|right]` | Send a canvas-local pointer event; the button defaults to `left`. |
| `strek automate text <text>` | Insert text into the active text-editing session. |
| `strek automate ui <target> <show\|hide>` | Show or hide a supported panel or overlay. |
| `strek automate screenshot <output.png>` | Activate Strek and save a complete window screenshot. |

The CLI accepts these UI targets: `main-menu`, `command-palette`,
`layers-panel`, and `design-panel`. Underscores are accepted in place of
hyphens.

All commands except `screenshot` print an `AutomationResponse` as formatted
JSON and exit nonzero when the request fails:

| Field | Meaning |
| --- | --- |
| `ok` | Whether Strek accepted and completed the request. |
| `message` | A human-readable result or error. |
| `state` | Current automation state, when available. |

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
| `numeric_property_scrub_active` | Whether a numeric property is being scrubbed. |
| `actions` | Editor command objects containing `id`, `label`, and `enabled`. |

Tool values are `select`, `frame`, `rectangle`, `ellipse`, `line`, `pen`,
`text`, and `vector_edit`. Interaction values are `idle`, `moving`, `resizing`,
`rotating`, `marquee`, `creating_shape`, `creating_frame`, `pen`,
`creating_text`, `text_editing`, `vector_editing`, and `panning`.

Only editor commands appear in `actions`; application-level operations with
dedicated automation commands are intentionally excluded. Always use the IDs
reported by the running version of Strek. Typical IDs include
`tool.rectangle` and `edit.undo`.

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

Pointer coordinates are pixels relative to the canvas, not the screen or
window. Use `canvas.width` and `canvas.height` from state to choose an in-bounds
pointer-down position. The CLI supports pointer buttons but not modifier keys;
use MCP when an event needs modifiers.

Text insertion is accepted only while `interaction` is `text_editing` and no
menu, command palette, context menu, or inline numeric editor is intercepting
input.

## AppleScript

The packaged application installs its helper here:

```text
/Applications/Strek.app/Contents/Resources/Strek Automation.applescript
```

Run any CLI automation subcommand through the helper's `run` handler:

```sh
osascript "/Applications/Strek.app/Contents/Resources/Strek Automation.applescript" state
osascript "/Applications/Strek.app/Contents/Resources/Strek Automation.applescript" action tool.rectangle
osascript "/Applications/Strek.app/Contents/Resources/Strek Automation.applescript" pointer down 120 100
osascript "/Applications/Strek.app/Contents/Resources/Strek Automation.applescript" screenshot /tmp/strek.png
```

The helper also exposes named handlers for use inside AppleScript:

| Handler | Equivalent command |
| --- | --- |
| `strekState()` | `strek automate state` |
| `strekAction(commandId)` | `strek automate action <command-id>` |
| `strekPointer(phase, x, y)` | `strek automate pointer <phase> <x> <y>` |
| `strekText(textValue)` | `strek automate text <text>` |
| `strekUi(targetName, isVisible)` | `strek automate ui <target> <show\|hide>` |
| `strekScreenshot(outputPath)` | `strek automate screenshot <output.png>` |

Load the helper once and call its handlers:

```applescript
set strekAutomation to load script POSIX file "/Applications/Strek.app/Contents/Resources/Strek Automation.applescript"

set stateJSON to strekAutomation's strekState()
strekAutomation's strekAction("tool.rectangle")
strekAutomation's strekPointer("down", 120, 100)
strekAutomation's strekPointer("move", 360, 260)
strekAutomation's strekPointer("up", 360, 260)
set screenshotPath to strekAutomation's strekScreenshot("/tmp/strek.png")
```

The state, action, pointer, text, and UI handlers return the CLI's JSON as
text. The screenshot handler returns the written path.

The helper resolves the `strek` executable in this order:

1. Its `strekBinary` property, when set.
2. `STREK_AUTOMATION_BINARY`.
3. `strek` on `/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin`.
4. The executable inside the installed Strek application.

For a development build, set the environment variable before invoking
`osascript`:

```sh
STREK_AUTOMATION_BINARY="$PWD/target/debug/strek" \
  osascript "apps/gpui/assets/Strek Automation.applescript" state
```

`strekPointer` uses the left button and does not expose modifiers. Use the
generic `run` handler from `osascript` for another button, or use MCP for
modifier keys:

```sh
osascript "apps/gpui/assets/Strek Automation.applescript" pointer down 120 100 right
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
from the stdio server to that application's local automation socket.

### Tools

| Tool | Parameters | Result |
| --- | --- | --- |
| `get_state` | None | Structured automation response containing current state. |
| `perform_action` | `id` | Runs an enabled editor command ID from `get_state`. |
| `pointer` | `phase`, `x`, `y`, optional `button`, `shift`, `control`, `alt`, `command` | Sends a canvas-local pointer event and returns current state. |
| `insert_text` | `text` | Inserts text into the active text-editing session and returns current state. |
| `set_ui` | `target`, `visible` | Shows or hides a supported UI target and returns current state. |
| `screenshot` | None | Activates Strek and returns the complete window as `image/png`. |

MCP pointer phases are `down`, `move`, and `up`; buttons are `left`, `middle`,
and `right`. All modifier flags default to `false`. MCP UI targets use
`main_menu`, `command_palette`, `layers_panel`, and `design_panel`.

A typical tool sequence is:

```text
get_state {}
perform_action {"id":"tool.rectangle"}
pointer {"phase":"down","x":120,"y":100}
pointer {"phase":"move","x":360,"y":260}
pointer {"phase":"up","x":360,"y":260}
screenshot {}
```

Socket-backed tools return structured errors when Strek rejects a request.
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
- The default socket permits one running Strek automation server per user. Use
  distinct `STREK_AUTOMATION_SOCKET` paths for concurrent development
  instances.
