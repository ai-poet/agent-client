package live

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"sync"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/client"
	cheapauthui "github.com/ai-poet/agent-client/desktop/internal/desktop/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/node/providers"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// The UI owns generation changes. Background mutations share effects: after
// taking it, a worker must check its generation's context before admission.
// Already admitted writes settle before another mutation can overtake them.
type cheapAuthRuntime struct {
	service    *core.Service
	state      *cheapauthui.State
	generation *cheapAuthGeneration
	attempt    *cheapLoginAttempt
	stopSync   context.CancelFunc

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
}

func (a *cheapAuthRuntime) advance(parent context.Context) *cheapAuthGeneration {
	if a.generation != nil {
		a.generation.cancel()
	}
	a.stopSync, a.attempt = nil, nil
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
		if state.Stage == cheapauthui.GatewayFailed && !state.LoggingOut {
			r.startCheapSync(account, account.generation)
		}
	}
	state.StopSync = r.stopCheapSync
	state.Logout = r.logoutCheapAuth
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
}

// startCheapSync is one cancellable pass. The account projection is optional;
// only a definite authentication rejection ends the session, and no outcome
// of the pass hides the workspace.
func (r *Runtime) startCheapSync(account *cheapAuthRuntime, generation *cheapAuthGeneration) {
	ctx, cancel := context.WithCancel(generation.ctx)
	account.stopSync = cancel
	account.state.Stage, account.state.Failure = cheapauthui.SyncingGateway, ""
	account.state.AccountFailure = ""
	r.async(func() {
		user, accountErr := account.service.Account(ctx)
		var gateway core.Gateway
		var gatewayErr error
		if !cheapAuthRejected(accountErr) && ctx.Err() == nil {
			gateway, gatewayErr = account.service.Gateway(ctx)
		}
		authErr := accountErr
		if !cheapAuthRejected(authErr) {
			authErr = gatewayErr
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
		r.enqueue(func(*ui.Context) {
			if account.generation != generation || ctx.Err() != nil {
				return
			}
			cancel()
			account.stopSync = nil
			if cheapAuthRejected(authErr) {
				account.state.Stage, account.state.Failure = cheapauthui.SignedOut, cheapAuthError(authErr)
				if disableErr != nil {
					account.state.Failure += "；CheapRouter provider 未能停用，请在模型设置中关闭"
				}
				account.state.Account, account.state.ModelCount = core.User{}, 0
				return
			}
			account.state.Account = user
			if accountErr != nil {
				account.state.AccountFailure = cheapAuthError(accountErr)
			}
			if gatewayErr != nil {
				account.state.Stage, account.state.Failure = cheapauthui.GatewayFailed, cheapAuthError(gatewayErr)
				return
			}
			account.state.ModelCount = modelCount
			account.state.Stage, account.state.Failure = cheapauthui.Ready, ""
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
	err = r.updateCheapProvider(ctx, account, generation, func(existing *protocol.ConversationProvider) *protocol.CommandSaveProviderPayload {
		provider.Revision = 0
		if existing != nil {
			provider.Revision = existing.Revision
		}
		return &protocol.CommandSaveProviderPayload{Provider: provider, ExpectedRevision: provider.Revision, Secret: secret}
	})
	if err != nil {
		var fault *protocol.Fault
		if !errors.Is(err, context.Canceled) && (!errors.As(err, &fault) || fault.Code != protocol.ErrorCodeInvalidRequest) {
			return 0, &cheapProviderSaveError{err: err}
		}
		return 0, err
	}
	return len(provider.Models), nil
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
	resolution, err := r.controller.Resolve(ctx, account.pending.request)
	if err != nil {
		return protocol.Output{}, err
	}
	if resolution.State != client.RequestPending {
		account.pending = nil
	}
	return resolution.Result()
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
			if disableErr != nil || logoutErr != nil {
				account.state.Failure = "本地登录已退出；远程撤销或禁用模型网关未完全成功，请检查网络后重试"
			}
		})
	})
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
