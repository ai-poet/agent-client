package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"slices"
	"sort"

	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func CaptureSessionRoles(ctx context.Context, db *sql.Conn, node protocol.NodeID, execution protocol.ConversationExecution, team *protocol.TeamSelection, models map[protocol.RoleID]protocol.RoleModel) (protocol.RoleSnapshot, error) {
	value := protocol.DefaultRoleSnapshot()
	value.Execution = execution
	value.Models = models
	if execution == protocol.ConversationExecutionModel {
		if team != nil || len(models) > 0 {
			return value, invalid("model execution cannot select a delegation roster")
		}
		return value, nil
	}
	if execution != protocol.ConversationExecutionTeam {
		return value, invalid("invalid execution selection")
	}
	if team == nil {
		return captureAllSessionRoles(ctx, db, node, models, nil)
	}
	selected, e := readUserConfiguration[protocol.TeamProfile](ctx, db, "teams", team.ID.String())
	if e != nil {
		return value, e
	}
	if selected == nil {
		return value, fault(protocol.ErrorCodeNotFound, "team does not exist")
	}
	if team.Revision != nil {
		if e = checkRevision(selected.Revision, *team.Revision); e != nil {
			return value, e
		}
	}
	value.Team = &protocol.TeamReference{ID: selected.ID, Revision: selected.Revision}
	value.Instructions = selected.Instructions
	for _, id := range selected.Members {
		role, e := readUserConfiguration[protocol.RoleProfile](ctx, db, "roles", id.String())
		if e != nil {
			return value, e
		}
		if role == nil {
			return value, fault(protocol.ErrorCodeNotFound, "team role does not exist")
		}
		value.Profiles = append(value.Profiles, *role)
	}
	return completeRoleSnapshot(ctx, db, node, value, nil)
}

func captureAllSessionRoles(ctx context.Context, db *sql.Conn, node protocol.NodeID, models map[protocol.RoleID]protocol.RoleModel, lead *protocol.SessionConfig) (protocol.RoleSnapshot, error) {
	value := protocol.DefaultRoleSnapshot()
	value.Models = models
	var err error
	if value.Profiles, err = ReadRoles(ctx, db); err != nil {
		return value, err
	}
	return completeRoleSnapshot(ctx, db, node, value, lead)
}

// RoleCaptureError carries local roster identifiers. Only its underlying
// fault is returned on the wire and recorded in the admission ledger.
type RoleCaptureError struct {
	Role     protocol.RoleID
	Provider protocol.ProviderID
	err      error
}

func (e *RoleCaptureError) Error() string { return e.err.Error() }
func (e *RoleCaptureError) Unwrap() error { return e.err }

// The role profile and its frozen provider (including its absence) are session
// revision state. Use those facts, not the live catalog, for deferred failures.
func effectiveRoleModel(snapshot protocol.RoleSnapshot, role protocol.RoleProfile) *protocol.RoleModel {
	if override, ok := snapshot.Models[role.ID]; ok {
		return &override
	}
	return role.Model
}

// Inert provider metadata describes a deferred error; it must not require
// a live channel preset or a runtime credential just to start another role.
func servesRoster(provider protocol.ConversationProvider, snapshot protocol.RoleSnapshot) bool {
	serves := func(model *protocol.RoleModel) bool {
		return model != nil && model.Provider == provider.ID && slices.ContainsFunc(provider.Models, func(m protocol.ConversationModel) bool { return m.ID == model.Model })
	}
	return provider.Enabled && slices.ContainsFunc(snapshot.Profiles, func(role protocol.RoleProfile) bool {
		override, ok := snapshot.Models[role.ID]
		return serves(role.Model) || ok && serves(&override)
	})
}

func capturedRoleModel(snapshot protocol.RoleSnapshot, role protocol.RoleProfile) (*protocol.RoleModel, *protocol.ConversationProvider, error) {
	model := effectiveRoleModel(snapshot, role)
	if model == nil {
		return nil, nil, nil
	}
	var provider *protocol.ConversationProvider
	for i := range snapshot.Providers {
		if snapshot.Providers[i].ID == model.Provider {
			provider = &snapshot.Providers[i]
			break
		}
	}
	var reason string
	switch {
	case provider == nil:
		reason = "provider was missing or removed at capture"
	case !provider.Enabled:
		reason = "provider was disabled at capture"
	case !slices.ContainsFunc(provider.Models, func(m protocol.ConversationModel) bool { return m.ID == model.Model }):
		reason = "model was unavailable at capture"
	default:
		return model, provider, nil
	}
	return model, provider, fault(protocol.ErrorCodeNotConfigured, fmt.Sprintf("model %q is unavailable for role %q (%s), provider %s: %s", model.Model, role.Name, role.Key, model.Provider, reason))
}

// CapturedRoleAvailability checks an explicitly requested role without consulting current settings.
func CapturedRoleAvailability(snapshot protocol.RoleSnapshot, role protocol.RoleProfile) error {
	_, _, err := capturedRoleModel(snapshot, role)
	return err
}

