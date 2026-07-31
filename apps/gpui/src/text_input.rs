//! Native IME bridge for semantic text editing on the canvas.

use std::ops::Range;

use gpui::{
    div, point, relative, App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, GlobalElementId, IntoElement, LayoutId, ParentElement, Pixels, Style,
    Styled, UTF16Selection, Window,
};

use crate::VectorEditor;

pub fn canvas_text_input(editor: Entity<VectorEditor>) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .child(CanvasTextInputElement { editor })
}

struct CanvasTextInputElement {
    editor: Entity<VectorEditor>,
}

impl IntoElement for CanvasTextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CanvasTextInputElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let active = self.editor.read(cx).editor.text_input_snapshot().is_some();
        if active {
            let focus_handle = self.editor.read(cx).focus_handle.clone();
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.editor.clone()),
                cx,
            );
            self.editor.update(cx, |editor, _| {
                editor.canvas_input_bounds = Some(bounds);
            });
        }
    }
}

impl EntityInputHandler for VectorEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let snapshot = self.editor.text_input_snapshot()?;
        let range = range_from_utf16(&snapshot.text, &range_utf16);
        actual_range.replace(range_to_utf16(&snapshot.text, &range));
        snapshot.text.get(range).map(ToOwned::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let snapshot = self.editor.text_input_snapshot()?;
        Some(UTF16Selection {
            range: range_to_utf16(&snapshot.text, &snapshot.selection),
            reversed: snapshot.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let snapshot = self.editor.text_input_snapshot()?;
        snapshot
            .marked
            .map(|range| range_to_utf16(&snapshot.text, &range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.editor.set_marked_text_range(None);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.editor.text_input_snapshot() else {
            return;
        };
        let range = range_utf16.map(|range| range_from_utf16(&snapshot.text, &range));
        if self.editor.replace_text(range, new_text) {
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.editor.text_input_snapshot() else {
            return;
        };
        let replacement_range = range_utf16
            .as_ref()
            .map(|range| range_from_utf16(&snapshot.text, range))
            .or(snapshot.marked)
            .unwrap_or(snapshot.selection);
        let insertion_start = replacement_range.start;
        if !self.editor.replace_text(Some(replacement_range), new_text) {
            return;
        }
        let inserted = insertion_start..insertion_start + new_text.len();
        if let Some(selected_utf16) = new_selected_range_utf16 {
            let selected = range_from_utf16(new_text, &selected_utf16);
            self.editor.set_text_selection(
                insertion_start + selected.start..insertion_start + selected.end,
            );
        } else {
            self.editor.set_text_selection(inserted.end..inserted.end);
        }
        // Selection updates clear stale composition state, so install the new
        // marked range only after positioning the composition selection.
        self.editor.set_marked_text_range(Some(inserted));
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let snapshot = self.editor.text_input_snapshot()?;
        let range = range_from_utf16(&snapshot.text, &range_utf16);
        let bounds = self.editor.text_range_screen_bounds(range)?;
        Some(Bounds::from_corners(
            point(
                element_bounds.left() + gpui::px(bounds.min.x),
                element_bounds.top() + gpui::px(bounds.min.y),
            ),
            point(
                element_bounds.left() + gpui::px(bounds.max.x),
                element_bounds.top() + gpui::px(bounds.max.y),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.canvas_input_bounds?;
        let snapshot = self.editor.text_input_snapshot()?;
        let local = glam::Vec2::new(
            (position.x - bounds.left()).0,
            (position.y - bounds.top()).0,
        );
        let utf8 = self.editor.text_index_at_screen_point(local)?;
        Some(offset_to_utf16(&snapshot.text, utf8))
    }
}

fn offset_from_utf16(text: &str, offset: usize) -> usize {
    let mut utf8 = 0usize;
    let mut utf16 = 0usize;
    for character in text.chars() {
        if utf16 >= offset {
            break;
        }
        utf8 += character.len_utf8();
        utf16 += character.len_utf16();
    }
    utf8
}

fn offset_to_utf16(text: &str, offset: usize) -> usize {
    let mut utf8 = 0usize;
    let mut utf16 = 0usize;
    for character in text.chars() {
        if utf8 >= offset {
            break;
        }
        utf8 += character.len_utf8();
        utf16 += character.len_utf16();
    }
    utf16
}

fn range_from_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    offset_from_utf16(text, range.start)..offset_from_utf16(text, range.end)
}

fn range_to_utf16(text: &str, range: &Range<usize>) -> Range<usize> {
    offset_to_utf16(text, range.start)..offset_to_utf16(text, range.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_round_trip_unicode() {
        let text = "a🙂é";
        for utf8 in [0, 1, 5, 7] {
            assert_eq!(offset_from_utf16(text, offset_to_utf16(text, utf8)), utf8);
        }
    }
}
