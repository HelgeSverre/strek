//! Main editor state combining document, selection, history, and tools.

use editor_render::DisplayList;
use glam::{Affine2, Vec2};

use crate::action::EditorAction;
use crate::command::{Command, Patch};
use crate::history::History;
use crate::input::{Cursor, Effects, InputEvent, Key, Modifiers, MouseButton};
use crate::layers::{LayerEntry, LayerIcon, LayerPanelState};
use crate::node::{Node, NodeId, NodeKind};
use crate::path::PathData;
use crate::selection::{Selection, SelectionAction};
use crate::snap::{SnapEngine, SnapPoint, SnapResult};
use crate::style::{Paint, Style};
use crate::transaction::Transaction;
use crate::transform::View;
use crate::Document;

/// The active tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    // Future: Pan, Rectangle, Ellipse, Pen, Text, etc.
}

/// Drag state during pointer operations.
#[derive(Default)]
enum DragState {
    /// No drag in progress
    #[default]
    None,
    /// Dragging to move selected nodes
    Moving {
        start_pos: Vec2,
        transaction: Transaction,
        /// Snap points from the moving nodes (for snap source)
        source_snap_points: Vec<SnapPoint>,
        /// Current snap result (for rendering guides)
        snap_result: Option<SnapResult>,
    },
    /// Marquee selection
    Marquee { start_pos: Vec2, current_pos: Vec2 },
    /// Panning the view
    Panning { start_pos: Vec2, start_pan: Vec2 },
}

/// The main editor state.
pub struct Editor {
    /// The document being edited
    pub document: Document,

    /// Current selection
    pub selection: Selection,

    /// Undo/redo history
    pub history: History,

    /// Camera/viewport state
    pub view: View,

    /// Current tool
    pub tool: Tool,

    /// Drag state
    drag: DragState,

    /// Whether a redraw is needed
    needs_redraw: bool,

    /// Snap engine for finding snap targets
    snap_engine: SnapEngine,

    /// Layer panel UI state (expanded/collapsed)
    pub layer_panel: LayerPanelState,
}

impl Editor {
    /// Create a new editor with an empty document.
    pub fn new() -> Self {
        Self {
            document: Document::new(),
            selection: Selection::new(),
            history: History::new(),
            view: View::default(),
            tool: Tool::Select,
            drag: DragState::None,
            needs_redraw: true,
            snap_engine: SnapEngine::new(),
            layer_panel: LayerPanelState::new(),
        }
    }

    /// Create an editor with a demo document.
    pub fn with_demo_content() -> Self {
        let mut editor = Self::new();

        // Add some demo shapes
        let rect1 = Node::shape("Blue Rectangle", PathData::rect(0.0, 0.0, 150.0, 100.0))
            .with_style(Style::fill(Paint::rgb(0.2, 0.4, 0.8)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(50.0, 50.0)));

        let rect2 = Node::shape("Red Rectangle", PathData::rect(0.0, 0.0, 100.0, 150.0))
            .with_style(Style::fill(Paint::rgb(0.8, 0.2, 0.2)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(250.0, 100.0)));

        let rect3 = Node::shape("Green Rectangle", PathData::rect(0.0, 0.0, 120.0, 80.0))
            .with_style(Style::fill(Paint::rgb(0.2, 0.7, 0.3)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(150.0, 300.0)));

        // Add demo text
        let text1 = Node::text("Hello Text", "Hello, World!")
            .with_style(Style::fill(Paint::rgb(0.1, 0.1, 0.1)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(400.0, 80.0)));

        let text2 = Node::text("Title", "Vector Editor")
            .with_style(Style::fill(Paint::rgb(0.3, 0.3, 0.8)))
            .with_transform(glam::Affine2::from_translation(Vec2::new(450.0, 250.0)));

        editor.document.add_child(editor.document.root, rect1);
        editor.document.add_child(editor.document.root, rect2);
        editor.document.add_child(editor.document.root, rect3);
        editor.document.add_child(editor.document.root, text1);
        editor.document.add_child(editor.document.root, text2);

        editor.needs_redraw = true;
        editor
    }

    // === Action System ===

