package model

import (
	"encoding/json"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/output"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestVisibleRecordsCanBeProjectedAgain(t *testing.T) {
	turn := protocol.NewTurnID()
	message := func(text string) canonical.Record {
		return canonical.NewRecord(turn, "message", canonical.NewMessage(canonical.User, canonical.Text(0, text)), time.Time{})
	}
	old, revealed, hidden := message("Old history"), message("Revealed input"), message("Hidden input")
	revealed.Metadata.Runtime = &canonical.Runtime{AgentVisible: false}
	summary := canonical.NewRecord(turn, "compaction", &canonical.Compaction{Boundary: old.ID, Summary: canonical.NewMessage(canonical.User, canonical.Text(0, "Summary"))}, time.Time{})
	visibility := func(record canonical.Record, visible bool) canonical.Record {
		operation := canonical.NewRecord(turn, "operation", &canonical.Operation{Kind: "visibility", Data: &canonical.Visibility{Message: record.ID, AgentVisible: visible}}, time.Time{})
		operation.Metadata.Runtime = &canonical.Runtime{AgentVisible: false}
		return operation
	}
	records := []canonical.Record{old, revealed, hidden, summary, visibility(revealed, true), visibility(hidden, false)}
	once := VisibleRecords(records)
	if len(once) != 2 || once[0].ID != summary.ID || once[1].ID != revealed.ID {
		t.Fatalf("projection = %+v", once)
	}
	if twice := VisibleRecords(once); !reflect.DeepEqual(once, twice) {
		t.Fatal("reprojection lost the handoff or a visibility override")
	}
	if revealed.Metadata.Runtime.AgentVisible || !reflect.DeepEqual(once[1].Message(), revealed.Message()) {
		t.Fatal("projection mutated the original audience or replaced message content")
	}
}

