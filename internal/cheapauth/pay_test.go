package cheapauth

import (
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"
)

func TestPayServiceContract(t *testing.T) {
	const token = "account/+ token"
	const cap = "cap/+ status"
	var fallback, created, cancelled int
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		q := r.URL.Query()
		if q.Get("lang") != "zh" || r.Header.Get("Accept-Language") != "zh" {
			t.Error("missing locale")
		}
		switch r.URL.Path {
		case "/pay/api/orders/my":
			if q.Get("page_size") != "20" || q.Get("page") != "1" {
				t.Error("pagination")
			}
			if q.Get("token") == "" {
				if r.Header.Get("Authorization") != "Bearer "+token {
					t.Error("bearer")
				}
				w.WriteHeader(401)
				return
			}
			if q.Get("token") != token || r.Header.Get("Authorization") != "" {
				t.Error("legacy auth")
			}
			fallback++
			io.WriteString(w, `{"user":{"id":5,"username":" user ","balance":2.5},"summary":{"pending":1}}`)
		case "/pay/api/user":
			if r.Header.Get("Authorization") != "Bearer "+token || q.Get("user_id") != "5" || q.Has("token") {
				t.Error("config auth")
			}
			io.WriteString(w, `{"config":{"methodLimits":{"bank":{"feeRate":2.5}},"enabledPaymentTypes":["bank"],"balanceCreditCnyPerUsd":7,"stripePublishableKey":"pk","helpImageUrl":"/help","promotions":[null,{"id":2},{"id":"b","bonusType":"future","bonusValue":3}]}}`)
		case "/pay/api/subscription-plans":
			io.WriteString(w, `{"plans":[false,{"id":"invalid"},{"id":"p","groupId":9,"features":[" a ",null,{"text":"b"}],"validityUnit":"future","limits":{"daily_limit_usd":10}}]}`)
		case "/pay/api/orders":
			created++
			var body map[string]any
			_ = json.NewDecoder(r.Body).Decode(&body)
			if r.Method != "POST" || len(body) != 6 || body["token"] != token || body["order_type"] != "subscription" || body["plan_id"] != "p" || body["amount"] != .01 || body["is_mobile"] != false || r.Header.Get("Authorization") != "" {
				t.Error("plan order body")
			}
			io.WriteString(w, `{"orderId":"o","amount":1.50,"bonusAmount":0.1,"paymentType":"bank","qrCode":"weixin://pay","payUrl":"/short","statusAccessToken":"cap/+ status"}`)
		case "/pay/api/orders/o":
			if r.Header.Get("Authorization") != "" || q.Get("access_token") != cap || q.Has("token") {
				t.Error("status auth")
			}
			io.WriteString(w, `{"id":"o","status":"RECHARGING","paymentSuccess":true}`)
		case "/pay/api/orders/o/cancel":
			var body map[string]string
			_ = json.NewDecoder(r.Body).Decode(&body)
			if r.Method != "POST" || len(body) != 1 || body["token"] != token {
				t.Error("cancel body")
			}
			cancelled++
			io.WriteString(w, `{}`)
		default:
			t.Error("unknown path", r.URL.Path)
		}
	}))
	defer srv.Close()
	p, err := NewPayClient(srv.URL, "zh-CN")
	if err != nil {
		t.Fatal(err)
	}
	cfg, err := p.LoadConfig(t.Context(), token)
	if err != nil || cfg.MinAmount != 1 || cfg.MaxAmount != 1000 || cfg.PendingCount != 1 || cfg.MaxPendingOrders != 3 || cfg.UserDisplayName != "user" || cfg.UserBalance == nil || *cfg.UserBalance != 2.5 || !cfg.StripeEnabled || cfg.HelpImageURL != srv.URL+"/help" || cfg.FeeRate("bank") != 2.5 || !cfg.MethodLimits["bank"].Available || len(cfg.Promotions) != 1 || ComputeBonus(10, cfg.Promotions[0]) != 3 {
		t.Fatalf("config %+v: %v", cfg, err)
	}
	plans, err := p.ListPlans(t.Context(), token)
	if err != nil || len(plans) != 1 || plans[0].ValidityUnit != "day" || !slices.Equal(plans[0].Features, []string{"a", "b"}) || plans[0].Limits == nil || *plans[0].Limits.DailyLimitUSD != 10 {
		t.Fatalf("plans %+v: %v", plans, err)
	}
	o, err := p.CreateOrder(t.Context(), token, OrderKind{PlanID: "p"}, "bank")
	if err != nil || o.PayURL != srv.URL+"/short" || o.QRCode != "weixin://pay" || o.CreditedTotal() != 1.6 || ResolveFlow(o) != PayFlowRedirect {
		t.Fatal("order:", err)
	}
	st, err := p.OrderStatus(t.Context(), o.OrderID, o.StatusAccessToken)
	if err != nil || !st.PaymentSuccess || st.IsSettled() || st.IsTerminal() {
		t.Fatal("status:", err)
	}
	if err := p.CancelOrder(t.Context(), token, o.OrderID); err != nil {
		t.Fatal(err)
	}
	if created != 1 || cancelled != 1 || fallback != 1 {
		t.Fatal("unexpected request counts")
	}
	u, _ := url.Parse(p.PayCenterURL(token))
	if u.Host != strings.TrimPrefix(srv.URL, "http://") || u.Query().Get("token") != token || u.Query().Get("ui_mode") != "standalone" {
		t.Fatal("hosted checkout")
	}
}

