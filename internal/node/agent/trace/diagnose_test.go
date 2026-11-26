package trace

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

func tokens(n uint64) *uint64 { return &n }

func TestMetadataDiagnoses(t *testing.T) {
	for _, tt := range []struct {
		r    Round
		code string
	}{
		{Round{Status: 503}, "unavailable"}, {Round{Status: 401}, "authentication"},
		{Round{Status: 403, Failure: "authentication"}, "authentication"},
		{Round{Status: 403, Failure: "quota"}, "quota"}, {Round{Status: 402}, "quota"},
		{Round{Status: 429}, "rate_limited"}, {Round{Status: 400, Failure: "context_overflow"}, "context_overflow"},
		{Round{Status: 400}, "invalid_request"}, {Round{End: "stream-truncated"}, "invalid_response"},
		{Round{Failure: "network", DialTimeout: true}, "network"}, {Round{Outcome: "cancelled"}, "cancelled"},
		{Round{FirstUs: tokens(30_000_001)}, "slow_first"},
		{Round{FirstUs: tokens(500), MaxGapUs: tokens(31_000_000)}, "slow_output"},
		{Round{Outcome: "succeeded"}, "no_usage"}, {Round{Outcome: "succeeded", HasUsage: true}, "normal"},
	} {
		if got := Diagnose(tt.r, nil); got.Code != tt.code {
			t.Errorf("%+v: got %s, want %s", tt.r, got.Code, tt.code)
		}
	}
	v := Diagnose(Round{Status: 403, Failure: "authentication", Gateway: "gateway-id", RetrySequence: []string{"network", "authentication/403"}}, nil)
	for _, key := range []string{"diagnosis_action_account", "connections_copy_id", "diagnosis_action_fluctuation"} {
		if !slices.Contains(v.Actions, key) {
			t.Errorf("missing %s: %+v", key, v)
		}
	}
}

func TestPrefixThresholdsAndAttribution(t *testing.T) {
	makeRound := func(body string) Round {
		return Round{Provider: Provider{API: "chat_completions"}, StartedMs: 10000, Outcome: "succeeded", HasUsage: true, Input: tokens(5120), Read: tokens(0), Segments: segments("chat_completions", []byte(body))}
	}
	body := `{"messages":[{"role":"system","content":"Policy for 2025-07-12T12:30:00Z"},{"role":"assistant","tool_calls":[{"id":"c","function":{"name":"lookup"}}]},{"role":"tool","tool_call_id":"c","content":"first result"}],"tools":[{"function":{"name":"alpha"}}]}`
	p := makeRound(body)
	for _, tt := range []struct {
		read uint64
		code string
	}{{0, "cache_miss"}, {1023, "cache_miss"}, {1024, "cache_partial"}, {4095, "cache_partial"}, {4096, "normal"}} {
		r := p
		r.Read = tokens(tt.read)
		if v := Diagnose(r, &p); v.Code != tt.code {
			t.Errorf("read %d: %+v", tt.read, v)
		}
	}
	for _, tt := range []struct {
		from, to, cause string
		compact         bool
	}{
		{"alpha", "beta", "tool-defs:set", false},
		{"Policy", "Guides", "system", false},
		{"Policy", "Guides", "history:truncated", true},
		{"first result", "other result", "tool-result:lookup", false},
		{"12:30", "12:31", "system:volatile", false},
	} {
		r := makeRound(strings.Replace(body, tt.from, tt.to, 1))
		r.AfterCompaction = tt.compact
		// Enough reported input to put even the last divergent block above the cache floor.
		r.Input = tokens(20000)
		v := Diagnose(r, &p)
		if v.Code != "prefix_broken" || v.CauseKey != tt.cause || v.LostTokens == 0 {
			t.Errorf("%s: %+v", tt.cause, v)
		}
	}
	r := p
	r.Read = nil
	if v := Diagnose(r, &p); v.Code != "normal" {
		t.Fatalf("missing cache usage diagnosed as a miss: %+v", v)
	}
}

func TestCacheSegmentOrderAndBoundaries(t *testing.T) {
	s := segments("anthropic", []byte(`{"messages":[{"role":"user","content":"a"}],"system":[{"type":"text","text":"b","cache_control":{"type":"ephemeral"}}],"tools":[{"name":"one"}]}`))
	if len(s) != 3 || s[0].kind != "tool-def" || s[1].kind != "system" || s[2].role != "user" || !s[1].breakpoint || strings.Contains(s[1].text, "cache_control") {
		t.Fatalf("%+v", s)
	}
	a := segments("responses", []byte(`{"input":[{"type":"function_call","id":"item1","call_id":"call","name":"lookup","arguments":"{}"}]}`))
	b := segments("responses", []byte(`{"input":[{"type":"function_call","id":"item2","call_id":"call","name":"lookup","arguments":"{}"}]}`))
	if len(a) != 1 || len(b) != 1 || !a[0].same(b[0]) {
		t.Fatal("Responses item ID broke the cache prefix")
	}
}

