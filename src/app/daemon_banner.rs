//! One place that knows whether the daemon socket is up, and one strip that
//! says so.
//!
//! Fork addition. The supervisor in `waku_client` redials a dropped socket and
//! restarts a dead process on its own, but the app used to learn about either
//! only by having a request fail: once per background poll, once per state
//! save, once per draft save, and it answered each failure with the same
//! toast. Now the maintenance clock reads the connection once a second,
//! derives a banner stage from how long it has been down, and everything else
//! consults the phase instead of discovering the outage for itself.
//!
//! Short outages stay invisible: a same-process redial or a development
//! rebuild swap is back within the grace period. Longer ones show a strip with
//! a spinner, and past the point where the supervisor has exhausted its
//! redials the strip offers to restart the daemon outright.

use crate::ui::ActivationExt as _;

use super::*;

/// A dev rebuild swap or a same-process redial is back within this.
pub(super) const DAEMON_BANNER_GRACE: Duration = Duration::from_millis(1500);
/// Past this the supervisor has spent its redial budget and is restarting the
/// process or stuck on one that refuses the socket; offer a restart.
pub(super) const DAEMON_RESTART_OFFER_AFTER: Duration = Duration::from_secs(10);

/// The daemon connection as observed on the maintenance clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DaemonConnectionPhase {
    Connected,
    /// The socket is down; the supervisor is redialing or restarting.
    Reconnecting {
        since: Instant,
    },
    /// A restart from the banner failed with this error.
    Failed(String),
}

impl DaemonConnectionPhase {
    pub(super) fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }
}

/// What the strip across the top of the content shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DaemonBannerStage {
    Hidden,
    Reconnecting,
    OfferRestart,
    Failed,
}

/// The phase after one observation of the socket, or `None` when nothing
/// changed. Pure: the clock hands in `now`.
pub(super) fn next_connection_phase(
    current: &DaemonConnectionPhase,
    disconnected: bool,
    now: Instant,
) -> Option<DaemonConnectionPhase> {
    match (current, disconnected) {
        (DaemonConnectionPhase::Connected, true) => {
            Some(DaemonConnectionPhase::Reconnecting { since: now })
        }
        (DaemonConnectionPhase::Reconnecting { .. } | DaemonConnectionPhase::Failed(_), false) => {
            Some(DaemonConnectionPhase::Connected)
        }
        _ => None,
    }
}

/// The banner stage for a phase at `now`. A user-driven reconfigure already
/// says "Restarting" in Settings, so it never doubles as an outage; a
/// dismissed strip stays away until the connection is back.
pub(super) fn daemon_banner_stage(
    phase: &DaemonConnectionPhase,
    now: Instant,
    reconfigure_pending: bool,
    dismissed: bool,
) -> DaemonBannerStage {
    match phase {
        DaemonConnectionPhase::Connected => DaemonBannerStage::Hidden,
        _ if reconfigure_pending || dismissed => DaemonBannerStage::Hidden,
        DaemonConnectionPhase::Failed(_) => DaemonBannerStage::Failed,
        DaemonConnectionPhase::Reconnecting { since } => {
            let down = now.saturating_duration_since(*since);
            if down < DAEMON_BANNER_GRACE {
                DaemonBannerStage::Hidden
            } else if down < DAEMON_RESTART_OFFER_AFTER {
                DaemonBannerStage::Reconnecting
            } else {
                DaemonBannerStage::OfferRestart
            }
        }
    }
}

