//! The right panel's Plan surface: the plan an agent asks to have approved.
//!
//! Fork addition. When an agent finishes planning it asks to leave plan mode
//! (`ExitPlanMode` on Claude Code and the built-in agent), and the drivers
//! raise that as a permission titled `plan.ready_title` whose body carries
//! the plan. The card above the composer used to hold the whole plan in a
//! capped box with a one-line note. Now it shrinks to one row, and the plan
//! opens here instead:
//!
//! - the whole plan, rendered and selectable;
//! - every version the agent submitted in the session, with a line diff
//!   against the one before, so a revision shows what changed;
//! - a multi-line note. "Quote selection" drops the selected plan text into
//!   it as a Markdown quote, so a comment says which part it is about;
//! - the same answers as the card. Sending it back with notes refuses the
//!   request and steers the note into the turn, exactly as the card's
//!   "keep planning with notes" did.
//!
//! The history lives in memory, per session: a plan is a turn's question,
//! and like the permission it came with it does not outlive the app.

use similar::{ChangeTag, TextDiff};

use super::permission_card::is_plan_approval;
use super::providers_page::card_button;
use super::right_panel::{DiffRowStyle, render_diff_code_row};
use super::*;
use crate::ui::ActivationExt as _;

/// Versions kept per session; older ones drop off the front.
const MAX_VERSIONS: usize = 20;

#[derive(Clone, Debug, PartialEq)]
struct PlanVersion {
    request_id: String,
    /// The plan, without the dialog's lead-in. Empty when the agent attached
    /// none.
    text: String,
    /// How the person answered it: `Some(true)` approved, `Some(false)` sent
    /// back to keep planning.
    answer: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanStatus {
    /// Still waiting for an answer.
    Pending,
    Approved,
    SentBack,
    /// The turn ended (stopped, failed) before anyone answered.
    Closed,
}

impl PlanVersion {
    fn status(&self, pending: bool) -> PlanStatus {
        match (pending, self.answer) {
            (true, _) => PlanStatus::Pending,
            (false, Some(true)) => PlanStatus::Approved,
            (false, Some(false)) => PlanStatus::SentBack,
            (false, None) => PlanStatus::Closed,
        }
    }
}

pub(super) struct PlanReviewState {
    versions: HashMap<Uuid, Vec<PlanVersion>>,
    /// The version the panel shows, per session. Absent means the latest.
    viewing: HashMap<Uuid, usize>,
    /// Show the line diff against the previous version instead of the plan.
    compare: bool,
    markdown: RefCell<MarkdownView>,
    pub(super) selection: TranscriptSelection,
    scroll: ScrollHandle,
    scrollbar: Rc<ScrollbarState>,
    /// The note field, made on the surface's first render (it needs the
    /// window).
    notes: Option<Entity<TextInput>>,
}

impl Default for PlanReviewState {
    fn default() -> Self {
        Self {
            versions: HashMap::new(),
            viewing: HashMap::new(),
            compare: false,
            markdown: RefCell::new(MarkdownView::default()),
            selection: TranscriptSelection::default(),
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
            notes: None,
        }
    }
}

impl PlanReviewState {
    /// Record a plan asked for approval. Returns `false` when the request was
    /// already recorded (a replayed event).
    fn record(&mut self, session_id: Uuid, request_id: &str, text: String) -> bool {
        let versions = self.versions.entry(session_id).or_default();
        if versions
            .iter()
            .any(|version| version.request_id == request_id)
        {
            return false;
        }
        versions.push(PlanVersion {
            request_id: request_id.to_owned(),
            text,
            answer: None,
        });
        if versions.len() > MAX_VERSIONS {
            let excess = versions.len() - MAX_VERSIONS;
            versions.drain(..excess);
        }
        // A new plan is what the person is here to read.
        self.viewing.remove(&session_id);
        true
    }

