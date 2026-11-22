package composer_test

import (
	"slices"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/composer"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/egoist/mygo/ui"
)

type answerLog struct {
	composer.Actions
	ids     []string
	answers []composer.Answer
}

func (l *answerLog) Answer(id string, a composer.Answer) {
	l.ids = append(l.ids, id)
	l.answers = append(l.answers, a)
}

func TestQuestionChoiceKeyboard(t *testing.T) {
	q := composer.Ask{ID: "one", Prompt: "Choose", Options: []string{"First", "Second"}, Other: true}
	ctx := &composer.Context{Docked: true, Interactions: composer.Interactions{Question: &q}}
	log := &answerLog{}
	tt := trayOf(composer.New(""), log, "en", func() *composer.Context { return ctx })
	first, ok := tt.AccessByRole(ui.RoleRadio, "First")
	if !ok || !first.Focused || first.Checked || len(log.ids) != 0 {
		t.Fatalf("initial option: shown=%v, focused=%v, checked=%v, answers=%v", ok, first.Focused, first.Checked, log.ids)
	}
	tt.Key(0, ui.KeyDown)
	tt.Key(0, ui.KeyEnter)
	if !slices.Equal(log.ids, []string{"one"}) || !slices.Equal(log.answers[0].Choices, []int{1}) {
		t.Fatalf("Down + Enter: %v %+v", log.ids, log.answers)
	}
	// A successor needs no intervening empty frame.
	q.ID, q.Multi = "two", true
	tt.Frame()
	tt.Key(0, ui.KeySpace)
	tt.Key(0, ui.KeyDown)
	tt.Key(0, ui.KeySpace)
	tt.Key(0, ui.KeyUp)
	tt.Key(0, ui.KeyEnter)
	if !slices.Equal(log.ids, []string{"one", "two"}) || !slices.Equal(log.answers[1].Choices, []int{0, 1}) {
		t.Fatalf("multi-select: %v %+v", log.ids, log.answers)
	}
	q.ID, q.Multi = "three", false
	tt.Frame()
	tt.Key(0, ui.KeyEnd) // selects Other
	field, ok := tt.AccessByRole(ui.RoleTextField, i18n.In("en", "interaction_input"))
	if !ok || !field.Focused || len(log.ids) != 2 {
		t.Fatalf("custom field: shown=%v, focused=%v, submissions=%v", ok, field.Focused, log.ids)
	}
	tt.Type("custom")
	tt.Key(0, ui.KeyEnter)
	if !slices.Equal(log.ids, []string{"one", "two", "three"}) || log.answers[2].Text != "custom" {
		t.Fatalf("custom answer: %v %+v", log.ids, log.answers)
	}

	ctx.Interactions.Question = nil
	ctx.Interactions.QuestionReceipts = []composer.QuestionReceipt{{ID: "unknown", Prompt: "Previous request"}}
	tt.Frame()
	q.ID = "unknown"
	ctx.Interactions.Question = &q
	tt.Frame()
	if _, ok := tt.AccessByRole(ui.RoleRadio, "First"); ok || len(log.ids) != 3 {
		t.Fatal("a question with an outstanding receipt reopened")
	}
}

func TestQuestionArrivalWhileTyping(t *testing.T) {
	ctx := &composer.Context{Docked: true}
	comp := composer.New("")
	tt := trayOf(comp, &answerLog{}, "en", func() *composer.Context { return ctx })
	comp.Focus()
	tt.Frame()
	tt.Type("before")
	ctx.Interactions.Question = &composer.Ask{ID: "one", Options: []string{"Only option"}}
	tt.Frame()
	if _, ok := tt.AccessByRole(ui.RoleRadio, "Only option"); !ok {
		t.Fatal("option missing in the arrival frame")
	}
	tt.Type(" after")
	if comp.Draft != "before after" {
		t.Fatalf("typing interrupted: draft %q", comp.Draft)
	}
	tt.Key(0, ui.KeyEscape)
	settle(tt)
	ctx.Interactions.Question = nil
	tt.Frame()
	ctx.Interactions.Question = &composer.Ask{ID: "one", Options: []string{"Only option"}}
	tt.Frame()
	if _, ok := tt.AccessByRole(ui.RoleRadio, "Only option"); ok {
		t.Fatal("dismissed question reopened on replay")
	}
	if err := tt.Click(i18n.In("en", "question_reply")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if first, ok := tt.AccessByRole(ui.RoleRadio, "Only option"); !ok || !first.Focused {
		t.Fatal("manual reopen did not focus the option")
	}
}
