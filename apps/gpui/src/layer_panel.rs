//! Layer panel UI component.

use std::borrow::BorrowMut;

use editor_core::{Editor, EditorAction, LayerEntry, LayerIcon, NodeId, NodeKind};
use gpui::{
    anchored, div, prelude::*, px, rgb, rgba, Action, Context, Corner, Entity, MouseButton,
    MouseDownEvent, Pixels, Point, Render, SharedString, StatefulInteractiveElement, WeakEntity,
    Window,
};

use crate::{
    assets::{icon, Icon},
    commands::{CommandTarget, Keymap},
    layer_name_input::LayerNameInput,
    properties_panel::{self, PropertiesSnapshot},
    toolbar::editor_tooltip,
    Delete, Duplicate, Group, Strek, Ungroup,
};

pub const DEFAULT_PANEL_WIDTH: f32 = 248.0;
pub const MIN_PANEL_WIDTH: f32 = 180.0;
pub const MAX_PANEL_WIDTH: f32 = 420.0;
pub const MIN_CANVAS_WIDTH: f32 = 320.0;

const CONTEXT_MENU_WIDTH: f32 = 224.0;
const LAYER_ROW_HEIGHT: f32 = 28.0;
const LAYER_INDENT_STEP: f32 = 12.0;
const LAYER_ROW_PADDING_LEFT: f32 = 6.0;
const LAYER_EXPAND_WIDTH: f32 = 18.0;

