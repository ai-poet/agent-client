package wire

import (
	"errors"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// TestFrameNetworkFaultKeepsItsWording pins the text users read in failure
// details. Diagnostics belong in logs, not in this string: changing it breaks
// the interface copy that promises a single sentence, and anything that needs
// to tell a dropped frame from an unknown error must add its own channel.
func TestFrameNetworkFaultKeepsItsWording(t *testing.T) {
	var fault *protocol.Fault
	if !errors.As(network(), &fault) {
		t.Fatal("a dropped frame is not a fault")
	}
	if fault.Code != protocol.ErrorCodeUnavailable || fault.Message != "Link connection is unavailable" {
		t.Fatalf("frame fault = %s: %s", fault.Code, fault.Message)
	}
}
