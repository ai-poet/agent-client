//! Comments on lines of the Review panel's diff, sent to the agent together.
//!
//! Fork addition, after Claude Desktop's diff review: click a line (or move
//! to it with the arrow keys and press Enter) to comment on it, Enter keeps
//! the comment, and ⌘↩ sends every comment at once as one message the agent
//! can act on — the line, a few lines of the hunk around it and what was
//! said. Works for every provider: the message goes through the ordinary
//! send path, which queues it while a turn runs.
//!
//! A comment is anchored by file, side and line number, never by row index,
//! so it stays put when a gap is expanded or the diff is refreshed. It shows
//! under its line only while that line still reads the same; otherwise it is
//! counted as "not in this view" and still sent, with its own quote.
//! Comments live in memory and belong to the task they were written on.

use crate::input::{InputEvent, SubmitSteer, TextInput};
use crate::review_diff::{Line, LineKind};

use super::providers_page::card_button;
use super::right_panel::DiffRowStyle;
use super::*;

/// The `key_context` of the diff body while it has focus.
pub(super) const REVIEW_DIFF_CONTEXT: &str = "ReviewDiff";
/// The `key_context` of the comment editor.
pub(super) const REVIEW_COMMENT_EDITOR_CONTEXT: &str = "ReviewCommentEditor";
/// Lines of the hunk quoted with a comment, the commented one last.
const QUOTE_LINES: usize = 3;

/// Which side of the diff a line number counts on.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
pub(super) enum Side {
    /// An added or unchanged line, numbered in the new file.
    New,
    /// A removed line, numbered in the old file.
    Old,
}

/// Where a comment sits.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct Anchor {
    pub(super) path: String,
    pub(super) side: Side,
    pub(super) line: u32,
}

impl Anchor {
    /// A code row's anchor; `None` for chrome rows and rows without numbers.
    pub(super) fn of(line: &Line, path: &str) -> Option<Self> {
        let (side, number) = match line.kind {
            LineKind::Addition | LineKind::Context => (Side::New, line.new_line?),
            LineKind::Deletion => (Side::Old, line.old_line?),
            _ => return None,
        };
        Some(Self {
            path: path.to_owned(),
            side,
            line: number,
        })
    }
}

/// One quoted line: its diff marker and text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct QuoteLine {
    pub(super) marker: char,
    pub(super) text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReviewComment {
    pub(super) id: u64,
    pub(super) anchor: Anchor,
    /// The commented line as it read when the comment was written.
    pub(super) line_text: String,
    pub(super) quote: Vec<QuoteLine>,
    pub(super) body: String,
}

struct CommentEditor {
    session: Uuid,
    anchor: Anchor,
    line_text: String,
    quote: Vec<QuoteLine>,
    /// The comment being edited, or `None` for a new one.
    editing: Option<u64>,
    input: Entity<TextInput>,
}

#[derive(Default)]
pub(super) struct ReviewCommentsState {
    by_session: HashMap<Uuid, Vec<ReviewComment>>,
    editor: Option<CommentEditor>,
    /// The line the arrow keys are on.
    cursor: Option<Anchor>,
    next_id: u64,
}

fn marker_of(kind: &LineKind) -> char {
    match kind {
        LineKind::Addition => '+',
        LineKind::Deletion => '-',
        _ => ' ',
    }
}

fn is_code_row(kind: &LineKind) -> bool {
    matches!(
        kind,
        LineKind::Context | LineKind::Addition | LineKind::Deletion
    )
}

/// The commented line and up to two code lines before it in the same file,
/// stopping at anything that is not code (a hunk header, a gap).
pub(super) fn quote_for(lines: &[Line], index: usize) -> Vec<QuoteLine> {
    let Some(line) = lines.get(index) else {
        return Vec::new();
    };
    let mut quote = Vec::new();
    let mut at = index;
    loop {
        let row = &lines[at];
        if row.file_index != line.file_index || !is_code_row(&row.kind) {
            break;
        }
        quote.push(QuoteLine {
            marker: marker_of(&row.kind),
            text: row.content.clone(),
        });
        if quote.len() == QUOTE_LINES || at == 0 {
            break;
        }
        at -= 1;
    }
    quote.reverse();
    quote
}

