//! Web frontend for the vector editor.
//!
//! This crate provides a WASM-based web frontend using SVG rendering.

use std::cell::RefCell;
use std::rc::Rc;

use editor_core::{Editor, InputEvent, Key, LayerEntry, LayerIcon, Modifiers, MouseButton, NodeId};
use glam::Vec2;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, KeyboardEvent, MouseEvent, PointerEvent, WheelEvent, Window};

/// Global editor state wrapped for sharing across callbacks.
type EditorRef = Rc<RefCell<Editor>>;

/// Shared canvas dimensions
type DimensionsRef = Rc<RefCell<(f32, f32)>>;

/// Cached layer entries for click handling (maps index to NodeId).
type LayerEntriesRef = Rc<RefCell<Vec<LayerEntry>>>;

/// Initialize the editor when the WASM module loads.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    // Set up panic hook for better error messages
    console_error_panic_hook::set_once();

    log("Vector Editor starting...");

    let window = web_sys::window().expect("no window");
    let document = window.document().expect("no document");

    // Find the canvas container (where SVG goes)
    let canvas_container = document
        .get_element_by_id("canvas-container")
        .expect("no canvas-container element");

    // Get initial viewport dimensions
    let (width, height) = get_container_size(&canvas_container);
    let dimensions = Rc::new(RefCell::new((width, height)));

    // Create the editor with demo content
    let editor = Rc::new(RefCell::new(Editor::with_demo_content()));

    // Create layer entries cache for click handling
    let layer_entries: LayerEntriesRef = Rc::new(RefCell::new(Vec::new()));

    // Create SVG element
    let svg = create_svg(&document, width, height)?;
    canvas_container.append_child(&svg)?;

    // Set up event handlers
    setup_pointer_events(&svg, editor.clone())?;
    setup_keyboard_events(&window, editor.clone())?;
    setup_wheel_events(&svg, editor.clone())?;
    setup_resize_events(&window, &svg, editor.clone(), dimensions.clone())?;
    setup_layer_panel_events(&document, editor.clone(), layer_entries.clone())?;

    // Initial render
    render(&svg, &mut editor.borrow_mut());
    render_layers(&document, &editor.borrow(), &layer_entries);

    // Start animation loop
    start_animation_loop(window, document, svg, editor, dimensions, layer_entries)?;

    log("Vector Editor initialized");

    Ok(())
}

/// Get the size of the container element.
fn get_container_size(container: &Element) -> (f32, f32) {
    let rect = container.get_bounding_client_rect();
    (rect.width() as f32, rect.height() as f32)
}

/// Create the SVG element for rendering.
fn create_svg(document: &Document, width: f32, height: f32) -> Result<Element, JsValue> {
    let svg = document.create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")?;

    svg.set_attribute("width", "100%")?;
    svg.set_attribute("height", "100%")?;
    svg.set_attribute("viewBox", &format!("0 0 {} {}", width, height))?;
    svg.set_attribute("style", "touch-action: none; cursor: default;")?;
    svg.set_attribute("tabindex", "0")?; // Make focusable for keyboard events

    Ok(svg)
}

/// Update SVG viewBox for new dimensions.
fn update_svg_viewbox(svg: &Element, width: f32, height: f32) {
    let _ = svg.set_attribute("viewBox", &format!("0 0 {} {}", width, height));
}

