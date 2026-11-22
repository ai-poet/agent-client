package cheapauth

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
)

// ErrRechargeUncertain means the order may exist. Do not retry the POST automatically.
var ErrRechargeUncertain = errors.New("recharge order outcome unknown; inspect order history")

func reflected(value string, secrets ...string) bool {
	for _, secret := range secrets {
		if secret != "" && strings.Contains(value, secret) {
			return true
		}
	}
	return false
}

type PayError struct {
	Status        int
	Code, Message string
}

func (e *PayError) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return fmt.Sprintf("the payment service answered with status %d", e.Status)
}
func ErrorCode(err error) string {
	var e *PayError
	if errors.As(err, &e) {
		return e.Code
	}
	return ""
}
func PlansUnavailable(err error) bool {
	var e *PayError
	if !errors.As(err, &e) {
		return false
	}
	switch e.Status {
	case 404, 405, 501, 502, 503:
		return true
	}
	return e.Code == "NOT_JSON"
}

type PayClient struct {
	client *Client
	lang   string
}

func NewPayClient(endpoint, lang string) (*PayClient, error) {
	c, err := NewClient(endpoint, nil)
	if err != nil {
		return nil, err
	}
	return &PayClient{c, payLang(lang)}, nil
}
func payLang(lang string) string {
	tag, _, _ := strings.Cut(strings.ToLower(strings.TrimSpace(lang)), "-")
	if tag == "" {
		return "en"
	}
	return tag
}

// PaySession refreshes on the login origin before selecting a payment route.
func (s *Service) PaySession(ctx context.Context, lang string) (*PayClient, string, error) {
	cr, c, err := s.session(ctx)
	if err != nil {
		return nil, "", err
	}
	p, err := NewPayClient(paymentOrigin(s.store.Path(), c.endpoint), lang)
	if err != nil {
		return nil, "", err
	}
	p.client.http = c.http
	return p, cr.AccessToken, nil
}
func (p *PayClient) Endpoint() string { return p.client.endpoint }
func (p *PayClient) AbsoluteURL(raw string) string {
	raw = strings.TrimSpace(raw)
	if strings.HasPrefix(raw, "//") {
		return "https:" + raw
	}
	if !strings.HasPrefix(raw, "/") {
		return raw
	}
	u, _ := url.Parse(p.Endpoint())
	return u.Scheme + "://" + u.Host + raw
}
func (p *PayClient) PayCenterURL(token string) string {
	return p.Endpoint() + "/pay?" + url.Values{"token": {token}, "theme": {"dark"}, "ui_mode": {"standalone"}, "lang": {p.lang}}.Encode()
}

func (p *PayClient) request(ctx context.Context, method, path string, q url.Values, bearer string, body, out any, secrets ...string) error {
	if q == nil {
		q = url.Values{}
	}
	q.Set("lang", p.lang)
	var input io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return err
		}
		input = bytes.NewReader(b)
	}
	req, err := http.NewRequestWithContext(ctx, method, p.Endpoint()+"/pay/api/"+path+"?"+q.Encode(), input)
	if err != nil {
		return &AuthError{Kind: ErrorResponse, Op: "payment"}
	}
	req.Header.Set("Accept", "application/json")
	req.Header.Set("Accept-Language", p.lang)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if bearer != "" {
		req.Header.Set("Authorization", "Bearer "+bearer)
	}
	h := *p.client.http
	h.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	res, err := h.Do(req)
	if err != nil {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		return &AuthError{Kind: ErrorNetwork, Op: "payment"}
	}
	defer res.Body.Close()
	if res.StatusCode < 200 || res.StatusCode >= 300 {
		// A malformed field does not discard other string fields. Older
		// services use "message" instead of "error".
		var v struct{ Code, Error, Message json.RawMessage }
		decoder := json.NewDecoder(res.Body)
		if decoder.Decode(&v) != nil || decoder.Decode(new(json.RawMessage)) != io.EOF {
			return &PayError{Status: res.StatusCode}
		}
		field := func(raw json.RawMessage) string {
			var s string
			_ = json.Unmarshal(raw, &s)
			return strings.TrimSpace(s)
		}
		code, msg := field(v.Code), field(v.Error)
		if msg == "" {
			msg = field(v.Message)
		}
		if reflected(code+msg, secrets...) {
			code, msg = "", ""
		}
		return &PayError{res.StatusCode, code, msg}
	}
	b, err := io.ReadAll(io.LimitReader(res.Body, maxResponseSize+1))
	if err != nil || len(b) > maxResponseSize {
		return &AuthError{Kind: ErrorResponse, Op: "payment"}
	}
	if bytes.HasPrefix(bytes.TrimSpace(b), []byte("<")) {
		return &PayError{Status: res.StatusCode, Code: "NOT_JSON"}
	}
	if bytes.Equal(bytes.TrimSpace(b), []byte("null")) || json.Unmarshal(b, out) != nil {
		return &AuthError{Kind: ErrorResponse, Op: "payment"}
	}
	return nil
}

