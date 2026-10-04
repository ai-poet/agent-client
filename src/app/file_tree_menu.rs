//! The Files panel's context menu, keyboard and in-place edits.
//!
//! Fork addition. A row used to take one left click — expand a folder or
//! open a file in the editor. Now a right click (or Shift+F10 / the menu key
//! on the keyboard cursor) offers: open, open with the system's default app,
//! reveal in the file manager, mention in the composer, copy the path or the
//! relative path, new file / new folder, rename, move to the recycle bin and
//! refresh. Rows are keyboard operable: arrows, Home/End, Enter, F2, Delete.
//!
//! Creating, renaming and deleting run on the daemon host like every other
//! workspace file operation (`WorkspaceOperation::{CreateFile,
//! CreateDirectory, RenamePath, TrashPath}`), so they work on a remote
//! workspace too. Opening with the default app and revealing act on this
//! machine and are offered only when the daemon is local.
//!
//! A child of `right_panel`, so it can reuse that module's private helpers;
//! the hook points there are a handful of lines in the tree's render.

use std::cell::Cell;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::rc::Rc;

use gpui::{ClipboardItem, MouseButton, MouseDownEvent, Subscription};

use super::*;

const FILE_TREE_MENU_ID: &str = "file-tree";

/// The Files panel's interaction state, one field on [`Waku`].
#[derive(Default)]
pub(in crate::app) struct FileTreeUi {
    /// The row a context menu was opened on; `None` is the tree's background.
    menu_target: Option<FileTarget>,
    /// The keyboard cursor, by relative path.
    cursor: Option<String>,
    /// The last interaction was the keyboard, so the cursor shows.
    keyboard: bool,
    /// The cursor row's bounds as of the last frame, where a keyboard-opened
    /// menu is anchored.
    cursor_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    edit: Option<FileEdit>,
    input: Option<Entity<TextInput>>,
    _input_events: Option<Subscription>,
}

/// One row, as the menu and the edits need it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FileTarget {
    relative_path: String,
    absolute_path: PathBuf,
    is_dir: bool,
    expanded: bool,
    depth: usize,
}

impl FileTarget {
    pub(super) fn of(entry: &WorkingTreeEntry) -> Self {
        Self {
            relative_path: entry.relative_path.clone(),
            absolute_path: entry.absolute_path.clone(),
            is_dir: entry.is_dir,
            expanded: entry.expanded,
            depth: entry.depth,
        }
    }

    fn name(&self) -> &str {
        Path::new(&self.relative_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&self.relative_path)
    }

