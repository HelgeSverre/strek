# Strek native file format

> [!WARNING]
> The native file format is not stable yet. `.strek.json` files are an internal
> persistence format, not a public interchange API. Field names, enum shapes,
> defaults, validation rules, and arena encoding may change while Strek is under
> development. Keep backups, use SVG for interchange, and do not build external
> tooling that assumes arbitrary Strek versions will remain compatible.

The implementation and its tests are authoritative when this document drifts.
The primary definitions live in `crates/editor_core/src/lib.rs`, `node.rs`,
`path.rs`, `style.rs`, `layout.rs`, `precision.rs`, and `color_library.rs`;
application-level reading and writing live in `apps/gpui/src/document_io.rs`.

## Current encoding

| Property | Current value |
| --- | --- |
| Format version | `3` |
| Filename extension | `.strek.json` |
| Encoding | UTF-8 JSON |
| Writer output | Pretty-printed JSON |
| Maximum file size | 64 MiB |
| Interchange format | SVG, not native JSON |

Saving an extensionless path appends `.strek.json`. Strek validates a document
before saving, writes a temporary file in the destination directory, syncs it,
and replaces the destination. Opening validates the decoded document before it
becomes editor state.

## Top-level object

The top-level object is the serialized `SavedDocument`:

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | unsigned integer | Native format version. Writers currently emit `3`. |
| `nodes` | serialized slot-map array | Generational arena containing the scene graph. |
| `root` | node ID | ID of the single live root group. |
| `grid` | object | Document grid spacing and major-line interval. |
| `guides` | array | Persistent horizontal and vertical ruler guides. |
| `color_library` | object | Document-local color groups and saved colors. |

An empty document currently serializes as:

```json
{
  "version": 3,
  "nodes": [
    {
      "value": null,
      "version": 0
    },
    {
      "value": {
        "name": "Root",
        "parent": null,
        "children": [],
        "kind": "Group",
        "transform": [
          1.0,
          0.0,
          0.0,
          1.0,
          0.0,
          0.0
        ],
        "style": {
          "fill": {
            "Solid": [
              0.0,
              0.0,
              0.0,
              1.0
            ]
          },
          "stroke": null,
          "opacity": 1.0
        },
        "layout": "Free",
        "visible": true,
        "locked": false,
        "deleted": false
      },
      "version": 1
    }
  ],
  "root": {
    "idx": 1,
    "version": 1
  },
  "grid": {
    "spacing": 10.0,
    "major_every": 5
  },
  "guides": [],
  "color_library": {
    "groups": [],
    "colors": []
  }
}
```

## Node arena and IDs

`nodes` is the JSON representation of a `slotmap::SlotMap<NodeId, Node>`. The
first array entry is a sentinel slot. Occupied entries contain a `value` and a
slot `version`; vacant entries contain `null`. A node ID is encoded as:

```json
{ "idx": 1, "version": 1 }
```

IDs are generational arena handles, not user-facing identifiers. Treat the
entire `{ "idx", "version" }` value as opaque. Saving compacts soft-deleted
runtime nodes and remaps the remaining IDs, so IDs are not stable across a save
cycle and must not be stored outside the document.

Every node contains:

| Field | Meaning |
| --- | --- |
| `name` | Layer name. |
| `parent` | Parent node ID, or `null` for the root. |
| `children` | Ordered child IDs; the last child is topmost. |
| `kind` | Type-specific group, frame, shape, or text data. |
| `transform` | Six-number local affine transform, relative to the parent. |
| `style` | Optional fill, optional stroke, and overall opacity. |
| `layout` | `"Free"` or an `{"Auto": ...}` configuration. |
| `visible` | Visibility flag. |
| `locked` | Editing lock flag. |
| `deleted` | Soft-delete marker retained for legacy/runtime compatibility. |

The externally tagged `kind` variants are:

- `"Group"`
- `{"Frame": {"width": ..., "height": ..., "background": ...}}`
- `{"Shape": {"contours": [...]}}`
- `{"Text": {"content": ..., "font": ..., "font_size": ...,
  "line_height": ..., "align": ..., "sizing": ...}}`

Paint is currently limited to normalized sRGB solid color and is encoded as
`{"Solid": [r, g, b, a]}`. A stroke contains `width` and `paint`. Path data is
stored as contours containing anchors. Each anchor has a local `position`,
optional relative `in_handle` and `out_handle`, and a handle `mode` of
`Corner`, `Mirrored`, `Aligned`, or `Independent`.

These representations follow the current Rust `serde` output. They are
descriptive, not a separately governed schema.

## Document metadata

`grid` contains a positive finite `spacing` and `major_every` in the range
`1..=10`. Each guide contains a non-zero numeric `id`, a snake-case `axis` of
`horizontal` or `vertical`, and a finite document-space `position`.

`color_library` contains ordered `groups` and `colors`. Group and color IDs are
non-zero numeric values. RGBA values are four normalized finite components.
Manual order values must be unique and contiguous from zero within their scope.
Saved colors are copied into node paint when used; nodes do not reference color
library entries.

## Compatibility behavior

- Strek writes version 3.
- Version 1 and 2 documents are accepted. Grid, guides, and Color Library data
  are replaced with version-3 defaults during migration.
- Versions newer than 3 are rejected.
- Legacy shape data using a `commands` array is accepted and converted to the
  current contour-and-anchor representation. Writers emit `contours`.
- Unknown JSON fields are not a preservation mechanism. A load-save cycle may
  discard data the current model does not store.

The version number protects Strek from silently opening known future formats;
it does not establish a long-term third-party compatibility guarantee.

## Validation and limits

Strek rejects malformed or unreasonable documents before use. Current limits
include:

- 64 MiB per native document;
- 100,000 occupied nodes and 100,001 serialized node slots;
- 100,000 children per node;
- 10,000 contours and 1,000,000 anchors per shape;
- 4 MiB of UTF-8 text per text node;
- 10,000 guides;
- 256 color groups and 10,000 saved colors;
- 256 UTF-8 bytes per color or group name.

The scene graph must have one live group root with no parent. References must
resolve, parent and child links must agree, children cannot be duplicated,
cycles are rejected, and every live node must be reachable from the root.
Precision and color-library values must also satisfy their numeric, identifier,
ordering, and reference invariants.

## Not persisted

The native file does not store selection, viewport position or zoom, panel
state, undo/redo history, caches, shaped text layouts, the active tool, command
state, or an in-progress interaction. These are runtime concerns rebuilt or
reset when a document opens.
