package live

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"image"
	"image/draw"
	"maps"
	"net/http"
	"net/url"
	"path/filepath"
	"slices"
	"strings"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	accountui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
	"rsc.io/qr"
)

// Each sheet belongs to one generation. Replacing it cancels the observation,
// not an order already admitted by the service.
type rechargeSession struct {
	*Runtime
	*accountui.RechargeDialog
	ctx         context.Context
	cancel      context.CancelFunc
	generation  *cheapAuthGeneration
	pay         *core.PayClient
	lang        string
	accountID   int64
	loadedToken [32]byte
	order       core.PayOrder
	stopPoll    context.CancelFunc
}

func (r *Runtime) openRecharge() { r.openPayment(nil, false) }

func (r *Runtime) openPayment(plan *core.SubscriptionPlan, listing bool) {
	a := r.cheapAuth
	if a == nil || a.generation == nil || !a.state.SignedIn() || a.state.LoggingOut {
		return
	}
	if prev := a.state.Recharge; prev != nil {
		prev.Close()
	}
	ctx, cancel := context.WithCancel(a.generation.ctx)
	s := &rechargeSession{Runtime: r, RechargeDialog: &accountui.RechargeDialog{Amount: "10", Plan: plan, Listing: listing}, ctx: ctx, cancel: cancel, generation: a.generation, accountID: a.state.Account.ID}
	a.state.Recharge = s.RechargeDialog
	s.RechargeDialog.Close = func() {
		cancel()
		if a.state.Recharge == s.RechargeDialog {
			a.state.Recharge = nil
		}
	}
	s.Submit = s.submit
	s.CheckStatus = s.checkStatus
	s.CancelOrder = s.cancelOrder
	s.OpenCenter = s.openCenter
	s.Redeem = s.redeem
	s.ChoosePlan = func(p core.SubscriptionPlan) { r.openPayment(&p, false) }
	s.Back = func() { r.openPayment(plan, false) }
	r.enqueue(func(c *ui.Context) {
		if !s.active() {
			return
		}
		s.lang = i18n.Match(c.Locale())
		if listing || plan == nil {
			s.loadPlans()
		}
		if listing {
			s.loadSubscriptions()
		} else {
			s.load()
		}
	})
}

func (s *rechargeSession) active() bool {
	return s.ctx.Err() == nil && s.cheapAuth.state.Recharge == s.RechargeDialog && s.cheapAuth.state.SignedIn()
}

// A pay-service refusal may be about the order. Only the account service can
// definitively reject the renewable desktop session.
func (s *rechargeSession) sessionEnded(err error) bool {
	if cheapAuthRejected(err) {
		s.RechargeDialog.Close()
		s.startCheapSync(s.cheapAuth, s.generation)
		return true
	}
	return false
}

func (s *rechargeSession) redeem(code string) {
	if !s.active() || s.Busy || s.Plan != nil || s.Config == nil || strings.TrimSpace(code) == "" {
		return
	}
	s.Busy, s.Message = true, ""
	s.async(func() {
		client, token, err := s.paymentCredentials()
		var result core.RedeemResult
		if err == nil {
			result, err = client.RedeemCode(s.ctx, token, code)
		}
		s.enqueue(func(c *ui.Context) {
			if !s.active() || s.sessionEnded(err) {
				return
			}
			s.Busy = false
			if err != nil {
				s.Message = err.Error()
				return
			}
			s.RedeemCode = ""
			if result.NewBalance != nil {
				s.cheapAuth.state.Account.Balance = *result.NewBalance
				s.Config.UserBalance = result.NewBalance
			}
			msg := result.Message
			if msg == "" {
				msg = i18n.T(c, "pay_redeemed", "value", fmt.Sprintf("%.2f", result.Value))
			}
			kit.Toast(c, kit.Done, msg)
		})
	})
}

func (s *rechargeSession) loadSubscriptions() {
	s.async(func() {
		p, token, err := s.cheapAuth.service.PaySession(s.ctx, s.lang)
		if err != nil {
			return
		}
		rows, err := p.SubscriptionProgress(s.ctx, token)
		s.enqueue(func(c *ui.Context) {
			if !s.active() {
				return
			}
			if err != nil {
				if s.Activated {
					s.Message, s.Retry = err.Error(), s.loadSubscriptions
				}
				return
			}
			s.OwnedGroups = map[int64]bool{}
			for _, row := range rows {
				sub := row.Subscription
				if sub.Status != "" && !strings.EqualFold(sub.Status, "active") {
					continue
				}
				s.OwnedGroups[sub.GroupID] = true
				if s.Activated && sub.GroupID == s.Plan.GroupID {
					s.Message, s.Retry = "", nil
					if sub.ExpiresAt != "" {
						s.Expiry = i18n.T(c, "pay_plan_expires", "date", sub.ExpiresAt)
					}
					if sub.Group != nil {
						if lane := core.GroupLane(*sub.Group, s.cheapAuth.state.Catalog); lane != "" {
							s.BindLane = core.LaneDisplayName(lane)
							s.BindPlan = func() { s.selectCheapLaneGroup(lane, &sub.GroupID); s.RechargeDialog.Close() }
						}
					}
				}
			}
			if s.Activated && !s.OwnedGroups[s.Plan.GroupID] {
				s.Retry = s.loadSubscriptions
			}
		})
	})
}

