package protocol

// Summary and Actions are stable locale keys. CauseKey is an attribution
// identifier, not provider prose. LostTokens is an estimate, not billed usage.
type TraceVerdict struct {
	KeyOrderOnly  bool     `json:"key_order_only,omitempty"`
	GapStartUs    *uint64  `json:"gap_start_us,omitempty"`
	MaxGapUs      *uint64  `json:"max_gap_us,omitempty"`
	Code          string   `json:"code"`
	Severity      string   `json:"severity"`
	CauseKey      string   `json:"cause_key,omitempty"`
	LostTokens    uint64   `json:"lost_tokens,omitempty"`
	Summary       string   `json:"summary"`
	Actions       []string `json:"actions,omitempty"`
	Path          string   `json:"path,omitempty"`
	Volatile      string   `json:"volatile,omitempty"`
	RetryAfterMs  *uint64  `json:"retry_after_ms,omitempty"`
	RetryDelayMs  *uint64  `json:"retry_delay_ms,omitempty"`
	RetrySequence []string `json:"retry_sequence,omitempty"`
}
