//! Event handling for UI widgets.

use glam::Vec2;

/// Mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// Pointer (mouse/touch) event.
#[derive(Debug, Clone, Copy)]
pub struct PointerEvent {
    /// Position in widget-local coordinates.
    pub position: Vec2,
    /// Which button is involved (if any).
    pub button: Option<MouseButton>,
}

impl PointerEvent {
    /// Create a new pointer event.
    pub fn new(position: Vec2) -> Self {
        Self {
            position,
            button: None,
        }
    }

    /// Create a pointer event with a button.
    pub fn with_button(position: Vec2, button: MouseButton) -> Self {
        Self {
            position,
            button: Some(button),
        }
    }
}

/// UI event.
#[derive(Debug, Clone)]
pub enum Event {
    /// Pointer entered the widget.
    PointerEnter(PointerEvent),
    /// Pointer left the widget.
    PointerLeave,
    /// Pointer moved within the widget.
    PointerMove(PointerEvent),
    /// Pointer button pressed.
    PointerDown(PointerEvent),
    /// Pointer button released.
    PointerUp(PointerEvent),
    /// Click (pointer down + up on same widget).
    Click(PointerEvent),
    /// Scroll/wheel event.
    Scroll {
        position: Vec2,
        delta: Vec2,
    },
}

/// Response from event handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventResponse {
    /// Event was not handled, continue propagation.
    #[default]
    Ignored,
    /// Event was handled, stop propagation.
    Handled,
    /// Widget wants to capture all pointer events.
    Capture,
}

impl EventResponse {
    /// Check if the event was handled.
    pub fn is_handled(self) -> bool {
        matches!(self, EventResponse::Handled | EventResponse::Capture)
    }

    /// Merge two responses, preferring handled/capture.
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (EventResponse::Capture, _) | (_, EventResponse::Capture) => EventResponse::Capture,
            (EventResponse::Handled, _) | (_, EventResponse::Handled) => EventResponse::Handled,
            _ => EventResponse::Ignored,
        }
    }
}