func (s *rechargeSession) loadPlans() {
	if s.Listing {
		s.Busy = true
	}
	s.async(func() {
		client, token, err := s.cheapAuth.service.PaySession(s.ctx, s.lang)
		var plans []core.SubscriptionPlan
		if err == nil {
			plans, err = client.ListPlans(s.ctx, token)
		}
		s.enqueue(func(c *ui.Context) {
			if !s.active() || s.sessionEnded(err) {
				return
			}
			if s.Listing {
				s.Busy = false
				s.Message, s.Retry = "", nil
				if err != nil && !core.PlansUnavailable(err) {
					s.Message, s.Retry = err.Error(), s.loadPlans
				}
			}
			s.Plans = plans
			if len(plans) > 0 {
				s.BrowsePlans = func() { s.openPayment(nil, true) }
			}
		})
	})
}

func (s *rechargeSession) load() {
	if !s.active() || s.Busy {
		return
	}
	s.Busy, s.Retry, s.Message = true, nil, ""
	s.async(func() {
		client, token, err := s.cheapAuth.service.PaySession(s.ctx, s.lang)
		var config core.PayConfig
		if err == nil {
			config, err = client.LoadConfig(s.ctx, token)
		}
		s.enqueue(func(c *ui.Context) {
			if !s.active() || s.sessionEnded(err) {
				return
			}
			s.Busy = false
			if err != nil {
				s.Message, s.Retry = err.Error(), s.load
				return
			}
			if s.accountID != 0 && s.accountID != config.UserID {
				s.RechargeDialog.Close()
				return
			}
			s.pay, s.accountID, s.loadedToken = client, config.UserID, sha256.Sum256([]byte(token))
			s.Config = &config
			s.Method = s.FirstAvailableMethod()
			if raw := config.HelpImageURL; raw != "" {
				dir := filepath.Dir(filepath.Dir(s.Preferences.Path))
				s.async(func() {
					card, err := client.LoadHelpImage(s.ctx, dir, raw)
					if err != nil {
						return
					}
					pic, err := decodeAttachmentBitmap(card.Bytes, 640)
					if err == nil {
						s.enqueue(func(*ui.Context) {
							if s.active() {
								s.HelpImage = pic
							}
						})
					}
				})
			}
		})
	})
}

// Another window can replace the stored session. When renewal changes the
// bearer, verify the account that owns the form before admitting a mutation.
func (s *rechargeSession) paymentCredentials() (*core.PayClient, string, error) {
	p, token, err := s.cheapAuth.service.PaySession(s.ctx, s.lang)
	if err != nil {
		return nil, "", err
	}
	if s.pay != nil && (s.pay.Endpoint() != p.Endpoint() || sha256.Sum256([]byte(token)) != s.loadedToken) {
		cfg, err := p.LoadConfig(s.ctx, token)
		if err != nil {
			return nil, "", err
		}
		if cfg.UserID != s.accountID || p.Endpoint() != s.pay.Endpoint() {
			return nil, "", errors.New("payment account changed; reopen the payment dialog")
		}
	}
	return p, token, nil
}

