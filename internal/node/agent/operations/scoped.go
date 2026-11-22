package operations

import (
	"bytes"
	"encoding/json"
	"math"
	"strconv"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// ScopedOperation reports whether an operation manages the role catalog.
// The Node dispatches these under the
// declaring package instead of a workspace command.
func ScopedOperation(operation protocol.ToolOperation) bool {
	switch operation {
	case protocol.ToolOperationListRoles, protocol.ToolOperationSaveRole, protocol.ToolOperationRemoveRole, protocol.ToolOperationApplyRoles:
		return true
	}
	return false
}

// ScopedCommand maps a role operation onto the Node command that owns
// it. Role application always targets the captured session.
func ScopedCommand(operation protocol.ToolOperation, arguments json.RawMessage, session protocol.SessionID) (protocol.Command, error) {
	strict := func(target any) error {
		decoder := json.NewDecoder(bytes.NewReader(arguments))
		decoder.DisallowUnknownFields()
		if err := decoder.Decode(target); err != nil {
			return invalidCommand("invalid role arguments")
		}
		return nil
	}
	switch operation {

	case protocol.ToolOperationListRoles:
		var input struct{}
		if err := strict(&input); err != nil {
			return protocol.Command{}, err
		}
		return protocol.Command{Kind: "list_roles"}, nil
	case protocol.ToolOperationSaveRole:
		var input struct {
			Role             map[string]json.RawMessage `json:"role"`
			ExpectedRevision *string                    `json:"expected_revision"`
		}
		if err := strict(&input); err != nil {
			return protocol.Command{}, err
		}
		if input.Role == nil || input.ExpectedRevision == nil {
			return protocol.Command{}, invalidCommand("invalid role arguments")
		}
		var current string
		if json.Unmarshal(input.Role["revision"], &current) != nil {
			return protocol.Command{}, invalidCommand("role revision must be a decimal string")
		}
		revision, err := roleRevision(current)
		if err != nil {
			return protocol.Command{}, err
		}
		input.Role["revision"] = json.RawMessage(strconv.FormatUint(revision, 10))
		body, err := json.Marshal(input.Role)
		if err != nil {
			return protocol.Command{}, invalidCommand("invalid role arguments")
		}
		var role protocol.RoleProfile
		if json.Unmarshal(body, &role) != nil {
			return protocol.Command{}, invalidCommand("invalid role arguments")
		}
		expected, err := roleRevision(*input.ExpectedRevision)
		if err != nil {
			return protocol.Command{}, err
		}
		return protocol.Command{Kind: "put_role", Data: &protocol.CommandPutRolePayload{Role: role, ExpectedRevision: expected}}, nil
	case protocol.ToolOperationRemoveRole:
		var input struct {
			Role             *protocol.RoleID `json:"role"`
			ExpectedRevision *string          `json:"expected_revision"`
		}
		if err := strict(&input); err != nil {
			return protocol.Command{}, err
		}
		if input.Role == nil || input.ExpectedRevision == nil {
			return protocol.Command{}, invalidCommand("invalid role arguments")
		}
		expected, err := roleRevision(*input.ExpectedRevision)
		if err != nil {
			return protocol.Command{}, err
		}
		return protocol.Command{Kind: "remove_role", Data: &protocol.CommandRemoveRolePayload{Role: *input.Role, ExpectedRevision: expected}}, nil
	case protocol.ToolOperationApplyRoles:
		var input struct {
			Roles *[]struct {
				ID       *protocol.RoleID `json:"id"`
				Revision *string          `json:"revision"`
			} `json:"roles"`
			ExpectedRevision *string `json:"expected_revision"`
		}
		if err := strict(&input); err != nil {
			return protocol.Command{}, err
		}
		if input.Roles == nil || input.ExpectedRevision == nil {
			return protocol.Command{}, invalidCommand("invalid role arguments")
		}
		seen := map[protocol.RoleID]bool{}
		for _, reference := range *input.Roles {
			if reference.ID == nil || reference.Revision == nil {
				return protocol.Command{}, invalidCommand("invalid role arguments")
			}
			if seen[*reference.ID] {
				return protocol.Command{}, invalidCommand("session roles must be distinct")
			}
			seen[*reference.ID] = true
		}
		roles := make([]protocol.RoleReference, 0, len(*input.Roles))
		for _, reference := range *input.Roles {
			revision, err := roleRevision(*reference.Revision)
			if err != nil {
				return protocol.Command{}, err
			}
			roles = append(roles, protocol.RoleReference{ID: *reference.ID, Revision: revision})
		}
		expected, err := roleRevision(*input.ExpectedRevision)
		if err != nil {
			return protocol.Command{}, err
		}
		return protocol.Command{Kind: "set_session_roles", Data: &protocol.CommandSetSessionRolesPayload{Session: session, ExpectedRevision: expected, Roles: roles}}, nil
	}
	return protocol.Command{}, invalidCommand("native operation is unavailable")
}

// roleRevision reads the exact decimal revision a script-safe caller supplies.
func roleRevision(value string) (uint64, error) {
	number, err := strconv.ParseUint(value, 10, 64)
	if err != nil || strconv.FormatUint(number, 10) != value || number > math.MaxInt64 {
		return 0, invalidCommand("invalid role revision")
	}
	return number, nil
}

// ScopedOutput projects a role outcome for the calling tool. Role
// outcomes carry public definitions with decimal-string revisions.
func ScopedOutput(operation protocol.ToolOperation, output protocol.Output) (any, error) {
	if !ScopedOperation(operation) {
		return output, nil
	}
	if value, listed := outputPayload[[]protocol.RoleProfile](output); listed {
		profiles := make([]any, 0, len(value))
		for _, role := range value {
			profile, err := ExactRevision(role, role.Revision)
			if err != nil {
				return nil, err
			}
			profiles = append(profiles, profile)
		}
		return map[string]any{"kind": "roles", "data": profiles}, nil
	}
	if value, saved := outputPayload[protocol.RoleProfile](output); saved {
		profile, err := ExactRevision(value, value.Revision)
		if err != nil {
			return nil, err
		}
		return map[string]any{"kind": "role", "data": profile}, nil
	}
	if value, applied := outputPayload[protocol.Session](output); applied {
		roles := make([]any, 0, len(value.Roles.Profiles))
		for _, role := range value.Roles.Profiles {
			roles = append(roles, map[string]any{"id": role.ID, "revision": strconv.FormatUint(role.Revision, 10)})
		}
		return map[string]any{"kind": "session_roles", "data": map[string]any{"session": value.ID, "revision": strconv.FormatUint(value.Revision, 10), "roles": roles}}, nil
	}
	return nil, &protocol.Fault{Code: protocol.ErrorCodeInternal, Message: "role outcome is unavailable"}
}

// outputPayload reads a command outcome delivered by value or by pointer.
func outputPayload[Value any](output protocol.Output) (Value, bool) {
	switch value := output.Data.(type) {
	case Value:
		return value, true
	case *Value:
		if value != nil {
			return *value, true
		}
	}
	var empty Value
	return empty, false
}

// ExactRevision renders a catalog entry with its revision as a decimal string,
// which keeps exact 64-bit values for script-safe models.
func ExactRevision(entry any, revision uint64) (map[string]any, error) {
	body, err := json.Marshal(entry)
	if err != nil {
		return nil, err
	}
	var value map[string]any
	if err := json.Unmarshal(body, &value); err != nil {
		return nil, err
	}
	value["revision"] = strconv.FormatUint(revision, 10)
	return value, nil
}
