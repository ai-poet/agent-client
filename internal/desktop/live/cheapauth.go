package live

import (
	"context"
	"errors"
	"log/slog"
	"path/filepath"
	"strings"
	"sync"

	core "github.com/ai-poet/agent-client/internal/cheapauth"
	"github.com/ai-poet/agent-client/internal/client"
	cheapauthui "github.com/ai-poet/agent-client/internal/desktop/cheapauth"
	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/kit"
	"github.com/ai-poet/agent-client/internal/node/providers"
	"github.com/ai-poet/agent-client/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// The UI owns generation changes. Background mutations share effects: after
// taking it, a worker must check its generation's context before admission.
// Already admitted writes settle before another mutation can overtake them.
type cheapAuthRuntime struct {
	service      *core.Service
	state        *cheapauthui.State
	generation   *cheapAuthGeneration
	attempt      *cheapLoginAttempt
	stopSync     context.CancelFunc
	autoBindDone bool

	effects sync.Mutex
	pending *cheapProviderWrite // guarded by effects; never redispatch this request
}

type cheapAuthGeneration struct {
	ctx    context.Context
	cancel context.CancelFunc
}

type cheapLoginAttempt struct {
	*cheapAuthGeneration
	flow       *core.LoginFlow
	once       sync.Once
	committing bool
}

type cheapProviderWrite struct {
	generation *cheapAuthGeneration
	request    protocol.Request
	binding    *cheapGroupBinding
}

type cheapGroupBinding struct {
	lane            string
	previous        *core.LaneBinding
	previousCleared bool
	target          *core.LaneBinding
	legacyPrev      *int64
	legacyTarget    *int64
	legacyMode      bool
}

func (a *cheapAuthRuntime) advance(parent context.Context) *cheapAuthGeneration {
	if a.generation != nil {
		a.generation.cancel()
	}
	a.stopSync, a.attempt = nil, nil
	a.autoBindDone = false
	ctx, cancel := context.WithCancel(parent)
	a.generation = &cheapAuthGeneration{ctx: ctx, cancel: cancel}
	return a.generation
}

func (r *Runtime) initCheapAuth() {
	profile := filepath.Dir(filepath.Dir(r.Preferences.Path))
	service, err := core.NewService(profile)
	state := cheapauthui.NewState()
	account := &cheapAuthRuntime{service: service, state: state}
	r.cheapAuth = account
	r.App.Settings.CheapRouter = state
	state.Start = r.startCheapLogin
	state.SubmitPaste = r.submitCheapPaste
	state.Cancel = r.cancelCheapLogin
	state.Retry = func() {
		if state.Stage == cheapauthui.GatewayFailed && !state.LoggingOut && !state.SwitchingGroup {
			r.startCheapSync(account, account.generation)
		}
	}
	state.StopSync = r.stopCheapSync
	state.Logout = r.logoutCheapAuth
	state.RefreshGroups = r.refreshCheapGroups
	state.SelectGroup = r.selectCheapGroup
	state.SelectLaneGroup = r.selectCheapLaneGroup
	if err != nil {
		state.Failure = cheapAuthError(err)
		return
	}
	if err := service.Restore(); err != nil {
		state.Failure = "无法读取已保存的登录状态，请重新登录"
		return
	}
	if service.SignedIn() {
		state.Endpoint = service.Endpoint()
		state.SelectedGroupID = service.SelectedGroup()
		state.LaneBindings = service.LaneBindings()
		r.startCheapSync(account, account.advance(r.ctx))
	}
}

func (r *Runtime) startCheapLogin(manual bool) {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.state.LoggingOut || account.state.Stage != cheapauthui.SignedOut {
		return
	}
	generation := account.advance(r.ctx)
	attempt := &cheapLoginAttempt{cheapAuthGeneration: generation}
	account.attempt = attempt
	state := account.state
	endpoint := state.Endpoint
	state.Stage, state.Failure, state.Pasted, state.Exchanging = cheapauthui.Starting, "", "", false
	r.async(func() {
		var flow *core.LoginFlow
		var err error
		if manual {
			flow, err = account.service.StartManualLogin(endpoint)
		} else {
			flow, err = account.service.StartLogin(attempt.ctx, endpoint)
		}
		if err != nil {
			r.enqueue(func(*ui.Context) {
				if account.attempt == attempt && attempt.ctx.Err() == nil {
					state.Stage, state.Failure = cheapauthui.SignedOut, cheapAuthError(err)
				}
			})
			return
		}
		context.AfterFunc(attempt.ctx, flow.Close)
		r.enqueue(func(c *ui.Context) {
			if account.attempt != attempt || attempt.ctx.Err() != nil {
				return
			}
			attempt.flow = flow
			state.Stage, state.LoginURL, state.Loopback = cheapauthui.WaitingForCode, flow.LoginURL(), flow.Loopback()
			c.OpenURL(flow.LoginURL())
		})
		if flow.Loopback() {
			credentials, waitErr := flow.Wait(attempt.ctx)
			r.finishCheapLogin(attempt, credentials, waitErr, false)
		}
	})
}

func (r *Runtime) submitCheapPaste() {
	account := r.cheapAuth
	if account == nil || account.state.Stage != cheapauthui.WaitingForCode ||
		account.state.Exchanging || account.state.LoggingOut {
		return
	}
	attempt := account.attempt
	pasted := account.state.Pasted
	account.state.Exchanging = true
	account.state.Failure = ""
	r.async(func() {
		credentials, err := attempt.flow.ExchangePasted(attempt.ctx, pasted)
		r.finishCheapLogin(attempt, credentials, err, true)
	})
}

func (r *Runtime) finishCheapLogin(attempt *cheapLoginAttempt, credentials core.Credentials, err error, pasted bool) {
	if errors.Is(err, context.Canceled) {
		return
	}
	r.enqueue(func(*ui.Context) {
		account := r.cheapAuth
		if account == nil || account.generation != attempt.cheapAuthGeneration || account.attempt != attempt ||
			attempt.ctx.Err() != nil || account.state.Stage != cheapauthui.WaitingForCode {
			return
		}
		if err != nil {
			if !attempt.committing {
				if pasted {
					account.state.Exchanging = false
				}
				account.state.Failure = cheapAuthError(err)
			}
			return
		}
		attempt.once.Do(func() {
			attempt.committing = true
			account.state.Exchanging = true
			account.state.Failure = ""
			r.async(func() {
				account.effects.Lock()
				saveErr := account.service.Establish(attempt.ctx, credentials)
				account.effects.Unlock()
				r.enqueue(func(c *ui.Context) {
					if account.attempt != attempt || attempt.ctx.Err() != nil {
						return
					}
					account.state.Exchanging = false
					if saveErr != nil {
						attempt.committing = false
						attempt.once = sync.Once{} // no credential was adopted
						account.state.Failure = cheapAuthError(saveErr)
						return
					}
					account.state.Pasted = ""
					account.state.Endpoint = credentials.Endpoint
					generation := account.advance(r.ctx)
					r.startCheapSync(account, generation)
					// The browser round trip may end on any page; the sign-in
					// reports there instead of taking the window to Settings.
					kit.Toast(c, kit.Done, i18n.T(c, "cheaprouter_signed_in"))
				})
			})
		})
	})
}

func (r *Runtime) cancelCheapLogin() {
	account := r.cheapAuth
	if account == nil || account.attempt == nil || account.state.LoggingOut {
		return
	}
	if account.attempt.committing {
		// The save may already be admitted. Order a local clear after it.
		r.logoutCheapAuth()
		return
	}
	account.advance(r.ctx)
	account.state.Stage = cheapauthui.SignedOut
	account.state.Failure, account.state.Pasted, account.state.Exchanging = "", "", false
}

func (r *Runtime) stopCheapSync() {
	account := r.cheapAuth
	if account == nil || account.stopSync == nil || account.state.LoggingOut {
		return
	}
	account.stopSync()
	account.stopSync = nil
	account.state.Stage, account.state.Failure = cheapauthui.GatewayFailed, "已取消本次模型网关配置"
	r.syncImageStudioCatalog()
}

// startCheapSync is one cancellable pass. The account projection is optional;
// only a definite authentication rejection ends the session, and no outcome
// of the pass hides the workspace.
func (r *Runtime) startCheapSync(account *cheapAuthRuntime, generation *cheapAuthGeneration) {
	ctx, cancel := context.WithCancel(generation.ctx)
	account.stopSync = cancel
	account.state.Stage, account.state.Failure = cheapauthui.SyncingGateway, ""
	account.state.AccountFailure = ""
	account.state.AutoBindFailures = nil
	shouldAutoBind := !account.autoBindDone
	r.syncImageStudioCatalog()
	r.async(func() {
		// A group write can outlive its reply. Resolve it before reading the
		// catalog so a confirmed rejection restores the previous binding first.
		account.effects.Lock()
		var gatewayErr error
		if pending := account.pending; pending != nil && pending.binding != nil {
			_, gatewayErr = r.resolveCheapProvider(ctx, account)
		}
		account.effects.Unlock()
		user, accountErr := account.service.Account(ctx)
		var gateway core.Gateway
		var groups []core.Group
		var groupsErr error
		var autoBindAttempted bool
		var autoBindFailures map[string]string
		var statuses []core.GroupStatus
		var statusErr error
		statusAttempted := false
		if !cheapAuthRejected(accountErr) && ctx.Err() == nil {
			groups, groupsErr = account.service.AvailableGroups(ctx)
			if !cheapAuthRejected(groupsErr) && ctx.Err() == nil && gatewayErr == nil {
				gateway, gatewayErr = account.service.GatewayWithGroups(ctx, groups)
				if gatewayErr == nil && len(groups) > 0 {
					_ = account.service.MigrateLegacyGroup(ctx, groups, gateway.Catalog)
				}
				if gatewayErr == nil && groupsErr == nil && shouldAutoBind && ctx.Err() == nil {
					autoBindAttempted = true
					updated, autoResult, autoErr := account.service.AutoBindUnboundLanes(
						ctx,
						groups,
						&core.ModelCatalog{BaseURL: gateway.BaseURL, Items: gateway.Catalog},
						nil,
					)
					if len(autoResult.Failures) > 0 {
						autoBindFailures = make(map[string]string, len(autoResult.Failures))
						for _, failure := range autoResult.Failures {
							autoBindFailures[failure.Lane] = cheapAuthError(failure.Err)
						}
					}
					if autoErr != nil {
						slog.Warn("cheaprouter auto-bind gateway refresh failed", "error", autoErr.Error())
						for lane := range autoResult.Bound {
							_ = account.service.RestoreLaneBinding(ctx, lane, nil, false)
						}
					} else if len(autoResult.Bound) > 0 {
						if _, _, providerErr := providers.CheapRouterProvider(updated); providerErr != nil {
							slog.Warn("cheaprouter auto-bind provider invalid, keeping base gateway", "error", providerErr.Error())
							for lane := range autoResult.Bound {
								_ = account.service.RestoreLaneBinding(ctx, lane, nil, false)
							}
						} else {
							gateway = updated
						}
					}
				}
			}
			// Group status is optional decoration: a failure must not reject the
			// session or block group switching.
			if !cheapAuthRejected(groupsErr) && !cheapAuthRejected(gatewayErr) && ctx.Err() == nil {
				statusAttempted = true
				statuses, statusErr = account.service.GroupStatuses(ctx)
			}
		}
		authErr := accountErr
		if !cheapAuthRejected(authErr) {
			authErr = gatewayErr
		}
		if !cheapAuthRejected(authErr) {
			authErr = groupsErr
		}

		var modelCount int
		var disableErr error
		if ctx.Err() == nil {
			if cheapAuthRejected(authErr) {
				account.effects.Lock()
				if ctx.Err() == nil {
					// Refresh rejection is already cleared by Service. A
					// bearer rejection from /me or the gateway clears here.
					authErr = errors.Join(authErr, account.service.Invalidate())
					// The session is over as after a sign-out: its gateway
					// stops routing, and nothing else of the Node changes.
					disableErr = r.disableCheapGateway(account, generation)
				}
				account.effects.Unlock()
			} else if gatewayErr == nil {
				modelCount, gatewayErr = r.saveCheapGateway(ctx, account, generation, gateway)
			}
		}
		selectedGroup := account.service.SelectedGroup()
		laneBindings := account.service.LaneBindings()
		r.enqueue(func(*ui.Context) {
			if account.generation != generation || ctx.Err() != nil {
				return
			}
			cancel()
			account.stopSync = nil
			if cheapAuthRejected(authErr) {
				account.autoBindDone = false
				account.state.Stage, account.state.Failure = cheapauthui.SignedOut, cheapAuthError(authErr)
				if disableErr != nil {
					account.state.Failure += "；CheapRouter provider 未能停用，请在模型设置中关闭"
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
			account.state.Account = user
			if accountErr != nil {
				account.state.AccountFailure = cheapAuthError(accountErr)
			}
			if groupsErr == nil {
				account.state.Groups = groups
				account.state.GroupsError = ""
			} else if len(account.state.Groups) == 0 {
				account.state.GroupsError = cheapAuthError(groupsErr)
			}
			if statusAttempted {
				account.state.GroupStatusLoading = false
				if statusErr == nil {
					account.state.GroupStatuses = core.IndexGroupStatuses(statuses)
					account.state.GroupStatusLoaded = true
					account.state.GroupStatusError = ""
				} else {
					account.state.GroupStatuses = nil
					account.state.GroupStatusLoaded = false
					account.state.GroupStatusError = cheapAuthError(statusErr)
				}
			}
			if len(gateway.Catalog) > 0 {
				account.state.Catalog = gateway.Catalog
			}
			account.state.SelectedGroupID = selectedGroup
			account.state.LaneBindings = laneBindings
			account.state.AutoBindFailures = autoBindFailures
			if gatewayErr != nil {
				account.state.Stage, account.state.Failure = cheapauthui.GatewayFailed, cheapAuthError(gatewayErr)
				r.syncImageStudioCatalog()
				r.projectModels()
				return
			}
			if autoBindAttempted {
				account.autoBindDone = true
			}
			account.state.ModelCount = modelCount
			account.state.Stage, account.state.Failure = cheapauthui.Ready, ""
			account.state.GroupError = ""
			r.syncImageStudioCatalog()
			r.projectModels()
			r.project()
		})
	})
}

func cheapAuthRejected(err error) bool {
	return errors.Is(err, core.ErrRefresh) || errors.Is(err, core.ErrUnauthorized)
}

func (r *Runtime) saveCheapGateway(ctx context.Context, account *cheapAuthRuntime, generation *cheapAuthGeneration, gateway core.Gateway) (int, error) {
	account.effects.Lock()
	defer account.effects.Unlock()
	if pending := account.pending; pending != nil {
		output, err := r.resolveCheapProvider(ctx, account)
		if err != nil || pending.generation == generation {
			if err != nil {
				return 0, err
			}
			provider, ok := detail[protocol.ConversationProvider](output.Data)
			if !ok {
				return 0, errors.New("provider detail is missing")
			}
			return len(provider.Models), nil
		}
	}

	provider, secret, err := providers.CheapRouterProvider(gateway)
	if err != nil {
		return 0, err
	}
	err = r.saveCheapProvider(ctx, account, generation, provider, secret)
	return len(provider.Models), err
}

func (r *Runtime) saveCheapProvider(ctx context.Context, account *cheapAuthRuntime, generation *cheapAuthGeneration, provider protocol.ConversationProvider, secret *protocol.Secret) error {
	err := r.updateCheapProvider(ctx, account, generation, func(existing *protocol.ConversationProvider) *protocol.CommandSaveProviderPayload {
		provider.Revision = 0
		if existing != nil {
			provider.Revision = existing.Revision
		}
		return &protocol.CommandSaveProviderPayload{Provider: provider, ExpectedRevision: provider.Revision, Secret: secret}
	})
	if err != nil {
		var fault *protocol.Fault
		if !errors.Is(err, context.Canceled) && (!errors.As(err, &fault) || fault.Code != protocol.ErrorCodeInvalidRequest) {
			return &cheapProviderSaveError{err: err}
		}
	}
	return err
}

// updateCheapProvider writes next's answer over the stored CheapRouter
// provider at its current revision; nil leaves the provider as it is. Another
// window's sync or an edit in Settings can move the revision between the read
// and the write: the change is then made once more, to what is stored now.
func (r *Runtime) updateCheapProvider(ctx context.Context, account *cheapAuthRuntime, generation *cheapAuthGeneration, next func(existing *protocol.ConversationProvider) *protocol.CommandSaveProviderPayload) error {
	for reread := false; ; reread = true {
		existing, err := r.readCheapProvider(ctx)
		if err != nil {
			return err
		}
		input := next(existing)
		if input == nil {
			return nil
		}
		err = r.writeCheapProvider(ctx, account, generation, *input)
		if reread || faultCode(err) != protocol.ErrorCodeRevisionConflict {
			return err
		}
	}
}

func (r *Runtime) readCheapProvider(ctx context.Context) (*protocol.ConversationProvider, error) {
	// A previous write can have completed before its event reaches Snapshot.
	output, err := r.controller.Do(ctx, protocol.Command{Kind: "list_providers"})
	if err != nil {
		return nil, err
	}
	list, ok := detail[[]protocol.ConversationProvider](output.Data)
	if !ok {
		return nil, errors.New("provider list is missing")
	}
	for _, provider := range list {
		if provider.ID == providers.CheapRouterProviderID {
			return &provider, nil
		}
	}
	return nil, nil
}

func (r *Runtime) writeCheapProvider(ctx context.Context, account *cheapAuthRuntime, generation *cheapAuthGeneration, input protocol.CommandSaveProviderPayload) error {
	// This is the admission boundary. Once admitted, observe using the
	// runtime's lifetime so logout can run after the write, not race it.
	if err := ctx.Err(); err != nil {
		return err
	}
	request := r.controller.Prepare(protocol.Command{Kind: "save_provider", Data: input})
	_, err := r.controller.Settle(r.ctx, request)
	if client.Retained(err) {
		account.pending = &cheapProviderWrite{generation: generation, request: request}
	}
	return err
}

func (r *Runtime) resolveCheapProvider(ctx context.Context, account *cheapAuthRuntime) (protocol.Output, error) {
	pending := account.pending
	resolution, err := r.controller.Resolve(ctx, pending.request)
	if err != nil {
		return protocol.Output{}, err
	}
	output, outcomeErr := resolution.Result()
	if binding := pending.binding; binding != nil && (resolution.State == client.RequestAbsent || resolution.State == client.RequestFailed) {
		if err := restoreCheapGroup(r.ctx, account, binding); err != nil {
			return protocol.Output{}, errors.Join(outcomeErr, err)
		}
		// The earlier switch failed definitively and was rolled back. Do not
		// fail a newer switch or a gateway retry with that stale result.
		outcomeErr = nil
	}
	if resolution.State != client.RequestPending {
		account.pending = nil
	}
	return output, outcomeErr
}

func restoreCheapGroup(ctx context.Context, account *cheapAuthRuntime, binding *cheapGroupBinding) error {
	if !account.service.SignedIn() || binding == nil {
		return nil
	}
	if binding.legacyMode {
		if !sameGroupID(account.service.SelectedGroup(), binding.legacyTarget) {
			return nil
		}
		return account.service.SetGroup(ctx, binding.legacyPrev)
	}
	current := account.service.LaneBindingFor(binding.lane)
	if !sameLaneBinding(current, binding.target) {
		return nil
	}
	return account.service.RestoreLaneBinding(ctx, binding.lane, binding.previous, binding.previousCleared)
}

func sameLaneBinding(a, b *core.LaneBinding) bool {
	if a == nil || b == nil {
		return a == b
	}
	return a.GroupID == b.GroupID && a.APIKey == b.APIKey
}

func (r *Runtime) bindCheapLaneGroup(account *cheapAuthRuntime, generation *cheapAuthGeneration, lane string, targetGroup *int64, groups []core.Group) (int, []core.Model, map[string]int64, error) {
	account.effects.Lock()
	defer account.effects.Unlock()
	ctx := generation.ctx
	if account.pending != nil {
		if _, err := r.resolveCheapProvider(ctx, account); err != nil {
			return 0, nil, nil, err
		}
	}
	gateway, targetBinding, err := account.service.GatewayForLaneChange(ctx, lane, targetGroup, groups)
	if err != nil {
		return 0, nil, nil, err
	}
	provider, secret, err := providers.CheapRouterProvider(gateway)
	if err != nil {
		return 0, nil, nil, err
	}
	previous := account.service.LaneBindingFor(lane)
	previousCleared := account.service.LaneCleared(lane)
	binding := &cheapGroupBinding{lane: lane, previous: previous, previousCleared: previousCleared, target: targetBinding}
	// Persist first: a local write failure must never replace the working Node
	// route. If Node rejects the change, restore only this lane's prior binding.
	if err := account.service.SetLaneBinding(ctx, lane, targetBinding); err != nil {
		return 0, nil, nil, err
	}
	if err := r.saveCheapProvider(ctx, account, generation, provider, secret); err != nil {
		if account.pending != nil {
			account.pending.binding = binding
			return 0, nil, nil, err
		}
		if faultCode(err) == protocol.ErrorCodeOutcomeUnknown {
			return 0, nil, nil, err // No confirmed result: don't claim rollback.
		}
		return 0, nil, nil, errors.Join(err, restoreCheapGroup(r.ctx, account, binding))
	}
	return len(provider.Models), gateway.Catalog, account.service.LaneBindings(), nil
}

func (r *Runtime) bindCheapGroup(account *cheapAuthRuntime, generation *cheapAuthGeneration, target *int64) (int, error) {
	account.effects.Lock()
	defer account.effects.Unlock()
	ctx := generation.ctx
	if account.pending != nil {
		if _, err := r.resolveCheapProvider(ctx, account); err != nil {
			return 0, err
		}
	}
	gateway, err := account.service.GatewayForGroup(ctx, target)
	if err != nil {
		return 0, err
	}
	provider, secret, err := providers.CheapRouterProvider(gateway)
	if err != nil {
		return 0, err
	}
	binding := &cheapGroupBinding{legacyPrev: account.service.SelectedGroup(), legacyTarget: target, legacyMode: true}
	if err := account.service.SetGroup(ctx, target); err != nil {
		return 0, err
	}
	if err := r.saveCheapProvider(ctx, account, generation, provider, secret); err != nil {
		if account.pending != nil {
			account.pending.binding = binding
			return 0, err
		}
		if faultCode(err) == protocol.ErrorCodeOutcomeUnknown {
			return 0, err // No confirmed result: don't claim rollback.
		}
		return 0, errors.Join(err, restoreCheapGroup(r.ctx, account, binding))
	}
	return len(provider.Models), nil
}

func (r *Runtime) logoutCheapAuth() {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.state.LoggingOut {
		return
	}
	account.state.LoggingOut = true
	account.state.Failure = ""
	generation := account.advance(r.ctx)
	r.async(func() {
		account.effects.Lock()
		// Clear locally even if Node or the revocation endpoint is unreachable.
		logoutErr := account.service.Logout(generation.ctx)
		disableErr := r.disableCheapGateway(account, generation)
		account.effects.Unlock()
		r.enqueue(func(*ui.Context) {
			if account.generation != generation {
				return
			}
			account.state.LoggingOut = false
			account.state.Stage = cheapauthui.SignedOut
			account.state.Account = core.User{}
			account.state.AccountFailure = ""
			account.state.ModelCount = 0
			account.state.Pasted = ""
			account.state.Exchanging = false
			account.state.Groups = nil
			account.state.Catalog = nil
			account.state.GroupsLoading = false
			account.state.GroupsError = ""
			account.state.GroupStatuses = nil
			account.state.GroupStatusLoading = false
			account.state.GroupStatusLoaded = false
			account.state.GroupStatusError = ""
			account.state.SelectedGroupID = nil
			account.state.LaneBindings = nil
			account.state.AutoBindFailures = nil
			account.state.SwitchingGroup = false
			account.state.SwitchingLane = ""
			account.state.GroupError = ""
			r.syncImageStudioCatalog()
			r.projectModels()
			if disableErr != nil || logoutErr != nil {
				account.state.Failure = "本地登录已退出；远程撤销或禁用模型网关未完全成功，请检查网络后重试"
			}
		})
	})
}

func (r *Runtime) refreshCheapGroups() {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.generation == nil ||
		!account.state.SignedIn() || account.state.LoggingOut || account.state.GroupsLoading {
		return
	}
	generation := account.generation
	account.state.GroupsLoading = true
	account.state.GroupsError = ""
	account.state.GroupStatusLoading = true
	r.async(func() {
		groups, groupsErr := account.service.AvailableGroups(generation.ctx)
		var statuses []core.GroupStatus
		var statusErr error
		if generation.ctx.Err() == nil {
			statuses, statusErr = account.service.GroupStatuses(generation.ctx)
		}
		r.enqueue(func(*ui.Context) {
			if account.generation != generation || generation.ctx.Err() != nil || !account.state.SignedIn() {
				return
			}
			account.state.GroupsLoading = false
			account.state.GroupStatusLoading = false
			if groupsErr != nil {
				account.state.GroupsError = cheapAuthError(groupsErr)
			} else {
				account.state.Groups = groups
				account.state.GroupsError = ""
			}
			if statusErr != nil {
				account.state.GroupStatuses = nil
				account.state.GroupStatusLoaded = false
				account.state.GroupStatusError = cheapAuthError(statusErr)
			} else {
				account.state.GroupStatuses = core.IndexGroupStatuses(statuses)
				account.state.GroupStatusLoaded = true
				account.state.GroupStatusError = ""
			}
			r.projectModels()
		})
	})
}

func (r *Runtime) selectCheapLaneGroup(lane string, groupID *int64) {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.generation == nil ||
		!account.state.SignedIn() || account.state.LoggingOut || account.state.SwitchingGroup ||
		account.state.Stage == cheapauthui.SyncingGateway {
		return
	}
	if sameGroupID(account.state.BoundGroupID(lane), groupID) && account.state.GroupError == "" {
		return
	}
	generation := account.generation
	target := copyGroupID(groupID)
	groups := append([]core.Group(nil), account.state.Groups...)
	account.state.SwitchingGroup = true
	account.state.SwitchingLane = lane
	account.state.GroupError = ""
	r.async(func() {
		modelCount, catalog, bindings, err := r.bindCheapLaneGroup(account, generation, lane, target, groups)
		r.enqueue(func(c *ui.Context) {
			if account.generation != generation || generation.ctx.Err() != nil {
				return
			}
			account.state.SwitchingGroup = false
			account.state.SwitchingLane = ""
			if err != nil {
				account.state.GroupError = "切换分组失败：" + cheapAuthError(err)
				kit.Toast(c, kit.Failed, account.state.GroupError)
				return
			}
			account.state.LaneBindings = bindings
			account.state.SelectedGroupID = nil
			if len(catalog) > 0 {
				account.state.Catalog = catalog
			}
			account.state.ModelCount = modelCount
			account.state.GroupError = ""
			if len(account.state.AutoBindFailures) > 0 {
				key := core.NormalizeLane(lane)
				if key == "" {
					key = strings.ToLower(strings.TrimSpace(lane))
				}
				delete(account.state.AutoBindFailures, key)
				if len(account.state.AutoBindFailures) == 0 {
					account.state.AutoBindFailures = nil
				}
			}
			account.state.Stage = cheapauthui.Ready
			account.state.Failure = ""
			r.syncImageStudioCatalog()
			r.project()
			label := core.LaneDisplayName(lane)
			if target == nil {
				kit.Toast(c, kit.Done, "已取消 "+label+" 分组绑定")
			} else {
				kit.Toast(c, kit.Done, "已更新 "+label+" 分组并刷新模型")
			}
		})
	})
}

func (r *Runtime) selectCheapGroup(groupID *int64) {
	account := r.cheapAuth
	if account == nil || account.service == nil || account.generation == nil ||
		!account.state.SignedIn() || account.state.LoggingOut || account.state.SwitchingGroup ||
		account.state.Stage == cheapauthui.SyncingGateway {
		return
	}
	if sameGroupID(account.state.SelectedGroupID, groupID) && account.state.GroupError == "" {
		return
	}
	generation := account.generation
	target := copyGroupID(groupID)
	account.state.SwitchingGroup = true
	account.state.GroupError = ""
	r.async(func() {
		modelCount, err := r.bindCheapGroup(account, generation, target)
		r.enqueue(func(c *ui.Context) {
			if account.generation != generation || generation.ctx.Err() != nil {
				return
			}
			account.state.SwitchingGroup = false
			if err != nil {
				// Keep the last confirmed selection and show the failure (or
				// an unconfirmed write). An account API rejection is not
				// evidence that the previously bound model key has stopped working.
				account.state.GroupError = "切换分组失败：" + cheapAuthError(err)
				kit.Toast(c, kit.Failed, account.state.GroupError)
				return
			}
			account.state.SelectedGroupID = target
			account.state.ModelCount = modelCount
			account.state.GroupError = ""
			account.state.Stage = cheapauthui.Ready
			account.state.Failure = ""
			// Refresh projected models and provider settings immediately so the
			// composer model selector reflects the newly bound group's catalog.
			r.project()
			kit.Toast(c, kit.Done, "已切换分组并刷新模型")
		})
	})
}

func sameGroupID(a, b *int64) bool {
	if a == nil || b == nil {
		return a == b
	}
	return *a == *b
}

func copyGroupID(v *int64) *int64 {
	if v == nil {
		return nil
	}
	copied := *v
	return &copied
}

func (r *Runtime) disableCheapGateway(account *cheapAuthRuntime, generation *cheapAuthGeneration) error {
	ctx := generation.ctx
	if account.pending != nil {
		_, err := r.resolveCheapProvider(ctx, account)
		if err != nil && account.pending != nil {
			return err // An admitted enable may still be running.
		}
	}
	// Disable only CheapRouter; do not alter other providers or captured tasks.
	return r.updateCheapProvider(ctx, account, generation, func(existing *protocol.ConversationProvider) *protocol.CommandSaveProviderPayload {
		if existing == nil || !existing.Enabled {
			return nil
		}
		existing.Enabled = false
		return &protocol.CommandSaveProviderPayload{Provider: *existing, ExpectedRevision: existing.Revision}
	})
}

type cheapProviderSaveError struct {
	err error
}

func (e *cheapProviderSaveError) Error() string { return e.err.Error() }
func (e *cheapProviderSaveError) Unwrap() error { return e.err }

// cheapAuthError turns the sync failure into what the user can act on. A
// protocol fault carries the cause as an English message, so the common ones
// get their own sentence rather than one catch-all line that hides which step
// broke.
func cheapAuthError(err error) string {
	var auth *core.AuthError
	if errors.As(err, &auth) {
		return auth.Error()
	}
	var saveErr *cheapProviderSaveError
	if errors.As(err, &saveErr) {
		return "保存 CheapRouter 模型网关配置失败，请重试（" + saveErr.err.Error() + "）"
	}
	var fault *protocol.Fault
	if errors.As(err, &fault) {
		switch {
		case strings.Contains(fault.Message, "gateway key is empty"):
			return "账号没有可用的模型网关密钥，自动创建也失败了，请稍后重试"
		case strings.Contains(fault.Message, "no chat models"):
			return "账号下没有可用的聊天模型，请确认套餐或模型分组已开通"
		case strings.Contains(fault.Message, "provider endpoint"):
			return "模型网关地址不可用，请稍后重试或切换服务入口"
		case fault.Code == protocol.ErrorCodeInvalidRequest:
			return "CheapRouter 模型网关配置参数无效（" + fault.Message + "）"
		}
	}
	return "配置 CheapRouter 模型网关失败，请检查网络后重试（" + err.Error() + "）"
}
