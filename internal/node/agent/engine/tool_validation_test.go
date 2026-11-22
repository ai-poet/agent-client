package engine

import (
	"encoding/json"
	"slices"
	"testing"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestToolValidationErrorOrderInNextRequest(t *testing.T) {
	tool := jsonTool("probe", "Fixture tool", `{
		"type":"object", "additionalProperties":false,
		"properties":{
			"choice":{"anyOf":[{"type":"string"},{"type":"boolean"}]},
			"details":{"type":"object", "additionalProperties":false, "required":["path","count"],
				"properties":{"path":{"type":"string"},"count":{"type":"integer"}}}
		}
	}`, true, true, nil)
	const arguments = `{"z":1,"details":{"y":1,"x":2},"choice":null,"a":2}`
	m := &scriptedModel{scripts: []script{
		response(
			call("first", "probe", arguments),
			call("repeat", "probe", arguments),
			call("reordered", "probe", `{"a":2,"choice":null,"details":{"x":2,"y":1},"z":1}`),
			finish(),
		),
		response(text("Done"), finish()),
	}}
	f := newFixture(t, m, tool)
	f.start()
	f.waitRun(f.submit("Validate the arguments"), protocol.ConversationStatusCompleted)
	if got := m.count(); got != 2 {
		t.Fatalf("model requests = %d, want 2", got)
	}
	var diagnostics []string
	for _, message := range m.calls[1].Prompt {
		for _, part := range message.Content {
			if result, ok := part.(fantasy.ToolResultPart); ok {
				diagnostics = append(diagnostics, result.Output.(fantasy.ToolResultOutputContentText).Text)
			}
		}
	}
	const message = `invalid tool arguments: jsonschema validation failed with 'urn:sailry:tool:probe#'
- at '': additional properties 'a', 'z' not allowed
- at '/choice': 'anyOf' failed
  - at '/choice': got null, want string
  - at '/choice': got null, want boolean
- at '/details': validation failed
  - at '/details': additional properties 'x', 'y' not allowed
  - at '/details': missing properties 'path', 'count'`
	data, err := json.Marshal(protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: message})
	if err != nil {
		t.Fatal(err)
	}
	want := "The tool call returned the following error:\nMcp error: -32000: " + message + " data: " + string(data)
	if !slices.Equal(diagnostics, []string{want, want, want}) {
		t.Fatalf("next request diagnostics = %q, want three copies of:\n%s", diagnostics, want)
	}
}
