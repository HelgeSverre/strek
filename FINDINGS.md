# Findings — Selection / State / Logic Audit

Audit date: 2026-08-31. Scope: full read of `crates/editor_core` (editor.rs, lib.rs, node.rs, command.rs, action.rs, path.rs, layout.rs, render.rs, snap.rs, history.rs) and `apps/gpui` (main.rs, canvas.rs, toolbar.rs, commands.rs, layer_panel.rs, properties_panel.rs, precision_ui.rs, layer_name_input.rs, color_picker.rs, document_io.rs, svg_import.rs, export.rs, automation*, mcp.rs, render_svg). Behavior cross-referenced against Figma and Lunacy conventions. Only issues verified in code are listed.

---

## High

### 1. Pending numeric property input commits to whichever node is selected at Enter-time, not the node it was opened for

- **Where:** `apps/gpui/src/main.rs:3346` (`start_numeric_property_input`), `apps/gpui/src/main.rs:4463`–`4498` (`handle_numeric_property_key`), `apps/gpui/src/layer_panel.rs:588`–`599` (row `on_click`), `crates/editor_core/src/editor.rs:7346` (`set_numeric_property`).
- **Defect:** `numeric_property_input` stores only a `NumericPropertyTarget` enum and the typed text — not the node id being edited. Enter applies the value via `set_numeric_property`, which operates on the *current* `editor.selection()`. Canvas mouse-downs correctly dismiss the pending input (`dismiss_inline_input_from_mouse`, main.rs:3910–3930), but the layer panel row click calls `select_layer(...)` directly and never dismisses it.
- **Failure scenario:** Start editing shape A's X field, type a value, click layer B in the layer panel, press Enter → the value is applied to B's X. Silently moves the wrong shape. Worse across kinds: a pending `StrokeWidth` edit can be applied to a subsequently-selected text layer.
- **Figma/Lunacy:** switching selection commits or discards the in-progress inline edit against the *previous* selection; a stale edit never floats onto an unrelated node.
- **Suggested fix:** route layer-panel selection through the same dismiss helper the canvas uses (clear `numeric_property_input`/`property_color_input`/`zoom_input` before `select_layer`), or store the target node id(s) with the pending edit and validate on commit.

### 2. Undo/redo never restores selection — structural gap in the history model

- **Where:** `crates/editor_core/src/history.rs` (`History`/`HistoryEntry`, whole file), `crates/editor_core/src/command.rs` (`Command` has no selection field), `crates/editor_core/src/editor.rs` ~3703–3719 (`undo_document`/`redo_document`), ~5298–5338 (`delete_selection`).
- **Defect:** Commands carry only document patches. Selection changes made alongside document mutations (`delete_selection` clears selection; `group_selection`/`duplicate_selection` select the new node) happen outside the undoable command. Undo/redo only calls `prune_selection()`, which can drop stale ids but never re-select anything.
- **Failure scenario:** Select a shape, Delete, Ctrl+Z → shape reappears but nothing is selected; the user must re-click it. Same for undoing group/ungroup/duplicate/reparent — selection is left wherever the forward operation put it. The existing test `deleting_the_last_vector_contour_clears_selection_and_is_undoable` (editor.rs ~11291) asserts geometry restoration but (correctly, given current behavior) never asserts selection restoration.
- **Figma/Lunacy:** undo restores the selection active immediately before the action; redo restores the selection that resulted from it.
- **Suggested fix:** store before/after selection snapshots on `HistoryEntry` (or `Command`) and apply them in `History::undo`/`redo` alongside the patches.

---

## Important

### 3. Resizing a rotated shape shears it; resize handles sit on the world-aligned AABB, not the rotated corners

- **Where:** `crates/editor_core/src/editor.rs:4847`–`5086` (`start_selection_transform`, `update_resize`), `editor.rs:4834` (`selection_world_bounds`), backed by `transform_rect` in `crates/editor_core/src/lib.rs:2281`–`2296`.
- **Defect:** `transform_rect` returns the axis-aligned bounding box of the transformed corners. The transform session uses this AABB both to place the 8 handles and as the frame for `update_resize`, which composes `translate(fixed) * scale * translate(-fixed)` in **world axes** and pre-multiplies it onto the node's world transform. For any rotation that isn't a multiple of 90°, an axis-aligned scale composed with a rotated transform produces shear.
- **Failure scenario:** Rotate a rectangle 45°, drag a corner handle: (a) the handle is at the AABB corner, visually offset from the shape's actual corner; (b) the rectangle skews into a parallelogram instead of scaling along its own axes.
- **Figma/Lunacy:** handles render on the object's own rotated bounding box; corner drag scales along local axes, preserving shape ("resize follows rotation"). Figma falls back to axis-aligned bounds only for multi-selections with mixed rotations.
- **Suggested fix:** for single-object (or uniform-rotation) selections, run the transform session in the object's local/rotated frame; keep the AABB path only for mixed-rotation multi-selections.

