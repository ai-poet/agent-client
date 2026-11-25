package panel

import (
	"fmt"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// The same category names and chart tokens as the conversation context meter.
var traceCategoryChart = map[protocol.ConversationObservationCategoryKind]int{
	"system": 3, "tools": 0, "user": 2, "skills": 4, "injected": 3, "assistant": 1, "results": 2,
}

func traceCategories(c *ui.Context, r traceRow) {
	var total float64
	for _, v := range r.Categories {
		if _, ok := traceCategoryChart[v.Kind]; ok && v.Tokens != nil {
			total += float64(*v.Tokens)
		}
	}
	if total == 0 {
		return
	}
	p := theme.Of(c)
	ui.Column(c.Key("composition")).FillWidth().Gap(4).Children(func() {
		ui.Text(c, i18n.T(c, "meter_title")).FontSize(14).FontWeight(500)
		ui.Row(c.Key("bar")).FillWidth().Height(6).Radius(3).Children(func() {
			for _, v := range r.Categories {
				chart, ok := traceCategoryChart[v.Kind]
				if !ok || v.Tokens == nil || *v.Tokens == 0 {
					continue
				}
				ui.Box(c.Key(v.Kind)).Basis(0).Grow(float32(float64(*v.Tokens) / total)).Height(6).Background(p.Chart[chart])
			}
		})
		for _, v := range r.Categories {
			chart, ok := traceCategoryChart[v.Kind]
			if !ok || v.Tokens == nil || *v.Tokens == 0 {
				continue
			}
			label := i18n.T(c, "meter_"+string(v.Kind))
			row := ui.Row(c.Key(v.Kind)).FillWidth().Height(26).Gap(8).AlignItems(ui.Center)
			if v.Method != nil && *v.Method == "identified_source_characters_quarter" {
				kit.Tooltip(c, row, i18n.T(c, "meter_identified_note"))
			}
			row.Children(func() {
				ui.Box(c).Size(7, 7).Radius(2).Background(p.Chart[chart])
				ui.Text(c, label).FontSize(13).TextColor(p.MutedForeground).Grow(1)
				ui.Text(c, fmt.Sprintf("~%d   %.1f%%", *v.Tokens, 100*float64(*v.Tokens)/total)).FontSize(13).FontFeatures("tnum")
			})
		}
		if !r.complete {
			ui.Text(c, i18n.T(c, "meter_payload_partial")).FontSize(12).TextColor(p.MutedForeground)
		}
	})
}