func (s *rechargeSession) submit(amount float64, method string) {
	if !s.active() || s.Busy || s.Submit == nil || s.OrderID != "" || s.Config == nil {
		return
	}
	kind := core.OrderKind{Amount: amount}
	if s.Plan != nil {
		kind.PlanID = s.Plan.ID
		kind.Amount = s.Plan.Price
	}
	s.Busy, s.Message = true, ""
	s.async(func() {
		client, token, err := s.paymentCredentials()
		var order core.PayOrder
		if err == nil {
			order, err = client.CreateOrder(s.ctx, token, kind, method)
		}
		s.enqueue(func(c *ui.Context) {
			if !s.active() || s.sessionEnded(err) {
				return
			}
			s.Busy = false
			if err != nil {
				var refusal *core.PayError
				if errors.As(err, &refusal) && refusal.Status >= 400 && refusal.Status < 500 && refusal.Status != http.StatusRequestTimeout {
					if s.Plan != nil && slices.Contains([]string{"PLAN_NOT_AVAILABLE", "GROUP_NOT_FOUND", "GROUP_NOT_BOUND", "GROUP_TYPE_MISMATCH"}, refusal.Code) {
						kit.Toast(c, kit.Failed, err.Error())
						s.openPayment(nil, true)
						return
					}
					s.Message = err.Error()
				} else if errors.Is(err, core.ErrRechargeUncertain) {
					// Losing the reply is not evidence that no order exists.
					s.Message = i18n.T(c, "recharge_order_unknown")
					s.Submit = nil
				} else {
					s.Message = err.Error()
				}
				return
			}
			s.order, s.OrderID, s.Submit = order, order.OrderID, nil
			status := order.Seed()
			s.Status = &status
			s.PayURL = rechargeBrowserURL(order.PayURL)
			switch core.ResolveFlow(order) {
			case core.PayFlowRedirect:
				if s.PayURL != "" {
					c.OpenURL(s.PayURL)
				} else {
					s.Message = i18n.T(c, "pay_redirect_url_missing")
				}
			case core.PayFlowQR:
				s.CheckoutData = order.QRCode
				s.QR = rechargeQR(order.QRCode)
				if s.PayURL == "" {
					s.PayURL = rechargeBrowserURL(order.QRCode)
				}
			}
			s.OrderSummary = accountui.PaymentSummary(c, order, s.Plan)
			s.watch(time.Now().Add(5 * time.Minute))
		})
	})
}

func rechargeBrowserURL(raw string) string {
	u, err := url.Parse(raw)
	if err != nil || (u.Scheme != "https" && u.Scheme != "http") || u.Hostname() == "" || u.User != nil {
		return ""
	}
	return raw
}

func rechargeQR(payload string) *image.NRGBA {
	if payload == "" || len(payload) > 4096 {
		return nil
	}
	code, err := qr.Encode(payload, qr.M)
	if err != nil {
		return nil
	}
	img := code.Image()
	pixels := image.NewNRGBA(img.Bounds())
	draw.Draw(pixels, pixels.Bounds(), img, img.Bounds().Min, draw.Src)
	return pixels
}

func (s *rechargeSession) watch(deadline time.Time) {
	ctx, stop := context.WithDeadline(s.ctx, deadline)
	s.stopPoll = stop
	s.Deadline = deadline
	order := s.order
	s.async(func() {
		tick := time.NewTicker(time.Second)
		defer tick.Stop()
		for n := 1; ; n++ {
			select {
			case <-ctx.Done():
				if errors.Is(ctx.Err(), context.DeadlineExceeded) {
					s.enqueue(func(c *ui.Context) {
						if s.active() && !s.Terminal && !s.Activated {
							s.Terminal = true
							s.Message = i18n.T(c, "recharge_timeout")
						}
					})
				}
				return
			case <-tick.C:
			}
			if n%2 != 0 {
				s.wake()
				continue
			}
			status, err := s.pay.OrderStatus(ctx, order.OrderID, order.StatusAccessToken)
			s.enqueue(func(c *ui.Context) {
				if !s.active() || ctx.Err() != nil {
					return
				}
				if err != nil {
					s.Message = err.Error()
					return
				}
				s.applyStatus(c, status)
			})
		}
	})
}

func (s *rechargeSession) checkStatus() {
	if !s.active() || s.Busy || s.CheckingStatus || s.Terminal || s.OrderID == "" || s.pay == nil {
		return
	}
	s.CheckingStatus, s.Message = true, ""
	order := s.order
	s.async(func() {
		status, err := s.pay.OrderStatus(s.ctx, order.OrderID, order.StatusAccessToken)
		s.enqueue(func(c *ui.Context) {
			if !s.active() {
				return
			}
			s.CheckingStatus = false
			if err != nil {
				s.Message = err.Error()
				return
			}
			s.applyStatus(c, status)
			if !s.Terminal && !status.IsSettled() && !status.PaymentSuccess {
				s.Message = i18n.T(c, "pay_not_paid_yet")
			}
		})
	})
}

func (s *rechargeSession) applyStatus(c *ui.Context, status core.OrderStatus) {
	keepNotPaid := !status.IsSettled() && !status.IsTerminal() && !status.PaymentSuccess && status.RechargeStatus != "failed" && s.Message == i18n.T(c, "pay_not_paid_yet")
	s.Status = &status
	if !keepNotPaid {
		s.Message = ""
	}
	if !status.IsSettled() && !status.IsTerminal() && status.RechargeStatus != "failed" {
		return
	}
	if s.stopPoll != nil {
		s.stopPoll()
	}
	s.Terminal = true
	if !status.IsSettled() {
		return
	}
	if s.Plan != nil {
		s.Activated = true
		s.refreshCheapGroups()
		s.loadSubscriptions()
	} else {
		s.RechargeDialog.Close()
		message := i18n.T(c, "recharge_paid")
		if s.order.Bonus() > 0 {
			message = i18n.T(c, "pay_success_toast_bonus", "total", fmt.Sprintf("$%.2f", s.order.CreditedTotal()), "bonus", fmt.Sprintf("$%.2f", s.order.Bonus()))
		}
		kit.Toast(c, kit.Done, message)
	}
	s.refreshAccount()
}

