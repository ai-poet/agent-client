package live

import (
	"context"
	"time"

	"github.com/egoist/mygo/ui"
)

func (r *Runtime) readInsights(p *insightsPage, more bool) {
	s := &p.state
	q := p.query
	if more {
		q.Before = s.Next()
		if s.Loading || q.Before == nil {
			return
		}
	}
	p.stop()
	s.Error = ""
	s.Loading = false
	if !more {
		s.Report = nil
		var err error
		q, err = s.Draft.Query(time.Now())
		if err != nil {
			s.Error = "insights_invalid"
			return
		}
		p.query = q
	}
	owner := p.owner
	if owner == nil || !owner.controller.Connected() {
		s.Error = "unavailable"
		return
	}
	ctx, cancel := context.WithCancel(owner.ctx)
	p.cancel = cancel
	s.Loading = true
	owner.async(func() {
		report, err := owner.controller.ReadUsageV2(ctx, q)
		r.enqueue(func(*ui.Context) {
			if ctx.Err() != nil || r.insights != p || r.runtimeFor(p.node) != owner {
				return
			}
			s.Loading = false
			if err != nil {
				s.Error = "usage_query_failed"
				return
			}
			if more && s.Report != nil {
				report.Requests.Items = append(s.Report.Requests.Items, report.Requests.Items...)
			}
			s.Report = &report
		})
	})
}
