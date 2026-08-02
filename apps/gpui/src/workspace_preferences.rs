//! Per-user presentation and precision preferences.

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const PREFERENCES_FILE: &str = "workspace.json";
const DEFAULT_PANEL_WIDTH: f32 = 440.0;
const DEFAULT_PANEL_HEIGHT: f32 = 480.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FloatingPanelPreferences {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for FloatingPanelPreferences {
    fn default() -> Self {
        Self {
            x: 72.0,
            y: 84.0,
            width: DEFAULT_PANEL_WIDTH,
            height: DEFAULT_PANEL_HEIGHT,
        }
    }
}

impl FloatingPanelPreferences {
    pub(crate) fn sanitized(self) -> Self {
        Self {
            x: finite_or(self.x, 72.0).max(0.0),
            y: finite_or(self.y, 84.0).max(0.0),
            width: finite_or(self.width, DEFAULT_PANEL_WIDTH).clamp(320.0, 960.0),
            height: finite_or(self.height, DEFAULT_PANEL_HEIGHT).clamp(240.0, 1_200.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspacePreferences {
    pub show_rulers: bool,
    pub show_grid: bool,
    pub show_guides: bool,
    pub guides_locked: bool,
    pub snapping_enabled: bool,
    pub snap_to_objects: bool,
    pub snap_to_guides: bool,
    pub snap_to_grid: bool,
    pub snap_tolerance: f32,
    pub color_library_panel: FloatingPanelPreferences,
    pub collapsed_color_groups: BTreeSet<String>,
    pub last_color_group: Option<String>,
}

impl Default for WorkspacePreferences {
    fn default() -> Self {
        Self {
            show_rulers: false,
            show_grid: false,
            show_guides: true,
            guides_locked: false,
            snapping_enabled: true,
            snap_to_objects: true,
            snap_to_guides: true,
            snap_to_grid: false,
            snap_tolerance: 8.0,
            color_library_panel: FloatingPanelPreferences::default(),
            collapsed_color_groups: BTreeSet::new(),
            last_color_group: None,
        }
    }
}

impl WorkspacePreferences {
    pub(crate) fn load() -> Self {
        let Some(path) = preferences_path() else {
            return Self::default();
        };
        let Ok(contents) = crate::document_io::read_path_to_string(&path) else {
            return Self::default();
        };
        let Ok(preferences) = serde_json::from_str::<Self>(&contents) else {
            return Self::default();
        };
        preferences.sanitized()
    }

    pub(crate) fn persist(&self) {
        if let Err(error) = self.write() {
            log::warn!("could not persist workspace preferences: {error}");
        }
    }

    fn sanitized(mut self) -> Self {
        self.snap_tolerance = finite_or(self.snap_tolerance, 8.0).clamp(1.0, 32.0);
        self.color_library_panel = self.color_library_panel.sanitized();
        self.last_color_group = self
            .last_color_group
            .take()
            .filter(|group| !group.trim().is_empty());
        self
    }

    fn write(&self) -> io::Result<()> {
        let Some(path) = preferences_path() else {
            return Ok(());
        };
        let json = serde_json::to_vec_pretty(&self.sanitized_clone()).map_err(io::Error::other)?;
        crate::document_io::write_atomic(&path, &json)
    }

    fn sanitized_clone(&self) -> Self {
        self.clone().sanitized()
    }
}

fn preferences_path() -> Option<PathBuf> {
    crate::document_io::app_config_directory().map(|directory| directory.join(PREFERENCES_FILE))
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_scoped_precision_behavior() {
        let preferences = WorkspacePreferences::default();
        assert!(preferences.show_guides);
        assert!(preferences.snapping_enabled);
        assert!(preferences.snap_to_objects);
        assert!(preferences.snap_to_guides);
        assert!(!preferences.snap_to_grid);
        assert_eq!(preferences.snap_tolerance, 8.0);
    }

    #[test]
    fn malformed_numeric_preferences_are_sanitized() {
        let preferences = WorkspacePreferences {
            snap_tolerance: f32::NAN,
            color_library_panel: FloatingPanelPreferences {
                x: f32::INFINITY,
                y: -10.0,
                width: 10.0,
                height: 50_000.0,
            },
            last_color_group: Some("".to_owned()),
            ..WorkspacePreferences::default()
        }
        .sanitized();

        assert_eq!(preferences.snap_tolerance, 8.0);
        assert_eq!(preferences.color_library_panel.x, 72.0);
        assert_eq!(preferences.color_library_panel.y, 0.0);
        assert_eq!(preferences.color_library_panel.width, 320.0);
        assert_eq!(preferences.color_library_panel.height, 1_200.0);
        assert_eq!(preferences.last_color_group, None);
    }

    #[test]
    fn older_partial_json_uses_field_defaults() {
        let preferences: WorkspacePreferences =
            serde_json::from_str(r#"{"show_grid":true}"#).unwrap();
        assert!(preferences.show_grid);
        assert!(preferences.show_guides);
        assert_eq!(preferences.snap_tolerance, 8.0);
    }
}
