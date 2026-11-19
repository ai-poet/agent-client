package live

import (
	"context"

	"github.com/ai-poet/agent-client/desktop/internal/client"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/conversation"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func (r *Runtime) enrichDetails(cv *viewmodel.Conversation, state client.ConversationState, locales ...string) {
	type source struct {
		ref        protocol.ToolDetailReference
		summary    *protocol.ToolDetailSummary
		historical *client.HistoricalCommandReference
	}
	page := state.Snapshot.Page
	sources := map[string]source{}
	pending := map[string][]toolAssociation{}
	for _, entry := range page.Entries {
		for index, part := range entry.Parts {
			var id *string
			var name string
			var summary *protocol.ToolDetailSummary
			key := toolCallKey(entry.ID, uint64(index))
			switch part.Kind {
			case "tool_call":
				if p, ok := detail[protocol.ConversationPartToolCallPayload](part.Data); ok {
					id, name, summary = p.ID, p.Name, p.Summary
					if id != nil {
						k := toolPendingKey(entry.Turn, entry.Branch, *id)
						pending[k] = append(pending[k], toolAssociation{ID: key, Name: name})
					}
				}
			case "tool_result":
				if p, ok := detail[protocol.ConversationPartToolResultPayload](part.Data); ok {
					id, name, summary = p.ID, p.Name, p.Summary
					if id != nil {
						k := toolPendingKey(entry.Turn, entry.Branch, *id)
						matches := pending[k]
						delete(pending, k)
						if len(matches) == 1 && matches[0].Name == name {
							key = matches[0].ID
						}
					}
				}
			}
			if summary == nil {
				continue
			}
			ref := protocol.ToolDetailReference{Session: page.Session, Turn: entry.Turn, Entry: entry.ID, Index: uint64(index), Call: id, Revision: summary.Revision}
			var historical *client.HistoricalCommandReference
			if part.Kind == "tool_result" {
				historical = client.HistoricalCommandDetail(page, ref, name, summary)
			}
			if !summary.HasDetails && historical == nil {
				delete(sources, key)
				continue
			}
			sources[key] = source{ref, summary, historical}
		}
	}
	for ti := range cv.Turns {
		turn := &cv.Turns[ti]
		for bi := range turn.Reply.Blocks {
			for ci := range turn.Reply.Blocks[bi].Calls {
				call := &turn.Reply.Blocks[bi].Calls[ci]
				call.Load, call.TrackLoad, call.BackgroundDetail = nil, false, nil
				source, ok := sources[call.ID]
				if !ok {
					continue
				}
				var loaded bool
				if source.summary.HasDetails {
					var value protocol.ToolDetail
					if value, loaded = r.controller.ConversationDetail(source.ref); loaded {
						r.bindPluginCards(cv, call.ID, value)
						ProjectToolDetail(call, source.summary, value, locales...)
					}
				}
				if ref := source.historical; ref != nil && state.Connected && !call.Pending && call.State != conversation.Denied && call.State != conversation.StateInterrupted {
					binding := *ref
					call.BackgroundDetail = func() conversation.BackgroundDetail {
						return r.historicalCommandDetail(cv, binding)
					}
				}
				if !source.summary.HasDetails || loaded || !state.Connected || call.BackgroundDetail != nil {
					continue
				}
				if call.Output == nil {
					call.Output = &conversation.Output{}
				}
				ref := source.ref
				call.TrackLoad = true
				call.Load = func() { r.openToolDetail(cv, ref) }
			}
		}
	}
}

// The Client caches authorized detail data by canonical reference. Desktop
// separately owns each presentation's pending read and its completion.
type toolDetailViewKey struct {
	owner *viewmodel.Conversation
	ref   protocol.ToolDetailReference
	call  string
}

type toolDetailViewRead struct {
	cancel context.CancelFunc
	seen   bool
}

func (r *Runtime) openToolDetail(owner *viewmodel.Conversation, ref protocol.ToolDetailReference) {
	key := toolDetailViewKey{owner: owner, ref: ref}
	if ref.Call != nil {
		key.call, key.ref.Call = *ref.Call, nil
	}
	if pending := r.toolDetailReads[key]; pending != nil {
		pending.seen = true
		return
	}
	ctx, cancel := context.WithCancel(r.ctx)
	read := &toolDetailViewRead{cancel: cancel, seen: true}
	if r.toolDetailReads == nil {
		r.toolDetailReads = make(map[toolDetailViewKey]*toolDetailViewRead)
	}
	r.toolDetailReads[key] = read
	if !r.async(func() {
		_, err := r.controller.ReadConversationDetail(ctx, ref)
		r.enqueue(func(c *ui.Context) {
			if ctx.Err() == nil && r.toolDetailReads[key] == read && err != nil {
				r.report(c, err, "session_changed")
			}
		})
	}) {
		cancel()
		delete(r.toolDetailReads, key)
	}
}

func (r *Runtime) pruneToolDetails() {
	for key, read := range r.toolDetailReads {
		if !read.seen || !r.conversationSeen[key.ref.Session] {
			read.cancel()
			delete(r.toolDetailReads, key)
		}
		read.seen = false
	}
}

// ProjectToolDetail applies a loaded tool detail to its projected call without
// I/O. Live views and offline acceptance replay share the same row, content and
// failure projection; loading callbacks and plugin cards remain runtime-owned.
func ProjectToolDetail(call *conversation.Call, summary *protocol.ToolDetailSummary, loaded protocol.ToolDetail, locales ...string) {
	language := ""
	if len(locales) > 0 {
		language = locales[0]
	}
	value := protocol.ToolDetailSummary{}
	if summary != nil {
		value = *summary
	}
	value.Resolved = loaded.Resolved
	projected := projectCall(call.Name, &value, language)
	projected.ID = call.ID
	projected.Pending = call.Pending
	projected.Running = call.Running
	projected.Standalone, projected.Terminal = call.Standalone, call.Terminal
	projected.Background = call.Background
	if !projected.Failed {
		projected.State = call.State
	}
	if call.State == conversation.Denied || call.State == conversation.StateInterrupted {
		projected.State = call.State
		projected.Failed = false
		projected.Warning = true
		projected.Running = false
	}
	if call.Pending {
		projected.State = conversation.Pending
		projected.Failed = false
		projected.Running = false
	}
	projectToolContent(&projected, loaded, language)
	if call.Output != nil && len(call.Output.Images) > 0 {
		if projected.Output == nil {
			projected.Output = &conversation.Output{}
		}
		projected.Output.Images = call.Output.Images
	}
	*call = projected
}