func TestRollingSummaryRetainsTailAndResetsReadProjection(t *testing.T) {
	turn := protocol.NewTurnID()
	message := func(role canonical.Role, blocks ...canonical.Block) canonical.Record {
		return canonical.NewRecord(turn, "message", canonical.NewMessage(role, blocks...), time.Time{})
	}
	pair := func(id string) (canonical.Record, canonical.Record) {
		call := message(canonical.Assistant, canonical.Block{Kind: "tool_request", Data: &canonical.ToolRequest{ID: id, Name: "read_file", Arguments: json.RawMessage(`{"path":"fixture.go"}`)}})
		result := message(canonical.User, canonical.Block{Kind: "tool_response", Data: &canonical.ToolResponse{ID: id, Name: "read_file", Revision: strings.Repeat("a", 64), Status: canonical.ToolSuccess, Format: canonical.JSON,
			Result: canonical.Payload{Value: json.RawMessage(`{"kind":"file_lines","data":{"path":"fixture.go","lines":[{"number":1,"text":"retained line\n"}],"size":14,"revision":"file-revision","offset":0,"truncated":false}}`)}}})
		return call, result
	}
	oldCall, oldResult := pair("old")
	tailCall, tailResult := pair("tail")
	tail := message(canonical.User, canonical.Text(0, "Read the file again"))
	newest := message(canonical.User, canonical.Text(0, "Continue with this evidence"))
	summary := canonical.NewRecord(turn, "compaction", &canonical.Compaction{Boundary: oldResult.ID, Summary: canonical.NewMessage(canonical.User, canonical.Text(0, "Rolling handoff"))}, time.Time{})
	records := []canonical.Record{oldCall, oldResult, tail, tailCall, tailResult, summary, newest}
	before, err := json.Marshal(records)
	if err != nil {
		t.Fatal(err)
	}
	visible := VisibleRecords(records)
	want := []string{summary.ID, tail.ID, tailCall.ID, tailResult.ID, newest.ID}
	ids := func(records []canonical.Record) []string {
		var ids []string
		for _, record := range records {
			ids = append(ids, record.ID)
		}
		return ids
	}
	if !reflect.DeepEqual(ids(visible), want) || !reflect.DeepEqual(VisibleRecords(visible), visible) || !reflect.DeepEqual(Answered(visible), visible) {
		t.Fatal("rolling summary changed tail order, visibility or complete tool pairs")
	}
	var cache output.Cache
	warmed := cache.Project(Answered(records[:5]))
	if response := warmed[4].Message().Content[0].Data.(*canonical.ToolResponse); !strings.Contains(string(response.Result.Value), "are unchanged") {
		t.Fatal("fixture did not establish an earlier read")
	}
	projected := cache.Project(Answered(visible))
	if response := projected[3].Message().Content[0].Data.(*canonical.ToolResponse); strings.Contains(string(response.Result.Value), "are unchanged") || !strings.Contains(string(response.Result.Value), "retained line") {
		t.Fatal("a compacted read suppressed retained tail evidence")
	}
	provider := protocol.DefaultConversationProvider()
	provider.Api = protocol.ConversationModelApiChatCompletions
	binding := Binding{Provider: provider, Config: protocol.DefaultSessionConfig()}
	fullCall, err := BuildCall(binding, Request{Records: records})
	if err != nil {
		t.Fatal(err)
	}
	tailCallProjection, err := BuildCall(binding, Request{Records: visible})
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(fullCall.Prompt, tailCallProjection.Prompt) || len(fullCall.Prompt) != 5 || fullCall.Prompt[0].Content[0].(fantasy.TextPart).Text != "Rolling handoff" {
		t.Fatal("SDK request changed the summary and retained tail")
	}
	selection := protocol.DefaultConversationPresetsBinding()
	selection.Adapter = &protocol.PluginModelAdapterBinding{Capabilities: protocol.PluginModelAdapterCapabilities{Tools: true}}
	captured := &selection
	binding.Provider.Api, binding.Provider.Binding = protocol.ConversationModelApiPlugin, &captured
	fullPlugin, err := pluginRequest(binding, Request{Records: records})
	if err != nil {
		t.Fatal(err)
	}
	tailPlugin, err := pluginRequest(binding, Request{Records: visible})
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(fullPlugin.Messages, tailPlugin.Messages) || len(fullPlugin.Messages) != 5 || fullPlugin.Messages[0].Data.(*protocol.PluginModelAdapterWireMessageUserPayload).Parts[0].Data.(*protocol.PluginModelAdapterWirePartTextPayload).Text != "Rolling handoff" {
		t.Fatal("plugin request changed the summary and retained tail")
	}
	next := canonical.NewRecord(turn, "compaction", &canonical.Compaction{Boundary: tailResult.ID, Summary: canonical.NewMessage(canonical.User, canonical.Text(0, "Updated handoff"))}, time.Time{})
	if got := VisibleRecords(append(append([]canonical.Record(nil), records...), next)); !reflect.DeepEqual(ids(got), []string{next.ID, newest.ID}) {
		t.Fatal("a later rolling summary retained superseded history")
	}
	after, err := json.Marshal(records)
	if err != nil || string(before) != string(after) {
		t.Fatal("request projection mutated canonical records")
	}
}

func TestAnsweredPairsReusedIDsByOccurrence(t *testing.T) {
	turn := protocol.NewTurnID()
	request := func() canonical.Record {
		return canonical.NewRecord(turn, "message", canonical.NewMessage(canonical.Assistant, canonical.Block{
			Kind: "tool_request", Data: &canonical.ToolRequest{ID: "reused", Name: "read", Arguments: json.RawMessage(`{}`)},
		}), time.Time{})
	}
	response := func(value string) canonical.Block {
		return canonical.Block{Kind: "tool_response", Data: &canonical.ToolResponse{ID: "reused", Name: "read", Result: canonical.Payload{Value: json.RawMessage(value)}}}
	}
	result := func(blocks ...canonical.Block) canonical.Record {
		return canonical.NewRecord(turn, "message", canonical.NewMessage(canonical.User, blocks...), time.Time{})
	}
	records := []canonical.Record{request(), result(response(`1`), response(`2`)), request(), result(response(`3`)), request()}
	projected := Answered(records)
	first := projected[0].Message().Content[0].Data.(*canonical.ToolRequest).ID
	firstResult := projected[1].Message().Content
	second := projected[2].Message().Content[0].Data.(*canonical.ToolRequest).ID
	secondResult := projected[3].Message().Content[0].Data.(*canonical.ToolResponse)
	if first != "reused" || len(firstResult) != 1 || firstResult[0].Data.(*canonical.ToolResponse).ID != first || second == first || secondResult.ID != second {
		t.Fatalf("history pairs lost their occurrences: %q, %v, %q, %+v", first, firstResult, second, secondResult)
	}
	if len(projected[4].Message().Content) != 0 || len(records[1].Message().Content) != 2 || records[2].Message().Content[0].Data.(*canonical.ToolRequest).ID != "reused" {
		t.Fatal("repair did not remove exactly the unpaired blocks or mutated stored history")
	}
	if again := Answered(projected); !reflect.DeepEqual(again, projected) {
		t.Fatal("a second projection changed established tool pairs")
	}
}

