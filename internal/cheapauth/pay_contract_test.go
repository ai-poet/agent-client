package cheapauth

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestPaymentWireModels(t *testing.T) {
	var order PayOrder
	for _, tc := range []struct {
		raw  string
		flow PayFlow
	}{
		{`{"qrCode":"scan","clientSecret":""}`, PayFlowStripe},
		{`{"qrCode":"scan","clientSecret":null}`, PayFlowQR},
		{`{"paymentType":"wxpay","qrCode":"scan","payUrl":"/hosted"}`, PayFlowRedirect},
	} {
		order = PayOrder{}
		if err := json.Unmarshal([]byte(tc.raw), &order); err != nil || ResolveFlow(order) != tc.flow {
			t.Errorf("%s: %v", tc.raw, err)
		}
	}
	var promotion Promotion
	var limit MethodLimit
	if err := json.Unmarshal([]byte(`{"bonusType":"future"}`), &promotion); err != nil || !promotion.Available || promotion.BonusType != BonusFixed {
		t.Fatal("promotion default", err)
	}
	if err := json.Unmarshal([]byte(`{}`), &limit); err != nil || !limit.Available {
		t.Fatal("limit default", err)
	}
	for _, tc := range []struct {
		model  any
		fields string
	}{
		{order, "orderId payUrl qrCode clientSecret statusAccessToken"}, {OrderStatus{}, "id expiresAt paymentSuccess rechargeSuccess rechargeStatus failedReason"},
		{SubscriptionPlan{}, "id groupId supportedModelScopes validityUnit"}, {promotion, "id bonusType minAmount sortOrder endsAt"},
		{limit, "available singleMin singleMax feeRate"},
	} {
		b, err := json.Marshal(tc.model)
		if err != nil {
			t.Fatal(err)
		}
		var obj map[string]any
		if json.Unmarshal(b, &obj) != nil {
			t.Fatal("bad JSON")
		}
		for _, k := range strings.Fields(tc.fields) {
			if _, ok := obj[k]; !ok {
				t.Errorf("missing wire field %s", k)
			}
		}
	}
}

func TestOpaqueOrderIDsAndBareLanguage(t *testing.T) {
	const id = "order/+ ?#%2F"
	const encoded = "order%2F%2B%20%3F%23%252F"
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("lang") != "pt" || r.Header.Get("Accept-Language") != "pt" {
			t.Error("lost language")
		}
		switch r.Method {
		case "GET":
			if r.URL.EscapedPath() != "/pay/api/orders/"+encoded {
				t.Error(r.URL.EscapedPath())
			}
			json.NewEncoder(w).Encode(OrderStatus{ID: id, Status: "PAID"})
		case "POST":
			if r.URL.EscapedPath() != "/pay/api/orders/"+encoded+"/cancel" {
				t.Error(r.URL.EscapedPath())
			}
			io.WriteString(w, `{}`)
		}
	}))
	defer s.Close()
	p, _ := NewPayClient(s.URL, "pt-BR")
	st, err := p.OrderStatus(t.Context(), id, "order-capability")
	if err != nil || st.IsSettled() {
		t.Fatal("unexpected status", err)
	}
	if err = p.CancelOrder(t.Context(), "session", id); err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{"", ".", ".."} {
		if p.CancelOrder(t.Context(), "session", bad) == nil {
			t.Fatalf("accepted %q", bad)
		}
	}
}

func TestSingleBalanceWriteOnUncertainOutcome(t *testing.T) {
	for _, kind := range []string{"redirect", "unavailable", "timeout", "reflected", "success"} {
		t.Run(kind, func(t *testing.T) {
			var calls atomic.Int32
			destination := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { t.Error("followed payment redirect") }))
			defer destination.Close()
			s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
				var body map[string]any
				if json.NewDecoder(r.Body).Decode(&body) != nil || len(body) != 4 || body["token"] != "fixture-bearer" || body["is_mobile"] != false || body["amount"] != 12.34 || body["payment_type"] != "wallet" {
					t.Error("balance request")
				}
				switch kind {
				case "redirect":
					http.Redirect(w, r, destination.URL, 307)
				case "unavailable":
					http.Error(w, "fixture-bearer", 502)
				case "timeout":
					http.Error(w, "request timed out", http.StatusRequestTimeout)
				case "reflected":
					io.WriteString(w, `{"orderId":"id","qrCode":"fixture-bearer","statusAccessToken":"private-cap"}`)
				case "success":
					io.WriteString(w, `{"orderId":"id/%","payAmount":88.9,"bonusAmount":1.54,"qrCode":"/pay/short","statusAccessToken":"private-cap"}`)
				}
			}))
			defer s.Close()
			p, _ := NewPayClient(s.URL, "en")
			o, err := p.CreateOrder(t.Context(), "fixture-bearer", OrderKind{Amount: 12.34}, "wallet")
			if kind == "success" {
				if err != nil || o.OrderID != "id/%" || o.Bonus() != 1.54 || o.PayAmount == nil || *o.PayAmount != 88.9 || o.QRCode != s.URL+"/pay/short" || strings.Contains(fmt.Sprintf("%+v %#v", o, o), "private-cap") {
					t.Fatal("invalid order", err)
				}
			} else if !errors.Is(err, ErrRechargeUncertain) || strings.Contains(err.Error(), "fixture-bearer") || o.OrderID != "" {
				t.Fatal("unexpected outcome", err)
			}
			if calls.Load() != 1 {
				t.Fatal("repeated order")
			}
		})
	}
}

