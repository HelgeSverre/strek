//! In-window prompt dialog used where the platform has no native prompt.
//!
//! GPUI's Linux (X11 and Wayland) windows do not implement native prompts, so
//! `Window::prompt` falls back to GPUI's built-in renderer: a fixed-width white
//! card whose text does not wrap and ignores the application's chrome. This
//! module renders the same prompt contract (a message, optional detail, and
//! ordered answers resolved by index) with the editor's dark chrome, wrapped
//! text that stays inside the window, and keyboard handling.
//!
//! Keyboard: Enter chooses the highlighted answer (initially the first),
//! Tab/Right and Shift-Tab/Left move the highlight, and Escape chooses the last
//! answer. Callers therefore list the cancelling answer last.

use gpui::{
    actions, div, prelude::*, px, rgb, rgba, App, Context, EventEmitter, FocusHandle, Focusable,
    KeyBinding, PromptHandle, PromptLevel, PromptResponse, RenderablePromptHandle, SharedString,
    Window,
};

const CONTEXT: &str = "PromptDialog";
const CARD_WIDTH: f32 = 420.0;
const DETAIL_MAX_HEIGHT: f32 = 240.0;

actions!(
    prompt_dialog,
    [PromptConfirm, PromptCancel, PromptNext, PromptPrevious]
);

/// Routes every `Window::prompt` through [`build_prompt`] on platforms whose
/// windows lack a native prompt. macOS and Windows keep their native dialogs,
/// because a custom prompt builder replaces the platform prompt everywhere.
pub(crate) fn install(cx: &mut App) {
    register_keybindings(cx);
    if cfg!(not(any(target_os = "macos", target_os = "windows"))) {
        cx.set_prompt_builder(build_prompt);
    }
}

fn register_keybindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", PromptConfirm, Some(CONTEXT)),
        KeyBinding::new("escape", PromptCancel, Some(CONTEXT)),
        KeyBinding::new("tab", PromptNext, Some(CONTEXT)),
        KeyBinding::new("right", PromptNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PromptPrevious, Some(CONTEXT)),
        KeyBinding::new("left", PromptPrevious, Some(CONTEXT)),
    ]);
}

pub(crate) fn build_prompt(
    level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    actions: &[&str],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let dialog = cx.new(|cx| PromptDialog {
        level,
        message: message.to_owned().into(),
        detail: detail
            .filter(|detail| !detail.is_empty())
            .map(|detail| detail.to_owned().into()),
        actions: actions
            .iter()
            .map(|action| SharedString::from(action.to_string()))
            .collect(),
        selected: 0,
        focus_handle: cx.focus_handle(),
    });
    handle.with_view(dialog, window, cx)
}

pub(crate) struct PromptDialog {
    level: PromptLevel,
    message: SharedString,
    detail: Option<SharedString>,
    actions: Vec<SharedString>,
    selected: usize,
    focus_handle: FocusHandle,
}

impl PromptDialog {
    fn respond(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PromptResponse(index));
        // GPUI detaches the prompt without scheduling a frame, so a prompt
        // whose caller does not otherwise redraw would stay painted.
        window.refresh();
    }

    fn confirm(&mut self, _: &PromptConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions.is_empty() {
            self.respond(self.selected, window, cx);
        }
    }

    fn cancel(&mut self, _: &PromptCancel, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(last) = self.actions.len().checked_sub(1) {
            self.respond(last, window, cx);
        }
    }

    fn next(&mut self, _: &PromptNext, _: &mut Window, cx: &mut Context<Self>) {
        if !self.actions.is_empty() {
            self.selected = (self.selected + 1) % self.actions.len();
            cx.notify();
        }
    }

    fn previous(&mut self, _: &PromptPrevious, _: &mut Window, cx: &mut Context<Self>) {
        if !self.actions.is_empty() {
            self.selected = (self.selected + self.actions.len() - 1) % self.actions.len();
            cx.notify();
        }
    }

    fn accent(&self) -> u32 {
        match self.level {
            PromptLevel::Info => 0x0c8ce9,
            PromptLevel::Warning => 0xf2b33d,
            PromptLevel::Critical => 0xf0616d,
        }
    }
}

impl EventEmitter<PromptResponse> for PromptDialog {}

