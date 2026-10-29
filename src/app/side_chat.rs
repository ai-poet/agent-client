//! The side chat: ask about a task without steering it.
//!
//! Fork addition, after Claude Desktop's side chat (⌘;, `/btw`). A surface in
//! the right panel holds one side thread per task. A question goes out with
//! a digest of the task's transcript (`waku_client::side_chat`) to the
//! built-in engine through the daemon, and the answer streams back here. The
//! task itself is never touched: nothing reaches its transcript, its queue,
//! its agent or the disk, and the thread is gone when the app quits.
//!
//! Works for every provider: the digest is Waku's own transcript. A built-in
//! agent task is answered by its own model; any other by the built-in
//! agent's default model.

use std::cell::RefCell;

use waku_client::side_chat::{
    DIGEST_BUDGET, SIDE_SYSTEM, SideExchange, build_digest, side_messages,
};

use crate::input::{InputEvent, TextInput};

use super::providers_page::card_button;
use super::*;

/// The `key_context` of the side chat's surface.
pub(super) const SIDE_CHAT_CONTEXT: &str = "SideChat";
/// How long a poll waits for more of the answer.
const POLL_WAIT_MS: u64 = 250;

#[derive(Clone, Debug, PartialEq)]
enum ExchangeStatus {
    Streaming,
    Done,
    Stopped,
    Failed(String),
}

struct Exchange {
    question: String,
    answer: String,
    status: ExchangeStatus,
    /// The model that answered, as the header names it.
    model: String,
    markdown: RefCell<MarkdownView>,
}

#[derive(Default)]
struct SideThread {
    exchanges: Vec<Exchange>,
    /// The daemon job answering the last question, while it runs.
    job: Option<Uuid>,
    /// Bumped by every question and every stop, so a superseded answer's
    /// late text is dropped.
    generation: u64,
}

#[derive(Default)]
pub(super) struct SideChatState {
    threads: HashMap<Uuid, SideThread>,
    input: Option<Entity<TextInput>>,
    /// Put the caret in the field on the surface's next render.
    focus_pending: bool,
    selection: TranscriptSelection,
    scroll: ScrollHandle,
}

impl Waku {
    /// The built-in agent's model id for the task's side questions: a
    /// built-in agent task's own model, else the engine's default.
    fn side_chat_model(&self, session: &AgentSession) -> Option<String> {
        (session.provider == ProviderKind::Native)
            .then(|| session.model.clone())
            .flatten()
            .filter(|model| !model.trim().is_empty())
    }

    fn side_chat_model_label(model: Option<&str>) -> String {
        match model {
            Some(model) => model
                .split_once("::")
                .map_or(model, |(_, bare)| bare)
                .to_owned(),
            None => tr!("side_chat.default_model"),
        }
    }

    fn side_chat_open(&self) -> bool {
        self.right_panel_visible
            && matches!(
                self.active_right_panel_surface(),
                Some(RightPanelSurface::SideChat)
            )
    }

