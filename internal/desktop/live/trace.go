package live

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/panel"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// Use the existing conversation watch as the invalidation source. Draft token
// deltas don't publish attempts, so they must not trigger trace reads.
type traceStamp struct {
	inventory, revision, index uint64
	request                    string
}

func (r *Runtime) traceStamp(id protocol.SessionID) traceStamp {
	state, _ := r.conversationState(id)
	key := traceStamp{inventory: state.InventoryVersion, revision: state.Snapshot.Page.Revision}
	if stats := state.Snapshot.Statistics; stats != nil && len(stats.Observations.Recent) > 0 {
		last := stats.Observations.Recent[len(stats.Observations.Recent)-1]
		key.request = last.ID
		if n := len(last.Attempts); n > 0 {
			key.index = last.Attempts[n-1].Index
		}
	}
	return key
}

type traceBadge struct {
	owner                    *Runtime
	stamp                    traceStamp
	status                   *protocol.TraceStatus
	requested, loading, seen bool
}

func (r *Runtime) traceStatus(cv *viewmodel.Conversation) (bool, bool, int) {
	owner := r.conversationRuntime(cv, nil)
	id, err := protocol.ParseSessionID(cv.ID)
	if err != nil || owner == nil || !owner.controller.Connected() {
		return false, false, 0
	}
	if r.traceBadges == nil {
		r.traceBadges = make(map[protocol.SessionID]*traceBadge)
	}
	b := r.traceBadges[id]
	if b == nil || b.owner != owner {
		b = &traceBadge{owner: owner}
		r.traceBadges[id] = b
	}
	b.seen = true
	stamp := owner.traceStamp(id)
	if !b.loading && (!b.requested || b.stamp != stamp) {
		b.requested, b.loading, b.stamp = true, true, stamp
		owner.async(func() {
			page, err := owner.controller.ReadTrace(owner.ctx, protocol.TraceQuery{Session: id, StatusOnly: true})
			r.enqueue(func(*ui.Context) {
				if r.traceBadges[id] == b {
					b.loading = false
					b.status = nil
					if err == nil {
						b.status = page.Status
					}
				}
			})
		})
	}
	if b.status == nil {
		return false, false, 0
	}
	return true, b.status.Recording, b.status.Failed
}

type sessionTrace struct {
	id                       protocol.SessionID
	owner                    *Runtime
	ctx                      context.Context
	cancel                   context.CancelFunc
	bodyCancel               context.CancelFunc
	stamp                    traceStamp
	requested, loading, seen bool
	jump                     protocol.TurnID
	tail                     string
	paging                   bool
}

func (r *Runtime) clearTracePanel() {
	if r.tracePanel != nil {
		r.tracePanel.cancel()
		r.tracePanel = nil
		r.App.Panel.ResetTrace()
	}
}

func (r *Runtime) finishTraceFrame() {
	for id, b := range r.traceBadges {
		if !b.seen {
			delete(r.traceBadges, id)
		}
		b.seen = false
	}
	if v := r.tracePanel; v != nil {
		if !v.seen {
			r.clearTracePanel()
		} else {
			v.seen = false
		}
	}
}

func (r *Runtime) mountTrace(idText string) *sessionTrace {
	id, err := protocol.ParseSessionID(idText)
	cv := r.App.Data.Find(idText)
	if err != nil || cv == nil {
		r.clearTracePanel()
		return nil
	}
	owner := r.conversationRuntime(cv, nil)
	if owner == nil || !owner.controller.Connected() {
		r.clearTracePanel()
		return nil
	}
	if v := r.tracePanel; v != nil && v.id == id && v.owner == owner {
		return v
	}
	r.clearTracePanel()
	ctx, cancel := context.WithCancel(owner.ctx)
	v := &sessionTrace{id: id, owner: owner, ctx: ctx, cancel: cancel}
	r.tracePanel = v
	p := r.App.Panel
	p.ResetTrace()
	p.TraceReadBody = func(name string) {
		if v.bodyCancel != nil {
			v.bodyCancel()
		}
		ctx, cancel := context.WithCancel(v.ctx)
		v.bodyCancel = cancel
		p.TraceContent, p.TraceError = protocol.TracePage{}, ""
		owner.async(func() {
			body, err := owner.controller.ReadTrace(ctx, protocol.TraceQuery{Session: id, Body: name, PreviewBytes: protocol.TracePreviewLimit})
			r.enqueue(func(c *ui.Context) {
				if ctx.Err() != nil {
					return
				}
				if err != nil {
					p.TraceError = i18n.T(c, "chat_history_failed")
				} else {
					p.TraceContent = body
				}
			})
		})
	}
	p.TraceJump = func(_, turn string) { v.jump, _ = protocol.ParseTurnID(turn); r.wake() }
	if r.exportTrace != nil {
		p.TraceExport = func(scope, mode, turn string) {
			q := protocol.TraceExportQuery{Session: id, Scope: scope, Mode: mode}
			if scope == "turn" {
				parsed, err := protocol.ParseTurnID(turn)
				if err != nil {
					r.ReportExport(err)
					return
				}
				q.Turn = parsed
			}
			r.exportTrace(q)
		}
	}
	return v
}