    /// Execute an editor action.
    /// Returns true if the action was executed.
    pub fn execute_action(&mut self, action: EditorAction) -> bool {
        if !self.can_execute(action) {
            return false;
        }

        match action {
            // Selection
            EditorAction::SelectAll => {
                let all_nodes: Vec<_> = self.document.descendants(self.document.root).collect();
                self.selection.set(all_nodes);
                self.needs_redraw = true;
            }
            EditorAction::DeselectAll => {
                self.selection.clear();
                self.needs_redraw = true;
            }

            // Edit
            EditorAction::Undo => {
                if self.history.undo(&mut self.document) {
                    self.needs_redraw = true;
                }
            }
            EditorAction::Redo => {
                if self.history.redo(&mut self.document) {
                    self.needs_redraw = true;
                }
            }
            EditorAction::Delete => {
                self.delete_selection();
            }
            EditorAction::Duplicate => {
                self.duplicate_selection();
            }

            // Arrange
            EditorAction::Group => {
                self.group_selection();
            }
            EditorAction::Ungroup => {
                self.ungroup_selection();
            }

            // Nudge
            EditorAction::NudgeUp => {
                self.nudge_selection(Vec2::new(0.0, -1.0));
            }
            EditorAction::NudgeDown => {
                self.nudge_selection(Vec2::new(0.0, 1.0));
            }
            EditorAction::NudgeLeft => {
                self.nudge_selection(Vec2::new(-1.0, 0.0));
            }
            EditorAction::NudgeRight => {
                self.nudge_selection(Vec2::new(1.0, 0.0));
            }
            EditorAction::NudgeUpLarge => {
                self.nudge_selection(Vec2::new(0.0, -10.0));
            }
            EditorAction::NudgeDownLarge => {
                self.nudge_selection(Vec2::new(0.0, 10.0));
            }
            EditorAction::NudgeLeftLarge => {
                self.nudge_selection(Vec2::new(-10.0, 0.0));
            }
            EditorAction::NudgeRightLarge => {
                self.nudge_selection(Vec2::new(10.0, 0.0));
            }

            // View
            EditorAction::ZoomIn => {
                self.view.zoom *= 1.25;
                self.needs_redraw = true;
            }
            EditorAction::ZoomOut => {
                self.view.zoom /= 1.25;
                self.needs_redraw = true;
            }
            EditorAction::ZoomReset => {
                self.view.zoom = 1.0;
                self.needs_redraw = true;
            }
            EditorAction::ZoomResetAll => {
                self.view.zoom = 1.0;
                self.view.pan = Vec2::ZERO;
                self.needs_redraw = true;
            }
        }

        true
    }

    /// Check if an action can be executed in the current state.
    pub fn can_execute(&self, action: EditorAction) -> bool {
        match action {
            // Always available
            EditorAction::SelectAll
            | EditorAction::ZoomIn
            | EditorAction::ZoomOut
            | EditorAction::ZoomReset
            | EditorAction::ZoomResetAll => true,

            // Require selection
            EditorAction::DeselectAll
            | EditorAction::Delete
            | EditorAction::Duplicate
            | EditorAction::Group
            | EditorAction::NudgeUp
            | EditorAction::NudgeDown
            | EditorAction::NudgeLeft
            | EditorAction::NudgeRight
            | EditorAction::NudgeUpLarge
            | EditorAction::NudgeDownLarge
            | EditorAction::NudgeLeftLarge
            | EditorAction::NudgeRightLarge => !self.selection.is_empty(),

            // Ungroup requires grouped selection
            EditorAction::Ungroup => {
                self.selection.iter().any(|id| {
                    self.document
                        .get(id)
                        .map(|n| n.is_group())
                        .unwrap_or(false)
                })
            }

            // History actions
            EditorAction::Undo => self.history.can_undo(),
            EditorAction::Redo => self.history.can_redo(),
        }
    }

    /// Find the action for a key press.
    pub fn action_for_key(&self, key: Key, modifiers: Modifiers) -> Option<EditorAction> {
        for action in EditorAction::all() {
            if let Some(shortcut) = action.meta().default_shortcut {
                if shortcut.key == key
                    && shortcut.ctrl == modifiers.primary()
                    && shortcut.shift == modifiers.shift
                    && shortcut.alt == modifiers.alt
                {
                    return Some(*action);
                }
            }
        }

        // Handle Backspace as alternative for Delete
        if key == Key::Backspace && !modifiers.any() {
            return Some(EditorAction::Delete);
        }

        // Handle Ctrl+Y as alternative for Redo
        if key == Key::Y && modifiers.primary() && !modifiers.shift {
            return Some(EditorAction::Redo);
        }

        None
    }

