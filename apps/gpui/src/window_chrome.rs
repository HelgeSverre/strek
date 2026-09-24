//! Client-side window decorations ([`Decorations::Client`]) for Linux compositors that leave them to the app.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    div, point, prelude::*, px, rgb, rgba, size, Bounds, CursorStyle, Decorations, Div,
    MouseButton, Pixels, ResizeEdge, SharedString, Size, Stateful, Tiling, Window, WindowControls,
    WindowDecorations,
};

use crate::{
    assets::{icon, Icon},
    toolbar::editor_tooltip,
    CloseWindow,
};

/// Thickness of the invisible resize strip along each untiled window edge.
pub(crate) const RESIZE_BORDER: f32 = 5.0;
/// Length of each corner handle along both adjoining edges.
pub(crate) const RESIZE_CORNER: f32 = 12.0;

const BUTTON_ICON: u32 = 0xf1f3f4;
const BUTTON_HOVER: u32 = 0x35363b;
const CLOSE_HOVER: u32 = 0xc42b1c;

/// Decoration mode requested when opening the editor window.
///
/// Linux always asks for client-side decorations so Wayland and X11 behave the
/// same. Other platforms keep GPUI's native frame.
pub(crate) fn requested_window_decorations() -> Option<WindowDecorations> {
    if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        Some(WindowDecorations::Client)
    } else {
        None
    }
}

/// Snapshot of the window state needed to draw client-side decorations.
#[derive(Clone)]
pub(crate) struct WindowChrome {
    pub controls: WindowControls,
    pub maximized: bool,
    pub tiling: Tiling,
    /// Set by a left press inside a drag region, consumed by the next mouse move.
    drag_armed: Rc<Cell<bool>>,
}

impl WindowChrome {
    /// Returns `None` when the platform draws the window frame itself.
    pub(crate) fn for_window(window: &Window, drag_armed: Rc<Cell<bool>>) -> Option<Self> {
        match window.window_decorations() {
            Decorations::Server => None,
            Decorations::Client { tiling } => Some(Self {
                controls: window.window_controls(),
                maximized: window.is_maximized(),
                tiling,
                drag_armed,
            }),
        }
    }
}

/// Makes `element` behave like a native title bar when decorations are client-side:
/// drag to move, double click to maximize or restore, right click for the window menu.
///
/// Apply it only to header regions without their own buttons so clicks on controls
/// never start a window move.
pub(crate) fn drag_region(element: Stateful<Div>, chrome: Option<&WindowChrome>) -> Stateful<Div> {
    let Some(chrome) = chrome else {
        return element;
    };
    let arm = Rc::clone(&chrome.drag_armed);
    let consume = Rc::clone(&chrome.drag_armed);
    let release = Rc::clone(&chrome.drag_armed);
    let release_out = Rc::clone(&chrome.drag_armed);
    element
        .on_mouse_down(MouseButton::Left, move |_, _, _| arm.set(true))
        .on_mouse_up(MouseButton::Left, move |_, _, _| release.set(false))
        .on_mouse_down_out(move |_, _, _| release_out.set(false))
        .on_mouse_move(move |_, window, _| {
            if consume.replace(false) {
                window.start_window_move();
            }
        })
        .on_click(|event, window, _| {
            if event.up.click_count == 2 {
                window.zoom_window();
            }
        })
        .when(chrome.controls.window_menu, |element| {
            element.on_mouse_down(MouseButton::Right, |event, window, cx| {
                cx.stop_propagation();
                window.show_window_menu(event.position);
            })
        })
}

/// Minimize, maximize/restore, and close buttons for the right end of the header.
pub(crate) fn render_window_controls(chrome: &WindowChrome) -> impl IntoElement {
    let (zoom_icon, zoom_label) = if chrome.maximized {
        (Icon::WindowRestore, "Restore")
    } else {
        (Icon::WindowMaximize, "Maximize")
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .when(chrome.controls.minimize, |controls| {
            controls.child(window_control_button(
                "minimize",
                Icon::Minus,
                "Minimize",
                BUTTON_HOVER,
                |window, _| window.minimize_window(),
            ))
        })
        .when(chrome.controls.maximize, |controls| {
            controls.child(window_control_button(
                "maximize",
                zoom_icon,
                zoom_label,
                BUTTON_HOVER,
                |window, _| window.zoom_window(),
            ))
        })
        .child(window_control_button(
            "close",
            Icon::Close,
            "Close",
            CLOSE_HOVER,
            |window, cx| window.dispatch_action(Box::new(CloseWindow), cx),
        ))
}

fn window_control_button(
    id: &'static str,
    icon_kind: Icon,
    label: &'static str,
    hover: u32,
    on_press: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("window-control-{id}")))
        .w(px(30.0))
        .h(px(30.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(rgba(0x00000000))
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(hover)))
        .child(icon(icon_kind, 16.0, rgb(BUTTON_ICON)))
        .tooltip(editor_tooltip(label, None))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_press(window, cx);
        })
}

