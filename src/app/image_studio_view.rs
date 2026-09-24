//! What the image studio looks like: a gallery of what was drawn, newest
//! first, above a field shaped like the task composer.
//!
//! Fork addition; the state and the requests are in `image_studio.rs`.

use sub2api::images::{self, ImageErrorKind};

use super::image_studio::{JobStatus, StudioJob, Thumb};
use super::providers_page::card_button;
use super::*;

const CARD_WIDTH: f32 = 204.0;
const GALLERY_MAX_WIDTH: f32 = 984.0;
const REFERENCE_SIZE: f32 = 56.0;

pub(super) fn format_usd(value: f64) -> String {
    if value < 0.1 {
        format!("${value:.3}")
    } else {
        format!("${value:.2}")
    }
}

fn size_label(size: &str) -> String {
    if size == "auto" {
        tr!("image_studio.size_auto")
    } else {
        size.replace('x', "×")
    }
}

fn quality_label(quality: Option<&str>) -> String {
    match quality {
        Some("low") => tr!("image_studio.quality_low"),
        Some("medium") => tr!("image_studio.quality_medium"),
        Some("high") => tr!("image_studio.quality_high"),
        _ => tr!("image_studio.quality_auto"),
    }
}

fn error_text(kind: ImageErrorKind) -> String {
    match kind {
        ImageErrorKind::NoImagePermission => tr!("image_studio.error_no_permission"),
        ImageErrorKind::WrongPlatform => tr!("image_studio.error_wrong_platform"),
        ImageErrorKind::ModelUnavailable => tr!("image_studio.error_model_unavailable"),
        ImageErrorKind::ContentPolicy => tr!("image_studio.error_content_policy"),
        ImageErrorKind::OwnBalance => tr!("image_studio.error_balance"),
        ImageErrorKind::UpstreamBalance => tr!("image_studio.error_upstream_balance"),
        ImageErrorKind::Busy => tr!("image_studio.error_busy"),
        ImageErrorKind::Timeout => tr!("image_studio.error_timeout"),
        ImageErrorKind::TaskLost => tr!("image_studio.error_task_lost"),
        ImageErrorKind::Interrupted => tr!("image_studio.error_interrupted"),
        ImageErrorKind::Unauthorized => tr!("image_studio.error_unauthorized"),
        ImageErrorKind::Network => tr!("image_studio.error_network"),
        ImageErrorKind::AsyncUnavailable | ImageErrorKind::BadRequest | ImageErrorKind::Other => {
            tr!("image_studio.error_other")
        }
    }
}