impl Focusable for PromptDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PromptDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons = self.actions.iter().enumerate().map(|(index, label)| {
            let selected = index == self.selected;
            div()
                .id(("prompt-action", index))
                .debug_selector(|| format!("prompt-dialog-action-{index}"))
                .h(px(28.0))
                .px(px(14.0))
                .flex()
                .items_center()
                .rounded(px(6.0))
                .border_1()
                .text_size(px(12.0))
                .cursor_pointer()
                .when(selected, |button| {
                    button
                        .bg(rgb(0x0c8ce9))
                        .border_color(rgb(0x3aa3f0))
                        .text_color(rgb(0xffffff))
                })
                .when(!selected, |button| {
                    button
                        .bg(rgb(0x2c2d31))
                        .border_color(rgb(0x46484e))
                        .text_color(rgb(0xe3e5e8))
                        .hover(|style| style.bg(rgb(0x37383d)))
                })
                .child(label.clone())
                .on_click(
                    cx.listener(move |dialog, _, window, cx| dialog.respond(index, window, cx)),
                )
        });

        div()
            .id("prompt-dialog-overlay")
            .size_full()
            .p(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000080))
            .occlude()
            .cursor_default()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::previous))
            .child(
                div()
                    .debug_selector(|| "prompt-dialog-card".to_owned())
                    .w(px(CARD_WIDTH))
                    .p(px(18.0))
                    .overflow_hidden()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(rgb(0x46484e))
                    .bg(rgb(0x202124))
                    .shadow_xl()
                    .child(
                        div()
                            .debug_selector(|| "prompt-dialog-accent".to_owned())
                            .w(px(28.0))
                            .h(px(3.0))
                            .rounded_full()
                            .bg(rgb(self.accent())),
                    )
                    .child(
                        div()
                            .debug_selector(|| "prompt-dialog-message".to_owned())
                            .mt(px(10.0))
                            .w_full()
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(0xf1f3f4))
                            .child(self.message.clone()),
                    )
                    .when_some(self.detail.clone(), |card, detail| {
                        card.child(
                            div()
                                .id("prompt-dialog-detail")
                                .debug_selector(|| "prompt-dialog-detail".to_owned())
                                .mt(px(8.0))
                                .w_full()
                                .max_h(px(DETAIL_MAX_HEIGHT))
                                .overflow_y_scroll()
                                .text_size(px(12.0))
                                .line_height(px(17.0))
                                .text_color(rgb(0xa8abb2))
                                .child(
                                    div()
                                        .debug_selector(|| "prompt-dialog-detail-text".to_owned())
                                        .w_full()
                                        .child(detail),
                                ),
                        )
                    })
                    .child(
                        div()
                            .mt(px(16.0))
                            .w_full()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .justify_end()
                            .gap(px(8.0))
                            .children(buttons),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{size, Bounds, Modifiers, Pixels, TestAppContext, VisualTestContext};

    struct EmptyView;

    impl Render for EmptyView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    const LONG_MESSAGE: &str =
        "Save changes to “A particularly long document name used for wrapping checks.strek”?";
    const LONG_DETAIL: &str = "Your changes will be lost if you continue without saving. \
        The file could not be written to /home/someone/Documents/illustrations/very/deeply/\
        nested/folder/with-an-unbroken-path-segment-that-exceeds-the-card-width.strek";

    type Answer = Rc<Cell<Option<usize>>>;

    fn open_prompt<'a>(
        cx: &'a mut TestAppContext,
        width: f32,
        actions: &'static [&'static str],
    ) -> (&'a mut VisualTestContext, Answer) {
        open_prompt_with(cx, width, LONG_MESSAGE, LONG_DETAIL, actions)
    }

    fn open_prompt_with<'a>(
        cx: &'a mut TestAppContext,
        width: f32,
        message: &'static str,
        detail: &'static str,
        actions: &'static [&'static str],
    ) -> (&'a mut VisualTestContext, Answer) {
        cx.update(|cx| {
            register_keybindings(cx);
            cx.set_prompt_builder(build_prompt);
        });
        let (_, cx) = cx.add_window_view(|_, _| EmptyView);
        cx.simulate_resize(size(px(width), px(600.0)));
        let receiver = cx.update(|window, cx| {
            window.prompt(PromptLevel::Warning, message, Some(detail), actions, cx)
        });
        let answer = Answer::default();
        let slot = answer.clone();
        cx.update(|window, cx| {
            window
                .spawn(cx, async move |_| slot.set(receiver.await.ok()))
                .detach()
        });
        cx.run_until_parked();
        (cx, answer)
    }

    fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not rendered"))
    }

    fn contains(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
        inner.left() >= outer.left()
            && inner.top() >= outer.top()
            && inner.right() <= outer.right()
            && inner.bottom() <= outer.bottom()
    }

    #[gpui::test]
    fn narrow_window_keeps_wrapped_prompt_inside_card(cx: &mut TestAppContext) {
        let (cx, _receiver) = open_prompt(cx, 300.0, &["Save", "Discard", "Cancel"]);

        let card = bounds(cx, "prompt-dialog-card");
        let viewport = Bounds::new(Default::default(), size(px(300.0), px(600.0)));
        assert!(contains(viewport, card), "card {card:?} escapes window");
        assert!(
            card.size.width < px(CARD_WIDTH),
            "card must shrink below its preferred width in a narrow window"
        );

        let message = bounds(cx, "prompt-dialog-message");
        let detail = bounds(cx, "prompt-dialog-detail");
        assert!(
            contains(card, message),
            "message {message:?} overflows {card:?}"
        );
        assert!(
            contains(card, detail),
            "detail {detail:?} overflows {card:?}"
        );
        assert!(
            message.size.height > px(30.0),
            "long message should wrap onto several lines, got {message:?}"
        );
        assert!(
            detail.size.height > px(17.0 * 3.0),
            "long detail should wrap onto several lines, got {detail:?}"
        );
        for selector in [
            "prompt-dialog-action-0",
            "prompt-dialog-action-1",
            "prompt-dialog-action-2",
        ] {
            let button = bounds(cx, selector);
            assert!(
                contains(card, button),
                "{selector} {button:?} overflows {card:?}"
            );
        }
    }

    #[gpui::test]
    fn two_line_detail_is_not_clipped(cx: &mut TestAppContext) {
        // The unsaved-changes detail wraps to exactly two lines in the
        // preferred card width; the detail box must be tall enough for both.
        let (cx, _receiver) = open_prompt_with(
            cx,
            1280.0,
            "Save changes to “Strek Showcase”?",
            "Your changes will be lost if you continue without saving.",
            &["Save", "Discard", "Cancel"],
        );

        let detail = bounds(cx, "prompt-dialog-detail");
        let text = bounds(cx, "prompt-dialog-detail-text");
        assert!(
            text.size.height >= px(17.0 * 2.0),
            "detail should wrap onto two lines, got {text:?}"
        );
        assert!(
            detail.size.height >= text.size.height,
            "detail box {detail:?} clips its text {text:?}"
        );
        let accent = bounds(cx, "prompt-dialog-accent");
        assert_eq!(accent.size.height, px(3.0), "card squeezed its accent bar");
    }

    #[gpui::test]
    fn wide_window_uses_preferred_card_width(cx: &mut TestAppContext) {
        let (cx, _receiver) = open_prompt(cx, 1000.0, &["OK"]);

        let card = bounds(cx, "prompt-dialog-card");
        assert_eq!(card.size.width, px(CARD_WIDTH));
        assert!(card.left() > px(0.0) && card.right() < px(1000.0));
    }

    #[gpui::test]
    fn enter_confirms_first_answer(cx: &mut TestAppContext) {
        let (cx, receiver) = open_prompt(cx, 800.0, &["Save", "Discard", "Cancel"]);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(receiver.get(), Some(0));
        assert!(cx.debug_bounds("prompt-dialog-card").is_none());
    }

    #[gpui::test]
    fn escape_chooses_last_answer(cx: &mut TestAppContext) {
        let (cx, receiver) = open_prompt(cx, 800.0, &["Save", "Discard", "Cancel"]);
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(receiver.get(), Some(2));
    }

    #[gpui::test]
    fn tab_and_arrows_move_highlight_before_confirming(cx: &mut TestAppContext) {
        let (cx, receiver) = open_prompt(cx, 800.0, &["Save", "Discard", "Cancel"]);
        cx.simulate_keystrokes("tab tab left shift-tab shift-tab right");
        cx.run_until_parked();
        assert_eq!(receiver.get(), None);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        // 0 → 1 → 2 → 1 → 0 → 2 (wraps back) → 0 (wraps forward)
        assert_eq!(receiver.get(), Some(0));
    }

    #[gpui::test]
    fn moving_highlight_changes_confirmed_answer(cx: &mut TestAppContext) {
        let (cx, receiver) = open_prompt(cx, 800.0, &["Save", "Discard", "Cancel"]);
        cx.simulate_keystrokes("right enter");
        cx.run_until_parked();
        assert_eq!(receiver.get(), Some(1));
    }

    #[gpui::test]
    fn clicking_an_answer_resolves_its_index(cx: &mut TestAppContext) {
        let (cx, receiver) = open_prompt(cx, 800.0, &["Save", "Discard", "Cancel"]);
        let discard = bounds(cx, "prompt-dialog-action-1");
        cx.simulate_click(discard.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(receiver.get(), Some(1));
    }
}
