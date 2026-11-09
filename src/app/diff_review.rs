//! Search, reviewed-state and visibility orchestration for the Review diff.
//!
//! The source snapshot stays immutable during file folding. Search entries use
//! logical order across visible lines and gap-hidden lines, so expanding a gap
//! does not move a match to another result.

use std::collections::HashSet;

use super::*;
use crate::review_diff::{ExpansionDirection, LineKind, Snapshot};
use crate::ui::text_field::TextField;
use crate::ui::tooltip::Tooltip;
use crate::ui::{ActivationExt as _, icon, icon_button, ui_theme};
use client_ui::{Banner, BannerSurface};

#[derive(Clone, Debug, Eq, PartialEq)]
struct DiffSearchEntry {
    file_index: usize,
    /// Current source-line index; `None` while this line is inside a gap.
    line_index: Option<usize>,
    gap_id: Option<u64>,
    content: String,
}

fn diff_search_entries(snapshot: &Snapshot) -> Vec<DiffSearchEntry> {
    let mut entries = Vec::with_capacity(snapshot.lines.len());
    for (line_index, line) in snapshot.lines.iter().enumerate() {
        if let LineKind::Gap(gap) = &line.kind {
            entries.extend(gap.hidden_lines().iter().map(|hidden| DiffSearchEntry {
                file_index: hidden.file_index,
                line_index: None,
                gap_id: Some(gap.id),
                content: hidden.content.clone(),
            }));
        } else {
            entries.push(DiffSearchEntry {
                file_index: line.file_index,
                line_index: Some(line_index),
                gap_id: None,
                content: line.content.clone(),
            });
        }
    }
    entries
}

fn diff_search_matches(snapshot: &Snapshot, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let query = query.to_lowercase();
    diff_search_entries(snapshot)
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            entry
                .content
                .to_lowercase()
                .contains(&query)
                .then_some(index)
        })
        .collect()
}

fn visible_diff_rows(snapshot: &Snapshot, collapsed: &HashSet<String>) -> Vec<usize> {
    snapshot
        .lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let file_collapsed = snapshot
                .files
                .get(line.file_index)
                .is_some_and(|file| collapsed.contains(&file.path));
            (!file_collapsed || matches!(&line.kind, LineKind::FileHeader)).then_some(index)
        })
        .collect()
}

impl Waku {
    pub(super) fn reset_right_panel_diff_search_state(&mut self) {
        self.right_panel_diff_search_open = false;
        self.right_panel_diff_search_matches.clear();
        self.right_panel_diff_search_current = None;
        self.right_panel_diff_search_line = None;
    }