    // === Layer Panel ===

    /// Build the layer tree for the layer panel UI.
    /// Returns a flat list of entries in display order (top to bottom).
    pub fn build_layer_tree(&self) -> Vec<LayerEntry> {
        let mut entries = Vec::new();
        self.build_layer_tree_recursive(self.document.root, 0, &mut entries);
        entries
    }

    fn build_layer_tree_recursive(
        &self,
        id: NodeId,
        depth: usize,
        entries: &mut Vec<LayerEntry>,
    ) {
        let Some(node) = self.document.get(id) else {
            return;
        };

        // Skip the root node itself, but process its children
        if id != self.document.root {
            let is_container = matches!(node.kind, NodeKind::Group | NodeKind::Frame(_));
            let has_children = !node.children.is_empty();
            let expanded = self.layer_panel.is_expanded(id);

            let icon = match &node.kind {
                NodeKind::Group => LayerIcon::Group,
                NodeKind::Frame(_) => LayerIcon::Frame,
                NodeKind::Shape(_) => LayerIcon::Shape,
                NodeKind::Text(_) => LayerIcon::Text,
            };

            entries.push(LayerEntry {
                id,
                name: node.name.clone(),
                depth,
                selected: self.selection.contains(id),
                visible: node.visible,
                locked: node.locked,
                is_container,
                expanded,
                has_children,
                icon,
            });

            // If collapsed, don't show children
            if is_container && !expanded {
                return;
            }
        }

        // Process children in reverse order (topmost first in the layer panel)
        for child_id in node.children.iter().rev() {
            let child_depth = if id == self.document.root { 0 } else { depth + 1 };
            self.build_layer_tree_recursive(*child_id, child_depth, entries);
        }
    }

    /// Toggle visibility of a node.
    pub fn toggle_visibility(&mut self, id: NodeId) {
        if let Some(node) = self.document.get_mut(id) {
            node.visible = !node.visible;
            self.needs_redraw = true;
        }
    }

    /// Toggle locked state of a node.
    pub fn toggle_locked(&mut self, id: NodeId) {
        if let Some(node) = self.document.get_mut(id) {
            node.locked = !node.locked;
        }
    }

    /// Select a node from the layer panel (replacing current selection).
    pub fn select_layer(&mut self, id: NodeId, add_to_selection: bool) {
        if add_to_selection {
            self.selection.add(id);
        } else {
            self.selection.set(vec![id]);
        }
        self.needs_redraw = true;
    }

    /// Toggle expand/collapse state of a layer in the layer panel.
    pub fn toggle_layer_expand(&mut self, id: NodeId) {
        self.layer_panel.toggle(id);
        self.needs_redraw = true;
    }

    /// Handle an input event and return effects.
    pub fn handle_event(&mut self, event: InputEvent) -> Effects {
        match event {
            InputEvent::PointerDown {
                position,
                button,
                modifiers,
            } => self.handle_pointer_down(position, button, modifiers),
            InputEvent::PointerMove {
                position,
                modifiers,
            } => self.handle_pointer_move(position, modifiers),
            InputEvent::PointerUp {
                position,
                button,
                modifiers,
            } => self.handle_pointer_up(position, button, modifiers),
            InputEvent::Scroll {
                position,
                delta,
                modifiers,
            } => self.handle_scroll(position, delta, modifiers),
            InputEvent::KeyDown { key, modifiers } => self.handle_key_down(key, modifiers),
            InputEvent::KeyUp { .. } => Effects::default(),
        }
    }

