# Vector Editor Specification

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
│              (winit/wgpu kept as a test harness)               │
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
│                    RENDER BACKENDS                              │
│  ┌─────────────────────┐       ┌─────────────────────┐          │
│  │    render_wgpu      │       │     render_svg      │          │
│  │  (GPU triangles)    │       │  (SVG string/DOM)   │          │
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

#### B. Edit Layer (`editor_core::edit`)
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
vector-editor/
├── Cargo.toml                    # Workspace manifest
├── SPEC.md                       # This document
├── crates/
│   ├── editor_core/              # Document + tools + undo + snapping + layout
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs            # Editor entry point
│   │       ├── document.rs       # Document struct, node storage
│   │       ├── node.rs           # Node types and properties
│   │       ├── style.rs          # Paint, stroke, style
│   │       ├── path.rs           # Path commands and data
│   │       ├── text.rs           # Text data and font spec
│   │       ├── layout.rs         # Auto-layout types and engine
│   │       ├── transform.rs      # Affine transforms, view
│   │       ├── query.rs          # Bounds, hit-test, traversal
│   │       ├── cache.rs          # Derived data caching
│   │       ├── selection.rs      # Selection state
│   │       ├── tools/            # Tool implementations
│   │       │   ├── mod.rs
│   │       │   ├── select.rs
│   │       │   ├── move_tool.rs
│   │       │   └── text_edit.rs
│   │       ├── command.rs        # Command/patch system
│   │       ├── history.rs        # Undo/redo stack
│   │       ├── input.rs          # Platform-agnostic input events
│   │       ├── effects.rs        # Output effects (redraw, cursor)
│   │       └── view.rs           # Camera/viewport state
│   │
│   ├── editor_render/            # Display list IR (backend-agnostic)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs            # DisplayList, DisplayItem types
│   │
│   ├── render_wgpu/              # Desktop GPU renderer
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs            # wgpu + lyon tessellation
│   │
│   └── render_svg/               # SVG export renderer
│       ├── Cargo.toml
│       └── src/
│           └── lib.rs            # SVG string generation
│
└── apps/
    ├── gpui/                     # Primary product UI
    │   ├── Cargo.toml
    │   └── src/
    │       └── main.rs           # GPUI shell, menus, native input
    │
    └── desktop/                  # Legacy wgpu renderer harness
    │   ├── Cargo.toml
    │   └── src/
    │       └── main.rs           # Event loop, window management
```

### Cargo Workspace Configuration

```toml
# Cargo.toml (root)
[workspace]
members = [
    "crates/editor_core",
    "crates/editor_render",
    "crates/render_wgpu",
    "crates/render_svg",
    "apps/desktop",
    "apps/gpui",
]
resolver = "2"

[workspace.dependencies]
glam = "0.29"
slotmap = "1.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
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

Frames are containers with explicit dimensions, useful for defining artboards and export viewports:

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
- **Frame**: Explicit width/height; optional clipping and background

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
- Desktop: `rustybuzz`/`harfbuzz` for shaping, custom or `fontdue` for rasterization
- Web: Browser handles text via SVG `<text>` elements

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
                    // Frames are unclipped artboards by default.
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

### 11.3 wgpu Backend (`render_wgpu`)

For desktop:

1. **Path tessellation**: Use `lyon` to convert paths to triangles
2. **GPU rendering**: Draw tessellated meshes with wgpu
3. **Text**: Rasterize glyphs to texture atlas, draw as quads

```rust
pub struct WgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    // ... pipeline, buffers, etc.
}

impl WgpuRenderer {
    pub fn render(&mut self, display_list: &DisplayList, surface: &wgpu::Surface) {
        // Tessellate paths, build vertex buffers, render
    }
}
```

### 11.4 SVG Backend (`render_svg`)

For web:

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

---

## 13. Platform Frontends

### 13.1 Desktop (winit)

```rust
// apps/desktop/src/main.rs
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use editor_core::{Editor, EditorEvent};

fn main() {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Vector Editor")
        .build(&event_loop)
        .unwrap();

    let mut editor = Editor::new();
    // let mut renderer = render_wgpu::Renderer::new(&window);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event, .. } => {
                if let Some(editor_event) = translate_event(event) {
                    let fx = editor.handle_event(editor_event);
                    if fx.request_redraw {
                        window.request_redraw();
                    }
                }
            }
            Event::RedrawRequested(_) => {
                let display_list = editor.display_list();
                // renderer.render(&display_list);
            }
            _ => {}
        }
    });
}
```

### 13.2 GPUI

GPUI is the primary product frontend. It owns the Figma/Lunacy-style chrome,
native text input bridge, pointer translation, and painting of the shared
display list. The winit/wgpu app remains a renderer harness only.

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

Generate standard SVG for interchange. Optionally export a specific Frame for precise viewport control:

