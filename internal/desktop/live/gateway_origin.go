package live

import (
	"errors"
	"slices"
	"strings"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
)

func (r *Runtime) addCheapGatewayOrigin(raw string) {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.state.LoggingOut || account.state.OriginSwitching {
		return
	}
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return
	}
	cfg, added, err := account.service.AddGatewayOrigin(raw)
	if err != nil {
		r.enqueue(func(c *ui.Context) {
			if r.cheapAuth != account {
				return
			}
			var urlErr *core.URLError
			if errors.As(err, &urlErr) {
				account.state.OriginError = i18n.T(c, "gateway_origin_invalid")
			} else {
				account.state.OriginError = cheapAuthError(err)
			}
		})
		return
	}
	if !added {
		r.enqueue(func(c *ui.Context) {
			if r.cheapAuth != account {
				return
			}
			account.state.OriginError = i18n.T(c, "gateway_origin_duplicate")
		})
		return
	}
	account.state.OriginInput = ""
	account.state.OriginError = ""
	account.state.OriginConfig = cfg
}

func (r *Runtime) removeCheapGatewayOrigin(origin string) {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.state.LoggingOut || account.state.OriginSwitching {
		return
	}
	prevCfg := account.service.GatewayOriginConfig()
	prevActive := prevCfg.ResolveAPIOrigin(account.state.Endpoint)
	cfg, _, err := account.service.RemoveGatewayOrigin(origin)
	if err != nil {
		r.enqueue(func(c *ui.Context) {
			if r.cheapAuth != account {
				return
			}
			account.state.OriginError = cheapAuthError(err)
			kit.Toast(c, kit.Failed, account.state.OriginError)
		})
		return
	}
	account.state.OriginResults = nil
	account.state.OriginError = ""
	account.state.OriginConfig = cfg
	nextActive := cfg.ResolveAPIOrigin(account.state.Endpoint)
	if account.state.SignedIn() && prevActive != nextActive {
		r.syncCheapOriginChange(account, prevCfg, cfg, nextActive, "")
	}
}

func (r *Runtime) setCheapGatewayOriginAuto(on bool) {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.state.LoggingOut {
		return
	}
	cfg, err := account.service.SetGatewayOriginAutoSelect(on)
	if err != nil {
		r.enqueue(func(c *ui.Context) {
			if r.cheapAuth != account {
				return
			}
			account.state.OriginError = cheapAuthError(err)
			kit.Toast(c, kit.Failed, account.state.OriginError)
		})
		return
	}
	account.state.OriginConfig = cfg
}

func (r *Runtime) selectCheapGatewayOrigin(origin string) {
	r.switchCheapGatewayOrigin(origin, false, "")
}

// switchCheapGatewayOrigin persists the selected origin and resyncs business
// clients and Node provider routes against it. If syncing the new origin fails
// (e.g. unreachable or broken node), gateway-origin.json and UI state roll back
// to the previous origin with an error notice.
func (r *Runtime) switchCheapGatewayOrigin(origin string, isFallback bool, toastKey string) {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.generation == nil ||
		!account.state.SignedIn() || account.state.LoggingOut ||
		account.state.OriginSwitching || account.state.Stage == cheapauthui.SyncingGateway {
		return
	}
	normalized, err := core.NormalizeGatewayURL(origin)
	if err != nil {
		r.enqueue(func(c *ui.Context) {
			if r.cheapAuth != account {
				return
			}
			account.state.OriginError = i18n.T(c, "gateway_origin_invalid")
			kit.Toast(c, kit.Failed, account.state.OriginError)
		})
		return
	}
	prevCfg := account.service.GatewayOriginConfig()
	prevActive := prevCfg.ResolveAPIOrigin(account.state.Endpoint)
	if prevActive == normalized {
		if !isFallback && prevCfg.FallbackFrom != nil {
			if cfg, _, saveErr := account.service.SelectGatewayOrigin(normalized); saveErr == nil {
				account.state.OriginConfig = cfg
			}
		}
		account.state.OriginError = ""
		return
	}

	var nextCfg core.GatewayOriginConfig
	if isFallback {
		nextCfg, err = account.service.FallBackGatewayOrigin(normalized)
	} else {
		nextCfg, _, err = account.service.SelectGatewayOrigin(normalized)
	}
	if err != nil {
		r.enqueue(func(c *ui.Context) {
			if r.cheapAuth != account {
				return
			}
			account.state.OriginError = cheapAuthError(err)
			kit.Toast(c, kit.Failed, account.state.OriginError)
		})
		return
	}
	r.syncCheapOriginChange(account, prevCfg, nextCfg, normalized, toastKey)
}

