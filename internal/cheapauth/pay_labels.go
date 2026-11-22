package cheapauth

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
)

type paymentText struct{ Alipay, WeChat, Network, Fixed, Percent, Cap string }

//go:embed pay_labels.json
var paymentTranslations []byte
var paymentLabels = func() map[string]paymentText {
	var labels map[string]paymentText
	if err := json.Unmarshal(paymentTranslations, &labels); err != nil {
		panic(err)
	}
	return labels
}()

func PaymentLabel(method string, chinese bool) string {
	words := paymentLabels["en"]
	if chinese {
		words = paymentLabels["zh"]
	}
	m := strings.ToLower(strings.TrimSpace(method))
	switch {
	case strings.HasPrefix(m, "alipay"):
		return words.Alipay
	case strings.HasPrefix(m, "wxpay"):
		return words.WeChat
	case StablecoinToken(m) != "":
		token := StablecoinToken(m)
		if network := StablecoinNetwork(m); network != "" {
			return fmt.Sprintf(words.Network, token, network)
		}
		return token
	case IsStripe(m):
		return "Stripe"
	default:
		return method
	}
}

func DescribePromotion(p Promotion, locale string) string {
	tag := strings.ToLower(strings.TrimSpace(locale))
	words := paymentLabels["en"]
	if strings.HasPrefix(tag, "zh") {
		words = paymentLabels["zh"]
	} else if strings.HasPrefix(tag, "ja") {
		words = paymentLabels["ja"]
	}
	minimum := FormatAmount(p.MinAmount)
	if p.BonusType != BonusPercent {
		return fmt.Sprintf(words.Fixed, minimum, FormatAmount(p.BonusValue))
	}
	percent := "0"
	if finite(p.BonusValue) {
		percent = strconv.FormatFloat(math.Round(p.BonusValue*100)/100, 'f', -1, 64)
	}
	text := fmt.Sprintf(words.Percent, minimum, percent)
	if p.MaxBonus != nil && *p.MaxBonus > 0 {
		text += fmt.Sprintf(words.Cap, FormatAmount(*p.MaxBonus))
	}
	return text
}