func (p *PayClient) get(ctx context.Context, path string, q url.Values, token string, out any) error {
	if token == "" {
		return ErrUnauthorized
	}
	err := p.request(ctx, "GET", path, q, token, nil, out, token)
	var e *PayError
	if !errors.As(err, &e) || e.Status != 400 && e.Status != 401 {
		return err
	}
	if q == nil {
		q = url.Values{}
	}
	q.Set("token", token)
	return p.request(ctx, "GET", path, q, "", nil, out, token)
}

func (p *PayClient) LoadConfig(ctx context.Context, token string) (PayConfig, error) {
	var my struct {
		User struct {
			ID                           int64
			DisplayName, Username, Email string
			Balance                      *float64
		}
		Summary struct{ Pending int64 }
	}
	if err := p.get(ctx, "orders/my", url.Values{"page": {"1"}, "page_size": {"20"}}, token, &my); err != nil {
		return PayConfig{}, err
	}
	if my.User.ID <= 0 {
		return PayConfig{}, errors.New("payment service did not identify account")
	}
	// Decode only service-owned fields. Account identity and Stripe availability
	// come from orders/my and the publishable key, not config look-alikes.
	var data struct {
		Config *struct {
			MinAmount, MaxAmount                         *float64
			MaxDailyAmount                               float64
			MaxPendingOrders                             *int64
			EnabledPaymentTypes                          []string
			MethodLimits                                 map[string]json.RawMessage
			BalanceCreditCNYPerUSD                       *float64
			BalanceDisabled                              bool
			Promotions                                   json.RawMessage
			HelpText, HelpImageURL, StripePublishableKey string
		}
	}
	if err := p.get(ctx, "user", url.Values{"user_id": {strconv.FormatInt(my.User.ID, 10)}}, token, &data); err != nil {
		return PayConfig{}, err
	}
	cfg := PayConfig{UserID: my.User.ID, UserBalance: my.User.Balance, PendingCount: my.Summary.Pending, MinAmount: 1, MaxAmount: 1000, MaxPendingOrders: 3}
	for _, name := range []string{my.User.DisplayName, my.User.Username, my.User.Email} {
		if name = strings.TrimSpace(name); name != "" {
			cfg.UserDisplayName = name
			break
		}
	}
	if d := data.Config; d != nil {
		if d.MinAmount != nil {
			cfg.MinAmount = *d.MinAmount
		}
		if d.MaxAmount != nil {
			cfg.MaxAmount = *d.MaxAmount
		}
		if d.MaxPendingOrders != nil {
			cfg.MaxPendingOrders = *d.MaxPendingOrders
		}
		cfg.MaxDailyAmount = max(0, d.MaxDailyAmount)
		cfg.EnabledPaymentTypes = d.EnabledPaymentTypes
		cfg.BalanceCreditCNYPerUSD = d.BalanceCreditCNYPerUSD
		cfg.BalanceDisabled = d.BalanceDisabled
		cfg.StripeEnabled = strings.TrimSpace(d.StripePublishableKey) != ""
		cfg.HelpText = strings.TrimSpace(d.HelpText)
		cfg.HelpImageURL = p.AbsoluteURL(d.HelpImageURL)
		cfg.MethodLimits = map[string]MethodLimit{}
		for k, raw := range d.MethodLimits {
			limit := MethodLimit{Available: true}
			if json.Unmarshal(raw, &limit) != nil {
				limit = MethodLimit{Available: true}
			}
			cfg.MethodLimits[k] = limit
		}
		var promos []json.RawMessage
		_ = json.Unmarshal(d.Promotions, &promos)
		for _, raw := range promos {
			var promo Promotion
			if json.Unmarshal(raw, &promo) == nil && promo.ID != "" {
				cfg.Promotions = append(cfg.Promotions, promo)
			}
		}
	}
	if reflected(cfg.UserDisplayName+cfg.HelpImageURL+cfg.HelpText, token) {
		return PayConfig{}, &AuthError{Kind: ErrorResponse, Op: "payment config"}
	}
	for _, method := range cfg.EnabledPaymentTypes {
		if reflected(method, token) {
			return PayConfig{}, &AuthError{Kind: ErrorResponse, Op: "payment method"}
		}
	}
	for _, promo := range cfg.Promotions {
		if reflected(promo.ID+promo.Name+promo.Description, token) {
			return PayConfig{}, &AuthError{Kind: ErrorResponse, Op: "payment promotion"}
		}
	}
	return cfg, nil
}

