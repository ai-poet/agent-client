package store

import (
	"database/sql"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// Exercise the default insights path, including visibility, pricing, percentiles
// and the first page. Fixture insertion is outside the timed loop.
func BenchmarkUsageInsights100K(b *testing.B) {
	storage, node, session, turn, now := agentHistoryFixture(b, protocol.WorkModeCode)
	ctx := b.Context()
	const rows = 100000
	if err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		_, err := db.ExecContext(ctx, `WITH RECURSIVE fixture(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM fixture WHERE n<?1)
			INSERT INTO agent_events(session,id,turn,body)
			SELECT ?2,'record-' || n,?3,json_object('created',?4,'usage',json_object('input',120,'output',24),
				'metadata',json_object('execution',json_object('observation',json_object('id','request-' || n,'attempts',
				json_array(json_object('index',1,'status',200,'outcome','succeeded','end_kind','completed','first_token_us',n+1000,'elapsed_us',n+3000))))))
			FROM fixture`, rows, session.ID.String(), turn.String(), now.Format(time.RFC3339Nano))
		return err
	}); err != nil {
		b.Fatal(err)
	}
	q := protocol.DefaultUsageQuery()
	q.StartMs, q.EndMs = now.UnixMilli(), now.Add(24*time.Hour).UnixMilli()
	q.Dimension, q.ObservedOnly = protocol.UsageDimensionModel, true
	for b.Loop() {
		err := storage.Node.Read(ctx, func(db *sql.Conn) error {
			report, err := readAgentUsage(ctx, db, node, q, true)
			if err == nil && (report.Insights == nil || report.Insights.Attempts != rows || len(report.Requests.Items) != 20) {
				b.Fatal("incomplete first page or totals")
			}
			return err
		})
		if err != nil {
			b.Fatal(err)
		}
	}
}
