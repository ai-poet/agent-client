package model

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"testing"
	"testing/iotest"

	"charm.land/fantasy"
)

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
