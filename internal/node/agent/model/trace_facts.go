package model

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/url"
	"strings"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/trace"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// TraceProvider identifies the actual SDK route, not a URL with query or
// userinfo. Package model callbacks and external agent programs are out of scope.
func TraceProvider(language fantasy.LanguageModel, binding Binding) (trace.Provider, bool) {
	sdk, ok := language.(*sdkModel)
	if !ok {
		return trace.Provider{}, false
	}
	source := Scope(binding)
	p := trace.Provider{ID: source.ID.String(), API: string(source.API), Auth: string(source.Authentication), Model: source.Model}
	if u, err := url.Parse(sdk.config.Endpoint); err == nil {
		p.EndpointHost = u.Hostname()
	}
	credential := binding.Config.Credential
	if credential == nil {
		credential = binding.Provider.Credential
	}
	if credential != nil {
		p.Credential = credential.ID.String()
	}
	return p, true
}

func (a Attempt) observedTiming() canonical.Timing {
	elapsed := uint64(max(0, a.Finished.Sub(a.Started).Microseconds()))
	t := canonical.Timing{ElapsedUs: &elapsed}
	if a.FirstToken != nil {
		first := uint64(max(0, a.FirstToken.Sub(a.Started).Microseconds()))
		t.FirstTokenUs = &first
	}
	return t
}

func (a Attempt) Outcome() protocol.ConversationObservationOutcome {
	switch {
	case a.Failure == nil:
		return protocol.ConversationObservationOutcomeSucceeded
	case a.Failure.Kind == Cancelled:
		return protocol.ConversationObservationOutcomeCancelled
	default:
		return protocol.ConversationObservationOutcomeFailed
	}
}

func gatewayRequestID(h http.Header) string {
	for _, key := range []string{"X-Request-Id", "Request-Id", "X-Oneapi-Request-Id", "Cf-Ray"} {
		if id := h.Get(key); len(id) > 0 && len(id) <= 256 {
			return id
		}
	}
	return ""
}

func (a Attempt) endKind(err error) protocol.ConversationObservationEndKind {
	f := a.Failure
	switch {
	case f == nil:
		return protocol.ConversationObservationEndKindCompleted
	case f.Kind == Cancelled:
		return protocol.ConversationObservationEndKindClientAbort
	case a.Status != nil && *a.Status >= http.StatusMultipleChoices:
		return protocol.ConversationObservationEndKindHTTPError
	case f.truncated:
		return protocol.ConversationObservationEndKindStreamTruncated
	case f.Kind == Network:
		if timeout, ok := errors.AsType[net.Error](err); ok && timeout.Timeout() || errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
			return protocol.ConversationObservationEndKindUpstreamTimeout
		}
		if a.Status == nil {
			return protocol.ConversationObservationEndKindUpstreamUnreachable
		}
		return protocol.ConversationObservationEndKindUpstreamReset
	default:
		return protocol.ConversationObservationEndKindStreamError
	}
}

// The ledger and private trace both project this completed attempt, including
// its retry decision. Do not parse usage or time the stream again in the sink.
func (a *Attempt) finishTransport(err error, secrets []protocol.Secret, c *attemptCapture) {
	end := a.endKind(err)
	a.EndKind = &end
	id := c.gatewayID
	if id != "" {
		// Only retain an opaque identifier, never a credential echoed by a gateway.
		if strings.ContainsAny(id, " \r\n\t") || strings.Contains(id, "sk-") || strings.Contains(id, "ghp_") || strings.Contains(id, "AKIA") || safeDiagnostic(id, secrets) != id {
			id = "[redacted]"
		}
		a.GatewayRequestID = &id
	}
	if c.trace == nil {
		return
	}
	timing := struct {
		canonical.Timing
		Dispatch *uint64 `json:"dispatch_us,omitempty"`
		Sent     *uint64 `json:"sent_us,omitempty"`
		Headers  *uint64 `json:"headers_us,omitempty"`
	}{a.Timing, a.DispatchUs, a.RequestSentUs, a.ResponseUs}
	usage := a.Usage
	usage.Provider = nil // provider extensions belong in the redacted body
	f := trace.Facts{StartedMs: a.Started.UnixMilli(), Outcome: string(a.Outcome()), EndKind: string(end), GatewayRequestID: id, Usage: usage, Timing: timing}
	if len(a.Categories) != 0 {
		f.Categories = a.Categories
		f.CompleteContext = a.CompleteContext
	}
	if a.Status != nil {
		f.Status = int(*a.Status)
	}
	if a.Failure != nil {
		// Provider prose can contain arbitrary request data; the redacted
		// response sidecar, not the index row, owns that diagnostic.
		f.Failure = map[string]string{"kind": string(a.Failure.Kind)}
	}
	if a.RetryDelay != nil {
		f.Retry = map[string]any{"index": a.Index + 1, "limit": a.RetryLimit, "delay_ms": a.RetryDelay.Milliseconds(), "reason": a.Failure.Kind}
	}
	c.trace.Finish(f)
}
