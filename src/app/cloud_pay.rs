//! The native top-up modal — the Electron client's pay sheet, in GPUI.
//!
//! The flow is pure API, no webview: load the payment config, take an amount
//! and a method, create an order, then either render the QR code natively or
//! hand `pay_url` to the browser, polling the order every two seconds until
//! it settles. Stripe is the one flow a native window cannot carry (it needs
//! the hosted checkout), so picking it opens the full pay center instead.
//!
//! All service traffic lives in `sub2api::pay`; this file is the view and the
//! plumbing that runs it off the UI thread.

use std::rc::Rc;
use std::time::{Duration, Instant};

use sub2api::pay::{OrderStatus, PayClient, PayConfig, PayFlow, PayOrder};

use super::*;

/// Poll cadence and give-up horizon, the Electron client's numbers.
const ORDER_POLL_INTERVAL: Duration = Duration::from_secs(2);
const ORDER_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The Electron client's quick-amount chips, filtered to the method's range.
const QUICK_AMOUNTS: [f64; 6] = [10.0, 20.0, 50.0, 100.0, 200.0, 500.0];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PayStage {
    LoadingConfig,
    /// Config loading failed; only the hosted pay center can help.
    ConfigError,
    Form,
    Paying,
    /// The order ended without settling: failed, cancelled, or expired.
    Result,
}

/// The QR code as a bit matrix: side length and row-major dark flags.
type QrMatrix = Rc<(usize, Vec<bool>)>;

pub(super) struct CloudPayState {
    pub stage: PayStage,
    pub config: Option<PayConfig>,
    pub selected_type: Option<String>,
    pub amount: Entity<TextInput>,
    pub error: Option<String>,
    pub order: Option<PayOrder>,
    pub order_status: Option<OrderStatus>,
    pub qr: Option<QrMatrix>,
    pub busy: bool,
    /// Bumped on every new order/close so stale poll loops fall silent.
    pub epoch: usize,
}

impl Waku {
    /// Open the top-up modal. Materialized on the next frame, where a
    /// `Window` exists to build the amount field.
    pub(super) fn open_cloud_pay_modal(&mut self, cx: &mut Context<Self>) {
        if self.cloud_account.credentials.is_none() {
            return;
        }
        self.cloud_pay_request = true;
        cx.notify();
    }

