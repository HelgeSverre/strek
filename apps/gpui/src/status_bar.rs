//! Status bar UI component.

use editor_core::{InteractionKind, Tool};
use gpui::{div, prelude::*, px, rgb};

pub const HEIGHT: f32 = 24.0;

/// Render the status bar at the bottom of the window.
pub fn render_status_bar(
    tool: Tool,
    interaction: InteractionKind,
    selection_count: usize,
    zoom: f32,
) -> impl IntoElement {
    let tool_name = match tool {
        Tool::Select => "Select",
        Tool::Frame => "Frame",
        Tool::Rectangle => "Rectangle",
        Tool::Ellipse => "Ellipse",
        Tool::Pen => "Pen",
        Tool::Text => "Text",
        Tool::VectorEdit => "Edit vector",
    };
    let tool_shortcut = match tool {
        Tool::Select => "V",
        Tool::Frame => "F",
        Tool::Rectangle => "R",
        Tool::Ellipse => "O",
        Tool::Pen => "P",
        Tool::Text => "T",
        Tool::VectorEdit => "Enter",
    };

    let selection_text = if selection_count == 0 {
        "No selection".to_string()
    } else if selection_count == 1 {
        "1 object selected".to_string()
    } else {
        format!("{} objects selected", selection_count)
    };

    let zoom_text = format!("{:.0}%", zoom * 100.0);
    let hint = match interaction {
        InteractionKind::Idle => match tool {
            Tool::Select => "Drag handles to resize · rotate above selection · Space to pan",
            Tool::Frame => "Drag an artboard · enclosed objects become children",
            Tool::Rectangle | Tool::Ellipse => "Drag to draw · Shift constrains · Alt centers",
            Tool::Pen => "Click for corners · drag for curves · Enter finishes",
            Tool::Text => "Click for point text · drag for a wrapping text box",
            Tool::VectorEdit => "Select anchors or handles · double-click a segment to insert",
        },
        InteractionKind::Moving => "Moving · release to commit · Esc cancels",
        InteractionKind::Resizing => "Resizing · Shift constrains · Alt resizes from center",
        InteractionKind::Rotating => "Rotating · Shift snaps to 15°",
        InteractionKind::Marquee => "Selecting objects",
        InteractionKind::CreatingShape => "Drawing shape · release to create",
        InteractionKind::CreatingFrame => "Drawing artboard · release to adopt enclosed objects",
        InteractionKind::Pen => "Building path · Enter finishes · Backspace removes last anchor",
        InteractionKind::CreatingText => "Drawing text box",
        InteractionKind::TextEditing => "Editing text · ⌘Enter commits · Esc cancels",
        InteractionKind::VectorEditing => "Editing vector · Enter commits · Delete removes anchors",
        InteractionKind::Panning => "Panning canvas",
    };

    div()
        .id("status-bar")
        .h(px(HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .px(px(10.0))
        .gap(px(8.0))
        .bg(rgb(0x202124))
        .border_t_1()
        .border_color(rgb(0x414349))
        .text_color(rgb(0xa8abb2))
        .text_size(px(10.0))
        .child(
            div()
                .w(px(5.0))
                .h(px(5.0))
                .rounded(px(2.5))
                .bg(rgb(0x0c8ce9)),
        )
        .child(
            div()
                .text_color(rgb(0xd8dade))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(tool_name),
        )
        .child(div().w(px(1.0)).h(px(12.0)).bg(rgb(0x414349)))
        .child(div().child(selection_text))
        .child(div().flex_1().text_color(rgb(0x777a82)).child(hint))
        .child(
            div()
                .text_color(rgb(0x777a82))
                .child(format!("{tool_shortcut}  {tool_name}")),
        )
        .child(div().w(px(1.0)).h(px(12.0)).bg(rgb(0x414349)))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(3.0))
                .child("Canvas")
                .child(zoom_text),
        )
}
