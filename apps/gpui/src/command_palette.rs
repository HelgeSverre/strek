//! Searchable, keyboard-driven command palette.

use std::cmp::Reverse;
use std::ops::Range;

use gpui::{
    actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, App, Bounds,
    ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, IntoElement,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle,
    Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    assets::{icon, Icon},
    commands::CommandTarget,
};

const MAX_VISIBLE_RESULTS: usize = 11;

actions!(
    command_palette,
    [
        PaletteBackspace,
        PaletteDelete,
        PaletteLeft,
        PaletteRight,
        PaletteSelectLeft,
        PaletteSelectRight,
        PaletteSelectAll,
        PaletteHome,
        PaletteEnd,
        PaletteCopy,
        PaletteCut,
        PalettePaste,
        PalettePrevious,
        PaletteNext,
        PaletteConfirm,
        PaletteDismiss,
    ]
);

#[derive(Debug, Clone)]
pub(crate) struct PaletteEntry {
    pub target: CommandTarget,
    pub label: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub recent_rank: Option<usize>,
}

pub(crate) enum CommandPaletteEvent {
    Execute(CommandTarget),
    Dismiss,
}

pub(crate) struct CommandPalette {
    focus_handle: FocusHandle,
    content: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    entries: Vec<PaletteEntry>,
    filtered: Vec<usize>,
    selected_match: usize,
}

impl CommandPalette {
    pub(crate) fn new(entries: Vec<PaletteEntry>, cx: &mut Context<Self>) -> Self {
        let mut palette = Self {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            entries,
            filtered: Vec::new(),
            selected_match: 0,
        };
        palette.refresh_matches();
        palette
    }

    pub(crate) fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn refresh_matches(&mut self) {
        let query = self.content.trim().to_lowercase();
        let mut matches = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let score = if query.is_empty() {
                    Some(0)
                } else {
                    fuzzy_score(
                        &query,
                        &format!("{} {} {}", entry.label, entry.category, entry.description),
                    )
                }?;
                Some((index, score))
            })
            .collect::<Vec<_>>();

        matches.sort_by_key(|(index, score)| {
            let entry = &self.entries[*index];
            (
                !entry.enabled,
                query
                    .is_empty()
                    .then_some(entry.recent_rank)
                    .flatten()
                    .is_none(),
                query
                    .is_empty()
                    .then_some(entry.recent_rank)
                    .flatten()
                    .unwrap_or(usize::MAX),
                Reverse(*score),
                entry.label.to_lowercase(),
            )
        });
        self.filtered = matches.into_iter().map(|(index, _)| index).collect();
        self.selected_match = self
            .filtered
            .iter()
            .position(|index| self.entries[*index].enabled)
            .unwrap_or(0);
    }

    fn move_selection(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        let mut next = self.selected_match as isize;
        for _ in 0..self.filtered.len() {
            next = (next + direction).rem_euclid(len);
            if self.entries[self.filtered[next as usize]].enabled {
                self.selected_match = next as usize;
                cx.notify();
                return;
            }
        }
    }

    fn previous(&mut self, _: &PalettePrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn next(&mut self, _: &PaletteNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn confirm(&mut self, _: &PaletteConfirm, _: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.filtered.get(self.selected_match).copied() else {
            return;
        };
        let entry = &self.entries[index];
        if entry.enabled {
            cx.emit(CommandPaletteEvent::Execute(entry.target));
        }
    }

    fn dismiss(&mut self, _: &PaletteDismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CommandPaletteEvent::Dismiss);
    }

    fn dismiss_from_overlay(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CommandPaletteEvent::Dismiss);
        cx.stop_propagation();
    }

    fn stop_mouse_propagation(
        &mut self,
        _: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
    }

    fn execute_row(
        &mut self,
        index: usize,
        _: &gpui::ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        if entry.enabled {
            cx.emit(CommandPaletteEvent::Execute(entry.target));
        }
        cx.stop_propagation();
    }

    fn left(&mut self, _: &PaletteLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &PaletteRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &PaletteSelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &PaletteSelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &PaletteSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &PaletteHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &PaletteEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &PaletteBackspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &PaletteDelete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn copy(&mut self, _: &PaletteCopy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected) = self.content.get(self.selected_range.clone()) {
            if !selected.is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(selected.to_owned()));
            }
        }
    }

    fn cut(&mut self, _: &PaletteCut, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&PaletteCopy, window, cx);
        self.replace_text_in_range(None, "", window, cx);
    }

    fn paste(&mut self, _: &PalettePaste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace(['\r', '\n'], " "), window, cx);
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
        cx.stop_propagation();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.marked_range = None;
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) else {
            return self.content.len();
        };
        line.closest_index_for_x(position.x - bounds.left())
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        utf8_offset_from_utf16(&self.content, offset)
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for character in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += character.len_utf8();
            utf16_offset += character.len_utf16();
        }
        utf16_offset
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn visible_range(&self) -> Range<usize> {
        if self.filtered.len() <= MAX_VISIBLE_RESULTS {
            return 0..self.filtered.len();
        }
        let half = MAX_VISIBLE_RESULTS / 2;
        let start = self
            .selected_match
            .saturating_sub(half)
            .min(self.filtered.len() - MAX_VISIBLE_RESULTS);
        start..start + MAX_VISIBLE_RESULTS
    }
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

