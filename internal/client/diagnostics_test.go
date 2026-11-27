package client

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestReadRetainsNodeDiagnostic(t *testing.T) {
	failure := protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "Node could not finish the read", ReportOwned: true}
	for _, test := range []struct {
		name       string
		admission  *protocol.Fault
		completion *protocol.Fault
		owned      bool
		message    string
	}{
		{name: "admission", admission: &failure, owned: true, message: failure.Message},
		{name: "completion", completion: &failure, owned: true, message: failure.Message},
		{name: "lost response", message: "Link connection was lost before the reply"},
	} {
		t.Run(test.name, func(t *testing.T) {
			c := New(&clientTransport{dispatch: func(protocol.Request) (link.Admission, error) {
				if test.admission != nil {
					return link.Admission{}, test.admission
				}
				completion := make(chan link.Response, 1)
				if test.completion != nil {
					completion <- link.Response{Fault: test.completion}
				}
				close(completion)
				return link.Admission{Completion: completion}, nil
			}})
			t.Cleanup(func() { _ = c.Close() })
			_, err := c.read(t.Context(), protocol.Command{Kind: "snapshot"})
			if fault := observerFault(err); fault == nil || fault.Code != protocol.ErrorCodeUnavailable || fault.ReportOwned != test.owned || fault.Message != test.message {
				t.Fatalf("read fault = %+v", fault)
			}
			if failure.Code != protocol.ErrorCodeOutcomeUnknown {
				t.Fatalf("read rewrote the original fault: %+v", failure)
			}
		})
	}
}