func TestPaymentErrorMessageSelection(t *testing.T) {
	for _, tc := range []struct{ body, code, message string }{
		{`{"error":" preferred ","message":"other","code":" REFUSED "}`, "REFUSED", "preferred"},
		{`{"error":{},"code":7,"message":" fallback "}`, "", "fallback"},
		{`{"ignored":"` + strings.Repeat("x", maxResponseSize) + `","message":"full"}`, "", "full"},
		{`not JSON`, "", "the payment service answered with status 409"},
	} {
		s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(409); io.WriteString(w, tc.body) }))
		p, _ := NewPayClient(s.URL, "en")
		err := p.CancelOrder(t.Context(), "session-token", "id")
		s.Close()
		if err == nil || ErrorCode(err) != tc.code || err.Error() != tc.message {
			t.Errorf("%v; want %s", err, tc.message)
		}
	}
}

func TestNativeAmountBoundsAndPromotionCents(t *testing.T) {
	remaining := 2.
	cfg := PayConfig{MinAmount: 1, MaxAmount: 10, MaxDailyAmount: 3, EnabledPaymentTypes: []string{"bank"}, MethodLimits: map[string]MethodLimit{"bank": {Available: true, SingleMin: 5, SingleMax: 20, Remaining: &remaining}}}
	if cfg.EffectiveMax("bank") != 20 || cfg.EffectiveMin("bank") != 5 || cfg.EffectiveMax("other") != 10 {
		t.Fatal("effective method bounds")
	}
	cap := 3.
	p := Promotion{MinAmount: 10, BonusType: BonusPercent, BonusValue: 12.5, MaxBonus: &cap}
	if ComputeBonus(9.99, p) != 0 || ComputeBonus(12.34, p) != 1.54 || ComputeBonus(32.5, p) != 3 || ComputeBonus(MaxCreditAmount-.01, p) != .01 || ComputeBonus(math.NaN(), p) != 0 {
		t.Fatal("incorrect cent rounding or cap")
	}
	a := p
	a.ID = "a"
	a.SortOrder = 2
	b := p
	b.ID = "b"
	b.SortOrder = 1
	if got := PickBestPromotion(10, []Promotion{a, b}); got == nil || got.Rule.ID != "b" {
		t.Fatal("sort order tie break")
	}
	p.StartsAt = "2026-09-02T00:00:00.500Z"
	now := time.Date(2026, 9, 2, 0, 0, 0, 0, time.UTC)
	if !IsInWindow(p, now) {
		t.Fatal("service window uses Unix seconds")
	}
	p.EndsAt = p.StartsAt
	if IsInWindow(p, now) {
		t.Fatal("end exclusive")
	}
	if DescribePromotion(p, "en-US") != "Top up $10+ and get 12.5% bonus (up to $3)" || DescribePromotion(p, "zh-CN") != "充 $10 送 12.5%（最高 $3）" || DescribePromotion(Promotion{MinAmount: 100, BonusValue: 10}, "ja") != "$100 以上のチャージで $10 進呈" {
		t.Fatal("promotion descriptions")
	}
	if PaymentLabel("UsDc.solana", false) != "USDC (Solana)" || PaymentLabel("usdt.trc20", true) != "USDT（TRC20）" || PaymentLabel("wxpay_direct", true) != "微信支付" || PaymentLabel("Custom", false) != "Custom" {
		t.Fatal("payment labels")
	}
}