func (r *Runtime) syncCheapOriginChange(
	account *cheapAuthRuntime,
	prevCfg, nextCfg core.GatewayOriginConfig,
	targetOrigin, toastKey string,
) {
	generation := account.generation
	if generation == nil {
		return
	}
	// Payment sessions are bound to a specific origin; close any open sheet
	// before switching so orders cannot straddle two nodes.
	if account.state.Recharge != nil && account.state.Recharge.Close != nil {
		account.state.Recharge.Close()
	}
	prevStage := account.state.Stage
	account.state.OriginConfig = nextCfg
	account.state.OriginSwitching = true
	account.state.OriginError = ""

	r.async(func() {
		ctx := generation.ctx
		account.effects.Lock()
		var gatewayErr error
		if pending := account.pending; pending != nil && pending.binding != nil {
			_, gatewayErr = r.resolveCheapProvider(ctx, account)
		}
		account.effects.Unlock()

		result := readCheapGateway(ctx, account.service, gatewayErr, false)
		if result.recoveringOrigin {
			nextCfg = account.service.GatewayOriginConfig()
		}

		var modelCount int
		var disableErr error
		rolledBack := false
		if ctx.Err() == nil {
			if cheapAuthRejected(result.authErr) {
				account.effects.Lock()
				if ctx.Err() == nil {
					result.authErr, disableErr = r.invalidateCheapSession(account, generation, result.authErr, "origin_switch", result.sameOrigin)
				}
				account.effects.Unlock()
			} else if result.gatewayErr == nil {
				modelCount, result.gatewayErr = r.saveCheapGateway(ctx, account, generation, result.gateway)
			}
			if result.gatewayErr != nil && result.authErr == nil && !result.recoveringOrigin {
				_ = account.service.SaveGatewayOriginConfig(prevCfg)
				rolledBack = true
			}
		}
		selectedGroup := account.service.SelectedGroup()
		laneBindings := account.service.LaneBindings()

		r.enqueue(func(c *ui.Context) {
			if account.generation != generation || ctx.Err() != nil {
				return
			}
			account.state.OriginSwitching = false
			account.state.OriginConfig = nextCfg
			if cheapAuthRejected(result.authErr) {
				account.autoBindDone = false
				account.originChecked = false
				account.state.Stage, account.state.Failure = cheapauthui.SignedOut, cheapAuthError(result.authErr)
				if disableErr != nil {
					account.state.Failure = i18n.T(c, "cheaprouter_disable_failed")
				}
				account.state.Account, account.state.ModelCount = core.User{}, 0
				account.state.Groups, account.state.Catalog = nil, nil
				account.state.GroupStatuses, account.state.GroupStatusLoading = nil, false
				account.state.GroupStatusLoaded, account.state.GroupStatusError = false, ""
				account.state.SelectedGroupID, account.state.LaneBindings = nil, nil
				account.state.AutoBindFailures = nil
				account.state.GroupsLoading, account.state.SwitchingLane = false, ""
				account.state.GroupsError, account.state.GroupError = "", ""
				r.syncImageStudioCatalog()
				r.projectModels()
				return
			}
			if rolledBack {
				account.state.OriginConfig = prevCfg
				account.state.Stage = prevStage
				msg := i18n.T(c, "gateway_origin_switch_failed", "url", targetOrigin, "error", cheapAuthError(result.gatewayErr))
				account.state.OriginError = msg
				kit.Toast(c, kit.Failed, msg)
				return
			}
			if result.gatewayErr != nil {
				account.state.Stage, account.state.Failure = cheapauthui.GatewayFailed, cheapAuthError(result.gatewayErr)
				account.state.OriginError = account.state.Failure
				kit.Toast(c, kit.Failed, account.state.OriginError)
				return
			}
			account.state.OriginError = ""
			account.state.Account = result.user
			if result.accountErr != nil {
				account.state.AccountFailure = cheapAuthError(result.accountErr)
			} else {
				account.state.AccountFailure = ""
			}
			if result.groupsErr == nil {
				account.state.Groups = result.groups
				account.state.GroupsError = ""
			} else if len(account.state.Groups) == 0 {
				account.state.GroupsError = cheapAuthError(result.groupsErr)
			}
			if result.statusAttempted {
				account.state.GroupStatusLoading = false
				if result.statusErr == nil {
					account.state.GroupStatuses = core.IndexGroupStatuses(result.statuses)
					account.state.GroupStatusLoaded = true
					account.state.GroupStatusError = ""
				} else {
					account.state.GroupStatuses = nil
					account.state.GroupStatusLoaded = false
					account.state.GroupStatusError = cheapAuthError(result.statusErr)
				}
			}
			if len(result.gateway.Catalog) > 0 {
				account.state.Catalog = result.gateway.Catalog
			}
			account.state.SelectedGroupID = selectedGroup
			account.state.LaneBindings = laneBindings
			account.state.ModelCount = modelCount
			account.state.Stage = cheapauthui.Ready
			account.state.Failure = ""
			account.state.GroupError = ""
			r.syncImageStudioCatalog()
			r.projectModels()
			r.project()
			if result.recoveringOrigin {
				account.state.OriginError = i18n.T(c, "gateway_origin_fallback", "url", account.state.ActiveOrigin())
				kit.Toast(c, kit.Failed, account.state.OriginError)
				return
			}
			key := toastKey
			if key == "" {
				key = "gateway_origin_switched"
			}
			kit.Toast(c, kit.Done, i18n.T(c, key, "url", targetOrigin))
		})
	})
}