/// Set up pointer event handlers.
fn setup_pointer_events(svg: &Element, editor: EditorRef) -> Result<(), JsValue> {
    // Pointer down
    {
        let editor = editor.clone();
        let svg_inner = svg.clone();
        let svg_outer = svg.clone();
        let closure = Closure::wrap(Box::new(move |event: PointerEvent| {
            event.prevent_default();

            // Focus the SVG for keyboard events
            if let Some(el) = svg_inner.dyn_ref::<HtmlElement>() {
                let _ = el.focus();
            }

            let input = pointer_event_to_input(&event, true);
            let effects = editor.borrow_mut().handle_event(input);

            if effects.redraw {
                render(&svg_inner, &mut editor.borrow_mut());
            }

            if let Some(cursor) = effects.cursor {
                let _ = svg_inner.set_attribute(
                    "style",
                    &format!("touch-action: none; cursor: {};", cursor.to_css()),
                );
            }
        }) as Box<dyn FnMut(_)>);

        svg_outer
            .add_event_listener_with_callback("pointerdown", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    // Pointer move
    {
        let editor = editor.clone();
        let svg_inner = svg.clone();
        let svg_outer = svg.clone();
        let closure = Closure::wrap(Box::new(move |event: PointerEvent| {
            let input = InputEvent::PointerMove {
                position: pointer_position(&event),
                modifiers: modifiers_from_pointer(&event),
            };

            let effects = editor.borrow_mut().handle_event(input);

            if effects.redraw {
                render(&svg_inner, &mut editor.borrow_mut());
            }

            if let Some(cursor) = effects.cursor {
                let _ = svg_inner.set_attribute(
                    "style",
                    &format!("touch-action: none; cursor: {};", cursor.to_css()),
                );
            }
        }) as Box<dyn FnMut(_)>);

        svg_outer
            .add_event_listener_with_callback("pointermove", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    // Pointer up
    {
        let editor = editor.clone();
        let svg_inner = svg.clone();
        let svg_outer = svg.clone();
        let closure = Closure::wrap(Box::new(move |event: PointerEvent| {
            let input = pointer_event_to_input(&event, false);
            let effects = editor.borrow_mut().handle_event(input);

            if effects.redraw {
                render(&svg_inner, &mut editor.borrow_mut());
            }

            if let Some(cursor) = effects.cursor {
                let _ = svg_inner.set_attribute(
                    "style",
                    &format!("touch-action: none; cursor: {};", cursor.to_css()),
                );
            }
        }) as Box<dyn FnMut(_)>);

        svg_outer
            .add_event_listener_with_callback("pointerup", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    Ok(())
}

/// Set up keyboard event handlers.
fn setup_keyboard_events(window: &Window, editor: EditorRef) -> Result<(), JsValue> {
    let editor = editor.clone();
    let closure = Closure::wrap(Box::new(move |event: KeyboardEvent| {
        let key = Key::from_js_key(&event.key());
        let modifiers = Modifiers {
            shift: event.shift_key(),
            ctrl: event.ctrl_key(),
            alt: event.alt_key(),
            meta: event.meta_key(),
        };

        // Prevent default for our shortcuts
        if modifiers.primary() && matches!(key, Key::Z | Key::Y | Key::A | Key::G | Key::D) {
            event.prevent_default();
        }

        let input = InputEvent::KeyDown { key, modifiers };
        let _effects = editor.borrow_mut().handle_event(input);

        // Note: We'll redraw in the animation loop
    }) as Box<dyn FnMut(_)>);

    window.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())?;
    closure.forget();

    Ok(())
}

/// Set up wheel event handlers for zooming and scrolling.
fn setup_wheel_events(svg: &Element, editor: EditorRef) -> Result<(), JsValue> {
    let editor = editor.clone();
    let svg_inner = svg.clone();
    let svg_outer = svg.clone();
    let closure = Closure::wrap(Box::new(move |event: WheelEvent| {
        event.prevent_default();

        let position = Vec2::new(event.offset_x() as f32, event.offset_y() as f32);
        let delta = Vec2::new(event.delta_x() as f32, event.delta_y() as f32);
        let modifiers = Modifiers {
            shift: event.shift_key(),
            ctrl: event.ctrl_key(),
            alt: event.alt_key(),
            meta: event.meta_key(),
        };

        let input = InputEvent::Scroll {
            position,
            delta,
            modifiers,
        };
        let effects = editor.borrow_mut().handle_event(input);

        if effects.redraw {
            render(&svg_inner, &mut editor.borrow_mut());
        }
    }) as Box<dyn FnMut(_)>);

    svg_outer.add_event_listener_with_callback("wheel", closure.as_ref().unchecked_ref())?;
    closure.forget();

    Ok(())
}

/// Set up window resize event handler.
fn setup_resize_events(
    window: &Window,
    svg: &Element,
    editor: EditorRef,
    dimensions: DimensionsRef,
) -> Result<(), JsValue> {
    let svg = svg.clone();
    let closure = Closure::wrap(Box::new(move || {
        // Get the app container and its new size
        if let Some(window) = web_sys::window() {
            if let Some(document) = window.document() {
                if let Some(container) = document.get_element_by_id("app") {
                    let (width, height) = get_container_size(&container);
                    *dimensions.borrow_mut() = (width, height);
                    update_svg_viewbox(&svg, width, height);
                    render(&svg, &mut editor.borrow_mut());
                }
            }
        }
    }) as Box<dyn FnMut()>);

    window.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())?;
    closure.forget();

    Ok(())
}

/// Start the requestAnimationFrame loop.
fn start_animation_loop(
    window: Window,
    document: Document,
    svg: Element,
    editor: EditorRef,
    dimensions: DimensionsRef,
    layer_entries: LayerEntriesRef,
) -> Result<(), JsValue> {
    // Use a closure that calls itself via requestAnimationFrame
    #[allow(clippy::type_complexity)]
    let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let g = f.clone();

    let window_clone = window.clone();
    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        // Check if we need to redraw
        if editor.borrow_mut().take_redraw() {
            render(&svg, &mut editor.borrow_mut());
            render_layers(&document, &editor.borrow(), &layer_entries);
        }

        // Schedule next frame
        let _ = window_clone
            .request_animation_frame(f.borrow().as_ref().unwrap().as_ref().unchecked_ref());
    }) as Box<dyn FnMut()>));

    // Suppress unused variable warning
    let _ = &dimensions;

    // Start the loop
    window.request_animation_frame(g.borrow().as_ref().unwrap().as_ref().unchecked_ref())?;

    Ok(())
}