    /// Where a new file goes when this row is the target: inside a folder,
    /// beside a file.
    fn container(&self) -> String {
        if self.is_dir {
            self.relative_path.clone()
        } else {
            parent_of(&self.relative_path)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FileEdit {
    Rename { target: FileTarget },
    Create { parent: String, directory: bool },
}

/// The parent of a relative path, `""` for the workspace root.
fn parent_of(relative: &str) -> String {
    Path::new(relative)
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A relative path as the composer's `@` mentions write it: forward slashes,
/// and a trailing one for a folder.
fn mention_path(relative: &str, is_dir: bool) -> String {
    let mut path = relative.replace('\\', "/");
    if is_dir && !path.ends_with('/') {
        path.push('/');
    }
    path
}

/// A name typed into the tree, as a path under `parent`. `None` when it is
/// empty or would leave the folder it is typed in.
fn typed_path(parent: &str, name: &str) -> Option<String> {
    let name = name.trim().replace(['/', '\\'], &MAIN_SEPARATOR.to_string());
    if name.is_empty()
        || Path::new(&name).is_absolute()
        || Path::new(&name)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(if parent.is_empty() {
        name
    } else {
        Path::new(parent).join(name).to_string_lossy().into_owned()
    })
}

/// `path` moved from under `from` to under `to`, or `None` when it was not
/// under `from` at all.
fn moved_path(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        return Some(to.to_owned());
    }
    let rest = path.strip_prefix(from)?;
    rest.starts_with(['/', '\\'])
        .then(|| format!("{to}{rest}"))
}

fn is_under(path: &str, root: &str) -> bool {
    moved_path(path, root, root).is_some()
}

/// The span a rename preselects: the name without its extension, as file
/// managers do, or all of it for a folder or a dotfile.
fn stem_range(name: &str, is_dir: bool) -> std::ops::Range<usize> {
    if is_dir {
        return 0..name.len();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => 0..dot,
        _ => 0..name.len(),
    }
}

impl Waku {
    fn file_tree_menu(&self, cx: &mut App) -> ContextMenuHandle {
        self.menu_handle(FILE_TREE_MENU_ID, cx)
    }

    /// Hook: every tree row. Records the row a right click lands on (the
    /// tree's own handler opens the menu after it), moves the keyboard
    /// cursor on a left click, marks the cursor row, and swaps the row for a
    /// name field while it is being renamed.
    pub(super) fn decorate_file_tree_row(
        &self,
        row: Stateful<Div>,
        target: FileTarget,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        if let Some(FileEdit::Rename { target: renaming }) = &self.file_tree.edit
            && renaming.relative_path == target.relative_path
        {
            return self.render_file_edit_row(target.depth, target.is_dir, cx);
        }
        let theme = Theme::current(cx);
        let cursor = self.file_tree.keyboard
            && self.file_tree.cursor.as_deref() == Some(target.relative_path.as_str());
        let bounds = self.file_tree.cursor_bounds.clone();
        let right = target.clone();
        let left = target.relative_path.clone();
        row.relative()
            // Every row keeps a border, so the cursor's does not shift it.
            .border_1()
            .border_color(if cursor {
                theme.accent
            } else {
                gpui::transparent_black()
            })
            .when(cursor, |row| {
                row.child(
                    canvas(|_, _, _| (), move |row_bounds, _, _, _| bounds.set(Some(row_bounds)))
                        .absolute()
                        .size_full(),
                )
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _: &MouseDownEvent, _, _| {
                    this.file_tree.menu_target = Some(right.clone());
                    this.file_tree.cursor = Some(right.relative_path.clone());
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, _| {
                    this.file_tree.cursor = Some(left.clone());
                    this.file_tree.keyboard = false;
                }),
            )
    }

    /// Hook: after each row, and once before the first with `None`. The
    /// field for a new file or folder, under the folder it goes into.
    pub(super) fn file_tree_create_row(
        &self,
        after: Option<&FileTarget>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Some(FileEdit::Create { parent, directory }) = &self.file_tree.edit else {
            return None;
        };
        let (matches, depth) = match after {
            None => (parent.is_empty(), 0),
            Some(target) => (target.is_dir && target.relative_path == *parent, target.depth + 1),
        };
        matches.then(|| {
            self.render_file_edit_row(depth, *directory, cx)
                .into_any_element()
        })
    }

    fn render_file_edit_row(
        &self,
        depth: usize,
        is_dir: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let field = self.file_tree.input.clone();
        div()
            .id("file-tree-edit")
            .h(px(30.0))
            .mx(px(8.0))
            .child(
                div()
                    .size_full()
                    .pl(px(8.0 + depth as f32 * 16.0))
                    .pr(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    // The row this replaces still carries its click (open
                    // the file, fold the folder); a press in here must not
                    // reach it.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            this.cancel_file_edit(window, cx);
                        }
                    }))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        if this.file_tree.edit.is_some() {
                            this.commit_file_edit(cx);
                        }
                    }))
                    .child(div().w(px(10.0)).h(px(10.0)).flex_none())
                    .child(if is_dir {
                        icon("icons/folder.svg", 13.0, theme.text_tertiary).into_any_element()
                    } else {
                        icon("icons/file.svg", 13.0, theme.text_tertiary).into_any_element()
                    })
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .h(px(22.0))
                            .px(px(4.0))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(theme.accent)
                            .bg(theme.inset)
                            .flex()
                            .items_center()
                            .text_size(sp(12.5))
                            .text_color(theme.text)
                            .children(field),
                    ),
            )
    }

    /// Hook: the tree's scroll area. It takes keyboard focus, clears a stale
    /// menu target before a right click reaches the rows, and opens the
    /// context menu.
    pub(super) fn file_tree_area(&self, area: Div, cx: &mut Context<Self>) -> AnyElement {
        let menu = self.file_tree_menu(cx);
        let focus = menu.trigger_focus_handle().clone();
        let weak = cx.entity().downgrade();
        let area = area
            .track_focus(&focus)
            .tab_index(0)
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Right {
                    this.file_tree.menu_target = None;
                }
            }))
            .on_key_down(cx.listener(Self::file_tree_key_down));
        context_menu(area, "file-tree-menu", &menu, move |cx| {
            weak.upgrade()
                .map(|waku| file_tree_menu_items(&waku, cx))
                .unwrap_or_default()
        })
    }

    /// Hook: actions in the Files header.
    pub(super) fn file_tree_header_actions(&self, cx: &mut Context<Self>) -> Div {
        let theme = Theme::current(cx);
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(
                icon_button("file-tree-new-file", "icons/plus.svg", theme)
                    .tooltip(|window, cx| Tooltip::new(tr!("file_tree.new_file")).build(window, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let parent = this.file_tree_focus_container();
                        this.begin_file_create(parent, false, window, cx);
                    })),
            )
            .child(
                icon_button("file-tree-new-folder", "icons/folder-new.svg", theme)
                    .tooltip(|window, cx| Tooltip::new(tr!("file_tree.new_folder")).build(window, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let parent = this.file_tree_focus_container();
                        this.begin_file_create(parent, true, window, cx);
                    })),
            )
            .child(
                icon_button("file-tree-refresh", "icons/rotate-cw.svg", theme)
                    .tooltip(|window, cx| Tooltip::new(tr!("files.refresh")).build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh_right_panel_working_tree(cx);
                        cx.notify();
                    })),
            )
    }

    /// The folder the header's "new" buttons create in: the cursor's.
    fn file_tree_focus_container(&self) -> String {
        self.file_tree
            .cursor
            .as_deref()
            .and_then(|cursor| self.file_tree_target(cursor))
            .map(|target| target.container())
            .unwrap_or_default()
    }

    fn file_tree_target(&self, relative_path: &str) -> Option<FileTarget> {
        self.right_panel_working_tree
            .iter()
            .find(|entry| entry.relative_path == relative_path)
            .map(FileTarget::of)
    }

    fn file_tree_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_tree.edit.is_some() {
            return;
        }
        let entries = &self.right_panel_working_tree;
        if entries.is_empty() {
            return;
        }
        let index = self.file_tree.cursor.as_deref().and_then(|cursor| {
            entries
                .iter()
                .position(|entry| entry.relative_path == cursor)
        });
        let current = index.map(|index| FileTarget::of(&entries[index]));
        let last = entries.len() - 1;
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        let move_to = |index: usize| Some(entries[index].relative_path.clone());
        let handled = match key {
            "down" => {
                self.file_tree.cursor = move_to(index.map_or(0, |index| (index + 1).min(last)));
                true
            }
            "up" => {
                self.file_tree.cursor = move_to(index.map_or(0, |index| index.saturating_sub(1)));
                true
            }
            "home" => {
                self.file_tree.cursor = move_to(0);
                true
            }
            "end" => {
                self.file_tree.cursor = move_to(last);
                true
            }
            "right" => match current {
                Some(target) if target.is_dir && !target.expanded => {
                    self.toggle_file_tree_dir(&target, cx);
                    true
                }
                Some(_) => {
                    self.file_tree.cursor = move_to(index.map_or(0, |index| (index + 1).min(last)));
                    true
                }
                None => false,
            },
            "left" => match current {
                Some(target) if target.is_dir && target.expanded => {
                    self.toggle_file_tree_dir(&target, cx);
                    true
                }
                Some(target) => {
                    let parent = parent_of(&target.relative_path);
                    if !parent.is_empty() {
                        self.file_tree.cursor = Some(parent);
                    }
                    true
                }
                None => false,
            },
            "enter" | "space" => match current {
                Some(target) if target.is_dir => {
                    self.toggle_file_tree_dir(&target, cx);
                    true
                }
                Some(target) => {
                    self.open_right_panel_file(target.relative_path, cx);
                    true
                }
                None => false,
            },
            "f2" => match current {
                Some(target) => {
                    self.begin_file_rename(target, window, cx);
                    true
                }
                None => false,
            },
            "delete" => match current {
                Some(target) => {
                    self.confirm_file_trash(target, cx);
                    true
                }
                None => false,
            },
            "f10" if shift => self.open_file_tree_menu_from_keyboard(current, window, cx),
            "menu" | "contextmenu" | "apps" => {
                self.open_file_tree_menu_from_keyboard(current, window, cx)
            }
            _ => false,
        };
        if handled {
            self.file_tree.keyboard = true;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn open_file_tree_menu_from_keyboard(
        &mut self,
        target: Option<FileTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.file_tree.menu_target = target;
        let menu = self.file_tree_menu(cx);
        match self.file_tree.cursor_bounds.get() {
            Some(bounds) if self.file_tree.menu_target.is_some() => {
                menu.open_context_menu_at(point(bounds.left() + px(8.0), bounds.bottom()), window, cx)
            }
            _ => menu.open_context_menu(window, cx),
        }
        true
    }

    fn toggle_file_tree_dir(&mut self, target: &FileTarget, cx: &mut Context<Self>) {
        if !self.right_panel_expanded_paths.remove(&target.absolute_path) {
            self.right_panel_expanded_paths
                .insert(target.absolute_path.clone());
        }
        self.refresh_right_panel_working_tree(cx);
        cx.notify();
    }

    fn begin_file_create(
        &mut self,
        parent: String,
        directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The field shows under its folder, so the folder has to be open.
        if let Some(root) = self.selected_workspace_path().map(Path::to_path_buf)
            && !parent.is_empty()
            && self.right_panel_expanded_paths.insert(root.join(&parent))
        {
            self.refresh_right_panel_working_tree(cx);
        }
        self.begin_file_edit(FileEdit::Create { parent, directory }, String::new(), 0..0, window, cx);
    }

    fn begin_file_rename(&mut self, target: FileTarget, window: &mut Window, cx: &mut Context<Self>) {
        let name = target.name().to_owned();
        let selection = stem_range(&name, target.is_dir);
        self.begin_file_edit(FileEdit::Rename { target }, name, selection, window, cx);
    }

    fn begin_file_edit(
        &mut self,
        edit: FileEdit,
        initial: String,
        selection: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = match self.file_tree.input.clone() {
            Some(input) => input,
            None => {
                let input = cx.new(|cx| TextInput::new(window, cx));
                self.file_tree._input_events = Some(cx.subscribe(
                    &input,
                    |this: &mut Self, _, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Submit(_)) {
                            this.commit_file_edit(cx);
                        }
                    },
                ));
                self.file_tree.input = Some(input.clone());
                input
            }
        };
        input.update(cx, |input, cx| {
            input.set_content(initial, cx);
            if selection.is_empty() {
                input.select_all_text(cx);
            } else {
                input.select_range(selection, cx);
            }
        });
        self.file_tree.edit = Some(edit);
        let focus = input.read(cx).focus();
        window.on_next_frame(move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    fn cancel_file_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_tree.edit.take().is_none() {
            return;
        }
        let focus = self.file_tree_menu(cx).trigger_focus_handle().clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    fn commit_file_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.file_tree.edit.take() else {
            return;
        };
        cx.notify();
        let typed = self
            .file_tree
            .input
            .as_ref()
            .map(|input| input.read(cx).content().trim().to_owned())
            .unwrap_or_default();
        if typed.is_empty() {
            return;
        }
        let Some(root) = self.selected_workspace_path().map(Path::to_path_buf) else {
            return;
        };
        let (operation, outcome) = match edit {
            FileEdit::Rename { target } => {
                let Some(to) = typed_path(&parent_of(&target.relative_path), &typed) else {
                    self.show_toast(tr!("file_tree.invalid_name"));
                    return;
                };
                if to == target.relative_path {
                    return;
                }
                (
                    waku_client::WorkspaceOperation::RenamePath {
                        root: root.clone(),
                        from: PathBuf::from(&target.relative_path),
                        to: PathBuf::from(&to),
                    },
                    FileEditOutcome::Renamed {
                        from: target.relative_path,
                        to,
                    },
                )
            }
            FileEdit::Create { parent, directory } => {
                let Some(path) = typed_path(&parent, &typed) else {
                    self.show_toast(tr!("file_tree.invalid_name"));
                    return;
                };
                let operation = if directory {
                    waku_client::WorkspaceOperation::CreateDirectory {
                        root: root.clone(),
                        relative_path: PathBuf::from(&path),
                    }
                } else {
                    waku_client::WorkspaceOperation::CreateFile {
                        root: root.clone(),
                        relative_path: PathBuf::from(&path),
                    }
                };
                (operation, FileEditOutcome::Created { path, directory })
            }
        };
        self.run_file_tree_operation(root, operation, outcome, cx);
    }

    fn confirm_file_trash(&mut self, target: FileTarget, cx: &mut Context<Self>) {
        let Some(root) = self.selected_workspace_path().map(Path::to_path_buf) else {
            return;
        };
        let name = target.name().to_owned();
        self.request_confirm(
            tr!("file_tree.trash_title", name = name),
            Some(tr!("file_tree.trash_detail")),
            tr!("file_tree.trash"),
            true,
            cx,
            move |waku, _, cx| {
                let operation = waku_client::WorkspaceOperation::TrashPath {
                    root: root.clone(),
                    relative_path: PathBuf::from(&target.relative_path),
                };
                waku.run_file_tree_operation(
                    root,
                    operation,
                    FileEditOutcome::Trashed {
                        path: target.relative_path,
                    },
                    cx,
                );
            },
        );
    }

    /// Send one edit to the daemon off the UI thread, then bring the tree,
    /// open editors and tabs in line with what happened.
    fn run_file_tree_operation(
        &mut self,
        root: PathBuf,
        operation: waku_client::WorkspaceOperation,
        outcome: FileEditOutcome,
        cx: &mut Context<Self>,
    ) {
        let workspace = waku_client::WorkspaceClient::new(self.daemon.client());
        cx.spawn(async move |waku, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { workspace.request(operation) })
                .await;
            waku.update(cx, |waku, cx| {
                if let Err(error) = result {
                    waku.show_toast(tr!("file_tree.failed", error = format!("{error:#}")));
                    cx.notify();
                    return;
                }
                if waku.selected_workspace_path() == Some(root.as_path()) {
                    waku.apply_file_edit_outcome(&root, outcome, cx);
                }
                waku.refresh_right_panel_working_tree(cx);
                waku.invalidate_composer_sources(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_file_edit_outcome(&mut self, root: &Path, outcome: FileEditOutcome, cx: &mut Context<Self>) {
        match outcome {
            FileEditOutcome::Created { path, directory } => {
                self.file_tree.cursor = Some(path.clone());
                if directory {
                    self.right_panel_expanded_paths.insert(root.join(&path));
                } else {
                    self.open_right_panel_file(path, cx);
                }
            }
            FileEditOutcome::Renamed { from, to } => {
                let remap = |path: &mut String| {
                    if let Some(moved) = moved_path(path, &from, &to) {
                        *path = moved;
                    }
                };
                for surface in &mut self.right_panel_surfaces {
                    if let RightPanelSurface::File(path) = surface {
                        remap(path);
                    }
                }
                if let Some(selected) = self.right_panel_files_selected_path.as_mut() {
                    remap(selected);
                }
                if let Some(cursor) = self.file_tree.cursor.as_mut() {
                    remap(cursor);
                }
                let editors = std::mem::take(&mut self.right_panel_file_editors);
                self.right_panel_file_editors = editors
                    .into_iter()
                    .map(|(mut path, editor)| {
                        remap(&mut path);
                        (path, editor)
                    })
                    .collect();
                let (old_root, new_root) = (root.join(&from), root.join(&to));
                self.right_panel_expanded_paths = std::mem::take(&mut self.right_panel_expanded_paths)
                    .into_iter()
                    .map(|path| match path.strip_prefix(&old_root) {
                        Ok(rest) if rest.as_os_str().is_empty() => new_root.clone(),
                        Ok(rest) => new_root.join(rest),
                        Err(_) => path,
                    })
                    .collect();
            }
            FileEditOutcome::Trashed { path } => {
                // Tabs on what is gone close; their editors go with them.
                for index in (0..self.right_panel_surfaces.len()).rev() {
                    if matches!(&self.right_panel_surfaces[index], RightPanelSurface::File(open) if is_under(open, &path))
                    {
                        self.close_right_panel_surface(index, cx);
                    }
                }
                self.right_panel_file_editors
                    .retain(|open, _| !is_under(open, &path));
                if self
                    .right_panel_files_selected_path
                    .as_deref()
                    .is_some_and(|selected| is_under(selected, &path))
                {
                    self.right_panel_files_selected_path = None;
                }
                let gone = root.join(&path);
                self.right_panel_expanded_paths
                    .retain(|expanded| !expanded.starts_with(&gone));
                if self
                    .file_tree
                    .cursor
                    .as_deref()
                    .is_some_and(|cursor| is_under(cursor, &path))
                {
                    self.file_tree.cursor = Some(parent_of(&path)).filter(|parent| !parent.is_empty());
                }
            }
        }
    }

    /// Put `@path` into the composer at its caret and give it focus.
    fn mention_in_composer(&mut self, target: &FileTarget, window: &mut Window, cx: &mut Context<Self>) {
        let mention = format!("@{} ", mention_path(&target.relative_path, target.is_dir));
        self.composer.update(cx, |composer, cx| {
            let at = composer.cursor(cx);
            let needs_space = composer.content(cx)[..at]
                .chars()
                .next_back()
                .is_some_and(|before| !before.is_whitespace());
            let text = if needs_space {
                format!(" {mention}")
            } else {
                mention
            };
            composer.replace_range(at..at, &text, cx);
        });
        let focus = self.composer_focus(cx);
        window.focus(&focus, cx);
        cx.notify();
    }
}

enum FileEditOutcome {
    Created { path: String, directory: bool },
    Renamed { from: String, to: String },
    Trashed { path: String },
}

/// A menu entry's handler that updates the app.
fn act(
    weak: &WeakEntity<Waku>,
    action: impl Fn(&mut Waku, &mut Window, &mut Context<Waku>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let weak = weak.clone();
    move |window, cx| {
        let _ = weak.update(cx, |waku, cx| action(waku, window, cx));
    }
}

/// The context menu for whatever the right click landed on.
fn file_tree_menu_items(waku: &Entity<Waku>, cx: &mut App) -> Vec<MenuItem> {
    let (target, remote, root) = {
        let waku = waku.read(cx);
        (
            waku.file_tree.menu_target.clone(),
            waku.daemon.is_remote(),
            waku.selected_workspace_path().map(Path::to_path_buf),
        )
    };
    let Some(root) = root else {
        return Vec::new();
    };
    let weak = waku.downgrade();
    let mut items = Vec::new();
    let refresh = |weak: &WeakEntity<Waku>| {
        MenuItem::new(
            tr!("files.refresh"),
            act(weak, |waku, _, cx| {
                waku.refresh_right_panel_working_tree(cx);
                cx.notify();
            }),
        )
        .icon("icons/rotate-cw.svg")
    };

    let Some(target) = target else {
        // The tree's background: the workspace root.
        items.push(
            MenuItem::new(
                tr!("file_tree.new_file"),
                act(&weak, |waku, window, cx| {
                    waku.begin_file_create(String::new(), false, window, cx);
                }),
            )
            .icon("icons/plus.svg"),
        );
        items.push(
            MenuItem::new(
                tr!("file_tree.new_folder"),
                act(&weak, |waku, window, cx| {
                    waku.begin_file_create(String::new(), true, window, cx);
                }),
            )
            .icon("icons/folder-new.svg"),
        );
        items.push(MenuItem::Separator);
        let reveal_root = root.clone();
        items.push(
            MenuItem::new(tr!("common.reveal_in_finder"), move |_, cx| {
                crate::platform::reveal_in_file_manager(&reveal_root, cx);
            })
            .icon("icons/folder-open.svg")
            .disabled(remote),
        );
        let copy_root = root.display().to_string();
        items.push(
            MenuItem::new(tr!("file_tree.copy_path"), move |_, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_root.clone()));
            })
            .icon("icons/copy.svg"),
        );
        items.push(MenuItem::Separator);
        items.push(refresh(&weak));
        return items;
    };

    let open_target = target.clone();
    items.push(if target.is_dir {
        MenuItem::new(
            if target.expanded {
                tr!("file_tree.collapse")
            } else {
                tr!("file_tree.expand")
            },
            act(&weak, move |waku, _, cx| {
                waku.toggle_file_tree_dir(&open_target, cx);
            }),
        )
        .icon("icons/folder.svg")
    } else {
        MenuItem::new(
            tr!("file_tree.open"),
            act(&weak, move |waku, _, cx| {
                waku.open_right_panel_file(open_target.relative_path.clone(), cx);
            }),
        )
        .icon("icons/file.svg")
    });
    let external = target.absolute_path.clone();
    items.push(
        MenuItem::new(tr!("file_tree.open_external"), move |_, cx| {
            crate::platform::open_with_default_app(&external, cx);
        })
        .icon("icons/external-link.svg")
        .disabled(remote),
    );
    let reveal = target.absolute_path.clone();
    items.push(
        MenuItem::new(tr!("common.reveal_in_finder"), move |_, cx| {
            crate::platform::reveal_in_file_manager(&reveal, cx);
        })
        .icon("icons/folder-open.svg")
        .disabled(remote),
    );
    items.push(MenuItem::Separator);
    let mention = target.clone();
    items.push(MenuItem::new(
        tr!("file_tree.mention"),
        act(&weak, move |waku, window, cx| {
            waku.mention_in_composer(&mention, window, cx);
        }),
    ));
    let absolute = target.absolute_path.display().to_string();
    items.push(
        MenuItem::new(tr!("file_tree.copy_path"), move |_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(absolute.clone()));
        })
        .icon("icons/copy.svg"),
    );
    let relative = mention_path(&target.relative_path, false);
    items.push(MenuItem::new(
        tr!("file_tree.copy_relative_path"),
        move |_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(relative.clone()));
        },
    ));
    items.push(MenuItem::Separator);
    let file_container = target.container();
    items.push(
        MenuItem::new(
            tr!("file_tree.new_file"),
            act(&weak, move |waku, window, cx| {
                waku.begin_file_create(file_container.clone(), false, window, cx);
            }),
        )
        .icon("icons/plus.svg"),
    );
    let folder_container = target.container();
    items.push(
        MenuItem::new(
            tr!("file_tree.new_folder"),
            act(&weak, move |waku, window, cx| {
                waku.begin_file_create(folder_container.clone(), true, window, cx);
            }),
        )
        .icon("icons/folder-new.svg"),
    );
    let rename = target.clone();
    items.push(MenuItem::new(
        tr!("common.rename"),
        act(&weak, move |waku, window, cx| {
            waku.begin_file_rename(rename.clone(), window, cx);
        }),
    ));
    let trash = target;
    items.push(
        MenuItem::new(
            tr!("file_tree.trash"),
            act(&weak, move |waku, _, cx| {
                waku.confirm_file_trash(trash.clone(), cx);
            }),
        )
        .icon("icons/trash.svg"),
    );
    items.push(MenuItem::Separator);
    items.push(refresh(&weak));
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sep(path: &str) -> String {
        path.replace('/', &MAIN_SEPARATOR.to_string())
    }

    #[test]
    fn a_typed_name_stays_inside_its_folder() {
        assert_eq!(typed_path("", "notes.md").as_deref(), Some("notes.md"));
        assert_eq!(typed_path(&sep("src/app"), "mod.rs"), Some(sep("src/app/mod.rs")));
        assert_eq!(typed_path("src", " a/b.rs "), Some(sep("src/a/b.rs")));
        assert_eq!(typed_path("src", "a\\b.rs"), Some(sep("src/a/b.rs")));
        for bad in ["", "  ", "../x", "a/../../x", "/abs", "./x"] {
            assert_eq!(typed_path("src", bad), None, "{bad:?}");
        }
    }

    #[test]
    fn moved_paths_follow_a_rename_but_not_a_lookalike() {
        assert_eq!(moved_path("src", "src", "lib").as_deref(), Some("lib"));
        assert_eq!(moved_path(&sep("src/a.rs"), "src", "lib"), Some(sep("lib/a.rs")));
        assert_eq!(moved_path("src2/a.rs", "src", "lib"), None);
        assert!(is_under(&sep("src/deep/a.rs"), "src"));
        assert!(!is_under("srcs", "src"));
    }

    #[test]
    fn mentions_use_forward_slashes_and_mark_folders() {
        assert_eq!(mention_path("src\\app\\mod.rs", false), "src/app/mod.rs");
        assert_eq!(mention_path("src\\app", true), "src/app/");
        assert_eq!(mention_path("docs/", true), "docs/");
    }

    #[test]
    fn rename_preselects_the_stem() {
        assert_eq!(stem_range("main.rs", false), 0..4);
        assert_eq!(stem_range("archive.tar.gz", false), 0..11);
        assert_eq!(stem_range(".gitignore", false), 0..10);
        assert_eq!(stem_range("Makefile", false), 0..8);
        assert_eq!(stem_range("v1.2", true), 0..4);
    }

    #[test]
    fn new_files_go_inside_a_folder_and_beside_a_file() {
        let target = |relative: &str, is_dir: bool| FileTarget {
            relative_path: sep(relative),
            absolute_path: PathBuf::from(relative),
            is_dir,
            expanded: false,
            depth: 0,
        };
        assert_eq!(target("src/app", true).container(), sep("src/app"));
        assert_eq!(target("src/app/mod.rs", false).container(), sep("src/app"));
        assert_eq!(target("README.md", false).container(), "");
        assert_eq!(target("src/app/mod.rs", false).name(), "mod.rs");
    }
}
