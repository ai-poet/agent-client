package cheapauth

import (
	"image"
	"slices"
	"testing"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/egoist/mygo/ui"
)

func TestRechargeDraftAndOrder(t *testing.T) {
	state := &State{Stage: GatewayFailed, AccountFailure: "Gateway unreachable"}
	d := &RechargeDialog{Config: &core.PayConfig{MinAmount: 5, MaxAmount: 50, MaxPendingOrders: 3, MaxDailyAmount: 100,
		EnabledPaymentTypes: []string{"wxpay", "stripe_card", "alipay"}}}
	state.OpenRecharge = func() { state.Recharge = d }
	d.Close = func() { state.Recharge = nil }
	calls := 0
	d.Submit = func(amount float64, method string) {
		if amount != 12 || method != "alipay" {
			t.Fatalf("unexpected order: %g %s", amount, method)
		}
		calls++
		d.Busy = true
	}
	tt := accountTester(state)
	tt.SetLocale("en")
	loginClick(t, tt, "Add funds")
	tt.Frame()
	for _, label := range []string{"Credits -- · range $5.00 – $50.00", "Max daily credited balance $100.00"} {
		if !tt.HasText(label) {
			t.Errorf("missing limit %q", label)
		}
	}
	first := loginControl(t, tt, ui.RoleToggleButton, "WeChat Pay")
	stripe := loginControl(t, tt, ui.RoleToggleButton, "Stripe")
	last := loginControl(t, tt, ui.RoleToggleButton, "Alipay")
	if stripe.Disabled || first.Bounds.X >= stripe.Bounds.X || stripe.Bounds.X >= last.Bounds.X {
		t.Fatal("method order or Stripe availability differs from configuration")
	}
	loginClick(t, tt, "Alipay")
	for _, amount := range []string{"", "x", "NaN", "4", "51"} {
		d.Amount = amount
		tt.Frame()
		loginClick(t, tt, "Continue to payment")
		tt.Frame()
		if calls != 0 || (d.Message != "Enter a valid amount" && d.Message != "Amount must be between $5 and $50") {
			t.Fatalf("invalid amount %q: %q", amount, d.Message)
		}
	}
	d.Amount = ""
	tt.Frame()
	field := loginControl(t, tt, ui.RoleTextField, "Amount (USD)").Bounds
	tt.ClickAt(field.X+field.W/2, field.Y+field.H/2)
	tt.Type("12")
	if !tt.HasText("Estimated charge $12.00") {
		t.Fatal("estimate did not follow the entered amount")
	}
	loginClick(t, tt, "Continue to payment")
	tt.Frame()
	if calls != 1 || !loginControl(t, tt, ui.RoleButton, "Creating order…").Disabled {
		t.Fatal("order creation did not lock the draft")
	}
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if state.Recharge != d {
		t.Fatal("creation dismissed by Escape")
	}
	d.Busy, d.OrderID, d.PayURL = false, "order-7", "https://example.com/checkout"
	d.QR = image.NewNRGBA(image.Rect(0, 0, 32, 32))
	d.Message = "Awaiting payment."
	tt.Frame()
	if !tt.HasText("Order: order-7") || !tt.HasText(d.Message) || !tt.HasText("Scan to pay") || tt.HasText("Continue to payment") {
		t.Fatal("payment view does not identify the existing order")
	}
	loginClick(t, tt, "Open payment page")
	if !slices.Equal(tt.OpenedURLs(), []string{d.PayURL}) || calls != 1 {
		t.Fatal("reopening checkout did not reuse the admitted order")
	}
	loginClick(t, tt, "Close")
	if state.Recharge != nil {
		t.Fatal("order dialog did not close")
	}
}

func TestRechargeUnavailableAndRetry(t *testing.T) {
	d := &RechargeDialog{Message: "Payment options unavailable"}
	state := &State{Stage: Ready, Recharge: d}
	tt := accountTester(state)
	tt.SetLocale("en")
	if !tt.HasText(d.Message) || tt.HasText("Retry") || tt.HasText("Continue to payment") {
		t.Fatal("failed configuration without recovery showed a payment action")
	}
	d.Retry = func() {
		d.Config = &core.PayConfig{}
		d.Submit = func(float64, string) { t.Fatal("unexpected submission") }
		d.Message, d.Retry = "", nil
	}
	tt.Frame()
	loginClick(t, tt, "Retry")
	tt.Frame()
	if !tt.HasText("No payment methods are available.") || !loginControl(t, tt, ui.RoleButton, "Continue to payment").Disabled {
		t.Fatal("empty payment-method list permits payment or lacks an explanation")
	}
}

