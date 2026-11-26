package live

import (
	"log/slog"
	"maps"
	"path/filepath"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestDraftModelRejectionLog(t *testing.T) {
	closeLog, err := logging.Install(filepath.Join(t.TempDir(), "profile"), "desktop")
	if err != nil {
		t.Fatal(err)
	}
	defer closeLog()
	var record logging.Fields
	logging.SetReport(func(level slog.Level, fields logging.Fields) {
		if level == slog.LevelWarn && fields["area"] == "draft_model" {
			record = fields
		}
	})
	defer logging.SetReport(nil)
	lane := providers.CheapRouterLaneID(cheapauth.LaneCodex)
	own := protocol.ProviderID{1}
	snapshot := []protocol.ConversationProvider{
		{ID: lane, Models: []protocol.ConversationModel{{ID: "vendor/model"}}},
		{ID: own, Enabled: true},
	}
	for _, tc := range []struct {
		id                               protocol.ProviderID
		lane                             string
		managed, present, enabled, model bool
	}{
		{lane, cheapauth.LaneCodex, true, true, false, true},
		{own, "", false, true, true, false},
		{providers.CheapRouterProviderID, "", true, false, false, false},
	} {
		t.Run(tc.id.String(), func(t *testing.T) {
			record = nil
			logDraftModelRejection(modelKey(tc.id, "vendor/model"), snapshot)
			want := logging.Fields{
				"area": "draft_model", "phase": "failed", "code": "not_configured", "provider": tc.id.String(),
				"cheaprouter_managed": tc.managed, "provider_present": tc.present, "provider_enabled": tc.enabled, "model_present": tc.model,
			}
			if tc.lane != "" {
				want["cheaprouter_lane"] = tc.lane
			}
			if !maps.Equal(record, want) {
				t.Errorf("warning fields = %v, want %v", record, want)
			}
		})
	}
}
