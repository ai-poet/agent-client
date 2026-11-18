//go:build !darwin && !linux && !windows

package core

import "github.com/ai-poet/agent-client/desktop/internal/protocol"

func trashNative(string) (string, error) {
	return "", fileFault(protocol.ErrorCodeUnavailable, "native Trash is unavailable on this platform")
}
