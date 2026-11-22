//go:build !windows

package files

import "github.com/ai-poet/cheaprouter-harness/internal/protocol"

func driveDetail(string) (string, *protocol.FileBrowserMedium) { return "", nil }
