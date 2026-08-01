# Strek Specification

A minimal, cross-platform vector editor core written in Rust. Designed for creating logos and icons with support for groups/layers, text/fonts, basic editing/positioning, and simple auto-layout alignment.

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [Workspace Structure](#2-workspace-structure)
3. [Core Data Model](#3-core-data-model)
4. [Node System](#4-node-system)
5. [Transform System](#5-transform-system)
6. [Style System](#6-style-system)
7. [Layout System](#7-layout-system)
8. [Text System](#8-text-system)
9. [Query System](#9-query-system)
10. [Editing Model](#10-editing-model)
11. [Rendering System](#11-rendering-system)
12. [Input System](#12-input-system)
13. [Platform Frontends](#13-platform-frontends)
14. [File Format](#14-file-format)
15. [Implementation Milestones](#15-implementation-milestones)

---

## 1. Architecture Overview

The editor follows a three-layer architecture with strict separation of concerns:

```
┌─────────────────────────────────────────────────────────────────┐
│                         FRONTEND                                │
│              GPUI product shell + native input                 │
│                    + canvas rendering                          │
└───────────────────────────────┬─────────────────────────────────┘
                                │ InputEvent / DisplayList
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│                      EDITOR CORE                                │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐                 │
│  │  Document  │  │   Tools    │  │  History   │                 │
│  │   Model    │  │   State    │  │  (Undo)    │                 │
│  └────────────┘  └────────────┘  └────────────┘                 │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐                 │
│  │  Selection │  │   Layout   │  │   Cache    │                 │
│  │   Model    │  │   Engine   │  │  (Derived) │                 │
│  └────────────┘  └────────────┘  └────────────┘                 │
└─────────────────────────────────────────────────────────────────┘
              │
              ▼ DisplayList
┌─────────────────────────────────────────────────────────────────┐
│                    OUTPUT BACKENDS                              │
│  ┌─────────────────────┐       ┌─────────────────────┐          │
│  │     GPUI canvas     │       │     render_svg      │          │
│  │   (native window)   │       │    (SVG string)     │          │
│  └─────────────────────┘       └─────────────────────┘          │
└─────────────────────────────────────────────────────────────────┘
```

### Layer Responsibilities

#### A. Core Layer (`editor_core`)
- **Document model**: Scene graph, layer tree, node storage
- **Stable IDs**: Generational arena for safe node references
- **Geometry + style + text data**: All node properties
- **Queries**: World transforms, bounds, hit-test, z-order traversal
- **Derived caches**: Dirty flags for incremental updates
- **Selection model**: Multi-select, marquee selection
- **Tool logic**: Move, scale, rotate, group/ungroup
- **Command system**: Undo/redo with patch-based history
- **Snapping/alignment**: Guides, distribute, align operations
- **Transactions**: Interactive edit sessions (drag start → updates → commit)

#### B. Edit Layer (`editor_core::editor`)
- Intent-to-mutation translation
- Tool state machines
- Interactive editing sessions

#### C. Layout Layer (`editor_core::layout`)
- Auto-layout computation for groups
- Child positioning based on layout rules

#### D. Render Layer (`editor_render`)
- Backend-agnostic display list generation
- No direct rendering; produces intermediate representation

---

## 2. Workspace Structure

```
strek/
├── Cargo.toml                    # Workspace manifest
├── SPEC.md                       # This document
├── crates/
│   ├── editor_core/              # Document + tools + undo + snapping + layout
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs            # Document graph, queries, serialization
│   │       ├── editor.rs         # Editor state machines and operations
│   │       ├── action.rs         # Core action catalog
│   │       ├── node.rs           # Node types and properties
│   │       ├── style.rs          # Paint, stroke, style
│   │       ├── path.rs           # Contours, anchors, and geometry
│   │       ├── text.rs           # Text data and font spec
│   │       ├── layout.rs         # Auto-layout types and engine
│   │       ├── transform.rs      # Affine transforms, view
│   │       ├── cache.rs          # Derived data caching
│   │       ├── selection.rs      # Selection state
│   │       ├── command.rs        # Command/patch system
│   │       ├── history.rs        # Undo/redo stack
│   │       ├── input.rs          # Platform-agnostic input events
│   │       ├── layers.rs         # Layer-panel projection
│   │       ├── render.rs         # Document-to-display-list conversion
│   │       ├── snap.rs           # Snapping and alignment
│   │       └── transaction.rs    # Reversible pointer transactions
│   │
│   ├── editor_render/            # Display list IR (backend-agnostic)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs            # DisplayList, DisplayItem types
│   │
│   ├── render_svg/               # SVG export renderer
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs            # SVG string generation
│   │
└── apps/
    └── gpui/                     # Native product UI
        ├── Cargo.toml
        └── src/
            ├── main.rs           # GPUI shell and event routing
            ├── canvas.rs         # GPUI display-list renderer
            ├── commands.rs       # Command registry and keymap
            ├── command_palette.rs
            ├── document_io.rs
            ├── export.rs
            ├── layer_panel.rs
            ├── properties_panel.rs
            ├── text_input.rs
            └── toolbar.rs
```

### Cargo Workspace Configuration

```toml
# Cargo.toml (root)
[workspace]
members = [
    "crates/editor_core",
    "crates/editor_render",
    "crates/render_svg",
    "apps/gpui",
]
resolver = "2"

[workspace.dependencies]
glam = { version = "0.29", features = ["serde"] }
slotmap = "1.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
unicode-segmentation = "1.12"
```

---

## 3. Core Data Model

### 3.1 Generational Arena

The document uses a **generational arena** (`slotmap`) for node storage:

- **Stable IDs**: Node references remain valid across insertions/deletions
- **Safe deletion**: Old IDs become invalid (generation mismatch) rather than aliasing new nodes
- **Efficient access**: O(1) lookup by ID

```rust
use slotmap::{SlotMap, SecondaryMap, new_key_type};

new_key_type! { pub struct NodeId; }
```

**Why generational arena matters:**
- Selection stores `NodeId` → survives reordering, editing, grouping
- Undo/redo references nodes by ID → patches remain valid
- Caches keyed by ID → invalidation is straightforward
- No index shifting bugs from Vec-based storage

### 3.2 Document Structure

```rust
#[derive(Debug, Clone)]
pub struct Document {
    /// Primary node storage (generational arena)
    pub nodes: SlotMap<NodeId, Node>,

    /// Root node ID (always a Group)
    pub root: NodeId,

    /// Cached derived data
    pub cache: Cache,

    /// Dirty flags for incremental updates
    pub dirty: DirtyFlags,
}

#[derive(Debug, Clone, Default)]
pub struct Cache {
    /// World transform for each node (parent chain composed)
    pub world_transform: SecondaryMap<NodeId, Affine2>,

    /// Axis-aligned bounding box in world space
    pub world_bounds: SecondaryMap<NodeId, Rect>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DirtyFlags {
    pub transforms: bool,
    pub bounds: bool,
    pub layout: bool,
}
```

---

## 4. Node System

### 4.1 Node Structure

Each node in the scene graph contains:

```rust
#[derive(Debug, Clone)]
pub struct Node {
    /// Human-readable name (for layers panel)
    pub name: String,

    /// Parent reference (None for root)
    pub parent: Option<NodeId>,

    /// Ordered children (layer order: last = topmost)
    pub children: Vec<NodeId>,

    /// Node type and type-specific data
    pub kind: NodeKind,

    /// Local transform (relative to parent)
    pub transform: Affine2,

    /// Visual style (fill, stroke, opacity)
    pub style: Style,

    /// Layout mode (free or auto-layout)
    pub layout: Layout,

    /// Visibility flag
    pub visible: bool,

    /// Lock flag (prevent editing)
    pub locked: bool,

    /// Soft-delete flag used by undo/redo to preserve node identity
    pub deleted: bool,
}
```

### 4.2 Node Kinds

```rust
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// Container node with ordered children
    Group,

    /// Frame: container with explicit dimensions (like Figma frames/artboards)
    Frame(FrameData),

    /// Vector path (lines, curves, shapes)
    Shape(PathData),

    /// Text content with font properties
    Text(TextData),
}
```

### 4.3 Frame Data

Frames are containers with explicit dimensions, useful for defining artboards and fixed-size layouts:

```rust
#[derive(Debug, Clone)]
pub struct FrameData {
    /// Explicit width in local units
    pub width: f32,

    /// Explicit height in local units
    pub height: f32,

    /// Optional background (white by default)
    pub background: Option<Paint>,
}

impl Default for FrameData {
    fn default() -> Self {
        Self {
            width: 100.0,
            height: 100.0,
            background: Some(Paint::white()),
        }
    }
}
```

**Frame vs Group:**
- **Group**: Bounds derived from children; no clipping
- **Frame**: Explicit width/height; optional background, but no clipping

### 4.4 Path Data

```rust
#[derive(Debug, Clone, Default)]
pub struct PathData {
    pub commands: Vec<PathCmd>,
}

#[derive(Debug, Clone, Copy)]
pub enum PathCmd {
    /// Move to position (start new subpath)
    MoveTo(Vec2),

    /// Line to position
    LineTo(Vec2),

    /// Cubic bezier curve
    CubicTo { c1: Vec2, c2: Vec2, p: Vec2 },

    /// Close current subpath
    Close,
}
```

**Path coordinate space**: All path coordinates are in the node's local space. The node's `transform` positions the path in parent space.

---

## 5. Transform System

### 5.1 Affine Transforms

An **affine transform** represents 2D transformations:
- Translation (move)
- Rotation
- Scale (uniform or non-uniform)
- Shear/skew

Represented as a 2×3 matrix (or 3×3 with implicit bottom row `[0, 0, 1]`):

```rust
use glam::Affine2;

// Affine2 provides:
// - from_translation(Vec2)
// - from_scale(Vec2)
// - from_angle(f32)
// - from_scale_angle_translation(Vec2, f32, Vec2)
// - inverse()
// - Matrix multiplication via `*` operator
```

### 5.2 Transform Hierarchy

```
World Transform = Parent World Transform × Node Local Transform
```

For a node at depth 3:
```
world = root.transform × group1.transform × group2.transform × node.transform
```

### 5.3 View Transform (Camera)

The view transform maps world coordinates to screen coordinates:

```rust
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Pan offset in screen pixels
    pub pan: Vec2,

    /// Zoom level (1.0 = 100%)
    pub zoom: f32,
}

impl View {
    /// Transform from world space to screen space
    pub fn screen_from_world(&self) -> Affine2 {
        Affine2::from_scale_angle_translation(
            Vec2::splat(self.zoom),
            0.0,
            self.pan
        )
    }

    /// Transform from screen space to world space
    pub fn world_from_screen(&self) -> Affine2 {
        self.screen_from_world().inverse()
    }
}
```

### 5.4 Dual Transform Model (for Auto-Layout)

When using auto-layout, nodes have two conceptual transforms:

1. **Author transform**: User-specified (rotation, scale)
2. **Layout offset**: Computed by layout engine (position within parent)

```
World = Parent × Author × LayoutOffset
```

This prevents auto-layout from destroying user transformations.

---

## 6. Style System

### 6.1 Style Structure

```rust
#[derive(Debug, Clone)]
pub struct Style {
    /// Fill paint (None = no fill)
    pub fill: Option<Paint>,

    /// Stroke properties (None = no stroke)
    pub stroke: Option<Stroke>,

    /// Overall opacity (0.0 - 1.0)
    pub opacity: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Some(Paint::Solid([0.0, 0.0, 0.0, 1.0])),
            stroke: None,
            opacity: 1.0,
        }
    }
}
```

### 6.2 Paint Types

```rust
#[derive(Debug, Clone)]
pub enum Paint {
    /// Solid color [R, G, B, A] in 0.0-1.0 range
    Solid([f32; 4]),

    // Future: LinearGradient, RadialGradient, Pattern
}
```

### 6.3 Stroke Properties

```rust
#[derive(Debug, Clone)]
pub struct Stroke {
    /// Stroke width in local units
    pub width: f32,

    /// Stroke paint
    pub paint: Paint,

    // Future: line cap, line join, dash pattern
}
```

---

## 7. Layout System

### 7.1 Layout Modes

```rust
#[derive(Debug, Clone)]
pub enum Layout {
    /// Manual positioning via transform
    Free,

    /// Automatic child positioning
    Auto(AutoLayout),
}
```

### 7.2 Auto-Layout Configuration

```rust
#[derive(Debug, Clone)]
pub struct AutoLayout {
    /// Stack direction
    pub direction: Direction,

    /// Space between children
    pub spacing: f32,

    /// Inner padding
    pub padding: Edges,

    /// Main axis alignment
    pub align_main: AlignMain,

    /// Cross axis alignment
    pub align_cross: AlignCross,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy)]
pub enum AlignMain {
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Debug, Clone, Copy)]
pub enum AlignCross {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Edges {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
```

### 7.3 Layout Algorithm

For each group with `Layout::Auto`:

1. **Measure**: Compute intrinsic size of each child
   - Shapes: from path bounds
   - Text: from text metrics (via `TextEngine` trait)
   - Groups: recursive measurement

2. **Position**: Place children along main axis
   - Apply padding
   - Distribute spacing
   - Apply main axis alignment

3. **Align**: Apply cross-axis alignment to each child

4. **Output**: Set layout offset for each child (separate from author transform)

---

## 8. Text System

### 8.1 Text Data

```rust
#[derive(Debug, Clone)]
pub struct TextData {
    /// Text content (UTF-8)
    pub content: String,

    /// Font specification
    pub font: FontSpec,

    /// Font size in local units
    pub font_size: f32,

    /// Line height multiplier
    pub line_height: f32,

    /// Text alignment within bounds
    pub align: TextAlign,

    /// Auto-width or fixed-width wrapping behavior
    pub sizing: TextSizing,
}

#[derive(Debug, Clone, Copy)]
pub enum TextSizing {
    /// The text box grows to the measured line width.
    AutoWidth,

    /// Text wraps to an explicit local-space width.
    FixedWidth { width: f32 },
}

#[derive(Debug, Clone)]
pub struct FontSpec {
    /// Font family name
    pub family: String,

    /// Weight (100-900, 400 = normal, 700 = bold)
    pub weight: u16,

    /// Italic flag
    pub italic: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}
```

### 8.2 Text Engine Trait

Text rendering is abstracted to support different backends:

```rust
pub trait TextEngine {
    /// Measure text bounds
    fn measure(&self, text: &TextData) -> Rect;

    /// Shape text into positioned glyphs
    fn shape(&self, text: &TextData) -> Vec<GlyphRun>;
}

#[derive(Debug, Clone)]
pub struct GlyphRun {
    pub glyphs: Vec<PositionedGlyph>,
    pub font_id: FontId,
}

#[derive(Debug, Clone, Copy)]
pub struct PositionedGlyph {
    pub glyph_id: u16,
    pub position: Vec2,
}
```

**Platform implementations:**
- Core: deterministic fallback metrics and line wrapping
- GPUI: native shaping, caret geometry, IME composition, and rasterization
- SVG export: resolved line positions serialized as `<text>` elements

---

## 9. Query System

Queries are read-only functions that compute derived data from the document.

### 9.1 Core Queries

```rust
impl Document {
    /// Get world transform for a node (cached)
    pub fn world_transform(&mut self, id: NodeId) -> Affine2;

    /// Get local bounds (in node's coordinate space)
    /// - Group: union of children bounds
    /// - Frame: explicit width × height (independent of children)
    /// - Shape: path bounding box
    /// - Text: text metrics
    pub fn local_bounds(&self, id: NodeId) -> Rect;

    /// Get world bounds (axis-aligned in world space)
    pub fn world_bounds(&mut self, id: NodeId) -> Rect;

    /// Hit test: find topmost node at world position
    pub fn hit_test(&mut self, world_pos: Vec2) -> Option<NodeId>;

    /// Get snap points for alignment (corners, centers, baselines)
    pub fn snap_points(&self, id: NodeId) -> Vec<SnapPoint>;
}
```

### 9.2 Traversal

```rust
impl Document {
    /// Iterate nodes in paint order (DFS, children in order)
    pub fn paint_order(&self) -> impl Iterator<Item = NodeId>;

    /// Iterate nodes in reverse paint order (for hit testing)
    pub fn reverse_paint_order(&self) -> impl Iterator<Item = NodeId>;

    /// Get all ancestors of a node
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId>;

    /// Get all descendants of a node
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId>;
}
```

### 9.3 Caching Strategy

- **Lazy evaluation**: Caches populated on first query
- **Dirty flags**: Mutations set flags; caches invalidated selectively
- **Incremental update**: During drags, only update affected subtree

```rust
impl Document {
    pub fn mark_transform_dirty(&mut self, id: NodeId) {
        // Mark this node and all descendants
        self.dirty.transforms = true;
        // Could be more granular with per-node dirty tracking
    }

    pub fn ensure_transforms_valid(&mut self) {
        if self.dirty.transforms {
            self.recompute_world_transforms();
            self.dirty.transforms = false;
        }
    }
}
```

---

## 10. Editing Model

### 10.1 Patch-Based Undo/Redo

Avoid storing full document snapshots. Use small patches:

```rust
#[derive(Debug, Clone)]
pub enum Patch {
    SetTransform {
        id: NodeId,
        before: Affine2,
        after: Affine2,
    },
    SetStyle {
        id: NodeId,
        before: Style,
        after: Style,
    },
    SetText {
        id: NodeId,
        before: TextData,
        after: TextData,
    },
    SetPath {
        id: NodeId,
        before: PathData,
        after: PathData,
    },
    Reparent {
        id: NodeId,
        before_parent: Option<NodeId>,
        after_parent: Option<NodeId>,
        before_index: usize,
        after_index: usize,
    },
    CreateNode {
        id: NodeId,
        node: Node,
        parent: NodeId,
        index: usize,
    },
    DeleteNode {
        id: NodeId,
        node: Node,  // stored for undo
        parent: NodeId,
        index: usize,
    },
}
```

### 10.2 Commands

```rust
#[derive(Debug, Clone)]
pub struct Command {
    /// Human-readable description
    pub description: String,

    /// Patches to apply (in order)
    pub patches: Vec<Patch>,
}

impl Command {
    pub fn apply(&self, doc: &mut Document) {
        for patch in &self.patches {
            patch.apply_forward(doc);
        }
    }

    pub fn unapply(&self, doc: &mut Document) {
        for patch in self.patches.iter().rev() {
            patch.apply_backward(doc);
        }
    }
}
```

### 10.3 History Stack

```rust
#[derive(Debug, Default)]
pub struct History {
    pub undo_stack: Vec<Command>,
    pub redo_stack: Vec<Command>,
}

impl History {
    pub fn push(&mut self, cmd: Command) {
        self.undo_stack.push(cmd);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.undo_stack.pop() {
            cmd.unapply(doc);
            self.redo_stack.push(cmd);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.redo_stack.pop() {
            cmd.apply(doc);
            self.undo_stack.push(cmd);
            true
        } else {
            false
        }
    }
}
```

### 10.4 Transactions (Interactive Edits)

For drag operations, use transactions to coalesce updates:

```rust
#[derive(Debug)]
pub struct Transaction {
    /// In-progress patches (not yet committed)
    patches: Vec<Patch>,

    /// Original values (for updating "after" during drag)
    originals: HashMap<NodeId, Affine2>,
}

impl Transaction {
    pub fn start(selection: &[NodeId], doc: &Document) -> Self {
        let originals = selection
            .iter()
            .map(|&id| (id, doc.nodes[id].transform))
            .collect();
        Self { patches: vec![], originals }
    }

    pub fn update(&mut self, id: NodeId, new_transform: Affine2, doc: &mut Document) {
        // Update document directly (preview)
        doc.nodes[id].transform = new_transform;
        doc.mark_transform_dirty(id);
    }

    pub fn commit(self, doc: &Document) -> Command {
        // Build patches from originals → current
        let patches = self.originals
            .into_iter()
            .map(|(id, before)| Patch::SetTransform {
                id,
                before,
                after: doc.nodes[id].transform,
            })
            .collect();

        Command {
            description: "Move".into(),
            patches,
        }
    }

    pub fn cancel(self, doc: &mut Document) {
        // Restore original values
        for (id, transform) in self.originals {
            doc.nodes[id].transform = transform;
            doc.mark_transform_dirty(id);
        }
    }
}
```

---

## 11. Rendering System

### 11.1 Display List

Backend-agnostic intermediate representation:

```rust
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

#[derive(Debug, Clone)]
pub enum DisplayItem {
    FillPath {
        path: PathData,
        paint: Paint,
        transform: Affine2,
        opacity: f32,
    },
    StrokePath {
        path: PathData,
        stroke: Stroke,
        transform: Affine2,
        opacity: f32,
    },
    Text {
        text: TextItem,
        transform: Affine2,
        opacity: f32,
    },
    // Future: explicit masks, blend modes, filters
}

#[derive(Debug, Clone)]
pub struct TextItem {
    pub content: String,
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_italic: bool,
    pub fill: Paint,
}
```

### 11.2 Display List Generation

```rust
impl Document {
    pub fn build_display_list(&mut self, view: &View) -> DisplayList {
        let mut items = Vec::new();

        for id in self.paint_order() {
            let node = &self.nodes[id];
            if !node.visible {
                continue;
            }

            let world_transform = self.world_transform(id);
            let screen_transform = view.screen_from_world() * world_transform;
            let opacity = self.compute_opacity_chain(id);

            match &node.kind {
                NodeKind::Group => {
                    // Groups don't render directly (children handle it)
                }
                NodeKind::Frame(frame) => {
                    // Render background if present
                    if let Some(bg) = &frame.background {
                        items.push(DisplayItem::FillPath {
                            path: PathData::rect(frame.width, frame.height),
                            paint: bg.clone(),
                            transform: screen_transform,
                            opacity,
                        });
                    }
                    // Frames are unclipped artboards.
                }
                NodeKind::Shape(path) => {
                    if let Some(fill) = &node.style.fill {
                        items.push(DisplayItem::FillPath {
                            path: path.clone(),
                            paint: fill.clone(),
                            transform: screen_transform,
                            opacity,
                        });
                    }
                    if let Some(stroke) = &node.style.stroke {
                        items.push(DisplayItem::StrokePath {
                            path: path.clone(),
                            stroke: stroke.clone(),
                            transform: screen_transform,
                            opacity,
                        });
                    }
                }
                NodeKind::Text(text) => {
                    if let Some(fill) = &node.style.fill {
                        items.push(DisplayItem::Text {
                            text: TextItem {
                                content: text.content.clone(),
                                font_family: text.font.family.clone(),
                                font_size: text.font_size,
                                font_weight: text.font.weight,
                                font_italic: text.font.italic,
                                fill: fill.clone(),
                            },
                            transform: screen_transform,
                            opacity,
                        });
                    }
                }
            }
        }

        DisplayList { items }
    }
}
```

### 11.3 SVG Backend (`render_svg`)

For file export and interchange:

```rust
pub fn to_svg_string(display_list: &DisplayList) -> String {
    let mut svg = String::new();

    for (index, item) in display_list.items.iter().enumerate() {
        match item {
            DisplayItem::FillPath { path, paint, transform, opacity } => {
                let d = path_to_d_attribute(path);
                let fill = paint_to_css(paint);
                let matrix = transform_to_matrix(transform);
                svg.push_str(&format!(
                    r#"<path data-node-id="{}" d="{}" fill="{}" opacity="{}" transform="{}"/>"#,
                    index, d, fill, opacity, matrix
                ));
            }
            DisplayItem::Text { text, transform, opacity } => {
                let fill = paint_to_css(&text.fill);
                let matrix = transform_to_matrix(transform);
                svg.push_str(&format!(
                    r#"<text data-node-id="{}" font-family="{}" font-size="{}" fill="{}" opacity="{}" transform="{}">{}</text>"#,
                    index, text.font_family, text.font_size, fill, opacity, matrix,
                    html_escape(&text.content)
                ));
            }
            // ... stroke, etc.
        }
    }

    svg
}

fn path_to_d_attribute(path: &PathData) -> String {
    let mut d = String::new();
    for cmd in &path.commands {
        match cmd {
            PathCmd::MoveTo(p) => write!(d, "M{} {} ", p.x, p.y).unwrap(),
            PathCmd::LineTo(p) => write!(d, "L{} {} ", p.x, p.y).unwrap(),
            PathCmd::CubicTo { c1, c2, p } => {
                write!(d, "C{} {} {} {} {} {} ", c1.x, c1.y, c2.x, c2.y, p.x, p.y).unwrap()
            }
            PathCmd::Close => d.push_str("Z "),
        }
    }
    d
}
```

---

## 12. Input System

### 12.1 Platform-Agnostic Events

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,  // Cmd on Mac, Win on Windows
}

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone)]
pub enum EditorEvent {
    PointerDown {
        pointer_id: u64,
        screen_pos: Vec2,
        button: MouseButton,
        mods: Mods,
    },
    PointerMove {
        pointer_id: u64,
        screen_pos: Vec2,
        mods: Mods,
    },
    PointerUp {
        pointer_id: u64,
        screen_pos: Vec2,
        button: MouseButton,
        mods: Mods,
    },
    Scroll {
        delta: Vec2,
        mods: Mods,
    },
    KeyDown {
        key: Key,
        mods: Mods,
    },
    TextInput {
        text: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum Key {
    Escape,
    Delete,
    Backspace,
    Enter,
    Tab,
    Space,
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    // Arrow keys, function keys, etc.
}
```

### 12.2 Effects

Output from event handling:

```rust
#[derive(Debug, Default)]
pub struct Effects {
    /// Request a redraw
    pub request_redraw: bool,

    /// Change cursor appearance
    pub set_cursor: Option<Cursor>,

    /// Copy text to clipboard
    pub copy_to_clipboard: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Cursor {
    Default,
    Hand,
    Move,
    Crosshair,
    Text,
    ResizeNS,
    ResizeEW,
    ResizeNESW,
    ResizeNWSE,
}
```

### 12.3 Editor Entry Point

```rust
pub struct Editor {
    pub document: Document,
    pub history: History,
    pub selection: Selection,
    pub view: View,
    pub tool: ToolState,

    /// In-progress transaction (during drags)
    pub transaction: Option<Transaction>,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            document: Document::new(),
            history: History::default(),
            selection: Selection::default(),
            view: View { pan: Vec2::ZERO, zoom: 1.0 },
            tool: ToolState::Select,
            transaction: None,
        }
    }

    pub fn handle_event(&mut self, event: EditorEvent) -> Effects {
        // Convert screen coordinates to world
        let world_pos = match &event {
            EditorEvent::PointerDown { screen_pos, .. }
            | EditorEvent::PointerMove { screen_pos, .. }
            | EditorEvent::PointerUp { screen_pos, .. } => {
                Some(self.view.world_from_screen().transform_point2(*screen_pos))
            }
            _ => None,
        };

        // Dispatch to current tool
        match &mut self.tool {
            ToolState::Select => self.handle_select_tool(event, world_pos),
            // ... other tools
        }
    }

    pub fn display_list(&mut self) -> DisplayList {
        self.document.build_display_list(&self.view)
    }
}
```

### 12.4 Numeric Property Adjustment

Numeric property steppers and horizontal scrubs MUST use the same adjustment policy.

The unmodified mode uses these steps:

| Property | Fine step |
|---|---:|
| X and Y position | 1 |
| Opacity | 1 percentage point (`0.01` internally) |
| Stroke width | `0.1` |
| Text size | 1 |
| Auto-layout spacing and padding | 1 |

Holding `Shift` MUST select coarse mode. Coarse mode uses these steps:

| Property | Coarse step | Clean quantum |
|---|---:|---:|
| X and Y position | 10 | 1 |
| Opacity | 10 percentage points (`0.1` internally) | 1 percentage point |
| Stroke width | 1 | 1 |
| Text size | 10 | 1 |
| Auto-layout spacing and padding | 10 | 1 |

Coarse mode MUST round the resulting value to its clean quantum before applying the property bounds.
Fine mode MUST preserve fractional results.

The modifier MUST be evaluated for each button click and pointer-move event. Changing `Shift` during a scrub changes the active mode without changing the scrub origin.

A `+` or `−` click and a scrub MUST produce the same property adjustment for the same target, direction, mode, and starting value.

Each completed scrub produces at most one undo entry. Escape cancels the scrub without an undo entry.

Rotation controls retain their dedicated 15-degree step and do not use this policy.

---

## 13. Platform Frontends

### 13.1 GPUI

GPUI is the product frontend. It owns the Figma/Lunacy-style chrome,
native text input bridge, pointer translation, and painting of the shared
display list.

---

## 14. File Format

### 14.1 Native Format (JSON)

Simple JSON serialization of the document:

```json
{
  "version": 2,
  "root": "node_0",
  "nodes": {
    "node_0": {
      "name": "Root",
      "parent": null,
      "children": ["node_1"],
      "kind": "Group",
      "transform": [1, 0, 0, 1, 0, 0],
      "style": { "fill": null, "stroke": null, "opacity": 1 },
      "layout": "Free",
      "visible": true,
      "locked": false
    },
    "node_1": {
      "name": "Icon 24x24",
      "parent": "node_0",
      "children": ["node_2"],
      "kind": {
        "Frame": {
          "width": 24,
          "height": 24,
          "background": { "Solid": [1.0, 1.0, 1.0, 1.0] }
        }
      },
      "transform": [1, 0, 0, 1, 0, 0],
      "style": { "fill": null, "stroke": null, "opacity": 1 },
      "layout": "Free",
      "visible": true,
      "locked": false
    },
    "node_2": {
      "name": "Rectangle",
      "parent": "node_1",
      "children": [],
      "kind": {
        "Shape": {
          "commands": [
            { "MoveTo": [0, 0] },
            { "LineTo": [100, 0] },
            { "LineTo": [100, 100] },
            { "LineTo": [0, 100] },
            "Close"
          ]
        }
      },
      "transform": [1, 0, 0, 1, 50, 50],
      "style": {
        "fill": { "Solid": [0.2, 0.4, 0.8, 1.0] },
        "stroke": null,
        "opacity": 1
      },
      "layout": "Free",
      "visible": true,
      "locked": false
    }
  }
}
```

### 14.2 SVG Export

The native frontend exports a settled snapshot of all effectively visible
artwork. The view box is derived from the artwork bounds, including frame
dimensions and enough stroke margin to avoid clipping visible strokes; viewport
pan, zoom, selections, guides, and tool previews are excluded.

The supported formats are:

- Standard SVG, preserving editable text elements.
- Outlined-text SVG, converting text to paths with the system font database.
- Transparent PNG and lossless WebP raster exports.
- Opaque JPEG export, compositing transparent pixels over white.

The same snapshot and encoders back clipboard image export. macOS exposes SVG,
PNG, and WebP; Windows exposes SVG and PNG; Linux exposes no image-copy commands
because the pinned GPUI backend does not publish image-only clipboard payloads.
The current product does not export a selected frame or provide custom export
presets. Those are product-level features rather than part of the
backend-neutral display-list contract.

### 14.3 SVG Import

The native frontend can open `.svg` files by converting a deliberately limited
subset into editable document nodes. Supported source elements are groups,
paths, rectangles, circles, ellipses, lines, polylines, and polygons. The
conversion preserves affine transforms, visibility, solid fills and strokes,
and per-fill or per-stroke opacity. Shape primitives and quadratic path segments
become native path commands.

Import is strict: the entire operation fails with a descriptive error when the
source contains a feature Strek cannot represent faithfully. Unsupported
features include text, images, gradients, patterns, clipping, masks, filters,
markers, nested SVG documents, CSS style blocks, even-odd fills, dashed strokes,
object/group opacity, and advanced stroke caps, joins, or paint order. Imports
also enforce bounded nesting, node counts, path segments, and geometry attribute
sizes before native document construction. This avoids silently producing
artwork that differs from the source or accepting unreasonable conversion work.

SVG is an interchange format, not a native save format. A successful import is
treated as an unsaved native document: Save prompts for a `.strek.json`
destination and never writes back to the SVG source.

---

## 15. Implementation Milestones

Completed items describe the current implementation. Unchecked items are
intentional product backlog.

### Milestone 1: Core Foundation
- [x] Generational arena with `NodeId`
- [x] Document structure with root node
- [x] Group, frame, shape, and text nodes
- [x] Affine transform system
- [x] Paint-order traversal
- [x] World-transform and bounds caching

### Milestone 2: Basic Editing
- [x] Single, multi, scoped, and marquee selection
- [x] Geometry-aware hit testing
- [x] Move, resize, and rotate transactions
- [x] Patch-based undo/redo
- [x] Group/ungroup operations

### Milestone 3: Rendering
- [x] Display-list generation
- [x] SVG backend
- [x] GPUI canvas renderer

### Milestone 4: GPUI Product Frontend
- [x] Native application shell
- [x] Pointer, keyboard, IME, and clipboard routing
- [x] Layers and design panels
- [x] Menus, tool rail, command palette, and configurable shortcuts
- [x] Native open, save, and export dialogs

### Milestone 5: Text Support
- [x] Semantic `TextData` model
- [x] Deterministic fallback layout
- [x] GPUI-shaped text, caret, selection, and composition
- [x] SVG text export

### Milestone 6: Auto-Layout
- [x] Auto-layout configuration
- [x] Layout computation engine
- [x] Author transforms plus derived layout offsets
- [x] GPUI mode, direction, spacing, and padding controls

### Milestone 7: Snapping & Alignment
- [x] Snap-point generation
- [x] Snapping and guides during move and shape creation
- [x] Align/distribute commands

### Milestone 8: Polish
- [x] Validated, versioned JSON save/load
- [x] SVG and PNG export
- [x] Configurable keyboard shortcuts
- [x] Resizable Layers and Design panels

### Milestone 9: Action System & Command Palette
- [x] `EditorAction` catalog
- [x] Unified command metadata and default shortcuts
- [x] Validated user keymap overrides
- [x] Fuzzy command palette with keyboard navigation
- [x] Context-aware action enablement and execution
- [x] Recent-command ranking

### Milestone 10: Layer Panel
- [x] Layer tree
- [x] Node names and inline rename
- [x] Visibility toggle
- [x] Lock toggle
- [x] Canvas/panel selection synchronization
- [x] Drag-to-reorder within a parent
- [x] Drag-to-reparent into containers
- [x] Expand/collapse containers
- [x] Context menu (duplicate, delete, group, ungroup)

### Milestone 11: Properties Panel
- [x] Transform position, bounds, and rotation controls
- [x] Transform decomposition helpers and translate/rotate/scale/skew readouts
- [x] Validated hexadecimal fill editing and opacity
- [x] Validated stroke color and width editing
- [x] Text size and alignment controls
- [x] Node name and type
- [x] Mixed-value handling for multi-selection
- [x] Font family, weight, and italic controls
- [x] Live preview during value scrubbing with one undo entry on commit
- [x] Numeric values with horizontal drag-to-adjust and Escape cancellation
- [x] Shift-aware fine/coarse steps shared by numeric steppers and scrubs
- [x] Direct typed entry for position, opacity, stroke width, text size, spacing,
  and uniform padding without removing scrub or stepper interaction
- [x] Compatible repeated keyboard nudges coalesced into one undoable movement

### Milestone 12: Drawing Tools
- [x] Rectangle tool with constrained and centered drawing
- [x] Ellipse tool with constrained and centered drawing
- [x] Line tool with 45-degree and centered drawing
- [x] Pen tool with anchor and handle editing
- [x] Text tool with point and fixed-width creation
- [x] Context bar with per-tool fill and stroke creation defaults
- [x] Direct vector editing for existing paths

### Milestone 13: Toolbar & Status Bar
- [x] Tool buttons with icons and keyboard hints
- [x] Active-tool state
- [x] Fill and stroke editors
- [x] Direct zoom buttons, percentage input, and menu
- [x] Selection, cursor, interaction, and zoom status
- [x] Tooltips

### Milestone 14: Export & File Operations
- [x] Native open/save dialogs
- [x] Validated JSON documents with version migration
- [x] Artwork-bounded SVG export
- [x] Outlined-text SVG export
- [x] Transparent PNG export
- [x] Opaque JPEG and lossless WebP export
- [x] Copy visible artwork in platform-supported SVG, PNG, or WebP formats
- [x] Strict editable SVG subset import
- [x] Recent files
- [x] Dirty-state tracking and unsaved-change prompts
- [ ] Automatic SVG font embedding (outlined text is the portable alternative)

### Milestone 15: Automation & MCP
- [x] Session-stable opaque layer identifiers and document-tree inspection
- [x] Selection by layer identifier and semantic layer-property updates
- [x] Direct fill, stroke, and numeric-property operations
- [x] Explicit-path new, open, save, and export operations
- [x] Inline SVG, PNG, JPEG, and WebP artifact responses
- [x] Matching command-socket, CLI, AppleScript, and MCP access

### Product backlog

The following candidates are intentionally not marked complete. They are
ordered roughly by how much they improve the core logo/icon workflow and file
interoperability.

Important next candidates:

- Editable dimensions, rotation, scale, and skew controls, including direct
  numeric entry; typed entry is complete for the inspector's existing semantic
  numeric controls
- Exporting a selected frame or selection, with explicit dimensions and reusable
  export presets
- Autosave and crash recovery for dirty documents
- Boolean path operations and a more complete path/stroke editing workflow

Nice-to-have candidates:

- Gradients and advanced stroke controls such as caps, joins, and dash patterns
- Actual cross-axis stretch and size constraints in auto layout

### Implemented: Canvas precision controls

Status: implemented in native format version 3. The comparative rationale and
source links are in
[`docs/PRECISION_AND_COLOR_LIBRARY_RESEARCH.md`](docs/PRECISION_AND_COLOR_LIBRARY_RESEARCH.md).

#### Vocabulary and scope

- A **grid** is a repeating, non-exporting alignment lattice in document
  coordinates.
- **Rulers** are non-exporting viewport chrome showing document coordinates.
- A **guide** is a persistent document-owned horizontal or vertical line.
- A **smart guide** is temporary feedback produced by object or guide snapping.
- **Snapping** is the screen-tolerance correction applied during an edit.

Version one adds one square document grid, pixel rulers, document-wide guides,
and coarse snap-target controls. It does not add layout columns/rows, per-frame
precision aids, or specialist construction geometry.

#### Ownership and persistence

Document-owned state:

```text
GridSettings
  spacing: finite positive document units
  major_every: integer in 1..=10

Guide
  id: stable GuideId
  axis: Horizontal | Vertical
  position: finite document coordinate
```

The grid definition and guide collection are saved, validated, dirty-tracked,
and undoable. Guide drags coalesce into one history entry.

Workspace preferences are not saved in the document and do not enter undo:

- show rulers;
- show grid;
- show guides;
- lock guides;
- master snapping;
- snap to Objects, Guides, and Grid;
- screen-space tolerance.

Visibility and snap eligibility are independent. A hidden grid or hidden guide
collection may remain a snap target, but the precision popover must make that
state explicit. Rulers and the grid use a fixed document origin of `(0, 0)` in
version one.

The native format is version 3.
Version 1 and 2 files load with default grid settings and no guides. Validation
must reject non-finite positions, invalid spacing, duplicate IDs, and more than
10,000 guides.

#### Grid and ruler rendering

- Grid spacing stays fixed in document units at every zoom.
- Minor lines are skipped when their screen separation becomes too dense;
  major lines remain until they also need adaptive skipping.
- Rendering suppression does not change logical snap locations.
- Grid and ruler density is bounded by the viewport, not document extent.
- Ruler tick intervals adapt to zoom and show pixel/document-unit labels.
- The grid is the bottommost canvas layer: artwork paints over it, while guides,
  rulers, selection chrome, and interactive feedback paint above artwork.
- Grid lines, rulers, and guides are editor chrome and never enter artwork
  snapshots, SVG, raster export, or clipboard images.
- Grid colors and opacity follow the application theme in version one rather
  than becoming document settings.

#### Guide interaction

- Drag from the top ruler to create a horizontal guide.
- Drag from the left ruler to create a vertical guide.
- Drag an unlocked guide to reposition it.
- Option-drag duplicates a guide.
- Double-click a guide's ruler marker to enter its coordinate exactly.
- Delete or Backspace removes the active guide.
- Dragging a guide back over its source ruler removes it.
- View commands show/hide rulers, grid, and guides; lock/unlock guides; and clear
  all guides.
- Clearing all guides commits one undo entry, so it can be reversed without an
  interrupting confirmation.
- Guide creation, movement, duplication, and deletion use semantic core commands
  rather than direct GPUI mutation.

Guides do not participate in layer selection and do not appear in the Layers
panel. GPUI may keep an active-guide ID for keyboard/context actions, but that is
ephemeral workspace state.

#### Snapping behavior

The toolbar uses a split magnet control: the primary action toggles master
snapping and the disclosure opens a compact popover containing:

```text
Snapping                 On
  Objects                 On
  Guides                  On
  Grid                    Off
Tolerance               8 px
-----------------------------
Show rulers
Show grid
Show guides
Lock guides
Grid settings…
Clear guides
```

The tolerance defaults to the existing 8 screen pixels and is constrained to
1–32 pixels. Holding Control during a pointer interaction temporarily bypasses
all snapping without changing the stored master setting.

For each axis, the snap engine chooses the eligible correction with the smallest
screen-space distance. Effective ties use `guide > object > grid`; remaining
ties use stable target order. X and Y are resolved independently so different
targets may win on each axis. Smart-guide feedback identifies the winning
target rather than every eligible candidate.

The first release must apply these controls to:

- moving a selection;
- resizing a selection;
- rectangle, ellipse, line, and frame creation.

Pen-anchor and direct-vector snapping may follow under the Objects category once
their candidate and handle semantics are specified. Rotation, equal-spacing,
tangent, perpendicular, path-intersection, and pixel-fitting snaps are deferred.

#### Commands and automation

All visible controls must have command-palette equivalents. Automation should
gain semantic operations to:

- inspect grid, guides, visibility, and snap settings;
- update grid spacing and major interval;
- create, move, and remove guides by stable ID;
- set visibility, guide lock, snap categories, and tolerance.

Pointer automation remains useful for testing ruler drags, but is not the only
way to manage precision data.

#### Acceptance criteria

- Grid and guide edits round-trip through native files and undo/redo.
- View/snap toggles neither dirty the document nor enter history.
- Old files migrate with safe defaults.
- Snapping remains consistent in screen pixels across zoom levels.
- Competing targets resolve deterministically and Control bypasses all targets.
- Dense grids remain bounded and readable while panning and zooming.
- Guides are keyboard-editable and can be managed without precise dragging.
- No precision overlay appears in any export or clipboard artifact.

#### Explicitly deferred

- row, column, baseline, isometric, angular, modular, and multiple grids;
- per-frame grids/guides and frame-local ruler origins;
- custom ruler/grid origin and alternate measurement units;
- rotated, named, colored, or individually locked guides;
- arbitrary artwork converted to guides;
- snap presets and candidate-count algorithms;
- equal-gap/size, tangent, perpendicular, glyph, and full path-geometry snaps;
- separate pixel fitting or whole-pixel transform enforcement.

### Implemented: Document Color Library

Status: implemented in native format version 3. “Color manager” is the informal
workflow name; the product vocabulary is:

- **Document Color Library**: the complete document-local data set;
- **Color Library**: the modeless manager panel;
- **Saved Colors**: the picker section;
- **Saved Color**: one reusable solid RGBA value;
- **Color Group**: an ordered, collapsible organizational container;
- **swatch**: the visual sample for a color, not a competing feature name;
- **Color Variable**: reserved for a future linked value that updates artwork.

#### Semantics

Saved Colors are deliberately unlinked and copy-by-value:

- applying one copies its RGBA value into the target property;
- editing one changes only the library entry;
- deleting one changes only the library entry;
- existing artwork never depends on the entry or its group;
- direct color edits never need a detach operation.

This behavior is required even when an artwork color exactly matches a Saved
Color. Linked colors, aliases, and themes require a separate Color Variables
specification and data model.

Version one stores document-local solid sRGB RGBA values. It does not store
gradients, patterns, color-space formulas, or application-wide/shared palettes.

#### Data model and persistence

```text
ColorLibrary
  groups: ordered ColorGroup records
  colors: SavedColor records

ColorGroup
  id: stable ColorGroupId
  name: non-empty string
  manual_order: integer
  sort_mode: Manual | Name | HueAndShades | Lightness | Chroma | Brightness
  sort_direction: Ascending | Descending

SavedColor
  id: stable SavedColorId
  group_id: optional ColorGroupId
  name: optional non-empty string
  rgba: four finite normalized components
  manual_order: integer within its group or Ungrouped
```

Ungrouped is a virtual built-in section, not a deletable `ColorGroup`. Empty or
whitespace-only color names normalize to `None`; their fallback label is
uppercase HEX including alpha when it is not fully opaque.

Group and Saved Color mutations are document edits and are undoable. The chosen
sort mode is saved because it defines the library's authored organization.
Manual order is retained while another sort mode is active.

The Color Library ships in the native version 3 metadata migration
as precision aids. Version 1 and 2 files receive an empty library. Validation
limits are 256 groups, 10,000 Saved Colors, and 256 UTF-8 bytes per name, with
valid IDs, group references, orders, and normalized finite color components.
SVG import starts with an empty library; export does not serialize the library.

Panel geometry, collapsed groups, the last-used group, and picker presentation
are workspace preferences. Changing them does not dirty the document.

#### Sort behavior

Sort modes are defined rather than delegated to UI ordering:

- **Manual** uses group order and each color's stored `manual_order`.
- **Name** compares names case-insensitively; unnamed colors follow named colors
  and compare by displayed HEX.
- **Hue & Shades** compares OKLCH hue, then OKLCH lightness. Low-chroma colors
  are grouped as neutrals rather than receiving unstable hue positions.
- **Lightness** compares OKLCH `L`.
- **Chroma** compares OKLCH `C` and is described in UI help as color intensity.
- **Brightness** compares sRGB relative luminance.

All computed sorts are stable, using manual order and then stable ID as final
ties. Alpha is a preceding tie breaker where the visible RGB color is equal.

Changing sort mode never overwrites manual order. Dragging or editing an Order
value while a computed sort is active switches that group to Manual, captures
the currently displayed order, and then applies the requested move. The manual
order is normalized to contiguous zero-based values internally; the panel shows
1-based Order values.

#### Shared picker integration

Every fill/stroke selection surface and every creation-style color surface must
continue to use the shared color-picker component. The picker adds:

- a searchable Saved Colors section grouped like the library;
- typeahead matching optional name, group name, HEX, and RGB value;
- keyboard navigation and Enter to apply;
- a highlight for an exact RGBA match;
- a one-click **Save Color** action beside the current color;
- **Manage Color Library…** to open or focus the manager.

Save Color immediately adds an unnamed entry to the last-used group, or
Ungrouped when none exists. It leaves the picker open and offers a lightweight
confirmation with a secondary Rename action. If an exact RGBA entry already
exists, Save Color reveals that entry instead of silently adding a duplicate.
The manager provides an explicit Duplicate action when duplicates are desired.

The active target—selection fill, selection stroke, frame background, creation
fill, or creation stroke—must remain visible in the picker. Future color
properties must adopt the same component rather than implementing their own
Saved Colors lookup.

#### Color Library panel

The Color Library is a modeless, movable, resizable in-app utility panel. It is
not a separate operating-system window and does not require a docking framework
in version one. Opening its command focuses an existing panel before creating a
new presentation. Its position and size are restored on the next launch and
clamped into the available window.

Each collapsible group presents a compact table:

| Column | Interaction |
| --- | --- |
| Order/drag | Drag or type a 1-based position in Manual mode |
| Color | Click the swatch to edit with the shared picker |
| Name | Edit inline; blank is valid |
| Value | Edit validated HEX/RGBA text inline |
| Actions | Duplicate, move to group, or remove |

Enter commits inline edits and Escape cancels them. Invalid color text remains
in the editor with an error and cannot commit. Panel rows and controls support
keyboard traversal without relying on drag-and-drop.

Groups can be added, renamed, collapsed, reordered, sorted independently, and
removed. Removing a non-empty group moves its entries to Ungrouped by default.
Bulk deletion of a group and all of its colors is deferred; colors can be
removed individually. Removing a Saved Color never searches for or modifies
matching artwork.

#### Commands and automation

The command palette exposes Toggle Color Library and Save Current Color.
Semantic automation supports inspection, add/update/remove/apply operations,
group management, and opening the panel. Further reorder/duplicate operations
remain available through the native panel and core API. The semantic surface
uses IDs rather than names because names are optional and not unique.

The complete core API supports:

- inspect the complete library with stable group/color IDs;
- add, update, duplicate, move, reorder, and remove a Saved Color;
- add, rename, reorder, sort, and remove a Color Group;
- apply a Saved Color by ID to any supported color target;
- open/focus the manager and picker for interaction testing.

#### Acceptance criteria

- Saved Colors and groups round-trip, validate, migrate, undo, and redo.
- Applying a Saved Color copies its value and creates no hidden linkage.
- Editing/deleting entries and groups never changes existing artwork.
- Quick add and search work from every current color-selection context.
- Empty names, duplicate explicit entries, and alpha values behave consistently.
- Manual order survives computed sorting until the user explicitly reorders.
- Perceptual sorts are deterministic across launches and platforms.
- The manager remains usable by keyboard and remembers non-document UI state.
- Color Library data never appears in SVG/raster output.

#### Explicitly deferred

- linked Color Variables/global colors, aliases, modes, and themes;
- application, project, team, cloud, or operating-system libraries;
- gradients, patterns, images, spot colors, CMYK, Lab, and print separations;
- ASE/GPL/ACB or proprietary palette import/export;
- image palette extraction, harmony generation, and bulk recoloring;
- automatic “document colors,” usage counting, and unused-color cleanup;
- docking infrastructure and multiple simultaneous manager windows.

---

## Appendix A: Glossary

| Term | Definition |
|------|------------|
| **Affine Transform** | 2D transformation preserving straight lines (translate, rotate, scale, shear) |
| **DFS** | Depth-First Search; tree traversal visiting children before siblings |
| **Display List** | Intermediate rendering representation; list of draw commands |
| **Generational Arena** | Data structure with stable IDs that detect stale references |
| **Hit Test** | Finding which node is under a given point |
| **Node** | Element in the scene graph (group, shape, or text) |
| **Patch** | Atomic change to the document (for undo/redo) |
| **Query** | Read-only function computing derived data |
| **Scene Graph** | Tree structure representing the document |
| **Transaction** | In-progress edit session (e.g., during a drag) |
| **World Space** | Coordinate system of the document (after all transforms) |
| **Screen Space** | Coordinate system of the display (pixels) |

---

## Appendix B: Dependencies

```toml
# Core dependencies
glam = "0.29"           # Math (Vec2, Affine2)
slotmap = "1.0"         # Generational arena
serde = "1.0"           # Serialization
serde_json = "1.0"      # JSON format

# Product frontend
gpui = "0.1"
```
