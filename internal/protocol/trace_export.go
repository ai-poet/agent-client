package protocol

const MaxTraceExportBytes = 64 << 20

type TraceExportQuery struct {
	Session SessionID `json:"session"`
	Scope   string    `json:"scope"`
	Turn    TurnID    `json:"turn,omitzero"`
	Mode    string    `json:"mode"`
}

type TraceDownload struct {
	Query    TraceExportQuery `json:"query"`
	Stream   StreamID         `json:"stream"`
	Size     uint64           `json:"size"`
	Revision string           `json:"revision"`
}
