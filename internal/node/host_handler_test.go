package node

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/plugins"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestScheduledHostHandlerForwardPreservesPrincipal(t *testing.T) {
	node := &Node{}
	target := protocol.NodeID{2}
	local := protocol.User{ID: protocol.NewUserID(), Local: true}
	guest := protocol.User{ID: protocol.NewUserID(), Controller: &target}
	localCtx, guestCtx := store.WithUser(t.Context(), local), store.WithUser(t.Context(), guest)
	request := protocol.NewRequest(target, protocol.Command{Kind: "call_host_handler"})
	for _, test := range []struct {
		ctx      context.Context
		outbound bool
		code     protocol.ErrorCode
	}{
		{t.Context(), true, protocol.ErrorCodePermissionDenied},
		{guestCtx, true, protocol.ErrorCodePermissionDenied},
		{store.WithCapturedUser(localCtx, guest.ID), true, protocol.ErrorCodePermissionDenied},
		{store.WithCapturedUser(guestCtx, local.ID), false, protocol.ErrorCodePermissionDenied},
		// The authorized routes reach the (offline) peer lookup.
		{localCtx, true, protocol.ErrorCodeUnavailable},
		{guestCtx, false, protocol.ErrorCodeUnavailable},
	} {
		for _, call := range []protocol.ResourceCall{
			{Request: request}, {Request: protocol.NewRequest(target, protocol.Command{Kind: "inspect_request"}), Original: &request},
		} {
			_, err := node.sendResource(test.ctx, target, call, test.outbound)
			var fault *protocol.Fault
			if !errors.As(err, &fault) || fault.Code != test.code {
				t.Fatalf("%s outbound=%v: %v, want %s", call.Request.Command.Kind, test.outbound, err, test.code)
			}
		}
	}
}

func TestHostHandlerUsesCapturedPackageAndOriginalReceipt(t *testing.T) {
	node := nodeFixture(t)
	var info protocol.PluginInfo
	if err := node.profile.Store.Node.Read(t.Context(), func(db *sql.Conn) error {
		value, err := store.ReadPlugin(t.Context(), db, "reminders")
		if err == nil && value != nil {
			info = *value
		}
		return err
	}); err != nil || info.Extension == nil {
		t.Fatalf("bundled reminders unavailable: %v", err)
	}
	scope := protocol.DefaultPluginContext()
	scope.Package = plugins.PackageReference(info)
	id := protocol.NewRequestID().String()
	request := protocol.NewRequest(node.ID(), protocol.Command{Kind: "call_host_handler", Data: protocol.HostHandlerCall{
		Handler: "save", Input: map[string]any{
			"id": id, "project": nil, "title": "Captured host fixture", "message": "",
			"revision": "0", "due_ms": nil, "completed": false,
		},
	}})
	request.Plugin = &scope
	first := dispatchFixture(t, node, node.ID(), request)
	if first.Fault != nil {
		t.Fatal(first.Fault)
	}
	var result struct {
		Ok struct {
			Revision string `json:"revision"`
		} `json:"Ok"`
		Err *protocol.Fault `json:"Err"`
	}
	body, _ := json.Marshal(first.Output.Data)
	if err := json.Unmarshal(body, &result); err != nil || result.Err != nil || result.Ok.Revision != "1" {
		t.Fatalf("host handler failed: %s %v", body, err)
	}
	second := dispatchFixture(t, node, node.ID(), request)
	replayed, _ := json.Marshal(second.Output)
	original, _ := json.Marshal(first.Output)
	if second.Fault != nil || string(replayed) != string(original) {
		t.Fatalf("host handler replay changed result: %+v", second)
	}
}

func TestHostHandlerRejectsUndeclaredAndMismatchedScope(t *testing.T) {
	reference := protocol.PluginReference{Name: "fixture", Digest: strings.Repeat("a", 64)}
	scope := protocol.DefaultPluginContext()
	scope.Package = reference
	info := protocol.PluginInfo{}
	info.Summary.Name, info.Summary.Digest = reference.Name, reference.Digest
	request := protocol.NewRequest(protocol.NodeID{1}, protocol.Command{
		Kind: "call_host_handler", Data: protocol.HostHandlerCall{Handler: "undeclared", Input: nil},
	})
	request.Plugin = &scope
	for _, capture := range []*protocol.ResourceScope{nil, {Plugin: &info, Origin: &scope}} {
		if err := checkHostHandlerScope(request, capture); err == nil {
			t.Fatal("undeclared host callback was accepted")
		}
	}
}