impl EntityInputHandler for CommandPalette {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        self.content.get(range).map(ToOwned::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let replacement = text.replace(['\r', '\n'], " ");
        let mut content = self.content.to_string();
        content.replace_range(range.clone(), &replacement);
        let cursor = range.start + replacement.len();
        self.content = content.into();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.refresh_matches();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let replacement = text.replace(['\r', '\n'], " ");
        let mut content = self.content.to_string();
        content.replace_range(range.clone(), &replacement);
        let marked = range.start..range.start + replacement.len();
        self.content = content.into();
        self.marked_range = Some(marked.clone());
        self.selected_range = selected_utf16
            .map(|selection| {
                range.start + utf8_offset_from_utf16(&replacement, selection.start)
                    ..range.start + utf8_offset_from_utf16(&replacement, selection.end)
            })
            .unwrap_or(marked.end..marked.end);
        self.selection_reversed = false;
        self.refresh_matches();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let line = self.last_layout.as_ref()?;
        Some(self.offset_to_utf16(line.closest_index_for_x(position.x - bounds.left())))
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible_range = self.visible_range();
        let rows = visible_range
            .clone()
            .map(|match_index| {
                let entry_index = self.filtered[match_index];
                let entry = &self.entries[entry_index];
                let selected = match_index == self.selected_match;
                let enabled = entry.enabled;
                let shortcut = entry.shortcut.clone();

                div()
                    .id(SharedString::from(format!("command-row-{}", entry_index)))
                    .h(px(48.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(6.0))
                    .when(selected, |row| row.bg(rgb(0x0c8ce9)))
                    .when(!selected && enabled, |row| {
                        row.hover(|style| style.bg(rgb(0x303136)))
                    })
                    .when(enabled, |row| row.cursor_pointer())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(if enabled { 0xf1f3f4 } else { 0x70737a }))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(entry.label),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(px(9.0))
                                    .text_color(rgb(if selected && enabled {
                                        0xddeeff
                                    } else {
                                        0x92959d
                                    }))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(format!("{} · {}", entry.category, entry.description)),
                            ),
                    )
                    .when_some(shortcut, |row, shortcut| {
                        row.child(
                            div()
                                .px(px(7.0))
                                .py(px(3.0))
                                .rounded(px(4.0))
                                .bg(rgba(if selected { 0xffffff26 } else { 0xffffff0d }))
                                .text_size(px(10.0))
                                .text_color(rgb(if enabled { 0xd8dbe0 } else { 0x666970 }))
                                .child(shortcut),
                        )
                    })
                    .on_click(cx.listener(move |this, event, window, cx| {
                        this.execute_row(entry_index, event, window, cx);
                    }))
            })
            .collect::<Vec<_>>();
        let result_count = self.filtered.len();
        let query_is_empty = self.content.is_empty();

        div()
            .id("command-palette-overlay")
            .absolute()
            .inset_0()
            .pt(px(76.0))
            .flex()
            .items_start()
            .justify_center()
            .bg(rgba(0x00000080))
            .occlude()
            .key_context("CommandPalette")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::dismiss_from_overlay))
            .child(
                div()
                    .id("command-palette")
                    .w(px(560.0))
                    .max_h(px(590.0))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(rgb(0x46484e))
                    .bg(rgb(0x202124))
                    .shadow_xl()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::stop_mouse_propagation))
                    .child(
                        div()
                            .h(px(52.0))
                            .px(px(14.0))
                            .relative()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .border_b_1()
                            .border_color(rgb(0x414349))
                            .text_size(px(14.0))
                            .text_color(rgb(0xf1f3f4))
                            .cursor(CursorStyle::IBeam)
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                            .on_mouse_move(cx.listener(Self::on_mouse_move))
                            .child(
                                div()
                                    .w(px(18.0))
                                    .child(icon(Icon::Search, 16.0, rgb(0x92959d))),
                            )
                            .when(query_is_empty, |input| {
                                input.child(
                                    div()
                                        .absolute()
                                        .left(px(42.0))
                                        .text_color(rgb(0x8f929a))
                                        .child("Search commands…"),
                                )
                            })
                            .child(div().flex_1().h(px(26.0)).overflow_hidden().child(
                                CommandPaletteTextElement {
                                    palette: cx.entity().clone(),
                                },
                            )),
                    )
                    .child(
                        div()
                            .px(px(6.0))
                            .py(px(6.0))
                            .flex()
                            .flex_col()
                            .when(rows.is_empty(), |results| {
                                results.child(
                                    div()
                                        .h(px(92.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(11.0))
                                        .text_color(rgb(0x92959d))
                                        .child("No matching commands"),
                                )
                            })
                            .children(rows),
                    )
                    .child(
                        div()
                            .h(px(30.0))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_t_1()
                            .border_color(rgb(0x414349))
                            .text_size(px(9.0))
                            .text_color(rgb(0x858890))
                            .child(format!("{result_count} commands"))
                            .child("Up/Down navigate   Enter run   Esc close"),
                    ),
            )
    }
}

