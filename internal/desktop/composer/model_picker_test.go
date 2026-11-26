package composer

import (
	"fmt"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

func newPickerTester(state *State, ctx *Context) *ui.Tester {
	tester := ui.NewTester(func(c *ui.Context) {
		theme.Apply(c, "", theme.Dark)
		ui.Column(c).Size(640, 400).Children(func() {
			if state.layer == layerModels {
				state.modelPicker(c, ctx)
			}
		})
	}, 640, 400)
	tester.SetLocale("en")
	return tester
}

func clickAccess(t *testing.T, tester *ui.Tester, info ui.AccessInfo) {
	t.Helper()
	tester.ClickAt(info.Bounds.X+info.Bounds.W/2, info.Bounds.Y+info.Bounds.H/2)
	tester.Frame()
}

func pickerFocus(tester *ui.Tester) (ui.Role, string) {
	for _, info := range tester.AccessAll() {
		if info.Focused {
			return info.Role, info.Name
		}
	}
	return ui.RoleNone, ""
}

// A model's row opens its efforts without choosing it; the effort's row
// chooses. A model without efforts is chosen from its row.
func TestModelPickerChoosesAtAClickedEffort(t *testing.T) {
	ctx := &Context{Models: []Model{
		{ID: "a", Name: "Alpha", Provider: "P", Reasoning: true, Efforts: []string{"low", "high"}, DefaultEffort: "low"},
		{ID: "b", Name: "Beta", Provider: "P", Reasoning: true, Efforts: []string{"low", "medium", "high"}, DefaultEffort: "low"},
		{ID: "c", Name: "Gamma", Provider: "P"},
	}}
	state := New("a")
	state.layer = layerModels
	tester := newPickerTester(state, ctx)
	tester.Frame()
	click := func(name string) {
		t.Helper()
		row, ok := tester.Access(name)
		if !ok {
			t.Fatalf("no %q row: %q", name, tester.Texts())
		}
		clickAccess(t, tester, row)
	}

	if tester.HasText("medium") {
		t.Fatal("the column shows another model's efforts before a click")
	}
	click("Beta")
	if state.Model != "a" || state.layer != layerModels {
		t.Fatalf("a model's row chose it: model %q, layer %v", state.Model, state.layer)
	}
	if !tester.HasText("medium") {
		t.Fatalf("the clicked model's efforts are missing: %q", tester.Texts())
	}
	// Another model's column starts at its first effort.
	if first, ok := tester.AccessByRole(ui.RoleOption, "low"); !ok || !first.Selected {
		t.Fatalf("the first effort is not checked: %+v", first)
	}
	click("medium")
	if state.Model != "b" || state.Effort != 1 || state.layer != layerMain {
		t.Fatalf("the effort chose model %q at %v, layer %v", state.Model, state.Effort, state.layer)
	}

	state.layer, state.efforts = layerModels, effortState{}
	tester.Frame()
	click("Gamma")
	if state.Model != "c" || state.layer != layerMain {
		t.Fatalf("a model without efforts was not chosen from its row: %q", state.Model)
	}
}

func TestModelPickerSingleProviderHasNoGroupHeaders(t *testing.T) {
	ctx := &Context{Models: []Model{
		{ID: "p1/claude-opus", Name: "Claude Opus", Provider: "Anthropic Direct", ProviderID: "p1", Family: "anthropic", Reasoning: true, Efforts: []string{"low", "high"}},
		{ID: "p1/claude-sonnet", Name: "Claude Sonnet", Provider: "Anthropic Direct", ProviderID: "p1", Family: "anthropic"},
	}}
	state := New("p1/claude-opus")
	state.layer, state.family, state.efforts = layerModels, "anthropic", effortState{enter: true}
	tester := newPickerTester(state, ctx)
	tester.Frame()

	for _, info := range tester.AccessAll() {
		if info.Role == ui.RoleDisclosure {
			t.Fatalf("single-provider view rendered disclosure header %+v", info)
		}
	}
	if _, ok := tester.AccessByRole(ui.RoleButton, "Claude Opus"); !ok {
		t.Fatalf("expected Claude Opus row: %q", tester.Texts())
	}
	if _, ok := tester.AccessByRole(ui.RoleButton, "Claude Sonnet"); !ok {
		t.Fatalf("expected Claude Sonnet row: %q", tester.Texts())
	}
}

func TestModelPickerMultiProviderDefaultExpandChosenAndMemory(t *testing.T) {
	ctx := &Context{
		Models: []Model{
			{ID: "bedrock/claude-sonnet", Name: "Bedrock Sonnet", Provider: "AWS Bedrock", ProviderID: "bedrock", ProviderIcon: icons.Brand("bedrock"), ProviderTag: "Bedrock", Family: "anthropic", ContextWindow: 200000},
			{ID: "bedrock/claude-haiku", Name: "Bedrock Haiku", Provider: "AWS Bedrock", ProviderID: "bedrock", ProviderIcon: icons.Brand("bedrock"), ProviderTag: "Bedrock", Family: "anthropic"},
			{ID: "direct/claude-opus", Name: "Direct Opus", Provider: "Anthropic Direct", ProviderID: "direct", ProviderIcon: icons.Brand("anthropic"), Family: "anthropic", ContextWindow: 200000, Reasoning: true, Efforts: []string{"low", "high"}},
			{ID: "cheap/claude-sonnet", Name: "Cheap Sonnet", Provider: "CheapRouter", ProviderID: "cheap", ProviderIcon: icons.SailryMark, Family: "anthropic"},
			{ID: "openai-a/gpt-5", Name: "GPT-5 Direct", Provider: "OpenAI Direct", ProviderID: "openai-a", Family: "openai"},
			{ID: "azure/gpt-5", Name: "GPT-5 Azure", Provider: "Azure East", ProviderID: "azure", ProviderTag: "Azure", Family: "openai"},
		},
		RecentModels: []string{"azure/gpt-5", "direct/claude-opus"},
	}
	state := New("direct/claude-opus")
	state.layer, state.family, state.efforts = layerModels, "anthropic", effortState{enter: true}
	tester := newPickerTester(state, ctx)
	tester.Frame()

	directHdr, ok := tester.AccessByRole(ui.RoleDisclosure, "Anthropic Direct")
	if !ok || !directHdr.Expanded {
		t.Fatalf("chosen provider group should be expanded by default: %+v", directHdr)
	}
	bedrockHdr, ok := tester.AccessByRole(ui.RoleDisclosure, "AWS Bedrock")
	if !ok || bedrockHdr.Expanded {
		t.Fatalf("other provider group should be collapsed by default: %+v", bedrockHdr)
	}
	cheapHdr, ok := tester.AccessByRole(ui.RoleDisclosure, "CheapRouter")
	if !ok || cheapHdr.Expanded {
		t.Fatalf("third provider group should be collapsed by default: %+v", cheapHdr)
	}
	if directHdr.Bounds.Y >= bedrockHdr.Bounds.Y {
		t.Fatalf("chosen group (%v) should sort above other groups (%v)", directHdr.Bounds, bedrockHdr.Bounds)
	}
	if !tester.HasText("Bedrock") {
		t.Fatalf("expected Bedrock tag in group header: %q", tester.Texts())
	}
	opusRow, ok := tester.AccessByRole(ui.RoleButton, "Direct Opus")
	if !ok {
		t.Fatalf("chosen model row missing: %q", tester.Texts())
	}
	if strings.Contains(opusRow.Description, "Anthropic Direct") {
		t.Fatalf("grouped model row repeated provider name in subtitle: %q", opusRow.Description)
	}
	if _, name := pickerFocus(tester); name != "Direct Opus" {
		t.Fatalf("expected focus on chosen model row, got %q", name)
	}
	if _, ok := tester.AccessByRole(ui.RoleButton, "Bedrock Sonnet"); ok {
		t.Fatalf("collapsed Bedrock model should be hidden")
	}

	// Expand Bedrock and collapse Direct.
	clickAccess(t, tester, bedrockHdr)
	if _, ok := tester.AccessByRole(ui.RoleButton, "Bedrock Sonnet"); !ok {
		t.Fatalf("clicking Bedrock header did not expand it: %q", tester.Texts())
	}
	directHdr, _ = tester.AccessByRole(ui.RoleDisclosure, "Anthropic Direct")
	clickAccess(t, tester, directHdr)
	if _, ok := tester.AccessByRole(ui.RoleButton, "Direct Opus"); ok {
		t.Fatalf("clicking Direct header did not collapse it")
	}

	// Reopening the picker always re-expands chosen group while keeping Bedrock expanded.
	state.efforts = effortState{enter: true}
	tester.Frame()
	if h, ok := tester.AccessByRole(ui.RoleDisclosure, "Anthropic Direct"); !ok || !h.Expanded {
		t.Fatalf("reopening picker did not re-expand chosen group: %+v", h)
	}
	if h, ok := tester.AccessByRole(ui.RoleDisclosure, "AWS Bedrock"); !ok || !h.Expanded {
		t.Fatalf("reopening picker lost manual expand memory for Bedrock: %+v", h)
	}
	if h, ok := tester.AccessByRole(ui.RoleDisclosure, "CheapRouter"); !ok || h.Expanded {
		t.Fatalf("CheapRouter should remain collapsed: %+v", h)
	}

	// Switching to OpenAI sorts the group with a recent model first and expands only it.
	openaiRail, ok := tester.AccessByRole(ui.RoleButton, "OpenAI")
	if !ok {
		t.Fatalf("missing OpenAI rail button: %q", tester.Texts())
	}
	clickAccess(t, tester, openaiRail)
	azureHdr, ok := tester.AccessByRole(ui.RoleDisclosure, "Azure East")
	if !ok || !azureHdr.Expanded {
		t.Fatalf("recent provider group should be expanded in another family: %+v", azureHdr)
	}
	openaiHdr, ok := tester.AccessByRole(ui.RoleDisclosure, "OpenAI Direct")
	if !ok || openaiHdr.Expanded {
		t.Fatalf("non-recent provider group should be collapsed in another family: %+v", openaiHdr)
	}
	if azureHdr.Bounds.Y >= openaiHdr.Bounds.Y {
		t.Fatalf("recent group (%v) should sort above non-recent group (%v)", azureHdr.Bounds, openaiHdr.Bounds)
	}

	// Recent view stays flat (no disclosure headers) and retains provider name in row subtitle.
	recentRail, ok := tester.AccessByRole(ui.RoleButton, i18n.In("en", "composer_models_recent"))
	if !ok {
		t.Fatalf("missing recent rail button: %q", tester.Texts())
	}
	clickAccess(t, tester, recentRail)
	for _, info := range tester.AccessAll() {
		if info.Role == ui.RoleDisclosure {
			t.Fatalf("#recent view should stay flat, found disclosure %+v", info)
		}
	}
	recentRow, ok := tester.AccessByRole(ui.RoleButton, "GPT-5 Azure")
	if !ok || !strings.Contains(recentRow.Description, "Azure East") {
		t.Fatalf("#recent row missing provider subtitle: %+v", recentRow)
	}
}

func TestModelPickerDisclosureClickIsolationAndCollapseFallback(t *testing.T) {
	ctx := &Context{Models: []Model{
		{ID: "direct/opus", Name: "Direct Opus", Provider: "Anthropic Direct", ProviderID: "direct", Family: "anthropic", Reasoning: true, Efforts: []string{"low", "high"}, DefaultEffort: "low"},
		{ID: "bedrock/sonnet", Name: "Bedrock Sonnet", Provider: "AWS Bedrock", ProviderID: "bedrock", Family: "anthropic", Reasoning: true, Efforts: []string{"low", "medium", "high"}, DefaultEffort: "low"},
	}}
	state := New("direct/opus")
	state.layer, state.family, state.efforts = layerModels, "anthropic", effortState{enter: true}
	tester := newPickerTester(state, ctx)
	tester.Frame()

	// Expand Bedrock and preview Bedrock Sonnet's efforts (which includes "medium").
	bedrockHdr, _ := tester.AccessByRole(ui.RoleDisclosure, "AWS Bedrock")
	clickAccess(t, tester, bedrockHdr)
	sonnetRow, _ := tester.AccessByRole(ui.RoleButton, "Bedrock Sonnet")
	clickAccess(t, tester, sonnetRow)
	if state.efforts.model != "bedrock/sonnet" || !tester.HasText("medium") {
		t.Fatalf("expected effort column previewing bedrock/sonnet: model=%q texts=%q", state.efforts.model, tester.Texts())
	}

	// Clicking another group header (Direct) must NOT change efforts.model or steal the effort column.
	directHdr, _ := tester.AccessByRole(ui.RoleDisclosure, "Anthropic Direct")
	clickAccess(t, tester, directHdr)
	if state.efforts.model != "bedrock/sonnet" || !tester.HasText("medium") {
		t.Fatalf("clicking unrelated disclosure header changed efforts.model=%q texts=%q", state.efforts.model, tester.Texts())
	}

	// Re-expand Direct, then collapse Bedrock (the group currently being previewed): falls back to chosen.
	directHdr, _ = tester.AccessByRole(ui.RoleDisclosure, "Anthropic Direct")
	clickAccess(t, tester, directHdr)
	bedrockHdr, _ = tester.AccessByRole(ui.RoleDisclosure, "AWS Bedrock")
	clickAccess(t, tester, bedrockHdr)
	if state.efforts.model != "direct/opus" || tester.HasText("medium") || !tester.HasText("high") {
		t.Fatalf("collapsing previewed group did not fall back to chosen: model=%q texts=%q", state.efforts.model, tester.Texts())
	}

	// Collapsing Direct as well hides chosen -> effort column hides.
	directHdr, _ = tester.AccessByRole(ui.RoleDisclosure, "Anthropic Direct")
	clickAccess(t, tester, directHdr)
	if _, ok := tester.AccessByRole(ui.RoleOption, "low"); ok {
		t.Fatalf("effort column should hide when chosen group is also collapsed: %q", tester.Texts())
	}
}

func TestModelPickerSearchExpandsAllGroupsAndCapsOver50(t *testing.T) {
	models := []Model{
		{ID: "bedrock/claude-4", Name: "Claude 4", Provider: "AWS Bedrock", ProviderID: "bedrock", ProviderTag: "Bedrock", Family: "anthropic"},
		{ID: "acp/agent-1", Name: "Coder Agent", Provider: "My Plugin", ProviderID: "acp", ProviderTag: "ACP", Family: "other"},
		{ID: "ollama/llama3", Name: "Llama 3", Provider: "Ollama", ProviderID: "ollama", ProviderTag: "Local", Family: "other"},
	}
	for i := range 30 {
		models = append(models,
			Model{ID: fmt.Sprintf("p1/model-%02d", i), Name: fmt.Sprintf("Alpha Model %02d", i), Provider: "Provider One", ProviderID: "p1", Family: "openai"},
			Model{ID: fmt.Sprintf("p2/model-%02d", i), Name: fmt.Sprintf("Beta Model %02d", i), Provider: "Provider Two", ProviderID: "p2", Family: "deepseek"},
		)
	}
	ctx := &Context{Models: models}
	state := New("bedrock/claude-4")
	state.layer, state.family, state.efforts = layerModels, "anthropic", effortState{enter: true}
	tester := newPickerTester(state, ctx)
	tester.Frame()

	// Tag search: acp / local / bedrock
	state.modelQuery = "acp"
	tester.Frame()
	if _, ok := tester.AccessByRole(ui.RoleButton, "Coder Agent"); !ok {
		t.Fatalf("searching 'acp' did not match ACP model: %q", tester.Texts())
	}
	state.modelQuery = "local"
	tester.Frame()
	if _, ok := tester.AccessByRole(ui.RoleButton, "Llama 3"); !ok {
		t.Fatalf("searching 'local' did not match Local model: %q", tester.Texts())
	}

	// Cross-family, cross-provider search matching 60 models: forces all matching groups open, caps at 50, shows footer hint.
	state.modelQuery = "model"
	tester.Frame()
	p1Hdr, ok1 := tester.AccessByRole(ui.RoleDisclosure, "Provider One")
	p2Hdr, ok2 := tester.AccessByRole(ui.RoleDisclosure, "Provider Two")
	if !ok1 || !p1Hdr.Expanded || !ok2 || !p2Hdr.Expanded {
		t.Fatalf("search must keep group headers and expand all matches: p1=%+v p2=%+v", p1Hdr, p2Hdr)
	}
	modelCount := 0
	for _, info := range tester.AccessAll() {
		if info.Role == ui.RoleButton && (strings.HasPrefix(info.Name, "Alpha Model ") || strings.HasPrefix(info.Name, "Beta Model ")) {
			modelCount++
		}
	}
	if modelCount != maxSearchResults {
		t.Fatalf("rendered %d search models, want cap %d", modelCount, maxSearchResults)
	}
	if !tester.HasText(i18n.In("en", "composer_model_search_more")) {
		t.Fatalf("missing >50 search narrow hint: %q", tester.Texts())
	}

	// Empty search shows composer_model_empty.
	state.modelQuery = "definitely-no-such-model"
	tester.Frame()
	if !tester.HasText(i18n.In("en", "composer_model_empty")) {
		t.Fatalf("missing empty state message: %q", tester.Texts())
	}
}

func TestModelPickerKeyboardAcrossGroups(t *testing.T) {
	ctx := &Context{Models: []Model{
		{ID: "direct/opus", Name: "Direct Opus", Provider: "Anthropic Direct", ProviderID: "direct", Family: "anthropic", Reasoning: true, Efforts: []string{"low", "high"}, DefaultEffort: "low"},
		{ID: "bedrock/sonnet", Name: "Bedrock Sonnet", Provider: "AWS Bedrock", ProviderID: "bedrock", ProviderTag: "Bedrock", Family: "anthropic", Reasoning: true, Efforts: []string{"low", "medium", "high"}, DefaultEffort: "low"},
	}}
	state := New("direct/opus")
	state.layer, state.family, state.efforts = layerModels, "anthropic", effortState{enter: true}
	tester := newPickerTester(state, ctx)
	tester.Frame()

	expectFocus := func(wantRole ui.Role, wantName string) {
		t.Helper()
		r, n := pickerFocus(tester)
		if r != wantRole || n != wantName {
			t.Fatalf("focus is (%v, %q), want (%v, %q)", r, n, wantRole, wantName)
		}
	}
	expectFocus(ui.RoleButton, "Direct Opus")

	// Down moves onto Bedrock disclosure header (collapsed).
	tester.Key(0, ui.KeyDown)
	tester.Frame()
	expectFocus(ui.RoleDisclosure, "AWS Bedrock")

	// Space on disclosure header expands it without picking or entering effort column.
	tester.Key(0, ui.KeySpace)
	tester.Frame()
	if h, ok := tester.AccessByRole(ui.RoleDisclosure, "AWS Bedrock"); !ok || !h.Expanded {
		t.Fatalf("Space on disclosure header did not expand group: %+v", h)
	}
	if state.Model != "direct/opus" || state.layer != layerModels {
		t.Fatalf("Space on disclosure header picked a model: %q", state.Model)
	}

	// Down moves into the expanded Bedrock model row and updates efforts to include "medium".
	tester.Key(0, ui.KeyDown)
	tester.Frame()
	expectFocus(ui.RoleButton, "Bedrock Sonnet")
	if !tester.HasText("medium") {
		t.Fatalf("moving focus to Bedrock Sonnet did not update effort column: %q", tester.Texts())
	}

	// Right enters effort column, Left returns to model row.
	tester.Key(0, ui.KeyRight)
	tester.Frame()
	expectFocus(ui.RoleOption, "low")
	tester.Key(0, ui.KeyLeft)
	tester.Frame()
	expectFocus(ui.RoleButton, "Bedrock Sonnet")

	// Tab enters effort column, Shift+Tab returns to model row.
	tester.Key(0, ui.KeyTab)
	tester.Frame()
	expectFocus(ui.RoleOption, "low")
	tester.Key(ui.Shift, ui.KeyTab)
	tester.Frame()
	expectFocus(ui.RoleButton, "Bedrock Sonnet")

	// Up back to Bedrock disclosure header, Enter collapses it again.
	tester.Key(0, ui.KeyUp)
	tester.Frame()
	expectFocus(ui.RoleDisclosure, "AWS Bedrock")
	tester.Key(0, ui.KeyEnter)
	tester.Frame()
	if h, ok := tester.AccessByRole(ui.RoleDisclosure, "AWS Bedrock"); !ok || h.Expanded {
		t.Fatalf("Enter on disclosure header did not collapse group: %+v", h)
	}
}

func TestModelPickerUnavailableChosenModelStaysVisibleAndPinned(t *testing.T) {
	ctx := &Context{Models: []Model{
		{ID: "direct/sonnet", Name: "Direct Sonnet", Provider: "Anthropic Direct", ProviderID: "direct", Family: "anthropic"},
		{ID: "broken/opus", Name: "Broken Opus", Provider: "Disabled Channel", ProviderID: "broken", Family: "anthropic", Unavailable: true},
	}}
	state := New("broken/opus")
	state.layer, state.family, state.efforts = layerModels, "anthropic", effortState{enter: true}
	tester := newPickerTester(state, ctx)
	tester.Frame()

	brokenHdr, ok := tester.AccessByRole(ui.RoleDisclosure, "Disabled Channel")
	if !ok || !brokenHdr.Expanded {
		t.Fatalf("unavailable chosen model's group should be expanded: %+v", brokenHdr)
	}
	directHdr, ok := tester.AccessByRole(ui.RoleDisclosure, "Anthropic Direct")
	if !ok || directHdr.Expanded {
		t.Fatalf("other group should remain collapsed: %+v", directHdr)
	}
	if brokenHdr.Bounds.Y >= directHdr.Bounds.Y {
		t.Fatalf("unavailable chosen group (%v) should be pinned at top above %v", brokenHdr.Bounds, directHdr.Bounds)
	}
	row, ok := tester.AccessByRole(ui.RoleButton, "Broken Opus")
	if !ok || !row.Checked {
		t.Fatalf("unavailable chosen model row should stay visible and highlighted: %+v", row)
	}
	if !tester.HasText(i18n.In("en", "unavailable")) || !strings.Contains(row.Description, i18n.In("en", "unavailable")) {
		t.Fatalf("unavailable badge missing on row: desc=%q texts=%q", row.Description, tester.Texts())
	}
}