func TestBuildProjectedCallMatchesRawAudience(t *testing.T) {
	turn := protocol.NewTurnID()
	call := canonical.Block{Kind: "tool_request", Data: &canonical.ToolRequest{ID: "c", Name: "sample", Arguments: json.RawMessage(`{}`)}}
	result := canonical.Block{Kind: "tool_response", Data: &canonical.ToolResponse{ID: "c", Name: "sample", Format: canonical.MCP, Status: canonical.ToolSuccess, Result: canonical.Payload{Value: json.RawMessage(`{"content":[{"type":"text","text":"secret-fixture","annotations":{"audience":["user"]}},{"type":"text","text":"allowed-fixture"}]}`)}}}
	records := []canonical.Record{canonical.NewRecord(turn, "message", canonical.NewMessage(canonical.Assistant, call), time.Time{}), canonical.NewRecord(turn, "message", canonical.NewMessage(canonical.User, result), time.Time{})}
	var cache output.Cache
	filtered := cache.Project(records)
	binding := Binding{Provider: protocol.DefaultConversationProvider(), Config: protocol.DefaultSessionConfig()}
	for _, api := range []protocol.ConversationModelApi{protocol.ConversationModelApiChatCompletions, protocol.ConversationModelApiPlugin} {
		binding.Provider.Api = api
		if api == protocol.ConversationModelApiPlugin {
			selection := protocol.DefaultConversationPresetsBinding()
			selection.Adapter = &protocol.PluginModelAdapterBinding{Capabilities: protocol.PluginModelAdapterCapabilities{Tools: true}}
			capture := &selection
			binding.Provider.Binding = &capture
		}
		raw, err := BuildCall(binding, Request{Records: records})
		if err != nil {
			t.Fatal(err)
		}
		projected, err := BuildProjectedCall(binding, Request{Records: filtered})
		if err != nil {
			t.Fatal(err)
		}
		var source, actual any = raw.Prompt, projected.Prompt
		if api == protocol.ConversationModelApiPlugin {
			source = raw.ProviderOptions[pluginRequestKey]
			actual = projected.ProviderOptions[pluginRequestKey]
		}
		body, err := json.Marshal(source)
		if err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(source, actual) || strings.Contains(string(body), "secret-fixture") || !strings.Contains(string(body), "allowed-fixture") {
			t.Fatalf("%s raw/projected output diverged or contained wrong audience: %s", api, body)
		}
	}
}