    fn handle_pointer_down(
        &mut self,
        screen_pos: Vec2,
        button: MouseButton,
        modifiers: Modifiers,
    ) -> Effects {
        let world_pos = self.view.to_world(screen_pos);

        match button {
            MouseButton::Left => {
                match self.tool {
                    Tool::Select => {
                        // Hit test
                        // Ctrl/Cmd+click allows selecting groups directly
                        let hit = if modifiers.ctrl || modifiers.meta {
                            self.document.hit_test_with_groups(world_pos)
                        } else {
                            self.document.hit_test(world_pos)
                        };
                        let action = self.selection.action_for_click(hit, modifiers.shift);

                        match action {
                            SelectionAction::ClickEmpty => {
                                self.selection.clear();
                                self.needs_redraw = true;
                            }
                            SelectionAction::ClickUnselected(id) => {
                                self.selection.select(id);
                                // Start move drag - filter to exclude children of selected parents
                                let all_nodes: Vec<_> = self.selection.iter().collect();
                                let nodes = self.document.filter_selection_for_transform(&all_nodes);
                                let transaction =
                                    Transaction::start_move("Move", &nodes, &self.document);

                                // Collect source snap points from selected nodes' bounds
                                let mut source_snap_points = Vec::new();
                                for &node_id in &nodes {
                                    source_snap_points.extend(self.document.snap_points(node_id));
                                }

                                // Set snap targets (all nodes except selected ones)
                                let targets = self.document.all_snap_points(&nodes);
                                self.snap_engine.set_targets(targets);

                                self.drag = DragState::Moving {
                                    start_pos: world_pos,
                                    transaction,
                                    source_snap_points,
                                    snap_result: None,
                                };
                                self.needs_redraw = true;
                            }
                            SelectionAction::ClickSelected(_) => {
                                // Start move drag with current selection - filter to exclude children
                                let all_nodes: Vec<_> = self.selection.iter().collect();
                                let nodes = self.document.filter_selection_for_transform(&all_nodes);
                                let transaction =
                                    Transaction::start_move("Move", &nodes, &self.document);

                                // Collect source snap points from selected nodes' bounds
                                let mut source_snap_points = Vec::new();
                                for &node_id in &nodes {
                                    source_snap_points.extend(self.document.snap_points(node_id));
                                }

                                // Set snap targets (all nodes except selected ones)
                                let targets = self.document.all_snap_points(&nodes);
                                self.snap_engine.set_targets(targets);

                                self.drag = DragState::Moving {
                                    start_pos: world_pos,
                                    transaction,
                                    source_snap_points,
                                    snap_result: None,
                                };
                            }
                            SelectionAction::ToggleNode(id) => {
                                self.selection.toggle(id);
                                self.needs_redraw = true;
                            }
                            SelectionAction::StartMarquee => {
                                self.drag = DragState::Marquee {
                                    start_pos: world_pos,
                                    current_pos: world_pos,
                                };
                            }
                        }
                    }
                }
            }
            MouseButton::Middle => {
                // Start panning
                self.drag = DragState::Panning {
                    start_pos: screen_pos,
                    start_pan: self.view.pan,
                };
            }
            MouseButton::Right => {
                // Could show context menu
            }
        }

        Effects {
            redraw: self.needs_redraw,
            cursor: Some(self.cursor_for_state()),
        }
    }

    fn handle_pointer_move(&mut self, screen_pos: Vec2, _modifiers: Modifiers) -> Effects {
        let world_pos = self.view.to_world(screen_pos);

        match &mut self.drag {
            DragState::None => {
                // Update cursor based on hover
            }
            DragState::Moving {
                start_pos,
                transaction,
                source_snap_points,
                snap_result,
            } => {
                // Calculate raw delta
                let raw_delta = world_pos - *start_pos;

                // Try to snap the source points to targets
                let result = self.snap_engine.find_snap(source_snap_points, raw_delta, self.view.zoom);

                // Use snapped position if available
                let final_delta = result.position;
                transaction.translate(final_delta, &mut self.document);

                // Store snap result for rendering guides
                *snap_result = Some(result);

                self.needs_redraw = true;
            }
            DragState::Marquee { current_pos, .. } => {
                *current_pos = world_pos;
                self.needs_redraw = true;
            }
            DragState::Panning {
                start_pos,
                start_pan,
            } => {
                let delta = screen_pos - *start_pos;
                self.view.pan = *start_pan + delta;
                self.needs_redraw = true;
            }
        }

        Effects {
            redraw: self.needs_redraw,
            cursor: Some(self.cursor_for_state()),
        }
    }

