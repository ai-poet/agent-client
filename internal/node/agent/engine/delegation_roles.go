package engine

import (
	"slices"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

type missingRoleRequirements struct {
	skills []string
	tools  []string
}

func (m missingRoleRequirements) reason() string {
	var reasons []string
	if len(m.skills) > 0 {
		reasons = append(reasons, "required skills are missing: "+strings.Join(m.skills, ", "))
	}
	if len(m.tools) > 0 {
		reasons = append(reasons, "required tools are unavailable: "+strings.Join(m.tools, ", "))
	}
	return strings.Join(reasons, "; ")
}

func (r *Runtime) missingRoleRequirements(x *execution, role protocol.RoleProfile) missingRoleRequirements {
	var missing missingRoleRequirements
	var skills map[string]protocol.PluginSkill
	if len(role.Skills) != 0 && x.resources != nil {
		skills = x.resources.Skills()
	}
	for _, key := range role.Skills {
		if _, ok := skills[key]; !ok {
			missing.skills = append(missing.skills, key)
		}
	}
	missing.tools = missingRoleTools(role, x.inv.Turn.Config.Mode, roleModelSupportsTools(x, role), x.tools)
	return missing
}

func missingRoleTools(role protocol.RoleProfile, mode protocol.WorkMode, modelTools bool, tools []Tool) []string {
	var missing []string
	if role.Tools == nil {
		return nil
	}
	for _, selection := range *role.Tools {
		if !modelTools || !slices.ContainsFunc(tools, func(tool Tool) bool {
			return inheritableChildTool(tool) && (!(role.ReadOnly || mode == protocol.WorkModePlan) || tool.ReadOnly) && roleToolMatches(selection, tool)
		}) {
			missing = append(missing, roleToolLabel(selection))
		}
	}
	return missing
}

func roleToolLabel(selection protocol.PluginConversationTool) string {
	switch value := selection.Data.(type) {
	case *protocol.PluginConversationToolBuiltinPayload:
		return value.Name
	case *protocol.PluginConversationToolPackagePayload:
		return value.Package + "/" + value.Name
	default:
		return selection.Kind
	}
}

func roleModelSupportsTools(x *execution, role protocol.RoleProfile) bool {
	roster := x.inv.Turn.Roles
	selection := role.Model
	if override, ok := roster.Models[role.ID]; ok {
		selection = &override
	}
	if selection == nil {
		return modelSupportsTools(x.inv.Provider, x.inv.Turn.Config.Model)
	}
	for i := range roster.Providers {
		if roster.Providers[i].ID == selection.Provider {
			return modelSupportsTools(&roster.Providers[i], selection.Model)
		}
	}
	return false
}

func modelSupportsTools(provider *protocol.ConversationProvider, name string) bool {
	return provider != nil && slices.ContainsFunc(provider.Models, func(model protocol.ConversationModel) bool {
		return model.ID == name && model.Tools
	})
}
