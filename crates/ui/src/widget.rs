//! Widget trait and base types.

use crate::draw::DrawList;
use crate::event::{Event, EventResponse};
use crate::layout::{BoxConstraints, Size};
use crate::rect::Rect;
use crate::theme::Theme;

/// Unique identifier for a widget instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetId(pub u64);

impl WidgetId {
    /// Generate a new unique widget ID.
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for WidgetId {
    fn default() -> Self {
        Self::new()
    }
}

/// Mutable state for a widget.
#[derive(Debug, Clone, Default)]
pub struct WidgetState {
    /// Whether the pointer is over this widget.
    pub hovered: bool,
    /// Whether this widget has focus.
    pub focused: bool,
    /// Whether the widget is being pressed.
    pub pressed: bool,
    /// Whether the widget is disabled.
    pub disabled: bool,
}

impl WidgetState {
    /// Create a new default state.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Context passed to widgets during layout and rendering.
pub struct LayoutContext<'a> {
    pub theme: &'a Theme,
}

/// Context passed to widgets during rendering.
pub struct DrawContext<'a> {
    pub theme: &'a Theme,
    pub draw_list: &'a mut DrawList,
}

impl<'a> DrawContext<'a> {
    /// Fill a rectangle.
    pub fn fill_rect(&mut self, rect: Rect, color: crate::Color) {
        self.draw_list.fill_rect(rect, color);
    }

    /// Fill a rounded rectangle.
    pub fn fill_rounded_rect(&mut self, rect: Rect, color: crate::Color, radius: f32) {
        self.draw_list.fill_rounded_rect(rect, color, radius);
    }

    /// Draw text.
    pub fn text(&mut self, text: impl Into<String>, rect: Rect, style: crate::draw::TextStyle) {
        self.draw_list.text(text, rect, style);
    }

    /// Draw an icon.
    pub fn icon(&mut self, kind: crate::draw::IconKind, rect: Rect, color: crate::Color) {
        self.draw_list.icon(kind, rect, color);
    }
}

/// The core widget trait.
///
/// Widgets are the building blocks of the UI. Each widget is responsible for:
/// - Measuring its preferred size given constraints
/// - Laying out its children
/// - Drawing itself
/// - Handling events
pub trait Widget {
    /// Measure the widget's preferred size.
    ///
    /// This is called during the layout pass to determine how much space
    /// the widget wants. The widget should return a size that fits within
    /// the given constraints.
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size;

    /// Draw the widget.
    ///
    /// The widget should draw itself within the given rect using the draw context.
    fn draw(&self, rect: Rect, state: &WidgetState, ctx: &mut DrawContext);

    /// Handle an event.
    ///
    /// Returns whether the event was handled.
    fn event(&mut self, _event: &Event, _rect: Rect, _state: &mut WidgetState) -> EventResponse {
        EventResponse::Ignored
    }

    /// Get the children of this widget for hit testing.
    fn children(&self) -> &[WidgetNode] {
        &[]
    }

    /// Get mutable children.
    fn children_mut(&mut self) -> &mut [WidgetNode] {
        &mut []
    }
}

/// A positioned widget with its computed layout.
#[derive(Debug)]
pub struct WidgetNode {
    /// The widget.
    pub widget: Box<dyn Widget>,
    /// Unique identifier.
    pub id: WidgetId,
    /// Computed bounds after layout.
    pub rect: Rect,
    /// Widget state.
    pub state: WidgetState,
    /// Cached size from measurement.
    pub measured_size: Size,
}

impl WidgetNode {
    /// Create a new widget node.
    pub fn new(widget: impl Widget + 'static) -> Self {
        Self {
            widget: Box::new(widget),
            id: WidgetId::new(),
            rect: Rect::zero(),
            state: WidgetState::new(),
            measured_size: Size::zero(),
        }
    }

    /// Measure the widget.
    pub fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        self.measured_size = self.widget.measure(constraints, ctx);
        self.measured_size
    }

    /// Lay out the widget at a specific position.
    pub fn layout(&mut self, rect: Rect) {
        self.rect = rect;
    }

    /// Draw the widget.
    pub fn draw(&self, ctx: &mut DrawContext) {
        self.widget.draw(self.rect, &self.state, ctx);
    }

    /// Handle an event, returning whether it was handled.
    pub fn event(&mut self, event: &Event, offset: glam::Vec2) -> EventResponse {
        let local_rect = self.rect.offset(-offset);
        self.widget.event(event, local_rect, &mut self.state)
    }

    /// Check if a point is within the widget's bounds.
    pub fn hit_test(&self, point: glam::Vec2) -> bool {
        self.rect.contains(point)
    }
}

// Make WidgetNode implement Debug for the boxed trait
impl std::fmt::Debug for dyn Widget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Widget")
    }
}
