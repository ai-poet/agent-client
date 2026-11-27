package terminal

import (
	"errors"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestWorkspaceTerminalRequiresSandbox(t *testing.T) {
	t.Parallel()
	service := New(t.Context(), protocol.NodeID{1}, nil)
	t.Cleanup(func() { _ = service.Close() })
	launch := Launch{
		ID: protocol.TerminalID{1}, Root: t.TempDir(), Access: protocol.ProcessAccessWorkspace,
		Command: []string{"fixture-not-started"}, Viewport: protocol.TerminalViewport{Columns: 80, Rows: 24},
	}
	_, err := service.Start(t.Context(), launch)
	var refusal *protocol.Fault
	if !errors.As(err, &refusal) || refusal.Code != protocol.ErrorCodeNotConfigured {
		t.Fatalf("workspace launch without a sandbox = %v", err)
	}
	denied := fault(protocol.ErrorCodeNotConfigured, "fixture sandbox is unavailable")
	launch.Sandbox = func(string, []string, []string) ([]string, []string, func(), error) {
		return nil, nil, nil, denied
	}
	if _, err := service.Start(t.Context(), launch); !errors.Is(err, denied) {
		t.Fatalf("workspace launch refused by sandbox = %v", err)
	}
}
