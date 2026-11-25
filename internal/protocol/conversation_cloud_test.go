package protocol

import (
	"encoding/json"
	"testing"
)

func TestCloudConnectionRoundTrip(t *testing.T) {
	for _, tc := range []struct {
		api, family ConversationModelApi
		options     ConversationConnectionOptions
		endpoint    string
	}{
		{ConversationModelApiBedrock, ConversationModelApiAnthropic, ConversationConnectionOptions{"bedrock", ConversationConnectionOptionsBedrockPayload{"eu-west-1"}}, "https://bedrock-runtime.eu-west-1.amazonaws.com"},
		{ConversationModelApiVertexAi, ConversationModelApiGemini, ConversationConnectionOptions{"vertex_ai", &ConversationConnectionOptionsVertexAiPayload{"project-id", "global"}}, "https://aiplatform.googleapis.com"},
		{ConversationModelApiVertexAi, ConversationModelApiGemini, ConversationConnectionOptions{"vertex_ai", ConversationConnectionOptionsVertexAiPayload{"project-id", "us"}}, "https://aiplatform.us.rep.googleapis.com"},
		{ConversationModelApiVertexAi, ConversationModelApiGemini, ConversationConnectionOptions{"vertex_ai", ConversationConnectionOptionsVertexAiPayload{"project-id", "eu"}}, "https://aiplatform.eu.rep.googleapis.com"},
		{ConversationModelApiVertexAi, ConversationModelApiGemini, ConversationConnectionOptions{"vertex_ai", ConversationConnectionOptionsVertexAiPayload{"project-id", "us-central1"}}, "https://us-central1-aiplatform.googleapis.com"},
	} {
		p := DefaultConversationProvider()
		p.ID = NewProviderID()
		p.Api, p.Options, p.Authentication = tc.api, &tc.options, AuthenticationApiKey
		raw, err := json.Marshal(p)
		if err != nil {
			t.Fatal(err)
		}
		var decoded ConversationProvider
		if err := json.Unmarshal(raw, &decoded); err != nil {
			t.Fatal(err)
		}
		if decoded.Api != tc.api || decoded.Api.Family() != tc.family {
			t.Errorf("%s: unexpected API/family %s/%s", tc.api, decoded.Api, decoded.Api.Family())
		}
		for _, options := range []*ConversationConnectionOptions{p.Options, decoded.Options} {
			var endpoint string
			if v, ok := options.Bedrock(); ok {
				endpoint = v.Endpoint()
			}
			if v, ok := options.VertexAi(); ok && v.Project == "project-id" {
				endpoint = v.Endpoint()
			}
			if endpoint != tc.endpoint {
				t.Errorf("endpoint = %q, want %q", endpoint, tc.endpoint)
			}
		}
	}
}