### 4. Hidden layers cannot be selected from the layer panel, and hiding a layer force-deselects it

- **Where:** `crates/editor_core/src/lib.rs:576`–`594` (`is_effectively_visible`/`is_effectively_editable`), `crates/editor_core/src/editor.rs:1762`–`1791` (`select_layer`/`set_layer_selection`), `editor.rs:1606`–`1614` (`prune_selection`), `editor.rs:1676`–`1733` (visibility toggles), `apps/gpui/src/layer_panel.rs:588`–`599`.
- **Defect:** `select_layer` early-returns unless `is_effectively_editable`, which requires the node **and every ancestor** to be visible. So clicking a hidden layer's row in the panel is a silent no-op, and `toggle_visibility` calls `prune_selection` immediately, ejecting a just-hidden layer from the selection.
- **Failure scenario:** Hide a layer via the eye icon → it is instantly deselected; click its row to rename/inspect/re-position it while hidden → nothing happens; the only interaction left is the eye icon itself.
- **Figma/Lunacy:** hidden layers stay fully selectable from the layer panel (dimmed but clickable); hiding a selected layer keeps it selected. Only *canvas* hit-testing excludes hidden layers (which this codebase already does correctly in `hit_test_internal`).
- **Suggested fix:** split "selectable from panel" (lock-chain check only) from "interactable on canvas" (visibility + lock). Note the same gate also blocks *locked* layers from panel selection — Figma allows selecting locked layers from the panel too; arguably intentional here, but worth deciding deliberately.

---

## Medium

### 5. Layer panel shift-click toggles instead of range-selecting; no cmd/ctrl-toggle exists

- **Where:** `apps/gpui/src/layer_panel.rs:588`–`599` (`select_layer(layer_id, event.modifiers().shift)`), `crates/editor_core/src/editor.rs:1762` (`select_layer` treats the bool purely as toggle).
- **Defect:** There is no range-select concept in the layer list; shift is overloaded as toggle, and there is no separate cmd/ctrl toggle path.
- **Failure scenario:** Click layer 1, shift-click layer 5 expecting layers 1–5 selected → only layer 5 toggles.
- **Figma/Lunacy:** shift-click = contiguous range from the anchor row; cmd/ctrl-click = toggle a single row.
- **Suggested fix:** pass both modifiers into the row handler; compute ranges over the flattened `LayerEntry` order for shift, reserve toggle for cmd/ctrl.

---

## Low / notes

- **Misleading comment on `Patch::CreateNode`** (`crates/editor_core/src/command.rs`): the "already exists (redo)" branch is actually the normal path — every call site pre-inserts the node before building the patch. Not a bug; fix the comment.
- **`toggle_visibility`/`toggle_locked`** (`editor.rs:1676`, `1693`) lack the root-node guard that `set_layer_visible`/`set_layer_locked` have. Unreachable from the UI today (the layer tree never emits the root), flagged for awareness.
- **Layer rename input** relies on GPUI focus-out to commit; row clicks don't explicitly steal focus. Works in practice today, but is adjacent to the finding #1 dismissal gap.

---

## Areas audited and found sound

- **Save/load/automation state:** dirty tracking uses revision identity (`mark_revision_saved`), file I/O is serialized through a single `FileOperation` state, document swap bumps `document_epoch` invalidating automation sessions, and regression tests cover the save-race and open-race scenarios. No findings.
- **SVG import/export:** parsing is delegated to `usvg`'s resolved tree; local transform/paint conversion fails closed on non-finite/non-invertible geometry. No findings.
- **GPUI input glue:** screen↔canvas conversion, modifier translation, middle-button pan-outside-canvas, and Escape ownership per modal state are all consistent; transient UI state (pickers, inline inputs, menus, rename) is torn down on tool switches, actions, and document swaps — with the single exception of finding #1's layer-panel path.
- **Core interaction state machine:** `settle_interaction`/`cancel_interaction` gating before actions, `prune_selection` after undo/visibility changes, `filter_selection_for_transform` before move/delete/group/align/duplicate (prevents parent+child double-processing), deep-select (cmd-click), double-click enter-group, and pen/vector/text session cancel-vs-commit all verified correct.
- **Document model:** node ids are never reused (soft delete, slotmap entries kept), delete recursively soft-deletes descendants, group/ungroup snapshot and restore world transforms, save/load runs a full graph validator, and snap tolerance is correctly zoom-normalized in screen space.
