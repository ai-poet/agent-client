package composer

import "testing"

func TestDefaultModelPreference(t *testing.T) {
	gpt := Model{ID: "provider-a/gpt-5"}
	sonnet4 := Model{ID: "provider-b/claude-sonnet-4-6"}
	sonnet5 := Model{ID: "provider-b/claude-sonnet-5"}
	sonnet55 := Model{ID: "provider-b/claude-sonnet-5.5-latest"}
	deepseek := Model{ID: "provider-a/deepseek-v4"}
	for _, tc := range []struct {
		name   string
		models []Model
		want   string
	}{
		{"no models", nil, ""},
		{"first available", []Model{gpt, deepseek}, gpt.ID},
		{"any Sonnet", []Model{gpt, sonnet4}, sonnet4.ID},
		{"Sonnet 5", []Model{gpt, sonnet4, sonnet5}, sonnet5.ID},
		{"Sonnet 5.5", []Model{gpt, sonnet5, sonnet55}, sonnet55.ID},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := New("").Chosen(tc.models).ID; got != tc.want {
				t.Fatalf("default = %q, want %q", got, tc.want)
			}
		})
	}
	if got := New(gpt.ID).Chosen([]Model{gpt, sonnet55}).ID; got != gpt.ID {
		t.Fatalf("default preference replaced explicit selection with %q", got)
	}
}
