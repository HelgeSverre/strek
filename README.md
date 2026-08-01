<p align="center">
  <img src="apps/gpui/assets/app-icon.png" width="144" height="144" alt="Strek app icon">
</p>

<h1 align="center">Strek</h1>

<p align="center">
  A native, automation-ready vector editor for logos and icons, built in Rust with GPUI.
</p>

<p align="center">
  <a href="https://github.com/HelgeSverre/strek/actions/workflows/ci.yml"><img src="https://github.com/HelgeSverre/strek/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
  <img src="https://img.shields.io/badge/platform-macOS-0C8CE9?style=flat-square&amp;logo=apple&amp;logoColor=white" alt="Platform: macOS">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-0C8CE9?style=flat-square" alt="License: MIT"></a>
</p>

Strek is a GPUI desktop application backed by reusable editor, rendering, and
export crates. Its native interface is designed around focused logo and icon
workflows rather than general-purpose illustration.

The project is under active development. Core editing workflows are usable, but
the file format and UI should not yet be treated as stable.

## What works

- Selection, marquee selection, grouping, ordering, alignment, and distribution
- Layer tree editing: rename, visibility and lock state, reordering, and
  reparenting
- Frames, rectangles, ellipses, lines, Bézier paths, and editable text
- Direct vector editing for anchors and handles, including join, split, reverse,
  and open/close operations
- Move, resize, rotate, snapping, pan, zoom, undo, and redo
- Native Layers and Design panels, including layer context actions, transform
  decomposition readouts, and auto-layout mode, direction, spacing, and padding
  controls
- Text editing with selection, IME composition, family, weight, italic, size,
  and alignment controls
- Live horizontal scrubbing and fine/coarse steppers for position, opacity,
  stroke width, text size, and auto-layout spacing/padding; each completed scrub
  is committed as one undo step and `Escape` cancels it
- Frame background toggling and per-tool fill/stroke defaults for new shapes and
  paths
- Reusable fill/stroke color picker with a hue/saturation wheel, HEX and alpha
  editing, presets, and HSV, HSL, RGB, and gamut-mapped OKLCH controls
- Command palette, menus, and configurable shortcuts
- Native JSON document open/save with validated scene graphs and recent files
- Editable SVG import for paths and basic shape primitives, with explicit errors
  for unsupported features
- SVG, outlined-text SVG, PNG, JPEG, and WebP export
- Copy visible artwork to the clipboard in the image formats supported by the
  current platform

## Run the GPUI app

The current reference platform and CI environment is macOS. Install a stable
Rust toolchain and the Xcode Command Line Tools, then run:

```sh
cargo run -p strek
```

