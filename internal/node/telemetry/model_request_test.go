package telemetry

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"maps"
	"net/http"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/getsentry/sentry-go"
	"github.com/google/uuid"
)

func modelRequestFields(outcome, reason string, attempts, status uint64) logging.Fields {
	fields := logging.Fields{
		"area": "agent", "operation": "model_request", "phase": logging.Failed, "code": "unavailable",
		"retry_outcome": outcome, "reason_code": reason, "attempts": attempts,
	}
	if outcome == "recovered" {
		fields["phase"] = logging.Completed
	}
	if status != 0 {
		fields["http_status"] = status
	}
	return fields
}

func checkModelRequestFields(t *testing.T, got, want logging.Fields) {
	t.Helper()
	if len(got) != len(want) {
		t.Errorf("model request fields = %v, want only %v", got, want)
	}
	for key, value := range want {
		if n, ok := value.(uint64); ok {
			value = float64(n) // JSON numbers in an untyped Sentry context.
		}
		if got[key] != value {
			t.Errorf("model request %s = %v, want %v", key, got[key], value)
		}
	}
}

func checkModelRequestReport(t *testing.T, body []byte, level sentry.Level, fields logging.Fields) sentry.Event {
	t.Helper()
	assertClean(t, "model request report", body)
	event := sentry.Event{}
	lines := bytes.SplitN(body, []byte("\n"), 4)
	if len(lines) < 3 || json.Unmarshal(lines[2], &event) != nil {
		t.Fatal("invalid Sentry envelope")
	}
	if event.Message != "agent.model_request:unavailable" || event.Level != level {
		t.Errorf("model request report message/level = %q/%s, want agent.model_request:unavailable/%s", event.Message, event.Level, level)
	}
	if event.Tags["sailry.reason_code"] != fields["reason_code"] {
		t.Errorf("model request reason tag = %q, want %q", event.Tags["sailry.reason_code"], fields["reason_code"])
	}
	checkModelRequestFields(t, logging.Fields(event.Contexts["sailry"]), fields)
	if event.EventID == "" || len(event.Fingerprint) != 1 || event.Fingerprint[0] == "" || len(event.Exception) != 1 {
		t.Fatalf("missing reporting identity or exception: id=%q fingerprint=%v exceptions=%d", event.EventID, event.Fingerprint, len(event.Exception))
	}
	if m := event.Exception[0].Mechanism; m == nil || m.Handled == nil || !*m.Handled {
		t.Error("model request exception is not marked handled")
	}
	return event
}

func TestModelRequestReportConsent(t *testing.T) {
	for _, test := range []struct {
		name                   string
		errors, metrics        bool
		envKey, envValue       string
		wantUsage, wantReports int
	}{
		{name: "error consent only", errors: true, wantReports: 2},
		{name: "usage consent only", metrics: true, wantUsage: 2},
		{name: "both on", errors: true, metrics: true, wantUsage: 2, wantReports: 2},
		{name: "both off"},
		{name: "do not track", errors: true, metrics: true, envKey: "DO_NOT_TRACK", envValue: "1"},
		{name: "telemetry off", errors: true, metrics: true, envKey: "SAILRY_TELEMETRY", envValue: "off"},
		{name: "no Sentry endpoint", errors: true, metrics: true, envKey: "SAILRY_SENTRY_DSN", envValue: " ", wantUsage: 2},
	} {
		t.Run(test.name, func(t *testing.T) {
			env := map[string]string{"SAILRY_SENTRY_DSN": "http://public@reports.invalid/42", "SAILRY_TELEMETRY_KEY": "test-key"}
			if test.envKey != "" {
				env[test.envKey] = test.envValue
			}
			remote := &endpoints{}
			c := New(Options{Transport: remote, Timer: remote.timer, Lookup: func(key string) string { return env[key] }})
			t.Cleanup(c.Close)
			c.Configure(protocol.TelemetrySettings{InstallationID: uuid.New(), Errors: test.errors, Metrics: test.metrics})
			c.Observe(ModelRequest(Model{API: protocol.ConversationModelApiResponses, Authentication: protocol.AuthenticationApiKey, Model: "custom"}, "error", nil, nil, nil, Tokens{}))
			c.Record(slog.LevelWarn, modelRequestFields("recovered", "model_network", 2, 0))
			c.Record(slog.LevelError, modelRequestFields("failed", "model_network", 3, 0))
			c.reports.Wait()
			c.Close()
			usage, reports := remote.sent()
			if len(usage) != test.wantUsage || len(reports) != test.wantReports {
				t.Errorf("sent %d usage batches and %d reports, want %d and %d", len(usage), len(reports), test.wantUsage, test.wantReports)
			}
		})
	}
}

func TestModelRequestReportMalformedFields(t *testing.T) {
	c, endpoint := control(true, false)
	defer c.Close()
	for _, overrides := range []logging.Fields{
		{"retry_outcome": nil, "reason_code": nil, "attempts": nil},
		{"reason_code": "planted_private_reason"},
		{"attempts": uint64(1)},
		{"attempts": uint64(1_000_001)},
		{"phase": logging.Failed},
		{"operation": "unrelated_operation"},
	} {
		fields := modelRequestFields("recovered", "model_network", 2, http.StatusBadGateway)
		maps.Copy(fields, overrides)
		c.Record(slog.LevelWarn, fields)
	}
	want := modelRequestFields("failed", "model_network", 3, 0)
	fields := maps.Clone(want)
	fields["http_status"] = uint64(600)
	c.Record(slog.LevelError, fields)
	c.reports.Wait()
	_, reports := endpoint.sent()
	if len(reports) != 1 {
		t.Fatalf("unclassified recovery warnings must not report; got %d reports, want only the failure", len(reports))
	}
	event := checkModelRequestReport(t, reports[0], sentry.LevelError, want)
	for _, breadcrumb := range event.Breadcrumbs {
		for _, key := range []string{"retry_outcome", "reason_code", "attempts", "http_status"} {
			if _, exists := breadcrumb.Data[key]; exists {
				t.Errorf("unclassified recovery breadcrumb carried %q", key)
			}
		}
	}
}
