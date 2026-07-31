use std::collections::HashSet;

use crate::node::NodeId;
use crate::path::Rect;

/// Selection state for the editor.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// Selected node IDs in insertion order.
    nodes: Vec<NodeId>,

    /// Membership index for efficient selection checks.
    membership: HashSet<NodeId>,
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
        self.membership.contains(&id)
    }

    /// Get the primary (anchor) node.
    pub fn primary(&self) -> Option<NodeId> {
        self.nodes.first().copied()
    }

    /// Iterate over selected nodes.
    pub fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.iter().copied()
    }

    /// Get selected nodes as a vector.
    pub fn to_vec(&self) -> Vec<NodeId> {
        self.nodes.clone()
    }

    /// Clear the selection.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.membership.clear();
    }

    /// Select a single node (replacing any existing selection).
    pub fn select(&mut self, id: NodeId) {
        self.nodes.clear();
        self.nodes.push(id);
        self.membership.clear();
        self.membership.insert(id);
    }

    /// Add a node to the selection.
    pub fn add(&mut self, id: NodeId) {
        if self.membership.insert(id) {
            self.nodes.push(id);
        }
    }

    /// Remove a node from the selection.
    pub fn remove(&mut self, id: NodeId) {
        if self.membership.remove(&id) {
            self.nodes.retain(|selected| *selected != id);
        }
    }

    /// Toggle selection of a node.
    pub fn toggle(&mut self, id: NodeId) {
        if self.membership.contains(&id) {
            self.remove(id);
        } else {
            self.add(id);
        }
    }

    /// Set the selection to multiple nodes.
    pub fn set(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.clear();
        for id in ids {
            self.add(id);
        }
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
        let membership = &mut self.membership;
        self.nodes.retain(|&id| {
            let retain = intersects(id);
            if !retain {
                membership.remove(&id);
            }
            retain
        });
    }
}

/// Result of a click for selection purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAction {
    /// Click on unselected node - select it
    ClickUnselected(NodeId),
    /// Click on already selected node - keep selection (for drag)
    ClickSelected(NodeId),
    /// Shift-click to toggle
    ToggleNode(NodeId),
    /// Click on empty - start marquee (or clear on release if no drag)
    StartMarquee,
}

impl Selection {
    /// Determine what action to take based on a click.
    ///
    /// Clicking on empty space starts a marquee. If the user releases without
    /// dragging, the selection is cleared. If they drag, marquee selection occurs.
    pub fn action_for_click(&self, hit: Option<NodeId>, shift_held: bool) -> SelectionAction {
        match (hit, shift_held) {
            // Click on empty space always starts marquee potential
            (None, _) => SelectionAction::StartMarquee,
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
    fn set_preserves_first_occurrence_order() {
        let (a, b, c) = make_ids();
        let mut sel = Selection::new();

        sel.set([b, a, b, c]);

        assert_eq!(sel.iter().collect::<Vec<_>>(), [b, a, c]);
        assert_eq!(sel.to_vec(), [b, a, c]);
        assert_eq!(sel.primary(), Some(b));
    }

    #[test]
    fn removing_primary_promotes_the_next_inserted_node() {
        let (a, b, c) = make_ids();
        let mut sel = Selection::new();
        sel.set([a, b, c]);

        sel.remove(b);
        assert_eq!(sel.iter().collect::<Vec<_>>(), [a, c]);
        assert_eq!(sel.primary(), Some(a));

        sel.remove(a);
        assert_eq!(sel.iter().collect::<Vec<_>>(), [c]);
        assert_eq!(sel.primary(), Some(c));

        sel.remove(c);
        assert!(sel.is_empty());
        assert_eq!(sel.primary(), None);
    }

    #[test]
    fn select_in_rect_retains_order_and_promotes_primary() {
        let (a, b, c) = make_ids();
        let mut sel = Selection::new();
        sel.set([a, b, c]);
        let mut visited = Vec::new();

        sel.select_in_rect(Rect::empty(), |id| {
            visited.push(id);
            id != a
        });

        assert_eq!(visited, [a, b, c]);
        assert_eq!(sel.iter().collect::<Vec<_>>(), [b, c]);
        assert_eq!(sel.primary(), Some(b));
        assert!(sel.contains(b));
        assert!(!sel.contains(a));
    }

    #[test]
    fn test_action_for_click() {
        let (a, b, _) = make_ids();
        let mut sel = Selection::new();
        sel.select(a);

        // Click on empty always starts marquee (actual clear happens on release if no drag)
        assert_eq!(
            sel.action_for_click(None, false),
            SelectionAction::StartMarquee
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
