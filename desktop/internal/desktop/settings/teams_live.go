package settings

import (
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
)

// TeamActions connects existing catalog editors to revision-checked writes.
type TeamActions struct {
	SaveConcurrency func()
	SaveTeam        func(viewmodel.Team, func(kit.Outcome))
	SaveRole        func(viewmodel.Role, func(kit.Outcome))
	RemoveTeam      func(viewmodel.Team, func(kit.Outcome))
	RemoveRole      func(viewmodel.Role, func(kit.Outcome))
}
