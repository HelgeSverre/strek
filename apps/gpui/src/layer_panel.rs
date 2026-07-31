//! Layer panel UI component.

use editor_core::{LayerEntry, LayerIcon, NodeId};
use gpui::{
    div, prelude::*, px, rgb, rgba, Context, Entity, Render, SharedString,
    StatefulInteractiveElement, Window,
};

use crate::{
    assets::{icon, Icon},
    layer_name_input::LayerNameInput,
    properties_panel::{self, PropertiesSnapshot},
    toolbar::editor_tooltip,
    VectorEditor,
};

pub const LAYERS_WIDTH: f32 = 248.0;
pub const DESIGN_WIDTH: f32 = 248.0;

/// Render the persistent layer tree on the left side of the window.
pub fn render_layers_panel(
    layer_entries: Vec<LayerEntry>,
    layer_name_input: Option<(NodeId, Entity<LayerNameInput>)>,
    cx: &mut Context<VectorEditor>,
) -> impl IntoElement {
    let rows = layer_entries
        .into_iter()
        .map(|entry| {
            let input = layer_name_input
                .as_ref()
                .filter(|(id, _)| *id == entry.id)
                .map(|(_, input)| input.clone());
            render_layer_entry(entry, input, cx).into_any_element()
        })
        .collect::<Vec<_>>();

    div()
        .id("layers-panel")
        .w(px(LAYERS_WIDTH))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(rgb(0x202124))
        .border_r_1()
        .border_color(rgb(0x414349))
        .child(
            sidebar_section_header("Layers", Icon::Layers)
                .drag_over::<DraggedLayer>(|style, _, _, _| style.bg(rgb(0x164f73)))
                .on_drop(cx.listener(|editor, dragged: &DraggedLayer, _window, cx| {
                    let root = editor.editor.document().root;
                    let index = editor
                        .editor
                        .document()
                        .get(root)
                        .map(|node| node.children.len())
                        .unwrap_or_default();
                    if editor.editor.reparent_layer(dragged.id, root, index) {
                        cx.notify();
                    }
                    cx.stop_propagation();
                })),
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

/// Render the persistent design inspector on the right side of the window.
pub fn render_design_panel(properties: PropertiesSnapshot) -> impl IntoElement {
    div()
        .id("design-panel")
        .w(px(DESIGN_WIDTH))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(rgb(0x202124))
        .border_l_1()
        .border_color(rgb(0x414349))
        .child(sidebar_section_header("Design", Icon::Properties))
        .child(properties_panel::render(properties))
}

fn sidebar_section_header(label: &'static str, icon_kind: Icon) -> gpui::Div {
    div()
        .h(px(36.0))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(7.0))
        .px(px(12.0))
        .border_b_1()
        .border_color(rgb(0x414349))
        .text_color(rgb(0xd8dade))
        .text_size(px(11.0))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(icon(icon_kind, 14.0, rgb(0xa8abb2)))
        .child(label)
}

#[derive(Clone)]
struct DraggedLayer {
    id: NodeId,
    name: SharedString,
}

struct DraggedLayerPreview {
    name: SharedString,
}

impl Render for DraggedLayerPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.0))
            .py(px(6.0))
            .bg(rgb(0x292a2e))
            .border_1()
            .border_color(rgb(0x0c8ce9))
            .rounded(px(5.0))
            .shadow_md()
            .text_size(px(11.0))
            .text_color(rgb(0xf1f3f4))
            .child(self.name.clone())
    }
}

fn render_layer_entry(
    entry: LayerEntry,
    name_input: Option<Entity<LayerNameInput>>,
    cx: &mut Context<VectorEditor>,
) -> impl IntoElement {
    let is_renaming = name_input.is_some();
    let indent = entry.depth as f32 * 16.0;
    let bg = if entry.selected {
        rgb(0x164f73)
    } else {
        rgba(0x00000000)
    };
    let hover_bg = if entry.selected {
        rgb(0x164f73)
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
    let drag_payload = DraggedLayer {
        id: entry.id,
        name: entry.name.clone().into(),
    };
    let drop_target = entry.id;
    let expand_id = entry.id;
    let visibility_id = entry.id;
    let lock_id = entry.id;
    let expand_tooltip = if entry.expanded {
        format!("Collapse {}", entry.name)
    } else {
        format!("Expand {}", entry.name)
    };
    let visibility_tooltip = if entry.visible {
        format!("Hide {}", entry.name)
    } else {
        format!("Show {}", entry.name)
    };
    let lock_tooltip = if entry.locked {
        format!("Unlock {}", entry.name)
    } else {
        format!("Lock {}", entry.name)
    };

    div()
        .id(SharedString::from(format!("layer-{:?}", entry.id)))
        .h(px(32.0))
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
        .when(entry.selected, |row| {
            row.border_l_2().border_color(rgb(0x0c8ce9))
        })
        .text_color(text_color)
        .text_size(px(12.0))
        .on_drag(drag_payload, |dragged, _, _, cx| {
            cx.new(|_| DraggedLayerPreview {
                name: dragged.name.clone(),
            })
        })
        .drag_over::<DraggedLayer>(move |style, dragged, _, _| {
            if dragged.id == drop_target {
                style
            } else {
                style
                    .bg(rgb(0x164f73))
                    .border_1()
                    .border_color(rgb(0x0c8ce9))
            }
        })
        .on_drop(
            cx.listener(move |editor, dragged: &DraggedLayer, _window, cx| {
                if editor.drop_layer_on(dragged.id, drop_target) {
                    cx.notify();
                }
                cx.stop_propagation();
            }),
        )
        .on_click(
            cx.listener(move |editor, event: &gpui::ClickEvent, window, cx| {
                if event.down.click_count >= 2 {
                    editor.begin_layer_rename(layer_id, window, cx);
                } else {
                    editor
                        .editor
                        .select_layer(layer_id, event.modifiers().shift);
                }
                cx.notify();
            }),
        )
        // Expand/collapse chevron
        .child(
            div()
                .id(SharedString::from(format!("expand-{:?}", entry.id)))
                .w(px(20.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .text_size(px(8.0))
                .text_color(rgb(0x8f929a))
                .child(expand_icon)
                .when(entry.has_children, |chevron| {
                    chevron
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x35363b)))
                        .tooltip(editor_tooltip(expand_tooltip, None))
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
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .when_some(name_input, |name, input| name.child(input))
                .when(!is_renaming, |name| name.child(entry.name.clone())),
        )
        // Visibility toggle
        .child(
            div()
                .id(SharedString::from(format!("visibility-{:?}", entry.id)))
                .w(px(24.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
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
                .hover(|style| style.bg(rgb(0x35363b)))
                .tooltip(editor_tooltip(visibility_tooltip, None))
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
                .w(px(24.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
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
                .hover(|style| style.bg(rgb(0x35363b)))
                .tooltip(editor_tooltip(lock_tooltip, None))
                .on_click(cx.listener(move |editor, _, _window, cx| {
                    editor.editor.toggle_locked(lock_id);
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
}