// refreshCheapGatewayNodes refreshes the service's cached public node list
// when its 6-hour TTL expires, then triggers the once-per-launch check of the
// chosen origin.
func (r *Runtime) refreshCheapGatewayNodes(account *cheapAuthRuntime, generation *cheapAuthGeneration) {
	if account == nil || account.service == nil || generation == nil ||
		!account.state.SignedIn() || account.state.LoggingOut || account.nodesRefreshing {
		return
	}
	account.nodesRefreshing = true
	r.async(func() {
		cfg, originChanged, err := account.service.RefreshServerNodes(generation.ctx, false)
		r.enqueue(func(*ui.Context) {
			account.nodesRefreshing = false
			if account.generation != generation || generation.ctx.Err() != nil || !account.state.SignedIn() {
				return
			}
			if err == nil {
				account.state.OriginConfig = cfg
				if originChanged && !account.state.OriginSwitching && account.state.Stage != cheapauthui.SyncingGateway {
					r.startCheapSync(account, generation)
					return
				}
			}
			r.verifyCheapGatewayOrigin(account, generation)
		})
	})
}

// verifyCheapGatewayOrigin performs the once-per-launch connectivity check of
// candidate origins when the active route belongs to the managed candidate set.
func (r *Runtime) verifyCheapGatewayOrigin(account *cheapAuthRuntime, generation *cheapAuthGeneration) {
	if account == nil || account.originChecked || account.generation != generation || !account.state.SignedIn() {
		return
	}
	cfg := account.service.GatewayOriginConfig()
	candidates := cfg.EffectiveCandidates()
	active := cfg.ResolveAPIOrigin(account.state.Endpoint)
	if len(candidates) < 2 || !slices.Contains(candidates, active) {
		return
	}
	account.originChecked = true
	r.runCheapGatewayOriginTest(true)
}

// runCheapGatewayOriginTest measures every candidate origin concurrently and
// applies the Rust fallback_from / auto_select rules.
func (r *Runtime) runCheapGatewayOriginTest(automatic bool) {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.generation == nil ||
		!account.state.SignedIn() || account.state.LoggingOut || account.state.OriginSwitching {
		return
	}
	generation := account.generation
	account.originGeneration++
	testGen := account.originGeneration
	account.state.OriginTesting = true
	if !automatic {
		account.state.OriginError = ""
	}
	r.async(func() {
		results, err := account.service.TestGatewayOrigins(generation.ctx, core.DefaultSpeedTestTimeoutSecs)
		r.enqueue(func(c *ui.Context) {
			if account.generation != generation || account.originGeneration != testGen ||
				generation.ctx.Err() != nil || !account.state.SignedIn() {
				return
			}
			account.state.OriginTesting = false
			if err != nil {
				if errors.Is(err, core.ErrNoGatewayKey) && !automatic {
					msg := i18n.T(c, "gateway_origin_no_key")
					account.state.OriginError = msg
					kit.Toast(c, kit.Failed, msg)
				}
				return
			}
			account.state.OriginResults = results
			cfg := account.service.GatewayOriginConfig()
			decision := core.ApplyOriginTest(&cfg, results)
			switch decision.Action {
			case core.OriginActionFallback:
				r.switchCheapGatewayOrigin(decision.Target, true, "gateway_origin_fallback")
			case core.OriginActionRestored:
				r.switchCheapGatewayOrigin(decision.Target, false, "gateway_origin_restored")
			case core.OriginActionAutoSelected:
				r.switchCheapGatewayOrigin(decision.Target, false, "gateway_origin_auto_selected")
			case core.OriginActionNoOK:
				if !automatic {
					msg := i18n.T(c, "gateway_origin_no_ok")
					account.state.OriginError = msg
					kit.Toast(c, kit.Failed, msg)
				}
			}
		})
	})
}
