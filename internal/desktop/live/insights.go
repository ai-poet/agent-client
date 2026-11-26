package live

import (
	"context"
	"maps"
	"slices"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/insights"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

type insightsPage struct {
	owner      *Runtime
	node       string
	seen       bool
	cancel     context.CancelFunc
	jumpCancel context.CancelFunc
	query      protocol.UsageQuery
	state      insights.State
}

func (p *insightsPage) stop() {
	if p.jumpCancel != nil {
		p.jumpCancel()
		p.jumpCancel = nil
	}
	if p.cancel != nil {
		p.cancel()
		p.cancel = nil
	}
}

func (r *Runtime) finishInsightsFrame() {
	if p := r.insights; p != nil {
		if !p.seen {
			p.stop()
			r.insights = nil
		} else {
			p.seen = false
		}
	}
}

func (r *Runtime) insightsFrame(c *ui.Context) {
	p := r.insights
	newPage := p == nil
	if newPage {
		p = &insightsPage{node: r.controller.Target().String(), state: insights.State{Draft: insights.Filters{Days: 7, Dimension: protocol.UsageDimensionModel}}}
		r.insights = p
	}
	p.seen = true
	owner := r.runtimeFor(p.node)
	if owner != nil && !owner.controller.Connected() {
		owner = nil
	}
	if newPage || owner != p.owner {
		p.owner = owner
		r.readInsights(p, false)
	}
	insights.Page(c, &p.state, insights.Actions{
		Host: func(c *ui.Context) {
			nodes := slices.Sorted(maps.Keys(r.bindings))
			if !slices.Contains(nodes, p.node) && p.node != r.controller.Target().String() {
				nodes = append(nodes, p.node)
			}
			nodes = append([]string{r.controller.Target().String()}, nodes...)
			names := make([]string, len(nodes))
			for i, id := range nodes {
				names[i] = r.configurationHostName(id)
			}
			if _, changed := kit.Select(c, "insights-host", i18n.T(c, "insights_host"), &p.node, names, false, nodes); changed {
				p.owner = r.runtimeFor(p.node)
				p.state.Draft = insights.Filters{Days: p.state.Draft.Days, StartDate: p.state.Draft.StartDate, EndDate: p.state.Draft.EndDate, Dimension: p.state.Draft.Dimension, ObservedOnly: p.state.Tab == 1}
				r.readInsights(p, false)
			}
		},
		Apply: func() { r.readInsights(p, false) }, More: func() { r.readInsights(p, true) },
		Open: func(row protocol.UsageRequest) { r.openInsight(c, p, row) },
	})
}
