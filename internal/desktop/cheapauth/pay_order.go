package cheapauth

import (
	"fmt"
	"strings"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
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

func orderView(c *ui.Context, d *RechargeDialog) {
	status := d.Status
	title := i18n.T(c, "recharge_order", "id", d.OrderID)
	if status != nil {
		if seconds, ok := core.SecondsUntil(status.ExpiresAt, time.Now()); ok && !d.Terminal && !status.IsTerminal() {
			remaining := fmt.Sprintf("%d:%02d", seconds/60, seconds%60)
			if seconds == 0 {
				remaining = i18n.T(c, "pay_expired")
			}
			title = i18n.T(c, "pay_order_meta", "order", d.OrderID, "remaining", remaining)
		}
		heading, detail := paymentStatusKeys(*status, d.Plan != nil)
		ui.Text(c, i18n.T(c, heading)).FontWeight(600)
		message := i18n.T(c, detail)
		if status.FailedReason != "" && (heading == "pay_status_failed_title" || heading == "pay_status_unfinished_title") {
			message = status.FailedReason
		}
		ui.Text(c, message).Selectable()
	}
	ui.Text(c, title).Selectable()
	if d.OrderSummary != "" {
		ui.Text(c, d.OrderSummary).Selectable()
	}
	copyValue(c, "copy-order", "pay_copy_order", d.OrderID)
	if d.Terminal || status != nil && (status.IsTerminal() || status.PaymentSuccess || status.RechargeSuccess || status.RechargeStatus == "failed") {
		return
	}
	if d.QR != nil {
		ui.Text(c, i18n.T(c, "recharge_scan"))
		d.qrImage.show(c.Key("checkout-qr"), d.QR, 224, 224, true)
	}
	if d.PayURL != "" {
		kit.Secondary(c.Key("checkout"), i18n.T(c, "recharge_checkout")).OnClick(func() { c.OpenURL(d.PayURL) })
		copyValue(c, "copy-url", "pay_copy_url", d.PayURL)
	}
	if d.CheckoutData != "" && d.CheckoutData != d.PayURL {
		ui.Text(c, d.CheckoutData).Selectable()
		copyValue(c, "copy-data", "pay_copy_data", d.CheckoutData)
	}
	if d.CancelOrder != nil {
		label := i18n.T(c, "pay_cancel_order")
		kit.Secondary(c.Key("cancel-order"), label).Disabled(d.Busy).OnClick(func() { d.confirmCancel = true })
		if kit.Confirm(c.Key("confirm-order"), &d.confirmCancel, kit.Confirmation{Title: label, Message: i18n.T(c, "pay_cancel_consequence", "id", d.OrderID), Buttons: []string{i18n.T(c, "pay_keep_order"), label}, Cancel: 0, Default: 0, Destructive: []int{1}}) == 1 && !d.Busy {
			d.CancelOrder()
		}
	}
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