```rust
/// Export document or a specific frame to SVG
/// If frame_id is provided, uses the frame's dimensions as the viewBox
pub fn export_svg(doc: &Document, frame_id: Option<NodeId>) -> String {
    let (bounds, root_id) = match frame_id {
        Some(id) => {
            // Export specific frame with its explicit dimensions
            let frame = doc.nodes[id].kind.as_frame().unwrap();
            (Rect::new(Vec2::ZERO, Vec2::new(frame.width, frame.height)), id)
        }
        None => {
            // Fall back to root bounds
            (doc.world_bounds(doc.root), doc.root)
        }
    };

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}">"#,
        bounds.min.x, bounds.min.y,
        bounds.width(), bounds.height()
    );

    fn export_node(doc: &Document, id: NodeId, svg: &mut String) {
        let node = &doc.nodes[id];
        if !node.visible { return; }

        match &node.kind {
            NodeKind::Group => {
                svg.push_str(&format!(r#"<g transform="{}">"#,
                    transform_to_svg(&node.transform)));
                for &child in &node.children {
                    export_node(doc, child, svg);
                }
                svg.push_str("</g>");
            }
            NodeKind::Frame(frame) => {
                // Render background rect if present
                if let Some(bg) = &frame.background {
                    svg.push_str(&format!(
                        r#"<rect width="{}" height="{}" fill="{}"/>"#,
                        frame.width, frame.height, paint_to_css(bg)
                    ));
                }
                for &child in &node.children {
                    export_node(doc, child, svg);
                }
            }
            NodeKind::Shape(path) => {
                // <path d="..." fill="..." stroke="..." transform="..."/>
            }
            NodeKind::Text(text) => {
                // <text ...>content</text>
            }
        }
    }

    export_node(doc, root_id, &mut svg);
    svg.push_str("</svg>");
    svg
}
```

---

## 15. Implementation Milestones

### Milestone 1: Core Foundation
- [ ] Generational arena with `NodeId`
- [ ] Document structure with root node
- [ ] Node types: Group, Shape (PathData)
- [ ] Basic transform system
- [ ] DFS traversal (paint order)
- [ ] World transform computation and caching

### Milestone 2: Basic Editing
- [ ] Selection model (single and multi-select)
- [ ] Hit testing
- [ ] Move tool with transactions
- [ ] Patch-based undo/redo
- [ ] Group/ungroup operations

### Milestone 3: Rendering
- [ ] DisplayList generation
- [ ] SVG backend (string output)
- [ ] Basic wgpu backend (path tessellation)

### Milestone 4: Web Frontend
- [ ] WASM build setup
- [ ] Pointer event handling
- [ ] SVG DOM rendering
- [ ] requestAnimationFrame loop

### Milestone 5: Desktop Frontend
- [ ] winit event loop
- [ ] wgpu renderer integration
- [ ] Cursor management

### Milestone 6: Text Support
- [ ] TextData model
- [ ] TextEngine trait
- [ ] Web: SVG text rendering
- [ ] Desktop: font loading and glyph rendering

### Milestone 7: Auto-Layout
- [ ] AutoLayout configuration
- [ ] Layout computation engine
- [ ] Dual transform model (author + layout)

### Milestone 8: Snapping & Alignment
- [ ] Snap point generation
- [ ] Snapping during drag
- [ ] Align/distribute commands

### Milestone 9: Polish
- [ ] JSON save/load
- [ ] SVG export
- [ ] Keyboard shortcuts
- [ ] Layers panel (basic UI)

### Milestone 10: Action System & Command Palette
- [ ] `EditorAction` enum cataloging all operations
- [ ] Action registry with metadata (name, description, icon, default shortcut)
- [ ] Key binding system (configurable shortcuts)
- [ ] Command palette UI component (fuzzy search, keyboard navigation)
- [ ] Action execution via `editor.execute_action()`
- [ ] Recent actions tracking

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorAction {
    // Selection
    SelectAll,
    DeselectAll,
    InvertSelection,

    // Edit
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,

    // Arrange
    Group,
    Ungroup,
    BringToFront,
    SendToBack,
    BringForward,
    SendBackward,

    // View
    ZoomIn,
    ZoomOut,
    ZoomToFit,
    ZoomToSelection,
    ResetZoom,

    // Tools
    ToolSelect,
    ToolRectangle,
    ToolEllipse,
    ToolPen,
    ToolText,
}

pub struct ActionMeta {
    pub name: &'static str,
    pub description: &'static str,
    pub default_shortcut: Option<Shortcut>,
    pub category: ActionCategory,
}
```

### Milestone 11: Layer Panel
- [ ] Layer tree view component
- [ ] Node name display and inline rename
- [ ] Visibility toggle (eye icon)
- [ ] Lock toggle (lock icon)
- [ ] Selection sync (click to select, highlight selected)
- [ ] Drag-to-reorder within parent
- [ ] Drag-to-reparent (into groups)
- [ ] Expand/collapse groups
- [ ] Context menu (duplicate, delete, group, ungroup)

```rust
impl Document {
    /// Get layer tree for UI display
    pub fn layer_tree(&self) -> LayerTree;