func (r *Runtime) traceFrame(c *ui.Context, session string) {
	v := r.mountTrace(session)
	if v == nil {
		r.App.Panel.TraceError = i18n.T(c, "unavailable")
		return
	}
	v.seen = true
	owner, p := v.owner, r.App.Panel
	key := owner.traceStamp(v.id)
	state, _ := owner.conversationState(v.id)
	if stats := state.Snapshot.Statistics; stats != nil {
		p.TraceRecent = stats.Observations.Recent
	} else {
		p.TraceRecent = nil
	}
	if !v.loading && (!v.requested || key != v.stamp) {
		page := state.Snapshot.Page
		p.TraceMarkers = nil
		for _, entry := range page.Entries {
			if entry.TimestampMs <= 0 {
				continue
			}
			tool := false
			for _, part := range entry.Parts {
				if part.Kind == "tool_call" || part.Kind == "tool_result" {
					tool = true
					break
				}
			}
			if entry.Author == "user" || tool {
				p.TraceMarkers = append(p.TraceMarkers, panel.TraceMarker{Turn: entry.Turn.String(), TimeMs: entry.TimestampMs, Tool: tool})
			}
		}
		// An index inside a partial transcript is not an absolute turn number.
		p.TraceTurnNumbers = nil
		if page.NextBefore == nil {
			p.TraceTurnNumbers = map[string]int{}
			for i, turn := range r.App.Data.Find(session).Turns {
				p.TraceTurnNumbers[turn.ID] = i + 1
			}
		}
		v.requested, v.loading, v.stamp = true, true, key
		after := v.tail
		owner.async(func() {
			rows, tail, err := v.readRows(after)
			reset := after == ""
			if after != "" && (faultCode(err) == protocol.ErrorCodeRevisionConflict || faultCode(err) == protocol.ErrorCodeNotFound) {
				reset = true
				rows, tail, err = v.readRows("")
			}
			r.enqueue(func(c *ui.Context) {
				if v.ctx.Err() != nil {
					return
				}
				v.loading = false
				if err != nil {
					v.tail = ""
					p.TracePage, p.TraceContent = nil, protocol.TracePage{}
					p.TraceError = i18n.T(c, "chat_history_failed")
					return
				}
				if reset {
					p.TracePage = rows
					if after != "" {
						p.TraceContent = protocol.TracePage{}
					}
				} else {
					p.TracePage = append(p.TracePage, rows...)
				}
				v.tail, p.TraceError = tail, ""
			})
		})
	}
	if v.jump == (protocol.TurnID{}) {
		return
	}
	target := v.jump
	if joined, ok := turnOwners(state.Snapshot.Page.Runs)[target]; ok {
		target = joined
	}
	if r.App.Messages.JumpToTurn(session, r.App.Data.Find(session).Turns, target.String()) {
		v.jump = protocol.TurnID{}
		return
	}
	if state.LoadingOlder || v.paging {
		return
	}
	if state.Snapshot.Page.NextBefore == nil && len(state.Snapshot.Missing) == 0 {
		v.jump = protocol.TurnID{}
		kit.Toast(c, kit.Failed, i18n.T(c, "unavailable"))
		return
	}
	v.paging = true
	owner.async(func() {
		err := owner.controller.LoadOlderConversation(v.ctx, v.id)
		r.enqueue(func(c *ui.Context) {
			if v.ctx.Err() != nil {
				return
			}
			v.paging = false
			if err != nil {
				v.jump = protocol.TurnID{}
				kit.Toast(c, kit.Failed, i18n.T(c, "chat_history_failed"))
			}
		})
	})
}

func (v *sessionTrace) readRows(after string) ([]json.RawMessage, string, error) {
	q := protocol.TraceQuery{Session: v.id, Limit: 100, After: after}
	var rows []json.RawMessage
	tail := after
	for {
		page, err := v.owner.controller.ReadTrace(v.ctx, q)
		if err != nil {
			return nil, "", err
		}
		rows = append(rows, page.Records...)
		if len(page.Records) > 0 {
			tail = page.Tail
		}
		if page.Next == "" {
			return rows, tail, nil
		}
		if page.Next == q.After || len(rows) >= 200 || len(page.Records) == 0 {
			return nil, "", errors.New("invalid trace page cursor")
		}
		q.After = page.Next
	}
}

func (x *conversationActions) ViewTrace(request string, attempt int) {
	if !x.owned() {
		x.unavailable()
		return
	}
	r := x.r.localRuntime()
	r.App.OpenTrace(x.cv.ID)
	if v := r.mountTrace(x.cv.ID); v != nil {
		v.seen = true
		r.App.Panel.SelectTrace(request, attempt)
	}
}

func (r *Runtime) SetTraceExport(f func(protocol.TraceExportQuery)) { r.exportTrace = f }
