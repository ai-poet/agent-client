package cheapauth

import (
	"encoding/json"
	"math"
	"strings"
)

type PayConfig struct {
	UserID                                  int64
	UserDisplayName, HelpText, HelpImageURL string
	UserBalance, BalanceCreditCNYPerUSD     *float64
	MaxPendingOrders, PendingCount          int64
	StripeEnabled                           bool
	MinAmount, MaxAmount, MaxDailyAmount    float64
	BalanceDisabled                         bool
	EnabledPaymentTypes                     []string
	MethodLimits                            map[string]MethodLimit
	Promotions                              []Promotion
}

func (l *MethodLimit) UnmarshalJSON(raw []byte) error {
	type wire MethodLimit
	v := wire{Available: true}
	if err := json.Unmarshal(raw, &v); err != nil {
		return err
	}
	*l = MethodLimit(v)
	return nil
}

type MethodLimit struct {
	Available bool     `json:"available"`
	SingleMin float64  `json:"singleMin"`
	SingleMax float64  `json:"singleMax"`
	Remaining *float64 `json:"remaining"`
	FeeRate   float64  `json:"feeRate"`
}

func (c PayConfig) EffectiveMin(method string) float64 {
	if m := c.MethodLimits[method].SingleMin; m > 0 {
		return max(c.MinAmount, m)
	}
	return c.MinAmount
}
func (c PayConfig) EffectiveMax(method string) float64 {
	if m := c.MethodLimits[method].SingleMax; m > 0 {
		return m
	}
	return c.MaxAmount
}
func (c PayConfig) FeeRate(method string) float64 { return max(0, c.MethodLimits[method].FeeRate) }
func (c PayConfig) HelpLines() []string {
	var lines []string
	for _, s := range strings.Split(c.HelpText, "\n") {
		if s = strings.TrimSpace(s); s != "" {
			lines = append(lines, s)
		}
	}
	return lines
}

type PlanLimits struct {
	DailyLimitUSD   *float64 `json:"daily_limit_usd"`
	WeeklyLimitUSD  *float64 `json:"weekly_limit_usd"`
	MonthlyLimitUSD *float64 `json:"monthly_limit_usd"`
}

type SubscriptionPlan struct {
	ID                   string      `json:"id"`
	GroupID              int64       `json:"groupId"`
	GroupName            string      `json:"groupName"`
	Name                 string      `json:"name"`
	Description          string      `json:"description"`
	Price                float64     `json:"price"`
	OriginalPrice        *float64    `json:"originalPrice"`
	ValidityDays         int64       `json:"validityDays"`
	ValidityUnit         string      `json:"validityUnit"`
	Features             []string    `json:"features"`
	ProductName          string      `json:"productName"`
	Platform             string      `json:"platform"`
	RateMultiplier       *float64    `json:"rateMultiplier"`
	Limits               *PlanLimits `json:"limits"`
	DefaultMappedModel   string      `json:"defaultMappedModel"`
	SupportedModelScopes []string    `json:"supportedModelScopes"`
}

func (p SubscriptionPlan) DiscountedFrom() *float64 {
	if p.OriginalPrice != nil && *p.OriginalPrice > p.Price+.004 {
		return p.OriginalPrice
	}
	return nil
}

func (p *SubscriptionPlan) UnmarshalJSON(b []byte) error {
	type plain SubscriptionPlan
	var v struct {
		plain
		Features             json.RawMessage `json:"features"`
		SupportedModelScopes json.RawMessage `json:"supportedModelScopes"`
	}
	if err := json.Unmarshal(b, &v); err != nil {
		return err
	}
	*p = SubscriptionPlan(v.plain)
	if p.ValidityUnit != "month" && p.ValidityUnit != "week" {
		p.ValidityUnit = "day"
	}
	p.Features = payStrings(v.Features)
	p.SupportedModelScopes = payStrings(v.SupportedModelScopes)
	return nil
}

