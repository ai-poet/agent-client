//go:build !windows

package files

import "github.com/ai-poet/agent-client/internal/protocol"

func driveDetail(string) (string, *protocol.FileBrowserMedium) { return "", nil }
