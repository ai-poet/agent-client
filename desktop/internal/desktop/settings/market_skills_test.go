package settings

import (
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/preview"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func TestSkillMarketShowsResultsAlongsideSourceFailure(t *testing.T) {
	m := NewModel(preview.SettingsData())
	m.Live, m.MarketKind = true, 1
	m.MarketSkills = nil
	m.SkillMarket.Loading = true
	s := &State{Model: m}
	more, retry := 0, 0
	m.PackageActions = &PackageActions{
		MoreSkills:  func() { more++ },
		RetryMarket: func() { retry++ },
	}
	tt := ui.NewTester(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		MarketPage(c, s)
	}, 900, 600)
	tt.SetLocale("en")
	tt.Frame()
	empty := i18n.In("en", "skills_no_matches")
	if tt.HasText(empty) {
		t.Fatal("the first request is still loading, not empty")
	}
	m.SkillMarket = SkillMarketState{}
	tt.Frame()
	if !tt.HasText(empty) {
		t.Fatal("a successful search with no matches needs an empty state")
	}

	m.MarketQuery = "repository name"
	m.MarketSkills = []preview.Skill{{ID: "result", Name: "Server match"}}
	m.SkillMarket = SkillMarketState{
		More:   true,
		Issues: []SkillMarketIssue{{Source: protocol.PluginSkillcatalogSourceSkillsSh, Reason: "market_skill_catalog_invalid_response"}},
	}
	tt.Frame()
	if cause := "skills.sh: " + i18n.In("en", "market_skill_catalog_invalid_response"); !tt.HasText(cause) || !tt.HasText("Server match") || tt.HasText(empty) {
		t.Fatalf("want result and specific source failure, got %q", tt.Texts())
	}
	for _, key := range []string{"market_load_more", "settings_retry"} {
		if err := tt.Click(i18n.In("en", key)); err != nil {
			t.Fatal(err)
		}
	}
	if more != 1 || retry != 1 {
		t.Fatalf("load more dispatched %d times, retry %d times", more, retry)
	}
}
