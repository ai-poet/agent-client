package live

import (
	"context"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/shell"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func (r *Runtime) openInsight(c *ui.Context, p *insightsPage, row protocol.UsageRequest) {
	id := row.Session.String()
	cv := r.App.Data.Find(id)
	if p.jumpCancel != nil {
		p.jumpCancel()
		p.jumpCancel = nil
	}
	owner := p.owner
	if owner == nil || row.Position.Node != owner.controller.Target() || !owner.controller.Connected() {
		kit.Toast(c, kit.Failed, i18n.T(c, "unavailable"))
		return
	}
	if cv == nil {
		// The usage index is not bounded by the current sidebar snapshot.
		// Resolve this one session on its reporting Node without unarchiving it.
		ctx, cancel := context.WithCancel(owner.ctx)
		p.jumpCancel = cancel
		owner.async(func() {
			out, err := owner.controller.Do(ctx, protocol.Command{Kind: "read_session", Data: protocol.CommandReadSessionPayload{Session: row.Session}})
			r.enqueue(func(c *ui.Context) {
				if ctx.Err() != nil || r.insights != p || r.App.Page != shell.InsightsPage || r.runtimeFor(p.node) != owner {
					return
				}
				session, ok := detail[protocol.Session](out.Data)
				if err != nil || out.Kind != "session" || !ok || session.ID != row.Session {
					kit.Toast(c, kit.Failed, i18n.T(c, "unavailable"))
					return
				}
				if owner.admittedSessions == nil {
					owner.admittedSessions = map[string]protocol.Session{}
				}
				owner.admittedSessions[id] = session
				owner.project()
				r.revealInsight(c, owner, row)
			})
		})
		return
	}
	r.revealInsight(c, owner, row)
}

func (r *Runtime) revealInsight(c *ui.Context, owner *Runtime, row protocol.UsageRequest) {
	id := row.Session.String()
	cv := r.App.Data.Find(id)
	if cv == nil || r.conversationRuntime(cv, nil) != owner || !owner.controller.Connected() {
		kit.Toast(c, kit.Failed, i18n.T(c, "unavailable"))
		return
	}
	r.App.OpenTrace(id)
	if trace := r.mountTrace(id); trace != nil {
		trace.seen = true
		trace.jump = row.Turn
		r.App.Panel.SelectIndexedTrace(row)
	}
}
