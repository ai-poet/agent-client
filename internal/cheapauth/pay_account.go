package cheapauth

import (
	"context"
	"net/http"
)

// Unlike orders, these two account operations use the gateway's /api/v1
// envelope. Keep them on the same selected API origin as the payment form.
type RedeemResult struct {
	Message    string   `json:"message"`
	Value      float64  `json:"value"`
	NewBalance *float64 `json:"new_balance"`
}

func (p *PayClient) RedeemCode(ctx context.Context, token, code string) (RedeemResult, error) {
	var result RedeemResult
	err := p.client.do(ctx, http.MethodPost, "/redeem", token, map[string]string{"code": code}, &result)
	return result, err
}

type SubscriptionProgress struct {
	Subscription struct {
		GroupID   int64  `json:"group_id"`
		ExpiresAt string `json:"expires_at"`
		Status    string `json:"status"`
		Group     *Group `json:"group"`
	} `json:"subscription"`
}

func (p *PayClient) SubscriptionProgress(ctx context.Context, token string) ([]SubscriptionProgress, error) {
	var result []SubscriptionProgress
	err := p.client.do(ctx, http.MethodGet, "/subscriptions/progress", token, nil, &result)
	return result, err
}
