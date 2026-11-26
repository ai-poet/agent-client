package trace

import (
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"strings"
	"testing"
)

func TestSummaryGroupsDiagnosisLoss(t *testing.T) {
	rows := []row{{Verdict: &protocol.TraceVerdict{Code: "cache_partial", LostTokens: 1024}}, {Verdict: &protocol.TraceVerdict{Code: "cache_partial", LostTokens: 2048}}, {Outcome: "cancelled"}}
	out := string(summary(rows))
	if !strings.Contains(out, "cache_partial: 2; estimated lost tokens: 3072") || !strings.Contains(out, "cancelled: 1") {
		t.Fatal(out)
	}
}