    fn handle_pointer_up(
        &mut self,
        _screen_pos: Vec2,
        _button: MouseButton,
        modifiers: Modifiers,
    ) -> Effects {
        match std::mem::take(&mut self.drag) {
            DragState::None => {}
            DragState::Moving { transaction, .. } => {
                // Clear snap targets when done dragging
                self.snap_engine.clear_targets();
                let cmd = transaction.commit(&self.document);
                if !cmd.is_empty() {
                    self.history.push(cmd);
                }
            }
            DragState::Marquee {
                start_pos,
                current_pos,
            } => {
                // Compute marquee selection
                let selected = self.compute_marquee_selection(start_pos, current_pos);

                if modifiers.shift {
                    // Add to existing selection
                    for id in selected {
                        self.selection.add(id);
                    }
                } else {
                    // Replace selection
                    self.selection.set(selected);
                }
                self.needs_redraw = true;
            }
            DragState::Panning { .. } => {}
        }

        self.needs_redraw = true;
        Effects {
            redraw: true,
            cursor: Some(self.cursor_for_state()),
        }
    }

    /// Compute which nodes are within the marquee rectangle.
    fn compute_marquee_selection(&mut self, start: Vec2, end: Vec2) -> Vec<NodeId> {
        use crate::path::Rect;

        // Create the marquee rectangle
        let min_x = start.x.min(end.x);
        let min_y = start.y.min(end.y);
        let max_x = start.x.max(end.x);
        let max_y = start.y.max(end.y);
        let marquee = Rect::new(Vec2::new(min_x, min_y), Vec2::new(max_x, max_y));

        let mut selected = Vec::new();

        // Check all nodes (except root and groups)
        for id in self
            .document
            .descendants(self.document.root)
            .collect::<Vec<_>>()
        {
            if let Some(node) = self.document.get(id) {
                // Skip groups (select their children instead)
                if node.is_group() {
                    continue;
                }

                // Skip locked nodes
                if node.locked {
                    continue;
                }

                // Get world bounds
                if let Some(bounds) = self.document.world_bounds(id) {
                    // Check intersection with marquee
                    if marquee.intersects(&bounds) {
                        selected.push(id);
                    }
                }
            }
        }

        selected
    }

    fn handle_scroll(&mut self, screen_pos: Vec2, delta: Vec2, modifiers: Modifiers) -> Effects {
        if modifiers.ctrl || modifiers.meta {
            // Zoom
            let zoom_factor = if delta.y > 0.0 { 0.9 } else { 1.1 };
            self.view.zoom_at(zoom_factor, screen_pos);
        } else {
            // Pan
            self.view.pan -= delta;
        }
        self.needs_redraw = true;

        Effects {
            redraw: true,
            cursor: None,
        }
    }

    fn handle_key_down(&mut self, key: Key, modifiers: Modifiers) -> Effects {
        // Use the action system to handle keyboard shortcuts
        if let Some(action) = self.action_for_key(key, modifiers) {
            self.execute_action(action);
        }

        Effects {
            redraw: self.needs_redraw,
            cursor: None,
        }
    }

    /// Nudge the selection by a world-space delta.
    fn nudge_selection(&mut self, delta: Vec2) {
        if self.selection.is_empty() {
            return;
        }

        // Convert screen delta to world delta
        let world_delta = delta / self.view.zoom;

        // Filter selection to exclude children of selected parents
        let all_nodes: Vec<_> = self.selection.iter().collect();
        let nodes = self.document.filter_selection_for_transform(&all_nodes);

        let mut patches = Vec::new();

        for id in nodes {
            if let Some(node) = self.document.get(id) {
                let before = node.transform;
                let after = Affine2::from_translation(world_delta) * before;
                patches.push(Patch::SetTransform { id, before, after });
            }
        }

        if !patches.is_empty() {
            // Apply patches to document
            for patch in &patches {
                patch.apply_forward(&mut self.document);
            }

            // Record in history
            self.history.push(Command {
                description: "Nudge".into(),
                patches,
            });

            self.document.dirty.transforms = true;
            self.document.dirty.bounds = true;
            self.needs_redraw = true;
        }
    }

    fn cursor_for_state(&self) -> Cursor {
        match &self.drag {
            DragState::None => Cursor::Default,
            DragState::Moving { .. } => Cursor::Move,
            DragState::Marquee { .. } => Cursor::Crosshair,
            DragState::Panning { .. } => Cursor::Grabbing,
        }
    }

