package store

import (
	"database/sql"
	"encoding/json"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestAnthropicCacheWriteCost(t *testing.T) {
	t.Parallel()
	storage, _, _, _, _ := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		catalog := []byte(`{"cost":{"input":3,"output":15,"cache_read":0.3,"cache_write":3.75}}`)
		if _, err := db.ExecContext(ctx, "INSERT INTO model_catalog(provider,id,body) VALUES('anthropic','fixture',?1)", catalog); err != nil {
			return err
		}
		for _, test := range []struct {
			name, raw string
			micros    uint64
			write     uint64
		}{
			{"default", `{}`, 768, 150},
			{"five_minutes", `{"cache_creation":{"ephemeral_5m_input_tokens":40,"ephemeral_1h_input_tokens":0}}`, 768, 150},
			{"one_hour", `{"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":40}}`, 858, 240},
			{"mixed", `{"cache_creation":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":30}}`, 836, 218},
			{"incomplete", `{"cache_creation":{"ephemeral_1h_input_tokens":30}}`, 0, 0},
			{"inconsistent", `{"cache_creation":{"ephemeral_5m_input_tokens":20,"ephemeral_1h_input_tokens":30}}`, 0, 0},
		} {
			t.Run(test.name, func(t *testing.T) {
				usage := canonical.Usage{Input: ptr(uint64(200)), Output: ptr(uint64(20)), CacheRead: ptr(uint64(60)), CacheWrite: ptr(uint64(40)), Provider: json.RawMessage(test.raw)}
				cost, err := estimateAgentCost(ctx, db, modelPrices{}, "anthropic", "anthropic", "fixture", usage)
				if err != nil {
					t.Fatal(err)
				}
				if test.micros == 0 {
					if cost != nil {
						t.Fatalf("incomplete cache pricing should be unknown: %#v", cost)
					}
					return
				}
				if cost == nil || cost.UsdMicros != test.micros || cost.Breakdown == nil || cost.Breakdown.CacheWrite != test.write {
					t.Fatalf("cost = %#v, want total %d and cache writes %d", cost, test.micros, test.write)
				}
			})
		}
		// The one-hour multiplier applies to the selected context price tier.
		tier := []byte(`{"cost":{"input":3,"output":15,"cache_read":0.3,"cache_write":3.75,"tiers":[{"tier":{"type":"context","size":150},"input":6,"output":30,"cache_read":0.6,"cache_write":7.5}]}}`)
		if _, err := db.ExecContext(ctx, "UPDATE model_catalog SET body=?1 WHERE provider='anthropic' AND id='fixture'", tier); err != nil {
			return err
		}
		usage := canonical.Usage{Input: ptr(uint64(200)), Output: ptr(uint64(20)), CacheRead: ptr(uint64(60)), CacheWrite: ptr(uint64(40)), Provider: json.RawMessage(`{"cache_creation":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":30}}`)}
		cost, err := estimateAgentCost(ctx, db, modelPrices{}, "anthropic", "anthropic", "fixture", usage)
		if err != nil {
			return err
		}
		if cost == nil || cost.UsdMicros != 1671 || cost.Breakdown == nil || cost.Breakdown.CacheWrite != 435 {
			t.Fatalf("tiered cost = %#v, want total 1671 and cache writes 435", cost)
		}
		if _, err = db.ExecContext(ctx, "INSERT INTO model_catalog(provider,id,body) VALUES('reseller','fixture',?1)", catalog); err != nil {
			return err
		}
		cost, err = estimateAgentCost(ctx, db, modelPrices{}, "reseller", "anthropic", "fixture", usage)
		if err != nil {
			return err
		}
		if cost != nil {
			t.Fatalf("compatible API invented the reseller's one-hour price: %#v", cost)
		}
		// Bedrock bills Claude's one-hour writes as Anthropic does.
		if _, err = db.ExecContext(ctx, "INSERT INTO model_catalog(provider,id,body) VALUES('amazon-bedrock','fixture',?1)", tier); err != nil {
			return err
		}
		cost, err = estimateAgentCost(ctx, db, modelPrices{}, "bedrock", "bedrock", "fixture", usage)
		if err != nil {
			return err
		}
		if cost == nil || cost.UsdMicros != 1671 || cost.Breakdown == nil || cost.Breakdown.CacheWrite != 435 {
			// Errorf: a Fatalf here would exit while holding the writer.
			t.Errorf("Bedrock cost = %#v, want total 1671 and cache writes 435", cost)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestCloudUsageCost(t *testing.T) {
	const model = "fixture"
	for _, tc := range []struct{ api, preset, supplier string }{
		{"bedrock", "bedrock", "amazon-bedrock"},
		{"vertex_ai", "vertex-ai", "google-vertex"},
	} {
		prices := modelPrices{{tc.supplier, model}: []byte(`{"cost":{"input":3,"output":15,"cache_read":0.3,"cache_write":3.75}}`)}
		usage := canonical.Usage{Input: ptr(uint64(200)), Output: ptr(uint64(20)), CacheRead: ptr(uint64(60)), CacheWrite: ptr(uint64(40))}
		for _, preset := range []string{tc.preset, ""} {
			suppliers := priceSuppliers(preset, tc.api)
			if len(suppliers) == 0 || suppliers[0] != tc.supplier {
				t.Fatalf("%s/%s: suppliers %v", preset, tc.api, suppliers)
			}
			cost, err := estimateAgentCost(t.Context(), nil, prices, preset, tc.api, model, usage)
			if err != nil || cost == nil || cost.UsdMicros != 768 || cost.Breakdown.CacheWrite != 150 {
				t.Errorf("%s/%s: cost %+v, %v", preset, tc.api, cost, err)
			}
		}
		if tc.api == "bedrock" {
			usage.Provider = json.RawMessage(`{"cache_creation":{"ephemeral_5m_input_tokens":39,"ephemeral_1h_input_tokens":0}}`)
			if cost, err := estimateAgentCost(t.Context(), nil, prices, tc.preset, tc.api, model, usage); err != nil || cost != nil {
				t.Errorf("inconsistent Bedrock cache writes: cost %+v, %v", cost, err)
			}
		}
	}
}
