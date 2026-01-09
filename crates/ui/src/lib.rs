//! Retained-mode UI widget system.
//!
//! This crate provides a flexible, renderer-agnostic widget system for building
//! user interfaces. It uses a flexbox-inspired layout system and supports
//! theming, event handling, and custom widgets.
//!
//! # Architecture
//!
//! - **Widgets** are the building blocks of the UI. Each widget implements the
//!   `Widget` trait and can contain child widgets.
//! - **Layout** is computed in two passes: measure (bottom-up) and arrange (top-down).
//! - **Rendering** produces a list of `DrawCommand`s that can be executed by any renderer.
//! - **Events** bubble up from the target widget to its ancestors.

mod color;
mod draw;
mod event;
mod layout;
mod rect;
mod theme;
mod widget;
mod widgets;

pub use color::Color;
pub use draw::{DrawCommand, DrawList, TextStyle};
pub use event::{Event, EventResponse, MouseButton, PointerEvent};
pub use layout::{Align, Axis, BoxConstraints, CrossAlign, MainAlign, Padding, Size};
pub use rect::Rect;
pub use theme::Theme;
pub use widget::{Widget, WidgetId, WidgetNode, WidgetState};
pub use widgets::{Button, Container, Flex, Icon, Label, Panel, ScrollView, Spacer, Text};

// Re-export IconKind from draw
pub use draw::IconKind;
