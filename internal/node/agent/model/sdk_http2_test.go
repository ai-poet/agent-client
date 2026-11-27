package model

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"testing/iotest"
	"time"

	"charm.land/fantasy"
	"golang.org/x/net/http2"
)

func TestHTTP2TransportFailureClassification(t *testing.T) {
	t.Parallel()
	protocol := http2.StreamError{StreamID: 1, Code: http2.ErrCodeProtocol, Cause: errors.New("received from peer")}
	internal := http2.StreamError{StreamID: 559, Code: http2.ErrCodeInternal, Cause: errors.New("received from peer")}
	for _, cause := range []error{
		internal,
		protocol,
		http2.StreamError{StreamID: 3, Code: http2.ErrCodeRefusedStream},
		http2.StreamError{StreamID: 3, Code: http2.ErrCodeCancel, Cause: errors.New("received from peer")},
		http2.StreamError{StreamID: 3, Code: http2.ErrCodeHTTP11Required},
		http2.StreamError{StreamID: 3, Code: http2.ErrCode(0xab)},
		fmt.Errorf("wrapped: %w", protocol),
		errors.New("http2: client connection lost"),
		fmt.Errorf("read: %w", errors.New("http2: client connection lost")),
	} {
		t.Run(cause.Error(), func(t *testing.T) {
			for _, err := range []error{cause, &fantasy.ProviderError{Message: "model stream failed", StatusCode: 200, Cause: cause}} {
				if failure := Classify(err); failure.Kind != Network || !failure.Retryable() || !strings.Contains(failure.Message, cause.Error()) {
					t.Fatalf("HTTP/2 transport failure lost its retryable cause: %+v", failure)
				}
			}
		})
	}
	for _, test := range []struct {
		err      error
		fallback bool
	}{
		{protocol, true},
		{internal, false},
		{http2.StreamError{StreamID: 3, Code: http2.ErrCodeHTTP11Required}, true},
		{&fantasy.ProviderError{Cause: protocol}, true},
		{fmt.Errorf("wrapped: %w", protocol), true},
		{http2.ConnectionError(http2.ErrCodeProtocol), false},
		{errors.New(protocol.Error()), false},
		{http2.StreamError{StreamID: 1, Code: http2.ErrCodeRefusedStream}, false},
		{errors.New("http2: client connection lost"), false},
		{errors.New("PROTOCOL_ERROR"), false},
	} {
		if http2ProtocolFailure(test.err) != test.fallback {
			t.Errorf("protocol fallback for %v = %t", test.err, !test.fallback)
		}
	}
	provider := &fantasy.ProviderError{StatusCode: 200, ResponseBody: []byte(`{"error":{"type":"api_error","message":"stream error: stream ID 1; PROTOCOL_ERROR; received from peer"}}`)}
	for _, cause := range []error{internal, errors.New(protocol.Error()), protocol, errors.New("http2: client connection lost")} {
		provider.Cause = cause
		if failure := Classify(provider); failure.Kind != InvalidResponse || failure.Retryable() || http2ProtocolFailure(provider) {
			t.Fatalf("provider diagnostic was treated as a local HTTP/2 failure: %+v", failure)
		}
	}
}