func TestPayErrorCodes(t *testing.T) {
	for _, tc := range []struct {
		status     int
		body, code string
		absent     bool
	}{
		{200, `<html>`, "NOT_JSON", true}, {503, `{}`, "", true}, {429, `{"code":"TOO_MANY_PENDING","error":"wait"}`, "TOO_MANY_PENDING", false},
		{403, `{"code":"reflected-secret","message":"reflected-secret"}`, "", false},
	} {
		s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(tc.status); io.WriteString(w, tc.body) }))
		p, _ := NewPayClient(s.URL, "")
		_, err := p.ListPlans(t.Context(), "reflected-secret")
		s.Close()
		if err == nil || ErrorCode(err) != tc.code || PlansUnavailable(err) != tc.absent || strings.Contains(err.Error(), "reflected-secret") {
			t.Errorf("status %d: %v", tc.status, err)
		}
	}
	if PlansUnavailable(errors.New("offline")) {
		t.Fatal("network failure hid plans")
	}
}

func TestPaymentOriginSelection(t *testing.T) {
	d := t.TempDir()
	profile := filepath.Join(d, credentialName)
	login := "https://self.example"
	for _, tc := range []struct{ body, want string }{
		{`{}`, login}, {`{"chosen":"https://rogue.example"}`, login}, {`{"chosen":"https://cdn.cheaprouter.cc"}`, CDNEndpoint},
		{`{"chosen":"https://alt.example","candidates":["alt.example/v1"]}`, "https://alt.example"},
		{`{"chosen":"https://node.example","server_nodes":[{"origin":"https://node.example"}]}`, "https://node.example"},
	} {
		if err := os.WriteFile(filepath.Join(d, "gateway-origin.json"), []byte(tc.body), 0600); err != nil {
			t.Fatal(err)
		}
		if got := paymentOrigin(profile, login); got != tc.want {
			t.Errorf("got %s want %s", got, tc.want)
		}
	}
}

func TestPromotionThresholdsAndWindows(t *testing.T) {
	now := time.Date(2026, 9, 2, 0, 0, 0, 0, time.UTC)
	base := Promotion{ID: "b", MinAmount: 10, BonusValue: 2, Available: true, StartsAt: "2026-09-02T08:00:00+08:00"}
	a := base
	a.ID = "a"
	higher := base
	higher.MinAmount = 50
	higher.BonusValue = 3
	ended := base
	ended.EndsAt = "2026-09-02T00:00:00"
	ended.BonusValue = 100
	active := ActivePromotions([]Promotion{base, ended, a, higher}, now)
	best := PickBestPromotion(15, active)
	hint := NextPromotionHint(15, active)
	if len(active) != 3 || best == nil || best.Rule.ID != "a" || hint == nil || hint.NeedMore != 35 || hint.Bonus != 3 {
		t.Fatal("promotion selection")
	}
	if !slices.Equal(MergeQuickAmounts([]float64{100, 10}, []float64{50, 10, 500}, 1, 100), []float64{10, 50, 100}) {
		t.Fatal("threshold merge")
	}
}

func TestPaymentHelpImage(t *testing.T) {
	const png = "\x89PNG\r\n\x1a\n"
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "" || r.URL.RawQuery != "" {
			t.Error("image request contains credentials")
		}
		io.WriteString(w, png)
	}))
	defer s.Close()
	d := t.TempDir()
	p, _ := NewPayClient(s.URL, "en")
	img, err := p.LoadHelpImage(t.Context(), d, "/contact")
	if err != nil || img.Extension != "png" {
		t.Fatalf("contact: %v", err)
	}
	s.Close()
	img, err = p.LoadHelpImage(t.Context(), d, "/contact")
	if err != nil || string(img.Bytes) != png {
		t.Fatalf("cache: %v", err)
	}
	if _, err := p.LoadHelpImage(t.Context(), d, "data:text/html;base64,PGh0bWw+"); err == nil {
		t.Fatal("accepted HTML")
	}
}