func (p *PayClient) ListPlans(ctx context.Context, token string) ([]SubscriptionPlan, error) {
	var reply struct{ Plans []json.RawMessage }
	if err := p.get(ctx, "subscription-plans", nil, token, &reply); err != nil {
		return nil, err
	}
	var plans []SubscriptionPlan
	for _, raw := range reply.Plans {
		var v SubscriptionPlan
		if json.Unmarshal(raw, &v) == nil && v.ID != "" && v.GroupID > 0 {
			plans = append(plans, v)
		}
	}
	return plans, nil
}

func strValue(s *string) string {
	if s == nil {
		return ""
	}
	return *s
}

func validOrderID(s string) bool {
	return s != "" && s != "." && s != ".."
}

// QueryEscape uses the same unreserved set as the pay center. In path
// segments a space must be %20, never "+", and a literal slash is data.
func orderSegment(id string) string { return strings.ReplaceAll(url.QueryEscape(id), "+", "%20") }
func (p *PayClient) CreateOrder(ctx context.Context, token string, kind OrderKind, method string) (PayOrder, error) {
	if token == "" {
		return PayOrder{}, ErrUnauthorized
	}
	if !finite(kind.Amount) || kind.PlanID == "" && kind.Amount <= 0 || strings.TrimSpace(method) == "" {
		return PayOrder{}, errors.New("invalid payment order")
	}
	body := map[string]any{"token": token, "amount": kind.Amount, "payment_type": method, "is_mobile": false}
	if kind.PlanID != "" {
		body["plan_id"], body["order_type"] = kind.PlanID, "subscription"
		if kind.Amount <= 0 {
			body["amount"] = 0.01
		}
	}
	var o PayOrder
	if err := p.request(ctx, "POST", "orders", nil, "", body, &o, token); err != nil {
		var e *PayError
		if errors.As(err, &e) && e.Status >= 400 && e.Status < 500 && e.Status != http.StatusRequestTimeout {
			return PayOrder{}, err
		}
		return PayOrder{}, errors.Join(ErrRechargeUncertain, err)
	}
	if !validOrderID(o.OrderID) || o.StatusAccessToken == "" || reflected(o.StatusAccessToken, token) || reflected(o.OrderID+o.PayURL+o.QRCode+strValue(o.PromotionName)+o.Status+o.PaymentType, token, o.StatusAccessToken, strValue(o.ClientSecret)) {
		return PayOrder{}, ErrRechargeUncertain
	}
	o.QRCode = p.AbsoluteURL(o.QRCode)
	o.PayURL = p.AbsoluteURL(o.PayURL)
	return o, nil
}
func (p *PayClient) OrderStatus(ctx context.Context, id, statusToken string) (OrderStatus, error) {
	if !validOrderID(id) || statusToken == "" {
		return OrderStatus{}, errors.New("invalid order capability")
	}
	var st OrderStatus
	err := p.request(ctx, "GET", "orders/"+orderSegment(id), url.Values{"access_token": {statusToken}}, "", nil, &st, statusToken)
	if err != nil {
		return OrderStatus{}, err
	}
	if st.ID != "" && st.ID != id || reflected(st.FailedReason+st.Status+st.RechargeStatus, statusToken) {
		return OrderStatus{}, &AuthError{Kind: ErrorResponse, Op: "payment status"}
	}
	return st, nil
}
func (p *PayClient) CancelOrder(ctx context.Context, token, id string) error {
	if !validOrderID(id) {
		return errors.New("invalid order ID")
	}
	if token == "" {
		return ErrUnauthorized
	}
	var reply json.RawMessage
	return p.request(ctx, "POST", "orders/"+orderSegment(id)+"/cancel", nil, "", map[string]string{"token": token}, &reply, token)
}
