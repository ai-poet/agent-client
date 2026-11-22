package node

import (
	"context"
	"database/sql"

	"log/slog"
	"runtime"

	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/node/terminal"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func terminalCommand(kind string) bool {
	switch kind {
	case "create_terminal", "open_tool_terminal", "open_terminal", "list_terminal_tools", "close_terminal", "list_terminals", "inspect_terminal", "claim_terminal", "takeover_terminal", "input_terminal", "resize_terminal", "set_terminal_appearance", "read_terminal_settings", "save_terminal_settings", "list_shells":
		return true
	}
	return false
}

func (node *Node) initTerminals(ctx context.Context) error {
	if err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error { return store.RecoverTerminals(ctx, db) }); err != nil {
		return err
	}
	node.terminals = terminal.New(node.ctx, node.ID(), node.terminalChangedBy)
	return nil
}

func (node *Node) terminalChanged(info protocol.TerminalInfo) { node.terminalChangedBy(node.ctx, info) }

func (node *Node) terminalChangedBy(ctx context.Context, info protocol.TerminalInfo) {
	if err := node.begin(); err != nil {
		return
	}
	defer node.work.Done()
	err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		changes, err := store.SaveTerminal(ctx, db, info, true)
		if err == nil {
			_, err = store.RecordEvents(ctx, db, node.ID(), changes)
		}
		return err
	})
	if err == nil {
		node.publish()
	} else if ctx.Err() == nil {
		slog.Error("terminal inventory update failed", "error", err)
	}
}

func (node *Node) terminalIO(ctx context.Context, caller protocol.NodeID, request protocol.Request, effect *workspaceCompletion) (protocol.Output, error) {
	service := node.terminals
	if service == nil {
		return protocol.Output{}, unavailable()
	}
	switch request.Command.Kind {
	case "read_terminal_settings":
		settings, err := node.terminalSettings(ctx)
		return protocol.Output{Kind: "terminal_settings", Data: settings}, err
	case "list_shells":
		var distributions []protocol.WslDistribution
		if runtime.GOOS == "windows" {
			var err error
			if distributions, err = node.wsl.List(ctx, node.environment.TerminalValues()); err != nil {
				return protocol.Output{}, err
			}
		}
		return protocol.Output{Kind: "shells", Data: terminal.Shells(node.environment.TerminalValues(), distributions)}, nil
	}
	var info protocol.TerminalInfo
	var err error
	switch input := request.Command.Data.(type) {
	case *protocol.TerminalSettings:
		effect.commit = func(ctx context.Context, db *sql.Conn, _ protocol.Output) (protocol.Output, []protocol.Event, error) {
			saved, changes, err := store.SaveTerminalSettings(ctx, db, *input)
			return protocol.Output{Kind: "terminal_settings", Data: saved}, changes, err
		}
		return protocol.Output{}, nil
	case *protocol.TerminalLaunch:
		if info, err = node.startTerminal(ctx, terminal.Launch{ID: protocol.TerminalID(request.ID), Caller: caller, Worktree: input.Worktree, Access: input.Access, Viewport: input.Viewport, Appearance: input.Appearance}); err != nil {
			return protocol.Output{}, err
		}
	case *protocol.CommandListTerminalToolsPayload:
		root, err := node.workspaceRoot(ctx, input.Worktree, false)
		if err != nil {
			return protocol.Output{}, err
		}
		settings, err := node.terminalSettings(ctx)
		if err != nil {
			return protocol.Output{}, err
		}
		return protocol.Output{Kind: "terminal_tools", Data: service.Tools(root, terminal.Environment(node.environment.TerminalValues(), settings))}, nil

	case *protocol.CommandListTerminalsPayload:
		var values []protocol.TerminalInfo
		err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
			snapshot, err := store.Snapshot(ctx, db, node.ID())
			values = []protocol.TerminalInfo{}
			for _, value := range snapshot.Terminals {
				if value.Worktree != nil && *value.Worktree == input.Worktree {
					values = append(values, value)
				}
			}
			return err
		})
		return protocol.Output{Kind: "terminals", Data: values}, err
	case *protocol.CommandOpenToolTerminalPayload:
		if info, err = node.startTerminal(ctx, terminal.Launch{ID: protocol.TerminalID(request.ID), Caller: caller, Worktree: input.Launch.Worktree, Tool: &input.Tool, Access: input.Launch.Access, Viewport: input.Launch.Viewport, Appearance: input.Launch.Appearance}); err != nil {
			return protocol.Output{}, err
		}
	case *protocol.CommandOpenTerminalPayload:
		info, err = service.Info(input.Terminal)
	case *protocol.CommandInspectTerminalPayload:
		snapshot, err := service.Snapshot(input.Terminal)
		return protocol.Output{Kind: "terminal_snapshot", Data: snapshot}, err
	case *protocol.CommandClaimTerminalPayload:
		info, err = service.Claim(caller, input.Terminal, input.ExpectedRevision)
	case *protocol.CommandTakeoverTerminalPayload:
		info, err = service.Takeover(caller, input.Terminal, input.ExpectedRevision)
	case *protocol.CommandInputTerminalPayload:
		info, err = service.Input(caller, *input)
		return protocol.Output{Kind: "terminal_input", Data: protocol.OutputTerminalInputPayload{Terminal: info.ID}}, err
	case *protocol.CommandResizeTerminalPayload:
		info, err = service.Resize(caller, *input)
	case *protocol.CommandSetTerminalAppearancePayload:
		info, err = service.Appearance(caller, *input)
	case *protocol.CommandCloseTerminalPayload:
		info, err = service.CloseTerminal(&input.Worktree, input.Terminal)
	default:
		return protocol.Output{}, &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "terminal command is invalid"}
	}
	if err != nil {
		return protocol.Output{}, err
	}
	// The terminal was started, changed or closed already: a row that
	// could not be stored leaves an outcome to inspect, not an unapplied
	// request.
	effect.external = true
	effect.commit = func(ctx context.Context, db *sql.Conn, output protocol.Output) (protocol.Output, []protocol.Event, error) {
		changes, err := store.SaveTerminal(ctx, db, info, false)
		return output, changes, err
	}
	return protocol.Output{Kind: "terminal", Data: info}, nil
}

func (node *Node) terminalSettings(ctx context.Context) (protocol.TerminalSettings, error) {
	var settings protocol.TerminalSettings
	err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		var err error
		settings, err = store.ReadTerminalSettings(ctx, db)
		return err
	})
	return settings, err
}

// startTerminal starts a local terminal in its worktree with the Node's
// terminal environment and the saved shell and variables.
func (node *Node) startTerminal(ctx context.Context, launch terminal.Launch) (protocol.TerminalInfo, error) {
	root, err := node.workspaceRoot(ctx, launch.Worktree, false)
	if err != nil {
		return protocol.TerminalInfo{}, err
	}
	if launch.Settings, err = node.terminalSettings(ctx); err != nil {
		return protocol.TerminalInfo{}, err
	}
	launch.Root, launch.Environment, launch.Sandbox = root, node.environment.TerminalValues(), node.processes.SandboxTerminal
	return node.terminals.Start(ctx, launch)
}

func (node *Node) subscribeTerminal(topic protocol.Topic) (link.Subscription, error) {
	switch id := topic.Data.(type) {
	case protocol.TerminalID:
		return node.terminals.Subscribe(id)
	case *protocol.TerminalID:
		if id != nil {
			return node.terminals.Subscribe(*id)
		}
	}
	return nil, &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "terminal subscription requires a terminal"}
}
