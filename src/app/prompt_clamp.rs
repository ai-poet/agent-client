//! Long prompts fold to a few lines until asked for.
//!
//! Fork addition, after Zeron's collapsed user prompts
//! (crates/ui/src/transcript.rs; MIT, Copyright (c) 2026 Wing): a pasted log
//! or file otherwise drops into the transcript as one endless bubble. Past
//! [`COLLAPSE_ABOVE_LINES`] the bubble clips to [`COLLAPSED_LINES`], fades out
//! at its foot, and offers Show more / Show less beneath.
//!
//! Whether a prompt is long is estimated from its text and the bubble's
//! width, never measured: deciding from the laid-out height would change the
//! height it was decided from, and the row would re-measure forever. The
//! estimate errs short on purpose — a prompt it misjudges stays whole.

use std::rc::Rc;

use super::*;

/// The bubble's height while folded, in lines.
const COLLAPSED_LINES: f32 = 5.0;
/// Fold only prompts estimated longer than this, so the fold always hides
/// something worth a click.
const COLLAPSE_ABOVE_LINES: usize = 7;
/// The bubble's line box and font, in `sp` (components.rs).
const LINE_HEIGHT_SP: f32 = 20.0;
const FONT_SP: f32 = 14.0;
/// The bubble's widest and its horizontal padding, in pixels (components.rs).
const BUBBLE_MAX_WIDTH: f32 = 540.0;
const BUBBLE_PADDING_X: f32 = 24.0;
const BUBBLE_PADDING_Y_SP: f32 = 16.0;

/// A user prompt long enough to fold, and what toggling it does.
pub(super) struct PromptClamp {
    message_id: Uuid,
    expanded: bool,
    toggle: Rc<dyn Fn(&mut App)>,
}

impl Waku {
    /// From the transcript's row builder, for a user message on display.
    pub(super) fn prompt_clamp(
        &self,
        message: &Message,
        message_index: usize,
        window: &Window,
        waku: &WeakEntity<Waku>,
    ) -> Option<PromptClamp> {
        if message.role != MessageRole::User {
            return None;
        }
        let width =
            f32::from(self.transcript_layout_width.get()).min(BUBBLE_MAX_WIDTH) - BUBBLE_PADDING_X;
        let font =
            f32::from(window.rem_size()) * FONT_SP / waku_client::persistence::DEFAULT_UI_FONT_SIZE;
        if !self.prompt_is_long(message, width, font) {
            return None;
        }
        let message_id = message.id;
        let expanded = self
            .fork_motion
            .expanded_prompts
            .borrow()
            .contains(&message_id);
        let weak = waku.clone();
        Some(PromptClamp {
            message_id,
            expanded,
            toggle: Rc::new(move |cx: &mut App| {
                let _ = weak.update(cx, |this, cx| {
                    this.toggle_prompt_expanded(message_id, message_index, cx);
                });
            }),
        })
    }

    /// [`estimated_lines`] once per message, width and font: the row builder
    /// runs for every visible row on every frame, and a pasted log is long.
    fn prompt_is_long(&self, message: &Message, width: f32, font: f32) -> bool {
        let content = message.visible_content();
        let key = (content.len(), width.round() as u32, font.round() as u32);
        let mut cache = self.fork_motion.prompt_lengths.borrow_mut();
        if let Some((cached_key, long)) = cache.get(&message.id)
            && *cached_key == key
        {
            return *long;
        }
        let long = estimated_lines(content, width, font) > COLLAPSE_ABOVE_LINES;
        if cache.len() >= 4096 {
            cache.clear();
        }
        cache.insert(message.id, (key, long));
        long
    }

    fn toggle_prompt_expanded(
        &mut self,
        message_id: Uuid,
        message_index: usize,
        cx: &mut Context<Self>,
    ) {
        // Like every disclosure: hold the screen position before the height
        // changes, or a tail-pinned list keeps its bottom edge and the
        // bubble's top jumps.
        self.pin_transcript_for_disclosure();
        {
            let mut expanded = self.fork_motion.expanded_prompts.borrow_mut();
            if !expanded.remove(&message_id) {
                expanded.insert(message_id);
            }
        }
        self.remeasure_transcript_message(message_index);
        cx.notify();
    }
}

