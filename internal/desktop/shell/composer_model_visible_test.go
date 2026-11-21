package shell

import (
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/internal/desktop/composer"
	"github.com/ai-poet/agent-client/internal/desktop/preview"
	"github.com/egoist/mygo/ui"
)

func findModelChip(tt *ui.Tester, prefix string) (ui.AccessInfo, bool) {
	for _, e := range tt.AccessAll() {
		if e.Role == ui.RoleButton && strings.HasPrefix(e.Name, prefix) {
			return e, true
		}
	}
	return ui.AccessInfo{}, false
}

// TestComposerModelAlwaysVisibleAcrossBreakpoints verifies that the current
// model chip remains visible, inside bounds, and interactive across wide,
// compact, narrow, and tight breakpoints in both main-pane and docked sidebar
// hosting.
func TestComposerModelAlwaysVisibleAcrossBreakpoints(t *testing.T) {
	chipPrefix := zh("composer_model_settings") + ": "
	effortHeading := zh("composer_effort")
	breakpoints := []int{1200, 900, 640, 440, 360}

	for _, docked := range []bool{false, true} {
		host := "main"
		if docked {
			host = "sidebar"
		}
		for _, width := range breakpoints {
			t.Run(fmt.Sprintf("%s/%d", host, width), func(t *testing.T) {
				tt, comp, _ := composerWith(t, width, func(_ *composer.State, ctx *composer.Context) {
					ctx.Docked = docked
				})
				comp.Team, comp.Model, comp.Effort, comp.Features = "", "claude-opus-5-5", 2, nil
				tt.Frame()
				tt.Frame()

				chip, ok := findModelChip(tt, chipPrefix)
				if !ok {
					t.Fatalf("%s@%d: model chip missing: %q", host, width, tt.Texts())
				}
				if chip.Disabled {
					t.Fatalf("%s@%d: model chip disabled: %+v", host, width, chip)
				}
				if !strings.Contains(chip.Name, "Claude Opus 5.5") {
					t.Fatalf("%s@%d: model chip name %q does not include current model", host, width, chip.Name)
				}
				window := ui.Rect{W: float32(width), H: 520}
				if chip.Bounds.W <= 0 || chip.Bounds.H <= 0 || !inside(chip.Bounds, window) {
					t.Fatalf("%s@%d: model chip bounds %v outside window %v", host, width, chip.Bounds, window)
				}
				if !tt.HasText("Claude Opus 5.5") {
					t.Fatalf("%s@%d: visible model name missing from frame texts: %q", host, width, tt.Texts())
				}

				if !docked && (width == 640 || width == 440 || width == 360) {
					shot(t, tt, fmt.Sprintf("composer-model-visible-%d-zh", width))
				}
				if docked && width == 360 {
					shot(t, tt, "composer-model-visible-360-sidebar-zh")
				}

				// Click the model chip to open the model controls popover.
				tt.ClickAt(chip.Bounds.X+chip.Bounds.W/2, chip.Bounds.Y+chip.Bounds.H/2)
				rest(tt)
				panels, _ := popups(tt)
				if len(panels) != 1 || !inside(panels[0].Bounds, window) {
					t.Fatalf("%s@%d: model controls popover invalid: %+v in %v", host, width, panels, window)
				}
				if _, ok := tt.AccessByRole(ui.RoleSlider, effortHeading); !ok {
					t.Fatalf("%s@%d: model controls popover missing effort slider: %q", host, width, tt.Texts())
				}

				// Open the model picker and change both model and effort.
				pressButton(t, tt, "Claude Opus 5.5")
				rest(tt)
				if !docked && width == 360 {
					shot(t, tt, "composer-model-visible-360-picker-zh")
				}
				panels, _ = popups(tt)
				if len(panels) != 1 || !inside(panels[0].Bounds, window) {
					t.Fatalf("%s@%d: model picker popover leaves window: %+v in %v", host, width, panels, window)
				}
				pressButton(t, tt, zh("provider_openai"))
				rest(tt)
				row := modelRow(t, tt, "GPT-5.2")
				point(tt, row)
				pressEffort(t, tt, "high")
				if comp.Model != "gpt-5.2" || composer.Efforts[int(comp.Effort)] != "high" {
					t.Fatalf("%s@%d: chose %q at effort %v, want gpt-5.2 at high", host, width, comp.Model, comp.Effort)
				}
			})
		}
	}
}