    /// Delete the current selection.
    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }

        let mut patches = Vec::new();

        for id in self.selection.iter() {
            if let Some(node) = self.document.get(id) {
                if let Some(parent) = node.parent {
                    let index = self
                        .document
                        .get(parent)
                        .map(|p| p.children.iter().position(|&c| c == id).unwrap_or(0))
                        .unwrap_or(0);

                    patches.push(Patch::DeleteNode {
                        id,
                        node: Box::new(node.clone()),
                        parent,
                        index,
                    });
                }
            }
        }

        if !patches.is_empty() {
            let cmd = Command {
                description: "Delete".into(),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);
            self.selection.clear();
            self.needs_redraw = true;
        }
    }

    /// Group the current selection.
    pub fn group_selection(&mut self) {
        let nodes: Vec<_> = self.selection.iter().collect();
        if nodes.len() < 2 {
            return;
        }

        if let Some(group_id) = self.document.group_nodes(&nodes) {
            self.selection.select(group_id);
            self.needs_redraw = true;
            // Note: For proper undo, we'd create patches here
        }
    }

    /// Ungroup the selected group(s).
    pub fn ungroup_selection(&mut self) {
        let groups: Vec<_> = self
            .selection
            .iter()
            .filter(|&id| self.document.get(id).map(|n| n.is_group()).unwrap_or(false))
            .collect();

        let mut new_selection = Vec::new();

        for group_id in groups {
            if let Some(children) = self.document.ungroup(group_id) {
                new_selection.extend(children);
            }
        }

        if !new_selection.is_empty() {
            self.selection.set(new_selection);
            self.needs_redraw = true;
        }
    }

    /// Duplicate the current selection.
    pub fn duplicate_selection(&mut self) {
        let nodes: Vec<_> = self.selection.iter().collect();
        let mut new_selection = Vec::new();

        for id in nodes {
            if let Some(new_id) = self.document.duplicate(id) {
                new_selection.push(new_id);
            }
        }

        if !new_selection.is_empty() {
            self.selection.set(new_selection);
            self.needs_redraw = true;
        }
    }

    /// Align selected nodes along an axis.
    pub fn align_selection(&mut self, axis: crate::snap::AlignAxis) {
        let nodes: Vec<_> = self.selection.iter().collect();
        if nodes.len() < 2 {
            return;
        }

        // Get bounds for all selected nodes
        let bounds: Vec<_> = nodes
            .iter()
            .filter_map(|&id| self.document.world_bounds(id))
            .collect();

        if bounds.len() != nodes.len() {
            return; // Some nodes have no bounds
        }

        // Calculate alignment deltas
        let deltas = crate::snap::calculate_alignment(&bounds, axis);

        // Apply deltas to each node
        let mut patches = Vec::new();
        for (id, delta) in nodes.iter().zip(deltas.iter()) {
            if delta.length_squared() < 0.0001 {
                continue; // No change needed
            }

            if let Some(node) = self.document.get(*id) {
                let before = node.transform;
                let after = glam::Affine2::from_translation(*delta) * before;

                patches.push(Patch::SetTransform {
                    id: *id,
                    before,
                    after,
                });
            }
        }

        if !patches.is_empty() {
            let cmd = Command {
                description: format!("Align {:?}", axis),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);
            self.needs_redraw = true;
        }
    }

    /// Distribute selected nodes evenly along an axis.
    pub fn distribute_selection(&mut self, axis: crate::snap::DistributeAxis) {
        let nodes: Vec<_> = self.selection.iter().collect();
        if nodes.len() < 3 {
            return; // Need at least 3 nodes to distribute
        }

        // Get bounds for all selected nodes
        let bounds: Vec<_> = nodes
            .iter()
            .filter_map(|&id| self.document.world_bounds(id))
            .collect();

        if bounds.len() != nodes.len() {
            return; // Some nodes have no bounds
        }

        // Calculate distribution deltas
        let deltas = crate::snap::calculate_distribution(&bounds, axis);

        // Apply deltas to each node
        let mut patches = Vec::new();
        for (id, delta) in nodes.iter().zip(deltas.iter()) {
            if delta.length_squared() < 0.0001 {
                continue; // No change needed
            }

            if let Some(node) = self.document.get(*id) {
                let before = node.transform;
                let after = glam::Affine2::from_translation(*delta) * before;

                patches.push(Patch::SetTransform {
                    id: *id,
                    before,
                    after,
                });
            }
        }

        if !patches.is_empty() {
            let cmd = Command {
                description: format!("Distribute {:?}", axis),
                patches,
            };
            cmd.apply(&mut self.document);
            self.history.push(cmd);
            self.needs_redraw = true;
        }
    }

    /// Build a display list for rendering.
    pub fn build_display_list(&mut self) -> DisplayList {
        let mut display_list = self.document.build_display_list(&self.view);

        // Add snap guides if we're dragging and have snap results
        if let DragState::Moving { snap_result: Some(result), .. } = &self.drag {
            // Add vertical snap guide (for X alignment)
            if let Some(ref snap_x) = result.snap_x {
                let screen_x = self.view.to_screen(Vec2::new(snap_x.value, 0.0)).x;
                display_list.push(editor_render::DisplayItem::SnapGuide {
                    start: Vec2::new(screen_x, 0.0),
                    end: Vec2::new(screen_x, 10000.0), // Large enough to span viewport
                    horizontal: false, // vertical line for X snap
                });
            }

            // Add horizontal snap guide (for Y alignment)
            if let Some(ref snap_y) = result.snap_y {
                let screen_y = self.view.to_screen(Vec2::new(0.0, snap_y.value)).y;
                display_list.push(editor_render::DisplayItem::SnapGuide {
                    start: Vec2::new(0.0, screen_y),
                    end: Vec2::new(10000.0, screen_y), // Large enough to span viewport
                    horizontal: true, // horizontal line for Y snap
                });
            }
        }

        // Add selection rectangles for selected nodes
        for id in self.selection.iter() {
            if let Some(bounds) = self.document.world_bounds(id) {
                // Transform bounds to screen space
                let screen_min = self.view.to_screen(bounds.min);
                let screen_max = self.view.to_screen(bounds.max);

                display_list.push(editor_render::DisplayItem::SelectionRect {
                    min: screen_min,
                    max: screen_max,
                });
            }
        }

        display_list
    }

    /// Check if a redraw is needed and clear the flag.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    /// Mark the editor as needing a redraw.
    pub fn set_needs_redraw(&mut self, needs_redraw: bool) {
        self.needs_redraw = needs_redraw;
    }

    /// Get the current view.
    pub fn view(&self) -> &View {
        &self.view
    }

    /// Get a reference to the document.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Get a reference to the selection.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_editor() {
        let editor = Editor::new();
        assert!(editor.selection.is_empty());
        assert_eq!(editor.tool, Tool::Select);
    }

    #[test]
    fn test_demo_content() {
        let editor = Editor::with_demo_content();
        // Should have 3 shapes + 2 text nodes = 5 items
        let count = editor.document.descendants(editor.document.root).count();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_click_to_select() {
        let mut editor = Editor::with_demo_content();

        // Click on the first shape (at approximately 125, 100 - center of first rect)
        let event = InputEvent::PointerDown {
            position: Vec2::new(125.0, 100.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        };

        editor.handle_event(event);

        // Something should be selected
        assert!(!editor.selection.is_empty());
    }

    #[test]
    fn test_click_empty_deselects() {
        let mut editor = Editor::with_demo_content();

        // First select something
        let shapes: Vec<_> = editor.document.descendants(editor.document.root).collect();
        editor.selection.select(shapes[0]);
        assert!(!editor.selection.is_empty());

        // Click on empty space
        let event = InputEvent::PointerDown {
            position: Vec2::new(500.0, 500.0),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
        };

        editor.handle_event(event);

        // Selection should be cleared
        assert!(editor.selection.is_empty());
    }

    #[test]
    fn test_undo_redo() {
        let mut editor = Editor::with_demo_content();

        // Get a shape and move it
        let shapes: Vec<_> = editor.document.descendants(editor.document.root).collect();
        let shape_id = shapes[0];

        let original_pos = editor.document.get(shape_id).unwrap().transform.translation;

        // Simulate drag
        editor.selection.select(shape_id);
        let transaction = Transaction::start_move("Move", &[shape_id], &editor.document);
        transaction.translate(Vec2::new(50.0, 50.0), &mut editor.document);
        let cmd = transaction.commit(&editor.document);
        editor.history.push(cmd);

        let moved_pos = editor.document.get(shape_id).unwrap().transform.translation;
        assert_ne!(original_pos, moved_pos);

        // Undo
        editor.history.undo(&mut editor.document);
        let after_undo = editor.document.get(shape_id).unwrap().transform.translation;
        assert_eq!(original_pos, after_undo);

        // Redo
        editor.history.redo(&mut editor.document);
        let after_redo = editor.document.get(shape_id).unwrap().transform.translation;
        assert_eq!(moved_pos, after_redo);
    }
}