/// Window-space hit areas for the resize handles of a client-decorated window.
///
/// Tiled edges get no strip, and a corner exists only when both adjoining edges
/// are free, so maximized or snapped windows cannot be dragged from a docked side.
pub(crate) fn resize_handles(
    window_size: Size<Pixels>,
    tiling: Tiling,
) -> Vec<(ResizeEdge, Bounds<Pixels>)> {
    let width = window_size.width.0;
    let height = window_size.height.0;
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let border = RESIZE_BORDER.min(width / 2.0).min(height / 2.0);
    let corner = RESIZE_CORNER.min(width / 2.0).min(height / 2.0);
    let rect =
        |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
    let mut handles = Vec::with_capacity(8);

    if !tiling.top {
        handles.push((ResizeEdge::Top, rect(0.0, 0.0, width, border)));
    }
    if !tiling.bottom {
        handles.push((
            ResizeEdge::Bottom,
            rect(0.0, height - border, width, border),
        ));
    }
    if !tiling.left {
        handles.push((ResizeEdge::Left, rect(0.0, 0.0, border, height)));
    }
    if !tiling.right {
        handles.push((ResizeEdge::Right, rect(width - border, 0.0, border, height)));
    }
    // Corners come last so they sit above the edge strips they overlap.
    if !tiling.top && !tiling.left {
        handles.push((ResizeEdge::TopLeft, rect(0.0, 0.0, corner, corner)));
    }
    if !tiling.top && !tiling.right {
        handles.push((
            ResizeEdge::TopRight,
            rect(width - corner, 0.0, corner, corner),
        ));
    }
    if !tiling.bottom && !tiling.left {
        handles.push((
            ResizeEdge::BottomLeft,
            rect(0.0, height - corner, corner, corner),
        ));
    }
    if !tiling.bottom && !tiling.right {
        handles.push((
            ResizeEdge::BottomRight,
            rect(width - corner, height - corner, corner, corner),
        ));
    }
    handles
}

/// Returns the handle that receives a press at `position`, honoring paint order.
#[cfg(test)]
fn resize_edge_at(
    position: gpui::Point<Pixels>,
    window_size: Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    resize_handles(window_size, tiling)
        .into_iter()
        .rev()
        .find(|(_, bounds)| bounds.contains(&position))
        .map(|(edge, _)| edge)
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Invisible resize strips painted above all editor content.
///
/// Each strip owns an opaque hitbox, so the canvas below neither sees the press
/// nor overrides the resize cursor.
pub(crate) fn render_resize_handles(
    window_size: Size<Pixels>,
    chrome: &WindowChrome,
) -> Option<impl IntoElement> {
    if chrome.maximized {
        return None;
    }
    let handles = resize_handles(window_size, chrome.tiling);
    if handles.is_empty() {
        return None;
    }
    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .children(handles.into_iter().map(|(edge, bounds)| {
                div()
                    .id(SharedString::from(format!("window-resize-{edge:?}")))
                    .absolute()
                    .left(bounds.origin.x)
                    .top(bounds.origin.y)
                    .w(bounds.size.width)
                    .h(bounds.size.height)
                    .cursor(resize_cursor(edge))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        cx.stop_propagation();
                        window.start_window_resize(edge);
                    })
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Size<Pixels> {
        size(px(800.0), px(600.0))
    }

    fn at(x: f32, y: f32) -> gpui::Point<Pixels> {
        point(px(x), px(y))
    }

    #[test]
    fn free_window_resolves_every_edge_and_corner() {
        let tiling = Tiling::default();
        let cases = [
            (at(400.0, 1.0), Some(ResizeEdge::Top)),
            (at(400.0, 598.0), Some(ResizeEdge::Bottom)),
            (at(1.0, 300.0), Some(ResizeEdge::Left)),
            (at(798.0, 300.0), Some(ResizeEdge::Right)),
            (at(1.0, 1.0), Some(ResizeEdge::TopLeft)),
            (at(10.0, 2.0), Some(ResizeEdge::TopLeft)),
            (at(798.0, 1.0), Some(ResizeEdge::TopRight)),
            (at(1.0, 598.0), Some(ResizeEdge::BottomLeft)),
            (at(790.0, 598.0), Some(ResizeEdge::BottomRight)),
            (at(400.0, 300.0), None),
            (at(400.0, RESIZE_BORDER + 0.5), None),
            (at(RESIZE_BORDER + 0.5, 300.0), None),
        ];
        for (position, expected) in cases {
            assert_eq!(
                resize_edge_at(position, window(), tiling),
                expected,
                "position {position:?}"
            );
        }
    }

    #[test]
    fn tiled_edges_have_no_handles() {
        let tiling = Tiling {
            top: true,
            left: true,
            right: false,
            bottom: false,
        };
        assert_eq!(resize_edge_at(at(400.0, 1.0), window(), tiling), None);
        assert_eq!(resize_edge_at(at(1.0, 300.0), window(), tiling), None);
        // The top-left corner vanishes entirely, not just its tiled half.
        assert_eq!(resize_edge_at(at(1.0, 1.0), window(), tiling), None);
        // A corner with one tiled side degrades to the free edge.
        assert_eq!(
            resize_edge_at(at(798.0, 1.0), window(), tiling),
            Some(ResizeEdge::Right)
        );
        assert_eq!(
            resize_edge_at(at(1.0, 598.0), window(), tiling),
            Some(ResizeEdge::Bottom)
        );
        assert_eq!(
            resize_edge_at(at(798.0, 598.0), window(), tiling),
            Some(ResizeEdge::BottomRight)
        );
    }

    #[test]
    fn fully_tiled_or_empty_windows_have_no_handles() {
        assert!(resize_handles(window(), Tiling::tiled()).is_empty());
        assert!(resize_handles(size(px(0.0), px(600.0)), Tiling::default()).is_empty());
    }

    #[test]
    fn tiny_windows_keep_handles_inside_the_window() {
        let tiny = size(px(6.0), px(4.0));
        for (edge, bounds) in resize_handles(tiny, Tiling::default()) {
            assert!(
                bounds.origin.x.0 >= 0.0 && bounds.origin.y.0 >= 0.0,
                "{edge:?}"
            );
            assert!(
                bounds.right().0 <= 6.0 && bounds.bottom().0 <= 4.0,
                "{edge:?}"
            );
        }
    }
}