/// The code row `delta` code rows away from `from`, clamped to the first or
/// last one; the first code row when `from` is `None`.
pub(super) fn step_code_row(lines: &[Line], from: Option<usize>, delta: isize) -> Option<usize> {
    let code: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            is_code_row(&line.kind) && (line.new_line.is_some() || line.old_line.is_some())
        })
        .map(|(index, _)| index)
        .collect();
    if code.is_empty() {
        return None;
    }
    let Some(from) = from else {
        return Some(code[0]);
    };
    let position = code
        .iter()
        .position(|index| *index >= from)
        .unwrap_or(code.len() - 1);
    let target = (position as isize + delta).clamp(0, code.len() as isize - 1) as usize;
    Some(code[target])
}

/// The message the agent receives: every comment in file and line order,
/// each with where it is, the lines it is about and what was said.
pub(super) fn format_review_message(comments: &[ReviewComment]) -> String {
    let mut sorted: Vec<&ReviewComment> = comments.iter().collect();
    sorted.sort_by(|a, b| {
        (&a.anchor.path, a.anchor.line, a.anchor.side).cmp(&(
            &b.anchor.path,
            b.anchor.line,
            b.anchor.side,
        ))
    });
    let mut message = String::from(
        "Please address these review comments on the current changes. \
         Each names a line of the diff, quotes it with the lines before it, \
         and says what to change.\n",
    );
    for (number, comment) in sorted.iter().enumerate() {
        let place = match comment.anchor.side {
            Side::New => format!("`{}:{}`", comment.anchor.path, comment.anchor.line),
            Side::Old => format!(
                "`{}` (removed line {} of the old version)",
                comment.anchor.path, comment.anchor.line
            ),
        };
        message.push_str(&format!("\n{}. {place}\n```diff\n", number + 1));
        for line in &comment.quote {
            message.push(line.marker);
            message.push_str(&line.text);
            message.push('\n');
        }
        message.push_str("```\n");
        message.push_str(&format!("Comment: {}\n", comment.body.trim()));
    }
    message.push_str("\nApply each comment, then say briefly what you changed for each one.");
    message
}

impl Waku {
    fn review_comment_session(&self) -> Option<Uuid> {
        self.state.selected_session
    }

