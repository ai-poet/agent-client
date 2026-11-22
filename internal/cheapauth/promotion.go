package cheapauth

import (
	"encoding/json"
	"errors"
	"math"
	"slices"
	"strconv"
	"strings"
	"time"
)

const MaxCreditAmount = 99_999_999.99

type BonusType string

const (
	BonusFixed   BonusType = "fixed"
	BonusPercent BonusType = "percent"
)

func (b *BonusType) UnmarshalJSON(raw []byte) error {
	var value *string
	if err := json.Unmarshal(raw, &value); err != nil {
		return err
	}
	if value == nil {
		return errors.New("invalid promotion bonus type")
	}
	*b = BonusFixed
	if *value == "percent" {
		*b = BonusPercent
	}
	return nil
}
func (b BonusType) MarshalJSON() ([]byte, error) {
	if b == BonusPercent {
		return json.Marshal("percent")
	}
	return json.Marshal("fixed")
}

func (p *Promotion) UnmarshalJSON(raw []byte) error {
	type wire Promotion
	v := wire{Available: true, BonusType: BonusFixed}
	if err := json.Unmarshal(raw, &v); err != nil {
		return err
	}
	*p = Promotion(v)
	return nil
}

type Promotion struct {
	ID          string    `json:"id"`
	Name        string    `json:"name"`
	Description string    `json:"description"`
	MinAmount   float64   `json:"minAmount"`
	BonusType   BonusType `json:"bonusType"`
	BonusValue  float64   `json:"bonusValue"`
	MaxBonus    *float64  `json:"maxBonus"`
	SortOrder   int64     `json:"sortOrder"`
	StartsAt    string    `json:"startsAt"`
	EndsAt      string    `json:"endsAt"`
	Available   bool      `json:"available"`
}

func finite(x float64) bool { return !math.IsInf(x, 0) && !math.IsNaN(x) }

func ToCents(value float64) int64 {
	cents := math.Round(value * 100)
	switch {
	case math.IsNaN(cents):
		return 0
	case cents >= math.MaxInt64:
		return math.MaxInt64
	case cents <= math.MinInt64:
		return math.MinInt64
	default:
		return int64(cents)
	}
}

func ComputeBonus(amount float64, rule Promotion) float64 {
	return float64(bonusCents(amount, rule)) / 100
}

func bonusCents(amount float64, rule Promotion) int64 {
	if !finite(amount) || amount <= 0 || amount >= MaxCreditAmount || !finite(rule.MinAmount) || !finite(rule.BonusValue) || rule.BonusValue <= 0 {
		return 0
	}
	cents := ToCents(amount)
	if cents < ToCents(rule.MinAmount) {
		return 0
	}
	headroom := ToCents(MaxCreditAmount) - cents
	bonus := ToCents(rule.BonusValue)
	if rule.BonusType == BonusPercent {
		bonus = int64(min(float64(headroom), math.Floor(float64(cents)*rule.BonusValue/100+1e-9)))
		if cap := rule.MaxBonus; cap != nil && finite(*cap) && *cap > 0 {
			bonus = min(bonus, ToCents(*cap))
		}
	}
	return max(0, min(bonus, headroom))
}

type PromotionMatch struct {
	Rule  Promotion
	Bonus float64
}
type PromotionHint struct {
	Rule                       Promotion
	Threshold, NeedMore, Bonus float64
}

// PickBestPromotion expects rules already filtered for quota and time window.
func PickBestPromotion(amount float64, rules []Promotion) *PromotionMatch {
	var best *PromotionMatch
	for _, p := range rules {
		b := ComputeBonus(amount, p)
		if b > 0 && (best == nil || b > best.Bonus || b == best.Bonus && (p.SortOrder < best.Rule.SortOrder || p.SortOrder == best.Rule.SortOrder && p.ID < best.Rule.ID)) {
			best = &PromotionMatch{p, b}
		}
	}
	return best
}

func NextPromotionHint(amount float64, rules []Promotion) *PromotionHint {
	if !finite(amount) || amount < 0 {
		amount = 0
	}
	var bonus float64
	if best := PickBestPromotion(amount, rules); best != nil {
		bonus = best.Bonus
	}
	var thresholds []float64
	for _, p := range rules {
		thresholds = append(thresholds, p.MinAmount)
	}
	for _, t := range MergeQuickAmounts(nil, thresholds, 0, MaxCreditAmount) {
		if ToCents(t) <= ToCents(amount) {
			continue
		}
		if best := PickBestPromotion(t, rules); best != nil && best.Bonus > bonus {
			return &PromotionHint{best.Rule, t, float64(ToCents(t)-ToCents(amount)) / 100, best.Bonus}
		}
	}
	return nil
}

func MergeQuickAmounts(base, thresholds []float64, minAmount, maxAmount float64) []float64 {
	var values []float64
	for _, xs := range [][]float64{base, thresholds} {
		for _, x := range xs {
			y := float64(ToCents(x)) / 100
			if finite(x) && x > 0 && y >= minAmount && y <= maxAmount {
				values = append(values, y)
			}
		}
	}
	slices.Sort(values)
	return slices.Compact(values)
}

func payTime(raw string) (time.Time, bool) {
	raw = strings.TrimSpace(raw)
	for _, layout := range []string{time.RFC3339Nano, "2006-01-02T15:04:05"} {
		if t, e := time.Parse(layout, raw); e == nil {
			return t, true
		}
	}
	return time.Time{}, false
}

func SecondsUntil(raw string, now time.Time) (int64, bool) {
	t, ok := payTime(raw)
	if !ok {
		return 0, false
	}
	return max(0, t.Unix()-now.Unix()), true
}

func IsInWindow(p Promotion, now time.Time) bool {
	start, s := payTime(p.StartsAt)
	end, e := payTime(p.EndsAt)
	return (!s || now.Unix() >= start.Unix()) && (!e || now.Unix() < end.Unix())
}
func ActivePromotions(rules []Promotion, now time.Time) []Promotion {
	var active []Promotion
	for _, p := range rules {
		if p.Available && IsInWindow(p, now) {
			active = append(active, p)
		}
	}
	return active
}

func FormatAmount(value float64) string {
	if !finite(value) {
		return "0"
	}
	c := ToCents(value)
	if c%100 == 0 {
		return strconv.FormatInt(c/100, 10)
	}
	return strconv.FormatFloat(float64(c)/100, 'f', 2, 64)
}