impl Waku {
    /// Observe the socket once and settle the phase and banner stage. One
    /// atomic load behind a briefly held mutex, the same call the composer
    /// prewarm makes, so it runs on the maintenance clock and after each
    /// task-state sync, never from render.
    pub(super) fn maintain_daemon_connection(&mut self, cx: &mut Context<Self>) {
        let disconnected = self.daemon.client().is_disconnected();
        let now = Instant::now();
        let mut changed = false;
        if let Some(next) = next_connection_phase(&self.daemon_connection, disconnected, now) {
            let recovered = next.is_connected();
            self.daemon_connection = next;
            changed = true;
            if recovered {
                self.on_daemon_reconnected(cx);
            }
        }
        let stage = daemon_banner_stage(
            &self.daemon_connection,
            now,
            self.daemon_reconfigure_pending,
            self.daemon_banner_dismissed,
        );
        if stage != self.daemon_banner_stage {
            self.daemon_banner_stage = stage;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    /// The socket is back: put away what the outage left on screen and replay
    /// what could not be saved while it was down.
    fn on_daemon_reconnected(&mut self, cx: &mut Context<Self>) {
        self.daemon_banner_dismissed = false;
        let stale = tr!("errors.daemon_disconnected");
        for runtime in self.runtimes.values_mut() {
            if runtime.last_driver_error.as_deref().is_some_and(|error| {
                error == stale || waku_client::is_daemon_transport_error(error)
            }) {
                runtime.last_driver_error = None;
            }
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| toast.message == stale)
        {
            self.hide_toast();
        }
        // Dirty sessions survived the failed saves (`StateStore::save` clears
        // them only after the daemon acknowledged), and the draft store only
        // advances its snapshot on success, so both replays are exact.
        self.save();
        self.schedule_composer_draft_save(cx);
    }

    /// Replace the desktop-managed daemon from the banner. The tasks it was
    /// running end: their forwarding threads receive the new connection, find
    /// no runtime to attach to, and report the exit the ordinary way.
    pub(super) fn restart_daemon(&mut self, cx: &mut Context<Self>) {
        if self.daemon_reconfigure_pending || self.daemon.is_remote() {
            return;
        }
        // The Settings card shows "Restarting" off the same flag, and the
        // banner steps aside while it is set.
        self.daemon_reconfigure_pending = true;
        let daemon = self.daemon.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { daemon.restart() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.daemon_reconfigure_pending = false;
                if let Err(error) = result {
                    let error = error.to_string();
                    this.daemon_connection = DaemonConnectionPhase::Failed(error.clone());
                    this.show_toast(tr!("daemon.restart_failed", error = error));
                }
                // Success is observed through the published connection, not
                // assumed: the next observation flips the phase.
                this.maintain_daemon_connection(cx);
            });
        })
        .detach();
        self.maintain_daemon_connection(cx);
    }

    pub(super) fn dismiss_daemon_banner(&mut self, cx: &mut Context<Self>) {
        self.daemon_banner_dismissed = true;
        self.maintain_daemon_connection(cx);
    }

    /// The strip across the top of the content while the daemon socket is
    /// down. Reads only the settled stage and phase: no clocks, no locks.
    pub(super) fn render_daemon_connection_banner(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let stage = self.daemon_banner_stage;
        if stage == DaemonBannerStage::Hidden {
            return None;
        }
        let theme = Theme::current(cx);
        let foreground: gpui::Hsla = rgb(0xFFFFFF).into();
        let reconnecting = stage == DaemonBannerStage::Reconnecting;
        let (label, detail) = match (stage, &self.daemon_connection) {
            (DaemonBannerStage::Reconnecting, _) => (
                tr!("daemon.phase_connecting"),
                tr!("daemon.banner_reconnecting_detail"),
            ),
            (DaemonBannerStage::Failed, DaemonConnectionPhase::Failed(error)) => (
                tr!("daemon.phase_error"),
                tr!("daemon.restart_failed", error = error.clone()),
            ),
            (DaemonBannerStage::Failed, _) => (
                tr!("daemon.phase_error"),
                tr!("daemon.banner_unreachable_detail"),
            ),
            _ => (
                tr!("daemon.phase_disconnected"),
                tr!("daemon.banner_unreachable_detail"),
            ),
        };

        let mut banner = div()
            .id("daemon-banner")
            .w_full()
            .flex_none()
            .h(px(34.0))
            .px(px(14.0))
            .bg(if reconnecting {
                theme.gauge
            } else {
                theme.danger
            })
            .text_color(foreground)
            .text_size(sp(12.5))
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(if reconnecting {
                motion::spin_slow(icon("icons/loader-circle.svg", 13.0, foreground))
                    .into_any_element()
            } else {
                icon("icons/alert.svg", 13.0, foreground).into_any_element()
            })
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
            .child(div().flex_1().min_w_0().truncate().child(detail));

        if !reconnecting && !self.daemon.is_remote() {
            banner = banner.child(
                div()
                    .id("daemon-banner-restart")
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
                    .child(tr!("daemon.restart"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_activation(cx, |this, _, cx| this.restart_daemon(cx)),
            );
        }
        banner = banner.child(
            div()
                .id("daemon-banner-dismiss")
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
                .tooltip(Tooltip::text(tr_cow!("common.dismiss_notification")))
                .child(icon("icons/x.svg", 12.0, foreground))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_activation(cx, |this, _, cx| this.dismiss_daemon_banner(cx)),
        );
        Some(banner)
    }
}
