//! The failure of the latest turn, shown above the composer.
//!
//! Fork addition. A failed turn used to say so in a toast gone after five
//! seconds, or in an assistant message the provider never wrote. The failure
//! now stays on the turn (`AgentTurn::error`) and this banner shows it until
//! the person retries, dismisses it, or starts another turn.

use sub2api::paywall::{Paywall, PaywallState};

use super::*;
use crate::model::TurnFailureKind;
use crate::ui::ActivationExt as _;

/// Past this the message is clipped to two lines until its details open.
const COLLAPSED_CHARS: usize = 160;

/// Fork: the action that fixes a named failure, ahead of Retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fix {
    /// Compact the conversation that no longer fits.
    Compact,
    /// Check the key or the account it belongs to.
    Credentials,
}

/// Fork: what the banner offers for a failure of `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Remedy {
    /// Whether "Continue" makes sense: sending the same conversation on
    /// cannot get past an overflow, a refused key or a refused request.
    pub(super) continue_allowed: bool,
    pub(super) fix: Option<Fix>,
}

/// The banner's actions for a failure the built-in agent named. A failure
/// it did not name — and one named by a newer build — keeps the actions
/// every failure had before.
pub(super) fn remedy(kind: Option<TurnFailureKind>) -> Remedy {
    use TurnFailureKind as Kind;
    let (continue_allowed, fix) = match kind {
        Some(Kind::ContextOverflow) => (false, Some(Fix::Compact)),
        Some(Kind::Auth | Kind::Quota | Kind::Forbidden) => (false, Some(Fix::Credentials)),
        Some(Kind::ContentPolicy | Kind::BadRequest) => (false, None),
        Some(
            Kind::RateLimited
            | Kind::Network
            | Kind::Unavailable
            | Kind::Interrupted
            | Kind::Unknown,
        )
        | None => (true, None),
    };
    Remedy {
        continue_allowed,
        fix,
    }
}

