package live

import (
	"fmt"
	"reflect"
	"slices"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/client"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/shell"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func TestReleasedTerminalSaveKeepsProposalRevision(t *testing.T) {
	for _, tc := range []struct {
		outcome  string
		revision uint64
		canRetry bool
		edited   bool
	}{
		{outcome: "not_admitted", revision: 3}, {outcome: "unknown", revision: 3},
		{outcome: "not_admitted", revision: 1, canRetry: true}, {outcome: "unknown", revision: 1},
		{outcome: "not_admitted", revision: 3, edited: true},
		{outcome: "completed", revision: 3},
	} {
		t.Run(fmt.Sprintf("%s/revision_%d/edited_%t", tc.outcome, tc.revision, tc.edited), func(t *testing.T) {
			transport := &conversationTestTransport{snapshot: protocol.DefaultSnapshot()}
			transport.snapshot.Node = protocol.NodeID{1}
			controller := client.New(transport)
			if err := controller.Start(t.Context()); err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { controller.Close() })
			r := &Runtime{ctx: t.Context(), controller: controller, App: shell.NewLive(&viewmodel.Data{Workspace: &viewmodel.Workspace{}})}
			r.terminalSettings.shown, r.terminalSettings.visible = r, true
			r.acceptTerminalSettings(protocol.TerminalSettings{Revision: 1}, nil)
			r.App.Settings.Host.Draft.Env = []viewmodel.EnvVar{{Key: "LOCAL", Value: "draft"}}
			var saves []protocol.Request
			var inspections []protocol.CommandInspectRequestPayload
			transport.handle = func(request protocol.Request) (link.Response, bool) {
				switch request.Command.Kind {
				case "save_terminal_settings":
					saves = append(saves, request)
					if len(saves) == 1 {
						return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown}}, true
					}
					value, _ := detail[protocol.TerminalSettings](request.Command.Data)
					value.Revision++
					return link.Response{Output: protocol.Output{Kind: "terminal_settings", Data: value}}, true
				case "inspect_request":
					input, _ := detail[protocol.CommandInspectRequestPayload](request.Command.Data)
					inspections = append(inspections, input)
					outcome := protocol.RequestOutcome{Kind: tc.outcome}
					if len(inspections) == 1 {
						outcome.Kind = "admitted"
					} else if tc.outcome == "completed" {
						applied, _ := detail[protocol.TerminalSettings](saves[0].Command.Data)
						applied.Revision++
						out := protocol.Output{Kind: "terminal_settings", Data: applied}
						outcome.Data = protocol.Result[protocol.Output, protocol.Fault]{Ok: &out}
					}
					return link.Response{Output: protocol.Output{Kind: "request_outcome", Data: protocol.OutputRequestOutcomePayload{ID: saves[0].ID, Outcome: outcome}}}, true
				}
				return link.Response{}, false
			}
			tester := ui.NewTester(func(c *ui.Context) {
				theme.Apply(c, "", theme.Light)
				queue := r.queue
				r.queue = nil
				for _, apply := range queue {
					apply(c)
				}
			}, 640, 480)
			save := func() {
				r.saveTerminalSettings()
				r.workers.Wait()
				tester.Frame()
			}
			save()
			if tc.revision > 1 {
				r.acceptTerminalRead(protocol.TerminalSettings{Revision: tc.revision, Environment: map[string]string{"OTHER": "newer"}}, nil)
			}
			if tc.edited {
				r.App.Settings.Host.Draft.Env[0].Value = "edited"
			}
			wantEnv := slices.Clone(r.App.Settings.Host.Draft.Env)
			save()
			if len(saves) != 1 || len(inspections) != 2 || r.terminalSettings.uncertain != nil {
				t.Fatal("the original save was not released by inspection alone")
			}
			if inspections[0].ID != saves[0].ID || inspections[0].Digest == "" || inspections[0] != inspections[1] {
				t.Fatalf("recovery did not inspect the original proposal: %+v", inspections)
			}
			if tc.outcome == "completed" {
				host := r.App.Settings.Host
				if host.Revision != tc.revision || !slices.Equal(host.Saved.Env, []viewmodel.EnvVar{{Key: "OTHER", Value: "newer"}}) ||
					!slices.Equal(host.Draft.Env, host.Saved.Env) {
					t.Fatalf("recovering an older save replaced newer settings: %+v", host)
				}
				return
			}
			save()
			if len(inspections) != 2 {
				t.Fatal("the released request was inspected again")
			}
			if tc.canRetry {
				if len(saves) != 2 || saves[0].ID == saves[1].ID || !reflect.DeepEqual(saves[0].Command, saves[1].Command) {
					t.Fatal("an explicitly retried absent save did not keep its original payload and revision")
				}
			} else if len(saves) != 1 || !slices.Equal(r.App.Settings.Host.Draft.Env, wantEnv) {
				t.Fatalf("the released proposal was rewritten or discarded: writes %d, draft %+v", len(saves), r.App.Settings.Host.Draft)
			}
		})
	}
}

func TestEditedTerminalDraftAfterUncertainSave(t *testing.T) {
	for _, tc := range []struct {
		name     string
		revision uint64
		conflict bool
	}{
		{"original revision", 1, false},
		{"newer controller revision", 3, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			transport := &conversationTestTransport{snapshot: protocol.DefaultSnapshot()}
			transport.snapshot.Node = protocol.NodeID{1}
			r := &Runtime{ctx: t.Context(), controller: client.New(transport)}
			t.Cleanup(func() { r.controller.Close() })
			original := protocol.TerminalSettings{Revision: 1, Shell: "/bin/first"}
			previous := r.controller.Prepare(protocol.Command{Kind: "save_terminal_settings", Data: original})
			applied := original
			applied.Revision++
			edited := protocol.TerminalSettings{Revision: tc.revision, Shell: "/bin/edited"}
			transport.handle = func(request protocol.Request) (link.Response, bool) {
				switch request.Command.Kind {
				case "inspect_request":
					result := protocol.Output{Kind: "terminal_settings", Data: applied}
					outcome := protocol.RequestOutcome{Kind: "completed", Data: protocol.Result[protocol.Output, protocol.Fault]{Ok: &result}}
					return link.Response{Output: protocol.Output{Kind: "request_outcome", Data: protocol.OutputRequestOutcomePayload{ID: previous.ID, Outcome: outcome}}}, true
				case "save_terminal_settings":
					value, _ := detail[protocol.TerminalSettings](request.Command.Data)
					if request.ID == previous.ID || value.Revision != applied.Revision || value.Shell != edited.Shell {
						t.Fatalf("new edit did not build on the confirmed save: %+v", request)
					}
					value.Revision++
					return link.Response{Output: protocol.Output{Kind: "terminal_settings", Data: value}}, true
				}
				return link.Response{}, false
			}
			_, retained, err := r.writeTerminalSettings(edited, &previous)
			if tc.conflict {
				if faultCode(err) != protocol.ErrorCodeRevisionConflict || retained != nil || len(transport.requests) != 1 || transport.requests[0].Command.Kind != "inspect_request" {
					t.Fatalf("the newer revision should conflict without a write: %v, requests %+v", err, transport.requests)
				}
			} else if err != nil || retained != nil || len(transport.requests) != 2 || transport.requests[0].Command.Kind != "inspect_request" || transport.requests[1].Command.Kind != "save_terminal_settings" {
				t.Fatalf("the edited draft should follow the confirmed write: %v, requests %+v", err, transport.requests)
			}
		})
	}
}
