package cheapauth

import (
	"fmt"
	"strings"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// PaymentSummary projects the creation receipt without putting the order's
// status token or provider client secret into the dialog.
func PaymentSummary(c *ui.Context, o core.PayOrder, p *core.SubscriptionPlan) string {
	amount := o.Amount
	if o.PayAmount != nil {
		amount = *o.PayAmount
	}
	if p != nil {
		return i18n.T(c, "pay_plan_order_meta", "pay", cny(amount), "plan", p.Name)
	}
	pay := usd(amount)
	if o.PayAmount != nil {
		pay = cny(amount)
	}
	if o.Bonus() > 0 {
		return i18n.T(c, "pay_amount_meta_bonus", "pay", pay, "credited", usd(o.CreditedTotal()), "bonus", usd(o.Bonus()))
	}
	return i18n.T(c, "pay_amount_meta", "pay", pay, "credited", usd(o.Amount))
}

func orderClosed(d *RechargeDialog) bool {
	if d == nil {
		return true
	}
	status := d.Status
	return d.Terminal || status != nil && (status.IsTerminal() || status.PaymentSuccess || status.RechargeSuccess || status.RechargeStatus == "failed")
}

func orderCountdownSeconds(d *RechargeDialog, now time.Time) (int64, bool) {
	if d == nil || orderClosed(d) {
		return 0, false
	}
	if d.Status != nil {
		if seconds, ok := core.SecondsUntil(d.Status.ExpiresAt, now); ok {
			return seconds, true
		}
	}
	if !d.Deadline.IsZero() {
		return max(0, d.Deadline.Unix()-now.Unix()), true
	}
	return 0, false
}

func orderView(c *ui.Context, d *RechargeDialog) {
	p := theme.Of(c)
	status := d.Status
	closed := orderClosed(d)

	ui.Column(c.Key("order-view")).FillWidth().Gap(12).Children(func() {
		ui.Row(c.Key("order-header")).FillWidth().AlignItems(ui.Center).Justify(ui.SpaceBetween).Gap(8).Children(func() {
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
				if status != nil {
					heading, detail := paymentStatusKeys(*status, d.Plan != nil)
					ui.Text(c, i18n.T(c, heading)).FontSize(14).FontWeight(600).TextColor(p.PopoverForeground)
					message := i18n.T(c, detail)
					if status.FailedReason != "" && (heading == "pay_status_failed_title" || heading == "pay_status_unfinished_title") {
						message = status.FailedReason
					}
					ui.Text(c, message).FontSize(12).TextColor(p.MutedForeground).Selectable()
				}
			})
			if seconds, ok := orderCountdownSeconds(d, time.Now()); ok {
				if seconds == 0 {
					kit.Badge(c, i18n.T(c, "pay_expired"), kit.Negative)
				} else {
					rem := fmt.Sprintf("%d:%02d", seconds/60, seconds%60)
					tone := kit.Neutral
					if seconds <= 60 {
						tone = kit.Caution
					}
					kit.Badge(c, i18n.T(c, "pay_countdown_remaining", "remaining", rem), tone)
				}
			}
		})

		ui.Column(c.Key("order-meta")).FillWidth().Padding(8, 10).Radius(p.Radius).Background(p.Muted).Gap(4).Children(func() {
			if d.OrderSummary != "" {
				ui.Text(c, d.OrderSummary).FontSize(13).FontWeight(500).TextColor(p.PopoverForeground).Selectable()
			}
			ui.Row(c.Key("order-id-row")).FillWidth().AlignItems(ui.Center).Justify(ui.SpaceBetween).Gap(8).Children(func() {
				ui.Text(c, i18n.T(c, "recharge_order", "id", d.OrderID)).FontSize(12).TextColor(p.MutedForeground).Shrink(1).MinWidth(0).Selectable()
				kit.Ghost(c.Key("copy-order"), i18n.T(c, "pay_copy_order"), false).Height(24).PaddingX(8).OnClick(func() {
					c.WriteClipboard(d.OrderID)
					kit.Copied(c)
				})
			})
		})

		if closed {
			return
		}

		if d.QR != nil {
			ui.Column(c.Key("qr-box")).FillWidth().AlignItems(ui.Center).Gap(6).Padding(12).Radius(p.Radius).Background(p.Muted).Children(func() {
				d.qrImage.show(c.Key("checkout-qr"), d.QR, 200, 200, true)
				ui.Text(c, i18n.T(c, "recharge_scan")).FontSize(12).TextColor(p.MutedForeground)
			})
		}

		if d.PayURL != "" {
			ui.Row(c.Key("checkout-links")).FillWidth().Gap(8).Wrap().Children(func() {
				kit.Secondary(c.Key("checkout"), i18n.T(c, "recharge_checkout")).OnClick(func() { c.OpenURL(d.PayURL) })
				copyValue(c, "copy-url", "pay_copy_url", d.PayURL)
			})
		}

		if d.CheckoutData != "" && d.CheckoutData != d.PayURL {
			if d.QR == nil {
				ui.Text(c, d.CheckoutData).FontSize(12).TextColor(p.MutedForeground).Selectable()
			}
			copyValue(c, "copy-data", "pay_copy_data", d.CheckoutData)
		}
	})
}

