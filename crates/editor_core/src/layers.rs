//! Layer panel data structures and utilities.
//!
//! This module provides types for representing the document hierarchy
//! in a format suitable for layer panel UI rendering.

use crate::node::NodeId;

/// A single entry in the layer tree.
#[derive(Debug, Clone)]
pub struct LayerEntry {
    /// Node ID this entry represents
    pub id: NodeId,

    /// Display name
    pub name: String,

    /// Depth in the tree (0 = root children)
    pub depth: usize,

    /// Whether this node is selected
    pub selected: bool,

    /// Whether this node is visible
    pub visible: bool,

    /// Whether this node is locked
    pub locked: bool,

    /// Whether this is a container (group/frame) that can have children
    pub is_container: bool,

    /// Whether this container is expanded in the UI
    pub expanded: bool,

    /// Whether this node has children
    pub has_children: bool,

    /// Icon type hint for the UI
    pub icon: LayerIcon,
}

/// Icon type for layer entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerIcon {
    /// Folder/group icon
    Group,
    /// Frame/artboard icon
    Frame,
    /// Shape (rectangle, ellipse, path) icon
    Shape,
    /// Text icon
    Text,
}

/// State for tracking expanded/collapsed groups in the layer panel.
#[derive(Debug, Clone, Default)]
pub struct LayerPanelState {
    /// Set of collapsed node IDs (default is expanded)
    collapsed: std::collections::HashSet<NodeId>,
}

impl LayerPanelState {
    /// Create a new layer panel state with all groups expanded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a node is expanded.
    pub fn is_expanded(&self, id: NodeId) -> bool {
        !self.collapsed.contains(&id)
    }

    /// Toggle the expanded state of a node.
    pub fn toggle(&mut self, id: NodeId) {
        if self.collapsed.contains(&id) {
            self.collapsed.remove(&id);
        } else {
            self.collapsed.insert(id);
        }
    }

    /// Expand a node.
    pub fn expand(&mut self, id: NodeId) {
        self.collapsed.remove(&id);
    }

    /// Collapse a node.
    pub fn collapse(&mut self, id: NodeId) {
        self.collapsed.insert(id);
    }

    /// Expand all nodes.
    pub fn expand_all(&mut self) {
        self.collapsed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    fn make_id() -> NodeId {
        let mut map: SlotMap<NodeId, ()> = SlotMap::with_key();
        map.insert(())
    }

    #[test]
    fn test_layer_panel_state() {
        let mut state = LayerPanelState::new();
        let id = make_id();

        // Default is expanded
        assert!(state.is_expanded(id));

        // Toggle to collapsed
        state.toggle(id);
        assert!(!state.is_expanded(id));

        // Toggle back to expanded
        state.toggle(id);
        assert!(state.is_expanded(id));
    }

    #[test]
    fn test_expand_all() {
        let mut state = LayerPanelState::new();
        let id1 = make_id();
        let id2 = make_id();

        state.collapse(id1);
        state.collapse(id2);

        assert!(!state.is_expanded(id1));
        assert!(!state.is_expanded(id2));

        state.expand_all();

        assert!(state.is_expanded(id1));
        assert!(state.is_expanded(id2));
    }
}
