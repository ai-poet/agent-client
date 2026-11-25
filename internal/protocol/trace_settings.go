package protocol

// Revision is the expected revision on a save and the committed revision on read.
type TraceSettings struct {
	Revision       uint64 `json:"revision"`
	Enabled        bool   `json:"enabled"`
	GatewayEnabled bool   `json:"gateway_enabled"`
}

func (s *TraceSettings) UnmarshalJSON(data []byte) error {
	type fields TraceSettings
	var value fields
	if err := DecodeObject(data, &value, []string{"revision", "enabled", "gateway_enabled"}, false); err != nil {
		return err
	}
	*s = TraceSettings(value)
	return nil
}