func TestMethodAdmissionAndConfirmation(t *testing.T) {
	remaining := 15.
	d := &RechargeDialog{Amount: "10", Method: "usdt.plasma", Config: &core.PayConfig{
		EnabledPaymentTypes: []string{"stripe_card", "usdt.plasma", "alipay"},
		MaxPendingOrders:    3, MaxAmount: 8, MaxDailyAmount: 5,
		MethodLimits: map[string]core.MethodLimit{"usdt.plasma": {Available: true, SingleMin: 5, SingleMax: 20, Remaining: &remaining}},
	}}
	var admitted []string
	d.Submit = func(_ float64, m string) { admitted = append(admitted, m) }
	d.OpenCenter = func() { admitted = append(admitted, "hosted") }
	for _, bad := range []string{"3", "21", "1e1", "+10", "10.000"} {
		d.Amount = bad
		d.proceed("en", false)
		if d.stableNotice || len(admitted) != 0 {
			t.Fatalf("invalid amount admitted: %q", bad)
		}
	}
	d.Amount = "16"
	d.proceed("en", false)
	if !d.stableNotice || len(admitted) != 0 {
		t.Fatal("stablecoin order skipped instructions")
	}
	d.proceed("en", true)
	d.Method = "stripe_card"
	d.proceed("en", false)
	if !slices.Equal(admitted, []string{"usdt.plasma", "hosted"}) {
		t.Fatal(admitted)
	}
	d.Plan = &core.SubscriptionPlan{ID: "m1", Price: 29.9}
	d.Config.BalanceDisabled = true
	d.Amount, d.Method = "", "usdt.plasma"
	if _, key := d.submission(); key == "" {
		t.Fatal("plan admitted with a stablecoin")
	}
	d.Method = "alipay"
	if n, key := d.submission(); key != "" || n != 29.9 {
		t.Fatal("plan used balance constraints instead of its price")
	}
	d.Config.MaxPendingOrders, d.Config.PendingCount = 0, 0
	if _, key := d.submission(); key == "" {
		t.Fatal("pending order cap ignored")
	}
}

func TestClosedOrPaidOrderNeverOffersCheckout(t *testing.T) {
	d := &RechargeDialog{OrderID: "id-9", PayURL: "https://example.com/pay", CancelOrder: func() { t.Fatal("unexpected cancel") }}
	tt := accountTester(&State{Stage: Ready, Recharge: d})
	tt.SetLocale("en")
	for _, s := range []core.OrderStatus{{Status: "EXPIRED"}, {Status: "CANCELLED"}, {Status: "FAILED"}, {Status: "PAID", PaymentSuccess: true}, {Status: "COMPLETED", RechargeSuccess: true}} {
		d.Status = &s
		tt.Frame()
		if tt.HasText("Open payment page") || tt.HasText("Cancel order") {
			t.Errorf("%s order can still be paid or cancelled", s.Status)
		}
	}
}

func TestPromotionBannerUsesOnlyEarnableTiersForEstimate(t *testing.T) {
	cap := 3.
	d := &RechargeDialog{Amount: "20", Method: "alipay", Config: &core.PayConfig{
		MinAmount: 1, MaxAmount: 50, MaxPendingOrders: 2, EnabledPaymentTypes: []string{"alipay"},
		Promotions: []core.Promotion{
			{Name: "Member offer", Available: true, MinAmount: 10, BonusType: core.BonusPercent, BonusValue: 20, MaxBonus: &cap},
			{Name: "Used offer", MinAmount: 10, BonusType: core.BonusFixed, BonusValue: 30},
		},
	}}
	tt := accountTester(&State{Stage: Ready, Recharge: d})
	tt.SetLocale("en")
	for _, label := range []string{"Top up $10+ and get 20% bonus (up to $3)", "Top up $10+ and get $30 bonus (unavailable)", "Promotion bonus (Member offer) +$3.00", "Total credited (USD) $23.00"} {
		if !tt.HasText(label) {
			t.Errorf("missing promotion text %q", label)
		}
	}
}

func TestPaymentPageUnmountDetachesButDoesNotCancelOrder(t *testing.T) {
	s := &State{Stage: Ready}
	closeCalls := 0
	s.Recharge = &RechargeDialog{OrderID: "id-1", Close: func() { closeCalls++; s.Recharge = nil }, CancelOrder: func() { t.Fatal("unmount cancelled order") }}
	tt := accountTester(s)
	tt.Frame()
	s.Stage = SignedOut
	tt.Frame()
	if s.Recharge != nil || closeCalls != 1 {
		t.Fatal("unmounted account kept an order observer")
	}
}
