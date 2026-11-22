package cheapauth

import (
	"image"
	"slices"
	"strconv"
	"strings"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
)

// RechargeDialog is confined to the UI thread. Live retains order capabilities;
// payment links, snapshots and callbacks here belong to this sheet only.
type RechargeDialog struct {
	Config                        *core.PayConfig
	Plan                          *core.SubscriptionPlan
	Listing, Activated            bool
	Plans                         []core.SubscriptionPlan
	OwnedGroups                   map[int64]bool
	ChoosePlan                    func(core.SubscriptionPlan)
	BindPlan                      func()
	BindLane, Expiry              string
	Amount, Method                string
	Busy                          bool
	OrderID, PayURL, CheckoutData string
	QR                            *image.NRGBA
	HelpImage                     *ui.Bitmap
	Status                        *core.OrderStatus
	// OrderSummary is the localized service-confirmed price and credit.
	OrderSummary, Message                                    string
	Terminal                                                 bool
	Submit                                                   func(amount float64, method string)
	Close, Retry, CancelOrder, OpenCenter, BrowsePlans, Back func()
	Redeem                                                   func(string)
	RedeemCode                                               string

	stableNotice, confirmCancel bool
	qrImage                     bitmapCache
}

type bitmapCache struct {
	src    *image.NRGBA
	bitmap *ui.Bitmap
}

func (b *bitmapCache) show(c *ui.Context, src *image.NRGBA, w, h float32, qr bool) {
	if src == nil {
		return
	}
	if src != b.src {
		b.src, b.bitmap = src, ui.NewBitmap(src)
		if qr {
			b.bitmap.Pixelated()
		}
	}
	ui.Image(c, b.bitmap).Size(w, h).Fit(ui.Contain)
}

type dialogOwner struct {
	state  *State
	dialog *RechargeDialog
}

func (o *dialogOwner) Close() error {
	if o.state != nil && o.state.Recharge == o.dialog && o.dialog != nil && o.dialog.Close != nil {
		o.dialog.Close()
	}
	return nil
}

func (d *RechargeDialog) offered(method string) bool {
	if d.Plan == nil {
		return true
	}
	m := strings.ToLower(strings.TrimSpace(method))
	return strings.HasPrefix(m, "alipay") || strings.HasPrefix(m, "wxpay") || core.IsStripe(method)
}

func parseAmount(raw string) (float64, bool) {
	raw = strings.TrimSpace(raw)
	_, fraction, _ := strings.Cut(raw, ".")
	if len(fraction) > 2 || strings.IndexFunc(raw, func(r rune) bool { return (r < '0' || r > '9') && r != '.' }) != -1 {
		return 0, false
	}
	n, err := strconv.ParseFloat(raw, 64)
	return n, err == nil && n > 0
}

func (d *RechargeDialog) submission() (float64, string) {
	cfg := d.Config
	if cfg == nil || d.Busy || d.OrderID != "" || d.Submit == nil {
		return 0, "recharge_order_failed"
	}
	if d.Method == "" || !d.offered(d.Method) || !slices.Contains(cfg.EnabledPaymentTypes, d.Method) {
		return 0, "pay_select_method"
	}
	if d.Plan == nil && cfg.BalanceDisabled {
		return 0, "recharge_disabled"
	}
	// Stripe uses hosted checkout, independently of the native publishable key.
	if core.IsStripe(d.Method) {
		if d.OpenCenter == nil {
			return 0, "pay_method_unavailable"
		}
		return 0, ""
	}
	if cfg.PendingCount >= cfg.MaxPendingOrders {
		return 0, "pay_too_many_pending"
	}
	if limit, ok := cfg.MethodLimits[d.Method]; ok && !limit.Available {
		return 0, "pay_method_unavailable"
	}
	if d.Plan != nil {
		return d.Plan.Price, ""
	}
	n, ok := parseAmount(d.Amount)
	if !ok {
		return 0, "pay_enter_valid_amount"
	}
	if n < cfg.EffectiveMin(d.Method) || n > cfg.EffectiveMax(d.Method) {
		return 0, "pay_amount_range"
	}
	return n, ""
}

