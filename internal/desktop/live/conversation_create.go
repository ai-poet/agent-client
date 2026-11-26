package live

import (
	"encoding/json"
	"slices"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/client"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/composer"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

type conversationCreation struct {
	request protocol.Request
	actions *conversationActions
	finish  func(protocol.Output, bool)
	// A replacement binding may recover an empty session, but it must not
	// implicitly submit the captured turn through that replacement.
	detached bool
}

type detachedConversationCreation struct {
	execution protocol.NodeID
	request   protocol.Request
	message   *composer.Message
	draft     string
}

// The window retains the exact receipt, not the previous binding's callbacks
// or attachment ownership. Plugin drafts end with their mounted views.
func (r *Runtime) detachConversationCreations() {
	root := r.localRuntime()
	for id, pending := range r.conversationCreations {
		delete(r.conversationCreations, id)
		delete(r.conversationCreating, id)
		r.discardOutgoing(id)
		if _, open := root.openDrafts[id]; !open || root.App.Data.Find(id) == nil {
			continue
		}
		x := pending.actions
		var message *composer.Message
		if x.restore != nil && len(x.restore.Attachments) == 0 {
			message = x.restore
		}
		x.comp.SetAttachmentCallbacks(nil)
		if root.detachedCreations == nil {
			root.detachedCreations = map[string]detachedConversationCreation{}
		}
		root.detachedCreations[id] = detachedConversationCreation{
			execution: r.controller.Target(), request: pending.request, message: message, draft: x.restoreDraft,
		}
	}
}

func (r *Runtime) restoreConversationCreations() {
	root := r.localRuntime()
	for id, pending := range root.detachedCreations {
		if pending.execution != r.controller.Target() {
			continue
		}
		delete(root.detachedCreations, id)
		cv := r.App.Data.Find(id)
		if cv == nil || !unsentDraft(cv) {
			continue
		}
		comp := r.App.Composer(id)
		cv.Node, comp.Host = pending.execution.String(), pending.execution.String()
		r.conversationCreations[id] = conversationCreation{
			request: pending.request, detached: true,
			actions: &conversationActions{r: r, cv: cv, comp: comp, restore: pending.message, restoreDraft: pending.draft},
		}
		r.readsConnected = false
	}
}

// listedProviderUsable mirrors the credential owners this controller can use.
// A followed provider keeps its source identity in the executor's cache;
// a controller-owned provider forwarded to another Node is checked at entry.
func (r *Runtime) listedProviderUsable(p protocol.ConversationProvider) bool {
	if p.Credential == nil {
		return true
	}
	if p.Source == nil {
		return p.Credential.Node == r.controller.Target()
	}
	return r.parent != nil && p.Source.Node == r.parent.controller.Target() && p.Credential.Node == p.Source.Node
}

func (r *Runtime) capturedProviderUsable(profile *protocol.SessionProfile) bool {
	return profile != nil && (profile.Provider.Credential == nil ||
		profile.Provider.Credential.Node == r.controller.Target() ||
		r.parent != nil && profile.Source == r.parent.controller.Target() && profile.Provider.Credential.Node == profile.Source)
}

// composerProviders are the providers a composer of this Node resolves its
// model in: the Node's own; on another Node the local Node's as well, whose
// configuration a new session there is forwarded with; and the private
// provider of each session that was created that way.
func (r *Runtime) composerProviders() []protocol.ConversationProvider {
	snapshot := r.snapshot()
	providers := slices.DeleteFunc(slices.Clone(snapshot.Providers), func(p protocol.ConversationProvider) bool { return !r.listedProviderUsable(p) })
	if r.parent != nil {
		for _, provider := range r.parent.snapshot().Providers {
			// A followed provider already resolves on this execution Node. Keep
			// that copy, as creationCommand does, and offer its models once.
			if r.parent.listedProviderUsable(provider) && !slices.ContainsFunc(snapshot.Providers, func(p protocol.ConversationProvider) bool { return p.ID == provider.ID }) {
				providers = append(providers, provider)
			}
		}
	}
	private := func(session protocol.Session) {
		if r.capturedProviderUsable(session.Profile) && !slices.ContainsFunc(providers, func(p protocol.ConversationProvider) bool { return p.ID == session.Profile.Provider.ID }) {
			providers = append(providers, session.Profile.Provider)
		}
	}
	for _, session := range snapshot.Sessions {
		private(session)
	}
	for _, session := range r.admittedSessions {
		private(session)
	}
	return providers
}

// forwards reports whether provider is the local Node's while this Node is
// another: a session created with it is forwarded by the local Node.
func (r *Runtime) forwards(provider protocol.ProviderID) bool {
	is := func(p protocol.ConversationProvider) bool { return p.ID == provider }
	return r.parent != nil && !slices.ContainsFunc(r.snapshot().Providers, is) && slices.ContainsFunc(r.parent.snapshot().Providers, func(p protocol.ConversationProvider) bool { return is(p) && r.parent.listedProviderUsable(p) })
}

func (r *Runtime) listedComposerProvider(id protocol.ProviderID) (protocol.ConversationProvider, bool) {
	for _, p := range r.snapshot().Providers {
		if p.ID == id && r.listedProviderUsable(p) {
			return p, true
		}
	}
	return protocol.ConversationProvider{}, false
}

func (r *Runtime) draftProviderUsable(id protocol.ProviderID) bool {
	_, listed := r.listedComposerProvider(id)
	return listed || r.forwards(id)
}

// composerProvider resolves a selection using the same credential scope as
// the Node. A capture applies to its own session, never to another draft.
func (r *Runtime) composerProvider(id string, provider protocol.ProviderID) (protocol.ConversationProvider, bool) {
	if session, exists := r.session(id); exists {
		if profile := session.Profile; profile != nil && profile.Provider.ID == provider {
			return profile.Provider, r.capturedProviderUsable(profile)
		}
	} else if r.forwards(provider) {
		return r.parent.listedComposerProvider(provider)
	}
	return r.listedComposerProvider(provider)
}

// offeredModels are the models conversation id's composer offers: those of
// the Node's own providers; for a draft on another Node the local Node's as
// well; for a forwarded session its private provider's.
func (r *Runtime) offeredModels(id string, models []composer.Model) []composer.Model {
	allowed := map[string]bool{}
	for _, provider := range r.snapshot().Providers {
		if r.listedProviderUsable(provider) {
			allowed[provider.ID.String()] = true
		}
	}
	if session, exists := r.session(id); !exists {
		if r.parent != nil {
			for _, provider := range r.parent.snapshot().Providers {
				if r.forwards(provider.ID) {
					allowed[provider.ID.String()] = true
				}
			}
		}
	} else if profile := session.Profile; profile != nil {
		allowed[profile.Provider.ID.String()] = r.capturedProviderUsable(profile)
	}
	var offered []composer.Model
	for _, model := range models {
		if provider, _, _ := strings.Cut(model.ID, "/"); allowed[provider] {
			offered = append(offered, model)
		}
	}
	return offered
}

// memberModels are those of offered a team member may take: nil for all of
// them on the local Node, and on another Node only that Node's own, since a
// member's model is resolved there and the local Node's providers reach it
// only as the lead of a forwarded session.
func (r *Runtime) memberModels(offered []composer.Model) []composer.Model {
	if r.parent == nil {
		return nil
	}
	own := []composer.Model{}
	for _, model := range offered {
		if model.Unavailable {
			continue
		}
		provider, _, _ := strings.Cut(model.ID, "/")
		if slices.ContainsFunc(r.snapshot().Providers, func(p protocol.ConversationProvider) bool { return p.ID.String() == provider }) {
			own = append(own, model)
		}
	}
	return own
}

// creationCommand creates a session on this Node with config: by the Node
// itself where the provider is its own, and forwarded by the local Node,
// which holds the provider at revision and its credential, where it is not.
func (r *Runtime) creationCommand(project *protocol.ProjectID, worktree *protocol.WorktreeID, config protocol.SessionConfig, revision uint64) protocol.Command {
	if r.forwards(config.Provider) {
		return protocol.Command{Kind: "create_session_at", Data: protocol.CommandCreateSessionAtPayload{Target: r.controller.Target(), Project: project, Worktree: worktree, Config: &config, ProviderRevision: revision}}
	}
	return protocol.Command{Kind: "create_session", Data: protocol.CommandCreateSessionPayload{Project: project, Worktree: worktree, Config: &config}}
}

// executeCreation sends a creation command: a forwarded one through the
// local Node's controller, any other through this Node's.
func (r *Runtime) executeCreation(scope string, command protocol.Command, finished func(*ui.Context, protocol.Output, error)) {
	if command.Kind == "create_session_at" && r.parent != nil {
		r.parent.executeScoped(scope, command, finished)
		return
	}
	r.executeScoped(scope, command, finished)
}

// reportCreation toasts why a session was not created. A model that is not
// configured, as a provider the local Node no longer has or an account
// sign-in that stays with the Node it was made on, is its own failure.
func (r *Runtime) reportCreation(c *ui.Context, err error) {
	if faultCode(err) == protocol.ErrorCodeNotConfigured {
		kit.Toast(c, kit.Failed, i18n.T(c, "chat_config_unavailable"))
		return
	}
	r.report(c, err, "session_changed")
}

// adoptForwarded shows a forwarded session's own configuration, whose
// provider is the private copy the execution Node made.
func (r *Runtime) adoptForwarded(comp *composer.State, session protocol.Session) {
	if session.Profile != nil {
		r.applyComposer(comp, session.Config)
	}
}

func (r *Runtime) creationOwner(command protocol.Command) *Runtime {
	if command.Kind == "create_session_at" && r.parent != nil {
		return r.parent
	}
	return r
}

// resumeCreation observes a dispatched creation on the Node that received it.
// Only an explicitly replayable forward can execute again with its original ID.
func (r *Runtime) resumeCreation(before string, pending conversationCreation, first, explicit bool) {
	owner := r.creationOwner(pending.request.Command)
	if r.conversationCreating[before] || !r.controller.Connected() ||
		!owner.controller.Connected() || owner.controller.Target() != pending.request.Target {
		return
	}
	r.conversationCreating[before] = true
	if !r.async(func() {
		var out protocol.Output
		var err error
		terminal := false
		if first {
			out, err = owner.controller.Settle(r.ctx, pending.request)
			code := faultCode(err)
			terminal = err == nil || code != "" && code != protocol.ErrorCodeOutcomeUnknown
		} else {
			resolution, failure := owner.controller.Resolve(r.ctx, pending.request)
			if failure != nil {
				err = fileActionFault(protocol.ErrorCodeOutcomeUnknown, "session creation outcome is unavailable")
			} else if resolution.State == client.RequestReplayable && (!pending.detached || explicit) {
				out, err = owner.controller.Settle(r.ctx, pending.request)
				// Replay can fail after a prior effect. Resolve that receipt on
				// the next recovery rather than release it based on this failure.
				terminal = err == nil
			} else {
				out, err = resolution.Result()
				// A denied inspection can hide an already-applied result.
				terminal = resolution.State == client.RequestApplied || resolution.State == client.RequestAbsent ||
					resolution.State == client.RequestFailed && faultCode(err) != protocol.ErrorCodePermissionDenied
			}
		}
		// The entry receipt alone does not establish the replacement
		// executor's current access to a forwarded session.
		if pending.detached && owner != r && err == nil {
			if session, ok := detail[protocol.Session](out.Data); out.Kind == "session" && ok && session.ID != (protocol.SessionID{}) {
				visible, failure := r.controller.Do(r.ctx, protocol.Command{Kind: "read_session", Data: protocol.CommandReadSessionPayload{Session: session.ID}})
				current, valid := detail[protocol.Session](visible.Data)
				if failure != nil || visible.Kind != "session" || !valid || current.ID != session.ID || current.Revision < session.Revision {
					err = fileActionFault(protocol.ErrorCodeOutcomeUnknown, "recovered session is unavailable on its execution Node")
					terminal = false
				} else {
					out = visible
				}
			}
		}
		r.enqueue(func(c *ui.Context) {
			current, ok := r.conversationCreations[before]
			if r.ctx.Err() != nil || !ok || current.request.ID != pending.request.ID {
				return
			}
			delete(r.conversationCreating, before)
			x := *pending.actions
			x.c = c
			if err != nil {
				r.discardUnsubmitted(before)
				if terminal {
					delete(r.conversationCreations, before)
				}
				if terminal && x.restore != nil && x.restoreDraft != composerFingerprint(x.comp) {
					x.restoreFailedDraft(c, err)
				} else {
					r.reportCreation(c, err)
				}
				return
			}
			session, valid := detail[protocol.Session](out.Data)
			if out.Kind != "session" || !valid || session.ID == (protocol.SessionID{}) || session.Revision == 0 {
				r.discardUnsubmitted(before)
				r.reportCreation(c, fileActionFault(protocol.ErrorCodeOutcomeUnknown, "session creation result is unavailable"))
				return
			}
			delete(r.conversationCreations, before)
			x.adoptCreation(before, session, pending.finish, !pending.detached)
		})
	}) {
		delete(r.conversationCreating, before)
	}
}

func (x *conversationActions) adoptCreation(before string, session protocol.Session, finish func(protocol.Output, bool), submit bool) {
	if submit {
		x.stageOutgoing(*x.restore)
	} else if x.restore != nil && x.comp.Draft == "" && len(x.comp.References) == 0 && len(x.comp.Attachments) == 0 {
		x.comp.Draft = x.restore.Text
		x.comp.References = slices.Clone(x.restore.References)
	}
	x.r.admittedSessions[session.ID.String()] = session
	x.r.App.ReplaceConversationID(before, session.ID.String())
	if pending := x.r.composerOutgoing[before]; pending != nil {
		delete(x.r.composerOutgoing, before)
		x.r.composerOutgoing[session.ID.String()] = pending
	}
	x.r.browserAdopt(before, session.ID.String())
	x.cv.ID = session.ID.String()
	x.r.composerInitialized[x.cv.ID] = true
	x.r.composerRevisions[x.cv.ID] = session.Revision
	x.r.composerBases[x.comp] = session.Config
	x.r.adoptForwarded(x.comp, session)
	key, _ := json.Marshal(session.Config)
	x.r.composerObserved[x.cv.ID] = string(key)
	// The created session stays selected only where its draft still is.
	x.r.project()
	if x.created != nil {
		x.created(session)
	}
	if submit {
		x.r.withComposerTeam(session, x.comp, func(current protocol.Session) {
			x.turn(protocol.Command{Kind: "submit_turn", Data: protocol.CommandSubmitTurnPayload{Session: current.ID, ExpectedRevision: current.Revision, Message: conversationInput(*x.restore)}}, finish)
		})
	}
}
