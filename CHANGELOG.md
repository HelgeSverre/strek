# Changelog

All notable user-facing changes to Strek are documented here.

## [0.2.4] - 2026-10-09

### Added

- Added a select-layer menu to the canvas: Cmd+right-click on macOS or
  Ctrl+right-click elsewhere (either works on every platform) lists every
  visible, unlocked shape, text, or frame under the pointer plus the groups and
  frames that contain one, in Layers panel order. Choosing a row selects that
  layer, including layers hidden behind others, and the menu marks layers that
  are already selected.
- Added Strek-drawn window controls on Linux and FreeBSD. Strek requests
  client-side decorations; when the session provides them (Wayland compositors
  without server-side decorations, such as GNOME, and X11 window managers with a
  compositor that support GTK frame extents), the header shows minimize,
  maximize/restore, and close buttons, moves the window when dragged, maximizes
  or restores on double-click when the compositor allows maximizing, opens the
  window menu on right-click, and resizes from the window edges. Other X11
  sessions keep the window manager's frame. On X11, GPUI does not report the
  maximized state, so the maximize button keeps its Maximize icon while the
  window is maximized; clicking it still restores the window.
- Added a searchable font picker for text layers that lists installed font
  families with a preview. Open it from the text toolbar, the Typography
  family field, or **Choose Font…** in the command palette. Choosing a family is
  one undoable edit. The picker omits families the canvas cannot draw by name:
  fonts without a Latin "m" glyph (most non-Latin script, symbol, and emoji
  fonts) on macOS and Linux, and names DirectWrite does not list on Windows.
- Marked text families that are not installed as "(missing)" and listed them in
  the picker with the face they draw with.
- Bundled Inter 4.1 (SIL Open Font License 1.1) as the interface font. When
  Inter is not installed, the bundled copy is also available to text layers and
  export; an installed Inter takes precedence.

### Changed

- Set a minimum window size of 640 × 400 on every platform.

### Fixed

- Fixed GNOME on Wayland showing no window controls, which left the window
  impossible to move, minimize, maximize, or close from its frame.
- Replaced GPUI's fixed-width, unthemed Linux prompt, whose text overflowed the
  card, with an in-window prompt in the editor's theme and interface font. Text
  wraps, long details scroll, the card shrinks in narrow windows, Enter chooses
  the highlighted answer, Tab and the arrow keys move the highlight, and Escape
  chooses the last answer (Cancel). macOS and Windows keep native dialogs.
- Fixed System, Serif, Monospace, and missing font families drawing with
  different faces on the canvas, on rotated text, and in PNG/outlined-SVG
  export. On Linux, System text previously fell back to an arbitrary font
  because GPUI looks for an unbundled "Zed Plex Sans".
- Stopped generic families from naming an uninstalled font, which could drop
  text from Linux exports when DejaVu fonts were absent.
- Fixed the Linux interface drawing in the typewriter font FreeMono (GPUI's
  Linux default). The interface now uses the bundled Inter font on every
  platform, and numeric fields use an installed monospace face.

## [0.2.3] - 2026-09-01

### Changed

- Changed Layers panel selection to use contiguous ranges for Shift-click and
  individual toggles for Command-click on macOS or Control-click elsewhere.
- Kept hidden and locked layers selectable from the Layers panel, including
  after changing their visibility or lock state.

### Fixed

- Prevented pending inline property edits from applying to a different layer
  after the Layers panel changes the selection.
- Restored the corresponding selection when undoing or redoing creation,
  deletion, grouping, ungrouping, duplication, and paste operations.
- Made single rotated objects resize in their local frame so handles follow the
  rotated bounds and non-uniform resizing does not shear the object.
- Prevented hidden and locked selections from exposing canvas transform handles.

## [0.2.2] - 2026-08-31

### Added

- Added an agent control plane with runtime capability discovery, session-local
  revisions, guarded mutations, stable error codes, bounded retry
  deduplication, mutation receipts, and targeted document projections.
- Added CLI and MCP access to capability discovery, guarded calls, and bounded
  layer queries with optional descendants, styles, and computed geometry.
- Added subtle hierarchy guides to the Layers panel.

### Changed

- Made the Layers panel denser with smaller indentation and row controls,
  single-line ellipsized names, and improved text/icon alignment.
- Changed layer action buttons to use a vertical ellipsis and open without
  changing the current selection. Chosen actions still target the button's row.
- Expanded the architecture, automation, and delivery documentation around
  agent-safe control, verification, and precise capability claims.

### Fixed

- Prevented guarded automation retries from repeating accepted mutations or
  deadlocking after an out-of-order sequence.
- Kept exposed document revisions monotonic across undo and document
  replacement.

[0.2.4]: https://github.com/HelgeSverre/strek/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/HelgeSverre/strek/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/HelgeSverre/strek/compare/v0.2.1...v0.2.2