    fn answer(&mut self, session_id: Uuid, request_id: &str, allow: bool) {
        if let Some(version) = self.versions.get_mut(&session_id).and_then(|versions| {
            versions
                .iter_mut()
                .find(|version| version.request_id == request_id)
        }) {
            version.answer = Some(allow);
        }
    }
}

/// The plan itself from a "finished planning" dialog's body: the text under
/// the lead-in the drivers put above it, or nothing when the agent attached
/// none (the dialog then carries only the generic line).
fn plan_body(detail: &str) -> String {
    let detail = detail.trim();
    if let Some(plan) = detail.strip_prefix(tr!("plan.summary_lead").as_str()) {
        return plan.trim().to_owned();
    }
    if detail == tr!("plan.ready_detail").trim() {
        return String::new();
    }
    detail.to_owned()
}

/// `notes` with `selected` appended as a Markdown quote, followed by an empty
/// line for the comment about it.
fn quote_into(notes: &str, selected: &str) -> String {
    let quote = selected
        .trim()
        .lines()
        .map(|line| {
            let line = line.trim_end();
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let separator = if notes.trim().is_empty() {
        ""
    } else if notes.ends_with("\n\n") {
        ""
    } else if notes.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let notes = if notes.trim().is_empty() { "" } else { notes };
    format!("{notes}{separator}{quote}\n\n")
}

/// A line diff between two versions of a plan, as the rows the Review tab
/// draws.
fn plan_diff_lines(previous: &str, current: &str) -> Vec<crate::review_diff::Line> {
    use crate::review_diff::{Line, LineKind};
    let diff = TextDiff::from_lines(previous, current);
    let mut old_line = 0u32;
    let mut new_line = 0u32;
    diff.iter_all_changes()
        .map(|change| {
            let content = change.value().trim_end_matches(['\n', '\r']).to_owned();
            let (kind, old, new) = match change.tag() {
                ChangeTag::Equal => {
                    old_line += 1;
                    new_line += 1;
                    (LineKind::Context, Some(old_line), Some(new_line))
                }
                ChangeTag::Delete => {
                    old_line += 1;
                    (LineKind::Deletion, Some(old_line), None)
                }
                ChangeTag::Insert => {
                    new_line += 1;
                    (LineKind::Addition, None, Some(new_line))
                }
            };
            Line {
                file_index: 0,
                old_line: old,
                new_line: new,
                kind,
                content,
                tokens: Vec::new(),
            }
        })
        .collect()
}

impl Waku {
    /// Hook (`streaming.rs`): a permission arrived for `session_id`. A plan
    /// to approve is recorded and, on the session in view, opened here.
    pub(super) fn plan_requested(
        &mut self,
        session_id: Uuid,
        permission: &PendingPermission,
        cx: &mut Context<Self>,
    ) {
        if !is_plan_approval(permission) {
            return;
        }
        let recorded = self.plan_review.record(
            session_id,
            &permission.request_id,
            plan_body(&permission.detail),
        );
        if recorded && self.state.selected_session == Some(session_id) {
            self.plan_review.selection.clear();
            self.open_right_panel_surface(RightPanelSurface::Plan, cx);
        }
    }

    /// Hook (`sessions.rs`): the person answered `request_id`. Must run
    /// before the answer removes the request from the runtime. Notes written
    /// for an answered plan are moot, whichever way it was answered.
    pub(super) fn note_plan_answer(
        &mut self,
        session_id: Uuid,
        request_id: &str,
        option_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(allow) = self
            .runtimes
            .get(&session_id)
            .and_then(|runtime| {
                runtime
                    .pending_permissions
                    .iter()
                    .find(|permission| permission.request_id == request_id)
            })
            .filter(|permission| is_plan_approval(permission))
            .and_then(|permission| {
                permission
                    .options
                    .iter()
                    .find(|option| option.id == option_id)
            })
            .map(|option| option.allow)
        else {
            return;
        };
        self.plan_review.answer(session_id, request_id, allow);
        if let Some(notes) = self.plan_review.notes.clone() {
            notes.update(cx, |input, cx| input.clear(cx));
        }
    }

    /// Whether the session in view has submitted a plan this run.
    pub(super) fn selected_session_has_plan(&self) -> bool {
        self.state
            .selected_session
            .and_then(|session_id| self.plan_review.versions.get(&session_id))
            .is_some_and(|versions| !versions.is_empty())
    }

    /// The plan text selected in the panel, while the panel is in view.
    pub(super) fn plan_selected_text(&self) -> Option<String> {
        let shown = self.right_panel_visible
            && matches!(
                self.active_right_panel_surface(),
                Some(RightPanelSurface::Plan)
            );
        shown
            .then(|| {
                self.plan_review
                    .selection
                    .selection
                    .borrow()
                    .selected_text()
            })
            .flatten()
    }

    pub(super) fn open_plan_surface(&mut self, cx: &mut Context<Self>) {
        self.open_right_panel_surface(RightPanelSurface::Plan, cx);
    }

    /// The header button for the Plan surface, while the session in view has
    /// submitted a plan: one click shows it, another puts the panel away.
    /// Without it a closed Plan tab had no way back once the plan was
    /// answered — the card's "View plan" leaves with the request, and the
    /// panel's "+" menu is out of reach while the panel is hidden. A dot marks
    /// a plan still waiting for an answer.
    pub(super) fn render_plan_surface_button(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if !self.selected_session_has_plan() {
            return None;
        }
        let shown = self.right_panel_visible
            && matches!(
                self.active_right_panel_surface(),
                Some(RightPanelSurface::Plan)
            );
        let waiting = self
            .state
            .selected_session
            .is_some_and(|session_id| self.pending_plan(session_id).is_some());
        let tooltip: SharedString = if shown {
            tr!("plan.surface_hide")
        } else {
            tr!("plan.surface_open")
        }
        .into();
        Some(
            div()
                .id("surface-bar-plan")
                .tab_index(0)
                .focus_visible(|style| style.border_1().border_color(theme.accent))
                .relative()
                .w(px(26.0))
                .h(px(26.0))
                .flex_none()
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .when(shown, |element| element.bg(theme.overlay))
                .hover(|element| element.bg(theme.overlay))
                .child(icon(
                    "icons/list.svg",
                    14.0,
                    if shown || waiting {
                        theme.accent
                    } else {
                        theme.text_tertiary
                    },
                ))
                .when(waiting && !shown, |element| {
                    element.child(
                        div()
                            .absolute()
                            .top(px(4.0))
                            .right(px(4.0))
                            .size(px(6.0))
                            .rounded_full()
                            .bg(theme.accent),
                    )
                })
                .tooltip(Tooltip::text(tooltip))
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .on_activation(cx, move |this, _, cx| {
                    if shown {
                        this.set_right_panel_visible(false, cx);
                    } else {
                        this.open_plan_surface(cx);
                    }
                }),
        )
    }

    /// The plan approval the session is waiting on, if any.
    fn pending_plan(&self, session_id: Uuid) -> Option<PendingPermission> {
        self.runtimes
            .get(&session_id)?
            .pending_permissions
            .iter()
            .find(|permission| is_plan_approval(permission))
            .cloned()
    }

    fn plan_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some(notes) = &self.plan_review.notes {
            return notes.clone();
        }
        let notes = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line()
                .submit_on_enter()
                .auto_height()
                .placeholder(tr!("plan.notes_placeholder"))
        });
        cx.subscribe(
            &notes,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.send_plan_back(cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        self.plan_review.notes = Some(notes.clone());
        notes
    }

    fn quote_plan_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self
            .plan_review
            .selection
            .selection
            .borrow()
            .selected_text()
            .filter(|text| !text.trim().is_empty())
        else {
            return;
        };
        let notes = self.plan_notes(window, cx);
        notes.update(cx, |input, cx| {
            let content = quote_into(input.content(), &selected);
            let end = content.len();
            input.set_content(content, cx);
            input.select_range(end..end, cx);
        });
        self.plan_review.selection.clear();
        window.focus(&notes.read(cx).focus(), cx);
        cx.notify();
    }

    /// Send the plan back with the notes: refuse it, then steer the notes
    /// into the turn.
    fn send_plan_back(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.state.selected_session else {
            return;
        };
        let Some(pending) = self.pending_plan(session_id) else {
            return;
        };
        let Some(notes) = self.plan_review.notes.clone() else {
            return;
        };
        let note = notes.read(cx).content().trim().to_owned();
        if note.is_empty() {
            return;
        }
        if self.deny_permission_with_note(pending.request_id, note, cx) {
            notes.update(cx, |input, cx| input.clear(cx));
        }
    }

    fn answer_plan(&mut self, allow: bool, cx: &mut Context<Self>) {
        let Some(pending) = self
            .state
            .selected_session
            .and_then(|session_id| self.pending_plan(session_id))
        else {
            return;
        };
        let Some(option) = pending.options.iter().find(|option| option.allow == allow) else {
            return;
        };
        self.respond_permission(pending.request_id.clone(), option.id.clone(), cx);
    }

    fn view_plan_version(&mut self, session_id: Uuid, index: usize, cx: &mut Context<Self>) {
        self.plan_review.viewing.insert(session_id, index);
        self.plan_review.selection.clear();
        cx.notify();
    }

    pub(super) fn render_plan_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(cx);
        let Some(session_id) = self.state.selected_session else {
            return render_plan_empty(theme, tr!("plan.panel.no_session"));
        };
        let versions = self
            .plan_review
            .versions
            .get(&session_id)
            .cloned()
            .unwrap_or_default();
        if versions.is_empty() {
            return render_plan_empty(theme, tr!("plan.panel.empty"));
        }
        let pending = self.pending_plan(session_id);
        let latest = versions.len() - 1;
        let index = self
            .plan_review
            .viewing
            .get(&session_id)
            .copied()
            .filter(|index| *index <= latest)
            .unwrap_or(latest);
        let version = versions[index].clone();
        let is_pending = pending
            .as_ref()
            .is_some_and(|pending| pending.request_id == version.request_id);
        let previous = index
            .checked_sub(1)
            .map(|index| versions[index].text.clone());
        let comparing = self.plan_review.compare && previous.is_some();

        let header = self.render_plan_header(
            session_id,
            &version,
            is_pending,
            index,
            versions.len(),
            previous.is_some(),
            comparing,
            theme,
            cx,
        );

        let body = if comparing {
            let lines = plan_diff_lines(previous.as_deref().unwrap_or_default(), &version.text);
            if lines
                .iter()
                .all(|line| matches!(line.kind, crate::review_diff::LineKind::Context))
            {
                div()
                    .px(px(16.0))
                    .py(px(14.0))
                    .child(note(theme, tr!("plan.panel.no_changes")))
                    .into_any_element()
            } else {
                let style = DiffRowStyle::activity(self.state.code_font_size);
                div()
                    .py(px(6.0))
                    .flex()
                    .flex_col()
                    .children(lines.iter().enumerate().map(|(index, line)| {
                        render_diff_code_row(
                            line,
                            index,
                            "plan-diff",
                            &self.plan_review.selection,
                            style,
                            &theme,
                        )
                    }))
                    .into_any_element()
            }
        } else if version.text.trim().is_empty() {
            div()
                .px(px(16.0))
                .py(px(14.0))
                .child(note(theme, tr!("plan.panel.no_body")))
                .into_any_element()
        } else {
            let palette = MarkdownPalette::from_theme(&theme);
            let mut view = self.plan_review.markdown.borrow_mut();
            view.set_text(&version.text, false);
            let ctx = MarkdownCtx::new(
                format!("plan-{}", version.request_id),
                &palette,
                MarkdownMetrics::document(self.state.ui_font_size, self.state.code_font_size),
                self.plan_review.selection.clone(),
            )
            .with_link_handler(self.markdown_link_handler.clone());
            let document = md::render::markdown(&view, &ctx);
            div()
                .px(px(16.0))
                .pt(px(14.0))
                .pb(px(24.0))
                .text_color(theme.text)
                .children(document)
                .into_any_element()
        };

        let selection_input = {
            let selection = self.plan_review.selection.clone();
            canvas(
                |_, _, _| (),
                move |_, _, window, _| md::render::install_selection_input(window, &selection),
            )
            .absolute()
            .w(px(0.0))
            .h(px(0.0))
        };

        let footer = pending.map(|_| self.render_plan_footer(window, theme, cx));

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(
                        div()
                            .id("plan-surface-body")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.plan_review.scroll)
                            // Painted before the document, so the frame's
                            // selection registry holds exactly its elements.
                            .child(md::render::frame_reset(self.plan_review.selection.clone()))
                            .child(body),
                    )
                    .child(scrollbar::vertical(
                        &self.plan_review.scroll,
                        &self.plan_review.scrollbar,
                    )),
            )
            // Before the footer: the footer's listeners then run first and
            // keep a click on "quote" from clearing what it is to quote.
            .child(selection_input)
            .when_some(footer, |element, footer| element.child(footer))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_plan_header(
        &self,
        session_id: Uuid,
        version: &PlanVersion,
        is_pending: bool,
        index: usize,
        total: usize,
        has_previous: bool,
        comparing: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let (status_icon, status_label, status_color) = match version.status(is_pending) {
            PlanStatus::Pending => ("icons/list.svg", tr!("plan.status.pending"), theme.accent),
            PlanStatus::Approved => (
                "icons/check.svg",
                tr!("plan.status.approved"),
                theme.success,
            ),
            PlanStatus::SentBack => (
                "icons/pencil.svg",
                tr!("plan.status.sent_back"),
                theme.warning,
            ),
            PlanStatus::Closed => (
                "icons/x.svg",
                tr!("plan.status.closed"),
                theme.text_tertiary,
            ),
        };
        let mut header = div()
            .flex_none()
            .px(px(16.0))
            .py(px(10.0))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(status_color)
                    .child(icon(status_icon, 13.0, status_color))
                    .child(status_label),
            )
            .child(div().flex_1());
        if total > 1 {
            let older = index.checked_sub(1);
            let newer = (index + 1 < total).then_some(index + 1);
            header = header
                .child(stepper_button(
                    theme,
                    "plan-version-older",
                    "icons/chevron-left.svg",
                    tr!("plan.panel.older"),
                    older,
                    session_id,
                    cx,
                ))
                .child(
                    div()
                        .text_size(sp(12.0))
                        .text_color(theme.text_secondary)
                        .child(tr!("plan.panel.version", index = index + 1, total = total)),
                )
                .child(stepper_button(
                    theme,
                    "plan-version-newer",
                    "icons/chevron-right.svg",
                    tr!("plan.panel.newer"),
                    newer,
                    session_id,
                    cx,
                ));
        }
        if has_previous {
            header = header.child(card_button(
                theme,
                SharedString::from("plan-compare"),
                if comparing {
                    tr!("plan.panel.show_plan")
                } else {
                    tr!("plan.panel.compare")
                },
                false,
                false,
                cx,
                |this, _, cx| {
                    this.plan_review.compare = !this.plan_review.compare;
                    this.plan_review.selection.clear();
                    cx.notify();
                },
            ));
        }
        header
    }

    fn render_plan_footer(
        &mut self,
        window: &mut Window,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let notes = self.plan_notes(window, cx);
        let has_notes = !notes.read(cx).content().trim().is_empty();
        let has_selection = !self.plan_review.selection.selection.borrow().is_empty();
        div()
            .flex_none()
            .px(px(16.0))
            .py(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.raised)
            // A press anywhere here must not reach the plan's selection
            // listener, which clears the selection on any press outside text.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(card_button(
                        theme,
                        SharedString::from("plan-quote-selection"),
                        tr!("plan.panel.quote"),
                        false,
                        !has_selection,
                        cx,
                        |this, window, cx| this.quote_plan_selection(window, cx),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(note(theme, tr!("plan.panel.quote_hint"))),
                    ),
            )
            .child(
                div()
                    .min_h(px(64.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.inset)
                    .text_size(sp(12.5))
                    .child(notes),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .child(card_button(
                        theme,
                        SharedString::from("plan-send-back"),
                        tr!("plan.keep_planning_with_notes"),
                        false,
                        !has_notes,
                        cx,
                        |this, _, cx| this.send_plan_back(cx),
                    ))
                    .child(card_button(
                        theme,
                        SharedString::from("plan-keep-planning"),
                        tr!("plan.keep_planning"),
                        false,
                        false,
                        cx,
                        |this, _, cx| this.answer_plan(false, cx),
                    ))
                    .child(div().flex_1())
                    .child(card_button(
                        theme,
                        SharedString::from("plan-approve"),
                        tr!("plan.approve"),
                        true,
                        false,
                        cx,
                        |this, _, cx| this.answer_plan(true, cx),
                    )),
            )
    }
}