With [just](https://github.com/casey/just) installed, the equivalent command is:

```sh
just run
```

## Install

Strek is currently a private preview. Collaborators can download the Apple
Silicon/Intel universal macOS installer from the repository's private GitHub
Releases. The installer is Developer ID-signed and notarized by Apple. There is
no public download page or Homebrew formula yet. Everyone else should run from
source or create a local package as described below.

## Automation

Strek can be inspected and controlled without moving the system cursor through
its CLI, bundled AppleScript helper, or MCP server:

```sh
strek automate state
strek automate action tool.rectangle
strek automate pointer down 120 100
strek automate pointer move 360 260
strek automate pointer up 360 260
```

The packaged app includes an AppleScript helper:

```sh
osascript "/Applications/Strek.app/Contents/Resources/Strek Automation.applescript" state
```

### Connect the MCP server

Strek includes a local stdio MCP server. The desktop application must already
be running; the MCP process forwards tool calls to that application. First
resolve the absolute path to the installed binary:

```sh
STREK_BIN="$(command -v strek)"
```

When running from source instead, use an absolute development-build path such
as `STREK_BIN="$PWD/target/debug/strek"`.

Claude Code (user scope):

```sh
claude mcp add --scope user strek -- "$STREK_BIN" mcp
```

OpenCode:

```sh
opencode mcp add strek -- "$STREK_BIN" mcp
```

Codex:

```sh
codex mcp add strek -- "$STREK_BIN" mcp
```

GitHub Copilot CLI:

```sh
copilot mcp add strek -- "$STREK_BIN" mcp
```

Amp:

```sh
amp mcp add strek -- "$STREK_BIN" mcp
```

Restart or reconnect the client after adding the server if it does not discover
Strek immediately. The MCP server exposes state inspection, editor actions,
canvas pointer input with modifiers, text insertion, UI visibility controls,
and screenshots.

See [AUTOMATION.md](AUTOMATION.md) for the complete CLI, AppleScript, and MCP
surface, client configuration, state schema, coordinates, permissions, and
troubleshooting.

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
**Strek → Keyboard Shortcuts…** to create and open the user
`keybindings.json`; changes take effect after restarting the app. A string
replaces a binding, an array assigns alternatives, and `null` disables it.

## Documents and export

Documents are stored as versioned JSON. Saving an extensionless filename adds
the native `.strek.json` extension. The loader validates the scene graph before
opening it, and writes use a same-directory temporary file before replacement.

Use the File menu to export visible artwork as standard SVG, outlined-text SVG,
transparent PNG, JPEG, or lossless WebP. Export bounds are derived from the
artwork rather than the current viewport. JPEG output composites transparent
pixels over white; PNG and WebP preserve transparency.

The same visible-artwork snapshot can be copied from the File menu or command
palette. macOS supports SVG, PNG, and WebP clipboard images; Windows supports
SVG and PNG. Image-copy commands are unavailable on Linux and omitted from its
File menus because the pinned GPUI backend does not publish image-only clipboard
payloads there. Receiving applications may still vary in which advertised image
formats they accept.

Standard SVG keeps text as editable text and relies on compatible fonts being
available when the file is opened. Outlined-text SVG converts glyphs to paths
for portable appearance. Strek does not automatically embed fonts in SVG files.

### SVG import

Opening an `.svg` file imports its supported contents as editable native layers.
The supported subset includes groups, paths, rectangles, circles, ellipses,
lines, polylines, and polygons; affine transforms; solid fills and strokes;
fill and stroke opacity; and visibility. Quadratic curves and SVG shape
primitives are converted to the editor's path representation.

Strek rejects the whole import with a descriptive error when the SVG uses a
feature that cannot be represented faithfully. This includes text, images,
gradients, patterns, clipping, masks, filters, markers, nested SVG documents,
CSS style blocks, even-odd fills, dashed strokes, and advanced stroke caps,
joins, object/group opacity, or paint order. Excessive nesting, node counts, path
segments, and geometry attribute sizes are also rejected before conversion.
Convert unsupported features to ordinary paths and solid paints before
importing.

An imported SVG is treated as an unsaved native document. Saving opens a Save As
dialog and writes `.strek.json`; Strek never overwrites the source SVG.

Exports use the bounds of all visible artwork rather than a selected frame or a
custom export preset. In the Design panel, width/height and transform
decomposition values are currently readouts; numeric adjustment is available for
the properties listed above through stepping and scrubbing, but direct text entry
is not yet available.

## Development

Run the repository checks locally:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --all-targets --locked
cargo audit
```

Useful `just` recipes:

```sh
just test    # workspace tests
just lint    # Clippy with warnings denied
just format  # format the workspace
```

Tagged versions matching `vX.Y.Z` run the cargo-dist release workflow and attach
Apple Silicon and Intel archives to a private GitHub Release. The workflow does
not publish package-manager or shell installers while the repository remains
private.

## Package for macOS

The GPUI package metadata and packaging script produce a universal macOS
application and installer. Install the pinned packager once, then run a local
unsigned structure test:

```sh
cargo install cargo-packager --version 0.11.8 --locked
.github/scripts/package-macos-pkg.sh --unsigned
```

Unsigned local packages must not be distributed. Production packages are built
in GitHub Actions, signed with separate Developer ID Application and Installer
certificates, notarized, stapled, verified, and attached to tagged releases.
See [docs/MACOS_RELEASE.md](docs/MACOS_RELEASE.md) for the one-time Apple and
GitHub credential setup, manual test workflow, and release procedure.

The workspace is split by responsibility:

- `apps/gpui` — native application and GPUI interface
- `crates/editor_core` — document model, commands, history, tools, and state
  machines
- `crates/editor_render` — backend-neutral display-list types
- `crates/render_svg` — SVG serialization

`SPEC.md` describes intended editor behavior. `Strek Architecture.md` contains
broader historical architecture notes; when either differs from working code,
tests and implementation are authoritative.
