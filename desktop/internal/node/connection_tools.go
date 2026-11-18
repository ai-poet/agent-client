package node

import (
	"context"

	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

func connectionFault(code protocol.ErrorCode, message string) *protocol.Fault {
	return &protocol.Fault{Code: code, Message: message}
}

// connectionDispatch runs one captured request on this Node. A stopped turn
// cancels the running effect and reports stopped; a completion that never
// arrives reports lost.
func (node *Node) connectionDispatch(ctx context.Context, request protocol.Request, stopped, lost *protocol.Fault) (protocol.Output, error) {
	// A turn-scoped plugin request is admitted only from the execution Node.
	accepted, err := node.Dispatch(ctx, node.ID(), request)
	if err != nil {
		return protocol.Output{}, err
	}
	select {
	case response, delivered := <-accepted.Completion:
		if !delivered {
			return protocol.Output{}, lost
		}
		if response.Fault != nil {
			return protocol.Output{}, response.Fault
		}
		return response.Output, nil
	case <-ctx.Done():
		return protocol.Output{}, stopped
	}
}