/// The user bubble, folded or with its toggle, or as it was.
pub(super) fn clamp_user_bubble(
    bubble: Div,
    clamp: Option<&PromptClamp>,
    theme: &Theme,
) -> AnyElement {
    let Some(clamp) = clamp else {
        return bubble.into_any_element();
    };
    let bubble = if clamp.expanded {
        bubble
    } else {
        let fade = theme.raised;
        bubble
            .max_h(sp(COLLAPSED_LINES * LINE_HEIGHT_SP + BUBBLE_PADDING_Y_SP))
            .overflow_hidden()
            .relative()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(sp(LINE_HEIGHT_SP * 1.5))
                    .bg(linear_gradient(
                        180.0,
                        linear_color_stop(fade.opacity(0.0), 0.0),
                        linear_color_stop(fade, 1.0),
                    )),
            )
    };
    let label = if clamp.expanded {
        tr!("transcript.show_less_prompt")
    } else {
        tr!("transcript.show_more_prompt")
    };
    let click_toggle = clamp.toggle.clone();
    let key_toggle = clamp.toggle.clone();
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(2.0))
        .min_w_0()
        .max_w(px(BUBBLE_MAX_WIDTH))
        .child(bubble)
        .child(
            div()
                .id(SharedString::from(format!(
                    "prompt-clamp-toggle-{}",
                    clamp.message_id
                )))
                .tab_index(0)
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(5.0))
                .border_1()
                .border_color(gpui::transparent_black())
                .focus_visible(|style| style.border_color(theme.accent))
                .text_size(sp(12.5))
                .text_color(theme.text_tertiary)
                .hover(|style| style.text_color(theme.text_secondary))
                .cursor_default()
                .child(label)
                .on_click(move |_, _, cx| {
                    click_toggle(cx);
                    cx.stop_propagation();
                })
                .on_key_down(move |event: &KeyDownEvent, _, cx| {
                    if !event.keystroke.modifiers.modified()
                        && matches!(event.keystroke.key.as_str(), "enter" | "space")
                    {
                        key_toggle(cx);
                        cx.stop_propagation();
                    }
                }),
        )
        .into_any_element()
}

/// How many lines `content` wraps to at `width` pixels in a `font`-pixel
/// face. Wide (CJK, fullwidth) characters count a full em, others about
/// half, which is generous for prose and short for code — erring toward
/// "short" keeps a misjudged prompt whole.
fn estimated_lines(content: &str, width: f32, font: f32) -> usize {
    if width <= 0.0 || font <= 0.0 {
        return 0;
    }
    let ems_per_line = (width / font).max(1.0);
    content
        .trim_end()
        .split('\n')
        .map(|line| {
            let ems: f32 = line
                .chars()
                .map(|character| if is_wide(character) { 1.0 } else { 0.5 })
                .sum();
            ((ems / ems_per_line).ceil() as usize).max(1)
        })
        .sum()
}

fn is_wide(character: char) -> bool {
    matches!(character as u32,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_prompts_stay_whole() {
        assert_eq!(estimated_lines("fix the build", 516.0, 14.0), 1);
        assert!(
            estimated_lines("one\ntwo\nthree\nfour\nfive\nsix\nseven", 516.0, 14.0)
                <= COLLAPSE_ABOVE_LINES
        );
    }

    #[test]
    fn long_lines_wrap_and_wide_characters_count_double() {
        let width = 516.0;
        let font = 14.0;
        // 36.9 ems a line: 200 latin characters are 100 ems, three lines.
        assert_eq!(estimated_lines(&"a".repeat(200), width, font), 3);
        // 100 CJK characters are 100 ems too.
        assert_eq!(estimated_lines(&"字".repeat(100), width, font), 3);
    }

    #[test]
    fn a_pasted_log_folds() {
        let log = (0..40)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(estimated_lines(&log, 516.0, 14.0) > COLLAPSE_ABOVE_LINES);
    }

    #[test]
    fn trailing_newlines_do_not_count() {
        assert_eq!(estimated_lines("a\n\n\n", 516.0, 14.0), 1);
    }
}
