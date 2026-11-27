package telemetry

import (
	"context"
	"net/http"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/google/uuid"
)

// Hold the first send until the test releases it or its own consent expires.
type heldSend struct {
	endpoints
	started chan context.Context
	release chan struct{}
}

func (s *heldSend) RoundTrip(req *http.Request) (*http.Response, error) {
	select {
	case s.started <- req.Context():
	case <-s.release:
	}
	select {
	case <-s.release:
		return s.endpoints.RoundTrip(req)
	case <-req.Context().Done():
		return nil, req.Context().Err()
	}
}

func TestDiagnosticConsentScopes(t *testing.T) {
	for _, kind := range []string{"automatic", "metrics", "manual"} {
		t.Run(kind, func(t *testing.T) {
			s := &heldSend{started: make(chan context.Context), release: make(chan struct{})}
			c := New(Options{Transport: s, Timer: s.timer, Lookup: func(k string) string {
				switch k {
				case "SAILRY_SENTRY_DSN":
					return "http://public@reports.invalid/42"
				case "SAILRY_TELEMETRY_KEY":
					return "key"
				}
				return ""
			}})
			defer c.Close()
			defer close(s.release)
			settings := protocol.TelemetrySettings{Errors: true, Metrics: true, InstallationID: uuid.New()}
			c.Configure(settings)
			switch kind {
			case "automatic":
				c.report("crash", reduce([]byte(crashTrace)))
			case "metrics":
				go s.timers[0]()
			case "manual":
				go func() { _ = c.Diagnostics(t.Context()) }()
			}
			var active context.Context
			select {
			case active = <-s.started:
			case <-time.After(5 * time.Second):
				t.Fatal("send did not start")
			}
			if kind != "automatic" {
				settings.Errors = false
			}
			if kind != "metrics" {
				settings.Metrics = false
			}
			c.Configure(settings)
			if active.Err() != nil {
				t.Fatal("unrelated switch canceled an admitted send")
			}
			switch kind {
			case "automatic":
				settings.Errors = false
			case "metrics":
				settings.Metrics = false
			case "manual":
				settings.InstallationID = uuid.New()
			}
			c.Configure(settings)
			select {
			case <-active.Done():
			case <-time.After(5 * time.Second):
				t.Fatal("send survived its consent")
			}
		})
	}
}