    fn materialize_cloud_pay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloud_pay.is_some() {
            return;
        }
        let amount = cx.new(|cx| {
            let mut input = TextInput::new(window, cx).select_all_on_focus_click();
            input.set_content("10", cx);
            input
        });
        let epoch = self.cloud_pay_epoch.wrapping_add(1);
        self.cloud_pay_epoch = epoch;
        self.cloud_pay = Some(CloudPayState {
            stage: PayStage::LoadingConfig,
            config: None,
            selected_type: None,
            amount,
            error: None,
            order: None,
            order_status: None,
            qr: None,
            busy: false,
            epoch,
        });
        self.cloud_pay_load_config(cx);
        cx.notify();
    }

    /// Close and forget. A completed payment refreshed the balance already;
    /// an abandoned order keeps counting down service-side and expires there.
    pub(super) fn close_cloud_pay_modal(&mut self, cx: &mut Context<Self>) {
        self.cloud_pay_request = false;
        // Invalidate any running poll loop.
        self.cloud_pay_epoch = self.cloud_pay_epoch.wrapping_add(1);
        if self.cloud_pay.take().is_some() {
            cx.notify();
        }
    }

    /// The pay client for the signed-in session, with the UI's language.
    fn pay_client(&self) -> Option<(PayClient, String)> {
        let credentials = self.cloud_account.credentials.as_ref()?;
        let locale = self.state.language.locale();
        let lang = locale.split('-').next().unwrap_or(locale).to_owned();
        Some((
            PayClient::new(credentials.endpoint.clone(), lang),
            credentials.access_token.clone(),
        ))
    }

    fn cloud_pay_load_config(&mut self, cx: &mut Context<Self>) {
        let Some((client, token)) = self.pay_client() else {
            return;
        };
        let Some(state) = self.cloud_pay.as_mut() else {
            return;
        };
        state.stage = PayStage::LoadingConfig;
        state.error = None;
        let epoch = state.epoch;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { client.load_config(&token) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(state) = this.cloud_pay.as_mut().filter(|state| state.epoch == epoch)
                else {
                    return;
                };
                match loaded {
                    Ok(config) => {
                        // Keep a still-valid pick; otherwise the first method.
                        let keep = state
                            .selected_type
                            .take()
                            .filter(|current| config.enabled_payment_types.contains(current));
                        state.selected_type = keep.or_else(|| {
                            config.enabled_payment_types.first().cloned()
                        });
                        let floor = config.min_amount.max(1.0);
                        state.amount.update(cx, |input, cx| {
                            if input.content().trim().is_empty() {
                                input.set_content(format_amount(floor), cx);
                            }
                        });
                        state.config = Some(config);
                        state.stage = PayStage::Form;
                    }
                    Err(error) => {
                        state.stage = PayStage::ConfigError;
                        state.error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The form's main action: validate, create the order, start the flow.
    fn cloud_pay_create_order(&mut self, cx: &mut Context<Self>) {
        let Some((client, token)) = self.pay_client() else {
            return;
        };
        let Some(state) = self.cloud_pay.as_mut() else {
            return;
        };
        if state.busy {
            return;
        }
        let Some(config) = state.config.clone() else {
            return;
        };
        let Some(payment_type) = state.selected_type.clone() else {
            state.error = Some(tr!("pay.select_method"));
            cx.notify();
            return;
        };

        // Stripe needs the hosted checkout; the native modal hands over.
        if sub2api::pay::is_stripe(&payment_type) {
            let url = client.pay_center_url(&token);
            cx.open_url(&url);
            return;
        }

        if config.pending_count >= config.max_pending_orders {
            state.error = Some(tr!("pay.too_many_pending", count = config.pending_count));
            cx.notify();
            return;
        }
        if config
            .method_limits
            .get(&payment_type)
            .is_some_and(|limit| !limit.available)
        {
            state.error = Some(tr!("pay.method_unavailable"));
            cx.notify();
            return;
        }
        let raw_amount = state.amount.read(cx).content().trim().to_owned();
        let Some(amount) = parse_amount(&raw_amount) else {
            state.error = Some(tr!("pay.enter_valid_amount"));
            cx.notify();
            return;
        };
        let min = config.effective_min(&payment_type);
        let max = config.effective_max(&payment_type);
        if amount < min || amount > max {
            state.error = Some(tr!(
                "pay.amount_range",
                min = format_amount(min),
                max = format_amount(max)
            ));
            cx.notify();
            return;
        }

        state.busy = true;
        state.error = None;
        let epoch = state.epoch;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let created = cx
                .background_executor()
                .spawn(async move { client.create_order(&token, amount, &payment_type) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(state) = this.cloud_pay.as_mut().filter(|state| state.epoch == epoch)
                else {
                    return;
                };
                state.busy = false;
                match created {
                    Ok(order) => {
                        state.order_status = Some(OrderStatus::seed(&order));
                        state.qr = match sub2api::pay::resolve_flow(&order) {
                            PayFlow::Qr => order.qr_code.as_deref().and_then(qr_matrix),
                            _ => None,
                        };
                        let flow = sub2api::pay::resolve_flow(&order);
                        let pay_url = order.pay_url.clone();
                        state.order = Some(order);
                        state.stage = PayStage::Paying;
                        match flow {
                            PayFlow::Redirect => match pay_url {
                                Some(url) => cx.open_url(&url),
                                None => {
                                    state.error = Some(tr!("pay.redirect_url_missing"));
                                }
                            },
                            // A stripe order should not exist here (the form
                            // short-circuits), but a service that returns a
                            // clientSecret anyway still gets a way forward.
                            PayFlow::Stripe | PayFlow::Qr => {}
                        }
                        this.cloud_pay_start_polling(cx);
                    }
                    Err(error) => state.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// One loop per order: ticks every second for the countdown, polls the
    /// order every second tick, gives up after five minutes.
    fn cloud_pay_start_polling(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.cloud_pay.as_ref() else {
            return;
        };
        let epoch = state.epoch;
        let Some((client, _)) = self.pay_client() else {
            return;
        };
        let Some(order) = state.order.clone() else {
            return;
        };

        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let mut tick = 0u64;
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(1))
                    .await;
                tick += 1;

                // The countdown redraw; also verifies the modal still shows
                // this very order.
                let live = this
                    .update(cx, |this, cx| {
                        let live = this
                            .cloud_pay
                            .as_ref()
                            .is_some_and(|state| {
                                state.epoch == epoch && state.stage == PayStage::Paying
                            });
                        if live {
                            cx.notify();
                        }
                        live
                    })
                    .unwrap_or(false);
                if !live {
                    return;
                }
                if tick % (ORDER_POLL_INTERVAL.as_secs().max(1)) != 0 {
                    continue;
                }
                if started.elapsed() > ORDER_POLL_TIMEOUT {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(state) = this
                            .cloud_pay
                            .as_mut()
                            .filter(|state| state.epoch == epoch)
                        {
                            state.stage = PayStage::Result;
                            state.error = Some(tr!("pay.polling_timed_out"));
                            cx.notify();
                        }
                    });
                    return;
                }

                let poll_client = client.clone();
                let order_id = order.order_id.clone();
                let status_token = order.status_access_token.clone();
                let polled = cx
                    .background_executor()
                    .spawn(async move { poll_client.order_status(&order_id, &status_token) })
                    .await;

                let finished = this
                    .update(cx, |this, cx| this.cloud_pay_apply_status(epoch, polled, cx))
                    .unwrap_or(true);
                if finished {
                    return;
                }
            }
        })
        .detach();
    }

    /// Fold a poll answer into the modal. Returns `true` when polling should
    /// stop.
    fn cloud_pay_apply_status(
        &mut self,
        epoch: usize,
        polled: anyhow::Result<OrderStatus>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.cloud_pay.as_mut().filter(|state| state.epoch == epoch) else {
            return true;
        };
        let status = match polled {
            Ok(status) => status,
            Err(error) => {
                // A transient poll failure is worth showing but not worth
                // abandoning the order over.
                state.error = Some(format!("{error:#}"));
                cx.notify();
                return false;
            }
        };
        state.error = None;
        let settled = status.is_settled();
        let failed = status.recharge_status == "failed" || status.is_terminal();
        state.order_status = Some(status);
        if settled {
            self.close_cloud_pay_modal(cx);
            self.show_toast(tr!("pay.success_toast"));
            self.refresh_cloud_account(cx);
            return true;
        }
        if failed {
            state.stage = PayStage::Result;
            cx.notify();
            return true;
        }
        cx.notify();
        false
    }

    /// Cancel the active order — after a last poll, so a payment that landed
    /// in the meantime is honored rather than voided.
    fn cloud_pay_cancel_order(&mut self, cx: &mut Context<Self>) {
        let Some((client, token)) = self.pay_client() else {
            return;
        };
        let Some(state) = self.cloud_pay.as_mut() else {
            return;
        };
        if state.busy {
            return;
        }
        let Some(order) = state.order.clone() else {
            return;
        };
        state.busy = true;
        let epoch = state.epoch;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let latest = client
                        .order_status(&order.order_id, &order.status_access_token)
                        .ok();
                    if let Some(latest) = latest.as_ref() {
                        if latest.payment_success || latest.recharge_success || latest.is_terminal()
                        {
                            return anyhow::Ok(latest.clone());
                        }
                    }
                    client.cancel_order(&token, &order.order_id)?;
                    anyhow::Ok(OrderStatus {
                        id: order.order_id.clone(),
                        status: "CANCELLED".to_owned(),
                        expires_at: order.expires_at.clone(),
                        recharge_status: "closed".to_owned(),
                        ..Default::default()
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let finished = {
                    let Some(state) = this
                        .cloud_pay
                        .as_mut()
                        .filter(|state| state.epoch == epoch)
                    else {
                        return;
                    };
                    state.busy = false;
                    match outcome {
                        Ok(status) => {
                            let settled = status.is_settled();
                            state.order_status = Some(status);
                            if !settled {
                                state.stage = PayStage::Result;
                            }
                            settled
                        }
                        Err(error) => {
                            state.error = Some(format!("{error:#}"));
                            false
                        }
                    }
                };
                if finished {
                    this.close_cloud_pay_modal(cx);
                    this.show_toast(tr!("pay.success_toast"));
                    this.refresh_cloud_account(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_cloud_pay_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.cloud_pay_request {
            self.cloud_pay_request = false;
            self.materialize_cloud_pay(window, cx);
        }
        let state = self.cloud_pay.as_ref()?;
        let theme = Theme::current(cx);
        let chinese = self.state.language.locale().starts_with("zh");

        let mut body = div().flex().flex_col().gap(px(12.0));
        match state.stage {
            PayStage::LoadingConfig => {
                body = body.child(
                    div()
                        .py(px(22.0))
                        .flex()
                        .justify_center()
                        .text_size(sp(12.5))
                        .text_color(theme.text_secondary)
                        .child(tr!("pay.loading_options")),
                );
            }
            PayStage::ConfigError => {
                body = body
                    .child(
                        div()
                            .text_size(sp(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.text)
                            .child(tr!("pay.load_failed")),
                    )
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .line_height(sp(18.0))
                            .text_color(theme.text_secondary)
                            .child(state.error.clone().unwrap_or_default()),
                    )
                    .child(self.pay_secondary_button(
                        "pay-open-center",
                        tr!("pay.open_full_center"),
                        theme,
                        cx,
                        |this, cx| this.cloud_pay_open_full_center(cx),
                    ));
            }
            PayStage::Form => {
                body = self.render_pay_form(body, theme, chinese, cx);
            }
            PayStage::Paying => {
                body = self.render_pay_paying(body, theme, chinese, cx);
            }
            PayStage::Result => {
                body = self.render_pay_result(body, theme, cx);
            }
        }

        let card = div()
            .id("cloud-pay-card")
            .w_full()
            .max_w(px(440.0))
            .max_h(px(640.0))
            .overflow_hidden()
            .rounded(px(18.0))
            .bg(theme.composer)
            .shadow_xl()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
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
                            .child(icon("icons/wallet.svg", 15.0, theme.text))
                            .child(tr!("pay.title")),
                    )
                    .child(
                        div()
                            .id("cloud-pay-close")
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
                                this.close_cloud_pay_modal(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("cloud-pay-body")
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
            .id("cloud-pay-layer")
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
                cx.listener(|this, _, _, cx| this.close_cloud_pay_modal(cx)),
            )
            .child(card);
        Some(gpui::deferred(layer).with_priority(4).into_any_element())
    }

    fn cloud_pay_open_full_center(&mut self, cx: &mut Context<Self>) {
        if let Some((client, token)) = self.pay_client() {
            let url = client.pay_center_url(&token);
            cx.open_url(&url);
            // Payment now happens out of sight; watch the balance so the
            // figure catches up without a restart.
            self.poll_cloud_balance_until_changed(cx);
        }
    }

    /// The form: account, amount, method, and the main call to action.
    fn render_pay_form(
        &self,
        mut body: Div,
        theme: Theme,
        chinese: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(state) = self.cloud_pay.as_ref() else {
            return body;
        };
        let Some(config) = state.config.as_ref() else {
            return body;
        };
        let busy = state.busy;
        let selected_type = state.selected_type.clone();

        // Who is being credited.
        body = body.child(
            div()
                .px(px(14.0))
                .py(px(11.0))
                .rounded(px(11.0))
                .bg(theme.raised)
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(sp(11.0))
                        .text_color(theme.text_ghost)
                        .child(tr!("pay.account")),
                )
                .child(
                    div()
                        .mt(px(2.0))
                        .text_size(sp(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text)
                        .child(if config.user_display_name.is_empty() {
                            tr!("cloud.signed_in")
                        } else {
                            config.user_display_name.clone()
                        }),
                )
                .when_some(config.user_balance, |card, balance| {
                    card.child(
                        div()
                            .mt(px(2.0))
                            .text_size(sp(11.5))
                            .text_color(theme.text_ghost)
                            .child(tr!(
                                "pay.current_balance",
                                balance = format!("${balance:.2}")
                            )),
                    )
                }),
        );

        // Amount: quick chips, the field, and the settlement summary.
        let (min, max, fee_rate) = match selected_type.as_deref() {
            Some(payment_type) => (
                config.effective_min(payment_type),
                config.effective_max(payment_type),
                config.fee_rate(payment_type),
            ),
            None => (config.min_amount, config.max_amount, 0.0),
        };
        let amount_text = state.amount.read(cx).content().trim().to_owned();
        let parsed_amount = parse_amount(&amount_text);

        let mut amount_section = div().flex().flex_col().gap(px(7.0)).child(
            div()
                .text_size(sp(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_secondary)
                .child(tr!("pay.amount")),
        );
        let mut chips = div().flex().flex_wrap().gap(px(6.0));
        for value in QUICK_AMOUNTS {
            if value < min || value > max {
                continue;
            }
            let selected = parsed_amount == Some(value);
            chips = chips.child(
                div()
                    .id(SharedString::from(format!("pay-amount-{value:.0}")))
                    .tab_index(0)
                    .h(px(26.0))
                    .px(px(12.0))
                    .rounded_full()
                    .border_1()
                    .border_color(if selected { theme.accent } else { theme.border_strong })
                    .when(selected, |chip| chip.bg(theme.overlay))
                    .flex()
                    .items_center()
                    .cursor_default()
                    .text_size(sp(12.0))
                    .text_color(if selected { theme.text } else { theme.text_secondary })
                    .child(format!("${value:.0}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(state) = this.cloud_pay.as_ref() {
                            state.amount.update(cx, |input, cx| {
                                input.set_content(format_amount(value), cx)
                            });
                        }
                        cx.notify();
                    })),
            );
        }
        amount_section = amount_section.child(chips).child(
            TextField::new("pay-amount-field", state.amount.clone()).w_full(),
        );

        // The credited figure, the CNY rate, the fee, and the estimated
        // charge — each on its own quiet line, as the Electron form.
        amount_section = amount_section.child(
            div()
                .text_size(sp(11.5))
                .text_color(theme.text_ghost)
                .child(tr!(
                    "pay.credited_range",
                    credited = match parsed_amount {
                        Some(amount) => format!("${amount:.2}"),
                        None => "--".to_owned(),
                    },
                    min = format!("${min:.2}"),
                    max = format!("${max:.2}")
                )),
        );
        if let Some(rate) = config.balance_credit_cny_per_usd {
            amount_section = amount_section.child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_ghost)
                    .child(tr!(
                        "pay.settlement_rate",
                        amount = match parsed_amount {
                            Some(amount) => format!("\u{00a5}{:.2}", amount * rate),
                            None => "--".to_owned(),
                        }
                    )),
            );
        }
        if fee_rate > 0.0 {
            amount_section = amount_section.child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_ghost)
                    .child(tr!("pay.method_fee", rate = format!("{fee_rate}"))),
            );
        }
        if let Some(amount) = parsed_amount {
            let base = config.balance_credit_cny_per_usd.map_or(amount, |rate| amount * rate);
            let estimated = base * (1.0 + fee_rate / 100.0);
            let formatted = if config.balance_credit_cny_per_usd.is_some() {
                format!("\u{00a5}{estimated:.2}")
            } else {
                format!("${estimated:.2}")
            };
            amount_section = amount_section.child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_secondary)
                    .child(tr!("pay.estimated_charge", amount = formatted)),
            );
        }
        body = body.child(amount_section);

        // Payment methods.
        let mut method_section = div().flex().flex_col().gap(px(7.0)).child(
            div()
                .text_size(sp(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.text_secondary)
                .child(tr!("pay.method")),
        );
        if config.enabled_payment_types.is_empty() {
            method_section = method_section.child(
                div()
                    .text_size(sp(11.5))
                    .text_color(theme.text_ghost)
                    .child(tr!("pay.no_methods")),
            );
        }
        let mut method_chips = div().flex().flex_wrap().gap(px(6.0));
        for payment_type in &config.enabled_payment_types {
            let selected = selected_type.as_deref() == Some(payment_type.as_str());
            let label = sub2api::pay::payment_label(payment_type, chinese);
            let chosen = payment_type.clone();
            method_chips = method_chips.child(
                div()
                    .id(SharedString::from(format!("pay-method-{payment_type}")))
                    .tab_index(0)
                    .h(px(28.0))
                    .px(px(13.0))
                    .rounded_full()
                    .border_1()
                    .border_color(if selected { theme.accent } else { theme.border_strong })
                    .when(selected, |chip| chip.bg(theme.overlay))
                    .flex()
                    .items_center()
                    .cursor_default()
                    .text_size(sp(12.0))
                    .text_color(if selected { theme.text } else { theme.text_secondary })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(state) = this.cloud_pay.as_mut() {
                            state.selected_type = Some(chosen.clone());
                            state.error = None;
                        }
                        cx.notify();
                    })),
            );
        }
        method_section = method_section.child(method_chips);
        if let Some(payment_type) = selected_type.as_deref() {
            if config
                .method_limits
                .get(payment_type)
                .is_some_and(|limit| !limit.available)
            {
                method_section = method_section.child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.danger)
                        .child(tr!("pay.method_unavailable")),
                );
            }
            if sub2api::pay::is_stripe(payment_type) {
                method_section = method_section.child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .child(tr!("pay.stripe_hint")),
                );
            }
        }
        body = body.child(method_section);

        // Redeem code — relocated from the account page: crediting a code is
        // a way of topping up, so it belongs on this sheet.
        let redeem_busy = self.cloud_account.busy;
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(7.0))
                .child(
                    div()
                        .text_size(sp(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.text_secondary)
                        .child(tr!("cloud.redeem_title")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            TextField::new("cloud-redeem-field", self.cloud_redeem_input.clone())
                                .flex_1(),
                        )
                        .child(
                            div()
                                .id("cloud-redeem-submit")
                                .tab_index(0)
                                .h(px(28.0))
                                .px(px(11.0))
                                .rounded(px(7.0))
                                .border_1()
                                .border_color(theme.border_strong)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_default()
                                .text_size(sp(12.0))
                                .text_color(theme.text_secondary)
                                .hover(|style| style.bg(theme.overlay))
                                .opacity(if redeem_busy { 0.55 } else { 1.0 })
                                .child(tr!("cloud.redeem_action"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !redeem_busy {
                                        this.redeem_cloud_code(cx);
                                    }
                                })),
                        ),
                ),
        );

        if let Some(error) = &state.error {
            body = body.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }

        // The main call to action.
        let stripe_selected = selected_type
            .as_deref()
            .is_some_and(sub2api::pay::is_stripe);
        let cta_label = if busy {
            tr!("pay.creating_order")
        } else if stripe_selected {
            tr!("pay.open_full_center")
        } else {
            tr!("pay.continue")
        };
        let cta_enabled = !busy && selected_type.is_some();
        body = body.child(
            div()
                .id("pay-continue")
                .tab_index(0)
                .h(px(34.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .bg(theme.inverse)
                .text_color(theme.on_inverse)
                .text_size(sp(12.5))
                .font_weight(FontWeight::MEDIUM)
                .opacity(if cta_enabled { 1.0 } else { 0.55 })
                .child(cta_label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if cta_enabled {
                        this.cloud_pay_create_order(cx);
                    }
                })),
        );
        body
    }

    /// The in-flight order: status, countdown, and the flow's own surface.
    fn render_pay_paying(
        &self,
        mut body: Div,
        theme: Theme,
        chinese: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(state) = self.cloud_pay.as_ref() else {
            return body;
        };
        let Some(order) = state.order.as_ref() else {
            return body;
        };
        let status = state.order_status.clone().unwrap_or_else(|| OrderStatus::seed(order));
        let (title, message, tone) = describe_order_status(&status);
        let tone_color = match tone {
            StatusTone::Success => theme.success,
            StatusTone::Warning => theme.warning,
            StatusTone::Error => theme.danger,
            StatusTone::Default => theme.text,
        };
        let remaining = sub2api::pay::seconds_until(&status.expires_at)
            .map(format_countdown)
            .unwrap_or_else(|| "--".to_owned());

        body = body.child(
            div()
                .px(px(14.0))
                .py(px(12.0))
                .rounded(px(11.0))
                .bg(theme.raised)
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(sp(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(tone_color)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(sp(12.0))
                        .line_height(sp(17.0))
                        .text_color(theme.text_secondary)
                        .child(message),
                )
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .child(tr!(
                            "pay.order_meta",
                            order = order.order_id.clone(),
                            remaining = remaining
                        )),
                )
                .child(
                    div()
                        .text_size(sp(11.5))
                        .text_color(theme.text_ghost)
                        .child(tr!(
                            "pay.amount_meta",
                            pay = match order.pay_amount {
                                Some(pay) => format!("\u{00a5}{pay:.2}"),
                                None => format!("${:.2}", order.amount),
                            },
                            credited = format!("${:.2}", order.amount)
                        )),
                ),
        );

        match sub2api::pay::resolve_flow(order) {
            PayFlow::Qr => {
                let label = sub2api::pay::payment_label(&order.payment_type, chinese);
                let mut qr_card = div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .text_color(theme.text_secondary)
                            .child(tr!("pay.scan_with", method = label)),
                    );
                qr_card = qr_card.child(match state.qr.clone() {
                    Some(matrix) => qr_element(matrix).into_any_element(),
                    None => div()
                        .text_size(sp(12.0))
                        .text_color(theme.text_ghost)
                        .child(tr!("pay.qr_unavailable"))
                        .into_any_element(),
                });
                if order.pay_url.is_some() {
                    qr_card = qr_card.child(self.pay_secondary_button(
                        "pay-open-page",
                        tr!("pay.open_payment_page"),
                        theme,
                        cx,
                        |this, cx| this.cloud_pay_open_pay_url(cx),
                    ));
                }
                body = body.child(qr_card);
            }
            PayFlow::Redirect => {
                body = body
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .line_height(sp(18.0))
                            .text_color(theme.text_secondary)
                            .child(tr!("pay.redirect_hint")),
                    )
                    .child(self.pay_secondary_button(
                        "pay-open-page",
                        tr!("pay.open_payment_page"),
                        theme,
                        cx,
                        |this, cx| this.cloud_pay_open_pay_url(cx),
                    ));
            }
            PayFlow::Stripe => {
                body = body.child(self.pay_secondary_button(
                    "pay-open-center",
                    tr!("pay.open_full_center"),
                    theme,
                    cx,
                    |this, cx| this.cloud_pay_open_full_center(cx),
                ));
            }
        }

        if let Some(error) = &state.error {
            body = body.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.danger)
                    .child(error.clone()),
            );
        }

        let busy = state.busy;
        body = body.child(
            div()
                .id("pay-cancel-order")
                .tab_index(0)
                .h(px(30.0))
                .rounded_full()
                .border_1()
                .border_color(theme.border_strong)
                .flex()
                .items_center()
                .justify_center()
                .cursor_default()
                .text_size(sp(12.0))
                .text_color(theme.text_secondary)
                .opacity(if busy { 0.55 } else { 1.0 })
                .child(if busy {
                    tr!("pay.cancelling")
                } else {
                    tr!("pay.cancel_order")
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !busy {
                        this.cloud_pay_cancel_order(cx);
                    }
                })),
        );
        body
    }

    /// A finished-but-unsettled order: what happened, and the ways back.
    fn render_pay_result(&self, mut body: Div, theme: Theme, cx: &mut Context<Self>) -> Div {
        let Some(state) = self.cloud_pay.as_ref() else {
            return body;
        };
        if let Some(status) = state.order_status.as_ref() {
            let (title, message, tone) = describe_order_status(status);
            let tone_color = match tone {
                StatusTone::Success => theme.success,
                StatusTone::Warning => theme.warning,
                StatusTone::Error => theme.danger,
                StatusTone::Default => theme.text,
            };
            body = body.child(
                div()
                    .px(px(14.0))
                    .py(px(12.0))
                    .rounded(px(11.0))
                    .bg(theme.raised)
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(sp(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(tone_color)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(sp(12.0))
                            .line_height(sp(17.0))
                            .text_color(theme.text_secondary)
                            .child(message),
                    ),
            );
        }
        if let Some(error) = &state.error {
            body = body.child(
                div()
                    .text_size(sp(12.0))
                    .text_color(theme.text_secondary)
                    .child(error.clone()),
            );
        }
        body = body.child(self.pay_secondary_button(
            "pay-back-to-form",
            tr!("pay.back_to_form"),
            theme,
            cx,
            |this, cx| {
                if this.cloud_pay.is_none() {
                    return;
                }
                // A new order gets a new epoch so the old poll loop cannot
                // touch it.
                this.cloud_pay_epoch = this.cloud_pay_epoch.wrapping_add(1);
                let epoch = this.cloud_pay_epoch;
                if let Some(state) = this.cloud_pay.as_mut() {
                    state.order = None;
                    state.order_status = None;
                    state.qr = None;
                    state.error = None;
                    state.epoch = epoch;
                }
                this.cloud_pay_load_config(cx);
            },
        ));
        body
    }

    fn cloud_pay_open_pay_url(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self
            .cloud_pay
            .as_ref()
            .and_then(|state| state.order.as_ref())
            .and_then(|order| order.pay_url.clone())
        {
            cx.open_url(&url);
        }
    }

    fn pay_secondary_button(
        &self,
        id: &'static str,
        label: String,
        theme: Theme,
        cx: &mut Context<Self>,
        activate: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .tab_index(0)
            .h(px(30.0))
            .px(px(13.0))
            .rounded_full()
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .items_center()
            .justify_center()
            .cursor_default()
            .text_size(sp(12.0))
            .text_color(theme.text_secondary)
            .hover(|style| style.bg(theme.overlay))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| activate(this, cx)))
    }
}

