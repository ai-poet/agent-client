package live

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	accountui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
)

// Drive the real account window over a local gateway and pay service. The
// second order settles during cancellation: no cancellation POST is allowed.
func TestRechargeWindowOrderLifecycle(t *testing.T) {
	gateway := newCheapGateway(t)
	var creates, cancels, polls atomic.Int32
	var credited atomic.Bool
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/pay/api/orders/my":
			fmt.Fprint(w, `{"user":{"id":7,"balance":12.5},"summary":{"pending":0}}`)
		case "/pay/api/user":
			fmt.Fprint(w, `{"config":{"enabledPaymentTypes":["alipay","wxpay"],"minAmount":1,"maxAmount":1000,"maxPendingOrders":3,"methodLimits":{"alipay":{"available":false},"wxpay":{"available":true}}}}`)
		case "/pay/api/subscription-plans":
			fmt.Fprint(w, `{"plans":[{"id":"monthly","groupId":2,"name":"Monthly","price":25,"validityDays":1,"validityUnit":"month"}]}`)
		case "/api/v1/subscriptions/progress":
			if creates.Load() >= 3 {
				fmt.Fprint(w, `{"code":0,"data":[{"subscription":{"group_id":2,"status":"active","expires_at":"2027-01-01T00:00:00Z"}}]}`)
			} else {
				fmt.Fprint(w, `{"code":0,"data":[]}`)
			}
		case "/pay/api/orders":
			n := creates.Add(1)
			fmt.Fprintf(w, `{"orderId":"order-%d","statusAccessToken":"status-only","status":"PENDING","paymentType":"wxpay","qrCode":"weixin://fixture","amount":10}`, n)
		case "/pay/api/orders/order-1", "/pay/api/orders/order-2", "/pay/api/orders/order-3":
			if r.Header.Get("Authorization") != "" || r.URL.Query().Get("access_token") != "status-only" {
				t.Error("poll did not isolate order capability")
			}
			id := r.URL.Path[len("/pay/api/orders/"):]
			polls.Add(1)
			if id == "order-2" && credited.Load() || id == "order-3" {
				fmt.Fprintf(w, `{"id":%q,"status":"COMPLETED"}`, id)
			} else {
				fmt.Fprintf(w, `{"id":%q,"status":"PENDING"}`, id)
			}
		case "/pay/api/orders/order-1/cancel", "/pay/api/orders/order-2/cancel":
			cancels.Add(1)
			fmt.Fprint(w, `{}`)
		case "/api/v1/auth/me":
			if credited.Load() {
				fmt.Fprint(w, `{"code":0,"data":{"id":7,"username":"Dev","balance":22.5}}`)
				return
			}
			gateway.serve(w, r)
		default:
			gateway.serve(w, r)
		}
	}))
	t.Cleanup(server.Close)
	d := openCheapDesktop(t, func(d *cheapDesktop) { d.signedIn(server.URL, cheapTestAccess, time.Hour) })
	d.await("signed-in account", func() bool { return d.account().Stage == accountui.Ready })
	d.accountPage()
	tr := func(key string) string { return i18n.In("zh-CN", key) }
	buy := func() *accountui.RechargeDialog {
		d.click(tr("recharge_open"))
		d.await("payment form", func() bool { return d.account().Recharge != nil && d.account().Recharge.Config != nil })
		if got := d.account().Recharge.Method; got != "wxpay" {
			t.Fatalf("default method = %q, want wxpay (skipping unavailable alipay)", got)
		}
		d.click(i18n.In("zh-CN", "pay_plan_pay_cta", "amount", "$10.00"))
		d.await("order QR", func() bool { return d.account().Recharge.QR != nil })
		return d.account().Recharge
	}
	order := buy()
	if len(d.tester.OpenedURLs()) != 0 {
		t.Fatalf("weixin:// QR payload was opened as browser URL: %v", d.tester.OpenedURLs())
	}
	d.click(tr("pay_completed_check"))
	d.await("not-paid feedback after manual check", func() bool {
		return !order.CheckingStatus && order.Message == tr("pay_not_paid_yet")
	})
	d.await("automatic order poll", func() bool { return polls.Load() > 1 })
	order.CancelOrder()
	d.await("cancelled status", func() bool { return order.Terminal })
	if order.Status.Status != "CANCELLED" || cancels.Load() != 1 {
		t.Fatal("pending order did not cancel")
	}
	order.Close()
	d.tester.Frame()
	order = buy()
	credited.Store(true)
	order.CancelOrder()
	d.await("credited balance", func() bool { return d.account().Recharge == nil && d.account().Account.Balance == 22.5 })
	if cancels.Load() != 1 || creates.Load() != 2 {
		t.Fatal("settled order cancelled or purchase repeated")
	}
	d.account().OpenPlans()
	d.await("plan choices", func() bool { return d.account().Recharge.Listing && len(d.account().Recharge.Plans) == 1 })
	list := d.account().Recharge
	list.ChoosePlan(list.Plans[0])
	d.await("plan checkout", func() bool { return d.account().Recharge.Plan != nil && d.account().Recharge.Config != nil })
	d.click(i18n.In("zh-CN", "pay_plan_pay_cta", "amount", "\u00a525.00"))
	d.await("plan activated and subscription refreshed", func() bool {
		return d.account().Recharge.Activated && d.account().Recharge.OwnedGroups[2] && d.account().Recharge.Expiry != ""
	})
}