fn stepper_button(
    theme: Theme,
    id: &'static str,
    path: &'static str,
    label: String,
    target: Option<usize>,
    session_id: Uuid,
    cx: &mut Context<Waku>,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .tab_index(0)
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .size(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_default()
        .tooltip(Tooltip::text(label))
        .child(icon(
            path,
            13.0,
            if target.is_some() {
                theme.text_secondary
            } else {
                theme.text_ghost
            },
        ));
    match target {
        Some(index) => button
            .hover(|style| style.bg(theme.overlay))
            .on_activation(cx, move |this, _, cx| {
                this.view_plan_version(session_id, index, cx)
            }),
        None => button,
    }
}

fn note(theme: Theme, text: String) -> Div {
    div()
        .text_size(sp(12.0))
        .line_height(sp(17.0))
        .text_color(theme.text_tertiary)
        .child(text)
}

fn render_plan_empty(theme: Theme, message: String) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .px(px(24.0))
        .bg(theme.surface)
        .child(icon("icons/list.svg", 18.0, theme.text_tertiary))
        .child(
            div()
                .max_w(px(320.0))
                .text_center()
                .text_size(sp(12.5))
                .line_height(sp(18.0))
                .text_color(theme.text_tertiary)
                .child(message),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_diff::LineKind;

    #[test]
    fn the_plan_is_read_from_under_the_lead_in() {
        let lead = tr!("plan.summary_lead");
        assert_eq!(
            plan_body(&format!("{lead}\n\n## Plan\n\n1. Do it.")),
            "## Plan\n\n1. Do it."
        );
        assert_eq!(plan_body(&tr!("plan.ready_detail")), "");
        assert_eq!(plan_body("  just the plan  "), "just the plan");
    }

    #[test]
    fn a_quote_lands_under_the_notes_with_room_for_the_comment() {
        assert_eq!(
            quote_into("", "Step 2\n\nStep 3"),
            "> Step 2\n>\n> Step 3\n\n"
        );
        assert_eq!(
            quote_into("Overall fine.", "Step 2"),
            "Overall fine.\n\n> Step 2\n\n"
        );
        assert_eq!(quote_into("Fine.\n", "Step 2"), "Fine.\n\n> Step 2\n\n");
        assert_eq!(quote_into("> A\n\nwhy?\n\n", "B"), "> A\n\nwhy?\n\n> B\n\n");
        assert_eq!(quote_into("   ", " B "), "> B\n\n");
    }

    #[test]
    fn a_revision_diffs_line_by_line_against_the_previous_plan() {
        let lines = plan_diff_lines("- Read.\n- Write.\n", "- Read.\n- Test.\n- Write.\n");
        let kinds: Vec<_> = lines
            .iter()
            .map(|line| match line.kind {
                LineKind::Context => ' ',
                LineKind::Addition => '+',
                LineKind::Deletion => '-',
                _ => '?',
            })
            .collect();
        assert_eq!(kinds, vec![' ', '+', ' ']);
        assert_eq!(lines[1].content, "- Test.");
        assert_eq!(lines[1].new_line, Some(2));
        assert_eq!(lines[2].old_line, Some(2));
        assert_eq!(lines[2].new_line, Some(3));
    }

    #[test]
    fn versions_are_recorded_once_capped_and_answered() {
        let mut state = PlanReviewState::default();
        let session = Uuid::new_v4();
        assert!(state.record(session, "r1", "one".into()));
        assert!(!state.record(session, "r1", "one".into()));
        state.viewing.insert(session, 0);
        assert!(state.record(session, "r2", "two".into()));
        assert!(
            !state.viewing.contains_key(&session),
            "a new plan shows the latest"
        );
        state.answer(session, "r1", false);
        let versions = &state.versions[&session];
        assert_eq!(versions[0].status(false), PlanStatus::SentBack);
        assert_eq!(versions[1].status(true), PlanStatus::Pending);
        assert_eq!(versions[1].status(false), PlanStatus::Closed);
        for index in 0..MAX_VERSIONS + 3 {
            state.record(session, &format!("x{index}"), String::new());
        }
        assert_eq!(state.versions[&session].len(), MAX_VERSIONS);
    }
}