/// What tone a status card reads in.
enum StatusTone {
    Default,
    Success,
    Warning,
    Error,
}

/// Port of the Electron client's `describeOrderStatus`.
fn describe_order_status(status: &OrderStatus) -> (String, String, StatusTone) {
    if status.is_settled() {
        return (
            tr!("pay.status_complete_title"),
            tr!("pay.status_complete_message"),
            StatusTone::Success,
        );
    }
    if status.payment_success {
        if status.recharge_status == "paid_pending" || status.recharge_status == "recharging" {
            return (
                tr!("pay.status_received_title"),
                tr!("pay.status_received_message"),
                StatusTone::Success,
            );
        }
        if status.recharge_status == "failed" {
            return (
                tr!("pay.status_unfinished_title"),
                status
                    .failed_reason
                    .clone()
                    .unwrap_or_else(|| tr!("pay.status_unfinished_message")),
                StatusTone::Warning,
            );
        }
    }
    match status.status.to_uppercase().as_str() {
        "PENDING" => (
            tr!("pay.status_pending_title"),
            tr!("pay.status_pending_message"),
            StatusTone::Default,
        ),
        "FAILED" => (
            tr!("pay.status_failed_title"),
            status
                .failed_reason
                .clone()
                .unwrap_or_else(|| tr!("pay.status_failed_message")),
            StatusTone::Error,
        ),
        "CANCELLED" => (
            tr!("pay.status_cancelled_title"),
            tr!("pay.status_cancelled_message"),
            StatusTone::Warning,
        ),
        "EXPIRED" => (
            tr!("pay.status_expired_title"),
            tr!("pay.status_expired_message"),
            StatusTone::Warning,
        ),
        _ => (
            tr!("pay.status_updated_title"),
            tr!("pay.status_updated_message"),
            StatusTone::Default,
        ),
    }
}

