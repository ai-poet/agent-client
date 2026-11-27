package model

import (
	"context"
	"errors"
	"fmt"
	"io"
	"time"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/clock"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model/anthropic"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model/google"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/trace"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

type Purpose string

const (
	Main           Purpose = "main"
	SummaryPurpose Purpose = "summary"
	NativePurpose  Purpose = "native_compaction"
)

type Attempt struct {
	MaxGapUs, GapStartUs *uint64
	Verdict              *protocol.TraceVerdict
	RequestID            string
	RequestStarted       time.Time
	Index                uint64
	Started              time.Time
	Finished             time.Time
	Timing               canonical.Timing
	FirstToken           *time.Time
	Usage                canonical.Usage
	// Context is the window occupied by the last prompt of a multi-prompt reply.
	Context *uint64
	Failure *Failure
	// RetryLimit is the last resend allowed if the current failure streak continues.
	RetryLimit uint64
	// RetryDelay is the wait before the next attempt; nil on the last attempt.
	RetryDelay        *time.Duration
	Status            *uint16
	EndKind           *protocol.ConversationObservationEndKind
	GatewayRequestID  *string
	ResponseUs        *uint64
	RequestBytes      *uint64
	DispatchUs        *uint64
	RequestSentUs     *uint64
	Categories        []protocol.ConversationObservationCategory
	CompleteContext   bool
	ReasoningStreamed bool
}

func PartUsage(part fantasy.StreamPart) canonical.Usage {
	var result canonical.Usage
	metadata, ok := part.ProviderMetadata[UsageMetadataKey].(*UsageMetadata)
	if !ok || metadata == nil || !metadata.Reported {
		return result
	}
	count := func(value *int64) *uint64 {
		if value == nil || *value < 0 {
			return nil
		}
		v := uint64(*value)
		return &v
	}
	result.Input = count(metadata.Input)
	result.Output = count(metadata.Output)
	result.Total = count(metadata.Total)
	result.CacheRead = count(metadata.CacheRead)
	result.CacheWrite = count(metadata.CacheWrite)
	result.Reasoning = count(metadata.Reasoning)
	result.Provider = append([]byte(nil), metadata.Raw...)
	return result
}
func mergeUsage(previous, current canonical.Usage) canonical.Usage {
	for _, pair := range []struct {
		from *uint64
		to   **uint64
	}{{current.Input, &previous.Input}, {current.Output, &previous.Output}, {current.Total, &previous.Total}, {current.CacheRead, &previous.CacheRead}, {current.CacheWrite, &previous.CacheWrite}, {current.Reasoning, &previous.Reasoning}} {
		if pair.from != nil {
			*pair.to = pair.from
		}
	}
	if len(current.Provider) > 0 {
		previous.Provider = current.Provider
	}
	return previous
}

// MainRetries bounds resends of one main or summary request. Both also share
// the request's cumulative wait budget, regardless of the transient cause.
const MainRetries = 10

// Run uses reset to discard provisional output before replaying an interrupted attempt.
// Without it, main output that escaped to the consumer cannot be retried safely.
func Run(ctx context.Context, languageModel fantasy.LanguageModel, call fantasy.Call, purpose Purpose, timer clock.Clock, observe func(Attempt) error, yield func(fantasy.StreamPart) bool, reset func()) error {
	buffered := purpose == SummaryPurpose || purpose == NativePurpose
	schedule := newRetryPolicy()
	requestID, requestStarted := protocol.NewRequestID().String(), timer.Now()
	sdk, _ := languageModel.(*sdkModel)
	origin := sdk.origin()
	tryHTTP1 := sdk != nil && (sdk.preferHTTP1.Load() || defaultHTTP1Origins.preferred(origin, requestStarted))
	triedHTTP1 := false
	skipHTTP1 := false
	var secrets []protocol.Secret
	if sdk != nil {
		secrets = append(secrets, sdk.config.Protection...)
		if sdk.config.Key != "" {
			secrets = append(secrets, protocol.NewSecret(sdk.config.Key))
		}
	}
	var scope *trace.Scope
	var protected []string
	if sdk != nil {
		scope, _ = call.ProviderOptions[trace.ProviderKey].(*trace.Scope)
		if scope != nil {
			for _, secret := range secrets {
				protected = append(protected, secret.Expose())
			}
		}
	}
	var failureSequence []string
	for index := uint64(0); ; index++ {
		if err := ctx.Err(); err != nil {
			return Classify(err)
		}
		attempt := Attempt{RequestID: requestID, RequestStarted: requestStarted, Index: index, Started: timer.Now(), RetryLimit: MainRetries}
		owner, cancel := context.WithCancel(ctx)
		firstOutputTimer := schedule.firstOutputTimer(attempt.Started, cancel)
		owner, capture := captureAttempt(owner, call, timer, attempt.Started)
		if scope != nil {
			capture.trace = scope.Begin(requestID, int(index+1), protected...)
		}
		owner, largeHTTP1 := withLargeHTTP1State(owner, skipHTTP1)
		nextModel := languageModel
		var releaseHTTP1 func()
		if tryHTTP1 {
			triedHTTP1, tryHTTP1 = true, false
			if alternative, release := sdk.http1Fallback(); alternative != nil {
				nextModel, releaseHTTP1 = alternative, release
			}
		}
		stream, err := nextModel.Stream(owner, call)
		var pending []fantasy.StreamPart
		streamed, escaped, finished, released := false, false, false, false
		if err == nil && stream != nil {
			var lastOutput time.Time
			for part := range stream {
				attempt.Usage = mergeUsage(attempt.Usage, PartUsage(part))
				if metadata, ok := part.ProviderMetadata[UsageMetadataKey].(*UsageMetadata); ok && metadata != nil && metadata.Context != nil && *metadata.Context >= 0 {
					window := uint64(*metadata.Context)
					attempt.Context = &window
				}
				if part.Type == fantasy.StreamPartTypeReasoningDelta && part.Delta != "" {
					attempt.ReasoningStreamed = true
				}
				if part.Type == fantasy.StreamPartTypeReasoningEnd {
					if metadata, ok := part.ProviderMetadata[anthropic.Name].(*anthropic.ReasoningOptionMetadata); ok && metadata.RedactedData != "" {
						attempt.ReasoningStreamed = true
					}
				}
				if metadata, ok := part.ProviderMetadata[responseMetadataKey].(*responseMetadata); ok {
					attempt.Status = metadata.Status
				}
				hasOutput := outputPart(part)
				firstOutput := attempt.FirstToken == nil && hasOutput
				if firstOutputTimer != nil && (firstOutput || part.Type == fantasy.StreamPartTypeFinish) {
					if !firstOutputTimer.Stop() {
						break
					}
					firstOutputTimer = nil
				}
				if hasOutput {
					now := timer.Now()
					if firstOutput {
						attempt.FirstToken = &now
					} else if !lastOutput.IsZero() {
						gap := uint64(max(0, now.Sub(lastOutput).Microseconds()))
						if attempt.MaxGapUs == nil || gap > *attempt.MaxGapUs {
							start := uint64(max(0, lastOutput.Sub(attempt.Started).Microseconds()))
							attempt.MaxGapUs, attempt.GapStartUs = &gap, &start
						}
					}
					lastOutput = now
				}
				if part.Type == fantasy.StreamPartTypeError {
					err = part.Error
					if err == nil {
						err = &Failure{Kind: InvalidResponse, Message: "model stream failed without an error"}
					}
					break
				}
				streamed = true
				if part.Type == fantasy.StreamPartTypeFinish {
					finished = true
				}
				if buffered {
					pending = append(pending, part)
				} else {
					escaped = escaped || yield != nil && outputPart(part)
					if yield != nil && !yield(part) {
						released = true
						err = context.Canceled
						break
					}
				}
			}
		}
		waitExhausted := firstOutputTimer != nil && !firstOutputTimer.Stop() && !released
		if err == nil && !finished {
			err = io.ErrUnexpectedEOF
		}
		if ctx.Err() != nil {
			err, waitExhausted = ctx.Err(), false
		}
		cancel()
		attempt.Finished = timer.Now()
		if releaseHTTP1 != nil {
			releaseHTTP1()
			if err == nil || streamed && errors.Is(err, context.Canceled) {
				sdk.preferHTTP1.Store(true)
				defaultHTTP1Origins.remember(origin, attempt.Finished)
			}
		}
		attempt.Timing = attempt.observedTiming()
		capture.apply(&attempt)
		attemptSecrets := capture.redact(secrets)
		exhausted := false
		if err != nil {
			attempt.Failure = Classify(err, attemptSecrets...)
			if waitExhausted {
				if attempt.Failure.Kind == Cancelled || attempt.Failure.truncated {
					attempt.Failure = &Failure{Kind: Network, Message: "Model retry timed out waiting for output after HTTP 524"}
				}
				waitExhausted = attempt.Failure.Retryable()
			}
			usedHTTP1 := releaseHTTP1 != nil || largeHTTP1.used.Load()
			// Only a wire response on the selected transport can reject its HTTP version.
			http1Rejected := usedHTTP1 && http1ProtocolRejected(attempt.Status)
			if usedHTTP1 {
				if http1Rejected || attempt.Failure.Kind == Network || attempt.Failure.Kind == Unavailable && !attempt.Failure.overloaded {
					// Disable both the explicit probe and the automatic large-body route.
					tryHTTP1, skipHTTP1 = false, true
					if sdk != nil {
						sdk.preferHTTP1.Store(false)
						defaultHTTP1Origins.forget(origin)
					}
				} else if releaseHTTP1 != nil {
					// A provider rejection reached its destination on HTTP/1.1.
					tryHTTP1 = true
				}
			}
			if attempt.Failure.status != nil {
				attempt.Status = attempt.Failure.status
			}
			committed := finished && !buffered && yield != nil
			canReplay := !released && !committed && (!escaped || reset != nil)
			sharedRetry := attempt.Failure.Retryable()
			// A protocol rejection can switch routes once even if an earlier HTTP
			// error used the ordinary resend. It still shares the attempt and wait budgets.
			protocolRetry := http1Rejected && attempt.Failure.Kind != Cancelled && index < MainRetries
			if !waitExhausted && canReplay && ctx.Err() == nil && (sharedRetry || schedule.takeHTTPRetry(attempt) || protocolRetry) {
				attempt.RetryLimit = index + 1
				if sharedRetry {
					attempt.RetryLimit = schedule.limit(purpose, attempt, streamed)
				}
				if index >= attempt.RetryLimit {
					exhausted = true
				} else if delay, allowed := schedule.next(ctx, attempt, timer.Now()); allowed {
					attempt.RetryDelay = &delay
				} else {
					waitExhausted = true
				}
			}
		}
		if attempt.Failure != nil {
			entry := string(attempt.Failure.Kind)
			if attempt.Status != nil {
				entry = fmt.Sprintf("%s/%d", entry, *attempt.Status)
			}
			failureSequence = append(failureSequence, entry)
		}
		attempt.finishTransport(err, attemptSecrets, capture, failureSequence)
		if observe != nil {
			if observationErr := observe(attempt); observationErr != nil {
				return observationErr
			}
		}
		if attempt.RetryDelay != nil && reset != nil {
			// The observer may have durably replaced the old draft. Release
			// its local output even if cancellation arrived during that commit.
			reset()
		}
		if contextErr := ctx.Err(); contextErr != nil {
			return Classify(contextErr)
		}
		if err == nil {
			if buffered && yield != nil {
				for _, part := range pending {
					if !yield(part) {
						return Classify(context.Canceled)
					}
				}
			}
			return nil
		}
		if attempt.RetryDelay != nil {
			ready, waitErr := schedule.wait(ctx, timer, attempt)
			if waitErr != nil {
				return Classify(waitErr)
			}
			waitExhausted = !ready
		}
		if exhausted || waitExhausted {
			// Observations retain the provider failure. Only the terminal
			// result names the exhausted recovery for the turn's final fault.
			failure := *attempt.Failure
			if attempt.Failure.Retryable() && (exhausted || waitExhausted) {
				failure.Exhausted, failure.WaitExhausted = exhausted, waitExhausted
			}
			return &failure
		}
		if attempt.RetryDelay == nil {
			return attempt.Failure
		}
		if sdk != nil && !triedHTTP1 && !skipHTTP1 && attempt.Failure.Kind == Network && http2ProtocolFailure(err) {
			tryHTTP1 = true
		}
	}
}
func outputPart(part fantasy.StreamPart) bool {
	if PluginOutput(part) {
		return true
	}
	switch part.Type {
	case fantasy.StreamPartTypeTextDelta:
		return part.Delta != ""
	case fantasy.StreamPartTypeToolInputStart, fantasy.StreamPartTypeToolInputDelta, fantasy.StreamPartTypeToolCall:
		return true
	case fantasy.StreamPartTypeReasoningDelta:
		if part.Delta != "" {
			return true
		}
		fallthrough
	case fantasy.StreamPartTypeReasoningEnd:
		for _, value := range part.ProviderMetadata {
			switch metadata := value.(type) {
			case *anthropic.ReasoningOptionMetadata:
				if metadata.Signature != "" || metadata.RedactedData != "" {
					return true
				}
			case *google.ReasoningMetadata:
				if metadata.Signature != "" {
					return true
				}
			case *openAIResponsesReasoningMetadata:
				if metadata.EncryptedContent != nil || len(metadata.Summary) > 0 {
					return true
				}
			}
		}
		return false
	}
	return false
}