/// Layer and window-space anchor for the currently open layer-row menu.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerContextMenu {
    target: NodeId,
    position: Point<Pixels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextSelectionPolicy {
    Preserve,
    SelectTarget,
    TargetUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayerMenuInvocation {
    ContextClick,
    MoreButton,
}

fn context_selection_policy(
    target_selected: bool,
    target_editable: bool,
) -> ContextSelectionPolicy {
    if target_selected {
        ContextSelectionPolicy::Preserve
    } else if target_editable {
        ContextSelectionPolicy::SelectTarget
    } else {
        ContextSelectionPolicy::TargetUnavailable
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LayerMenuAvailability {
    duplicate: bool,
    group: bool,
    ungroup: bool,
    delete: bool,
}

fn layer_menu_availability(editor: &Editor, target: NodeId) -> LayerMenuAvailability {
    if !editor.document().is_effectively_editable(target) {
        return LayerMenuAvailability::default();
    }

    if !editor.selection().contains(target) {
        return LayerMenuAvailability {
            duplicate: true,
            group: false,
            ungroup: editor
                .document()
                .get(target)
                .is_some_and(|node| matches!(node.kind, NodeKind::Group)),
            delete: true,
        };
    }

    LayerMenuAvailability {
        duplicate: editor.can_execute(EditorAction::Duplicate),
        group: editor.can_execute(EditorAction::Group),
        ungroup: editor.can_execute(EditorAction::Ungroup),
        delete: editor.can_execute(EditorAction::Delete),
    }
}

impl Strek {
    fn open_layer_context_menu(
        &mut self,
        target: NodeId,
        position: Point<Pixels>,
        invocation: LayerMenuInvocation,
        cx: &mut Context<Self>,
    ) {
        self.finish_layer_rename(true, cx);
        self.open_menu = None;
        let target_selected = self.editor.selection().contains(target);
        let target_editable = self.editor.document().is_effectively_editable(target);
        if invocation == LayerMenuInvocation::ContextClick {
            match context_selection_policy(target_selected, target_editable) {
                ContextSelectionPolicy::Preserve => {}
                ContextSelectionPolicy::SelectTarget => self.editor.select_layer(target, false),
                ContextSelectionPolicy::TargetUnavailable => return,
            }
        } else if !target_editable {
            return;
        }
        self.layer_context_menu = Some(LayerContextMenu { target, position });
        cx.notify();
    }

    fn close_layer_context_menu_from_mouse(
        &mut self,
        _: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layer_context_menu = None;
        cx.stop_propagation();
        cx.notify();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelSide {
    Layers,
    Design,
}

#[derive(Clone)]
pub(crate) struct PanelResizeDrag(PanelSide);

impl PanelResizeDrag {
    pub(crate) fn side(&self) -> PanelSide {
        self.0
    }
}

impl Render for PanelResizeDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// Render the persistent layer tree on the left side of the window.
pub fn render_layers_panel(
    layer_entries: Vec<LayerEntry>,
    layer_name_input: Option<(NodeId, Entity<LayerNameInput>)>,
    width: f32,
    cx: &mut Context<Strek>,
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
        .relative()
        .w(px(width))
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
                .py(px(3.0))
                .children(rows),
        )
        .child(panel_resize_handle(PanelSide::Layers))
}

/// Render the window-level menu for a layer row.
pub(crate) fn render_layer_context_menu(
    menu: LayerContextMenu,
    editor: &Editor,
    keymap: &Keymap,
    cx: &mut Context<Strek>,
) -> impl IntoElement {
    let availability = layer_menu_availability(editor, menu.target);
    let title = editor
        .document()
        .get(menu.target)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| "Layer".to_owned());

    div()
        .id("layer-context-menu-scrim")
        .absolute()
        .inset_0()
        .occlude()
        .on_any_mouse_down(cx.listener(Strek::close_layer_context_menu_from_mouse))
        .child(
            anchored()
                .anchor(Corner::TopLeft)
                .position(menu.position)
                .snap_to_window()
                .child(
                    div()
                        .id("layer-context-menu")
                        .w(px(CONTEXT_MENU_WIDTH))
                        .flex()
                        .flex_col()
                        .py(px(6.0))
                        .bg(rgb(0x292a2e))
                        .border_1()
                        .border_color(rgb(0x414349))
                        .rounded(px(8.0))
                        .shadow_lg()
                        .occlude()
                        .on_any_mouse_down(|_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(
                            div()
                                .px(px(12.0))
                                .pt(px(3.0))
                                .pb(px(6.0))
                                .overflow_hidden()
                                .text_color(rgb(0xf1f3f4))
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(layer_action_menu_item(
                            menu.target,
                            "Duplicate",
                            shortcut_for(keymap, EditorAction::Duplicate),
                            availability.duplicate,
                            Duplicate,
                            cx,
                        ))
                        .child(layer_action_menu_item(
                            menu.target,
                            "Group Selection",
                            shortcut_for(keymap, EditorAction::Group),
                            availability.group,
                            Group,
                            cx,
                        ))
                        .child(layer_action_menu_item(
                            menu.target,
                            "Ungroup Selection",
                            shortcut_for(keymap, EditorAction::Ungroup),
                            availability.ungroup,
                            Ungroup,
                            cx,
                        ))
                        .child(layer_menu_separator())
                        .child(layer_action_menu_item(
                            menu.target,
                            "Delete",
                            shortcut_for(keymap, EditorAction::Delete),
                            availability.delete,
                            Delete,
                            cx,
                        )),
                ),
        )
}

fn shortcut_for(keymap: &Keymap, action: EditorAction) -> SharedString {
    keymap
        .shortcut_label(CommandTarget::Editor(action))
        .unwrap_or_default()
        .into()
}

fn layer_action_menu_item<A: Action + Clone>(
    target: NodeId,
    label: &'static str,
    shortcut: SharedString,
    enabled: bool,
    action: A,
    cx: &mut Context<Strek>,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!(
            "layer-menu-item-{}",
            label.replace(' ', "-").to_lowercase()
        )))
        .h(px(28.0))
        .mx(px(5.0))
        .px(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .rounded(px(4.0))
        .opacity(if enabled { 1.0 } else { 0.35 })
        .text_size(px(11.0))
        .text_color(rgb(0xf1f3f4))
        .child(div().flex_1().child(label))
        .child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(0xa8abb2))
                .child(shortcut),
        )
        .when(enabled, |item| {
            item.cursor_pointer()
                .hover(|style| style.bg(rgb(0x35363b)))
                .on_click(cx.listener(move |editor, _, window, cx| {
                    editor.layer_context_menu = None;
                    if !editor.editor.selection().contains(target) {
                        editor.editor.select_layer(target, false);
                    }
                    cx.stop_propagation();
                    window.dispatch_action(Box::new(action.clone()), cx.borrow_mut());
                    cx.notify();
                }))
        })
}

fn layer_menu_separator() -> impl IntoElement {
    div().h(px(1.0)).mx(px(8.0)).my(px(4.0)).bg(rgb(0x414349))
}

/// Render the persistent design inspector on the right side of the window.
pub fn render_design_panel(
    properties: PropertiesSnapshot,
    width: f32,
    editor_entity: WeakEntity<Strek>,
) -> impl IntoElement {
    div()
        .id("design-panel")
        .relative()
        .w(px(width))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(rgb(0x202124))
        .border_l_1()
        .border_color(rgb(0x414349))
        .child(sidebar_section_header("Design", Icon::Properties))
        .child(properties_panel::render(properties, editor_entity))
        .child(panel_resize_handle(PanelSide::Design))
}

fn panel_resize_handle(side: PanelSide) -> impl IntoElement {
    let handle = div()
        .id(SharedString::from(match side {
            PanelSide::Layers => "layers-panel-resize-handle",
            PanelSide::Design => "design-panel-resize-handle",
        }))
        .absolute()
        .top_0()
        .h_full()
        .w(px(6.0))
        .cursor_col_resize()
        .occlude()
        .hover(|style| style.bg(rgba(0x0c8ce955)))
        .on_drag(PanelResizeDrag(side), |drag, _, _, cx| {
            cx.stop_propagation();
            cx.new(|_| drag.clone())
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        });

    match side {
        PanelSide::Layers => handle.right_0(),
        PanelSide::Design => handle.left_0(),
    }
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
    cx: &mut Context<Strek>,
) -> impl IntoElement {
    let is_renaming = name_input.is_some();
    let indent = entry.depth as f32 * LAYER_INDENT_STEP;
    let indent_guides = (0..entry.depth).map(|depth| {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(LAYER_ROW_PADDING_LEFT
                + LAYER_EXPAND_WIDTH / 2.0
                + depth as f32 * LAYER_INDENT_STEP))
            .w(px(1.0))
            .bg(rgba(0xffffff12))
    });
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

    let expand_icon = entry.has_children.then_some(if entry.expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    });
    let layer_id = entry.id;
    let drag_payload = DraggedLayer {
        id: entry.id,
        name: entry.name.clone().into(),
    };
    let drop_target = entry.id;
    let expand_id = entry.id;
    let visibility_id = entry.id;
    let lock_id = entry.id;
    let context_menu_id = entry.id;
    let more_menu_id = entry.id;
    let context_menu_enabled = entry.editable;
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
        .relative()
        .h(px(LAYER_ROW_HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .pl(px(LAYER_ROW_PADDING_LEFT + indent))
        .pr(px(6.0))
        .gap(px(4.0))
        .bg(bg)
        .hover(|s| s.bg(hover_bg))
        .cursor_pointer()
        .when(entry.selected, |row| {
            row.border_l_2().border_color(rgb(0x0c8ce9))
        })
        .text_color(text_color)
        .text_size(px(12.0))
        .when(entry.editable, |row| {
            row.on_drag(drag_payload, |dragged, _, _, cx| {
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
            .on_drop(cx.listener(
                move |editor, dragged: &DraggedLayer, _window, cx| {
                    if editor.drop_layer_on(dragged.id, drop_target) {
                        cx.notify();
                    }
                    cx.stop_propagation();
                },
            ))
        })
        .when(context_menu_enabled, |row| {
            row.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |editor, event: &MouseDownEvent, _window, cx| {
                    editor.open_layer_context_menu(
                        context_menu_id,
                        event.position,
                        LayerMenuInvocation::ContextClick,
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
        })
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
        .children(indent_guides)
        // Expand/collapse chevron
        .child(
            div()
                .id(SharedString::from(format!("expand-{:?}", entry.id)))
                .w(px(LAYER_EXPAND_WIDTH))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .when_some(expand_icon, |chevron, kind| {
                    chevron.child(icon(kind, 13.0, rgb(0x8f929a)))
                })
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
                .min_w(px(0.0))
                .flex_1()
                .relative()
                .top(px(1.0))
                .overflow_hidden()
                .when_some(name_input, |name, input| name.child(input))
                .when(!is_renaming, |name| {
                    name.text_ellipsis()
                        .whitespace_nowrap()
                        .child(entry.name.clone())
                }),
        )
        // Explicit context-menu trigger for discoverability.
        .child(
            div()
                .id(SharedString::from(format!("more-{:?}", entry.id)))
                .w(px(22.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .opacity(if context_menu_enabled { 0.55 } else { 0.3 })
                .child(icon(
                    Icon::MoreVertical,
                    15.0,
                    rgb(if context_menu_enabled {
                        0xa8abb2
                    } else {
                        0x666971
                    }),
                ))
                .when(context_menu_enabled, |trigger| {
                    trigger
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x35363b)).opacity(1.0))
                        .tooltip(editor_tooltip("Layer actions", None))
                })
                .when(!context_menu_enabled, |trigger| {
                    trigger.tooltip(editor_tooltip(
                        "Show and unlock the layer to use actions",
                        None,
                    ))
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .when(context_menu_enabled, |trigger| {
                    trigger.on_click(cx.listener(
                        move |editor, event: &gpui::ClickEvent, _window, cx| {
                            editor.open_layer_context_menu(
                                more_menu_id,
                                event.down.position,
                                LayerMenuInvocation::MoreButton,
                                cx,
                            );
                            cx.stop_propagation();
                        },
                    ))
                }),
        )
        // Visibility toggle
        .child(
            div()
                .id(SharedString::from(format!("visibility-{:?}", entry.id)))
                .w(px(22.0))
                .h(px(22.0))
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
                .w(px(22.0))
                .h(px(22.0))
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

#[cfg(test)]
mod tests {
    use editor_core::{Document, Editor, Node, PathData};

    use super::{
        context_selection_policy, layer_menu_availability, ContextSelectionPolicy,
        LayerMenuAvailability,
    };

    #[test]
    fn context_selection_preserves_multiselect_and_replaces_only_editable_targets() {
        assert_eq!(
            context_selection_policy(true, true),
            ContextSelectionPolicy::Preserve
        );
        assert_eq!(
            context_selection_policy(true, false),
            ContextSelectionPolicy::Preserve
        );
        assert_eq!(
            context_selection_policy(false, true),
            ContextSelectionPolicy::SelectTarget
        );
        assert_eq!(
            context_selection_policy(false, false),
            ContextSelectionPolicy::TargetUnavailable
        );
    }

    #[test]
    fn menu_actions_target_an_unselected_editable_layer_without_grouping_it() {
        let mut document = Document::new();
        let first = document
            .add_child(
                document.root,
                Node::shape("First", PathData::rect(0.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let second = document
            .add_child(
                document.root,
                Node::shape("Second", PathData::rect(20.0, 0.0, 10.0, 10.0)),
            )
            .unwrap();
        let mut editor = Editor::from_document(document);
        editor.select_layer(first, false);

        assert_eq!(
            layer_menu_availability(&editor, second),
            LayerMenuAvailability {
                duplicate: true,
                group: false,
                ungroup: false,
                delete: true,
            }
        );

        editor.select_layer(second, true);
        assert_eq!(
            layer_menu_availability(&editor, first),
            LayerMenuAvailability {
                duplicate: true,
                group: true,
                ungroup: false,
                delete: true,
            }
        );
    }
}