    fn review_comments_here(&self) -> &[ReviewComment] {
        self.review_comment_session()
            .and_then(|session| self.review_comments.by_session.get(&session))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The row index an anchor is on in the current diff, if it is shown.
    fn review_row_of(&self, anchor: &Anchor) -> Option<usize> {
        let snapshot = self.right_panel_diff_snapshot.as_ref()?;
        snapshot.lines.iter().position(|line| {
            snapshot
                .files
                .get(line.file_index)
                .and_then(|file| Anchor::of(line, &file.path))
                .as_ref()
                == Some(anchor)
        })
    }

    fn remeasure_review_anchor(&self, anchor: &Anchor) {
        if let Some(index) = self.review_row_of(anchor) {
            self.right_panel_diff_list_state
                .remeasure_items(index..index + 1);
        }
    }

    /// How many of this task's comments are not under a line in the diff on
    /// screen (the line changed, or the diff shows another source).
    fn review_comments_out_of_view(&self) -> usize {
        let Some(snapshot) = self.right_panel_diff_snapshot.as_ref() else {
            return self.review_comments_here().len();
        };
        self.review_comments_here()
            .iter()
            .filter(|comment| {
                self.review_row_of(&comment.anchor)
                    .and_then(|index| snapshot.lines.get(index))
                    .is_none_or(|line| line.content != comment.line_text)
            })
            .count()
    }

    /// Open the editor on row `index` — for a new comment there.
    pub(super) fn open_review_comment(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.review_comment_session() else {
            return;
        };
        let Some(snapshot) = self.right_panel_diff_snapshot.clone() else {
            return;
        };
        let Some(line) = snapshot.lines.get(index) else {
            return;
        };
        let Some(file) = snapshot.files.get(line.file_index) else {
            return;
        };
        let Some(anchor) = Anchor::of(line, &file.path) else {
            return;
        };
        let quote = quote_for(&snapshot.lines, index);
        self.start_review_editor(
            session,
            anchor,
            line.content.clone(),
            quote,
            None,
            String::new(),
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn start_review_editor(
        &mut self,
        session: Uuid,
        anchor: Anchor,
        line_text: String,
        quote: Vec<QuoteLine>,
        editing: Option<u64>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous) = self.review_comments.editor.take() {
            self.remeasure_review_anchor(&previous.anchor);
        }
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .multi_line()
                .submit_on_enter()
                .auto_height()
                .placeholder(tr!("review_comments.placeholder"))
        });
        if !text.is_empty() {
            input.update(cx, |input, cx| input.set_content(text, cx));
        }
        cx.subscribe(
            &input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Submit(_) => this.save_review_comment(cx),
                InputEvent::Edited => cx.notify(),
                _ => {}
            },
        )
        .detach();
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.review_comments.cursor = Some(anchor.clone());
        self.remeasure_review_anchor(&anchor);
        self.review_comments.editor = Some(CommentEditor {
            session,
            anchor,
            line_text,
            quote,
            editing,
            input,
        });
        cx.notify();
    }

    /// Keep what the editor holds: a new comment, or the edited one. An
    /// empty editor keeps nothing (and an emptied comment is removed).
    fn save_review_comment(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.review_comments.editor.take() else {
            return;
        };
        let body = editor.input.read(cx).content().trim().to_owned();
        let comments = self
            .review_comments
            .by_session
            .entry(editor.session)
            .or_default();
        match editor.editing {
            Some(id) => {
                if body.is_empty() {
                    comments.retain(|comment| comment.id != id);
                } else if let Some(comment) = comments.iter_mut().find(|comment| comment.id == id) {
                    comment.body = body;
                }
            }
            None if !body.is_empty() => {
                self.review_comments.next_id += 1;
                comments.push(ReviewComment {
                    id: self.review_comments.next_id,
                    anchor: editor.anchor.clone(),
                    line_text: editor.line_text,
                    quote: editor.quote,
                    body,
                });
            }
            None => {}
        }
        self.remeasure_review_anchor(&editor.anchor);
        cx.notify();
    }

    fn cancel_review_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.review_comments.editor.take() {
            self.remeasure_review_anchor(&editor.anchor);
        }
        let focus = self.transcript_control_focus("review-diff-body", cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn edit_review_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.review_comment_session() else {
            return;
        };
        let Some(comment) = self
            .review_comments
            .by_session
            .get(&session)
            .and_then(|comments| comments.iter().find(|comment| comment.id == id))
            .cloned()
        else {
            return;
        };
        self.start_review_editor(
            session,
            comment.anchor,
            comment.line_text,
            comment.quote,
            Some(id),
            comment.body,
            window,
            cx,
        );
    }

    fn delete_review_comment(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.review_comment_session() else {
            return;
        };
        let mut removed = None;
        if let Some(comments) = self.review_comments.by_session.get_mut(&session) {
            if let Some(position) = comments.iter().position(|comment| comment.id == id) {
                removed = Some(comments.remove(position).anchor);
            }
        }
        if let Some(anchor) = removed {
            self.remeasure_review_anchor(&anchor);
        }
        cx.notify();
    }

    /// Send every comment on this task as one message, then forget them.
    pub(super) fn submit_review_comments(&mut self, cx: &mut Context<Self>) {
        // A comment still in the editor goes along.
        if self.review_comments.editor.is_some() {
            self.save_review_comment(cx);
        }
        let Some(session_id) = self.review_comment_session() else {
            return;
        };
        let Some(comments) = self
            .review_comments
            .by_session
            .remove(&session_id)
            .filter(|comments| !comments.is_empty())
        else {
            return;
        };
        let busy = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .is_some_and(|session| session.status.is_busy());
        let message = format_review_message(&comments);
        self.submit_submission_for_session(session_id, ComposerSubmission::plain(message), cx);
        for comment in &comments {
            self.remeasure_review_anchor(&comment.anchor);
        }
        self.show_success_toast(if busy {
            tr!("review_comments.queued", count = comments.len())
        } else {
            tr!("review_comments.sent", count = comments.len())
        });
        cx.notify();
    }

    fn discard_review_comments(&mut self, cx: &mut Context<Self>) {
        let count = self.review_comments_here().len();
        if count == 0 {
            return;
        }
        self.request_confirm(
            tr!("review_comments.discard_title", count = count),
            None,
            tr!("review_comments.discard"),
            true,
            cx,
            |this, _, cx| {
                if let Some(session) = this.review_comment_session()
                    && let Some(comments) = this.review_comments.by_session.remove(&session)
                {
                    for comment in &comments {
                        this.remeasure_review_anchor(&comment.anchor);
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn submit_review_comments_action(
        &mut self,
        _: &super::shortcuts::SubmitReviewComments,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_review_comments(cx);
    }

    pub(super) fn dismiss_review_comment_action(
        &mut self,
        _: &super::shortcuts::DismissReviewComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.review_comments.editor.is_some() {
            self.cancel_review_comment(window, cx);
        } else if self.review_comments.cursor.take().is_some() {
            cx.notify();
        }
    }

    /// Arrow keys over the code rows, Enter or `c` to comment on the one the
    /// cursor is on. Resolved on the key press, never per frame.
    fn review_diff_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.review_comments.editor.is_some() {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        let Some(snapshot) = self.right_panel_diff_snapshot.clone() else {
            return;
        };
        let current = self
            .review_comments
            .cursor
            .as_ref()
            .and_then(|anchor| self.review_row_of(anchor));
        let target = match event.keystroke.key.as_str() {
            "down" => step_code_row(
                &snapshot.lines,
                current,
                if current.is_some() { 1 } else { 0 },
            ),
            "up" => step_code_row(&snapshot.lines, current, -1),
            "pagedown" => step_code_row(&snapshot.lines, current, 20),
            "pageup" => step_code_row(&snapshot.lines, current, -20),
            "home" => step_code_row(&snapshot.lines, None, 0),
            "end" => step_code_row(&snapshot.lines, Some(snapshot.lines.len()), isize::MAX / 2),
            "enter" | "c" => {
                if let Some(index) = current {
                    self.open_review_comment(index, window, cx);
                    cx.stop_propagation();
                }
                return;
            }
            _ => return,
        };
        let Some(target) = target else {
            return;
        };
        let anchor = snapshot
            .lines
            .get(target)
            .and_then(|line| Anchor::of(line, &snapshot.files.get(line.file_index)?.path));
        if let Some(previous) = self
            .review_comments
            .cursor
            .replace(anchor.clone().unwrap_or_else(|| Anchor {
                path: String::new(),
                side: Side::New,
                line: 0,
            }))
        {
            self.remeasure_review_anchor(&previous);
        }
        if let Some(anchor) = &anchor {
            self.remeasure_review_anchor(anchor);
        }
        self.right_panel_diff_list_state
            .scroll_to_reveal_item(target);
        cx.stop_propagation();
        cx.notify();
    }

    /// The diff list's container: focusable, so the keyboard reaches the
    /// lines, with the bindings that only apply there.
    pub(super) fn review_diff_body(&self, element: Div, cx: &mut Context<Self>) -> Stateful<Div> {
        let focus = self.transcript_control_focus("review-diff-body", cx);
        element
            .id("review-diff-body")
            .key_context(REVIEW_DIFF_CONTEXT)
            .track_focus(&focus)
            .tab_index(0)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.review_diff_key(event, window, cx);
            }))
    }

    /// A code row with what comments add to it: the gutter's "+" on hover
    /// (or on the keyboard cursor), the comments written on the line, and
    /// the editor when it is open there.
    pub(super) fn review_comment_row(
        &self,
        line: &Line,
        index: usize,
        row: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(snapshot) = self.right_panel_diff_snapshot.as_ref() else {
            return row;
        };
        let Some(anchor) = snapshot
            .files
            .get(line.file_index)
            .and_then(|file| Anchor::of(line, &file.path))
        else {
            return row;
        };
        let theme = Theme::current(cx);
        let style = DiffRowStyle::review(self.state.code_font_size);
        let session = self.review_comment_session();
        let comments: Vec<ReviewComment> = self
            .review_comments_here()
            .iter()
            .filter(|comment| comment.anchor == anchor && comment.line_text == line.content)
            .cloned()
            .collect();
        let editor = self
            .review_comments
            .editor
            .as_ref()
            .filter(|editor| Some(editor.session) == session && editor.anchor == anchor);
        // The cursor exists only after an arrow key moved it, and Escape
        // clears it, so a mouse user never sees it.
        let on_cursor = self.review_comments.cursor.as_ref() == Some(&anchor);
        let group = SharedString::from(format!("review-comment-row-{index}"));

        let add = div()
            .id(SharedString::from(format!("review-comment-add-{index}")))
            .absolute()
            .left(px(2.0))
            .top(px(2.0))
            .size(px(16.0))
            .rounded(px(4.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.inverse)
            .cursor_pointer()
            .opacity(if on_cursor { 1.0 } else { 0.0 })
            .group_hover(group.clone(), |style| style.opacity(1.0))
            .tooltip(Tooltip::text(tr!("review_comments.add")))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_review_comment(index, window, cx);
            }))
            .child(icon("icons/plus.svg", 11.0, theme.on_inverse));

        let gutter = style.gutter_width();
        let cards = comments
            .into_iter()
            .map(|comment| {
                let id = comment.id;
                div()
                    .ml(px(gutter))
                    .mr(px(10.0))
                    .my(px(4.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .text_size(sp(12.5))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_color(theme.text_tertiary)
                            .child(icon("icons/pencil.svg", 12.0, theme.text_tertiary))
                            .child(tr!("review_comments.yours")),
                    )
                    .child(div().text_color(theme.text).child(comment.body.clone()))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(card_button(
                                theme,
                                SharedString::from(format!("review-comment-edit-{id}")),
                                tr!("review_comments.edit"),
                                false,
                                false,
                                cx,
                                move |this, window, cx| this.edit_review_comment(id, window, cx),
                            ))
                            .child(card_button(
                                theme,
                                SharedString::from(format!("review-comment-delete-{id}")),
                                tr!("review_comments.delete"),
                                false,
                                false,
                                cx,
                                move |this, _, cx| this.delete_review_comment(id, cx),
                            )),
                    )
            })
            .collect::<Vec<_>>();
        let editor = editor.map(|editor| {
            let input = editor.input.clone();
            div()
                .key_context(REVIEW_COMMENT_EDITOR_CONTEXT)
                // ⌘↩ in a multi-line field propagates its steering action;
                // here it sends every comment, this one included.
                .on_action(cx.listener(|this, _: &SubmitSteer, _, cx| {
                    this.submit_review_comments(cx);
                }))
                .ml(px(gutter))
                .mr(px(10.0))
                .my(px(4.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.accent)
                .bg(theme.surface)
                .flex()
                .flex_col()
                .gap(px(6.0))
                .text_size(sp(13.0))
                .child(div().min_h(px(40.0)).child(input))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(card_button(
                            theme,
                            SharedString::from("review-comment-save"),
                            tr!("review_comments.save"),
                            true,
                            false,
                            cx,
                            |this, _, cx| this.save_review_comment(cx),
                        ))
                        .child(card_button(
                            theme,
                            SharedString::from("review-comment-cancel"),
                            tr!("review_comments.cancel"),
                            false,
                            false,
                            cx,
                            |this, window, cx| this.cancel_review_comment(window, cx),
                        ))
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(sp(11.5))
                                .text_color(theme.text_tertiary)
                                .child(tr!("review_comments.editor_hint")),
                        ),
                )
        });
        let has_comments = self
            .review_comments_here()
            .iter()
            .any(|comment| comment.anchor == anchor && comment.line_text == line.content);

        div()
            .w_full()
            .flex()
            .flex_col()
            .flex_none()
            .child(
                div()
                    .group(group)
                    .relative()
                    .w_full()
                    .when(on_cursor, |row| {
                        row.border_1().border_color(theme.accent.opacity(0.6))
                    })
                    .when(has_comments && !on_cursor, |row| {
                        row.border_r_2().border_color(theme.accent)
                    })
                    .child(row)
                    .child(add),
            )
            .children(cards)
            .children(editor)
            .into_any_element()
    }

    /// Under the diff while this task has comments: how many, how many are
    /// not under a line on screen, and the buttons to send or drop them.
    pub(super) fn render_review_comment_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let count = self.review_comments_here().len();
        if count == 0 {
            return None;
        }
        let theme = Theme::current(cx);
        let hidden = self.review_comments_out_of_view();
        let mut summary = tr!("review_comments.count", count = count);
        if hidden > 0 {
            summary.push_str(" · ");
            summary.push_str(&tr!("review_comments.out_of_view", count = hidden));
        }
        Some(
            div()
                .flex_none()
                .w_full()
                .px(px(12.0))
                .py(px(8.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .border_t_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .child(icon("icons/pencil.svg", 13.0, theme.text_secondary))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .child(summary),
                )
                .child(card_button(
                    theme,
                    SharedString::from("review-comments-discard"),
                    tr!("review_comments.discard"),
                    false,
                    false,
                    cx,
                    |this, _, cx| this.discard_review_comments(cx),
                ))
                .child(card_button(
                    theme,
                    SharedString::from("review-comments-submit"),
                    format!(
                        "{} {}",
                        tr!("review_comments.submit"),
                        crate::platform::primary_shortcut("⌘↩", "Ctrl+Enter")
                    ),
                    true,
                    false,
                    cx,
                    |this, _, cx| this.submit_review_comments(cx),
                ))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, content: &str) -> Line {
        Line {
            file_index: 0,
            old_line: old,
            new_line: new,
            kind,
            content: content.to_owned(),
            tokens: Vec::new(),
        }
    }

    #[test]
    fn a_line_is_anchored_on_its_own_side() {
        let added = line(LineKind::Addition, None, Some(12), "let a = 1;");
        let removed = line(LineKind::Deletion, Some(9), None, "let a = 0;");
        let context = line(LineKind::Context, Some(8), Some(11), "fn f() {");
        assert_eq!(
            Anchor::of(&added, "src/a.rs"),
            Some(Anchor {
                path: "src/a.rs".into(),
                side: Side::New,
                line: 12
            })
        );
        assert_eq!(Anchor::of(&removed, "src/a.rs").unwrap().side, Side::Old);
        assert_eq!(Anchor::of(&context, "src/a.rs").unwrap().line, 11);
        assert_eq!(
            Anchor::of(&line(LineKind::FileHeader, None, None, ""), "x"),
            None
        );
        // A synthesized diff without numbers cannot be anchored.
        assert_eq!(
            Anchor::of(&line(LineKind::Addition, None, None, "x"), "x"),
            None
        );
    }

    #[test]
    fn the_quote_is_the_line_and_the_two_before_it_within_the_hunk() {
        let lines = vec![
            line(LineKind::Context, Some(1), Some(1), "a"),
            line(LineKind::Context, Some(2), Some(2), "b"),
            line(LineKind::Deletion, Some(3), None, "c"),
            line(LineKind::Addition, None, Some(3), "C"),
        ];
        let quote = quote_for(&lines, 3);
        assert_eq!(
            quote,
            vec![
                QuoteLine {
                    marker: ' ',
                    text: "b".into()
                },
                QuoteLine {
                    marker: '-',
                    text: "c".into()
                },
                QuoteLine {
                    marker: '+',
                    text: "C".into()
                },
            ]
        );
        assert_eq!(quote_for(&lines, 0).len(), 1);
    }

    #[test]
    fn the_cursor_moves_over_code_rows_only() {
        let lines = vec![
            line(LineKind::FileHeader, None, None, ""),
            line(LineKind::Context, Some(1), Some(1), "a"),
            line(LineKind::Addition, None, Some(2), "b"),
            line(LineKind::FileHeader, None, None, ""),
            line(LineKind::Deletion, Some(4), None, "d"),
        ];
        assert_eq!(step_code_row(&lines, None, 0), Some(1));
        assert_eq!(step_code_row(&lines, Some(1), 1), Some(2));
        assert_eq!(step_code_row(&lines, Some(2), 1), Some(4));
        assert_eq!(step_code_row(&lines, Some(4), 1), Some(4));
        assert_eq!(step_code_row(&lines, Some(1), -1), Some(1));
        assert_eq!(step_code_row(&lines, Some(5), isize::MAX / 2), Some(4));
    }

    #[test]
    fn the_message_orders_comments_and_quotes_each() {
        let comment = |path: &str, side: Side, line: u32, body: &str| ReviewComment {
            id: 0,
            anchor: Anchor {
                path: path.into(),
                side,
                line,
            },
            line_text: String::new(),
            quote: vec![QuoteLine {
                marker: '+',
                text: "x".into(),
            }],
            body: body.into(),
        };
        let message = format_review_message(&[
            comment("src/b.rs", Side::New, 3, "rename it"),
            comment("src/a.rs", Side::Old, 7, " keep this "),
        ]);
        let a = message.find("src/a.rs").unwrap();
        let b = message.find("src/b.rs").unwrap();
        assert!(a < b, "{message}");
        assert!(message.contains("removed line 7 of the old version"));
        assert!(message.contains("`src/b.rs:3`"));
        assert!(message.contains("```diff\n+x\n```"));
        assert!(message.contains("Comment: keep this\n"));
    }
}