/// Render the editor to SVG.
fn render(svg: &Element, editor: &mut Editor) {
    let display_list = editor.build_display_list();
    let svg_content = render_svg::to_svg_fragment(&display_list);
    svg.set_inner_html(&svg_content);
}

/// Convert a pointer event to an input event.
fn pointer_event_to_input(event: &PointerEvent, is_down: bool) -> InputEvent {
    let position = pointer_position(event);
    let button = match event.button() {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::Left,
    };
    let modifiers = modifiers_from_pointer(event);

    if is_down {
        InputEvent::PointerDown {
            position,
            button,
            modifiers,
        }
    } else {
        InputEvent::PointerUp {
            position,
            button,
            modifiers,
        }
    }
}

/// Get the position from a pointer event.
fn pointer_position(event: &PointerEvent) -> Vec2 {
    Vec2::new(event.offset_x() as f32, event.offset_y() as f32)
}

/// Get modifiers from a pointer event.
fn modifiers_from_pointer(event: &PointerEvent) -> Modifiers {
    Modifiers {
        shift: event.shift_key(),
        ctrl: event.ctrl_key(),
        alt: event.alt_key(),
        meta: event.meta_key(),
    }
}

/// Log to the browser console.
fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

// === Layer Panel ===

/// Render the layer panel.
fn render_layers(document: &Document, editor: &Editor, layer_entries: &LayerEntriesRef) {
    let Some(layers_list) = document.get_element_by_id("layers-list") else {
        return;
    };

    let layer_tree = editor.build_layer_tree();

    // Store entries for later lookup by click handlers
    *layer_entries.borrow_mut() = layer_tree.clone();

    let mut html = String::new();

    for (index, entry) in layer_tree.iter().enumerate() {
        // Build indentation
        let indent = entry.depth * 16;

        // Build classes
        let mut classes = vec!["layer-entry"];
        if entry.selected {
            classes.push("selected");
        }
        if !entry.visible {
            classes.push("invisible");
        }

        // Build expand/collapse indicator (use index as data-idx for lookup)
        let expand_html = if entry.is_container && entry.has_children {
            let arrow = if entry.expanded { "&#9662;" } else { "&#9656;" };
            format!(
                r#"<div class="layer-expand has-children" data-idx="{}">{}</div>"#,
                index,
                arrow
            )
        } else {
            r#"<div class="layer-expand"></div>"#.to_string()
        };

        // Build icon
        let icon_char = match entry.icon {
            LayerIcon::Group => "&#128193;", // folder
            LayerIcon::Frame => "&#9634;",   // square
            LayerIcon::Shape => "&#9632;",   // filled square
            LayerIcon::Text => "T",
        };

        // Build visibility button
        let vis_class = if entry.visible { "" } else { "hidden" };
        let vis_icon = if entry.visible { "&#128065;" } else { "&#128064;" }; // eye icons

        html.push_str(&format!(
            r#"<div class="{}" style="padding-left: {}px" data-idx="{}">
                {}
                <div class="layer-icon">{}</div>
                <div class="layer-name">{}</div>
                <div class="layer-visibility {}" data-idx="{}">{}</div>
            </div>"#,
            classes.join(" "),
            indent + 8,
            index,
            expand_html,
            icon_char,
            html_escape(&entry.name),
            vis_class,
            index,
            vis_icon
        ));
    }

    layers_list.set_inner_html(&html);
}