func (s *rechargeSession) cancelOrder() {
	if !s.active() || s.Busy || s.Terminal || s.OrderID == "" {
		return
	}
	s.Busy = true
	s.stopPoll()
	order := s.order
	s.async(func() {
		latest, err := s.pay.OrderStatus(s.ctx, order.OrderID, order.StatusAccessToken)
		if err != nil || !(latest.PaymentSuccess || latest.RechargeSuccess || latest.IsTerminal()) {
			client, token, sessionErr := s.paymentCredentials()
			err = sessionErr
			if err == nil {
				err = client.CancelOrder(s.ctx, token, order.OrderID)
			}
			if err == nil {
				latest = core.OrderStatus{ID: order.OrderID, Status: "CANCELLED", ExpiresAt: order.ExpiresAt, RechargeStatus: "closed"}
			}
		}
		s.enqueue(func(c *ui.Context) {
			if !s.active() || s.sessionEnded(err) {
				return
			}
			s.Busy = false
			if err != nil {
				s.Message = err.Error()
			} else {
				s.applyStatus(c, latest)
			}
			if !s.Terminal {
				s.watch(time.Now().Add(5 * time.Minute))
			}
		})
	})
}

func (s *rechargeSession) openCenter() {
	if !s.active() || s.Busy {
		return
	}
	s.Busy = true
	before := s.cheapAuth.state.Account
	plan := s.Plan != nil
	s.async(func() {
		client, token, err := s.paymentCredentials()
		var link string
		var subscriptions map[int64]string
		if err == nil {
			link = client.PayCenterURL(token)
			if plan {
				subscriptions = subscriptionExpiries(s.ctx, client, token)
			}
		}
		s.enqueue(func(c *ui.Context) {
			if !s.active() || s.sessionEnded(err) {
				return
			}
			s.Busy = false
			if err != nil {
				s.Message = err.Error()
				return
			}
			c.OpenURL(link)
			// There is no browser callback. Observe only the account; never
			// infer that this specific order settled from a changed balance.
			s.watchHosted(before, plan, subscriptions)
		})
	})
}

func (s *rechargeSession) refreshAccount() {
	a, g := s.cheapAuth, s.generation
	if a.generation != g || g.ctx.Err() != nil {
		return
	}
	s.async(func() {
		user, err := a.service.Account(g.ctx)
		s.enqueue(func(c *ui.Context) {
			if a.generation != g || g.ctx.Err() != nil || !a.state.SignedIn() {
				return
			}
			if err != nil {
				kit.Retry(c, i18n.T(c, "recharge_refresh_failed"), s.refreshAccount)
				return
			}
			if s.accountID == 0 || s.accountID == user.ID {
				a.state.Account, a.state.AccountFailure = user, ""
			}
		})
	})
}

func subscriptionExpiries(ctx context.Context, p *core.PayClient, token string) map[int64]string {
	rows, err := p.SubscriptionProgress(ctx, token)
	if err != nil {
		return nil
	}
	res := map[int64]string{}
	for _, row := range rows {
		res[row.Subscription.GroupID] = row.Subscription.ExpiresAt
	}
	return res
}

func (s *rechargeSession) watchHosted(before core.User, plan bool, subscriptions map[int64]string) {
	a, g := s.cheapAuth, s.generation
	if a.stopPayRefresh != nil {
		a.stopPayRefresh()
	}
	ctx, stop := context.WithCancel(g.ctx)
	a.stopPayRefresh = stop
	s.async(func() {
		defer stop()
		ticker := time.NewTicker(15 * time.Second)
		defer ticker.Stop()
		attempts := 8
		if plan {
			attempts = 12
		}
		for range attempts {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
			}
			user, err := a.service.Account(ctx)
			if err != nil {
				continue
			}
			changed := user.Balance != before.Balance
			if plan {
				changed = false
				p, token, err := a.service.PaySession(ctx, s.lang)
				if err == nil {
					if now := subscriptionExpiries(ctx, p, token); now != nil {
						changed = subscriptions != nil && !maps.Equal(now, subscriptions)
						subscriptions = now
					}
				}
			}
			s.enqueue(func(*ui.Context) {
				if a.generation == g && a.state.SignedIn() && (before.ID == 0 || before.ID == user.ID) {
					a.state.Account = user
					if plan && changed {
						s.refreshCheapGroups()
					}
				}
			})
			if changed {
				return
			}
		}
	})
}
