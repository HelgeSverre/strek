use glam::{Affine2, Vec2};

/// Smallest supported canvas zoom (1%).
pub const MIN_ZOOM: f32 = 0.01;

/// Largest supported canvas zoom (6400%).
pub const MAX_ZOOM: f32 = 64.0;

/// Camera/viewport state for mapping between world and screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Pan offset in screen pixels
    pub pan: Vec2,

    /// Zoom level (1.0 = 100%)
    pub zoom: f32,
}

impl View {
    /// Create a new view with default settings (no pan, 100% zoom).
    pub fn new() -> Self {
        Self::default()
    }

    /// Transform from world space to screen space.
    pub fn screen_from_world(&self) -> Affine2 {
        Affine2::from_scale_angle_translation(Vec2::splat(self.zoom), 0.0, self.pan)
    }

    /// Transform from screen space to world space.
    pub fn world_from_screen(&self) -> Affine2 {
        self.screen_from_world().inverse()
    }

    /// Convert a screen position to world position.
    pub fn to_world(&self, screen_pos: Vec2) -> Vec2 {
        self.world_from_screen().transform_point2(screen_pos)
    }

    /// Convert a world position to screen position.
    pub fn to_screen(&self, world_pos: Vec2) -> Vec2 {
        self.screen_from_world().transform_point2(world_pos)
    }

    /// Pan the view by a screen delta.
    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan += delta;
    }

    /// Zoom the view by a factor around a screen point.
    pub fn zoom_at(&mut self, factor: f32, screen_point: Vec2) {
        if factor.is_finite() && factor > 0.0 {
            self.set_zoom_at(self.zoom * factor, screen_point);
        }
    }

    /// Set an absolute zoom level while preserving the world point under a screen position.
    ///
    /// Returns whether the view changed.
    pub fn set_zoom_at(&mut self, zoom: f32, screen_point: Vec2) -> bool {
        if !zoom.is_finite() || !screen_point.is_finite() || self.zoom <= 0.0 {
            return false;
        }

        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (self.zoom - zoom).abs() <= f32::EPSILON {
            return false;
        }

        let world_pos = self.to_world(screen_point);
        self.zoom = zoom;
        let new_screen = self.to_screen(world_pos);
        self.pan += screen_point - new_screen;
        true
    }
}

impl Default for View {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_view() {
        let view = View::default();
        assert_eq!(view.pan, Vec2::ZERO);
        assert_eq!(view.zoom, 1.0);
    }

    #[test]
    fn test_screen_world_roundtrip() {
        let view = View {
            pan: Vec2::new(100.0, 50.0),
            zoom: 2.0,
        };

        let world_pos = Vec2::new(10.0, 20.0);
        let screen_pos = view.to_screen(world_pos);
        let back = view.to_world(screen_pos);

        assert!((back - world_pos).length() < 0.001);
    }

    #[test]
    fn test_zoom_at_origin() {
        let mut view = View::default();
        view.zoom_at(2.0, Vec2::ZERO);

        assert_eq!(view.zoom, 2.0);
        // Zooming at origin with no pan should keep pan at zero
        assert!((view.pan - Vec2::ZERO).length() < 0.001);
    }

    #[test]
    fn absolute_zoom_preserves_the_world_point_under_its_anchor() {
        let mut view = View {
            pan: Vec2::new(80.0, -30.0),
            zoom: 2.0,
        };
        let anchor = Vec2::new(320.0, 240.0);
        let world_before = view.to_world(anchor);

        assert!(view.set_zoom_at(4.0, anchor));

        assert!((view.to_world(anchor) - world_before).length() < 0.001);
    }

    #[test]
    fn zoom_is_clamped_and_rejects_invalid_values() {
        let mut view = View::default();

        assert!(view.set_zoom_at(1000.0, Vec2::ZERO));
        assert_eq!(view.zoom, MAX_ZOOM);
        assert!(view.set_zoom_at(0.0, Vec2::ZERO));
        assert_eq!(view.zoom, MIN_ZOOM);
        assert!(!view.set_zoom_at(f32::NAN, Vec2::ZERO));
        assert_eq!(view.zoom, MIN_ZOOM);
    }
}
