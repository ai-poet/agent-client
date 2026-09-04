//! A visible way to check for updates, and a visible way to install one.
//!
//! Fork addition. Upstream reaches the updater through the macOS app menu's
//! "Check for Updates…" and a small pill in the sidebar footer once a check
//! has found something. Windows has no menu bar in this window, so the check
//! was unreachable there, and the footer pill is easy to miss. This adds a
//! Check-for-updates row to Settings → General that reports "checking",
//! "up to date" and "available", and a banner across the top of the content
//! whenever an update is ready — one click installs it, a second control
//! puts it away until the next one.
//!
//! The updater itself is untouched: the row and the banner read
//! `updater_status`, drive the same `check_for_updates` and
//! `install_available_update` calls the menu and the pill use, and learn the
//! outcome from the same `UpdaterEvent`s.

use crate::ui::ActivationExt as _;

use super::*;

/// Presentation state around the updater.
#[derive(Default)]
pub(super) struct UpdateUiState {
    /// A user-initiated check is running and has not reported back.
    pub check_in_flight: bool,
    /// The banner was put away for the update currently on offer.
    pub banner_dismissed: bool,
}

impl UpdateUiState {
    /// Every updater event ends a manual check; a newly available update
    /// brings a dismissed banner back.
    pub(super) fn on_updater_event(&mut self, event: &crate::updater::UpdaterEvent) {
        self.check_in_flight = false;
        if matches!(
            event,
            crate::updater::UpdaterEvent::StatusChanged(crate::updater::UpdateStatus::Available)
        ) {
            self.banner_dismissed = false;
        }
    }
}

impl Waku {
    fn updater(cx: &App) -> Option<&crate::updater::Updater> {
        cx.try_global::<crate::updater::UpdaterState>()
            .and_then(|state| state.0.as_ref())
    }

    /// Ask the updater now. The outcome comes back as an event: up to date,
    /// available, or failed — each already surfaced by the app.
    pub(super) fn check_for_updates_now(&mut self, cx: &mut Context<Self>) {
        if self.update_ui.check_in_flight
            || self.updater_status == crate::updater::UpdateStatus::Updating
        {
            return;
        }
        let Some(updater) = Self::updater(cx) else {
            return;
        };
        updater.check_for_updates();
        self.update_ui.check_in_flight = true;
        cx.notify();
    }

    pub(super) fn dismiss_update_banner(&mut self, cx: &mut Context<Self>) {
        self.update_ui.banner_dismissed = true;
        cx.notify();
    }

    /// The version on offer, when the updater knows it.
    fn available_update_version(cx: &App) -> Option<String> {
        Self::updater(cx).and_then(|updater| updater.available_version())
    }

