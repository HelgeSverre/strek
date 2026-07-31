# Vector Editor

An experimental native vector editor written in Rust. The primary frontend is
the GPUI desktop application in `apps/gpui`, with a workspace of reusable editor,
rendering, and export crates underneath it.

The project is under active development. Core editing workflows are usable, but
the file format and UI should not yet be treated as stable.

## What works

- Selection, marquee selection, grouping, ordering, alignment, and distribution
- Frames, rectangles, ellipses, lines, Bézier paths, and editable text
- Direct vector editing for anchors and handles, including join, split, reverse,
  and open/close operations
- Move, resize, rotate, snapping, pan, zoom, undo, and redo
- Layers and design panels, command palette, menus, and configurable shortcuts
- Native JSON document open/save with validated scene graphs and recent files
- SVG and PNG export

## Run the GPUI app

The current reference platform and CI environment is macOS. Install a stable
Rust toolchain and the Xcode Command Line Tools, then run:

```sh
cargo run -p vector-editor-gpui
```

With [just](https://github.com/casey/just) installed, the equivalent command is:

```sh
just run
```

The older wgpu application is retained as a legacy renderer shell and is not the
primary frontend:

```sh
just run-legacy
```

## Keyboard and pointer controls

`Primary` means `⌘` on macOS and `Ctrl` on other platforms.

| Action | Shortcut |
| --- | --- |
| Command palette | `Primary`+`Shift`+`P` |
| Select, Frame, Rectangle | `V`, `F`, `R` |
| Ellipse, Line, Pen, Text | `O`, `L`, `P`, `T` |
| Undo / redo | `Primary`+`Z` / `Primary`+`Shift`+`Z` |
| Copy / cut / paste | `Primary`+`C` / `X` / `V` |
| Duplicate | `Primary`+`D` |
| Group / ungroup | `Primary`+`G` / `Primary`+`Shift`+`G` |
| Delete | `Backspace` or `Delete` |
| Finish Pen, text, or vector editing | `Primary`+`Enter` |
| Join selected path endpoints | `Primary`+`J` |
| Zoom in / out | `Primary`+`+` / `Primary`+`-` |
| 100% / reset view | `Primary`+`1` / `Primary`+`0` |
| Fit artwork / fit selection | `Primary`+`Shift`+`1` / `Primary`+`2` |
| Nudge / large nudge | Arrow keys / `Shift`+arrow keys |
| Enter selection / select parent | `Enter` / `Shift`+`Enter` |
| Cancel or leave the current mode | `Escape` |
| Temporary hand tool | Hold `Space` and drag |
| Pan | Middle-button drag or trackpad/scroll wheel |

The in-app command palette and menus expose the complete command set. Choose
**Vector Editor → Keyboard Shortcuts…** to create and open the user
`keybindings.json`; changes take effect after restarting the app. A string
replaces a binding, an array assigns alternatives, and `null` disables it.

## Documents and export

Documents are stored as versioned JSON. Saving an extensionless filename adds
the native `.vector.json` extension. The loader validates the scene graph before
opening it, and writes use a same-directory temporary file before replacement.

Use the File menu to export visible artwork as SVG or PNG. Export bounds are
derived from the artwork rather than the current viewport.

## Development

Run the same checks used by CI:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

Useful `just` recipes:

```sh
just test    # library tests
just lint    # Clippy with warnings denied
just format  # format the workspace
```

The workspace is split by responsibility:

- `apps/gpui` — primary native application and GPUI interface
- `crates/editor_core` — document model, commands, history, tools, and state
  machines
- `crates/editor_render` — backend-neutral display-list types
- `crates/render_svg` — SVG serialization
- `crates/render_wgpu` and `apps/desktop` — legacy wgpu renderer and shell
- `crates/ui` — shared UI-facing abstractions used by the legacy shell

`SPEC.md` describes intended editor behavior. `Rust Vector Editor
Architecture.md` contains the broader architecture notes; when either differs
from working code, tests and implementation are authoritative.