    /// ⌘;: open the side chat with the caret in it, or, when it already has
    /// the caret, put it away and go back to the composer.
    pub(super) fn toggle_side_chat_action(
        &mut self,
        _: &super::shortcuts::ToggleSideChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self
            .side_chat
            .input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window));
        if self.side_chat_open() && focused {
            self.close_active_right_panel_surface_if(RightPanelSurface::SideChat, cx);
            let focus = self.composer_focus(cx);
            window.focus(&focus, cx);
        } else {
            self.open_side_chat(cx);
        }
        cx.notify();
    }

    pub(super) fn open_side_chat(&mut self, cx: &mut Context<Self>) {
        self.settings_page = None;
        self.open_right_panel_surface(RightPanelSurface::SideChat, cx);
        self.side_chat.focus_pending = true;
        cx.notify();
    }

    /// Put away the surface when it is the one showing.
    fn close_active_right_panel_surface_if(
        &mut self,
        surface: RightPanelSurface,
        cx: &mut Context<Self>,
    ) {
        if self.active_right_panel_surface() == Some(&surface) {
            self.right_panel_visible = false;
            cx.notify();
        }
    }

    /// Escape in the side chat: stop an answer being written, or go back to
    /// the composer. Never further — unclaimed, Escape stops the task's turn.
    pub(super) fn side_chat_escape_action(
        &mut self,
        _: &super::shortcuts::SideChatEscape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let running = self
            .state
            .selected_session
            .and_then(|session| self.side_chat.threads.get(&session))
            .is_some_and(|thread| thread.job.is_some());
        if running {
            self.stop_side_answer(cx);
        } else {
            let focus = self.composer_focus(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// `/btw <question>` in the composer: ask it on the side, and leave the
    /// task alone. A bare `/btw` opens the side chat.
    pub(super) fn execute_side_chat_command(
        &mut self,
        prompt: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(question) = waku_client::side_chat::btw_question(prompt) else {
            return false;
        };
        let question = question.to_owned();
        self.composer.update(cx, |input, cx| input.clear(cx));
        self.open_side_chat(cx);
        if !question.is_empty() {
            self.ask_side_question(question, cx);
        }
        true
    }

    fn side_chat_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        if let Some(input) = &self.side_chat.input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line()
                .submit_on_enter()
                .auto_height()
                .placeholder(tr!("side_chat.placeholder"))
        });
        cx.subscribe(
            &input,
            |this: &mut Self, input, event: &InputEvent, cx| match event {
                InputEvent::Submit(text) => {
                    let question = text.trim().to_owned();
                    if !question.is_empty() {
                        input.update(cx, |input, cx| input.clear(cx));
                        this.ask_side_question(question, cx);
                    }
                }
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        self.side_chat.input = Some(input.clone());
        input
    }

    /// Ask `question` about the task on screen. The digest is built here, on
    /// the person's action; the request and the answer run off the UI thread.
    fn ask_side_question(&mut self, question: String, cx: &mut Context<Self>) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let session_id = session.id;
        let Some(cwd) = self
            .workspace_path_for_session(session)
            .map(Path::to_path_buf)
        else {
            return;
        };
        let model = self.side_chat_model(session);
        let digest = build_digest(session, DIGEST_BUDGET);
        if self
            .side_chat
            .threads
            .get(&session_id)
            .is_some_and(|thread| thread.job.is_some())
        {
            self.stop_side_answer(cx);
        }
        let thread = self.side_chat.threads.entry(session_id).or_default();
        let earlier: Vec<SideExchange> = thread
            .exchanges
            .iter()
            .map(|exchange| SideExchange {
                question: exchange.question.clone(),
                answer: (exchange.status == ExchangeStatus::Done).then(|| exchange.answer.clone()),
            })
            .collect();
        let messages = side_messages(&digest, &earlier, &question);
        thread.generation += 1;
        let generation = thread.generation;
        thread.exchanges.push(Exchange {
            question,
            answer: String::new(),
            status: ExchangeStatus::Streaming,
            model: Self::side_chat_model_label(model.as_deref()),
            markdown: RefCell::new(MarkdownView::default()),
        });
        self.side_chat.scroll.scroll_to_bottom();
        cx.notify();

        let workspace_client = waku_client::WorkspaceClient::new(self.daemon.client());
        cx.spawn(async move |waku, cx| {
            let started = {
                let client = workspace_client.clone();
                cx.background_executor()
                    .spawn(async move {
                        client.request(waku_client::WorkspaceOperation::SideChatStart {
                            cwd,
                            model,
                            system: SIDE_SYSTEM.to_owned(),
                            messages,
                        })
                    })
                    .await
            };
            let job = match started {
                Ok(waku_client::WorkspaceResult::SideChatStarted { job }) => job,
                Ok(_) => {
                    let _ = waku.update(cx, |this, cx| {
                        this.finish_side_answer(
                            session_id,
                            generation,
                            Some("unexpected answer".into()),
                            cx,
                        )
                    });
                    return;
                }
                Err(error) => {
                    let _ = waku.update(cx, |this, cx| {
                        this.finish_side_answer(session_id, generation, Some(error.to_string()), cx)
                    });
                    return;
                }
            };
            let current = waku
                .update(cx, |this, _| {
                    let thread = this.side_chat.threads.get_mut(&session_id)?;
                    (thread.generation == generation).then(|| thread.job = Some(job))
                })
                .ok()
                .flatten()
                .is_some();
            if !current {
                let _ = workspace_client
                    .request(waku_client::WorkspaceOperation::SideChatCancel { job });
                return;
            }
            let mut from = 0;
            loop {
                let client = workspace_client.clone();
                let polled = cx
                    .background_executor()
                    .spawn(async move {
                        client.request(waku_client::WorkspaceOperation::SideChatPoll {
                            job,
                            from,
                            wait_ms: POLL_WAIT_MS,
                        })
                    })
                    .await;
                let (text, next, done, error) = match polled {
                    Ok(waku_client::WorkspaceResult::SideChatProgress {
                        text,
                        next,
                        done,
                        error,
                    }) => (text, next, done, error),
                    Ok(_) => (String::new(), from, true, Some("unexpected answer".into())),
                    Err(error) => (String::new(), from, true, Some(error.to_string())),
                };
                from = next;
                let keep_going = waku
                    .update(cx, |this, cx| {
                        let Some(thread) = this.side_chat.threads.get_mut(&session_id) else {
                            return false;
                        };
                        if thread.generation != generation {
                            return false;
                        }
                        if let Some(exchange) = thread.exchanges.last_mut()
                            && !text.is_empty()
                        {
                            exchange.answer.push_str(&text);
                            cx.notify();
                        }
                        if done {
                            this.finish_side_answer(session_id, generation, error.clone(), cx);
                            return false;
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    fn finish_side_answer(
        &mut self,
        session_id: Uuid,
        generation: u64,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.side_chat.threads.get_mut(&session_id) else {
            return;
        };
        if thread.generation != generation {
            return;
        }
        thread.job = None;
        if let Some(exchange) = thread.exchanges.last_mut() {
            exchange.status = match error {
                Some(error) => ExchangeStatus::Failed(error),
                None => ExchangeStatus::Done,
            };
        }
        cx.notify();
    }

    /// Stop the answer being written for the task on screen.
    fn stop_side_answer(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.state.selected_session else {
            return;
        };
        let Some(thread) = self.side_chat.threads.get_mut(&session_id) else {
            return;
        };
        thread.generation += 1;
        if let Some(exchange) = thread.exchanges.last_mut()
            && exchange.status == ExchangeStatus::Streaming
        {
            exchange.status = ExchangeStatus::Stopped;
        }
        if let Some(job) = thread.job.take() {
            let client = waku_client::WorkspaceClient::new(self.daemon.client());
            cx.background_executor()
                .spawn(async move {
                    let _ = client.request(waku_client::WorkspaceOperation::SideChatCancel { job });
                })
                .detach();
        }
        cx.notify();
    }

    fn clear_side_chat(&mut self, cx: &mut Context<Self>) {
        self.stop_side_answer(cx);
        if let Some(session_id) = self.state.selected_session {
            self.side_chat.threads.remove(&session_id);
        }
        cx.notify();
    }

    pub(super) fn render_side_chat_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(cx);
        let input = self.side_chat_input(window, cx);
        if std::mem::take(&mut self.side_chat.focus_pending) {
            window.focus(&input.read(cx).focus_handle(cx), cx);
        }
        let session = self.selected_session();
        let model = session
            .map(|session| Self::side_chat_model_label(self.side_chat_model(session).as_deref()))
            .unwrap_or_default();
        let session_id = self.state.selected_session;
        let thread = session_id.and_then(|session| self.side_chat.threads.get(&session));
        let running = thread.is_some_and(|thread| thread.job.is_some());
        let has_exchanges = thread.is_some_and(|thread| !thread.exchanges.is_empty());

        let header = div()
            .flex_none()
            .px(px(14.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(
                        "icons/corner-down-right.svg",
                        13.0,
                        theme.text_secondary,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("side_chat.title")),
                    )
                    .when(has_exchanges, |header| {
                        header.child(card_button(
                            theme,
                            SharedString::from("side-chat-clear"),
                            tr!("side_chat.clear"),
                            false,
                            false,
                            cx,
                            |this, _, cx| this.clear_side_chat(cx),
                        ))
                    }),
            )
            .child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_tertiary)
                    .child(tr!("side_chat.subtitle", model = model)),
            );

        let palette = MarkdownPalette::from_theme(&theme);
        let metrics = MarkdownMetrics::document(self.state.ui_font_size, self.state.code_font_size);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .px(px(14.0))
            .py(px(12.0));
        match thread {
            Some(thread) if !thread.exchanges.is_empty() => {
                for (index, exchange) in thread.exchanges.iter().enumerate() {
                    let answer = {
                        let mut view = exchange.markdown.borrow_mut();
                        view.set_text(
                            &exchange.answer,
                            exchange.status == ExchangeStatus::Streaming,
                        );
                        let ctx = MarkdownCtx::new(
                            format!("side-chat-{}-{index}", session_id.unwrap_or_default()),
                            &palette,
                            metrics,
                            self.side_chat.selection.clone(),
                        )
                        .with_link_handler(self.markdown_link_handler.clone());
                        md::render::markdown(&view, &ctx)
                    };
                    let status = match &exchange.status {
                        ExchangeStatus::Streaming if exchange.answer.is_empty() => {
                            Some((tr!("side_chat.thinking"), theme.text_tertiary))
                        }
                        ExchangeStatus::Stopped => {
                            Some((tr!("side_chat.stopped"), theme.text_tertiary))
                        }
                        ExchangeStatus::Failed(error) => {
                            Some((tr!("side_chat.failed", error = error.clone()), theme.danger))
                        }
                        _ => None,
                    };
                    body = body.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .self_end()
                                    .max_w(gpui::relative(0.9))
                                    .px(px(10.0))
                                    .py(px(6.0))
                                    .rounded(px(10.0))
                                    .bg(theme.raised)
                                    .text_size(sp(13.0))
                                    .text_color(theme.text)
                                    .child(exchange.question.clone()),
                            )
                            .child(div().text_color(theme.text).children(answer))
                            .children(status.map(|(text, color)| {
                                div().text_size(sp(12.0)).text_color(color).child(text)
                            }))
                            .child(
                                div()
                                    .text_size(sp(11.0))
                                    .text_color(theme.text_ghost)
                                    .child(exchange.model.clone()),
                            ),
                    );
                }
            }
            _ => {
                body = body.child(
                    div()
                        .pt(px(24.0))
                        .text_center()
                        .text_size(sp(12.5))
                        .line_height(sp(18.0))
                        .text_color(theme.text_tertiary)
                        .child(tr!("side_chat.empty")),
                );
            }
        }

        let selection_input = {
            let selection = self.side_chat.selection.clone();
            canvas(
                |_, _, _| (),
                move |_, _, window, _| md::render::install_selection_input(window, &selection),
            )
            .absolute()
            .w(px(0.0))
            .h(px(0.0))
        };

        let footer = div()
            .flex_none()
            .px(px(12.0))
            .py(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.raised)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .min_h(px(44.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.inset)
                    .text_size(sp(13.0))
                    .child(input),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(sp(11.5))
                            .text_color(theme.text_tertiary)
                            .child(tr!("side_chat.hint")),
                    )
                    .when(running, |row| {
                        row.child(card_button(
                            theme,
                            SharedString::from("side-chat-stop"),
                            tr!("side_chat.stop"),
                            false,
                            false,
                            cx,
                            |this, _, cx| this.stop_side_answer(cx),
                        ))
                    }),
            );

        div()
            .key_context(SIDE_CHAT_CONTEXT)
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .child(header)
            .child(
                div().flex_1().min_h_0().relative().child(
                    div()
                        .id("side-chat-body")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.side_chat.scroll)
                        .child(md::render::frame_reset(self.side_chat.selection.clone()))
                        .child(body),
                ),
            )
            .child(selection_input)
            .child(footer)
            .into_any_element()
    }
}