/// `754` → `12:34`, the Electron client's countdown format.
fn format_countdown(seconds: i64) -> String {
    if seconds <= 0 {
        return tr!("pay.expired");
    }
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Parse a user-typed USD amount: positive, at most two decimals.
fn parse_amount(raw: &str) -> Option<f64> {
    if raw.is_empty() || !raw.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    if raw.matches('.').count() > 1 {
        return None;
    }
    if let Some((_, decimals)) = raw.split_once('.') {
        if decimals.len() > 2 {
            return None;
        }
    }
    let value: f64 = raw.parse().ok()?;
    (value.is_finite() && value > 0.0).then_some(value)
}

/// A whole amount renders bare (`10`), a fractional one keeps its cents.
fn format_amount(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

/// Encode `data` into a QR bit matrix. `None` when the payload cannot fit,
/// which for payment URLs it always can.
fn qr_matrix(data: &str) -> Option<QrMatrix> {
    let code = qrcode::QrCode::with_error_correction_level(
        data.as_bytes(),
        qrcode::EcLevel::M,
    )
    .ok()?;
    let width = code.width();
    let dark: Vec<bool> = code
        .to_colors()
        .into_iter()
        .map(|color| color == qrcode::Color::Dark)
        .collect();
    Some(Rc::new((width, dark)))
}

/// Paint the matrix as quads on a white card — always dark-on-light, whatever
/// the app theme, because scanners want contrast, and with the standard
/// four-module quiet zone.
fn qr_element(matrix: QrMatrix) -> impl IntoElement {
    div()
        .w(px(240.0))
        .h(px(240.0))
        .rounded(px(12.0))
        .overflow_hidden()
        .bg(gpui::white())
        .child(
            canvas(
                |_, _, _| (),
                move |bounds: Bounds<Pixels>, _, window: &mut Window, _: &mut App| {
                    let (width, dark) = (&matrix.0, &matrix.1);
                    let width = *width;
                    if width == 0 {
                        return;
                    }
                    const QUIET_MODULES: f32 = 4.0;
                    let total = width as f32 + QUIET_MODULES * 2.0;
                    let side = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
                    let module = side / total;
                    let left = f32::from(bounds.origin.x)
                        + (f32::from(bounds.size.width) - side) / 2.0
                        + module * QUIET_MODULES;
                    let top = f32::from(bounds.origin.y)
                        + (f32::from(bounds.size.height) - side) / 2.0
                        + module * QUIET_MODULES;
                    for row in 0..width {
                        for column in 0..width {
                            if !dark[row * width + column] {
                                continue;
                            }
                            // A half-pixel of overlap keeps antialiasing from
                            // drawing hairline seams between modules.
                            let cell = Bounds::new(
                                gpui::point(
                                    px(left + column as f32 * module),
                                    px(top + row as f32 * module),
                                ),
                                gpui::size(px(module + 0.5), px(module + 0.5)),
                            );
                            window.paint_quad(gpui::fill(cell, gpui::black()));
                        }
                    }
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_parse_like_the_electron_pattern() {
        assert_eq!(parse_amount("10"), Some(10.0));
        assert_eq!(parse_amount("10.5"), Some(10.5));
        assert_eq!(parse_amount("10.55"), Some(10.55));
        // More than two decimals, negatives, and junk are rejected.
        assert_eq!(parse_amount("10.555"), None);
        assert_eq!(parse_amount("-5"), None);
        assert_eq!(parse_amount("ten"), None);
        assert_eq!(parse_amount(""), None);
        assert_eq!(parse_amount("0"), None);
        assert_eq!(parse_amount("1.2.3"), None);
    }

    #[test]
    fn amount_formatting_drops_empty_cents() {
        assert_eq!(format_amount(10.0), "10");
        assert_eq!(format_amount(10.5), "10.50");
    }

    #[test]
    fn qr_matrix_is_square_and_nonempty() {
        let matrix = qr_matrix("https://example.org/pay/weixin://wxpay/abc").expect("qr");
        assert!(matrix.0 >= 21); // Version 1 is 21×21; payment URLs need more.
        assert_eq!(matrix.1.len(), matrix.0 * matrix.0);
        assert!(matrix.1.iter().any(|dark| *dark));
        assert!(matrix.1.iter().any(|dark| !*dark));
    }
}
