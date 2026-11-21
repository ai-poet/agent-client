package link

import (
	"errors"
	"testing"

	"github.com/ai-poet/agent-client/internal/protocol"
)

// TestFaultValueFallbackKeepsItsWording pins the fallback text users read in
// failure details. An error of an unknown type still surfaces as one stable
// sentence; naming its type belongs in a log, not in this contract.
func TestFaultValueFallbackKeepsItsWording(t *testing.T) {
	fault := faultValue(errors.New("boom"))
	if fault.Code != protocol.ErrorCodeUnavailable || fault.Message != "Link connection is unavailable" {
		t.Fatalf("fallback fault = %s: %s", fault.Code, fault.Message)
	}
}
