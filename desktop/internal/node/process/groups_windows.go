package process

import "github.com/ai-poet/agent-client/desktop/internal/protocol"

// A Windows command lives in a kill-on-close Job: the system ends it when
// the Node's last handle closes, a crash included, so nothing is recorded.
func recordGroup(string, protocol.RequestID, int) func() { return func() {} }

func reapGroups(string) error { return nil }