func TestAnthropicParallelToolCoalescingAndBreakpoints(t *testing.T) {
	turn := protocol.NewTurnID()
	makeMsg := func(role canonical.Role, blocks ...canonical.Block) canonical.Record {
		msg := canonical.NewMessage(role, blocks...)
		return canonical.NewRecord(turn, "message", &msg, time.Time{})
	}
	user := makeMsg(canonical.User, canonical.Text(0, "Inspect 12 files"))
	var reqBlocks, resBlocks []canonical.Block
	for i := range 12 {
		id := "call-" + strconv.Itoa(i)
		reqBlocks = append(reqBlocks, canonical.Block{Index: uint64(i), Kind: "tool_request", Data: &canonical.ToolRequest{ID: id, Name: "read_file", Arguments: json.RawMessage(`{"path":"a.go"}`)}})
		resBlocks = append(resBlocks, canonical.Block{Index: uint64(i), Kind: "tool_response", Data: &canonical.ToolResponse{
			ID: id, Name: "read_file", Revision: strings.Repeat("b", 64), Status: canonical.ToolSuccess, Format: canonical.JSON,
			Result: canonical.Payload{Value: json.RawMessage(`{"ok":true}`)},
		}})
	}
	assistant := makeMsg(canonical.Assistant, reqBlocks...)
	toolMsg := makeMsg(canonical.User, resBlocks...)

	provider := protocol.DefaultConversationProvider()
	provider.ID, provider.Api = protocol.NewProviderID(), protocol.ConversationModelApiAnthropic
	provider.Authentication = protocol.AuthenticationApiKey
	config := protocol.DefaultSessionConfig()
	config.Provider, config.Model = provider.ID, "claude-opus-5-5"
	binding := Binding{Provider: provider, Config: config, Session: protocol.NewSessionID()}

	call, err := BuildCall(binding, Request{System: "Stable system", Records: []canonical.Record{user, assistant, toolMsg}})
	if err != nil {
		t.Fatal(err)
	}
	body, err := requestJSON(transportConfig{Kind: "anthropic", Model: config.Model}, call)
	if err != nil {
		t.Fatal(err)
	}
	messages, _ := body["messages"].([]any)
	if len(messages) != 3 {
		t.Fatalf("anthropic messages = %d, want 3 (user + assistant + single coalesced tool_result user message)", len(messages))
	}
	// Prior user message MUST hold the earlier conversation cache breakpoint.
	userBlocks := messages[0].(map[string]any)["content"].([]any)
	if userBlocks[len(userBlocks)-1].(map[string]any)["cache_control"] == nil {
		t.Fatal("prior user message lost its cache breakpoint across a parallel tool batch")
	}
	// All 12 tool results must be coalesced into messages[2] with cache_control only on the final block.
	toolBlocks := messages[2].(map[string]any)["content"].([]any)
	if len(toolBlocks) != 12 {
		t.Fatalf("coalesced tool_result count = %d, want 12", len(toolBlocks))
	}
	for i, raw := range toolBlocks {
		block := raw.(map[string]any)
		hasCache := block["cache_control"] != nil
		if hasCache != (i == len(toolBlocks)-1) {
			t.Fatalf("tool_result[%d] cache_control=%v, want %v", i, hasCache, i == len(toolBlocks)-1)
		}
	}
}

func TestGeminiPreservesThoughtSignatureAcrossTurns(t *testing.T) {
	turn1, turn2 := protocol.NewTurnID(), protocol.NewTurnID()
	provider := protocol.DefaultConversationProvider()
	provider.ID, provider.Api = protocol.NewProviderID(), protocol.ConversationModelApiGemini
	provider.Authentication = protocol.AuthenticationApiKey
	config := protocol.DefaultSessionConfig()
	config.Provider, config.Model = provider.ID, "gemini-3-pro"
	binding := Binding{Provider: provider, Config: config, Session: protocol.NewSessionID()}
	scope := Scope(binding)

	sig := "c2lnbmVkLXRob3VnaHQ="
	u1Msg := canonical.NewMessage(canonical.User, canonical.Text(0, "First question"))
	u1 := canonical.NewRecord(turn1, "message", &u1Msg, time.Time{})
	a1Msg := canonical.NewMessage(canonical.Assistant,
		canonical.Block{Index: 0, Kind: "thinking", Data: &canonical.Thinking{Text: "Plan", Signature: &sig}},
		canonical.Text(1, "First answer"),
	)
	a1 := canonical.NewRecord(turn1, "message", &a1Msg, time.Time{})
	a1.Metadata.Execution = &canonical.Execution{Source: &scope}
	u2Msg := canonical.NewMessage(canonical.User, canonical.Text(0, "Second question"))
	u2 := canonical.NewRecord(turn2, "message", &u2Msg, time.Time{})

	call, err := BuildCall(binding, Request{System: "Stable", Records: []canonical.Record{u1, a1, u2}})
	if err != nil {
		t.Fatal(err)
	}
	body, err := requestJSON(transportConfig{Kind: "google", Model: config.Model}, call)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(body)
	if !strings.Contains(string(raw), sig) {
		t.Fatalf("prior turn Gemini thoughtSignature was stripped on turn 2: %s", raw)
	}
}