func completeRoleSnapshot(ctx context.Context, db *sql.Conn, node protocol.NodeID, value protocol.RoleSnapshot, lead *protocol.SessionConfig) (protocol.RoleSnapshot, error) {
	type memberModel struct {
		role     protocol.RoleID
		model    protocol.RoleModel
		explicit bool
	}
	required := map[protocol.ProviderID][]memberModel{}
	ids := map[protocol.RoleID]bool{}
	keys := map[string]bool{}
	for _, role := range value.Profiles {
		if err := ValidateRole(role); err != nil {
			return value, err
		}
		if ids[role.ID] || keys[role.Key] {
			return value, invalid("session roles must have distinct identities and keys")
		}
		ids[role.ID], keys[role.Key] = true, true
		if model := role.Model; model != nil {
			required[model.Provider] = append(required[model.Provider], memberModel{role.ID, *model, false})
		}
		if model, ok := value.Models[role.ID]; ok {
			required[model.Provider] = append(required[model.Provider], memberModel{role.ID, model, true})
		}
	}
	for id := range value.Models {
		if !ids[id] {
			return value, invalid("member model requires a role in the session roster")
		}
	}
	captured := value.Providers
	value.Providers = []protocol.ConversationProvider{}
	ordered := []protocol.ProviderID{}
	for id := range required {
		ordered = append(ordered, id)
	}
	sort.Slice(ordered, func(i, j int) bool { return ordered[i].String() < ordered[j].String() })
	deferMember := func(member memberModel, f *protocol.Fault) error {
		if member.explicit {
			return &RoleCaptureError{member.role, member.model.Provider, f}
		}
		if _, overridden := value.Models[member.role]; overridden {
			return nil
		}
		reason := string(f.ConfigurationReason())
		if lead != nil && lead.Provider == member.model.Provider && (reason != "" || lead.Model == member.model.Model) {
			// This failure affects the selected model itself, not just a role.
			if reason != "" {
				return protocol.NewConfigurationFault(f.ConfigurationReason(), "selected model provider is unavailable")
			}
			return fault(protocol.ErrorCodeNotConfigured, "selected model is unavailable")
		}
		if reason == "" {
			reason = "role_model_missing"
		}
		logging.Emit(slog.LevelWarn, slog.String("area", "role_capture"), slog.String("phase", "degraded"),
			slog.String("role", member.role.String()), slog.String("provider", member.model.Provider.String()), slog.String("reason", reason))
		return nil
	}
	for _, id := range ordered {
		members := required[id]
		provider, err := listedRoleProvider(ctx, db, id)
		if err != nil {
			var f *protocol.Fault
			if !errors.As(err, &f) || (f.ConfigurationReason() != protocol.ConfigurationProviderMissing && f.ConfigurationReason() != protocol.ConfigurationProviderRemoved) {
				return value, &RoleCaptureError{members[0].role, id, err}
			}
			index := slices.IndexFunc(captured, func(p protocol.ConversationProvider) bool { return p.ID == id })
			if index < 0 {
				for _, member := range members {
					if err := deferMember(member, f); err != nil {
						return value, err
					}
				}
				continue
			}
			provider = captured[index]
		}
		if servesRoster(provider, value) {
			provider, err = CaptureProvider(ctx, db, provider)
			if err != nil {
				return value, &RoleCaptureError{members[0].role, id, err}
			}
		}
		if provider.Credential != nil {
			controller, err := CurrentController(ctx, db, node)
			if err != nil {
				return value, &RoleCaptureError{members[0].role, id, err}
			}
			if provider.Credential.Node != controller {
				return value, &RoleCaptureError{members[0].role, id, fault(protocol.ErrorCodePermissionDenied, "role provider belongs to another controller")}
			}
		}
		for _, member := range members {
			if err = validateRoleModel(provider, member.model); err != nil {
				var f *protocol.Fault
				if errors.As(err, &f) && f.Code == protocol.ErrorCodeNotConfigured {
					if err := deferMember(member, f); err != nil {
						return value, err
					}
					continue
				}
				return value, &RoleCaptureError{member.role, id, err}
			}
		}
		value.Providers = append(value.Providers, provider)
	}
	return value, nil
}
func SelectSessionRoles(ctx context.Context, db *sql.Conn, node protocol.NodeID, previous protocol.RoleSnapshot, references []protocol.RoleReference) (protocol.RoleSnapshot, error) {
	previous.Execution = protocol.ConversationExecutionTeam
	previous.Profiles = []protocol.RoleProfile{}
	ids := map[protocol.RoleID]bool{}
	for _, reference := range references {
		if ids[reference.ID] {
			return previous, invalid("session roles must be distinct")
		}
		ids[reference.ID] = true
		role, err := readUserConfiguration[protocol.RoleProfile](ctx, db, "roles", reference.ID.String())
		if err != nil {
			return previous, err
		}
		if role == nil {
			return previous, fault(protocol.ErrorCodeNotFound, "role does not exist")
		}
		if err = checkRevision(role.Revision, reference.Revision); err != nil {
			return previous, err
		}
		previous.Profiles = append(previous.Profiles, *role)
	}
	models := map[protocol.RoleID]protocol.RoleModel{}
	for id, model := range previous.Models {
		if ids[id] {
			models[id] = model
		}
	}
	previous.Models = models
	return completeRoleSnapshot(ctx, db, node, previous, nil)
}
func AssignSessionMemberModel(ctx context.Context, db *sql.Conn, node protocol.NodeID, previous protocol.RoleSnapshot, role protocol.RoleID, model *protocol.RoleModel) (protocol.RoleSnapshot, error) {
	if !slices.ContainsFunc(previous.Profiles, func(p protocol.RoleProfile) bool { return p.ID == role }) {
		return previous, fault(protocol.ErrorCodeNotFound, "role is not in the session roster")
	}
	models := map[protocol.RoleID]protocol.RoleModel{}
	for id, v := range previous.Models {
		models[id] = v
	}
	if model == nil {
		delete(models, role)
	} else {
		models[role] = *model
	}
	previous.Models = models
	return completeRoleSnapshot(ctx, db, node, previous, nil)
}
func RefreshTurnRoles(ctx context.Context, db *sql.Conn, snapshot *protocol.RoleSnapshot) error {
	for index, provider := range snapshot.Providers {
		if !servesRoster(provider, *snapshot) {
			continue
		}
		captured, err := CaptureProvider(ctx, db, provider)
		if err != nil {
			return err
		}
		snapshot.Providers[index] = captured
	}
	return nil
}
func roleModelConfig(base protocol.SessionConfig, provider protocol.ConversationProvider, selection protocol.RoleModel) (protocol.SessionConfig, error) {
	if err := validateRoleModel(provider, selection); err != nil {
		return base, err
	}
	base.Provider, base.Model, base.Credential = provider.ID, selection.Model, provider.Credential
	for _, model := range provider.Models {
		if model.ID == selection.Model {
			base.Effort = model.DefaultEffort
			break
		}
	}
	if selection.Effort != nil {
		base.Effort = *selection.Effort
	}
	features := base.Features
	if selection.Features != nil {
		features = *selection.Features
	}
	base.Features = map[string]string{}
	for key, value := range features {
		candidate := base
		candidate.Features = map[string]string{key: value}
		if providers.ValidateSelection(provider, candidate) == nil {
			base.Features[key] = value
		}
	}
	return base, nil
}
func sessionTeamCommand(ctx context.Context, db *sql.Conn, node protocol.NodeID, command protocol.Command) (protocol.Output, []protocol.Event, error) {
	var id protocol.SessionID
	var revision uint64
	switch v := command.Data.(type) {
	case *protocol.CommandSetSessionRolesPayload:
		id, revision = v.Session, v.ExpectedRevision
	case *protocol.CommandSetSessionExecutionPayload:
		id, revision = v.Session, v.ExpectedRevision
	case *protocol.CommandSetSessionMemberModelPayload:
		id, revision = v.Session, v.ExpectedRevision
	default:
		return protocol.Output{}, nil, invalid("invalid session team command")
	}
	session, err := checkedSession(ctx, db, id, revision)
	if err != nil {
		return protocol.Output{}, nil, err
	}
	switch v := command.Data.(type) {
	case *protocol.CommandSetSessionRolesPayload:
		session.Roles, err = SelectSessionRoles(ctx, db, node, session.Roles, v.Roles)
	case *protocol.CommandSetSessionMemberModelPayload:
		session.Roles, err = AssignSessionMemberModel(ctx, db, node, session.Roles, v.Role, v.Model)
	case *protocol.CommandSetSessionExecutionPayload:
		session.Roles, err = CaptureSessionRoles(ctx, db, node, v.Execution, v.Team, v.Models)
		if err == nil && v.Execution == protocol.ConversationExecutionTeam {
			session.Config, err = configureSessionTeam(ctx, db, session.Config, v.Team)
		}
	}
	if err != nil {
		return protocol.Output{}, nil, err
	}
	return reviseSession(ctx, db, session)
}

func configureSessionTeam(ctx context.Context, db *sql.Conn, config protocol.SessionConfig, selection *protocol.TeamSelection) (protocol.SessionConfig, error) {
	if selection == nil {
		return config, nil
	}
	team, err := readUserConfiguration[protocol.TeamProfile](ctx, db, "teams", selection.ID.String())
	if err != nil {
		return config, err
	}
	if team == nil {
		return config, fault(protocol.ErrorCodeNotFound, "team does not exist")
	}
	if team.Model == nil {
		return config, nil
	}
	provider, err := listedRoleProvider(ctx, db, team.Model.Provider)
	if err != nil {
		return config, err
	}
	return roleModelConfig(config, provider, *team.Model)
}
