//! Service announcements from the managed gateway, natively.
//!
//! Fork addition, a port of the web console's announcement bell: a bell in
//! the window header with an unread dot, and a modal listing the account's
//! announcements with a markdown detail view. Data comes from
//! `/announcements`; opening a detail marks it read on the server.

use std::time::{Duration, Instant};

use gpui::ElementId;
use sub2api::client::Announcement;

use super::*;

/// How long a fetched announcement list stays fresh. The periodic account
/// refresh calls in through this guard, so the endpoint sees at most one
/// request per interval; opening the modal bypasses it.
const ANNOUNCEMENTS_TTL: Duration = Duration::from_secs(10 * 60);

/// Bell and modal state.
#[derive(Default)]
pub(super) struct AnnouncementsState {
    /// The modal is open.
    pub open: bool,
    /// The announcement whose detail view is open, if any.
    pub detail: Option<i64>,
    pub items: Vec<Announcement>,
    pub loading: bool,
    pub error: Option<String>,
    /// Last successful fetch, for the TTL guard.
    pub fetched_at: Option<Instant>,
}

impl Waku {
    pub(super) fn announcement_unread_count(&self) -> usize {
        self.cloud_announcements
            .items
            .iter()
            .filter(|item| item.is_unread())
            .count()
    }

    /// Fetch the announcement list. `force` bypasses the TTL (modal open);
    /// the periodic account refresh passes `false`.
    pub(super) fn refresh_cloud_announcements(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        if self.cloud_announcements.loading {
            return;
        }
        if !force
            && self
                .cloud_announcements
                .fetched_at
                .is_some_and(|at| at.elapsed() < ANNOUNCEMENTS_TTL)
        {
            return;
        }
        self.cloud_announcements.loading = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    sub2api::refresh_if_needed(&mut credentials)?;
                    let items = sub2api::Client::new(credentials.endpoint.clone())
                        .announcements(&credentials.access_token)?;
                    anyhow::Ok((credentials, items))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cloud_announcements.loading = false;
                match fetched {
                    Ok((credentials, items)) => {
                        this.adopt_cloud_tokens(credentials);
                        this.cloud_announcements.items = items;
                        this.cloud_announcements.error = None;
                        this.cloud_announcements.fetched_at = Some(Instant::now());
                    }
                    Err(error) => {
                        this.cloud_announcements.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_announcements_modal(&mut self, cx: &mut Context<Self>) {
        self.cloud_announcements.open = true;
        self.cloud_announcements.detail = None;
        self.refresh_cloud_announcements(true, cx);
        cx.notify();
    }

    fn close_announcements_modal(&mut self, cx: &mut Context<Self>) {
        self.cloud_announcements.open = false;
        self.cloud_announcements.detail = None;
        cx.notify();
    }

    /// Mark announcements read: optimistically locally, best-effort on the
    /// server. A failed call resurfaces the item as unread on the next fetch,
    /// nothing worse.
    fn mark_announcements_read(&mut self, ids: Vec<i64>, cx: &mut Context<Self>) {
        let Some(credentials) = self.cloud_account.credentials.clone() else {
            return;
        };
        let unread: Vec<i64> = self
            .cloud_announcements
            .items
            .iter()
            .filter(|item| ids.contains(&item.id) && item.is_unread())
            .map(|item| item.id)
            .collect();
        if unread.is_empty() {
            return;
        }
        for item in &mut self.cloud_announcements.items {
            if unread.contains(&item.id) {
                item.read_at = Some("local".to_owned());
            }
        }
        cx.notify();

        cx.spawn(async move |_this, cx| {
            cx.background_executor()
                .spawn(async move {
                    let mut credentials = credentials;
                    if sub2api::refresh_if_needed(&mut credentials).is_err() {
                        return;
                    }
                    let client = sub2api::Client::new(credentials.endpoint.clone());
                    for id in unread {
                        let _ = client.mark_announcement_read(&credentials.access_token, id);
                    }
                })
                .await;
        })
        .detach();
    }

    fn open_announcement_detail(&mut self, id: i64, cx: &mut Context<Self>) {
        self.cloud_announcements.detail = Some(id);
        self.mark_announcements_read(vec![id], cx);
        cx.notify();
    }

    // ── Bell ───────────────────────────────────────────────────────────────

    /// The header bell. Only rendered while signed in — announcements are an
    /// account feature.
    pub(super) fn render_announcement_bell(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = Theme::current(cx);
        let unread = self.announcement_unread_count();
        div()
            .id("announcement-bell")
            .w(px(26.0))
            .h(px(26.0))
            .flex_none()
            .relative()
            .rounded(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .hover(|element| element.bg(theme.overlay))
            .active(|element| element.bg(theme.overlay_strong))
            .child(icon(
                "icons/bell.svg",
                14.0,
                if unread > 0 {
                    theme.accent
                } else {
                    theme.text_tertiary
                },
            ))
            .when(unread > 0, |element| {
                element.child(
                    div()
                        .absolute()
                        .top(px(4.0))
                        .right(px(5.0))
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .bg(theme.danger),
                )
            })
            .tooltip(|window, cx| Tooltip::new(tr!("announcements.title")).build(window, cx))
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.open_announcements_modal(cx);
            }))
    }

    // ── Modal ──────────────────────────────────────────────────────────────

    pub(super) fn render_announcements_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.cloud_announcements.open {
            return None;
        }
        let theme = Theme::current(cx);

        let detail = self
            .cloud_announcements
            .detail
            .and_then(|id| {
                self.cloud_announcements
                    .items
                    .iter()
                    .find(|item| item.id == id)
            })
            .cloned();
        let body = match detail {
            Some(item) => self.render_announcement_detail(&item, theme, cx),
            None => self.render_announcement_list(theme, cx),
        };

        let unread = self.announcement_unread_count();
        let header = div()
            .h(px(48.0))
            .px(px(16.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .text_size(sp(14.0))
                    .text_color(theme.text)
                    .child(icon("icons/bell.svg", 15.0, theme.text))
                    .child(tr!("announcements.title"))
                    .when(unread > 0, |element| {
                        element.child(
                            div()
                                .px(px(7.0))
                                .py(px(1.0))
                                .rounded(px(8.0))
                                .bg(theme.danger_soft)
                                .text_size(sp(11.0))
                                .text_color(theme.danger)
                                .child(tr!("announcements.unread_count", count = unread)),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .when(unread > 0, |element| {
                        element.child(
                            div()
                                .id("announcements-mark-all")
                                .px(px(8.0))
                                .h(px(24.0))
                                .rounded(px(7.0))
                                .flex()
                                .items_center()
                                .cursor_default()
                                .text_size(sp(11.5))
                                .text_color(theme.text_secondary)
                                .hover(|style| style.bg(theme.overlay))
                                .child(tr!("announcements.mark_all_read"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let ids: Vec<i64> = this
                                        .cloud_announcements
                                        .items
                                        .iter()
                                        .map(|item| item.id)
                                        .collect();
                                    this.mark_announcements_read(ids, cx);
                                })),
                        )
                    })
                    .child(
                        div()
                            .id("announcements-close")
                            .tab_index(0)
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(px(7.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_default()
                            .hover(|style| style.bg(theme.overlay))
                            .child(icon("icons/x.svg", 14.0, theme.text_secondary))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_announcements_modal(cx);
                            })),
                    ),
            );

        let card = div()
            .id("announcements-card")
            .w_full()
            .max_w(px(480.0))
            .max_h(px(600.0))
            .overflow_hidden()
            .rounded(px(18.0))
            .bg(theme.composer)
            .shadow_xl()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(
                div()
                    .id("announcements-body")
                    .px(px(16.0))
                    .pb(px(16.0))
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(body),
            );

        let scrim = if theme.is_dark {
            gpui::hsla(0.0, 0.0, 0.0, 0.34)
        } else {
            gpui::hsla(0.0, 0.0, 0.0, 0.16)
        };
        let layer = div()
            .id("announcements-layer")
            .absolute()
            .inset_0()
            .occlude()
            .bg(scrim)
            .p(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.close_announcements_modal(cx)),
            )
            .child(card);
        Some(gpui::deferred(layer).with_priority(4).into_any_element())
    }

    fn render_announcement_list(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let mut body = div().flex().flex_col();
        if self.cloud_announcements.loading && self.cloud_announcements.items.is_empty() {
            return body.child(
                div()
                    .py(px(26.0))
                    .flex()
                    .justify_center()
                    .text_size(sp(12.5))
                    .text_color(theme.text_secondary)
                    .child(tr!("announcements.loading")),
            );
        }
        if let Some(error) = self
            .cloud_announcements
            .error
            .as_ref()
            .filter(|_| self.cloud_announcements.items.is_empty())
        {
            return body
                .child(
                    div()
                        .pt(px(18.0))
                        .text_size(sp(12.5))
                        .text_color(theme.text)
                        .child(tr!("announcements.load_failed")),
                )
                .child(
                    div()
                        .mt(px(6.0))
                        .text_size(sp(11.5))
                        .line_height(sp(17.0))
                        .text_color(theme.text_secondary)
                        .child(error.clone()),
                );
        }
        if self.cloud_announcements.items.is_empty() {
            return body.child(
                div()
                    .py(px(26.0))
                    .flex()
                    .justify_center()
                    .text_size(sp(12.5))
                    .text_color(theme.text_secondary)
                    .child(tr!("announcements.empty")),
            );
        }

        for item in self.cloud_announcements.items.clone() {
            let id = item.id;
            let unread = item.is_unread();
            body = body.child(
                div()
                    .id(ElementId::Name(format!("announcement-{id}").into()))
                    .w_full()
                    .px(px(10.0))
                    .py(px(10.0))
                    .rounded(px(9.0))
                    .cursor_default()
                    .hover(|style| style.bg(theme.overlay))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .w(px(6.0))
                            .h(px(6.0))
                            .flex_none()
                            .rounded_full()
                            .bg(if unread { theme.danger } else { theme.border }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(sp(13.0))
                                    .font_weight(if unread {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if unread {
                                        theme.text
                                    } else {
                                        theme.text_secondary
                                    })
                                    .child(SharedString::from(item.title.clone())),
                            )
                            .children(item.created_date().map(|date| {
                                div()
                                    .text_size(sp(11.0))
                                    .text_color(theme.text_tertiary)
                                    .child(SharedString::from(date.to_owned()))
                            })),
                    )
                    .child(icon("icons/chevron-right.svg", 12.0, theme.text_tertiary))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_announcement_detail(id, cx);
                    })),
            );
        }
        body
    }

    fn render_announcement_detail(
        &self,
        item: &Announcement,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        // The transcript's own markdown engine renders the body; the parse is
        // cached per announcement id.
        let palette = MarkdownPalette::from_theme(&theme);
        let document = {
            let mut cache = self.announcement_markdown.borrow_mut();
            if !matches!(cache.as_ref(), Some((cached, _)) if *cached == item.id) {
                *cache = Some((item.id, MarkdownView::new()));
            }
            let (_, view) = cache.as_mut().expect("entry ensured above");
            view.set_text(&item.content, false);
            let ctx = MarkdownCtx::new(
                format!("announcement-md-{}", item.id),
                &palette,
                self.scaled_markdown_metrics(MarkdownMetrics::COMPACT),
                self.announcement_selection.clone(),
            );
            div()
                .text_color(theme.text_secondary)
                .children(md::render::markdown(view, &ctx))
        };

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("announcement-back")
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .py(px(4.0))
                    .cursor_default()
                    .text_size(sp(11.5))
                    .text_color(theme.text_secondary)
                    .hover(|style| style.text_color(theme.text))
                    .child(icon("icons/arrow-left.svg", 11.0, theme.text_secondary))
                    .child(tr!("announcements.back"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cloud_announcements.detail = None;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .text_size(sp(15.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(SharedString::from(item.title.clone())),
            )
            .children(item.created_date().map(|date| {
                div()
                    .mt(px(4.0))
                    .text_size(sp(11.0))
                    .text_color(theme.text_tertiary)
                    .child(SharedString::from(date.to_owned()))
            }))
            .child(
                div()
                    .mt(px(12.0))
                    .pt(px(12.0))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(document),
            )
    }
}