// TestComposerModelPreservedAcrossWidthChanges ensures resizing across wide,
// compact, narrow, and tight widths retains the chosen model and reasoning
// effort in both the full shell window and a docked sidebar composer.
func TestComposerModelPreservedAcrossWidthChanges(t *testing.T) {
	chipPrefix := zh("composer_model_settings") + ": "

	t.Run("full shell window", func(t *testing.T) {
		a := New(preview.Load())
		tt := ui.NewTesterWith(a.View, 1200, 820, opaque)
		tt.SetLocale("zh-CN")
		tt.SetDark(true)
		a.Sidebar.Selected = preview.Big
		tt.Frame()
		tt.Frame()

		comp := a.Composer(preview.Big)
		comp.Team, comp.Model, comp.Effort = "", "gpt-5.2", 3 // high
		tt.Frame()
		tt.Frame()

		for _, step := range []struct {
			width   int
			navOpen bool
		}{
			{1200, true},
			{900, true},
			{640, true},
			{640, false},
			{440, false},
			{360, false},
			{1200, true},
		} {
			a.NavOpen = step.navOpen
			tt.SetSize(step.width, 820)
			tt.Frame()
			tt.Advance(300 * time.Millisecond)
			tt.Frame()
			tt.Frame()
			if comp.Model != "gpt-5.2" || composer.Efforts[int(comp.Effort)] != "high" {
				t.Fatalf("width=%d nav=%v: lost model/effort: model=%q effort=%v", step.width, step.navOpen, comp.Model, comp.Effort)
			}
			chip, ok := findModelChip(tt, chipPrefix)
			if !ok || !strings.Contains(chip.Name, "GPT-5.2") {
				t.Fatalf("width=%d nav=%v: model chip missing or stale: ok=%v chip=%+v texts=%q", step.width, step.navOpen, ok, chip, tt.Texts())
			}
		}
	})

	t.Run("docked sidebar composer", func(t *testing.T) {
		tt, comp, _ := composerWith(t, 640, func(_ *composer.State, ctx *composer.Context) {
			ctx.Docked = true
		})
		comp.Team, comp.Model, comp.Effort = "", "gpt-5.2", 3 // high
		tt.Frame()
		tt.Frame()

		for _, width := range []int{640, 440, 360, 900, 360} {
			tt.SetSize(width, 520)
			tt.Frame()
			tt.Frame()
			if comp.Model != "gpt-5.2" || composer.Efforts[int(comp.Effort)] != "high" {
				t.Fatalf("docked width=%d: lost model/effort: model=%q effort=%v", width, comp.Model, comp.Effort)
			}
			chip, ok := findModelChip(tt, chipPrefix)
			if !ok || !strings.Contains(chip.Name, "GPT-5.2") {
				t.Fatalf("docked width=%d: model chip missing or stale: ok=%v chip=%+v", width, ok, chip)
			}
		}
	})
}

// TestComposerModelChipKeyboardOpenAndEscape verifies Tab reachability,
// Enter/Space opening, and Escape closing across wide and narrow widths,
// including that Escape closes the popover without stopping a running turn.
func TestComposerModelChipKeyboardOpenAndEscape(t *testing.T) {
	chipPrefix := zh("composer_model_settings") + ": "
	effortHeading := zh("composer_effort")

	for _, width := range []int{1200, 440, 360} {
		for _, docked := range []bool{false, true} {
			host := "main"
			if docked {
				host = "sidebar"
			}
			t.Run(fmt.Sprintf("%s/%d", host, width), func(t *testing.T) {
				tt, comp, actions := composerWith(t, width, func(_ *composer.State, ctx *composer.Context) {
					ctx.Docked = docked
				})
				comp.Team, comp.Model, comp.Effort, comp.Running = "", "claude-opus-5-5", 2, true
				tt.Frame()
				tt.Frame()

				chip, ok := findModelChip(tt, chipPrefix)
				if !ok {
					t.Fatalf("%s@%d: model chip missing: %q", host, width, tt.Texts())
				}

				for _, trigger := range []struct {
					name string
					key  ui.Key
				}{
					{"Enter", ui.KeyEnter},
					{"Space", ui.KeySpace},
				} {
					if err := tt.Click(zh("composer")); err != nil {
						t.Fatalf("%s@%d (%s): focus draft: %v", host, width, trigger.name, err)
					}
					tt.Frame()
					reached := false
					for range 12 {
						tt.Key(0, ui.KeyTab)
						tt.Frame()
						if tt.Focused(chip.Name) {
							reached = true
							break
						}
					}
					if !reached {
						t.Fatalf("%s@%d (%s): could not reach model chip %q via Tab", host, width, trigger.name, chip.Name)
					}
					tt.Key(0, trigger.key)
					rest(tt)
					if panels, _ := popups(tt); len(panels) != 1 {
						t.Fatalf("%s@%d (%s): expected 1 popover open, got %d", host, width, trigger.name, len(panels))
					}
					if _, ok := tt.AccessByRole(ui.RoleSlider, effortHeading); !ok {
						t.Fatalf("%s@%d (%s): popover lacks model controls: %q", host, width, trigger.name, tt.Texts())
					}

					tt.Key(0, ui.KeyEscape)
					tt.Advance(300 * time.Millisecond)
					tt.Frame()
					if panels, _ := popups(tt); len(panels) != 0 {
						t.Fatalf("%s@%d (%s): Escape left %d popovers open", host, width, trigger.name, len(panels))
					}
					if len(actions.log) != 0 {
						t.Fatalf("%s@%d (%s): Escape on model popover triggered composer actions %v", host, width, trigger.name, actions.log)
					}
				}
			})
		}
	}
}

// TestComposerSelectModelChipVisibleWhenEmptyAtNarrowWidths verifies that even
// with no models configured, the Select model chip stays visible and clickable
// at narrow widths.
func TestComposerSelectModelChipVisibleWhenEmptyAtNarrowWidths(t *testing.T) {
	label := zh("composer_select_model")
	for _, width := range []int{640, 440, 360} {
		t.Run(fmt.Sprintf("%d", width), func(t *testing.T) {
			opened := 0
			tt, _, _ := composerWith(t, width, func(_ *composer.State, ctx *composer.Context) {
				ctx.Models, ctx.Families = nil, nil
				ctx.SelectModel = func() { opened++ }
			})
			chip, ok := tt.AccessByRole(ui.RoleButton, label)
			if !ok || chip.Disabled {
				t.Fatalf("width=%d: expected enabled Select model chip, got ok=%v chip=%+v", width, ok, chip)
			}
			tt.ClickAt(chip.Bounds.X+chip.Bounds.W/2, chip.Bounds.Y+chip.Bounds.H/2)
			tt.Frame()
			if opened != 1 {
				t.Fatalf("width=%d: Select model chip called SelectModel %d times, want 1", width, opened)
			}
		})
	}
}