impl Focusable for CommandPalette {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct CommandPaletteTextElement {
    palette: Entity<CommandPalette>,
}

struct PrepaintState {
    line: ShapedLine,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for CommandPaletteTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CommandPaletteTextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let palette = self.palette.read(cx);
        let content = palette.content.clone();
        let style = window.text_style();
        let run = TextRun {
            len: content.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = &palette.marked_range {
            vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(style.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: content.len() - marked.end,
                    ..run.clone()
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>()
        } else {
            vec![run]
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(content, font_size, &runs)
            .expect("command palette text shaping should succeed");
        let selected = palette.selected_range.clone();
        let cursor = palette.cursor_offset();
        let (selection, cursor) = if selected.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(
                            bounds.left() + line.x_for_index(cursor),
                            bounds.top() + px(2.0),
                        ),
                        size(px(1.0), (bounds.bottom() - bounds.top()) - px(4.0)),
                    ),
                    rgb(0xffffff),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selected.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(0x0c8ce966),
                )),
                None,
            )
        };
        PrepaintState {
            line,
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.palette.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.palette.clone()),
            cx,
        );
        if let Some(selection) = state.selection.take() {
            window.paint_quad(selection);
        }
        state
            .line
            .paint(bounds.origin, window.line_height(), window, cx)
            .expect("command palette text paint should succeed");
        if focus_handle.is_focused(window) {
            if let Some(cursor) = state.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        self.palette.update(cx, |palette, _| {
            palette.last_layout = Some(state.line.clone());
            palette.last_bounds = Some(bounds);
        });
    }
}

pub(crate) fn register_keybindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", PaletteBackspace, Some("CommandPalette")),
        KeyBinding::new("delete", PaletteDelete, Some("CommandPalette")),
        KeyBinding::new("left", PaletteLeft, Some("CommandPalette")),
        KeyBinding::new("right", PaletteRight, Some("CommandPalette")),
        KeyBinding::new("shift-left", PaletteSelectLeft, Some("CommandPalette")),
        KeyBinding::new("shift-right", PaletteSelectRight, Some("CommandPalette")),
        KeyBinding::new("secondary-a", PaletteSelectAll, Some("CommandPalette")),
        KeyBinding::new("home", PaletteHome, Some("CommandPalette")),
        KeyBinding::new("end", PaletteEnd, Some("CommandPalette")),
        KeyBinding::new("secondary-c", PaletteCopy, Some("CommandPalette")),
        KeyBinding::new("secondary-x", PaletteCut, Some("CommandPalette")),
        KeyBinding::new("secondary-v", PalettePaste, Some("CommandPalette")),
        KeyBinding::new("up", PalettePrevious, Some("CommandPalette")),
        KeyBinding::new("down", PaletteNext, Some("CommandPalette")),
        KeyBinding::new("enter", PaletteConfirm, Some("CommandPalette")),
        KeyBinding::new("escape", PaletteDismiss, Some("CommandPalette")),
    ]);
}

fn utf8_offset_from_utf16(text: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for character in text.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += character.len_utf16();
        utf8_offset += character.len_utf8();
    }
    utf8_offset
}

fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let candidate = candidate.to_lowercase();
    let mut candidate_chars = candidate.char_indices();
    let mut previous_match_end = None;
    let mut score = 0;
    for query_char in query.chars().filter(|character| !character.is_whitespace()) {
        let (index, _) = candidate_chars.find(|(_, character)| *character == query_char)?;
        score += 10;
        if previous_match_end == Some(index) {
            score += 8;
        }
        if index == 0
            || candidate[..index]
                .chars()
                .next_back()
                .is_some_and(|character| !character.is_alphanumeric())
        {
            score += 6;
        }
        score -= (index as i32).min(20);
        previous_match_end = Some(index + query_char.len_utf8());
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matching_rewards_contiguous_and_word_prefix_matches() {
        let contiguous = fuzzy_score("zoom", "Zoom to Selection").unwrap();
        let scattered = fuzzy_score("zoom", "Z order opens modes").unwrap();
        let missing = fuzzy_score("zoom", "Rectangle Tool");

        assert!(contiguous > scattered);
        assert!(missing.is_none());
    }

    #[test]
    fn utf16_offsets_are_safe_for_multibyte_queries() {
        assert_eq!(utf8_offset_from_utf16("A🙂é", 0), 0);
        assert_eq!(utf8_offset_from_utf16("A🙂é", 1), 1);
        assert_eq!(utf8_offset_from_utf16("A🙂é", 3), 5);
        assert_eq!(utf8_offset_from_utf16("A🙂é", 4), 7);
    }
}