impl Waku {
    pub(super) fn render_error_banner(&self, cx: &mut Context<Self>) -> Option<Div> {
        let session = self.selected_session()?;
        if session.is_busy() || self.submission_preparations.contains(&session.id) {
            return None;
        }
        let turn = session
            .turns
            .last()
            .filter(|turn| turn.status == TurnStatus::Failed)?;
        let error = turn.error.as_ref().filter(|error| !error.dismissed)?;
        let theme = Theme::current(cx);
        let session_id = session.id;
        let turn_id = turn.id;
        let message = error.message.trim().to_owned();
        let long = message.lines().count() > 2 || message.chars().count() > COLLAPSED_CHARS;
        let details_open = self.error_details_open == Some(turn_id);
        let copy_text = message.clone();
        let paywall = self.turn_paywall(session.provider, &message);
        // Fork: the failure's kind picks the action that fixes it; a paywall
        // still comes first.
        let remedy = remedy(error.kind);
        let fix = remedy.fix.filter(|_| paywall.is_none());
        let custom_endpoint = self.provider_on_custom_endpoint(session.provider);
        // Fork (model_retry): a turn that got work done can be continued
        // without rewinding it; a goal may be about to resume on its own.
        let continuable = paywall.is_none()
            && remedy.continue_allowed
            && super::model_retry::turn_did_work(session, turn_id);
        let goal_resume = self.pending_goal_resume(session);

        // Fork: every action is keyboard-operable — focusable, with a visible
        // ring, and Enter/Space activate it (`ActivationExt`). This banner is
        // the way out of a failed turn and was reachable by mouse only.
        let button = |id: &str, label: String, primary: bool| {
            div()
                .id(SharedString::from(format!("error-banner-{id}-{turn_id}")))
                .tab_index(0)
                .focus_visible(|style| style.border_1().border_color(theme.accent))
                .h(px(26.0))
                .px(px(10.0))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .cursor_default()
                .text_size(sp(12.5))
                .font_weight(FontWeight::MEDIUM)
                .when(primary, |button| {
                    button
                        .bg(theme.inverse)
                        .text_color(theme.on_inverse)
                        .hover(|button| button.opacity(0.9))
                })
                .when(!primary, |button| {
                    button
                        .border_1()
                        .border_color(theme.border_strong)
                        .text_color(theme.text_secondary)
                        .hover(|button| button.bg(theme.overlay).text_color(theme.text))
                })
                .active(|button| button.opacity(0.8))
                .child(label)
        };

        let mut actions = div().flex().items_center().gap(px(8.0)).mt(px(10.0));
        // A turn the account could not pay for: the purchase that fixes it
        // leads, and retrying — pointless until then — steps back.
        if let Some(paywall) = paywall {
            let label = match paywall {
                Paywall::Balance => tr!("error_banner.top_up"),
                Paywall::PlanLimit => tr!("error_banner.upgrade_plan"),
                Paywall::PlanInactive => tr!("error_banner.renew_plan"),
            };
            actions = actions.child(button("paywall", label, true).on_activation(
                cx,
                move |this, _, cx| match paywall {
                    Paywall::Balance => this.open_cloud_pay_modal(cx),
                    Paywall::PlanLimit | Paywall::PlanInactive => {
                        this.open_settings_page(SettingsPage::Plans, cx);
                    }
                },
            ));
        }
        match fix {
            Some(Fix::Compact) => {
                actions = actions.child(
                    button("compact", tr!("error_banner.compact"), true)
                        .on_activation(cx, move |this, _, cx| {
                            this.compact_failed_turn(session_id, cx)
                        }),
                );
            }
            Some(Fix::Credentials) => {
                let (label, page) = if custom_endpoint {
                    (
                        tr!("error_banner.model_providers"),
                        SettingsPage::ModelProviders,
                    )
                } else {
                    (
                        tr!("error_banner.account_settings"),
                        SettingsPage::CloudAccount,
                    )
                };
                actions = actions.child(
                    button("credentials", label, true)
                        .on_activation(cx, move |this, _, cx| this.open_settings_page(page, cx)),
                );
            }
            None => {}
        }
        if continuable {
            actions = actions.child(
                button("continue", tr!("error_banner.continue"), true).on_activation(
                    cx,
                    move |this, _, cx| {
                        this.continue_failed_turn(session_id, cx);
                    },
                ),
            );
        }
        actions = actions
            .child(
                button(
                    "retry",
                    tr!("error_banner.retry"),
                    paywall.is_none() && !continuable && fix.is_none(),
                )
                .on_activation(cx, move |this, _, cx| {
                    this.retry_failed_turn(session_id, cx);
                }),
            )
            .child(button("copy", tr!("common.copy"), false).on_activation(
                cx,
                move |this, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                    this.show_toast(tr!("common.copied"));
                    cx.notify();
                },
            ));
        if long {
            let label = if details_open {
                tr!("error_banner.hide_details")
            } else {
                tr!("error_banner.show_details")
            };
            actions = actions.child(button("details", label, false).on_activation(
                cx,
                move |this, _, cx| {
                    this.error_details_open = (!details_open).then_some(turn_id);
                    cx.notify();
                },
            ));
        }

