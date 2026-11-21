package composer

import "testing"

func TestModelDisplayAndShortName(t *testing.T) {
	cases := []struct {
		model     Model
		wantName  string
		wantShort string
	}{
		{
			model:     Model{ID: "claude-opus-5-5", Name: "Claude Opus 5.5"},
			wantName:  "Claude Opus 5.5",
			wantShort: "Claude Opus 5.5",
		},
		{
			model:     Model{ID: "anthropic/claude-sonnet-4-5", Name: "anthropic/claude-sonnet-4-5"},
			wantName:  "anthropic/claude-sonnet-4-5",
			wantShort: "claude-sonnet-4-5",
		},
		{
			model:     Model{ID: "cheaprouter::deepseek-v4", Name: ""},
			wantName:  "cheaprouter::deepseek-v4",
			wantShort: "deepseek-v4",
		},
		{
			model:     Model{ID: "openai/gpt-5.2", Name: "  "},
			wantName:  "openai/gpt-5.2",
			wantShort: "gpt-5.2",
		},
	}
	for _, tc := range cases {
		if got := modelDisplayName(tc.model); got != tc.wantName {
			t.Errorf("modelDisplayName(%+v) = %q, want %q", tc.model, got, tc.wantName)
		}
		if got := shortModelName(tc.model); got != tc.wantShort {
			t.Errorf("shortModelName(%+v) = %q, want %q", tc.model, got, tc.wantShort)
		}
	}
}

func TestModelChipWidthBreakpoints(t *testing.T) {
	cases := []struct {
		toolbar float32
		want    float32
	}{
		{0, chipWidth},
		{728, chipWidth},
		{compactWidth, chipWidth},
		{568, compactChip},
		{iconWidth, compactChip},
		{368, narrowChip},
		{tightWidth, narrowChip},
		{288, tightChipMax},
		{248, 100},
		{200, tightChipMin},
	}
	for _, tc := range cases {
		if got := modelChipWidth(tc.toolbar); got != tc.want {
			t.Errorf("modelChipWidth(%v) = %v, want %v", tc.toolbar, got, tc.want)
		}
	}
}

func TestOpenModelsAlwaysTargetsPickerAtNarrowWidths(t *testing.T) {
	ctx := &Context{Models: []Model{{ID: "gpt-5.2", Name: "GPT-5.2", Reasoning: true}}}
	for _, width := range []float32{728, 531, 368, 288} {
		s := New("gpt-5.2")
		s.width = width
		s.settings = true
		s.OpenModels(ctx)
		if !s.picker || s.settings || s.layer != layerMain {
			t.Errorf("width=%v: OpenModels left picker=%v settings=%v layer=%v", width, s.picker, s.settings, s.layer)
		}
	}
}

func TestEscapeDoesNotStopRunningTurnWhileModelPopoverOpen(t *testing.T) {
	s := New("gpt-5.2")
	s.Running = true
	s.picker = true
	if s.Escapes() {
		t.Fatal("Escapes() should defer to the open model popover")
	}
	if s.Escape(nil) {
		t.Fatal("Escape() should not consume Escape while the model popover is open")
	}
	s.picker = false
	s.settings = true
	if s.Escapes() {
		t.Fatal("Escapes() should defer to the open settings popover")
	}
	if s.Escape(nil) {
		t.Fatal("Escape() should not consume Escape while the settings popover is open")
	}
}
