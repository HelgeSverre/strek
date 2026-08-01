# Strek Architecture

This document describes the current implementation and the boundaries planned
for the next editor features. [`SPEC.md`](SPEC.md) defines product behavior;
tests and working code remain authoritative when documentation drifts.

## Design goals

Strek is a focused native editor for logos and icons. Its architecture favors:

- a platform-neutral document and editing core;
- deterministic, undoable document mutations;
- derived rendering data that cannot leak into saved files;
- one semantic command path shared by UI and automation;
- strict, versioned persistence and deliberately bounded interchange formats.

The project does not try to provide a general plugin host, a browser-style UI
runtime, or the full precision and design-system surface of a large layout tool.

## Workspace boundaries

```text
apps/gpui
  native UI, input translation, dialogs, clipboard, files, automation, MCP
       │
       ▼
crates/editor_core ───────────────► crates/editor_render
  document, tools, history,           backend-neutral display list
  layout, selection, snapping                    │
       │                                         ├──► apps/gpui canvas
       │                                         └──► crates/render_svg
       └── native JSON persistence                       SVG serialization
                                                            │
                                                            └──► raster export
```

### `crates/editor_core`

The core owns editor semantics and has no dependency on GPUI.

- `Document` is a scene graph stored in a `SlotMap<NodeId, Node>`. The root is
  always a group; parent and ordered-child links must agree.
- `Node` stores authored transform, style, layout, visibility, lock state, and
  one `NodeKind` such as group, frame, shape, or text.
- World transforms, bounds, and shaped text layouts are derived caches. They are
  invalidated after mutations and excluded from persistence.
- `Editor` combines the document with selection, current tool, viewport,
  interaction state, history, clipboard state, creation defaults, and snapping.
- `EditorAction` is the stable semantic action catalog. `InputEvent` translates
  lower-level pointer and keyboard input into editor operations.
- `Patch`, `Command`, `History`, and `Transaction` provide reversible edits.
  Interactive operations preview from a captured starting state and commit one
  history entry when the interaction finishes.
- `SnapEngine` finds screen-tolerance object, persistent-guide, and analytic-grid
  candidates. Its transient snap guides are display-list overlays rather than
  document objects.

The core is the right home for behavior that must be consistent across the UI,
tests, command socket, AppleScript helper, and MCP server.

### `crates/editor_render`

This crate defines `DisplayList` and `DisplayItem`. It is a transport boundary,
not a second document model. Artwork items use document transforms; editor-only
items include selection bounds, handles, carets, tool previews, and transient
snap guides.

Exporters must explicitly ignore editor-only items. New overlays such as grids,
rulers, and persistent guides must remain non-exporting by construction.

### `crates/render_svg`

The SVG backend serializes display lists for preview and export. Export uses an
explicit artwork view box and omits editor chrome. The GPUI app also rasterizes
the exported SVG representation for PNG, JPEG, and WebP, keeping vector and
raster output on the same settled artwork snapshot.

### `apps/gpui`

The application crate owns platform and presentation concerns:

- GPUI windows, menus, panels, focus, keyboard/IME input, and pointer routing;
- canvas painting and text-image caching;
- command metadata, configurable shortcuts, and command-palette presentation;
- native open/save/export dialogs, recent files, and atomic file writes;
- strict SVG import into the core document model;
- clipboard payloads and platform-specific image-format availability;
- the command socket, CLI forwarding, AppleScript integration, and MCP server.

The shared color picker in `color_picker.rs` is used by selection fill/stroke
editing and shape-creation defaults. Reusable color selection should extend this
component instead of creating context-specific pickers.

`main.rs` is currently the application composition root and contains substantial
UI orchestration. New self-contained surfaces should get their own modules and
expose state and event callbacks back to the root rather than making that file a
larger feature implementation.

## Runtime data flow

### Interactive editing

1. GPUI translates a platform event into an editor action or `InputEvent`.
2. `Editor` validates the active tool, selection, and interaction state.
3. A direct edit runs as one command; a drag or scrub runs in a transaction.
4. The document marks affected derived data dirty and recomputes layout/caches
   when queried.
5. `Editor::build_display_list` produces artwork and transient overlays.
6. The GPUI canvas paints the list and updates surrounding panel state.

UI code should not mutate nodes directly when the same operation belongs in
history or automation. Add a core operation and expose it through the semantic
action/command surface instead.

### Save and load

The current native format is version 3. `SavedDocument` contains the version,
node arena, root ID, grid settings, persistent guides, and the document Color
Library. Version 1 and 2 documents migrate with bounded defaults. Loading rejects
future versions, excessive input, and invalid scene graphs. Saving validates
first, serializes a snapshot, and replaces the destination through a
same-directory temporary file.

Runtime-only state is intentionally absent: selection, viewport, open panels,
undo history, caches, text layouts, tool state, and current interaction.

### Import and export

SVG import is an all-or-nothing conversion of a documented subset. Unsupported
features produce an error instead of silently changing appearance. Imported SVG
is an unsaved native document and is never overwritten by Save.

Export settles transient edits, builds one visible-artwork snapshot, derives its
bounds, and encodes SVG or a raster format. Copy-as-image uses the same path.

