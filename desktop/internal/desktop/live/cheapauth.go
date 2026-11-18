package live

import (
	"context"
	"errors"
	"path/filepath"
	"sync"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	cheapauthui "github.com/ai-poet/agent-client/desktop/internal/desktop/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/settings"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/shell"
	"github.com/ai-poet/agent-client/desktop/internal/node/providers"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/ui"
)

type cheapAuthRuntime struct {
	service    *core.Service
	state      *cheapauthui.State
	attempt    *cheapLoginAttempt
	generation uint64
	loggingOut bool
}

type cheapLoginAttempt struct {
	flow       *core.LoginFlow
	generation uint64
	once       sync.Once
}

func (r *Runtime) initCheapAuth() {
	profile := filepath.Dir(filepath.Dir(r.Preferences.Path))
	service, err := core.NewService(profile)
	state := cheapauthui.NewState()
	account := &cheapAuthRuntime{service: service, state: state}
	r.cheapAuth = account
	r.App.Settings.CheapRouter = state
	state.Start = func(manual bool) { r.startCheapLogin(manual) }
	state.SubmitPaste = r.submitCheapPaste
	state.Cancel = r.cancelCheapLogin
	state.Retry = func() {
		if r.cheapAuth != nil && r.cheapAuth.service.SignedIn() {
			state.Stage, state.Failure = cheapauthui.SyncingGateway, ""
			r.startCheapSync(r.cheapAuth.generation)
		}
	}
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
		state.Stage = cheapauthui.SyncingGateway
		r.startCheapSync(0)
	}
}

func (r *Runtime) startCheapLogin(manual bool) {
	account := r.cheapAuth
	if account == nil || account.state.Stage == cheapauthui.Starting {
		return
	}
	if account.attempt != nil {
		account.attempt.flow.Close()
	}
	account.generation++
	generation := account.generation
	state := account.state
	endpoint := state.Endpoint
	state.Stage, state.Failure, state.Pasted, state.Exchanging = cheapauthui.Starting, "", "", false
	r.async(func() {
		var flow *core.LoginFlow
		var err error
		if manual {
			flow, err = account.service.StartManualLogin(endpoint)
		} else {
			flow, err = account.service.StartLogin(endpoint)
		}
		if err != nil {
			r.enqueue(func(*ui.Context) {
				if account.generation == generation {
					state.Stage, state.Failure = cheapauthui.SignedOut, cheapAuthError(err)
				}
			})
			return
		}
		attempt := &cheapLoginAttempt{flow: flow, generation: generation}
		r.enqueue(func(c *ui.Context) {
			if account.generation != generation {
				flow.Close()
				return
			}
			account.attempt = attempt
			state.Stage, state.LoginURL, state.Loopback = cheapauthui.WaitingForCode, flow.LoginURL(), flow.Loopback()
			c.OpenURL(flow.LoginURL())
		})
		if !manual {
			credentials, waitErr := flow.Wait(r.ctx)
			r.finishCheapLogin(attempt, credentials, waitErr)
		}
	})
}

func (r *Runtime) submitCheapPaste() {
	account := r.cheapAuth
	if account == nil || account.attempt == nil || account.state.Exchanging {
		return
	}
	attempt := account.attempt
	pasted := account.state.Pasted
	account.state.Exchanging = true
	account.state.Failure = ""
	r.async(func() {
		credentials, err := attempt.flow.ExchangePasted(r.ctx, pasted)
		r.enqueue(func(*ui.Context) {
			if account.generation == attempt.generation {
				account.state.Exchanging = false
			}
		})
		r.finishCheapLogin(attempt, credentials, err)
	})
}

func (r *Runtime) finishCheapLogin(attempt *cheapLoginAttempt, credentials core.Credentials, err error) {
	attempt.once.Do(func() {
		r.enqueue(func(*ui.Context) {
			account := r.cheapAuth
			if account == nil || account.generation != attempt.generation || account.attempt != attempt {
				attempt.flow.Close()
				return
			}
			account.state.Exchanging = false
			if err != nil {
				if errors.Is(err, context.Canceled) {
					return
				}
				account.state.Failure = cheapAuthError(err)
				account.state.Stage = cheapauthui.WaitingForCode
				return
			}
			attempt.flow.Close()
			account.state.Pasted = ""
			account.state.Stage = cheapauthui.SyncingGateway
			r.startCheapSync(attempt.generation)
		})
	})
}

