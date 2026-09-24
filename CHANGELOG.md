# Changelog

All notable user-facing changes to Strek are documented here.

## [Unreleased]

### Added

- Added a searchable font picker for text layers that lists every installed
  font family with a preview. Open it from the text toolbar, the Typography
  family field, or **Choose Font…** in the command palette. Choosing a family is
  one undoable edit.
- Marked text families that are not installed as "(missing)" and listed them in
  the picker with the face they draw with.

### Fixed

- Fixed System, Serif, Monospace, and missing font families drawing with
  different faces on the canvas, on rotated text, and in PNG/outlined-SVG
  export. On Linux, System text previously fell back to an arbitrary font
  because GPUI looks for an unbundled "Zed Plex Sans".
- Stopped generic families from naming an uninstalled font, which could drop
  text from Linux exports when DejaVu fonts were absent.
- Fixed the Linux interface drawing in the typewriter font FreeMono (GPUI's
  Linux default). Strek now uses the first installed desktop sans-serif
  (Ubuntu Sans, Ubuntu, Adwaita Sans, Cantarell, Noto Sans, Inter, DejaVu Sans,
  Liberation Sans, or FreeSans) and an installed monospace face for numeric
  fields. macOS and Windows keep their existing interface fonts.

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

[0.2.3]: https://github.com/HelgeSverre/strek/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/HelgeSverre/strek/compare/v0.2.1...v0.2.2
