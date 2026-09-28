//! A sub-agent, opened in the right panel: what it was asked, and its record
//! as it happens.
//!
//! The background-work surface shows a process as a header and a log. A
//! sub-agent gets the same header — its task, where it stands, how long it
//! has run, a Stop while it can be stopped — over a record read the way the
//! transcript is read: what it wrote as markdown, and its tool calls as the
//! transcript's own rows, which open onto their arguments, output and diffs.
//! The record comes from `subagent_transcript.rs`; nothing here parses.

use super::subagent_transcript::{SubagentRow, SubagentTranscript};
use super::*;

impl Waku {
    pub(super) fn render_subagent_surface(
        &self,
        item: &BackgroundWorkItem,
        record: Option<&SubagentTranscript>,
        selection: TranscriptSelection,
        stop: Option<Stateful<Div>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let status_color = super::background_work::work_status_color(item.status, theme);
        let live = item.status.is_live();
        // A made-up entry for a call from before a restart has no clock.
        let elapsed = (item.duration_ms != Some(0))
            .then(|| super::background_work::work_elapsed(item));

        let header = div()
            .flex_none()
            .mx(px(12.0))
            .mt(px(12.0))
            .min_h(px(54.0))
            .px(px(11.0))
            .py(px(8.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .flex()
            .items_center()
            .gap(px(9.0))
            .child(icon("icons/bot.svg", 15.0, theme.text_secondary))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .when_some(item.role.clone(), |line, role| {
                                line.child(
                                    div()
                                        .flex_none()
                                        .max_w(px(140.0))
                                        .truncate()
                                        .text_size(sp(12.5))
                                        .font_family(md::render::MONO_FAMILY)
                                        .text_color(super::subagent_row::agent_type_color(
                                            &role, &theme,
                                        ))
                                        .child(role),
                                )
                            })
                            .child(
                                div()
                                    .id("subagent-surface-title")
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .text_size(sp(12.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .tooltip(Tooltip::text(item.title.clone()))
                                    .child(single_line_label(&item.title)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .text_size(sp(12.5))
                            .text_color(theme.text_tertiary)
                            .child(super::background_work::rendered_work_status_icon(
                                item.status,
                                9.0,
                                status_color,
                            ))
                            .child(super::background_work::work_status_label(item.status))
                            .when_some(elapsed, |line, elapsed| line.child("·").child(elapsed))
                            .when_some(
                                item.model.clone().filter(|model| !model.is_empty()),
                                |line, model| {
                                    line.child("·").child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .font_family(md::render::MONO_FAMILY)
                                            .child(model),
                                    )
                                },
                            ),
                    ),
            )
            .when_some(stop, |header, stop| header.child(stop));

        let mut body = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(12.0))
            .py(px(10.0));

        if let Some(prompt) = item.command.as_ref().filter(|prompt| !prompt.trim().is_empty()) {
            body = body.child(self.render_subagent_prompt(
                item,
                prompt,
                record.is_some_and(|record| record.prompt_expanded),
                &theme,
                cx,
            ));
        }

        let trimmed = record.map_or(0, SubagentTranscript::trimmed);
        if trimmed > 0 {
            body = body.child(
                div()
                    .py(px(4.0))
                    .text_size(sp(12.5))
                    .text_color(theme.text_ghost)
                    .child(tr!("subagent.trimmed", count = trimmed)),
            );
        }

        let has_rows = record.is_some_and(|record| !record.is_empty());
        if let Some(record) = record.filter(|_| has_rows) {
            let palette = MarkdownPalette::from_theme(&theme);
            let metrics = self.scaled_markdown_metrics(MarkdownMetrics::COMPACT);
            for row in record.rows() {
                match row {
                    SubagentRow::Text { id, view, .. } => {
                        let ctx = MarkdownCtx::new(
                            format!("subagent-md-{}-{id}", item.key.provider_id),
                            &palette,
                            metrics,
                            selection.clone(),
                        );
                        body = body.child(
                            div()
                                .w_full()
                                .min_w_0()
                                .py(px(4.0))
                                .text_color(theme.text)
                                .children(md::render::markdown(view, &ctx)),
                        );
                    }
                    SubagentRow::Activity { activity, .. } => {
                        body = body.child(
                            self.render_activity_item(activity, live, None, &theme, window, cx),
                        );
                    }
                }
            }
        } else if live {
            body = body.child(
                div()
                    .py(px(6.0))
                    .text_size(sp(12.5))
                    .child(motion::shimmer(
                        tr!("subagent.waiting"),
                        theme.text_tertiary,
                        theme.text,
                    )),
            );
        } else if let Some(output) = item.output.as_ref().filter(|output| !output.trim().is_empty())
        {
            // No record — a session restored after a restart, or a client
            // that never had one — but the sub-agent's answer is known.
            body = body.child(
                div()
                    .pt(px(6.0))
                    .text_size(sp(12.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("subagent.result")),
            );
            body = body.child(
                div()
                    .w_full()
                    .min_w_0()
                    .py(px(4.0))
                    .text_size(sp(12.5))
                    .text_color(theme.text_secondary)
                    .child(output.clone()),
            );
        }

        if item.status == BackgroundWorkStatus::Failed
            && let Some(reason) = item.detail.as_ref().filter(|reason| !reason.trim().is_empty())
        {
            body = body.child(
                div()
                    .mt(px(6.0))
                    .px(px(10.0))
                    .py(px(7.0))
                    .rounded(px(7.0))
                    .bg(theme.danger.opacity(0.08))
                    .text_size(sp(12.5))
                    .text_color(theme.danger)
                    .child(reason.clone()),
            );
        }

        let scroll = record.map(|record| (record.scroll.clone(), record.scrollbar.clone()));
        let mut scroller = div()
            .id(SharedString::from(format!(
                "subagent-record-{}",
                item.key.provider_id
            )))
            .size_full()
            .overflow_y_scroll();
        if let Some((handle, _)) = &scroll {
            let wheel = handle.clone();
            scroller = scroller
                .track_scroll(handle)
                .on_scroll_wheel(move |_, _, cx| contain_scroll(&wheel, cx));
        }

        div()
            .id("background-work-surface")
            .tab_group()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(md::render::frame_reset(selection.clone()))
                    .child(scroller.child(body))
                    .when_some(scroll, |area, (handle, scrollbar)| {
                        area.child(scrollbar::vertical(&handle, &scrollbar))
                    })
                    .child(super::background_work::background_work_selection_input(selection)),
            )
    }

    /// What the sub-agent was asked, folded away by default: it is usually
    /// long, and the record below is what one opens the panel for.
    fn render_subagent_prompt(
        &self,
        item: &BackgroundWorkItem,
        prompt: &str,
        expanded: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let key = item.key.clone();
        let key_for_keys = item.key.clone();
        let control = format!("subagent-prompt-{}", item.key.provider_id);
        let focus = self.transcript_control_focus(control.clone(), cx);
        let toggle = div()
            .id(SharedString::from(control))
            .track_focus(&focus)
            .tab_index(0)
            .h(px(24.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .cursor_default()
            .text_size(sp(12.5))
            .text_color(theme.text_tertiary)
            .hover(|style| style.text_color(theme.text_secondary))
            .focus_visible(|style| style.text_color(theme.text))
            .child(icon(
                if expanded {
                    "icons/chevron-down.svg"
                } else {
                    "icons/chevron-right.svg"
                },
                12.0,
                theme.text_tertiary,
            ))
            .child(if expanded {
                tr!("subagent.hide_prompt")
            } else {
                tr!("subagent.show_prompt")
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_subagent_prompt(&key, cx);
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.toggle_subagent_prompt(&key_for_keys, cx);
                    cx.stop_propagation();
                }
            }));
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .pb(px(4.0))
            .child(toggle)
            .when(expanded, |section| {
                section.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .px(px(10.0))
                        .py(px(8.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.surface)
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .child(prompt.to_owned()),
                )
            })
    }

    fn toggle_subagent_prompt(&mut self, key: &BackgroundWorkKey, cx: &mut Context<Self>) {
        let Some(session_id) = self.state.selected_session else {
            return;
        };
        let record = self
            .background_work
            .entry(session_id)
            .or_default()
            .transcript_entry(key);
        record.prompt_expanded = !record.prompt_expanded;
        cx.notify();
    }
}