/// Check if an element has a class.
fn has_class(el: &Element, class: &str) -> bool {
    if let Some(class_attr) = el.get_attribute("class") {
        class_attr.split_whitespace().any(|c| c == class)
    } else {
        false
    }
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Set up layer panel event handlers.
fn setup_layer_panel_events(
    document: &Document,
    editor: EditorRef,
    layer_entries: LayerEntriesRef,
) -> Result<(), JsValue> {
    let layers_list = document
        .get_element_by_id("layers-list")
        .ok_or("no layers-list")?;

    // Clone for closures
    let editor_click = editor.clone();
    let document_click = document.clone();
    let entries_click = layer_entries.clone();

    // Handle clicks on the layer list
    let click_handler = Closure::wrap(Box::new(move |event: MouseEvent| {
        let target = event.target();
        if target.is_none() {
            return;
        }

        let target: Element = target.unwrap().dyn_into().unwrap();

        // Helper to get NodeId from index
        let get_node_id = |idx_str: &str| -> Option<NodeId> {
            let idx: usize = idx_str.parse().ok()?;
            let entries = entries_click.borrow();
            entries.get(idx).map(|e| e.id)
        };

        // Check if clicking on visibility toggle
        if has_class(&target, "layer-visibility") {
            if let Some(idx_str) = target.get_attribute("data-idx") {
                if let Some(node_id) = get_node_id(&idx_str) {
                    editor_click.borrow_mut().toggle_visibility(node_id);
                    render_layers(&document_click, &editor_click.borrow(), &entries_click);
                }
            }
            return;
        }

        // Check if clicking on expand/collapse
        if has_class(&target, "layer-expand") {
            if let Some(idx_str) = target.get_attribute("data-idx") {
                if let Some(node_id) = get_node_id(&idx_str) {
                    editor_click.borrow_mut().toggle_layer_expand(node_id);
                    render_layers(&document_click, &editor_click.borrow(), &entries_click);
                }
            }
            return;
        }

        // Otherwise, find the layer-entry and select it
        let mut current = Some(target);
        while let Some(el) = current {
            if has_class(&el, "layer-entry") {
                if let Some(idx_str) = el.get_attribute("data-idx") {
                    if let Some(node_id) = get_node_id(&idx_str) {
                        // Check for shift key for add-to-selection
                        let add_to_selection = event.shift_key();
                        editor_click.borrow_mut().select_layer(node_id, add_to_selection);
                        render_layers(&document_click, &editor_click.borrow(), &entries_click);
                    }
                }
                break;
            }
            current = el.parent_element();
        }
    }) as Box<dyn FnMut(MouseEvent)>);

    layers_list.add_event_listener_with_callback("click", click_handler.as_ref().unchecked_ref())?;
    click_handler.forget();

    Ok(())
}

/// Exposed function to create a new editor (can be called from JS).
#[wasm_bindgen]
pub fn create_editor() -> *mut Editor {
    Box::into_raw(Box::new(Editor::with_demo_content()))
}
