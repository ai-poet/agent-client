package model

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/node/providers"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
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
		if body["model"] != "gpt-cheap-test" || body["stream"] != true {
			t.Errorf("模型请求体 = %#v", body)
		}
		writer.Header().Set("Content-Type", "text/event-stream")
		_, _ = fmt.Fprint(writer, "data: {\"id\":\"reply\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"网关已连接\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\ndata: [DONE]\n\n")
	}))
	defer server.Close()

	provider, secret, err := providers.CheapRouterProvider(cheapauth.Gateway{
		BaseURL: server.URL + "/v1", APIKey: apiKey,
		Models: []cheapauth.Model{{ID: "gpt-cheap-test", DisplayName: "Cheap Test", ContextWindow: 64000}},
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
	response, err := languageModel.Generate(t.Context(), requestCall(t, "chat"))
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
