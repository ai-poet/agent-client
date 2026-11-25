package node

import (
	"context"

	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/trace"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func (node *Node) readTrace(ctx context.Context, query protocol.TraceQuery) (protocol.Output, error) {
	page, err := store.ReadTrace(ctx, node.profile.Ownership.Path, query)
	if page.Status != nil {
		page.Status.Recording = node.profile.Store.Node.TraceEnabled()
	}
	return protocol.Output{Kind: "trace", Data: page}, err
}

func (node *Node) applyTraceCommand(ctx context.Context, job commandJob, err error) link.Response {
	if err != nil {
		return response(protocol.Output{}, err)
	}
	switch job.request.Command.Kind {
	case "save_trace_settings":
		value, err := node.profile.Store.Node.SaveTraceSettings(ctx, *job.request.Command.Data.(*protocol.TraceSettings))
		return response(protocol.Output{Kind: "trace_settings", Data: value}, err)
	case "clear_all_traces":
		return response(protocol.Output{Kind: "all_traces_cleared"}, trace.ClearAll(node.profile.Ownership.Path))
	}
	session := job.request.Command.Data.(*protocol.CommandReadSessionPayload).Session
	err = trace.Clear(node.profile.Ownership.Path, session.String())
	return response(protocol.Output{Kind: "trace_cleared", Data: session}, err)
}