func TestCapturedVerdictUsesRecordedPredecessor(t *testing.T) {
	scope := Scope{Profile: t.TempDir(), Session: "s", Purpose: "main", Provider: Provider{API: "responses", Model: "fixture"}}
	for i, id := range []string{"one", "unrelated", "three"} {
		a := scope.Begin(id, 1)
		body := `{"input":"same cached prefix"}`
		if i == 1 {
			body = `{"input":"different conversation, no common segment"}`
		}
		a.Request(http.Header{"Content-Type": {"application/json"}}, []byte(body))
		r := Round{StartedMs: int64(i + 1), Outcome: "succeeded", HasUsage: true, Input: tokens(4096), Read: tokens(0)}
		v := Diagnose(r, nil)
		a.Finish(Facts{Outcome: r.Outcome, StartedMs: r.StartedMs, Round: &r, Verdict: &v})
	}
	data, err := os.ReadFile(filepath.Join(scope.Profile, "traces", "s", "trace.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) != 3 {
		t.Fatalf("got %d records", len(lines))
	}
	var row struct {
		Verdict Verdict
		Lost    uint64 `json:"lost_tokens"`
	}
	if err := json.Unmarshal([]byte(lines[1]), &row); err != nil || row.Verdict.Code != "normal" || row.Lost != 0 {
		t.Fatalf("%s: %v", lines[1], err)
	}
	if err := json.Unmarshal([]byte(lines[2]), &row); err != nil || row.Verdict.Code != "cache_miss" || row.Lost != 4096 {
		t.Fatalf("%s: %v", lines[2], err)
	}
}

func TestPredecessorRanksPrefixThenOverlap(t *testing.T) {
	dig := func(parts string) string {
		var s []segment
		for _, p := range strings.Fields(parts) {
			s = append(s, segment{text: p})
		}
		return fingerprint(s)
	}
	choice := predecessorChoice{matcher: matchBranches(dig("a b c d"))}
	add := func(id, parts string, ts int64) {
		choice.consider(rowKey{StartedMs: ts, Request: headerSnapshot{Body: id}, Fingerprint: dig(parts)})
	}
	add("unrelated", "w x y z", 100)
	if got := choice.selected().Request.Body; got != "" {
		t.Fatal(got)
	}
	add("half", "x b c y", 7)
	add("greater", "x b c d", 5)
	if got := choice.selected().Request.Body; got != "greater" {
		t.Fatal(got)
	}
	add("prefix", "a z", 8)
	add("long", "a b z", 2)
	add("long-later", "a b q", 6)
	if got := choice.selected().Request.Body; got != "long-later" {
		t.Fatal(got)
	}
	choice = predecessorChoice{matcher: matchBranches(dig("a b c d"))}
	add("half", "x b c y", 7)
	if got := choice.selected().Request.Body; got != "half" {
		t.Fatal(got)
	}
}

func TestStructuralEvidence(t *testing.T) {
	for _, tt := range []struct{ api, body string }{
		{"chat_completions", `{"messages":[{"role":"assistant","tool_calls":[{"function":{"arguments":"{\"x\":1,\"y\":2}"}}]}]}`},
		{"responses", `{"input":[{"type":"function_call","arguments":"{\"x\":1,\"y\":2}"}]}`},
		{"chat_completions", `{"messages":[],"response_format":{"x":1,"y":2}}`},
		{"responses", `{"input":[],"text":{"x":1,"y":2}}`},
	} {
		swapped := strings.NewReplacer(`\"x\":1,\"y\":2`, `\"y\":2,\"x\":1`, `"x":1,"y":2`, `"y":2,"x":1`).Replace(tt.body)
		a, b := segments(tt.api, []byte(tt.body)), segments(tt.api, []byte(swapped))
		if len(a) != 1 || len(b) != 1 || a[0].same(b[0]) || !onlyKeyOrder(a[0], b[0]) {
			t.Fatalf("%s: %s", tt.api, swapped)
		}
	}
	gem := segments("gemini", []byte(`{"contents":[{"role":"model","parts":[{"functionCall":{"name":"read","args":{}}}]}],"generationConfig":{"temperature":0},"tools":[{"functionDeclarations":[{"name":"read"}]}],"systemInstruction":{"parts":[{"text":"policy"}]}}`))
	if len(gem) != 4 || gem[0].kind != "tool-call" || gem[1].kind != "options" || gem[2].kind != "tool-def" || gem[3].kind != "system" {
		t.Fatalf("%+v", gem)
	}
	r := Round{Provider: Provider{API: "gemini"}, Outcome: "succeeded", HasUsage: true, Input: tokens(4096), Read: tokens(0), Segments: gem}
	if got := Diagnose(r, &r); got.LostTokens != 0 || got.Code != "normal" {
		t.Fatalf("unsupported Gemini cache estimate: %+v", got)
	}
}
