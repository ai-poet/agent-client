//! The Files panel's previews of files that are not text.
//!
//! Fork addition. Opening a picture, a spreadsheet or a Word / PowerPoint
//! file used to land in the text editor, which could show none of them. A
//! file `waku_protocol::workspace::preview_kind` recognises is read on the
//! daemon host (`WorkspaceOperation::PreviewFile`, so a remote workspace
//! previews too) and shown here instead: a picture fitted to the panel or at
//! its own size, a spreadsheet's sheets as a grid, a document's words, or —
//! for a PDF, an archive, audio or video — what the file is and a button to
//! open it with the system's app.
//!
//! A child of `right_panel`; the hook is one early return at the top of
//! `render_right_panel_file`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{ImageFormat, ObjectFit, uniform_list};
use waku_protocol::workspace::{FilePreview, PreviewKind, PreviewSheet};

use super::*;

/// How many files' previews are kept, newest first.
const KEEP: usize = 8;
const ROW_HEIGHT: f32 = 26.0;
const ROW_NUMBER_WIDTH: f32 = 52.0;
/// Paragraphs a document preview draws; the rest is said to be there.
const DOCUMENT_PARAGRAPHS: usize = 3_000;

type PreviewKey = (PathBuf, String);

/// What the panel holds of each previewed file.
#[derive(Default)]
pub(in crate::app) struct FilePreviews {
    entries: HashMap<PreviewKey, Entry>,
    order: VecDeque<PreviewKey>,
    /// The sheet each spreadsheet shows.
    sheet: HashMap<PreviewKey, usize>,
    /// Pictures shown at their own size rather than fitted.
    actual_size: HashSet<PreviewKey>,
}

enum Entry {
    Loading,
    Ready(Rc<Loaded>),
    Failed(String),
}

enum Loaded {
    Image {
        image: Arc<gpui::Image>,
        size: u64,
    },
    Table {
        sheets: Vec<Arc<Sheet>>,
        size: u64,
    },
    Document {
        paragraphs: Vec<String>,
        truncated: bool,
        size: u64,
    },
    Unavailable {
        size: u64,
        reason: Option<String>,
    },
}

impl Loaded {
    fn size(&self) -> u64 {
        match self {
            Self::Image { size, .. }
            | Self::Table { size, .. }
            | Self::Document { size, .. }
            | Self::Unavailable { size, .. } => *size,
        }
    }
}

/// A sheet laid out once: its cells and each column's width.
struct Sheet {
    preview: PreviewSheet,
    widths: Vec<f32>,
    total_width: f32,
}

impl Sheet {
    fn new(preview: PreviewSheet) -> Self {
        let columns = preview.rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<f32> = (0..columns)
            .map(|column| {
                let widest = preview
                    .rows
                    .iter()
                    .take(200)
                    .filter_map(|row| row.get(column))
                    .map(|text| display_width(text.lines().next().unwrap_or_default()))
                    .fold(0.0_f32, f32::max);
                (widest + 18.0).clamp(56.0, 320.0)
            })
            .collect();
        let total_width = ROW_NUMBER_WIDTH + widths.iter().sum::<f32>();
        Self {
            preview,
            widths,
            total_width,
        }
    }
}

/// Rough width of a cell's text at 12px: CJK glyphs run about twice as wide.
fn display_width(text: &str) -> f32 {
    text.chars()
        .map(|glyph| if glyph.is_ascii() { 7.0 } else { 13.0 })
        .sum()
}

/// `A`, `B`, … `Z`, `AA` — a spreadsheet's column name, zero-based.
fn column_name(mut index: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in UNITS {
        value /= 1024.0;
        unit = next;
        if value < 1024.0 {
            break;
        }
    }
    if value < 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

fn image_format(format: &str) -> Option<ImageFormat> {
    Some(match format {
        "png" => ImageFormat::Png,
        "jpeg" | "jpg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        "ico" => ImageFormat::Ico,
        "svg" => ImageFormat::Svg,
        "tif" | "tiff" => ImageFormat::Tiff,
        _ => return None,
    })
}

/// Turn the daemon's answer into what the panel draws. Runs off the UI
/// thread: a picture is decoded from base64 here.
fn load(preview: FilePreview) -> Result<Loaded, String> {
    Ok(match preview {
        FilePreview::Image { format, data, size } => {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|error| error.to_string())?;
            let format = image_format(&format).ok_or_else(|| format!("unknown image format {format}"))?;
            Loaded::Image {
                image: Arc::new(gpui::Image::from_bytes(format, bytes)),
                size,
            }
        }
        FilePreview::Table { sheets, size } => Loaded::Table {
            sheets: sheets.into_iter().map(|sheet| Arc::new(Sheet::new(sheet))).collect(),
            size,
        },
        FilePreview::Document { text, truncated, size } => Loaded::Document {
            paragraphs: text.lines().map(str::to_owned).collect(),
            truncated,
            size,
        },
        FilePreview::Unavailable { size, reason } => Loaded::Unavailable { size, reason },
    })
}

