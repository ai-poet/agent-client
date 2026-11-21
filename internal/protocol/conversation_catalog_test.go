package protocol

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestModelCatalogBundledStatus(t *testing.T) {
	status := DefaultConversationCatalogStatus()
	status.Bundled, status.Providers, status.Models = true, 2, 3
	event := Event{Kind: "model_catalog_changed", Data: status}
	raw, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	var decoded Event
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatal(err)
	}
	actual, ok := decoded.Data.(*ConversationCatalogStatus)
	if !ok || !actual.Bundled || actual.UpdatedAtMs != nil || actual.Models != 3 {
		t.Fatalf("bundled catalog status lost its source: %+v", decoded)
	}
	if err := json.Unmarshal([]byte(`{"revision":1,"providers":2,"models":3,"bytes":4}`), &status); err != nil {
		t.Fatal(err)
	}
	if status.Bundled {
		t.Fatal("an older cached status retained the bundled source")
	}
	raw, err = json.Marshal(status)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), `"bundled"`) {
		t.Fatal("a downloaded catalog added an unnecessary bundled field")
	}
}

func TestConversationCatalogQueryChannelWireCompatibility(t *testing.T) {
	legacy := DefaultConversationCatalogQuery()
	legacy.Provider = new("openai")
	legacy.IDs = []string{"gpt-5"}
	legacy.Limit = 10
	raw, err := json.Marshal(legacy)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), `"channel"`) {
		t.Fatalf("legacy query unexpectedly serialized channel: %s", raw)
	}
	var decoded ConversationCatalogQuery
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatal(err)
	}
	if decoded.Channel != nil || decoded.Provider == nil || *decoded.Provider != "openai" {
		t.Fatalf("legacy query decode mismatch: %+v", decoded)
	}

	channelQuery := DefaultConversationCatalogQuery()
	channelQuery.Channel = &ConversationCatalogChannel{Endpoint: "https://gateway.example/v1", Authentication: AuthenticationApiKey}
	channelQuery.IDs = []string{"fixture-vision"}
	channelQuery.Limit = 100
	cmd := Command{Kind: "read_model_catalog", Data: channelQuery}
	raw, err = json.Marshal(cmd)
	if err != nil {
		t.Fatal(err)
	}
	var decodedCmd Command
	if err := json.Unmarshal(raw, &decodedCmd); err != nil {
		t.Fatal(err)
	}
	got, ok := decodedCmd.Data.(*ConversationCatalogQuery)
	if !ok || got.Provider != nil || got.Channel == nil ||
		got.Channel.Endpoint != "https://gateway.example/v1" || got.Channel.Authentication != AuthenticationApiKey {
		t.Fatalf("channel query roundtrip mismatch: %+v", decodedCmd)
	}
}