    /// The Settings → General row: current version, and a button that
    /// checks, reports, or installs depending on where the updater is.
    pub(super) fn render_update_check_card(&self, theme: Theme, cx: &mut Context<Self>) -> Div {
        let status = self.updater_status;
        let checking = self.update_ui.check_in_flight;
        let available_version = Self::available_update_version(cx);

        let detail = match status {
            crate::updater::UpdateStatus::Available => match &available_version {
                Some(version) => tr!("updater.available_version", version = version),
                None => tr!("updater.available"),
            },
            crate::updater::UpdateStatus::Updating => tr!("updater.updating"),
            crate::updater::UpdateStatus::Idle if checking => tr!("updater.checking"),
            crate::updater::UpdateStatus::Idle => {
                tr!("updater.current_version", version = env!("CARGO_PKG_VERSION"))
            }
        };
        let (label, primary, disabled): (String, bool, bool) = match status {
            crate::updater::UpdateStatus::Available => (tr!("updater.install_now"), true, false),
            crate::updater::UpdateStatus::Updating => (tr!("updater.updating"), true, true),
            crate::updater::UpdateStatus::Idle => (
                if checking {
                    tr!("updater.checking")
                } else {
                    tr!("updater.check_now")
                },
                false,
                checking,
            ),
        };

        let button = div()
            .id("check-for-updates")
            .tab_index(0)
            .focus_visible(|style| style.border_1().border_color(theme.accent))
            .h(px(29.0))
            .px(px(12.0))
            .rounded(px(7.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.5))
            .opacity(if disabled { 0.55 } else { 1.0 });
        let button = if primary {
            button
                .bg(theme.inverse)
                .text_color(theme.on_inverse)
                .font_weight(FontWeight::MEDIUM)
        } else {
            button
                .border_1()
                .border_color(theme.border_strong)
                .text_color(theme.text_secondary)
                .hover(|style| style.bg(theme.overlay))
        };
        let button = button.child(label);
        let button = if disabled {
            button
        } else {
            button.on_activation(cx, move |this, _, cx| match status {
                crate::updater::UpdateStatus::Available => this.start_available_update(cx),
                _ => this.check_for_updates_now(cx),
            })
        };

        div()
            .mt(px(15.0))
            .w_full()
            .min_h(px(60.0))
            .px(px(20.0))
            .py(px(12.0))
            .rounded(px(13.0))
            .bg(theme.raised)
            .flex()
            .items_center()
            .gap(px(24.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(sp(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("settings.check_for_updates")),
                    )
                    .child(
                        div()
                            .mt(px(5.0))
                            .text_size(sp(12.5))
                            .line_height(sp(18.0))
                            .text_color(if status == crate::updater::UpdateStatus::Available {
                                theme.success
                            } else {
                                theme.text_secondary
                            })
                            .child(detail),
                    ),
            )
            .child(button)
    }

    /// The strip across the top of the content while an update is on offer
    /// (or installing). Click the strip or its button to install; the close
    /// control puts it away until the next update.
    pub(super) fn render_update_banner(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let status = self.updater_status;
        let installing = status == crate::updater::UpdateStatus::Updating;
        if status == crate::updater::UpdateStatus::Idle
            || (!installing && self.update_ui.banner_dismissed)
        {
            return None;
        }
        let theme = Theme::current(cx);
        let foreground: gpui::Hsla = rgb(0xFFFFFF).into();
        let text = if installing {
            tr!("updater.updating")
        } else {
            match Self::available_update_version(cx) {
                Some(version) => tr!("updater.available_version", version = version),
                None => tr!("updater.available"),
            }
        };

        let mut banner = div()
            .id("update-banner")
            .w_full()
            .flex_none()
            .h(px(34.0))
            .px(px(14.0))
            .bg(theme.gauge)
            .text_color(foreground)
            .text_size(sp(12.5))
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(if installing {
                motion::spin_slow(icon("icons/loader-circle.svg", 13.0, foreground)).into_any_element()
            } else {
                icon("icons/arrow-down.svg", 13.0, foreground).into_any_element()
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(text),
            );

        if !installing {
            banner = banner
                .cursor_default()
                .hover(|style| style.opacity(0.94))
                .on_click(cx.listener(|this, _, _, cx| this.start_available_update(cx)))
                .child(
                    div()
                        .id("update-banner-install")
                        .tab_index(0)
                        .focus_visible(|style| style.border_1().border_color(foreground))
                        .h(px(24.0))
                        .px(px(10.0))
                        .rounded_full()
                        .flex()
                        .flex_none()
                        .items_center()
                        .cursor_default()
                        .bg(foreground.opacity(0.18))
                        .hover(|style| style.bg(foreground.opacity(0.28)))
                        .child(tr!("updater.install_now"))
                        .on_activation(cx, |this, _, cx| this.start_available_update(cx)),
                )
                .child(
                    div()
                        .id("update-banner-dismiss")
                        .tab_index(0)
                        .focus_visible(|style| style.border_1().border_color(foreground))
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded(px(6.0))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .cursor_default()
                        .hover(|style| style.bg(foreground.opacity(0.18)))
                        .tooltip(Tooltip::text(tr_cow!("updater.later")))
                        .child(icon("icons/x.svg", 12.0, foreground))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_activation(cx, |this, _, cx| this.dismiss_update_banner(cx)),
                );
        }
        Some(banner)
    }
}
