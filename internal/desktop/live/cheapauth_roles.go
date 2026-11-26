package live

import (
	"slices"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// Reconciliation keeps retired providers listed. A group change may instead
// leave its lane enabled while withdrawing the particular model a role uses.
func affectedCheapRoles(snapshot protocol.Snapshot) []protocol.RoleProfile {
	var affected []protocol.RoleProfile
	for _, role := range snapshot.Roles {
		if listed, usable := cheapRoleRoute(role.Model, snapshot.Providers); listed && !usable {
			affected = append(affected, role)
		}
	}
	return affected
}

func cheapRoleRoute(m *protocol.RoleModel, current []protocol.ConversationProvider) (listed, usable bool) {
	if m != nil && providers.IsCheapRouterProvider(m.Provider) {
		for _, p := range current {
			if p.ID == m.Provider {
				return true, p.Enabled && slices.ContainsFunc(p.Models, func(offer protocol.ConversationModel) bool { return offer.ID == m.Model })
			}
		}
	}
	return false, false
}

func (r *Runtime) migrateCheapRoles() {
	account := r.cheapAuth
	state := account.state
	if state.MigratingRoles || !r.cheapRoutesSettled() {
		return
	}
	roles := affectedCheapRoles(r.snapshot())
	if len(roles) == 0 {
		return
	}
	state.MigratingRoles = true
	generation := account.generation
	r.async(func() {
		// The provider write may have settled before the Client saw its event.
		current, err := r.readCheapProviders(r.ctx)
		r.enqueue(func(c *ui.Context) {
			switch {
			case err != nil:
				r.finishRoleMigration(c, roleMigrationReceipt{failed: len(roles)})
			case generation != account.generation || !r.cheapRoutesSettled():
				r.finishRoleMigration(c, roleMigrationReceipt{skipped: len(roles)})
			default:
				r.submitRoleMigration(c, roles, current)
			}
		})
	})
}

func (r *Runtime) submitRoleMigration(c *ui.Context, roles []protocol.RoleProfile, current []protocol.ConversationProvider) {
	var lanes map[string]string
	if hints := r.cheapModelGroupHints(); hints != nil {
		lanes = hints.modelLanes
	}
	var receipt roleMigrationReceipt
	remaining := 0
	// Keep the exact role revision the user acted on. Each admitted change
	// settles independently, including after leaving Settings.
	for _, role := range roles {
		model := *role.Model
		successor, ok := providers.CheapRouterSuccessor(model.Provider, model.Model, lanes, current)
		if !ok {
			receipt.skipped++
			if _, usable := cheapRoleRoute(&model, current); !usable {
				receipt.manual = append(receipt.manual, role.Name)
			}
			continue
		}
		model.Provider = successor.ID
		role.Model = &model
		command := protocol.Command{Kind: "put_role", Data: protocol.CommandPutRolePayload{Role: role, ExpectedRevision: role.Revision}}
		remaining++
		r.executeScoped("teams/migrate-role", command, func(c *ui.Context, _ protocol.Output, err error) {
			switch teamOutcome(err) {
			case kit.Done:
				receipt.saved++
			case kit.Uncertain:
				receipt.unknown++
			case kit.Conflict:
				receipt.failed++
				receipt.conflict = true
			default:
				receipt.failed++
			}
			remaining--
			if remaining == 0 {
				r.finishRoleMigration(c, receipt)
			}
		})
	}
	if remaining == 0 {
		r.finishRoleMigration(c, receipt)
	}
}

type roleMigrationReceipt struct {
	saved, failed, skipped, unknown int
	manual                          []string
	conflict                        bool
}

func (r *Runtime) finishRoleMigration(c *ui.Context, result roleMigrationReceipt) {
	r.cheapAuth.state.MigratingRoles = false
	outcome := kit.Done
	message := i18n.T(c, "cheaprouter_roles_receipt", "saved", result.saved, "failed", result.failed, "skipped", result.skipped)
	if result.failed > 0 || len(result.manual) != 0 {
		outcome = kit.Failed
	}
	if result.conflict {
		message += "\n" + i18n.T(c, "teams_role_conflict")
	}
	if len(result.manual) != 0 {
		message += "\n" + i18n.T(c, "cheaprouter_roles_manual", "names", strings.Join(result.manual, ", "))
	}
	if result.unknown > 0 {
		outcome = kit.Uncertain
		message += "\n" + i18n.T(c, "cheaprouter_roles_unknown", "count", result.unknown)
	}
	kit.Toast(c, outcome, message)
}
