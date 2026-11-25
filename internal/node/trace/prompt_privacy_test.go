package trace

import (
	"strings"
	"testing"
)

func TestProtectedBlocksAndToolDescriptionEchoes(t *testing.T) {
	const body = `{"messages":[{"role":"user","content":"keep"}],"preamble":"system-sentinel","tools":[{"toolSpec":{"description":"tool-sentinel"}}],"system_instruction":{"unknown-sensitive-key":"another-sentinel"}}`
	p := promptSet{}
	b, ok, err := p.request([]byte(body))
	if err != nil || !ok {
		t.Fatal(ok, err)
	}
	s := string(safeEvidence(b, p.replacer()))
	for _, val := range []string{"system-sentinel", "tool-sentinel", "unknown-sensitive-key", "another-sentinel"} {
		if strings.Contains(s, val) {
			t.Fatal(s)
		}
	}
	if !strings.Contains(s, "keep") || !strings.Contains(s, "sha256:") {
		t.Fatal(s)
	}
	response := string(safeEvidence([]byte(`{"error":"provider echoed tool-sentinel and unknown-sensitive-key and system-sentinel"}`), p.replacer()))
	if strings.Contains(response, "unknown-sensitive-key") || strings.Contains(response, "sentinel") {
		t.Fatal(response)
	}
}

func TestShareableRescrubsLegacyBodyShapes(t *testing.T) {
	data := []byte(`{"content":"github_pat_private","private_key":"old-key"}`)
	got := string(safeEvidence(data, promptSet{}.replacer()))
	if strings.Contains(got, "old-key") || strings.Contains(got, "github_pat_private") || !strings.Contains(got, "[REDACTED]") {
		t.Fatal(got)
	}
}