func payStrings(raw json.RawMessage) []string {
	var list []any
	_ = json.Unmarshal(raw, &list)
	var result []string
	for _, v := range list {
		if m, ok := v.(map[string]any); ok {
			v = m["text"]
		}
		if s, ok := v.(string); ok && strings.TrimSpace(s) != "" {
			result = append(result, strings.TrimSpace(s))
		}
	}
	return result
}

// Amount is credited USD for balance orders, or the plan's quoted price when PlanID is set.
type OrderKind struct {
	Amount float64
	PlanID string
}

type PayOrder struct {
	OrderID           string   `json:"orderId"`
	Amount            float64  `json:"amount"`
	PayAmount         *float64 `json:"payAmount"`
	Status            string   `json:"status"`
	PaymentType       string   `json:"paymentType"`
	PayURL            string   `json:"payUrl"`
	QRCode            string   `json:"qrCode"`
	ClientSecret      *string  `json:"clientSecret"`
	ExpiresAt         string   `json:"expiresAt"`
	StatusAccessToken string   `json:"statusAccessToken"`
	BonusAmount       *float64 `json:"bonusAmount"`
	PromotionName     *string  `json:"promotionName"`
	FeeRate           *float64 `json:"feeRate"`
}

func (PayOrder) String() string     { return "CheapRouter payment order (redacted)" }
func (o PayOrder) GoString() string { return o.String() }
func (o PayOrder) Bonus() float64 {
	if o.BonusAmount != nil && finite(*o.BonusAmount) {
		return max(0, *o.BonusAmount)
	}
	return 0
}
func (o PayOrder) CreditedTotal() float64 { return math.Round((o.Amount+o.Bonus())*100) / 100 }

type OrderStatus struct {
	ID              string `json:"id"`
	Status          string `json:"status"`
	ExpiresAt       string `json:"expiresAt"`
	PaymentSuccess  bool   `json:"paymentSuccess"`
	RechargeSuccess bool   `json:"rechargeSuccess"`
	RechargeStatus  string `json:"rechargeStatus"`
	FailedReason    string `json:"failedReason"`
}

func (s OrderStatus) IsTerminal() bool {
	switch strings.ToUpper(s.Status) {
	case "FAILED", "CANCELLED", "EXPIRED", "COMPLETED":
		return true
	}
	return false
}
func (s OrderStatus) IsSettled() bool {
	return s.RechargeSuccess || strings.EqualFold(s.Status, "completed")
}
func (o PayOrder) Seed() OrderStatus {
	return OrderStatus{ID: o.OrderID, Status: o.Status, ExpiresAt: o.ExpiresAt, RechargeStatus: "not_paid"}
}

type PayFlow string

const (
	PayFlowQR       PayFlow = "qr"
	PayFlowRedirect PayFlow = "redirect"
	PayFlowStripe   PayFlow = "stripe"
)

func ResolveFlow(o PayOrder) PayFlow {
	if o.ClientSecret != nil {
		return PayFlowStripe
	}
	m := strings.ToLower(strings.TrimSpace(o.PaymentType))
	if o.PayURL != "" && (strings.HasPrefix(m, "wxpay") || strings.HasPrefix(m, "bank")) {
		return PayFlowRedirect
	}
	if o.QRCode != "" {
		return PayFlowQR
	}
	return PayFlowRedirect
}
func IsStripe(method string) bool {
	return strings.HasPrefix(strings.ToLower(strings.TrimSpace(method)), "stripe")
}
func StablecoinToken(method string) string {
	s := strings.ToUpper(strings.TrimSpace(method))
	for _, token := range []string{"USDT", "USDC"} {
		if strings.HasPrefix(s, token) {
			return token
		}
	}
	return ""
}
func StablecoinNetwork(method string) string {
	if StablecoinToken(method) == "" {
		return ""
	}
	_, net, _ := strings.Cut(method, ".")
	switch net = strings.ToLower(strings.TrimSpace(net)); net {
	case "polygon":
		return "Polygon"
	case "plasma":
		return "Plasma"
	case "solana":
		return "Solana"
	default:
		return strings.ToUpper(net)
	}
}