fn elapsed(seconds: i64) -> String {
    let seconds = seconds.max(0);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A control-row chip that opens a menu.
fn chip(theme: Theme, id: &'static str, label: String) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .h(px(26.0))
        .px(px(8.0))
        .rounded(px(7.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.0))
        .cursor_default()
        .text_size(sp(12.0))
        .text_color(theme.text_secondary)
        .hover(|style| style.bg(theme.overlay))
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .child(label)
        .child(icon("icons/chevron-down.svg", 10.0, theme.text_tertiary))
}

impl Waku {
    pub(super) fn image_studio_title(&self) -> String {
        tr!("image_studio.title")
    }

    /// Everything under the header while the studio is open.
    pub(super) fn render_image_studio(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::current(cx);
        let drop_wash = theme.surface.blend(theme.overlay);
        div()
            .id("image-studio")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .flex()
            .flex_col()
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_wash))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_image_studio_references(paths.paths().to_vec(), cx);
            }))
            .child(self.render_image_studio_toolbar(theme, cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .child(
                        div()
                            .id("image-studio-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.image_studio.scroll)
                            .px(px(20.0))
                            .pb(px(20.0))
                            .child(self.render_image_studio_gallery(theme, cx)),
                    )
                    .child(scrollbar::vertical(
                        &self.image_studio.scroll,
                        &self.image_studio.scrollbar,
                    )),
            )
            .when(self.cloud_account.credentials.is_some(), |element| {
                element.child(self.render_image_studio_composer(theme, cx))
            })
            .into_any_element()
    }

    fn render_image_studio_toolbar(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let pictures: usize = self
            .image_studio
            .store
            .jobs
            .iter()
            .map(|job| match &job.status {
                JobStatus::Done { outputs, .. } => outputs.len(),
                _ => 0,
            })
            .sum();
        div().flex_none().px(px(20.0)).pb(px(8.0)).child(
            div()
                .w_full()
                .max_w(px(GALLERY_MAX_WIDTH))
                .mx_auto()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_size(sp(12.0))
                        .text_color(theme.text_tertiary)
                        .child(if pictures == 0 {
                            tr!("image_studio.subtitle")
                        } else {
                            tr!("image_studio.picture_count", count = pictures)
                        }),
                )
                .child(div().flex_1())
                .children(self.render_cloud_balance_badge(cx))
                .child(card_button(
                    theme,
                    SharedString::from("image-studio-open-folder"),
                    tr!("image_studio.open_folder"),
                    false,
                    false,
                    cx,
                    |this, _, cx| this.open_image_folder(cx),
                )),
        )
    }

    fn render_image_studio_gallery(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let content = div().w_full().max_w(px(GALLERY_MAX_WIDTH)).mx_auto();
        if self.cloud_account.credentials.is_none() {
            return content
                .child(self.render_image_studio_empty(
                    theme,
                    tr!("image_studio.signed_out_title"),
                    tr!("image_studio.signed_out_body"),
                    Some(card_button(
                        theme,
                        SharedString::from("image-studio-sign-in"),
                        tr!("image_studio.sign_in"),
                        true,
                        false,
                        cx,
                        |this, _, cx| this.open_settings_page(SettingsPage::CloudAccount, cx),
                    )),
                ))
                .into_any_element();
        }
        if self.image_studio.loaded && self.image_studio.store.jobs.is_empty() {
            return content
                .child(self.render_image_studio_empty(
                    theme,
                    tr!("image_studio.empty_title"),
                    tr!("image_studio.empty_body"),
                    None,
                ))
                .into_any_element();
        }
        let now = Utc::now().timestamp();
        let mut cards = div().w_full().flex().flex_wrap().gap(px(14.0)).pt(px(4.0));
        for job in &self.image_studio.store.jobs {
            match &job.status {
                JobStatus::Done { outputs, .. } => {
                    for (index, path) in outputs.iter().enumerate() {
                        cards =
                            cards.child(self.render_studio_picture(job, index, path, theme, cx));
                    }
                }
                JobStatus::Running {
                    task_id,
                    started_at,
                } => {
                    let queued = !self.image_studio.running.contains(&job.id);
                    cards = cards.child(self.render_studio_running(
                        job,
                        queued,
                        task_id.is_some(),
                        now - started_at,
                        theme,
                        cx,
                    ));
                }
                JobStatus::Failed { error, .. } => {
                    cards = cards.child(self.render_studio_failed(job, error, theme, cx));
                }
            }
        }
        content.child(cards).into_any_element()
    }

    fn render_image_studio_empty(
        &self,
        theme: Theme,
        title: String,
        body: String,
        action: Option<Stateful<Div>>,
    ) -> impl IntoElement {
        div()
            .w_full()
            .pt(px(96.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .size(px(44.0))
                    .rounded(px(12.0))
                    .bg(theme.raised)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("icons/image.svg", 22.0, theme.text_tertiary)),
            )
            .child(
                div()
                    .text_size(sp(15.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(title),
            )
            .child(
                div()
                    .max_w(px(420.0))
                    .text_center()
                    .text_size(sp(12.5))
                    .text_color(theme.text_secondary)
                    .child(body),
            )
            .children(action)
    }

    /// The prompt and what it was drawn with, under a card's picture.
    fn render_studio_caption(
        &self,
        job: &StudioJob,
        menu: Option<AnyElement>,
        theme: Theme,
    ) -> Div {
        let mut meta = vec![
            job.spec.model().to_owned(),
            size_label(job.spec.size.as_deref().unwrap_or("auto")),
        ];
        if job.spec.is_edit() {
            meta.push(tr!("image_studio.mode_edit"));
        }
        if let Some(estimate) = job.estimate_usd {
            meta.push(format_usd(estimate));
        }
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sp(12.5))
                            .text_color(theme.text)
                            .child(job.spec.prompt.clone()),
                    )
                    .children(menu),
            )
            .child(
                div()
                    .truncate()
                    .text_size(sp(11.5))
                    .text_color(theme.text_tertiary)
                    .child(meta.join(" · ")),
            )
    }

    fn render_studio_picture(
        &self,
        job: &StudioJob,
        index: usize,
        path: &Path,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let job_id = job.id;
        let key = format!("{job_id}-{index}");
        let thumb = self.studio_thumb(path, cx);
        let missing = matches!(
            self.image_studio.thumbs.borrow().get(path),
            Some(Thumb::Missing)
        );
        let picture = div()
            .id(SharedString::from(format!("studio-picture-{key}")))
            .w_full()
            .h(px(CARD_WIDTH))
            .rounded(px(10.0))
            .overflow_hidden()
            .border_1()
            .border_color(theme.border)
            .bg(theme.inset)
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .map(|element| match thumb {
                Some(image) => element.child(img(image).size_full().object_fit(ObjectFit::Cover)),
                None if missing => element.child(
                    div()
                        .px(px(12.0))
                        .text_center()
                        .text_size(sp(12.0))
                        .text_color(theme.text_tertiary)
                        .child(tr!("image_studio.file_missing")),
                ),
                None => element.child(icon("icons/image.svg", 20.0, theme.text_ghost)),
            })
            .on_click(cx.listener({
                let path = path.to_owned();
                move |this, _, window, cx| this.preview_studio_image(path.clone(), window, cx)
            }));

        let weak = cx.entity().downgrade();
        let handle = self.menu_handle(format!("studio-menu-{key}"), cx);
        let trigger = icon_button(
            SharedString::from(format!("studio-menu-trigger-{key}")),
            "icons/ellipsis.svg",
            theme,
        );
        let path = path.to_owned();
        let menu = dropdown_menu(
            trigger,
            SharedString::from(format!("studio-menu-{key}")),
            &handle,
            MenuAlign::BelowRight,
            move |_| {
                let item = |label: String, action: fn(&mut Waku, Uuid, &Path, &mut Window, &mut Context<Waku>)| {
                    let weak = weak.clone();
                    let path = path.clone();
                    MenuItem::new(label, move |window, cx| {
                        let _ = weak.update(cx, |this, cx| action(this, job_id, &path, window, cx));
                    })
                };
                vec![
                    item(
                        tr!("image_studio.use_as_reference"),
                        |this, _, path, _, cx| this.use_image_as_reference(path.to_owned(), cx),
                    ),
                    item(tr!("image_studio.edit_again"), |this, id, _, window, cx| {
                        this.edit_image_job(id, window, cx)
                    }),
                    item(tr!("image_studio.rerun"), |this, id, _, _, cx| {
                        this.rerun_image_job(id, cx)
                    }),
                    item(
                        tr!("image_studio.send_to_task"),
                        |this, _, path, window, cx| {
                            this.send_image_to_chat(path.to_owned(), window, cx)
                        },
                    ),
                    item(tr!("image_studio.copy_prompt"), |this, id, _, _, cx| {
                        this.copy_image_prompt(id, cx)
                    }),
                    item(tr!("common.reveal_in_finder"), |_, _, path, _, cx| {
                        crate::platform::reveal_in_file_manager(path, cx)
                    }),
                    MenuItem::Separator,
                    item(tr!("image_studio.remove"), |this, id, _, _, cx| {
                        this.remove_image_job(id, cx)
                    }),
                ]
            },
        );

        div()
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(picture)
            .child(self.render_studio_caption(job, Some(menu), theme))
            .into_any_element()
    }

    fn render_studio_running(
        &self,
        job: &StudioJob,
        queued: bool,
        as_task: bool,
        seconds: i64,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let job_id = job.id;
        let detail = if queued {
            tr!("image_studio.queued")
        } else if as_task {
            tr!("image_studio.running_task")
        } else if !job.group_name.is_empty() {
            job.group_name.clone()
        } else {
            tr!("image_studio.running_direct")
        };
        let count = job.spec.count();
        let status = if queued {
            tr!("image_studio.waiting")
        } else {
            tr!("image_studio.drawing", time = elapsed(seconds))
        };
        let cancel = icon_button(
            SharedString::from(format!("studio-cancel-{job_id}")),
            "icons/x.svg",
            theme,
        )
        .tooltip(Tooltip::text(tr!("image_studio.cancel")))
        .on_click(cx.listener(move |this, _, _, cx| this.remove_image_job(job_id, cx)));
        div()
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .w_full()
                    .h(px(CARD_WIDTH))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(theme.raised)
                    .relative()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .px(px(14.0))
                    .child(motion::spin_slow(icon(
                        "icons/loader-circle.svg",
                        18.0,
                        theme.text_secondary,
                    )))
                    .child(
                        div()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(status),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(sp(11.5))
                            .text_color(theme.text_tertiary)
                            .child(detail),
                    )
                    .when(count > 1, |element| {
                        element.child(
                            div()
                                .text_size(sp(11.5))
                                .text_color(theme.text_tertiary)
                                .child(tr!("image_studio.count_label", count = count)),
                        )
                    })
                    .child(div().absolute().top(px(6.0)).right(px(6.0)).child(cancel)),
            )
            .child(self.render_studio_caption(job, None, theme))
            .into_any_element()
    }

    fn render_studio_failed(
        &self,
        job: &StudioJob,
        error: &images::ImageError,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let job_id = job.id;
        let mut actions = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(6.0))
            .child(card_button(
                theme,
                SharedString::from(format!("studio-retry-{job_id}")),
                tr!("image_studio.retry"),
                true,
                false,
                cx,
                move |this, _, cx| this.retry_image_job(job_id, cx),
            ));
        if error.kind == ImageErrorKind::OwnBalance {
            actions = actions.child(card_button(
                theme,
                SharedString::from(format!("studio-top-up-{job_id}")),
                tr!("image_studio.top_up"),
                false,
                false,
                cx,
                |this, _, cx| this.open_cloud_pay_modal(cx),
            ));
        }
        actions = actions.child(card_button(
            theme,
            SharedString::from(format!("studio-remove-{job_id}")),
            tr!("image_studio.remove"),
            false,
            false,
            cx,
            move |this, _, cx| this.remove_image_job(job_id, cx),
        ));
        let detail: String = error.message.chars().take(160).collect();
        div()
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .id(SharedString::from(format!("studio-failed-{job_id}")))
                    .w_full()
                    .h(px(CARD_WIDTH))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(theme.danger.opacity(0.35))
                    .bg(theme.danger_soft)
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .child(icon("icons/alert.svg", 18.0, theme.danger))
                    .child(
                        div()
                            .text_center()
                            .text_size(sp(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.danger)
                            .child(error_text(error.kind)),
                    )
                    .when(
                        !detail.is_empty() && !matches!(error.kind, ImageErrorKind::Interrupted),
                        |element| {
                            element.child(
                                div()
                                    .max_h(px(48.0))
                                    .overflow_hidden()
                                    .text_center()
                                    .text_size(sp(11.0))
                                    .text_color(theme.text_tertiary)
                                    .child(detail),
                            )
                        },
                    )
                    .child(actions),
            )
            .child(self.render_studio_caption(job, None, theme))
            .into_any_element()
    }

    fn render_image_studio_composer(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let model = self.image_studio_model();
        let is_edit = !self.image_studio.references.is_empty();

        let mut references = div().flex().flex_wrap().gap(px(8.0)).px(px(12.0));
        for (index, reference) in self.image_studio.references.iter().enumerate() {
            let preview = reference.image.clone();
            let name = reference
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            references = references.child(
                div()
                    .id(SharedString::from(format!("studio-reference-{index}")))
                    .size(px(REFERENCE_SIZE))
                    .rounded(px(8.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.inset)
                    .relative()
                    .cursor_default()
                    .children(
                        preview
                            .clone()
                            .map(|image| img(image).size_full().object_fit(ObjectFit::Cover)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(image) = preview.clone() {
                            this.open_image_preview(
                                image,
                                SharedString::from(name.clone()),
                                window,
                                cx,
                            );
                        }
                    }))
                    .child(
                        div().absolute().top(px(2.0)).right(px(2.0)).child(
                            div()
                                .id(SharedString::from(format!(
                                    "studio-reference-remove-{index}"
                                )))
                                .size(px(18.0))
                                .rounded(px(9.0))
                                .bg(theme.inverse.opacity(0.7))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon("icons/x.svg", 10.0, theme.on_inverse))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.remove_image_studio_reference(index, cx);
                                })),
                        ),
                    ),
            );
        }

        let weak = cx.entity().downgrade();

        // Model.
        let models = self.image_studio_models();
        let model_handle = self.menu_handle("image-studio-model-menu", cx);
        let model_menu = dropdown_menu(
            chip(theme, "image-studio-model", model.clone()),
            "image-studio-model-menu",
            &model_handle,
            MenuAlign::AboveLeft,
            {
                let weak = weak.clone();
                let current = model.clone();
                move |_| {
                    models
                        .iter()
                        .map(|name| {
                            let weak = weak.clone();
                            let selected = *name == current;
                            let name = name.clone();
                            MenuItem::new(name.clone(), move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_image_studio_model(name.clone(), cx)
                                });
                            })
                            .selected(selected)
                        })
                        .collect()
                }
            },
        );

        // Group.
        let now = Utc::now().timestamp();
        let candidates = self.image_studio_candidates(&model);
        let picked = self.image_studio.store.prefs.group;
        let group_label = match picked {
            None => tr!("image_studio.group_auto"),
            Some(id) => candidates
                .iter()
                .find(|group| group.group_id == id)
                .filter(|group| !group.name.is_empty())
                .map(|group| group.name.clone())
                .unwrap_or_else(|| tr!("image_studio.group_number", id = id)),
        };
        let group_entries: Vec<(i64, String)> = candidates
            .iter()
            .map(|group| {
                let mut label = if group.name.is_empty() {
                    tr!("image_studio.group_number", id = group.group_id)
                } else {
                    group.name.clone()
                };
                if group.rate_multiplier > 0.0 {
                    label.push_str(&format!(" · ×{}", group.rate_multiplier));
                }
                if self.image_studio.store.denied(&model, group.group_id, now) {
                    label.push_str(&format!(" · {}", tr!("image_studio.group_denied")));
                }
                (group.group_id, label)
            })
            .collect();
        let group_handle = self.menu_handle("image-studio-group-menu", cx);
        let group_menu = dropdown_menu(
            chip(theme, "image-studio-group", group_label),
            "image-studio-group-menu",
            &group_handle,
            MenuAlign::AboveLeft,
            {
                let weak = weak.clone();
                move |_| {
                    let auto = {
                        let weak = weak.clone();
                        MenuItem::new(tr!("image_studio.group_auto"), move |_, cx| {
                            let _ =
                                weak.update(cx, |this, cx| this.set_image_studio_group(None, cx));
                        })
                        .selected(picked.is_none())
                    };
                    std::iter::once(auto)
                        .chain(group_entries.iter().map(|(id, label)| {
                            let weak = weak.clone();
                            let id = *id;
                            MenuItem::new(label.clone(), move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_image_studio_group(Some(id), cx)
                                });
                            })
                            .selected(picked == Some(id))
                        }))
                        .collect()
                }
            },
        );

        // Size.
        let size = self.image_studio_size(&model);
        let sizes: Vec<(String, String)> = images::size_options(&model)
            .iter()
            .map(|option| {
                let mut label = format!(
                    "{} · {}",
                    size_label(option),
                    images::tier_for_size(Some(*option)).label()
                );
                if let Some(price) = self.image_studio_unit_price(&model, option) {
                    label.push_str(&format!(" · {}", format_usd(price)));
                }
                ((*option).to_owned(), label)
            })
            .collect();
        let size_handle = self.menu_handle("image-studio-size-menu", cx);
        let size_menu = dropdown_menu(
            chip(theme, "image-studio-size", size_label(&size)),
            "image-studio-size-menu",
            &size_handle,
            MenuAlign::AboveLeft,
            {
                let weak = weak.clone();
                let current = size.clone();
                move |_| {
                    sizes
                        .iter()
                        .map(|(value, label)| {
                            let weak = weak.clone();
                            let value = value.clone();
                            let selected = value == current;
                            MenuItem::new(label.clone(), move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_image_studio_size(value.clone(), cx)
                                });
                            })
                            .selected(selected)
                        })
                        .collect()
                }
            },
        );

        // Quality: not for Grok.
        let quality_menu = images::supports_quality(&model).then(|| {
            let quality = self.image_studio_quality(&model);
            let handle = self.menu_handle("image-studio-quality-menu", cx);
            let weak = weak.clone();
            let current = quality.clone();
            dropdown_menu(
                chip(
                    theme,
                    "image-studio-quality",
                    quality_label(quality.as_deref()),
                ),
                "image-studio-quality-menu",
                &handle,
                MenuAlign::AboveLeft,
                move |_| {
                    images::QUALITY_OPTIONS
                        .iter()
                        .map(|option| {
                            let weak = weak.clone();
                            let value = (*option != "auto").then(|| (*option).to_owned());
                            let selected = current.as_deref().unwrap_or("auto") == *option;
                            MenuItem::new(quality_label(Some(*option)), move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_image_studio_quality(value.clone(), cx)
                                });
                            })
                            .selected(selected)
                        })
                        .collect()
                },
            )
        });

        // Count.
        let count = self.image_studio_count();
        let count_handle = self.menu_handle("image-studio-count-menu", cx);
        let count_menu = dropdown_menu(
            chip(
                theme,
                "image-studio-count",
                tr!("image_studio.count_label", count = count),
            ),
            "image-studio-count-menu",
            &count_handle,
            MenuAlign::AboveLeft,
            {
                let weak = weak.clone();
                move |_| {
                    (1..=images::MAX_COUNT)
                        .map(|option| {
                            let weak = weak.clone();
                            MenuItem::new(
                                tr!("image_studio.count_label", count = option),
                                move |_, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.set_image_studio_count(option, cx)
                                    });
                                },
                            )
                            .selected(option == count)
                        })
                        .collect()
                }
            },
        );

        let add_images = icon_button("image-studio-add-images", "icons/paperclip.svg", theme)
            .tooltip(Tooltip::text(tr!("image_studio.add_images")))
            .on_click(cx.listener(|this, _, _, cx| this.pick_image_studio_references(cx)));

        let can_submit = self.image_studio_can_submit(cx);
        let estimate = self.image_studio_estimate();
        let generate = card_button(
            theme,
            SharedString::from("image-studio-generate"),
            tr!("image_studio.generate"),
            true,
            !can_submit,
            cx,
            |this, _, cx| this.submit_image_studio(cx),
        );

        div().flex_none().px(px(20.0)).pb(px(16.0)).child(
            div()
                .w_full()
                .max_w(px(CONTENT_MAX_WIDTH))
                .mx_auto()
                .rounded(px(13.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.composer)
                .py(px(10.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .when(is_edit, |element| element.child(references))
                .children(self.image_studio.input.clone().map(|input| {
                    div()
                        .px(px(14.0))
                        .min_h(px(40.0))
                        .text_size(sp(14.0))
                        .child(input)
                }))
                .child(
                    div()
                        .px(px(8.0))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(2.0))
                        .child(add_images)
                        .child(model_menu)
                        .child(group_menu)
                        .child(size_menu)
                        .children(quality_menu)
                        .child(count_menu)
                        .child(div().flex_1())
                        .child(
                            div()
                                .px(px(6.0))
                                .text_size(sp(11.5))
                                .text_color(theme.text_tertiary)
                                .child(if is_edit {
                                    tr!("image_studio.mode_edit")
                                } else {
                                    tr!("image_studio.mode_generate")
                                }),
                        )
                        .children(estimate.map(|estimate| {
                            div()
                                .px(px(4.0))
                                .text_size(sp(11.5))
                                .text_color(theme.text_secondary)
                                .child(tr!("image_studio.estimate", price = format_usd(estimate)))
                        }))
                        .child(div().pl(px(4.0)).child(generate)),
                ),
        )
    }
}
