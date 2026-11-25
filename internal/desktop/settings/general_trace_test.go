package settings

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
)

func TestTraceSettingsConfirmation(t *testing.T) {
	m := New().Model
	m.Live, m.TraceReady = true, true
	clears := 0
	m.ClearAllTraces = func() { clears++ }
	var answer func(int)
	tester := ui.NewTesterWith(func(c *ui.Context) {
		kit.BindConfirmations(c, func(req kit.Confirmation, respond func(int)) {
			if req.Default != 0 || req.Cancel != 0 || len(req.Destructive) != 1 || req.Destructive[0] != 1 {
				t.Fatal(req)
			}
			answer = respond
		})
		traceSettings(c, m)
	}, 600, 700, ui.TesterOptions{})
	tester.SetLocale("en")
	tester.Frame()
	for choice := range 2 {
		pressButton(t, tester, i18n.In("en", "trace_clear_all"))
		tester.Frame()
		if answer == nil || clears != 0 {
			t.Fatalf("clear before confirmation: %d", clears)
		}
		answer(choice)
		answer = nil
		tester.Frame()
		if clears != choice {
			t.Fatalf("answer %d sent %d clears", choice, clears)
		}
	}
}