### Automation

The running app owns a local command endpoint. CLI, the packaged AppleScript
helper, and MCP all translate into the same request model and return structured
results. Semantic layer IDs and property operations are preferred; canvas-local
pointer events remain available for spatial gestures.

Automation additions should follow the UI's domain operations. A feature is not
complete when it can only be driven by coordinates if its intent can be
represented as a stable command.

## State ownership rules

Use these rules when adding a feature:

| State | Owner | Saved in document | Marks document dirty | Undoable |
| --- | --- | --- | --- | --- |
| Authored nodes and styles | `editor_core::Document` | Yes | Yes | Yes |
| Selection, tool, interaction | `editor_core::Editor` | No | No | No |
| Viewport and panel presentation | GPUI workspace state | No | No | No |
| Derived transforms, bounds, text | Core caches | No | No | No |
| Shortcut and recent-file preferences | GPUI user config | No | No | No |
| Export snapshot | Core-derived value | No | No | No |

Persist definitions needed to understand or continue editing a document. Keep
visibility, panel geometry, and interaction preferences local unless they alter
document meaning.

## Precision aids

The bounded design is specified in [`SPEC.md`](SPEC.md#implemented-canvas-precision-controls).
It extends the existing editor and canvas systems rather than adding a parallel
canvas model.

Document-owned, versioned data:

- one square `GridSettings` definition;
- horizontal and vertical `Guide` values with stable IDs.

Workspace-owned preferences:

- ruler, grid, and guide visibility;
- guide locking;
- master snapping and object/guide/grid target toggles;
- screen-space snap tolerance.

Current core behavior:

- stores document metadata in native format version 3 with defaults
  for version 1 and 2 files;
- represents snap targets by category rather than inferring guides from a missing
  node ID;
- chooses X and Y corrections independently, using deterministic tie breaking;
- preserves the existing transaction boundary while applying snap coverage to
  resize and creation operations;
- exposes guide/grid mutations as commands and semantic automation operations.

Current GPUI behavior:

- renders the zoom-adaptive grid below artwork and guides/rulers above artwork;
- renders and hit-tests document guides separately from transient smart guides;
- provides View commands and a compact snapping/precision popover;
- stores visibility and control choices as workspace preferences without making
  the document dirty.

Rulers, grids, guides, and smart-guide feedback never enter artwork snapshots.
User-interface iconography comes from embedded SVG assets with a shared 24-pixel
stroke style; text glyphs and emoji are not used as substitute icons.

## Document Color Library

The bounded design is specified in [`SPEC.md`](SPEC.md#implemented-document-color-library).
Version one is a collection of unlinked solid RGBA values. Applying a Saved
Color copies its value; editing or deleting it cannot alter existing artwork.

Document-owned, versioned data:

- ordered `ColorGroup` records with stable IDs;
- `SavedColor` records with stable ID, optional group and name, RGBA value, and
  manual order;
- each group's selected sort mode and direction, because these affect the
  document palette's authored organization.

Workspace-owned preferences:

- Color Library panel position and size;
- collapsed groups and the last-used group.

Current core behavior:

- provides bounded color-library storage, validation, sorting keys, and reversible
  CRUD/reorder operations;
- defines perceptual sorting in OKLCH and brightness sorting by relative
  luminance;
- includes library data in native version 3 without adding references from nodes
  to saved colors.

Current GPUI behavior:

- extends the existing shared picker with Saved Colors search and one-click Save
  Color;
- implements the Color Library as a modeless, movable, resizable in-app utility
  panel in its own module;
- routes inline edits, reordering, grouping, and deletion through core commands;
- provides semantic automation for library and group management.

Linked values, aliases, themes, and cross-document libraries are a separate
future concept named Color Variables. They must not be smuggled into Saved Color
semantics.

## Implemented sequence

The implementation proceeded in this order:

1. Add typed document metadata, version-3 migration, limits, and round-trip
   tests.
2. Add guide/grid and Color Library domain operations in separate core modules.
3. Extend snapping and canvas overlays, then add rulers and precision controls.
4. Add Saved Colors to the shared picker, then build the Color Library panel.
5. Add semantic automation, user documentation, and end-to-end regression tests.

Keeping domain operations ahead of UI wiring prevents the GPUI root from
becoming the only place where new behavior can be exercised.

## Architectural invariants

- The document graph is a valid rooted tree with stable generational IDs.
- Derived caches and editor chrome are never serialized as authored content.
- Every committed document mutation is undoable and clears redo history.
- A cancelled interaction restores its captured starting state.
- Export never includes selection, tools, rulers, grids, or guides.
- Invalid or unsupported input fails explicitly; it is not partially accepted.
- UI and automation use the same semantic operation wherever one exists.
- Saved Colors are copy-by-value until a separately specified linked system is
  introduced.

## Verification baseline

The normal repository gate is:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --all-targets --locked
cargo audit
```

Core model changes need unit tests for validation, persistence, undo/redo, and
cancelled transactions. UI features need focused interaction/state tests, and
new automation commands need parser, transport, and end-to-end coverage.