func (r *Runtime) cancelCheapLogin() {
	account := r.cheapAuth
	if account == nil {
		return
	}
	account.generation++
	if account.attempt != nil {
		account.attempt.flow.Close()
		account.attempt = nil
	}
	account.state.Stage = cheapauthui.SignedOut
	account.state.Failure, account.state.Pasted, account.state.Exchanging = "", "", false
}

// startCheapSync resolves the complete account/provider collection in one
// background pass. A superseded login/logout generation cannot overwrite it.
func (r *Runtime) startCheapSync(generation uint64) {
	account := r.cheapAuth
	if account == nil {
		return
	}
	r.async(func() {
		user, err := account.service.Account(r.ctx)
		var gateway core.Gateway
		var provider protocol.ConversationProvider
		var secret *protocol.Secret
		if err == nil {
			gateway, err = account.service.Gateway(r.ctx)
		}
		if err == nil {
			provider, secret, err = providers.CheapRouterProvider(gateway)
		}
		if err == nil {
			expected := uint64(0)
			for _, existing := range r.controller.Snapshot().Providers {
				if existing.ID == providers.CheapRouterProviderID {
					expected = existing.Revision
					provider.Revision = existing.Revision
					break
				}
			}
			_, err = r.controller.Do(r.ctx, protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{
				Provider: provider, ExpectedRevision: expected, Secret: secret,
			}})
		}
		r.enqueue(func(*ui.Context) {
			if r.cheapAuth != account || account.generation != generation {
				return
			}
			if err != nil {
				account.state.Stage = cheapauthui.SyncingGateway
				account.state.Failure = cheapAuthError(err)
				if !account.service.SignedIn() {
					account.state.Stage = cheapauthui.SignedOut
				}
				return
			}
			account.state.Account = user
			account.state.ModelCount = len(provider.Models)
			account.state.Endpoint = account.service.Endpoint()
			account.state.Stage, account.state.Failure = cheapauthui.Ready, ""
			if generation > 0 {
				// A fresh sign-in lands on the account/plan confirmation page;
				// startup restoration (generation zero) keeps the user's page.
				r.App.Settings.Open(settings.CheapAccount)
				r.App.Navigate(shell.SettingsPage)
			}
		})
	})
}

func (r *Runtime) logoutCheapAuth() {
	account := r.cheapAuth
	if account == nil || account.loggingOut {
		return
	}
	account.loggingOut = true
	account.state.LoggingOut = true
	account.state.Failure = ""
	account.generation++
	generation := account.generation
	if account.attempt != nil {
		account.attempt.flow.Close()
		account.attempt = nil
	}
	r.async(func() {
		// Disable only the provider created for CheapRouter. User-installed
		// providers and all other session configuration remain intact.
		var disableErr error
		for _, existing := range r.controller.Snapshot().Providers {
			if existing.ID != providers.CheapRouterProviderID || !existing.Enabled {
				continue
			}
			existing.Enabled = false
			_, disableErr = r.controller.Do(r.ctx, protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{
				Provider: existing, ExpectedRevision: existing.Revision,
			}})
			break
		}
		logoutErr := account.service.Logout(context.Background())
		r.enqueue(func(*ui.Context) {
			if r.cheapAuth != account || account.generation != generation {
				return
			}
			account.loggingOut = false
			account.state.LoggingOut = false
			account.state.Stage = cheapauthui.SignedOut
			account.state.Account = core.User{}
			account.state.ModelCount = 0
			account.state.Pasted = ""
			if disableErr != nil || logoutErr != nil {
				account.state.Failure = "本地登录已退出；远程撤销或禁用模型网关未完全成功，请检查网络后重试"
			}
		})
	})
}

// CheapAuthGate is installed by the desktop composition root. Embedders and
// isolated view tests can render the shared shell without an account gate.
func (r *Runtime) CheapAuthGate(c *ui.Context) bool {
	if r.cheapAuth == nil {
		return false
	}
	return cheapauthui.Gate(c, r.cheapAuth.state)
}

func cheapAuthError(err error) string {
	var auth *core.AuthError
	if errors.As(err, &auth) {
		return auth.Error()
	}
	return "配置 CheapRouter 模型网关失败，请检查网络后重试"
}