func (d *RechargeDialog) proceed(lang string, confirmed bool) {
	amount, key := d.submission()
	d.stableNotice = false
	if key != "" {
		if d.Config != nil {
			d.Message = i18n.In(lang, key, "count", d.Config.PendingCount, "min", core.FormatAmount(d.Config.EffectiveMin(d.Method)), "max", core.FormatAmount(d.Config.EffectiveMax(d.Method)))
		}
		return
	}
	d.Message = ""
	if core.IsStripe(d.Method) {
		d.OpenCenter()
		return
	}
	if core.StablecoinToken(d.Method) != "" && !confirmed {
		d.stableNotice = true
		return
	}
	d.Submit(amount, d.Method)
}
func rechargeView(c *ui.Context, d *RechargeDialog) {
	title := i18n.T(c, "recharge_open")
	if d.Listing || d.Plan != nil {
		title = i18n.T(c, "pay_plan_title")
	}
	w, h := c.Size()
	open := true
	creating := d.Busy && !d.Listing && d.Config != nil && d.OrderID == ""
	body := func() {
		ui.Text(c, title).FontSize(16).FontWeight(600).PaddingEnd(32)
		ui.Scroll(c.Key("payment-body")).MaxHeight(max(100, h*0.9-180)).Gap(8).Children(func() {
			switch {
			case d.Listing:
				plansList(c, d)
			case d.Activated && d.Plan != nil:
				ui.Text(c, i18n.T(c, "plans_activated_title", "plan", d.Plan.Name)).FontWeight(600)
				if d.Expiry != "" {
					ui.Text(c, d.Expiry)
				} else {
					ui.Text(c, i18n.T(c, "plans_syncing"))
				}
				if d.BindPlan != nil {
					ui.Text(c, i18n.T(c, "plans_use_for_cli_hint", "cli", d.BindLane))
					kit.Secondary(c.Key("bind-plan"), i18n.T(c, "plans_use_for_cli", "cli", d.BindLane)).Disabled(d.Busy).OnClick(d.BindPlan)
				}
				if d.Close != nil {
					kit.Secondary(c.Key("done"), i18n.T(c, "plans_done")).OnClick(d.Close)
				}
			case d.OrderID != "":
				orderView(c, d)
			case d.stableNotice:
				stablecoinView(c, d)
			case d.Config != nil && (d.Submit != nil || d.Message == ""):
				rechargeForm(c, d)
			case d.Busy:
				ui.Text(c, i18n.T(c, "recharge_loading"))
			}
			if d.Message != "" {
				ui.Text(c, d.Message).Selectable()
			}
			if d.OpenCenter != nil && !d.Listing && !d.Activated && (d.Config == nil && !d.Busy || d.OrderID != "" && d.PayURL == "" && d.QR == nil || d.Submit == nil && d.OrderID == "") {
				kit.Secondary(c.Key("pay-center"), i18n.T(c, "pay_open_full_center")).Disabled(d.Busy).OnClick(d.OpenCenter)
			}
			if d.Config != nil && !d.stableNotice && !d.Listing && !d.Activated {
				paymentHelp(c, d)
			}
		})
		if d.Retry != nil {
			kit.Secondary(c.Key("retry-recharge"), i18n.T(c, "recharge_retry")).Disabled(d.Busy).OnClick(d.Retry)
		}
		if !d.Activated && d.Terminal && d.Status != nil && (d.Status.IsTerminal() || d.Status.RechargeStatus == "failed") && d.Back != nil {
			kit.Secondary(c.Key("back"), i18n.T(c, "pay_back_to_form")).Disabled(d.Busy).OnClick(d.Back)
		}
		if d.Config != nil && !d.Activated && !d.Listing && !d.stableNotice && (d.Plan != nil || !d.Config.BalanceDisabled) && d.OrderID == "" && d.Submit != nil {
			label := i18n.T(c, "pay_continue")
			if d.Busy {
				label = i18n.T(c, "pay_creating_order")
			} else if core.IsStripe(d.Method) {
				label = i18n.T(c, "pay_open_full_center")
			}
			if d.Plan != nil && !d.Busy && !core.IsStripe(d.Method) {
				label = i18n.T(c, "pay_plan_pay_cta", "amount", cny(planCharge(d)))
			}
			kit.PrimaryBusy(c.Key("submit-recharge"), label, d.Busy || d.Method == "", d.Busy).OnClick(func() { d.proceed(c.Locale(), false) })
		}
	}
	var panel ui.Element
	if creating {
		panel = kit.BareDialog(c.Key("recharge-dialog"), &open, body)
	} else {
		panel = kit.Dialog(c.Key("recharge-dialog"), &open, body)
	}
	panel.Width(min(440, w-40)).Label(title)
	if !open && !creating && d.Close != nil {
		d.Close()
	}
}

func stablecoinView(c *ui.Context, d *RechargeDialog) {
	token := core.StablecoinToken(d.Method)
	ui.Text(c, i18n.T(c, "pay_stablecoin_title", "token", token)).FontWeight(600)
	ui.Text(c, i18n.T(c, "pay_stablecoin_direct", "token", token))
	if network := core.StablecoinNetwork(d.Method); network != "" {
		ui.Text(c, i18n.T(c, "pay_stablecoin_network", "network", network))
	} else {
		ui.Text(c, i18n.T(c, "pay_stablecoin_network_unknown"))
	}
	ui.Text(c, i18n.T(c, "pay_stablecoin_warning"))
	kit.Secondary(c.Key("stable-back"), i18n.T(c, "pay_back_to_form")).OnClick(func() { d.stableNotice = false })
	kit.PrimaryBusy(c.Key("stable-confirm"), i18n.T(c, "pay_stablecoin_confirm"), d.Busy, d.Busy).OnClick(func() { d.proceed(c.Locale(), true) })
}

func paymentHelp(c *ui.Context, d *RechargeDialog) {
	help := strings.TrimSpace(d.Config.HelpText)
	if help == "" && d.HelpImage == nil {
		return
	}
	ui.Text(c, i18n.T(c, "pay_help_title")).FontWeight(600)
	if help != "" {
		ui.Text(c, help).Selectable()
	}
	if d.HelpImage != nil {
		ui.Image(c.Key("help-image"), d.HelpImage).Size(250, 160).Fit(ui.Contain)
	}
}
