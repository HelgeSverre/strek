use glam::{Affine2, Vec2};
use slotmap::SecondaryMap;

use crate::node::NodeId;
use crate::path::Rect;

/// Cached derived data for the document.
/// These values are computed from the node tree and cached for performance.
#[derive(Debug, Clone, Default)]
pub struct Cache {
    /// World transform for each node (parent chain composed)
    pub world_transform: SecondaryMap<NodeId, Affine2>,

    /// Axis-aligned bounding box in world space
    pub world_bounds: SecondaryMap<NodeId, Rect>,

    /// Layout offset for each node (computed by auto-layout).
    /// This is separate from the node's author transform (dual transform model).
    pub layout_offset: SecondaryMap<NodeId, Vec2>,
}

impl Cache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all cached data.
    pub fn clear(&mut self) {
        self.world_transform.clear();
        self.world_bounds.clear();
        self.layout_offset.clear();
    }

    /// Clear cache for a specific node.
    pub fn invalidate(&mut self, id: NodeId) {
        self.world_transform.remove(id);
        self.world_bounds.remove(id);
        self.layout_offset.remove(id);
    }

    /// Get the layout offset for a node.
    pub fn get_layout_offset(&self, id: NodeId) -> Vec2 {
        self.layout_offset.get(id).copied().unwrap_or(Vec2::ZERO)
    }

    /// Set the layout offset for a node.
    pub fn set_layout_offset(&mut self, id: NodeId, offset: Vec2) {
        self.layout_offset.insert(id, offset);
    }
}

/// Dirty flags for incremental cache updates.
#[derive(Debug, Clone, Copy, Default)]
pub struct DirtyFlags {
    /// Transforms need recomputation
    pub transforms: bool,

    /// Bounds need recomputation
    pub bounds: bool,

    /// Layout needs recomputation
    pub layout: bool,
}

impl DirtyFlags {
    /// Create with all flags clean.
    pub fn clean() -> Self {
        Self::default()
    }

    /// Create with all flags dirty.
    pub fn all_dirty() -> Self {
        Self {
            transforms: true,
            bounds: true,
            layout: true,
        }
    }

    /// Mark all flags as dirty.
    pub fn mark_all_dirty(&mut self) {
        self.transforms = true;
        self.bounds = true;
        self.layout = true;
    }

    /// Clear all dirty flags.
    pub fn clear(&mut self) {
        self.transforms = false;
        self.bounds = false;
        self.layout = false;
    }

    /// Check if any flag is dirty.
    pub fn any_dirty(&self) -> bool {
        self.transforms || self.bounds || self.layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dirty_flags_default() {
        let flags = DirtyFlags::default();
        assert!(!flags.transforms);
        assert!(!flags.bounds);
        assert!(!flags.layout);
        assert!(!flags.any_dirty());
    }

    #[test]
    fn test_dirty_flags_all_dirty() {
        let flags = DirtyFlags::all_dirty();
        assert!(flags.transforms);
        assert!(flags.bounds);
        assert!(flags.layout);
        assert!(flags.any_dirty());
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = Cache::new();
        let mut document = crate::Document::new();
        let id = document
            .add_child(document.root, crate::Node::group("Cached"))
            .expect("a new document accepts a child");

        cache.world_transform.insert(id, Affine2::IDENTITY);
        cache
            .world_bounds
            .insert(id, Rect::new(Vec2::ZERO, Vec2::ONE));
        cache.layout_offset.insert(id, Vec2::new(4.0, 8.0));

        assert!(cache.world_transform.contains_key(id));
        assert!(cache.world_bounds.contains_key(id));
        assert_eq!(cache.get_layout_offset(id), Vec2::new(4.0, 8.0));

        cache.clear();

        assert!(cache.world_transform.is_empty());
        assert!(cache.world_bounds.is_empty());
        assert!(cache.layout_offset.is_empty());
        assert_eq!(cache.get_layout_offset(id), Vec2::ZERO);
    }
}