func TestResponsesRetriesHTTP2InternalReset(t *testing.T) {
	t.Parallel()
	var calls atomic.Int32
	success := fixture(t, "responses-text.sse")
	consumed := make(chan struct{})
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.ProtoMajor != 2 {
			t.Error("a peer internal error should retry over HTTP/2")
		}
		w.Header().Set("Content-Type", "text/event-stream")
		if calls.Add(1) == 1 {
			_, _ = io.WriteString(w, "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"message\",\"delta\":\"Discarded\"}\n\n")
			w.(http.Flusher).Flush()
			select {
			case <-consumed:
			case <-r.Context().Done():
			}
			panic(http.ErrAbortHandler)
		}
		_, _ = w.Write(success)
	}))
	server.EnableHTTP2 = true
	server.StartTLS()
	defer server.Close()
	language, err := newSDK(t.Context(), transportConfig{Kind: "responses", Model: "fixture", Endpoint: server.URL, Key: "fixture-key"})
	if err != nil {
		t.Fatal(err)
	}
	language.(*sdkModel).client.Transport = responseTransport{base: server.Client().Transport, stall: stallTimeout}
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	var attempts []Attempt
	var text string
	var resets int
	err = Run(ctx, language, requestCall(t, "responses"), Main, &policyClock{}, func(a Attempt) error {
		attempts = append(attempts, a)
		return nil
	}, func(p fantasy.StreamPart) bool {
		if p.Type == fantasy.StreamPartTypeTextDelta {
			text += p.Delta
			if p.Delta == "Discarded" {
				close(consumed)
			}
		}
		return true
	}, func() { text = ""; resets++ })
	if err != nil || len(attempts) != 2 || calls.Load() != 2 || text != "Read complete" || resets != 1 {
		t.Fatalf("peer internal reset: err=%v attempts=%d requests=%d text=%q resets=%d", err, len(attempts), calls.Load(), text, resets)
	}
	first := attempts[0]
	if first.Failure == nil || first.Failure.Kind != Network || !strings.Contains(first.Failure.Message, "INTERNAL_ERROR; received from peer") || first.RetryDelay == nil || first.RequestID != attempts[1].RequestID || attempts[1].Failure != nil {
		t.Fatalf("unexpected recovery: %+v", attempts)
	}
}

func TestHTTP2ConnectionLossRetry(t *testing.T) {
	lost := fmt.Errorf("read: %w", errors.New("http2: client connection lost"))
	for _, finished := range []bool{false, true} {
		t.Run(fmt.Sprintf("finished_%t", finished), func(t *testing.T) {
			language, err := newSDK(t.Context(), transportConfig{Kind: "chat", Model: "fixture", Endpoint: "https://model.invalid", Key: "fixture-key"})
			if err != nil {
				t.Fatal(err)
			}
			requests := 0
			language.(*sdkModel).client.Transport = responseTransport{base: pluginRoundTripper(func(r *http.Request) (*http.Response, error) {
				requests++
				const chunk = `data: {"choices":[{"index":0,"delta":{"content":"Answer"},"finish_reason":"stop"}]}` + "\n\n"
				var body io.Reader = strings.NewReader(chunk)
				if requests == 1 {
					if !finished {
						body = strings.NewReader(strings.Replace(chunk, `"stop"`, "null", 1))
					}
					body = io.MultiReader(body, iotest.ErrReader(lost))
				}
				return &http.Response{StatusCode: http.StatusOK, Request: r, Header: http.Header{"Content-Type": {"text/event-stream"}}, Body: io.NopCloser(body)}, nil
			}), stall: stallTimeout}
			timer := &policyClock{}
			text, finishes, resets := "", 0, 0
			err = Run(t.Context(), language, requestCall(t, "chat"), Main, timer, nil, func(part fantasy.StreamPart) bool {
				if part.Type == fantasy.StreamPartTypeTextDelta {
					text += part.Delta
				}
				if part.Type == fantasy.StreamPartTypeFinish {
					finishes++
				}
				return true
			}, func() { text = ""; resets++ })
			retries := 1
			if finished {
				retries = 0
			}
			if err != nil || requests != retries+1 || len(timer.delays) != retries || resets != retries || finishes != 1 || text != "Answer" {
				t.Fatalf("err=%v requests=%d delays=%v resets=%d finishes=%d text=%q", err, requests, timer.delays, resets, finishes, text)
			}
		})
	}
	failure := Classify(&fantasy.ProviderError{StatusCode: 200, ResponseBody: []byte(`{"error":{"message":"http2: client connection lost"}}`), Cause: lost})
	if failure.Kind != InvalidResponse || failure.Retryable() {
		t.Fatalf("provider diagnostic classified as a transport failure: %+v", failure)
	}
}
