package shell

import "github.com/ai-poet/agent-client/desktop/internal/workbench"

// View-facing aliases keep the shell API small without duplicating projection
// state or transcript types.
type Status = workbench.Status

const (
	Disconnected = workbench.Disconnected
	Connecting   = workbench.Connecting
	Connected    = workbench.Connected
	Failed       = workbench.Failed
)

type Snapshot = workbench.Snapshot
type SessionSummary = workbench.SessionSummary
type TranscriptKind = workbench.TranscriptKind

type TranscriptEntry = workbench.TranscriptEntry

const (
	TranscriptUser      = workbench.TranscriptUser
	TranscriptAssistant = workbench.TranscriptAssistant
	TranscriptReasoning = workbench.TranscriptReasoning
	TranscriptActivity  = workbench.TranscriptActivity
	TranscriptNotice    = workbench.TranscriptNotice
)
