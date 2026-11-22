package cheapauth

import (
	"image"
	"slices"
	"testing"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/egoist/mygo/ui"
)

func TestRechargeDraftAndOrder(t *testing.T) {
	state := &State{Stage: GatewayFailed, AccountFailure: "Gateway unreachable"}
	rem := 30.0
	d := &RechargeDialog{Config: &core.PayConfig{
		MinAmount: 5, MaxAmount: 50, MaxPendingOrders: 3, MaxDailyAmount: 100,
		EnabledPaymentTypes: []string{"wxpay", "stripe_card", "alipay"},
		MethodLimits: map[string]core.MethodLimit{
			"wxpay":  {Available: true, SingleMin: 5, SingleMax: 50, Remaining: &rem},
			"alipay": {Available: true, SingleMin: 5, SingleMax: 50, FeeRate: 2},
		},
	}}
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
	checks := 0
	d.CheckStatus = func() { checks++ }
	tt := accountTester(state)
	tt.SetLocale("en")
	loginClick(t, tt, "Add funds")
	tt.Frame()

	dialog := loginControl(t, tt, ui.RoleDialog, "Add funds")
	if dialog.Bounds.W < 360 || dialog.Bounds.W > 420 {
		t.Fatalf("recharge dialog width = %v, want 360..420", dialog.Bounds.W)
	}
	for _, label := range []string{"Estimated credit --", "$5–$50 · Left $30", "Fee 2% · $5–$50", "Cancel"} {
		if !tt.HasText(label) {
			t.Errorf("missing expected form text %q; texts=%q", label, tt.Texts())
		}
	}
	first := loginControl(t, tt, ui.RoleToggleButton, "WeChat Pay")
	stripe := loginControl(t, tt, ui.RoleToggleButton, "Stripe")
	last := loginControl(t, tt, ui.RoleToggleButton, "Alipay")
	if !first.Checked {
		t.Fatal("first available payment method was not selected by default")
	}
	if stripe.Disabled || first.Bounds.X >= stripe.Bounds.X || stripe.Bounds.X >= last.Bounds.X {
		t.Fatal("method order or Stripe availability differs from configuration")
	}
	loginClick(t, tt, "Alipay")
	for _, amount := range []string{"", "x", "NaN", "4", "51"} {
		d.Amount = amount
		tt.Frame()
		cta := "Continue to payment"
		if amount == "4" {
			cta = "Pay $4.08"
		} else if amount == "51" {
			cta = "Pay $52.02"
		}
		loginClick(t, tt, cta)
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
	if !tt.HasText("Estimated credit $12.00") || !tt.HasText("Pay $12.24") {
		t.Fatalf("summary or pay CTA did not follow entered amount and fee: %q", tt.Texts())
	}
	loginClick(t, tt, "Pay $12.24")
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
	d.Status = &core.OrderStatus{ID: "order-7", Status: "PENDING", ExpiresAt: time.Now().Add(3 * time.Minute).UTC().Format(time.RFC3339)}
	d.QR = image.NewNRGBA(image.Rect(0, 0, 32, 32))
	d.Message = "Awaiting payment."
	tt.Frame()
	if !tt.HasText("Order: order-7") || !tt.HasText(d.Message) || !tt.HasText("Scan to pay") || !tt.HasText("I've completed payment") || tt.HasText("Pay $12.24") {
		t.Fatalf("payment view does not identify the existing order: %q", tt.Texts())
	}
	loginClick(t, tt, "I've completed payment")
	if checks != 1 {
		t.Fatalf("CheckStatus calls = %d, want 1", checks)
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

func TestRechargeCancelButtonClosesForm(t *testing.T) {
	state := &State{Stage: Ready}
	d := &RechargeDialog{
		Amount: "10",
		Config: &core.PayConfig{
			MinAmount: 1, MaxAmount: 100, MaxPendingOrders: 2,
			EnabledPaymentTypes: []string{"wxpay", "alipay"},
			MethodLimits: map[string]core.MethodLimit{
				"wxpay":  {Available: false},
				"alipay": {Available: true, SingleMin: 1, SingleMax: 100},
			},
		},
		Submit: func(float64, string) {},
	}
	d.Close = func() { state.Recharge = nil }
	state.Recharge = d
	tt := accountTester(state)
	tt.SetLocale("en")
	tt.Frame()
	if d.Method != "alipay" {
		t.Fatalf("default method = %q, want alipay (skipping unavailable wxpay)", d.Method)
	}
	if !loginControl(t, tt, ui.RoleToggleButton, "WeChat Pay").Disabled {
		t.Fatal("unavailable payment method chip should be disabled")
	}
	if !tt.HasText("Unavailable") {
		t.Fatalf("unavailable chip missing label: %q", tt.Texts())
	}
	loginClick(t, tt, "Cancel")
	tt.Frame()
	if state.Recharge != nil {
		t.Fatal("Cancel button did not close recharge dialog")
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
	d := &RechargeDialog{
		OrderID:     "id-9",
		PayURL:      "https://example.com/pay",
		CancelOrder: func() { t.Fatal("unexpected cancel") },
		CheckStatus: func() { t.Fatal("unexpected check") },
	}
	tt := accountTester(&State{Stage: Ready, Recharge: d})
	tt.SetLocale("en")
	for _, s := range []core.OrderStatus{{Status: "EXPIRED"}, {Status: "CANCELLED"}, {Status: "FAILED"}, {Status: "PAID", PaymentSuccess: true}, {Status: "COMPLETED", RechargeSuccess: true}} {
		d.Status = &s
		tt.Frame()
		if tt.HasText("Open payment page") || tt.HasText("Cancel order") || tt.HasText("I've completed payment") {
			t.Errorf("%s order can still be paid, checked or cancelled", s.Status)
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
	tt.Frame()

	// Single-line summary shows earnable total + bonus; promotion rules stay collapsed by default.
	if !tt.HasText("Estimated credit $23.00 (incl. $3.00 bonus)") {
		t.Fatalf("missing estimated credit bonus summary: %q", tt.Texts())
	}
	if !tt.HasText("More rules") {
		t.Fatalf("missing collapsed promo rules toggle: %q", tt.Texts())
	}
	if tt.HasText("Top up $10+ and get 20% bonus (up to $3)") {
		t.Fatalf("promotion rules should be collapsed by default: %q", tt.Texts())
	}

	// Expanding shows all promotion rules including unavailable ones.
	loginClick(t, tt, "More rules")
	tt.Frame()
	for _, label := range []string{"Hide rules", "Top up $10+ and get 20% bonus (up to $3)", "Top up $10+ and get $30 bonus (unavailable)"} {
		if !tt.HasText(label) {
			t.Errorf("missing expanded promotion text %q: %q", label, tt.Texts())
		}
	}

	// Collapsing hides them again.
	loginClick(t, tt, "Hide rules")
	tt.Frame()
	if tt.HasText("Top up $10+ and get 20% bonus (up to $3)") {
		t.Fatalf("promotion rules did not collapse: %q", tt.Texts())
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
