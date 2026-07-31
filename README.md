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
- Frames, rectangles, ellipses, lines, Bézier paths, and editable text
- Direct vector editing for anchors and handles, including join, split, reverse,
  and open/close operations
- Move, resize, rotate, snapping, pan, zoom, undo, and redo
- Native Layers and Design panels, including layer context actions, transform
  decomposition readouts, and auto-layout mode, direction, spacing, and padding
  controls
- Text family, weight, italic, size, and alignment controls
- Live horizontal scrubbing for position, opacity, stroke width, text size, and
  auto-layout spacing/padding, committed as one undo step
- Per-tool fill and stroke defaults for new shapes and paths in the context bar
- Command palette, menus, and configurable shortcuts
- Native JSON document open/save with validated scene graphs and recent files
- SVG, outlined-text SVG, PNG, JPEG, and WebP export

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

Tagged releases provide macOS binaries for Apple Silicon and Intel. After the
first release, install Strek from Homebrew:

```sh
brew install helgesverre/tap/strek
```

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

Standard SVG keeps text as editable text and relies on compatible fonts being
available when the file is opened. Outlined-text SVG converts glyphs to paths
for portable appearance. Strek does not automatically embed fonts in SVG files.

## Development

Run the same checks used by CI:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo audit
```

Useful `just` recipes:

```sh
just test    # library tests
just lint    # Clippy with warnings denied
just format  # format the workspace
```

Tagged versions matching `vX.Y.Z` run the cargo-dist release workflow and
publish the `strek` formula to `helgesverre/homebrew-tap`. The repository needs
a `HOMEBREW_TAP_TOKEN` Actions secret with write access to that tap before the
first release.

## Package for macOS

The GPUI package metadata produces an ad-hoc-signed, architecture-specific
macOS application bundle for local testing. Install the packager once, then
build the bundle:

```sh
cargo install cargo-packager --locked
cargo packager -p strek --release --formats app
```

The bundle is written below `target/release/`. Signing, notarization, and a
second architecture must be added and tested before distributing it publicly.

The workspace is split by responsibility:

- `apps/gpui` — native application and GPUI interface
- `crates/editor_core` — document model, commands, history, tools, and state
  machines
- `crates/editor_render` — backend-neutral display-list types
- `crates/render_svg` — SVG serialization

`SPEC.md` describes intended editor behavior. `Strek Architecture.md` contains
broader historical architecture notes; when either differs from working code,
tests and implementation are authoritative.
