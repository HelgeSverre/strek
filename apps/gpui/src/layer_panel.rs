//! Layer panel UI component.

use editor_core::{LayerEntry, LayerIcon};
use gpui::{div, prelude::*, px, rgb, rgba, Context, SharedString, StatefulInteractiveElement};

use crate::{
    assets::{icon, Icon},
    VectorEditor,
};

pub const WIDTH: f32 = 248.0;

/// Render the layer panel on the right side of the window.
pub fn render_layer_panel(
    layer_entries: Vec<LayerEntry>,
    cx: &mut Context<VectorEditor>,
) -> impl IntoElement {
    let rows = layer_entries
        .into_iter()
        .map(|entry| render_layer_entry(entry, cx).into_any_element())
        .collect::<Vec<_>>();

    div()
        .id("layer-panel")
        .w(px(WIDTH))
        .h_full()
        .flex()
        .flex_col()
        .bg(rgb(0x202124))
        .border_l_1()
        .border_color(rgb(0x414349))
        .child(
            div()
                .h(px(40.0))
                .w_full()
                .flex()
                .items_center()
                .gap(px(7.0))
                .px(px(10.0))
                .border_b_1()
                .border_color(rgb(0x414349))
                .text_color(rgb(0xf1f3f4))
                .text_size(px(12.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(icon(Icon::Layers, 15.0, rgb(0xa8abb2)))
                .child("Layers")
                .child(div().flex_1())
                .child(
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .bg(rgb(0x292a2e))
                        .text_color(rgb(0x777a82))
                        .text_size(px(9.0))
                        .child("ASSETS"),
                ),
        )
        .child(
            div()
                .id("layer-list-scroll")
                .flex_1()
                .overflow_y_scroll()
                .py(px(5.0))
                .children(rows),
        )
}

fn render_layer_entry(entry: LayerEntry, cx: &mut Context<VectorEditor>) -> impl IntoElement {
    let indent = entry.depth as f32 * 16.0;
    let bg = if entry.selected {
        rgb(0x183d58)
    } else {
        rgba(0x00000000)
    };
    let hover_bg = if entry.selected {
        rgb(0x183d58)
    } else {
        rgb(0x2b2c31)
    };

    let text_color = if !entry.visible {
        rgb(0x666971)
    } else if entry.locked {
        rgb(0x9a9da5)
    } else {
        rgb(0xd8dade)
    };

    let type_icon = match entry.icon {
        LayerIcon::Group => Icon::Group,
        LayerIcon::Frame => Icon::Frame,
        LayerIcon::Shape => Icon::Rectangle,
        LayerIcon::Text => Icon::Text,
    };

    let expand_icon = if entry.has_children {
        if entry.expanded {
            "▼"
        } else {
            "▶"
        }
    } else {
        " "
    };
    let layer_id = entry.id;
    let expand_id = entry.id;
    let visibility_id = entry.id;
    let lock_id = entry.id;

    div()
        .id(SharedString::from(format!("layer-{:?}", entry.id)))
        .h(px(28.0))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .pl(px(8.0 + indent))
        .pr(px(8.0))
        .gap(px(5.0))
        .bg(bg)
        .hover(|s| s.bg(hover_bg))
        .cursor_pointer()
        .text_color(text_color)
        .text_size(px(12.0))
        .on_click(
            cx.listener(move |editor, event: &gpui::ClickEvent, _window, cx| {
                editor
                    .editor
                    .select_layer(layer_id, event.modifiers().shift);
                cx.notify();
            }),
        )
        // Expand/collapse chevron
        .child(
            div()
                .id(SharedString::from(format!("expand-{:?}", entry.id)))
                .w(px(12.0))
                .text_size(px(8.0))
                .text_color(rgb(0x8f929a))
                .child(expand_icon)
                .when(entry.has_children, |chevron| {
                    chevron
                        .cursor_pointer()
                        .on_click(cx.listener(move |editor, _, _window, cx| {
                            editor.editor.toggle_layer_expand(expand_id);
                            cx.stop_propagation();
                            cx.notify();
                        }))
                }),
        )
        // Type icon
        .child(
            div()
                .id(SharedString::from(format!("type-{:?}", entry.id)))
                .w(px(16.0))
                .h(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(type_icon, 13.0, rgb(0x8f929a))),
        )
        // Layer name
        .child(div().flex_1().overflow_hidden().child(entry.name.clone()))
        // Visibility toggle
        .child(
            div()
                .id(SharedString::from(format!("visibility-{:?}", entry.id)))
                .w(px(16.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .opacity(if entry.visible { 0.65 } else { 1.0 })
                .child(icon(
                    if entry.visible {
                        Icon::Eye
                    } else {
                        Icon::EyeOff
                    },
                    13.0,
                    rgb(if entry.visible { 0xa8abb2 } else { 0x6f7279 }),
                ))
                .cursor_pointer()
                .on_click(cx.listener(move |editor, _, _window, cx| {
                    editor.editor.toggle_visibility(visibility_id);
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
        // Lock toggle
        .child(
            div()
                .id(SharedString::from(format!("lock-{:?}", entry.id)))
                .w(px(16.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .opacity(if entry.locked { 1.0 } else { 0.35 })
                .child(icon(
                    if entry.locked {
                        Icon::Lock
                    } else {
                        Icon::Unlock
                    },
                    12.0,
                    rgb(0xa8abb2),
                ))
                .cursor_pointer()
                .on_click(cx.listener(move |editor, _, _window, cx| {
                    editor.editor.toggle_locked(lock_id);
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
}
