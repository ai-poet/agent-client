package node

import (
	"context"
	"database/sql"

	"github.com/ai-poet/agent-client/desktop/internal/link"
	"github.com/ai-poet/agent-client/desktop/internal/node/admission"
	"github.com/ai-poet/agent-client/desktop/internal/node/host"

	"github.com/ai-poet/agent-client/desktop/internal/node/wsl"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

func (node *Node) initHost() {
	node.hostSampler = host.NewSampler()
	node.wsl = wsl.NewService()
}

func hostCommand(kind string) bool {
	switch kind {
	case "inspect_host", "read_host_metrics", "read_plugin_memory", "stop_host_process":
		return true
	}
	return wslCommand(kind)
}

// dispatchResource executes WSL setup outside the store writer.
func (node *Node) dispatchResource(ctx context.Context, caller protocol.NodeID, request protocol.Request) (bool, protocol.Output, error) {
	return node.dispatchWSL(ctx, node.wsl, host.ConfigureNetwork, caller, request)
}

func (node *Node) runHost(ctx context.Context, caller protocol.NodeID, request protocol.Request) (protocol.Output, error) {
	if handled, output, err := node.dispatchResource(ctx, caller, request); handled {
		return output, err
	}
	input := request.Command.Data.(*protocol.CommandStopHostProcessPayload)
	err := host.StopProcess(ctx, input.Pid, input.StartedAtSecs, input.Force)
	return protocol.Output{Kind: "host_process_signalled", Data: protocol.OutputHostProcessSignalledPayload{Pid: input.Pid, StartedAtSecs: input.StartedAtSecs}}, err
}

func (node *Node) dispatchHost(ctx context.Context, caller protocol.NodeID, request protocol.Request, body []byte) (link.Admission, error) {
	// Plugin scopes never reach Node inspection or process signals; those stay
	// with controllers.
	if request.Plugin != nil && (request.Command.Kind == "inspect_host" || request.Command.Kind == "read_host_metrics" || request.Command.Kind == "read_plugin_memory" || request.Command.Kind == "stop_host_process") {
		return link.Admission{}, &protocol.Fault{Code: protocol.ErrorCodePermissionDenied, Message: "plugin invocation authority is unavailable"}
	}
	if err := node.begin(); err != nil {
		return link.Admission{}, err
	}
	completion := make(chan link.Response, 1)
	accepted := link.Admission{Receipt: protocol.Receipt{ID: request.ID, Durable: request.Command.Durable()}, Completion: completion}
	if !accepted.Receipt.Durable {
		go func() {
			defer node.work.Done()
			defer close(completion)
			completion <- node.readHost(ctx, caller, request)
		}()
		return accepted, nil
	}
	var previous *link.Response
	err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		var err error
		previous, err = admission.Admit(ctx, db, caller, request, body)
		return err
	})
	if err != nil {
		node.work.Done()
		return link.Admission{}, nodeFault(err)
	}
	if previous != nil {
		completion <- *previous
		close(completion)
		node.work.Done()
		return accepted, nil
	}
	go func() {
		defer node.work.Done()
		defer close(completion)
		owner := node.commandContext(commandJob{user: userFromContext(ctx), scope: ctx})
		completion <- node.executeHost(owner, caller, request)
	}()
	return accepted, nil
}

func (node *Node) readHost(ctx context.Context, caller protocol.NodeID, request protocol.Request) link.Response {
	if handled, output, err := node.dispatchResource(ctx, caller, request); handled {
		return response(output, err)
	}
	switch request.Command.Kind {
	case "inspect_host":
		value, err := node.hostSampler.Inspect(ctx, node.ID())
		return response(protocol.Output{Kind: "host_info", Data: value}, err)
	case "read_host_metrics":
		input := request.Command.Data.(*protocol.CommandReadHostMetricsPayload)
		value, err := node.hostSampler.Sample(ctx, node.ID(), input.Detailed)
		return response(protocol.Output{Kind: "host_metrics", Data: value}, err)
	case "read_plugin_memory":
		value, err := node.readPluginMemory(ctx)
		return response(protocol.Output{Kind: "plugin_memory", Data: value}, err)

	}
	return response(protocol.Output{}, &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "Host read expected"})
}

func (node *Node) executeHost(ctx context.Context, caller protocol.NodeID, request protocol.Request) link.Response {
	operationErr := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		return node.authorizeCommandDB(ctx, db, caller, request)
	})
	var output protocol.Output
	if operationErr == nil {
		output, operationErr = node.runHost(ctx, caller, request)
	}
	err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		return admission.Complete(ctx, db, caller, request.ID, response(output, operationErr))
	})
	if err != nil {
		return node.settle(caller, request.ID, response(output, operationErr))
	}
	return response(output, operationErr)
}