    pub(super) fn open_right_panel_diff_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right_panel_diff_search_open = true;
        self.right_panel_diff_search.update(cx, |input, cx| {
            input.select_all_text(cx);
        });
        window.focus(&self.right_panel_diff_search.read(cx).focus(), cx);
        self.sync_right_panel_diff_search(cx);
        cx.notify();
    }

    pub(super) fn close_right_panel_diff_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.right_panel_diff_search_open {
            return;
        }
        self.reset_right_panel_diff_search_state();
        let focus = self.transcript_control_focus("review-diff-body", cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Recompute matches without changing the logical cursor when possible.
    pub(super) fn refresh_right_panel_diff_search(&mut self, cx: &mut Context<Self>) {
        let query = self.right_panel_diff_search.read(cx).content().to_owned();
        self.right_panel_diff_search_matches = self
            .right_panel_diff_snapshot
            .as_deref()
            .map(|snapshot| diff_search_matches(snapshot, &query))
            .unwrap_or_default();
        self.right_panel_diff_search_current = if self.right_panel_diff_search_matches.is_empty() {
            self.right_panel_diff_search_line = None;
            None
        } else {
            Some(0)
        };
        cx.notify();
    }

    pub(super) fn sync_right_panel_diff_search(&mut self, cx: &mut Context<Self>) {
        if !self.right_panel_diff_search_open {
            return;
        }
        self.refresh_right_panel_diff_search(cx);
        self.reveal_right_panel_diff_search_match(cx);
    }

    pub(super) fn navigate_right_panel_diff_search(
        &mut self,
        backwards: bool,
        cx: &mut Context<Self>,
    ) {
        let count = self.right_panel_diff_search_matches.len();
        if count == 0 {
            return;
        }
        let current = self.right_panel_diff_search_current.unwrap_or(0);
        self.right_panel_diff_search_current = Some(if backwards {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        });
        self.reveal_right_panel_diff_search_match(cx);
        cx.notify();
    }

    fn reveal_right_panel_diff_search_match(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = self.right_panel_diff_search_current else {
            return;
        };
        let Some(entry_index) = self.right_panel_diff_search_matches.get(cursor).copied() else {
            return;
        };
        let Some(snapshot) = self.right_panel_diff_snapshot.clone() else {
            return;
        };
        let Some(entry) = diff_search_entries(&snapshot).get(entry_index).cloned() else {
            return;
        };

        // A reviewed file remains reviewed when search reveals it; only its
        // collapsed presentation changes.
        self.reveal_right_panel_diff_file(entry.file_index, cx);

        let mut line_index = entry.line_index;
        if line_index.is_none()
            && let Some(gap_id) = entry.gap_id
            && let Some(gap_index) =
                self.right_panel_diff_snapshot
                    .as_deref()
                    .and_then(|snapshot| {
                        snapshot.lines.iter().position(
                            |line| matches!(&line.kind, LineKind::Gap(gap) if gap.id == gap_id),
                        )
                    })
        {
            self.expand_right_panel_diff_gap(gap_index, ExpansionDirection::All, cx);
            line_index = self
                .right_panel_diff_snapshot
                .as_deref()
                .and_then(|snapshot| {
                    diff_search_entries(snapshot)
                        .get(entry_index)
                        .and_then(|entry| entry.line_index)
                });
        }

        self.right_panel_diff_search_line = line_index;
        self.right_panel_diff_selected_file = Some(entry.file_index);
        if let Some(row) = line_index.and_then(|line| self.right_panel_diff_visible_row(line)) {
            self.right_panel_diff_list_state.scroll_to_reveal_item(row);
        }
        cx.notify();
    }

    fn right_panel_diff_visible_row(&self, line_index: usize) -> Option<usize> {
        self.right_panel_diff_visible_rows
            .borrow()
            .iter()
            .position(|candidate| *candidate == line_index)
    }

    pub(super) fn render_right_panel_diff_search_bar(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(cx);
        let count = self.right_panel_diff_search_matches.len();
        let current = self
            .right_panel_diff_search_current
            .map_or(0, |index| index + 1);
        div()
            .h(px(38.0))
            .flex_none()
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .border_b_1()
            .border_color(theme.border)
            .child(
                TextField::new(
                    "right-panel-diff-search",
                    self.right_panel_diff_search.clone(),
                )
                .icon("icons/search.svg", 13.0)
                .flex_1(),
            )
            .child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_tertiary)
                    .child(format!("{current}/{count}")),
            )
            .child(
                icon_button(
                    "right-panel-diff-search-previous",
                    "icons/chevron-up.svg",
                    theme,
                )
                .tab_index(0)
                .focus_visible(|style| style.border_1().border_color(theme.accent))
                .tooltip(Tooltip::text(tr!("diff.search_previous")))
                .on_activation(cx, |this, _, cx| {
                    this.navigate_right_panel_diff_search(true, cx);
                }),
            )
            .child(
                icon_button(
                    "right-panel-diff-search-next",
                    "icons/chevron-down.svg",
                    theme,
                )
                .tab_index(0)
                .focus_visible(|style| style.border_1().border_color(theme.accent))
                .tooltip(Tooltip::text(tr!("diff.search_next")))
                .on_activation(cx, |this, _, cx| {
                    this.navigate_right_panel_diff_search(false, cx);
                }),
            )
            .child(
                Banner::dismiss(
                    "right-panel-diff-search-close",
                    BannerSurface::Card,
                    ui_theme(theme),
                    cx,
                    |this, window, cx| this.close_right_panel_diff_search(window, cx),
                )
                .tooltip(Tooltip::text(tr!("diff.search_close")))
                .child(icon("icons/x.svg", 12.0, theme.text_tertiary)),
            )
            .into_any_element()
    }

    pub(super) fn right_panel_diff_file_is_viewed(&self, file_index: usize) -> bool {
        let Some(snapshot) = self.right_panel_diff_snapshot.as_deref() else {
            return false;
        };
        snapshot
            .files
            .get(file_index)
            .is_some_and(|file| self.right_panel_diff_viewed_files.contains_key(&file.path))
    }

    pub(super) fn right_panel_diff_viewed_count(&self) -> usize {
        self.right_panel_diff_viewed_files.len()
    }

    /// Drop reviewed marks whose file disappeared or whose content changed.
    pub(super) fn retain_right_panel_diff_viewed(&mut self, snapshot: &Snapshot) {
        let fingerprints = snapshot
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                snapshot
                    .file_fingerprint(index)
                    .map(|fingerprint| (file.path.clone(), fingerprint))
            })
            .collect::<std::collections::HashMap<_, _>>();
        self.right_panel_diff_viewed_files
            .retain(|path, fingerprint| fingerprints.get(path) == Some(fingerprint));
        self.right_panel_diff_collapsed_files
            .retain(|path| self.right_panel_diff_viewed_files.contains_key(path));
    }

    pub(super) fn sync_right_panel_diff_visible_rows(&mut self, cx: &mut Context<Self>) {
        self.sync_right_panel_diff_visible_rows_state();
        cx.notify();
    }

    pub(super) fn sync_right_panel_diff_visible_rows_state(&mut self) {
        let rows = self
            .right_panel_diff_snapshot
            .as_deref()
            .map_or_else(Vec::new, |snapshot| {
                visible_diff_rows(snapshot, &self.right_panel_diff_collapsed_files)
            });
        let count = rows.len();
        *self.right_panel_diff_visible_rows.borrow_mut() = rows;
        self.right_panel_diff_list_state.reset(count);
    }

    pub(super) fn reveal_right_panel_diff_file(
        &mut self,
        file_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .right_panel_diff_snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.files.get(file_index))
            .map(|file| file.path.clone())
        else {
            return;
        };
        if self.right_panel_diff_collapsed_files.remove(&path) {
            self.sync_right_panel_diff_visible_rows(cx);
        }
        self.right_panel_diff_selected_file = Some(file_index);
        if let Some(line) = self
            .right_panel_diff_snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.files.get(file_index))
            .and_then(|file| file.diff_line)
            && let Some(row) = self.right_panel_diff_visible_row(line)
        {
            self.right_panel_diff_list_state
                .scroll_to(gpui::ListOffset {
                    item_ix: row,
                    offset_in_item: px(0.0),
                });
        }
        cx.notify();
    }

    pub(super) fn toggle_right_panel_diff_file_viewed(
        &mut self,
        file_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.right_panel_diff_snapshot.clone() else {
            return;
        };
        let Some(file) = snapshot.files.get(file_index) else {
            return;
        };
        let path = file.path.clone();
        if self.right_panel_diff_file_is_viewed(file_index) {
            self.right_panel_diff_viewed_files.remove(&path);
            self.right_panel_diff_collapsed_files.remove(&path);
        } else if let Some(fingerprint) = snapshot.file_fingerprint(file_index) {
            self.right_panel_diff_viewed_files
                .insert(path.clone(), fingerprint);
            self.right_panel_diff_collapsed_files.insert(path.clone());
            if self
                .right_panel_diff_search_line
                .and_then(|line| snapshot.lines.get(line))
                .is_some_and(|line| line.file_index == file_index)
            {
                self.right_panel_diff_search_line = None;
            }
        }
        self.sync_right_panel_diff_visible_rows(cx);
        if let Some(line) = file.diff_line
            && let Some(row) = self.right_panel_diff_visible_row(line)
        {
            self.right_panel_diff_list_state
                .scroll_to(gpui::ListOffset {
                    item_ix: row,
                    offset_in_item: px(0.0),
                });
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_diff::{Gap, GapPosition, Line};

    fn line(content: &str) -> Line {
        Line {
            file_index: 0,
            old_line: None,
            new_line: None,
            kind: LineKind::Context,
            content: content.into(),
            tokens: Vec::new(),
        }
    }

    fn snapshot_with_hidden_line() -> Snapshot {
        Snapshot {
            source: crate::review_diff::Source::Uncommitted,
            files: vec![crate::review_diff::File {
                path: "src/lib.rs".into(),
                additions: 0,
                deletions: 0,
                status: crate::review_diff::FileStatus::Modified,
                diff_line: Some(0),
            }],
            lines: vec![
                line("visible"),
                Line {
                    kind: LineKind::Gap(Gap::new_for_test(
                        7,
                        vec![line("needle in hidden context")],
                        GapPosition::Between,
                    )),
                    ..line("")
                },
                line("visible after"),
            ],
            additions: 0,
            deletions: 0,
            truncated: false,
        }
    }

    #[test]
    fn search_includes_hidden_lines_in_logical_order() {
        let snapshot = snapshot_with_hidden_line();
        let entries = diff_search_entries(&snapshot);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].gap_id, Some(7));
        assert_eq!(entries[1].line_index, None);
        assert_eq!(diff_search_matches(&snapshot, "hidden"), vec![1]);
        assert_eq!(diff_search_matches(&snapshot, "VISIBLE"), vec![0, 2]);
        assert!(diff_search_matches(&snapshot, "").is_empty());
    }

    #[test]
    fn folding_keeps_the_file_header_and_searchable_source() {
        let mut snapshot = snapshot_with_hidden_line();
        snapshot.lines[0].kind = LineKind::FileHeader;
        let collapsed = HashSet::from(["src/lib.rs".to_owned()]);
        assert_eq!(visible_diff_rows(&snapshot, &collapsed), vec![0]);
        assert_eq!(diff_search_matches(&snapshot, "hidden"), vec![1]);
    }

    #[test]
    fn expanding_a_gap_preserves_search_order_and_reviewed_fingerprint() {
        let mut snapshot = snapshot_with_hidden_line();
        let fingerprint = snapshot.file_fingerprint(0);
        snapshot.expand_gap(1, ExpansionDirection::All).unwrap();
        assert_eq!(snapshot.file_fingerprint(0), fingerprint);
        let entries = diff_search_entries(&snapshot);
        assert_eq!(entries[1].line_index, Some(1));
        assert_eq!(diff_search_matches(&snapshot, "hidden"), vec![1]);
    }
}
