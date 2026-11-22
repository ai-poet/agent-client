package model

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestCheapRouterProviderUsesInjectedGatewayAndAPIKey(t *testing.T) {
	const apiKey = "sk-cheaprouter-test-secret"
	requests := 0
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		requests++
		if request.URL.Path != "/v1/chat/completions" {
			t.Errorf("模型请求路径 = %q", request.URL.Path)
		}
		if got := request.Header.Get("Authorization"); got != "Bearer "+apiKey {
			t.Errorf("Authorization = %q", got)
		}
		var body map[string]any
		if err := json.NewDecoder(request.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		output := body["max_tokens"]
		if output == nil {
			output = body["max_completion_tokens"]
		}
		if body["model"] != "gpt-cheap-test" || body["stream"] != true || output != float64(32768) {
			t.Errorf("模型请求体 = %#v", body)
		}
		writer.Header().Set("Content-Type", "text/event-stream")
		_, _ = fmt.Fprint(writer, "data: {\"id\":\"reply\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"网关已连接\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\ndata: [DONE]\n\n")
	}))
	defer server.Close()

	provider, secret, err := providers.CheapRouterProvider(cheapauth.Gateway{
		BaseURL: server.URL + "/v1", APIKey: apiKey,
		Models: []cheapauth.Model{{ID: "gpt-cheap-test", DisplayName: "Cheap Test", ContextWindow: 200000, MaxOutputTokens: 32768}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if provider.Endpoint != server.URL+"/v1" || provider.ID != providers.CheapRouterProviderID || secret == nil || secret.Expose() != apiKey {
		t.Fatal("CheapRouter provider 未读取注入的 base_url/api_key")
	}
	config := protocol.DefaultSessionConfig()
	config.Provider, config.Model = provider.ID, provider.DefaultModel
	config.Mode, config.Permission = protocol.WorkModeCode, protocol.PermissionAsk
	binding, err := Capture(provider, config, secret, protocol.NewSessionID())
	if err != nil {
		t.Fatal(err)
	}
	languageModel, err := New(t.Context(), binding)
	if err != nil {
		t.Fatal(err)
	}
	if selected, estimated := ResolvedModel(binding); estimated || selected.Context != 200000 {
		t.Fatalf("gateway window = %d, estimated = %v", selected.Context, estimated)
	}
	message := canonical.NewRecord(protocol.NewTurnID(), "message", canonical.NewMessage(canonical.User, canonical.Text(0, "Hello")), time.Unix(0, 0))
	call, err := BuildCall(binding, Request{Records: []canonical.Record{message}})
	if err != nil {
		t.Fatal(err)
	}
	response, err := languageModel.Generate(t.Context(), call)
	if err != nil {
		t.Fatal(err)
	}
	if requests != 1 {
		t.Fatalf("假网关收到 %d 次请求", requests)
	}
	if len(response.Content) == 0 {
		t.Fatal("假网关响应未被模型链路读取")
	}
	if fmt.Sprint(response.Content[0]) == "" || !strings.Contains(fmt.Sprint(response.Content), "网关已连接") {
		t.Fatalf("模型响应 = %#v", response.Content)
	}
}

func TestCheapRouterBoundLaneRequests(t *testing.T) {
	cases := []struct {
		lane, platform, model, kind, path, header, field string
		api                                              protocol.ConversationModelApi
	}{
		{cheapauth.LaneClaude, "anthropic", "claude-test", "anthropic", "/v1/messages", "x-api-key", "messages", protocol.ConversationModelApiAnthropic},
		{cheapauth.LaneCodex, "openai", "gpt-test", "responses", "/v1/responses", "Authorization", "input", protocol.ConversationModelApiResponses},
		{cheapauth.LaneCodex, "gpt", "gpt-alias", "responses", "/v1/responses", "Authorization", "input", protocol.ConversationModelApiResponses},
		{cheapauth.LaneGemini, "gemini", "gemini-test", "google", "/v1beta/models/gemini-test:streamGenerateContent?alt=sse", "x-goog-api-key", "contents", protocol.ConversationModelApiGemini},
		{cheapauth.LaneClaude, "google", "claude-sonnet-4-6", "google", "/v1beta/models/claude-sonnet-4-6:streamGenerateContent?alt=sse", "x-goog-api-key", "contents", protocol.ConversationModelApiGemini},
		{cheapauth.LaneDomestic, "openai", "deepseek-test", "chat", "/v1/chat/completions", "Authorization", "messages", protocol.ConversationModelApiChatCompletions},
		{cheapauth.LaneGrok, "grok", "grok-test", "chat", "/v1/chat/completions", "Authorization", "messages", protocol.ConversationModelApiChatCompletions},
		{"future", "unknown", "future-model", "chat", "/v1/chat/completions", "Authorization", "messages", protocol.ConversationModelApiChatCompletions},
		// A Claude lane bound to a composite group does not speak Anthropic.
		{cheapauth.LaneClaude, "composite", "claude-alias", "chat", "/v1/chat/completions", "Authorization", "messages", protocol.ConversationModelApiChatCompletions},
	}
	for _, tt := range cases {
		server := serverFixture(t, string(fixture(t, tt.kind+"-text.sse")))
		gateway := cheapauth.Gateway{BaseURL: server.URL + "/prefix/v1/", LaneKeys: map[string]string{tt.lane: tt.lane + "-key"}, Lanes: []cheapauth.GatewayLane{{
			Lane: tt.lane, Group: cheapauth.Group{ID: 1, Platform: tt.platform, SupportsResponses: tt.lane == cheapauth.LaneCodex}, Models: []cheapauth.Model{{ID: tt.model, ContextWindow: 128000}},
		}}}
		routes, err := providers.CheapRouterProviders(gateway)
		if err != nil || len(routes) != 1 {
			t.Fatalf("routes=%d err=%v", len(routes), err)
		}
		provider, secret := routes[0].Provider, routes[0].Secret
		config := protocol.DefaultSessionConfig()
		config.Provider, config.Model = provider.ID, tt.model
		config.Mode, config.Permission = protocol.WorkModeCode, protocol.PermissionAsk
		binding, err := Capture(provider, config, secret, protocol.NewSessionID())
		if err != nil {
			t.Fatal(err)
		}
		if got := Scope(binding).API; got != tt.api {
			t.Fatalf("%s: projection API = %s, want %s", tt.model, got, tt.api)
		}
		lm, err := New(t.Context(), binding)
		if err != nil {
			t.Fatal(err)
		}
		message := canonical.NewRecord(protocol.NewTurnID(), "message", canonical.NewMessage(canonical.User, canonical.Text(0, "Hello")), time.Time{})
		call, err := BuildCall(binding, Request{Records: []canonical.Record{message}})
		if err != nil {
			t.Fatal(err)
		}
		if _, err := lm.Generate(t.Context(), call); err != nil {
			t.Fatalf("%s: %v", tt.model, err)
		}
		requests := server.captured()
		if len(requests) != 1 {
			t.Fatalf("%s: got %d total requests, want %d", tt.model, len(requests), 1)
		}
		req := requests[0]
		wantKey := gateway.LaneKeys[tt.lane]
		if tt.header == "Authorization" {
			wantKey = "Bearer " + wantKey
		}
		if req.Path != "/prefix"+tt.path || req.Headers.Get(tt.header) != wantKey || req.Body[tt.field] == nil {
			t.Errorf("%s: incorrect path (%s), credential, or missing %s payload", tt.model, req.Path, tt.field)
		}
	}
}