        let header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(icon("icons/circle-x.svg", 14.0, theme.danger))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(sp(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(tr!("error_banner.title")),
            )
            .child(
                div()
                    .id(SharedString::from(format!(
                        "error-banner-dismiss-{turn_id}"
                    )))
                    .tab_index(0)
                    .focus_visible(|style| style.bg(theme.overlay_strong))
                    .size(px(22.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_default()
                    .hover(|button| button.bg(theme.overlay_strong))
                    .child(icon("icons/x.svg", 12.0, theme.text_tertiary))
                    .tooltip(Tooltip::text(tr!("error_banner.dismiss")))
                    .on_activation(cx, move |this, _, cx| {
                        this.dismiss_turn_error(session_id, turn_id, cx);
                    }),
            );
        let body = div()
            .id(SharedString::from(format!(
                "error-banner-message-{turn_id}"
            )))
            .mt(px(6.0))
            .text_size(sp(12.5))
            .line_height(sp(17.0))
            .text_color(theme.text_secondary)
            .whitespace_normal()
            .when(details_open, |body| {
                body.max_h(px(180.0))
                    .overflow_y_scroll()
                    .font_family(crate::md::render::MONO_FAMILY)
            })
            .when(!details_open, |body| body.line_clamp(2))
            .child(message);
        let goal_note = goal_resume.map(|note| {
            div()
                .mt(px(6.0))
                .text_size(sp(12.0))
                .text_color(theme.text_tertiary)
                .child(note)
        });

        Some(
            div().px(px(20.0)).pb(px(8.0)).child(
                div()
                    .w_full()
                    .max_w(px(CONTENT_MAX_WIDTH))
                    .mx_auto()
                    .p(px(12.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(theme.danger.opacity(0.35))
                    .bg(theme.raised)
                    .shadow_md()
                    .child(header)
                    .child(body)
                    .children(goal_note)
                    .child(actions),
            ),
        )
    }

    /// Which purchase would have let a failed turn through — only for a turn
    /// that went through the managed gateway; see [`sub2api::paywall`].
    fn turn_paywall(&self, provider: ProviderKind, message: &str) -> Option<Paywall> {
        if self.cloud_account.credentials.is_none() || !self.cloud_account.routing_enabled {
            return None;
        }
        let subscriptions = self.cloud_account.subscriptions.as_deref().unwrap_or(&[]);
        let state = PaywallState {
            balance: self.cloud_account.user.as_ref().map(|user| user.balance),
            has_active_subscription: !subscriptions.is_empty(),
            exhausted_subscription: subscriptions.iter().any(|subscription| {
                super::cloud_subscriptions::subscription_windows(subscription)
                    .iter()
                    .any(|(_, window)| {
                        window.limit_usd > 0.0 && window.used_usd >= window.limit_usd
                    })
            }),
        };
        let paywall = sub2api::paywall::classify(message, &state)?;
        (!self.provider_on_custom_endpoint(provider)).then_some(paywall)
    }

    /// Whether `provider`'s requests leave through the user's own endpoint
    /// rather than the gateway — or whether the gateway routes it at all.
    /// Another sub2api deployment answers with the same codes, and buying
    /// here would not fix it. The built-in agent counts as custom when any of
    /// its three APIs is: which one the failed turn used is not recorded.
    fn provider_on_custom_endpoint(&self, provider: ProviderKind) -> bool {
        let slots: &[&str] = match provider {
            ProviderKind::Claude => &["claude"],
            ProviderKind::Codex => &["codex"],
            ProviderKind::Grok => &["grok"],
            ProviderKind::OpenCode => &["opencode"],
            ProviderKind::Pi => &["pi"],
            ProviderKind::Native => &sub2api::custom_api::NATIVE_SLOTS,
            _ => return true,
        };
        // A CLI kept on the user's own sign-in never reaches the gateway.
        if slots
            .iter()
            .any(|slot| self.cloud_account.cli_takeover.keeps_own(slot))
        {
            return true;
        }
        let config = self.custom_api_snapshot();
        slots
            .iter()
            .any(|slot| config.routed_endpoint(slot).is_some())
    }

    /// Put the banner away for this turn. The choice is kept on the turn, so
    /// it survives a restart.
    pub(super) fn dismiss_turn_error(
        &mut self,
        session_id: Uuid,
        turn_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        let dismissed = self
            .state
            .session_mut(session_id)
            .and_then(|session| session.turns.iter_mut().find(|turn| turn.id == turn_id))
            .and_then(|turn| turn.error.as_mut())
            .map(|error| error.dismissed = true)
            .is_some();
        if dismissed {
            self.state.mark_session_dirty(session_id);
        }
        if self.error_details_open == Some(turn_id) {
            self.error_details_open = None;
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remedies_follow_the_failure_kind() {
        use TurnFailureKind as Kind;
        assert_eq!(
            remedy(Some(Kind::ContextOverflow)),
            Remedy {
                continue_allowed: false,
                fix: Some(Fix::Compact)
            }
        );
        for kind in [Kind::Auth, Kind::Quota, Kind::Forbidden] {
            assert_eq!(
                remedy(Some(kind)),
                Remedy {
                    continue_allowed: false,
                    fix: Some(Fix::Credentials)
                }
            );
        }
        for kind in [Kind::ContentPolicy, Kind::BadRequest] {
            assert_eq!(
                remedy(Some(kind)),
                Remedy {
                    continue_allowed: false,
                    fix: None
                }
            );
        }
        // Transient failures, unnamed ones and kinds from a newer build keep
        // the actions every failure had.
        for kind in [
            Some(Kind::RateLimited),
            Some(Kind::Network),
            Some(Kind::Unavailable),
            Some(Kind::Interrupted),
            Some(Kind::Unknown),
            None,
        ] {
            assert_eq!(
                remedy(kind),
                Remedy {
                    continue_allowed: true,
                    fix: None
                }
            );
        }
    }
}