    /// Reorder child within same parent
    pub fn reorder_child(&mut self, id: NodeId, new_index: usize) -> Patch;
}

pub struct LayerEntry {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKindTag,  // Group, Shape, Text
    pub visible: bool,
    pub locked: bool,
    pub depth: usize,
    pub expanded: bool,     // For groups
    pub selected: bool,
    pub is_primary: bool,
}
```

### Milestone 12: Properties Panel
- [ ] Transform section (X, Y, W, H, Rotation)
- [ ] Transform decomposition helpers (Affine2 → translate/rotate/scale)
- [ ] Fill section (color picker, opacity)
- [ ] Stroke section (color, width, opacity)
- [ ] Text section (font family, size, weight, alignment) — when text selected
- [ ] Node info (name, type badge)
- [ ] Multi-selection: show "Mixed" for differing values
- [ ] Live preview during value scrubbing (uses Transaction)
- [ ] Numeric input with drag-to-adjust

```rust
/// Decomposed transform for UI editing
#[derive(Debug, Clone, Copy)]
pub struct TransformComponents {
    pub x: f32,
    pub y: f32,
    pub width: f32,   // from bounds
    pub height: f32,  // from bounds
    pub rotation: f32, // degrees
    pub scale_x: f32,
    pub scale_y: f32,
}

impl TransformComponents {
    pub fn from_node(doc: &Document, id: NodeId) -> Self;
    pub fn to_affine(&self) -> Affine2;
}

/// Property value that may be mixed across selection
pub enum PropertyValue<T> {
    Single(T),
    Mixed,
    None,  // No selection
}
```

**Properties Panel Layout (minimal Figma-style):**
```
┌─────────────────────────┐
│ ◆ Rectangle            │  <- Node name + type icon
├─────────────────────────┤
│ Position                │
│ X: [120.0]  Y: [80.0]  │
│ W: [200.0]  H: [150.0] │
│ ↻: [0.0°]              │  <- Rotation
├─────────────────────────┤
│ Fill                    │
│ [■ #3366CC] [100%]     │  <- Color swatch + opacity
├─────────────────────────┤
│ Stroke                  │
│ [■ #000000] [1.0px]    │  <- Color + width
└─────────────────────────┘
```

### Milestone 13: Drawing Tools
- [ ] Rectangle tool (drag to create, shift for square)
- [ ] Ellipse tool (drag to create, shift for circle)
- [ ] Line tool
- [ ] Pen tool (click to add points, drag for curves)
- [ ] Text tool (click to place text, then edit)
- [ ] Tool options bar (stroke/fill presets for new shapes)
- [ ] Double-click to enter path edit mode (for existing shapes)

```rust
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Line,
    Pen { points: Vec<PenPoint> },
    Text,
    PathEdit { node: NodeId },
}

pub struct PenPoint {
    pub anchor: Vec2,
    pub handle_in: Option<Vec2>,
    pub handle_out: Option<Vec2>,
}
```

### Milestone 14: Toolbar & Status Bar
- [ ] Tool buttons with icons and keyboard hints
- [ ] Active tool highlight
- [ ] Quick color picker for fill/stroke
- [ ] Zoom control (dropdown + slider)
- [ ] Status bar: selection count, cursor position, zoom level
- [ ] Tooltips on hover

### Milestone 15: Export & File Operations
- [ ] Save/Open dialogs (native file picker integration)
- [ ] JSON document save/load with version migration
- [ ] Export SVG with options (embed fonts, outline text)
- [ ] Export PNG/JPEG rasterization (via wgpu offscreen render)
- [ ] Recent files list
- [ ] Dirty document tracking (unsaved changes warning)

```rust
pub struct SvgExportOptions {
    pub outline_text: bool,      // Convert text to paths
    pub embed_fonts: bool,       // Base64 embed used fonts
    pub minify: bool,            // Remove whitespace
    pub precision: u8,           // Decimal places for coordinates
}

pub struct RasterExportOptions {
    pub format: RasterFormat,    // PNG, JPEG, WebP
    pub scale: f32,              // 1.0 = 1x, 2.0 = 2x (retina)
    pub background: Option<[f32; 4]>, // None = transparent (PNG only)
    pub quality: u8,             // JPEG quality 0-100
}

pub enum RasterFormat {
    Png,
    Jpeg,
    WebP,
}
```

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

# Legacy renderer harness
winit = "0.30"          # Windowing
wgpu = "23.0"           # GPU rendering
lyon = "1.0"            # Path tessellation
```
