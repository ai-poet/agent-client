//! Shared endpoint controls and cloud-route construction.
//!
//! The built-in agent and the model-provider settings use these primitives.
//! Connectivity and speed tests run on the background executor; render reads
//! only their in-memory results.

use std::time::Duration;

use sub2api::custom_api::ProbeVerdict;

use crate::ui::ActivationExt as _;

use super::*;

/// Latency past which a reachable endpoint is reported as slow.
const SLOW_ENDPOINT: Duration = Duration::from_millis(800);

/// The connectivity test's progress and outcome.
pub(super) struct EndpointTest {
    pub running: bool,
    pub result: Option<sub2api::custom_api::ProbeResult>,
    pub(super) generation: u64,
}

/// A candidate speed test's progress and results.
pub(super) struct SpeedTest {
    pub running: bool,
    /// One entry per candidate, in the order they were listed.
    pub results: Vec<sub2api::speedtest::CandidateResult>,
    pub(super) generation: u64,
}

/// Build the built-in agent's cloud route from in-memory account state.
pub(super) fn cloud_config(waku: &Waku) -> Option<sub2api::GatewayConfig> {
    let origin = waku.cloud_account.gateway_origin.origin();
    waku.cloud_account.credentials.as_ref().map(|credentials| {
        sub2api::gateway_config_with_origin(
            credentials,
            waku.cloud_account.routing_enabled,
            origin.as_deref(),
        )
    })
}

pub(super) fn url_error_label(error: &sub2api::custom_api::UrlError) -> String {
    use sub2api::custom_api::UrlError;
    let reason = match error {
        UrlError::Empty => tr!("cli_setup.custom_url_empty"),
        UrlError::Whitespace => tr!("cli_setup.custom_url_whitespace"),
        UrlError::Scheme(scheme) => tr!("cli_setup.custom_url_scheme", scheme = scheme),
        UrlError::NoHost => tr!("cli_setup.custom_url_no_host"),
    };
    tr!("cli_setup.custom_invalid_url", reason = reason)
}

/// Colour for a measured latency. Always rendered beside the number itself,
/// never as the only signal.
pub(super) fn latency_color(theme: Theme, ms: u128) -> gpui::Hsla {
    use sub2api::speedtest::LatencyTier;
    match sub2api::speedtest::latency_tier(ms) {
        LatencyTier::Fast | LatencyTier::Ok => theme.success,
        LatencyTier::Slow => theme.warning,
        LatencyTier::VerySlow => theme.danger,
    }
}

/// A card action button. Every one is keyboard-operable: focusable, with
/// a visible focus ring, and Enter/Space activate it like a click.
#[allow(clippy::too_many_arguments)]
pub(super) fn card_button(
    theme: Theme,
    id: SharedString,
    label: String,
    primary: bool,
    disabled: bool,
    cx: &mut Context<Waku>,
    activate: impl Fn(&mut Waku, &mut Window, &mut Context<Waku>) + 'static,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .tab_index(0)
        .focus_visible(|style| style.border_1().border_color(theme.accent))
        .h(px(26.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_default()
        .text_size(sp(11.5))
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
    if disabled {
        button
    } else {
        button.on_activation(cx, activate)
    }
}

/// How a connectivity probe went, in one line.
///
/// Shared with the model-providers page so the two surfaces cannot drift
/// into describing the same failure differently — an unauthorized key and an
/// unreachable host are different problems, and the wording is the only
/// thing that says which.
pub(super) fn probe_status_line(theme: Theme, test: &EndpointTest) -> Div {
    if test.running {
        return status_line(
            theme,
            "icons/loader-circle.svg",
            theme.text_ghost,
            tr!("cli_setup.custom_testing"),
        );
    }
    let Some(result) = &test.result else {
        return div();
    };
    match result.verdict {
        ProbeVerdict::Ok if result.latency_ms < SLOW_ENDPOINT.as_millis() => status_line(
            theme,
            "icons/check.svg",
            theme.success,
            tr!("cli_setup.custom_connect_ok", ms = result.latency_ms),
        ),
        ProbeVerdict::Ok => status_line(
            theme,
            "icons/check.svg",
            theme.warning,
            tr!("cli_setup.custom_connect_slow", ms = result.latency_ms),
        ),
        ProbeVerdict::Unauthorized => status_line(
            theme,
            "icons/alert.svg",
            theme.warning,
            tr!(
                "cli_setup.custom_test_unauthorized",
                status = result.status.unwrap_or_default()
            ),
        ),
        ProbeVerdict::HttpError => status_line(
            theme,
            "icons/alert.svg",
            theme.warning,
            tr!(
                "cli_setup.custom_test_http",
                status = result.status.unwrap_or_default(),
                detail = result.detail.clone()
            ),
        ),
        ProbeVerdict::Unreachable => status_line(
            theme,
            "icons/x.svg",
            theme.danger,
            tr!(
                "cli_setup.custom_connect_failed",
                error = result.detail.clone()
            ),
        ),
    }
}

/// A status line inside a card: icon, tinted text.
pub(super) fn status_line(
    theme: Theme,
    icon_path: &'static str,
    color: gpui::Hsla,
    text: String,
) -> Div {
    let _ = theme;
    div()
        .flex()
        .items_start()
        .gap(px(6.0))
        .text_size(sp(12.0))
        .line_height(sp(17.0))
        .text_color(color)
        .child(
            div()
                .flex_none()
                .pt(px(2.0))
                .child(icon(icon_path, 12.0, color)),
        )
        .child(div().min_w_0().flex_1().child(text))
}
