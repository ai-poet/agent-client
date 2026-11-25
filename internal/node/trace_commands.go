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
	return protocol.Output{Kind: "trace", Data: page}, err
}

func (node *Node) clearTrace(job commandJob, err error) link.Response {
	session := job.request.Command.Data.(*protocol.CommandReadSessionPayload).Session
	if err == nil {
		err = trace.Clear(node.profile.Ownership.Path, session.String())
	}
	return response(protocol.Output{Kind: "trace_cleared", Data: session}, err)
}
