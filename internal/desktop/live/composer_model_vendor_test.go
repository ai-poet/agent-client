package live

import (
	"slices"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestComposerModelVendor(t *testing.T) {
	gateway := protocol.ConversationProvider{ID: providers.CheapRouterProviderID, Api: protocol.ConversationModelApiChatCompletions}
	for _, tc := range []struct{ platform, model, want string }{
		{"anthropic", "deployment-a", "anthropic"},
		{"google", "deployment-b", "gemini"},
		{"openai", "deepseek-v4.1-flash", "deepseek"},
		{"openai", "glm-5", "zhipu"},
		{"openai", "kimi-k3", "kimi"},
		{"openai", "qwen3-coder", "qwen"},
		{"openai", "gpt-5", "openai"},
		{"composite", "owner/deepseek-v4", "deepseek"},
		{"", "claude-sonnet-4-6", "anthropic"},
		{"", "unrecognized-alias", "other"},
	} {
		if got := composerModelVendor(gateway, protocol.ConversationModel{ID: tc.model, Platform: tc.platform}); got != tc.want {
			t.Errorf("%q model %q vendor = %q, want %q", tc.platform, tc.model, got, tc.want)
		}
	}

	// Non-OpenAI compatible/local channels should not pollute the openai bucket with unknown models.
	ollama := protocol.ConversationProvider{
		ID:       protocol.ProviderID{1},
		Api:      protocol.ConversationModelApiChatCompletions,
		Endpoint: "http://127.0.0.1:11434/v1",
	}
	if got := composerModelVendor(ollama, protocol.ConversationModel{ID: "llama-3.3-70b"}); got != "other" {
		t.Errorf("ollama unknown model vendor = %q, want other", got)
	}
	if got := composerModelVendor(ollama, protocol.ConversationModel{ID: "qwen3-coder"}); got != "qwen" {
		t.Errorf("ollama qwen model vendor = %q, want qwen", got)
	}
	openrouter := protocol.ConversationProvider{
		ID:       protocol.ProviderID{2},
		Api:      protocol.ConversationModelApiResponses,
		Preset:   &protocol.ConversationPresetsPresetRef{ID: "openrouter"},
		Endpoint: "https://openrouter.ai/api/v1",
	}
	if got := composerModelVendor(openrouter, protocol.ConversationModel{ID: "mistral-large-latest"}); got != "other" {
		t.Errorf("openrouter unknown model vendor = %q, want other", got)
	}

	// Official OpenAI channels retain openai fallback for custom/fine-tuned names.
	official := protocol.ConversationProvider{
		ID:       protocol.ProviderID{3},
		Api:      protocol.ConversationModelApiResponses,
		Endpoint: "https://api.openai.com/v1",
	}
	if got := composerModelVendor(official, protocol.ConversationModel{ID: "ft:custom-assistant"}); got != "openai" {
		t.Errorf("official openai custom model vendor = %q, want openai", got)
	}
	chatgpt := protocol.ConversationProvider{
		ID:             protocol.ProviderID{4},
		Api:            protocol.ConversationModelApiResponses,
		Authentication: protocol.AuthenticationChatGptOAuth,
	}
	if got := composerModelVendor(chatgpt, protocol.ConversationModel{ID: "custom-codex"}); got != "openai" {
		t.Errorf("chatgpt oauth custom model vendor = %q, want openai", got)
	}

	// Special sources: Bedrock, Vertex AI, Azure, Plugin/ACP.
	bedrock := protocol.ConversationProvider{ID: protocol.ProviderID{5}, Api: protocol.ConversationModelApiBedrock}
	if got := composerModelVendor(bedrock, protocol.ConversationModel{ID: "anthropic.claude-sonnet-4-6-v1:0"}); got != "anthropic" {
		t.Errorf("bedrock vendor = %q, want anthropic", got)
	}
	if providerTag(bedrock) != "Bedrock" || providerIcon(bedrock) == nil {
		t.Errorf("bedrock tag=%q icon=%v, want Bedrock + brand icon", providerTag(bedrock), providerIcon(bedrock))
	}
	vertex := protocol.ConversationProvider{ID: protocol.ProviderID{6}, Api: protocol.ConversationModelApiVertexAi}
	if got := composerModelVendor(vertex, protocol.ConversationModel{ID: "gemini-2.5-pro"}); got != "gemini" {
		t.Errorf("vertex vendor = %q, want gemini", got)
	}
	if providerTag(vertex) != "Vertex" || providerIcon(vertex) == nil {
		t.Errorf("vertex tag=%q icon=%v, want Vertex + brand icon", providerTag(vertex), providerIcon(vertex))
	}
	acp := protocol.ConversationProvider{ID: protocol.ProviderID{7}, Api: protocol.ConversationModelApiPlugin}
	if got := composerModelVendor(acp, protocol.ConversationModel{ID: "custom-agent"}); got != "other" {
		t.Errorf("acp vendor = %q, want other", got)
	}
	if providerTag(acp) != "ACP" || providerIcon(acp) != icons.Plug {
		t.Errorf("acp tag=%q icon=%v, want ACP + icons.Plug", providerTag(acp), providerIcon(acp))
	}
	if providerTag(ollama) != "Local" {
		t.Errorf("ollama tag=%q, want Local", providerTag(ollama))
	}
}

func TestPureMediaModelFiltering(t *testing.T) {
	cases := []struct {
		name string
		m    protocol.ConversationModel
		want bool
	}{
		{"plain chat", protocol.ConversationModel{ID: "claude-sonnet-4-6", Tools: true}, false},
		{"chat with image tool", protocol.ConversationModel{ID: "gpt-5.2", Tools: true, Reasoning: true, Generates: []protocol.MediaGeneration{protocol.MediaGenerationImageTool}}, false},
		{"pure image no tools", protocol.ConversationModel{ID: "gpt-image-2", Generates: []protocol.MediaGeneration{protocol.MediaGenerationImage}}, true},
		{"dedicated image with default tools", protocol.ConversationModel{ID: "dall-e-3", Tools: true, Generates: []protocol.MediaGeneration{protocol.MediaGenerationImage}}, true},
		{"pure video", protocol.ConversationModel{ID: "veo-2", Tools: true, Generates: []protocol.MediaGeneration{protocol.MediaGenerationVideo}}, true},
		{"pure speech", protocol.ConversationModel{ID: "tts-1", Tools: true, Generates: []protocol.MediaGeneration{protocol.MediaGenerationSpeech}}, true},
		{"multimodal reasoning with image", protocol.ConversationModel{ID: "gemini-2.5-flash", Tools: true, Reasoning: true, Generates: []protocol.MediaGeneration{protocol.MediaGenerationImage}}, false},
	}
	for _, tc := range cases {
		if got := isPureMediaModel(tc.m); got != tc.want {
			t.Errorf("%s: isPureMediaModel(%+v) = %v, want %v", tc.name, tc.m, got, tc.want)
		}
	}
}

func TestProjectModelsReadinessAndUnavailableSelectedModel(t *testing.T) {
	node := protocol.NodeID{1}
	cred := func(b byte) *protocol.CredentialRef {
		return &protocol.CredentialRef{Node: node, ID: protocol.CredentialID{b}}
	}
	readyKeyed := protocol.ConversationProvider{
		ID: protocol.ProviderID{1}, Name: "Bedrock Prod", Api: protocol.ConversationModelApiBedrock,
		Authentication: protocol.AuthenticationApiKey, Enabled: true, Credential: cred(1),
		Models: []protocol.ConversationModel{
			{ID: "claude-sonnet-4-6", Tools: true},
			{ID: "gpt-image-2", Generates: []protocol.MediaGeneration{protocol.MediaGenerationImage}},
		},
	}
	optionalKey := protocol.ConversationProvider{
		ID: protocol.ProviderID{2}, Name: "Local Ollama", Api: protocol.ConversationModelApiChatCompletions,
		Authentication: protocol.AuthenticationApiKey, Endpoint: "http://127.0.0.1:11434/v1", Enabled: true,
		Models: []protocol.ConversationModel{{ID: "llama-3.3-70b", Tools: true}},
	}
	missingKey := protocol.ConversationProvider{
		ID: protocol.ProviderID{3}, Name: "OpenAI Missing Key", Api: protocol.ConversationModelApiResponses,
		Preset: &protocol.ConversationPresetsPresetRef{Source: protocol.ConversationPresetsSource{Kind: "builtin"}, ID: "openai"},
		Authentication: protocol.AuthenticationApiKey, Enabled: true,
		Models: []protocol.ConversationModel{{ID: "gpt-5", Tools: true}},
	}
	signedOutOAuth := protocol.ConversationProvider{
		ID: protocol.ProviderID{4}, Name: "Claude OAuth", Api: protocol.ConversationModelApiAnthropic,
		Authentication: protocol.AuthenticationClaudeOAuth, Enabled: true,
		Models: []protocol.ConversationModel{{ID: "claude-opus-4-6", Tools: true}},
	}
	disabledChannel := protocol.ConversationProvider{
		ID: protocol.ProviderID{5}, Name: "Disabled Direct", Api: protocol.ConversationModelApiAnthropic,
		Authentication: protocol.AuthenticationApiKey, Enabled: false, Credential: cred(5),
		Models: []protocol.ConversationModel{{ID: "claude-old", Tools: true}},
	}
	authNoneProvider := protocol.ConversationProvider{
		ID: protocol.ProviderID{6}, Name: "AuthNone Local", Api: protocol.ConversationModelApiPlugin,
		Preset:         &protocol.ConversationPresetsPresetRef{Source: protocol.ConversationPresetsSource{Kind: "builtin"}, ID: "local-acp"},
		Authentication: protocol.AuthenticationApiKey, Enabled: true,
		Models: []protocol.ConversationModel{{ID: "acp-agent", Tools: true}},
	}
	snapshot := protocol.DefaultSnapshot()
	snapshot.Node = node
	snapshot.Providers = []protocol.ConversationProvider{readyKeyed, optionalKey, missingKey, signedOutOAuth, disabledChannel, authNoneProvider}
	transport := &conversationTestTransport{snapshot: snapshot}
	r := providerReadFixture(t, transport)
	r.App.Data = &viewmodel.Data{}
	noneEntry := protocol.DefaultConversationPresetsEntry()
	noneEntry.Source.Kind = "builtin"
	noneEntry.Preset.ID = "local-acp"
	noneEntry.Preset.Auth.Kind = "none"
	r.providerPresets = map[string]protocol.ConversationPresetsEntry{"builtin:local-acp/0": noneEntry}
	r.projectModels()

	var ids []string
	for _, m := range r.nodeModels {
		ids = append(ids, m.ID)
	}
	wantIDs := []string{
		modelKey(readyKeyed.ID, "claude-sonnet-4-6"),
		modelKey(optionalKey.ID, "llama-3.3-70b"),
		modelKey(authNoneProvider.ID, "acp-agent"),
	}
	if !slices.Equal(ids, wantIDs) {
		t.Fatalf("projected model IDs = %v, want %v", ids, wantIDs)
	}
	if r.nodeModels[0].ProviderID != readyKeyed.ID.String() || r.nodeModels[0].ProviderTag != "Bedrock" || r.nodeModels[0].ProviderIcon == nil {
		t.Fatalf("bedrock metadata not projected: %+v", r.nodeModels[0])
	}
	if r.nodeModels[1].ProviderTag != "Local" || r.nodeModels[1].Family != "other" {
		t.Fatalf("local ollama metadata not projected: %+v", r.nodeModels[1])
	}
	if r.nodeModels[2].ProviderTag != "ACP" || r.nodeModels[2].ProviderIcon != icons.Plug {
		t.Fatalf("auth-none ACP metadata not projected: %+v", r.nodeModels[2])
	}

	// Currently selected model from a disabled or signed-out provider stays resolvable with Unavailable=true.
	for _, tc := range []struct {
		name string
		id   string
		want string
	}{
		{"disabled provider", modelKey(disabledChannel.ID, "claude-old"), "Disabled Direct"},
		{"signed-out oauth", modelKey(signedOutOAuth.ID, "claude-opus-4-6"), "Claude OAuth"},
	} {
		got, ok := r.unavailableSelectedModel("conv-1", tc.id)
		if !ok || !got.Unavailable || got.ID != tc.id || got.Provider != tc.want {
			t.Fatalf("%s: unavailableSelectedModel = (%+v, %v)", tc.name, got, ok)
		}
	}
}
