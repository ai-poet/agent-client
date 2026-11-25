package client

import (
	"context"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func (c *Client) ReadTrace(ctx context.Context, query protocol.TraceQuery) (protocol.TracePage, error) {
	out, err := c.read(ctx, protocol.Command{Kind: "read_trace", Data: query})
	if err != nil {
		return protocol.TracePage{}, err
	}
	page, err := payload[protocol.TracePage](out.Data)
	if err != nil || out.Kind != "trace" || page.Session != query.Session || page.Body != query.Body {
		return protocol.TracePage{}, fault(protocol.ErrorCodeInvalidRequest, "trace response does not match query")
	}
	return page, nil
}

func (c *Client) ClearTrace(ctx context.Context, session protocol.SessionID) error {
	out, err := c.Settle(ctx, c.Prepare(protocol.Command{Kind: "clear_trace", Data: protocol.CommandReadSessionPayload{Session: session}}))
	if err != nil {
		return err
	}
	id, err := payload[protocol.SessionID](out.Data)
	if err != nil || out.Kind != "trace_cleared" || id != session {
		return fault(protocol.ErrorCodeInvalidRequest, "trace clear response does not match session")
	}
	return nil
}
