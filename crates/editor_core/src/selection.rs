use std::collections::HashSet;

use crate::node::NodeId;
use crate::path::Rect;

/// Selection state for the editor.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// Set of selected node IDs
    nodes: HashSet<NodeId>,

    /// Primary/anchor node (for multi-select operations)
    primary: Option<NodeId>,
}

impl Selection {
    /// Create an empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if selection is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get the number of selected nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if a node is selected.
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains(&id)
    }

    /// Get the primary (anchor) node.
    pub fn primary(&self) -> Option<NodeId> {
        self.primary
    }

    /// Iterate over selected nodes.
    pub fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.iter().copied()
    }

    /// Get selected nodes as a vector.
    pub fn to_vec(&self) -> Vec<NodeId> {
        self.nodes.iter().copied().collect()
    }

    /// Clear the selection.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.primary = None;
    }

    /// Select a single node (replacing any existing selection).
    pub fn select(&mut self, id: NodeId) {
        self.nodes.clear();
        self.nodes.insert(id);
        self.primary = Some(id);
    }

    /// Add a node to the selection.
    pub fn add(&mut self, id: NodeId) {
        self.nodes.insert(id);
        if self.primary.is_none() {
            self.primary = Some(id);
        }
    }

    /// Remove a node from the selection.
    pub fn remove(&mut self, id: NodeId) {
        self.nodes.remove(&id);
        if self.primary == Some(id) {
            self.primary = self.nodes.iter().next().copied();
        }
    }

    /// Toggle selection of a node.
    pub fn toggle(&mut self, id: NodeId) {
        if self.nodes.contains(&id) {
            self.remove(id);
        } else {
            self.add(id);
        }
    }

    /// Set the selection to multiple nodes.
    pub fn set(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.nodes.clear();
        self.nodes.extend(ids);
        self.primary = self.nodes.iter().next().copied();
    }

    /// Select all nodes within a rectangle (marquee selection).
    /// Takes a predicate that checks if a node's bounds intersect the rect.
    pub fn select_in_rect<F>(&mut self, rect: Rect, mut intersects: F)
    where
        F: FnMut(NodeId) -> bool,
    {
        // This is called with a closure that has access to the document
        // to check bounds intersection
        let _ = rect; // rect is used by the closure
        self.nodes.retain(|&id| intersects(id));
    }
}

/// Result of a click for selection purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAction {
    /// Click on empty space - clear selection
    ClickEmpty,
    /// Click on unselected node - select it
    ClickUnselected(NodeId),
    /// Click on already selected node - keep selection (for drag)
    ClickSelected(NodeId),
    /// Shift-click to toggle
    ToggleNode(NodeId),
    /// Shift-click on empty - start marquee
    StartMarquee,
}

impl Selection {
    /// Determine what action to take based on a click.
    pub fn action_for_click(&self, hit: Option<NodeId>, shift_held: bool) -> SelectionAction {
        match (hit, shift_held) {
            (None, false) => SelectionAction::ClickEmpty,
            (None, true) => SelectionAction::StartMarquee,
            (Some(id), true) => SelectionAction::ToggleNode(id),
            (Some(id), false) if self.contains(id) => SelectionAction::ClickSelected(id),
            (Some(id), false) => SelectionAction::ClickUnselected(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    fn make_ids() -> (NodeId, NodeId, NodeId) {
        let mut slots: SlotMap<NodeId, ()> = SlotMap::with_key();
        let a = slots.insert(());
        let b = slots.insert(());
        let c = slots.insert(());
        (a, b, c)
    }

    #[test]
    fn test_empty_selection() {
        let sel = Selection::new();
        assert!(sel.is_empty());
        assert_eq!(sel.len(), 0);
    }

    #[test]
    fn test_select_single() {
        let (a, b, _) = make_ids();
        let mut sel = Selection::new();

        sel.select(a);
        assert!(sel.contains(a));
        assert!(!sel.contains(b));
        assert_eq!(sel.primary(), Some(a));

        sel.select(b);
        assert!(!sel.contains(a));
        assert!(sel.contains(b));
        assert_eq!(sel.primary(), Some(b));
    }

    #[test]
    fn test_add_remove() {
        let (a, b, _c) = make_ids();
        let mut sel = Selection::new();

        sel.add(a);
        sel.add(b);
        assert_eq!(sel.len(), 2);
        assert!(sel.contains(a));
        assert!(sel.contains(b));

        sel.remove(a);
        assert!(!sel.contains(a));
        assert!(sel.contains(b));
    }

    #[test]
    fn test_toggle() {
        let (a, _, _) = make_ids();
        let mut sel = Selection::new();

        sel.toggle(a);
        assert!(sel.contains(a));

        sel.toggle(a);
        assert!(!sel.contains(a));
    }

    #[test]
    fn test_action_for_click() {
        let (a, b, _) = make_ids();
        let mut sel = Selection::new();
        sel.select(a);

        assert_eq!(
            sel.action_for_click(None, false),
            SelectionAction::ClickEmpty
        );
        assert_eq!(
            sel.action_for_click(None, true),
            SelectionAction::StartMarquee
        );
        assert_eq!(
            sel.action_for_click(Some(a), false),
            SelectionAction::ClickSelected(a)
        );
        assert_eq!(
            sel.action_for_click(Some(b), false),
            SelectionAction::ClickUnselected(b)
        );
        assert_eq!(
            sel.action_for_click(Some(a), true),
            SelectionAction::ToggleNode(a)
        );
    }
}