func orderFooter(c *ui.Context, d *RechargeDialog) {
	ui.Row(c.Key("order-footer")).FillWidth().Gap(8).AlignItems(ui.Center).Children(func() {
		if d.CancelOrder != nil {
			label := i18n.T(c, "pay_cancel_order")
			kit.Secondary(c.Key("cancel-order"), label).Disabled(d.Busy || d.CheckingStatus).OnClick(func() { d.confirmCancel = true })
			if kit.Confirm(c.Key("confirm-order"), &d.confirmCancel, kit.Confirmation{
				Title:       label,
				Message:     i18n.T(c, "pay_cancel_consequence", "id", d.OrderID),
				Buttons:     []string{i18n.T(c, "pay_keep_order"), label},
				Cancel:      0,
				Default:     0,
				Destructive: []int{1},
			}) == 1 && !d.Busy {
				d.CancelOrder()
			}
		}
		if d.CheckStatus != nil {
			label := i18n.T(c, "pay_completed_check")
			if d.CheckingStatus {
				label = i18n.T(c, "pay_checking_status")
			}
			kit.PrimaryBusy(c.Key("check-order"), label, d.Busy || d.CheckingStatus, d.CheckingStatus).Grow(1).OnClick(d.CheckStatus)
		}
	})
}

func copyValue(c *ui.Context, key, label, value string) {
	kit.Secondary(c.Key(key), i18n.T(c, label)).OnClick(func() { c.WriteClipboard(value); kit.Copied(c) })
}

func paymentStatusKeys(s core.OrderStatus, plan bool) (string, string) {
	if s.IsSettled() {
		if plan {
			return "pay_status_complete_title", "pay_plan_status_complete_message"
		}
		return "pay_status_complete_title", "pay_status_complete_message"
	}
	if s.PaymentSuccess {
		switch s.RechargeStatus {
		case "paid_pending", "recharging":
			if plan {
				return "pay_status_received_title", "pay_plan_status_received_message"
			}
			return "pay_status_received_title", "pay_status_received_message"
		case "failed":
			return "pay_status_unfinished_title", "pay_status_unfinished_message"
		}
	}
	switch strings.ToUpper(s.Status) {
	case "PENDING":
		return "pay_status_pending_title", "pay_status_pending_message"
	case "FAILED":
		return "pay_status_failed_title", "pay_status_failed_message"
	case "EXPIRED":
		return "pay_status_expired_title", "pay_status_expired_message"
	case "CANCELLED":
		return "pay_status_cancelled_title", "pay_status_cancelled_message"
	default:
		return "pay_status_updated_title", "pay_status_updated_message"
	}
}
