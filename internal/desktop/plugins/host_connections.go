package plugins

import (
	"context"
	"encoding/json"
)

func (owner *Host) connectorModule() HostModule {
	return HostModule{Name: "sailry/connectors", Exports: []HostExport{
		hostAsync("list", owner.readOperation("list_connections", nil)),
		hostAsync("types", owner.readOperation("list_connector_types", nil)),
		hostAsync("status", func(ctx context.Context, arguments []json.RawMessage) (json.RawMessage, error) {
			connector, err := hostValue[string](arguments, 0)
			if err != nil {
				return nil, err
			}
			body, err := owner.readData(ctx, "list_connections", nil)
			if err != nil {
				return nil, err
			}
			var rows []map[string]json.RawMessage
			if json.Unmarshal(body, &rows) != nil {
				return nil, hostInvalid("invalid connector response")
			}
			for _, row := range rows {
				if nativeProp(row, "connector", "") == connector {
					return hostJSON(row)
				}
			}
			return json.RawMessage("null"), nil
		}),
		hostSync("login", func(ctx context.Context, arguments []json.RawMessage) (json.RawMessage, error) {
			input, err := hostValue[map[string]json.RawMessage](arguments, 0)
			if err != nil {
				return nil, err
			}
			return owner.prepareData(ctx, "start_connector_login", map[string]any{
				"package": owner.scope().Package, "connector": input["connector"],
				"connection": optionalRaw(input, "connection"), "expected_revision": optionalRaw(input, "expected_revision"),
				"method": input["method"], "input": map[string]any{"redirect": optionalRaw(input, "redirect")},
			})
		}),
		hostSync("cancel", owner.prepareOperation("cancel_connector_login", namedArguments("attempt"))),
	}}
}

func optionalRaw(values map[string]json.RawMessage, name string) json.RawMessage {
	if value, exists := values[name]; exists {
		return value
	}
	return json.RawMessage("null")
}