impl Waku {
    /// Hook: `render_right_panel_file` hands a file the editor cannot show
    /// here. The same frame — header, body, the tree beside it — with a
    /// preview for a body.
    pub(super) fn render_file_preview_surface(
        &mut self,
        relative_path: String,
        kind: PreviewKind,
        panel_width: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = Theme::current(cx);
        let file_tree_width = fitted_file_tree_width(panel_width, self.right_panel_file_tree_width);
        let root = self
            .selected_workspace_path()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let key: PreviewKey = (root.clone(), relative_path.clone());
        if !self.file_previews.entries.contains_key(&key) && !root.as_os_str().is_empty() {
            self.load_file_preview(key.clone(), cx);
        }
        let entry = self.file_previews.entries.get(&key);
        let loaded = match entry {
            Some(Entry::Ready(loaded)) => Some(loaded.clone()),
            _ => None,
        };

        let size = loaded.as_ref().map(|loaded| human_size(loaded.size()));
        let can_open = !self.daemon.is_remote();
        let absolute = root.join(&relative_path);
        let is_image = matches!(loaded.as_deref(), Some(Loaded::Image { .. }));
        let actual = self.file_previews.actual_size.contains(&key);

        let mut actions = div().flex().items_center().gap(px(2.0));
        if is_image {
            let toggle_key = key.clone();
            actions = actions.child(self.preview_button(
                "file-preview-fit",
                if actual {
                    "icons/window-restore.svg"
                } else {
                    "icons/window-maximize.svg"
                },
                if actual {
                    tr!("file_preview.fit")
                } else {
                    tr!("file_preview.actual_size")
                },
                move |this, cx| {
                    if !this.file_previews.actual_size.remove(&toggle_key) {
                        this.file_previews.actual_size.insert(toggle_key.clone());
                    }
                    cx.notify();
                },
                cx,
            ));
        }
        let reload_key = key.clone();
        actions = actions.child(self.preview_button(
            "file-preview-reload",
            "icons/rotate-cw.svg",
            tr!("file_preview.reload"),
            move |this, cx| {
                this.file_previews.entries.remove(&reload_key);
                cx.notify();
            },
            cx,
        ));
        if can_open {
            let path = absolute.clone();
            actions = actions.child(self.preview_button(
                "file-preview-open",
                "icons/external-link.svg",
                tr!("file_tree.open_external"),
                move |_, cx| crate::platform::open_with_default_app(&path, cx),
                cx,
            ));
        }

        let body = match entry {
            None | Some(Entry::Loading) => self.preview_message(tr!("file_preview.loading"), None, cx),
            Some(Entry::Failed(error)) => {
                let retry_key = key.clone();
                self.preview_message(
                    tr!("file_preview.failed", error = error.clone()),
                    Some(
                        self.preview_text_button(
                            "file-preview-retry",
                            tr!("file_preview.retry"),
                            move |this, cx| {
                                this.file_previews.entries.remove(&retry_key);
                                cx.notify();
                            },
                            cx,
                        )
                        .into_any_element(),
                    ),
                    cx,
                )
            }
            Some(Entry::Ready(_)) => match loaded.as_deref() {
                Some(Loaded::Image { image, .. }) => self.render_picture_preview(image.clone(), actual, cx),
                Some(Loaded::Table { sheets, .. }) => self.render_table_preview(&key, sheets, cx),
                Some(Loaded::Document {
                    paragraphs,
                    truncated,
                    ..
                }) => self.render_document_preview(paragraphs, *truncated, cx),
                Some(Loaded::Unavailable { reason, .. }) => {
                    self.render_unavailable_preview(kind, reason.clone(), absolute, can_open, cx)
                }
                None => div(),
            },
        };

        let header = div()
            .h(px(42.0))
            .flex_none()
            .px(px(16.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .child(file_icon(file_icon_for_path(&relative_path), 13.0))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(sp(12.5))
                    .text_color(theme.text_secondary)
                    .child(relative_path.clone()),
            )
            .when_some(size, |header, size| {
                header.child(
                    div()
                        .flex_none()
                        .text_size(sp(12.0))
                        .text_color(theme.text_tertiary)
                        .child(size),
                )
            })
            .child(actions);

        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(body),
            )
            .child(
                div()
                    .w(px(file_tree_width))
                    .min_w(px(FILE_TREE_MIN_WIDTH))
                    .h_full()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .relative()
                    .border_l_1()
                    .border_color(theme.border_strong)
                    .child(self.render_right_panel_working_tree(Some(&relative_path), cx))
                    .child(self.render_panel_resize_handle(
                        "right-panel-file-tree-resize-handle",
                        PanelResizeTarget::FileTree,
                        cx,
                    )),
            )
    }

