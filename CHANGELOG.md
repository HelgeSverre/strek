# Changelog

All notable user-facing changes to Strek are documented here.

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
