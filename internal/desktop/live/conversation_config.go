package live

import (
	"encoding/json"
	"maps"
	"reflect"
	"slices"

	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/composer"
	nodeproviders "github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func (r *Runtime) selectedComposerConfig(id string, c *composer.State, models []composer.Model) (protocol.SessionConfig, error) {
	effort := ""
	if chosen := c.Chosen(models); chosen.Reasoning {
		effort = c.EffortName(nil, chosen)
	}
	config, _, err := r.composerConfig(id, c, composer.Message{Model: c.Model, Mode: c.Mode, Permission: c.Permission, Features: c.Features, Effort: effort})
	return config, err
}

func (r *Runtime) composerKey(id string, c *composer.State) string {
	config, err := r.selectedComposerConfig(id, c, r.models())
	if err != nil {
		return ""
	}
	value, _ := json.Marshal(config)
	return string(value)
}

func (r *Runtime) defaultComposer(c *composer.State) {
	r.defaultComposerForFamily(c, "")
}

func (r *Runtime) defaultComposerForFamily(c *composer.State, preferredFamily string) {
	snapshot := r.snapshot()
	if preferredFamily == "" {
		if config := snapshot.Defaults.Config; config != nil {
			for _, p := range snapshot.Providers {
				if p.Enabled && r.listedProviderUsable(p) && p.ID == config.Provider && slices.ContainsFunc(p.Models, func(m protocol.ConversationModel) bool { return m.ID == config.Model }) {
					r.applyComposer(c, *config)
					return
				}
			}
		}
	}
	var candidates []composer.Model
	for _, p := range r.composerProviders() {
		if !p.Enabled || len(p.Models) == 0 || !r.draftProviderUsable(p.ID) {
			continue
		}
		for _, m := range p.Models {
			display := composer.Model{ID: modelKey(p.ID, m.ID), Family: composerModelVendor(p, m)}
			if m.DisplayName != nil {
				display.Name = *m.DisplayName
			}
			candidates = append(candidates, display)
		}
	}
	if len(candidates) == 0 {
		return
	}
	c.Model, c.Effort = composer.DefaultModelForFamily(candidates, preferredFamily).ID, 0
	for _, display := range r.models() {
		if display.ID == c.Model {
			if i := slices.Index(display.Efforts, display.DefaultEffort); i >= 0 {
				c.Effort = float64(i)
			}
			break
		}
	}
}

// followCheapSession keeps a conversation on its model after lanes replaced
// the unbound CheapRouter route it was selected on. The composer moves to the
// lane that carries the model, or to the lane route reconciliation already
// saved for the session. It reports whether that lane is still to be saved.
func (r *Runtime) followCheapSession(session protocol.Session, comp *composer.State) bool {
	unbound := nodeproviders.CheapRouterProviderID
	if r.parent != nil || session.Profile != nil || comp.Model != modelKey(unbound, session.Config.Model) {
		return false
	}
	current := slices.DeleteFunc(slices.Clone(r.snapshot().Providers), func(p protocol.ConversationProvider) bool { return !r.listedProviderUsable(p) })
	if session.Config.Provider != unbound {
		retired := slices.ContainsFunc(current, func(p protocol.ConversationProvider) bool { return p.ID == unbound && !p.Enabled })
		if retired && nodeproviders.IsCheapRouterProvider(session.Config.Provider) {
			id := session.ID.String()
			r.applyComposer(comp, session.Config)
			r.composerObserved[id] = r.composerKey(id, comp)
		}
		return false
	}
	var lanes map[string]string
	if hints := r.cheapModelGroupHints(); hints != nil {
		lanes = hints.modelLanes
	}
	successor, ok := nodeproviders.CheapRouterSuccessor(unbound, session.Config.Model, lanes, current)
	if !ok {
		return false
	}
	config := session.Config
	config.Provider, config.Credential = successor.ID, successor.Credential
	r.applyComposer(comp, config)
	return true
}

// cheapRoutesSettled reports that no route reconciliation is under way. Its
// worker moves the sessions of a replaced route itself.
func (r *Runtime) cheapRoutesSettled() bool {
	account := r.cheapAuth
	return account == nil || account.state == nil ||
		account.state.Stage != cheapauthui.SyncingGateway && !account.state.SwitchingGroup && !account.state.LoggingOut
}

// A send waits for this controller's in-flight configuration save. A failed or
// uncertain save preserves the draft and never admits the captured message.
func (r *Runtime) withComposerConfig(session protocol.Session, config protocol.SessionConfig, next func(protocol.Session)) {
	id := session.ID.String()
	if r.teams != nil {
		if state := r.teams.composers[id]; state != nil {
			if state.pending != nil {
				before := state.baseConfig
				r.withComposerTeam(session, state.composer, func(current protocol.Session) {
					r.withComposerConfig(current, composerConfigAfterTeam(before, config, current.Config), next)
				})
				return
			}
			if state.saving {
				before := state.baseConfig
				state.waiters = append(state.waiters, func(ok bool) {
					if ok {
						if current, exists := r.session(id); exists {
							r.withComposerConfig(current, composerConfigAfterTeam(before, config, current.Config), next)
						}
					}
				})
				return
			}
		}
	}
	if r.composerSaving[id] {
		r.composerWaiters[id] = append(r.composerWaiters[id], func(ok bool) {
			if ok {
				if current, yes := r.session(id); yes {
					r.withComposerConfig(current, config, next)
				}
			}
		})
		return
	}
	before, _ := json.Marshal(session.Config)
	after, _ := json.Marshal(config)
	if string(before) == string(after) {
		next(session)
		return
	}
	r.saveComposerConfig(session, config, func(ok bool) {
		if ok {
			if current, yes := r.session(id); yes {
				next(current)
			}
		}
	})
}

func composerConfigAfterTeam(before, selected, current protocol.SessionConfig) protocol.SessionConfig {
	if selected.Provider == before.Provider && selected.Model == before.Model {
		selected.Provider, selected.Model, selected.Credential = current.Provider, current.Model, current.Credential
		if reflect.DeepEqual(selected.Effort, before.Effort) {
			selected.Effort = current.Effort
		}
		if maps.Equal(selected.Features, before.Features) {
			selected.Features = current.Features
		}
	}
	return selected
}

func (r *Runtime) saveComposerConfig(session protocol.Session, config protocol.SessionConfig, done func(bool)) {
	id := session.ID.String()
	r.composerSaving[id] = true
	value, _ := json.Marshal(config)
	r.composerObserved[id] = string(value)
	r.execute(protocol.Command{Kind: "set_session_config", Data: protocol.CommandSetSessionConfigPayload{Session: session.ID, ExpectedRevision: session.Revision, Config: config}}, func(c *ui.Context, output protocol.Output, err error) {
		ok := err == nil
		if ok {
			updated, valid := detail[protocol.Session](output.Data)
			ok = valid && updated.ID == session.ID && updated.Revision > session.Revision
			if ok {
				r.admittedSessions[id] = updated
				r.composerRevisions[id] = updated.Revision
			} else {
				err = fileActionFault(protocol.ErrorCodeOutcomeUnknown, "session configuration result is unavailable")
			}
		}
		if !ok {
			if !r.failUnsubmitted(c, id, err) {
				r.report(c, err, "session_changed")
			}
		}
		r.composerSaving[id] = false
		waiters := r.composerWaiters[id]
		delete(r.composerWaiters, id)
		if done != nil {
			done(ok)
		}
		for _, f := range waiters {
			f(ok)
		}
	})
}