    fn load_file_preview(&mut self, key: PreviewKey, cx: &mut Context<Self>) {
        let previews = &mut self.file_previews;
        previews.entries.insert(key.clone(), Entry::Loading);
        previews.order.retain(|kept| *kept != key);
        previews.order.push_front(key.clone());
        while previews.order.len() > KEEP {
            if let Some(evicted) = previews.order.pop_back() {
                previews.entries.remove(&evicted);
                previews.sheet.remove(&evicted);
            }
        }
        let workspace = waku_client::WorkspaceClient::new(self.daemon.client());
        let (root, relative_path) = key.clone();
        cx.spawn(async move |waku, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    match workspace.request(waku_client::WorkspaceOperation::PreviewFile {
                        root,
                        relative_path: PathBuf::from(relative_path),
                    }) {
                        Ok(waku_client::WorkspaceResult::FilePreview { preview }) => load(preview),
                        Ok(_) => Err("the daemon answered something else".to_owned()),
                        Err(error) => Err(format!("{error:#}")),
                    }
                })
                .await;
            let _ = waku.update(cx, |waku, cx| {
                // Only an entry still waiting takes the answer: a reload or
                // an eviction in the meantime wins.
                if matches!(waku.file_previews.entries.get(&key), Some(Entry::Loading)) {
                    let entry = match loaded {
                        Ok(loaded) => Entry::Ready(Rc::new(loaded)),
                        Err(error) => Entry::Failed(error),
                    };
                    waku.file_previews.entries.insert(key, entry);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn preview_button(
        &self,
        id: &'static str,
        icon_path: &'static str,
        label: String,
        activate: impl Fn(&mut Waku, &mut Context<Waku>) + 'static,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let focus = self.transcript_control_focus(id, cx);
        let activate = Rc::new(activate);
        let on_key = activate.clone();
        div()
            .id(id)
            .track_focus(&focus)
            .tab_index(0)
            .size(px(26.0))
            .rounded(px(7.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .hover(|style| style.bg(theme.overlay))
            .child(icon(icon_path, 12.0, theme.text_tertiary))
            .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| activate(this, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    on_key(this, cx);
                    cx.stop_propagation();
                }
            }))
    }

    fn preview_text_button(
        &self,
        id: &'static str,
        label: String,
        activate: impl Fn(&mut Waku, &mut Context<Waku>) + 'static,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let focus = self.transcript_control_focus(id, cx);
        let activate = Rc::new(activate);
        let on_key = activate.clone();
        div()
            .id(id)
            .track_focus(&focus)
            .tab_index(0)
            .h(px(28.0))
            .px(px(12.0))
            .rounded(px(7.0))
            .flex()
            .items_center()
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.raised)
            .text_size(sp(12.5))
            .text_color(theme.text_secondary)
            .cursor_default()
            .focus_visible(|style| style.border_color(theme.accent))
            .hover(|style| style.bg(theme.overlay))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| activate(this, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    on_key(this, cx);
                    cx.stop_propagation();
                }
            }))
    }

    fn preview_message(&self, message: String, action: Option<AnyElement>, cx: &mut Context<Self>) -> Div {
        let theme = Theme::current(cx);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .p(px(24.0))
            .child(
                div()
                    .max_w(px(420.0))
                    .text_center()
                    .text_size(sp(13.0))
                    .text_color(theme.text_tertiary)
                    .child(message),
            )
            .children(action)
    }

    fn render_picture_preview(&self, image: Arc<gpui::Image>, actual: bool, cx: &mut Context<Self>) -> Div {
        let theme = Theme::current(cx);
        let picture = img(image).id("file-preview-image");
        let canvas = if actual {
            div()
                .id("file-preview-image-scroll")
                .size_full()
                .overflow_scroll()
                .p(px(16.0))
                .child(picture)
                .into_any_element()
        } else {
            div()
                .size_full()
                .p(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    picture
                        .max_w_full()
                        .max_h_full()
                        .object_fit(ObjectFit::ScaleDown),
                )
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .bg(theme.inset)
            .child(canvas)
    }

    fn render_table_preview(&self, key: &PreviewKey, sheets: &[Arc<Sheet>], cx: &mut Context<Self>) -> Div {
        let theme = Theme::current(cx);
        if sheets.is_empty() {
            return self.preview_message(tr!("file_preview.empty_sheet"), None, cx);
        }
        let selected = self
            .file_previews
            .sheet
            .get(key)
            .copied()
            .unwrap_or(0)
            .min(sheets.len() - 1);
        let sheet = sheets[selected].clone();

        let tabs = (sheets.len() > 1).then(|| {
            div()
                .id("file-preview-sheets")
                .flex_none()
                .h(px(34.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .overflow_x_scroll()
                .border_b_1()
                .border_color(theme.border)
                .children(sheets.iter().enumerate().map(|(index, tab)| {
                    let key = key.clone();
                    let chosen = index == selected;
                    let focus = self.transcript_control_focus(
                        SharedString::from(format!("file-preview-sheet-{index}")),
                        cx,
                    );
                    let on_key_key = key.clone();
                    div()
                        .id(("file-preview-sheet", index))
                        .track_focus(&focus)
                        .tab_index(0)
                        .h(px(24.0))
                        .px(px(10.0))
                        .rounded(px(6.0))
                        .flex()
                        .items_center()
                        .flex_none()
                        .text_size(sp(12.0))
                        .cursor_default()
                        .focus_visible(|style| style.border_1().border_color(theme.accent))
                        .when(chosen, |tab| {
                            tab.bg(theme.overlay_strong)
                                .text_color(theme.text)
                                .font_weight(FontWeight::MEDIUM)
                        })
                        .when(!chosen, |tab| {
                            tab.text_color(theme.text_tertiary)
                                .hover(|style| style.bg(theme.overlay))
                        })
                        .child(tab.preview.name.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.file_previews.sheet.insert(key.clone(), index);
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.file_previews.sheet.insert(on_key_key.clone(), index);
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }))
                }))
        });

        let preview = &sheet.preview;
        let shown_rows = preview.rows.len();
        let summary = if shown_rows < preview.total_rows || sheet.widths.len() < preview.total_columns {
            tr!(
                "file_preview.rows_shown",
                shown = shown_rows,
                rows = preview.total_rows,
                columns = preview.total_columns
            )
        } else {
            tr!(
                "file_preview.rows_all",
                rows = preview.total_rows,
                columns = preview.total_columns
            )
        };

        let header_sheet = sheet.clone();
        let column_header = div()
            .flex()
            .flex_none()
            .h(px(ROW_HEIGHT))
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border_strong)
            .child(
                div()
                    .w(px(ROW_NUMBER_WIDTH))
                    .flex_none()
                    .border_r_1()
                    .border_color(theme.border),
            )
            .children(header_sheet.widths.iter().enumerate().map(|(index, width)| {
                div()
                    .w(px(*width))
                    .flex_none()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_r_1()
                    .border_color(theme.border)
                    .text_size(sp(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text_tertiary)
                    .child(column_name(header_sheet.preview.first_column + index))
            }));

        let rows_sheet = sheet.clone();
        let rows = uniform_list(
            "file-preview-rows",
            shown_rows,
            move |range, _, cx| {
                let theme = Theme::current(cx);
                range
                    .map(|row_index| {
                        let row = &rows_sheet.preview.rows[row_index];
                        div()
                            .flex()
                            .h(px(ROW_HEIGHT))
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .w(px(ROW_NUMBER_WIDTH))
                                    .flex_none()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_end()
                                    .pr(px(8.0))
                                    .bg(theme.surface)
                                    .border_r_1()
                                    .border_color(theme.border)
                                    .text_size(sp(11.5))
                                    .text_color(theme.text_tertiary)
                                    .child((rows_sheet.preview.first_row + row_index + 1).to_string()),
                            )
                            .children(rows_sheet.widths.iter().enumerate().map(|(column, width)| {
                                let text = row.get(column).map(String::as_str).unwrap_or_default();
                                let numeric = !text.is_empty() && text.trim().parse::<f64>().is_ok();
                                div()
                                    .w(px(*width))
                                    .flex_none()
                                    .h_full()
                                    .px(px(8.0))
                                    .flex()
                                    .items_center()
                                    .when(numeric, |cell| cell.justify_end())
                                    .border_r_1()
                                    .border_color(theme.border)
                                    .text_size(sp(12.0))
                                    .text_color(theme.text_secondary)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .child(text.replace('\n', " ⏎ ")),
                                    )
                            }))
                    })
                    .collect()
            },
        )
        .flex_1()
        .w(px(sheet.total_width));

        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .children(tabs)
            .child(
                div()
                    .flex_none()
                    .px(px(16.0))
                    .py(px(6.0))
                    .text_size(sp(12.0))
                    .text_color(theme.text_tertiary)
                    .child(summary),
            )
            .child(
                div()
                    .id("file-preview-grid")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .child(
                        div()
                            .w(px(sheet.total_width))
                            .h_full()
                            .flex()
                            .flex_col()
                            .border_t_1()
                            .border_color(theme.border)
                            .child(column_header)
                            .child(rows),
                    ),
            )
    }

    fn render_document_preview(&self, paragraphs: &[String], truncated: bool, cx: &mut Context<Self>) -> Div {
        let theme = Theme::current(cx);
        let cut = truncated || paragraphs.len() > DOCUMENT_PARAGRAPHS;
        div().flex_1().min_h_0().min_w_0().child(
            div()
                .id("file-preview-document")
                .size_full()
                .overflow_y_scroll()
                .px(px(24.0))
                .py(px(18.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .children(paragraphs.iter().take(DOCUMENT_PARAGRAPHS).map(|paragraph| {
                    div()
                        .min_h(px(8.0))
                        .text_size(sp(13.0))
                        .line_height(sp(20.0))
                        .text_color(theme.text_secondary)
                        .child(paragraph.clone())
                }))
                .when(cut, |document| {
                    document.child(
                        div()
                            .pt(px(10.0))
                            .text_size(sp(12.0))
                            .text_color(theme.text_tertiary)
                            .child(tr!("file_preview.document_truncated")),
                    )
                }),
        )
    }

    fn render_unavailable_preview(
        &self,
        kind: PreviewKind,
        reason: Option<String>,
        path: PathBuf,
        can_open: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let message = match (kind, reason) {
            (_, Some(reason)) => tr!("file_preview.cannot_read", reason = reason),
            _ => tr!("file_preview.unavailable"),
        };
        let action = can_open.then(|| {
            self.preview_text_button(
                "file-preview-open-external",
                tr!("file_tree.open_external"),
                move |_, cx| crate::platform::open_with_default_app(&path, cx),
                cx,
            )
            .into_any_element()
        });
        self.preview_message(message, action, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_named_as_spreadsheets_name_them() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(27), "AB");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(25 * 1024 * 1024), "25 MB");
    }

    #[test]
    fn column_widths_follow_their_widest_cell() {
        let sheet = Sheet::new(PreviewSheet {
            name: "Sheet1".to_owned(),
            first_row: 0,
            first_column: 0,
            rows: vec![
                vec!["id".to_owned(), "a much longer description".to_owned()],
                vec!["1".to_owned(), "短".to_owned()],
            ],
            total_rows: 2,
            total_columns: 2,
        });
        assert_eq!(sheet.widths.len(), 2);
        assert_eq!(sheet.widths[0], 56.0);
        assert!(sheet.widths[1] > 150.0);
        assert_eq!(sheet.total_width, ROW_NUMBER_WIDTH + sheet.widths.iter().sum::<f32>());
    }

    #[test]
    fn previews_decode_pictures_and_split_documents() {
        assert!(matches!(
            load(FilePreview::Image {
                format: "png".to_owned(),
                data: "iVBORw==".to_owned(),
                size: 4,
            }),
            Ok(Loaded::Image { size: 4, .. })
        ));
        assert!(load(FilePreview::Image {
            format: "heic".to_owned(),
            data: String::new(),
            size: 0,
        })
        .is_err());
        let Ok(Loaded::Document { paragraphs, .. }) = load(FilePreview::Document {
            text: "one\ntwo".to_owned(),
            truncated: false,
            size: 7,
        }) else {
            panic!("a document");
        };
        assert_eq!(paragraphs, ["one", "two"]);
    }
}
