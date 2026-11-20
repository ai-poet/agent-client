package live

import (
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

func TestSkillMarketKeepsOtherPagesAndRetriesFailedCursor(t *testing.T) {
	official := skillMarketCursor{next: 1}
	directory := skillMarketCursor{next: 1}
	next := uint32(2)
	directory.apply(skillMarketPage(t.Context(), protocol.Output{Kind: "skill_catalog", Data: protocol.PluginSkillcatalogPage{
		Page: 1, Next: &next, Entries: []protocol.PluginSkillcatalogEntry{{ID: "team/tools/review"}},
	}}, nil, 1))
	official.apply(skillMarketPage(t.Context(), protocol.Output{Kind: "skill_catalog", Data: protocol.PluginSkillcatalogPage{
		Page: 1, Unavailable: true, Entries: []protocol.PluginSkillcatalogEntry{{ID: "bundled/review"}},
	}}, nil, 1))
	if directory.next != 2 || len(directory.entries) != 1 || directory.issue != "" {
		t.Fatalf("healthy source lost its results or next page: %#v", directory)
	}
	if official.next != 1 || len(official.fallback) != 1 || official.issue != "unavailable" {
		t.Fatalf("unavailable source lost its fallback or retry page: %#v", official)
	}
	official.apply(skillMarketPage(t.Context(), protocol.Output{Kind: "skill_catalog", Data: protocol.PluginSkillcatalogPage{
		Page: 1, Entries: []protocol.PluginSkillcatalogEntry{{ID: "official/review"}},
	}}, nil, 1))
	if official.next != 0 || len(official.entries) != 1 || len(official.fallback) != 0 || official.issue != "" {
		t.Fatalf("recovered source did not replace its fallback: %#v", official)
	}
	badNext := uint32(2)
	directory.apply(skillMarketPage(t.Context(), protocol.Output{Kind: "skill_catalog", Data: protocol.PluginSkillcatalogPage{
		Page: 2, Next: &badNext,
	}}, nil, 2))
	if directory.next != 2 || len(directory.entries) != 1 || directory.issue != "market_skill_catalog_invalid_response" {
		t.Fatalf("invalid next page changed loaded results or retry page: %#v", directory)
	}
}
