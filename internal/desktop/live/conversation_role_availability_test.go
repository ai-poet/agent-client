package live

import (
	"sync/atomic"
	"testing"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/shell"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestNewSessionSendsAfterUnrelatedRoleLaneIsUnbound(t *testing.T) {
	gateway := newCheapGateway(t)
	gateway.catalogBody = `{"data":{"items":[{"model":"claude-sonnet-4-6","best_group":{"id":1}},{"model":"gpt-5","best_group":{"id":2}}]}}`
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		d.signedIn(gateway.URL, cheapTestAccess, time.Hour)
	})
	codex := providers.CheapRouterLaneID(core.LaneCodex)
	d.await("Codex lane enabled", func() bool {
		p := d.provider(codex)
		return d.account().Stage == cheapauthui.Ready && p != nil && p.Enabled
	})
	role := protocol.DefaultRoleProfile()
	role.ID, role.Key, role.Name = protocol.NewRoleID(), "review", "Reviewer"
	role.Model = &protocol.RoleModel{Provider: codex, Model: "gpt-5"}
	d.do(protocol.Command{Kind: "put_role", Data: protocol.CommandPutRolePayload{Role: role}})
	d.account().SelectLaneGroup(core.LaneCodex, nil)
	d.await("Codex lane disabled", func() bool {
		p := d.provider(codex)
		return !d.account().SwitchingGroup && p != nil && !p.Enabled
	})

	d.runtime.App.Navigate(shell.ConversationPage)
	d.runtime.App.Sidebar.NewChat()
	d.tester.Frame()
	comp := d.runtime.App.Composer(d.runtime.App.Sidebar.Selected)
	comp.Model = modelKey(providers.CheapRouterLaneID(core.LaneClaude), "claude-sonnet-4-6")
	d.tester.Frame()
	d.click(i18n.In("zh-CN", "composer"))
	d.tester.Type("Hello")
	d.tester.Frame()
	var submitted atomic.Bool
	observe := func(req protocol.Request) {
		if req.Command.Kind == "submit_turn" {
			submitted.Store(true)
		}
	}
	d.before.Store(&observe)
	d.click(i18n.In("zh-CN", "send"))
	d.await("healthy draft created and sent", func() bool {
		return submitted.Load() && len(d.controller.Snapshot().Sessions) == 2
	})
	if d.stored() == nil || d.account().BoundGroupID(core.LaneCodex) != nil {
		t.Fatal("sending required changing the account or rebinding the disabled lane")
	}
}
