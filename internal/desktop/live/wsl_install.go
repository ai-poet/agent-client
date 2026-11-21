package live

import (
	"errors"
	"time"

	"github.com/egoist/mygo/ui"
	"github.com/ai-poet/agent-client/internal/client"
	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/kit"
	"github.com/ai-poet/agent-client/internal/desktop/settings"
	"github.com/ai-poet/agent-client/internal/protocol"
)

type wslInstallation struct {
	state      *liveHost
	request    *protocol.Request
	completed  func(bool)
	active     bool
	running    bool
	uncertain  bool
	stage      string
	polling    bool
	nextPoll   time.Time
	networkFix *protocol.HostInstallationOutcomeNetworkUnconfirmedPayload
}

func (r *Runtime) addWSLHost(draft settings.ConnectionDraft, completed func(bool)) {
	root := r.localRuntime()
	local := root.hostState(root.controller.Target().String())
	if local == nil {
		root.initHosts()
		local = root.hostState(root.controller.Target().String())
	}
	if local.install != nil && (local.install.running || local.install.uncertain) {
		completed(false)
		return
	}
	selection, err := root.connectionSelection(draft)
	if err != nil {
		root.enqueue(func(c *ui.Context) {
			root.reportConnection(c, err, "pairing_relay_invalid")
			completed(false)
		})
		return
	}
	request := root.controller.Prepare(protocol.Command{Kind: "install_wsl_host", Data: protocol.CommandInstallWslHostPayload{Distribution: draft.Distribution, Network: &selection}})
	operation := &wslInstallation{state: local, request: &request, completed: completed, active: true, stage: "connecting"}
	local.install = operation
	root.App.Settings.ConnectionPending = true
	root.executeWSLHost(operation)
}

func (r *Runtime) executeWSLHost(operation *wslInstallation) {
	request := *operation.request
	operation.running, operation.uncertain = true, false
	r.async(func() {
		output, err := r.controller.Execute(r.ctx, request)
		r.enqueue(func(c *ui.Context) {
			operation.running = false
			if operation.active {
				r.completeWSLHost(c, operation, output, err)
			} else if operation.state.install == operation {
				operation.state.install = nil
			}
		})
	})
}

func (r *Runtime) completeWSLHost(c *ui.Context, operation *wslInstallation, output protocol.Output, err error) {
	if err == nil {
		outcome, valid := detail[protocol.HostInstallationOutcome](output.Data)
		if valid && output.Kind == "host_installation" {
			switch outcome.Kind {
			case "installed":
				operation.request, operation.networkFix = nil, nil
				operation.active, operation.uncertain = false, false
				r.App.Settings.ConnectionPending = false
				kit.Toast(c, kit.Done, i18n.T(c, "host_added"))
				operation.completed(true)
				return
			case "network_unconfirmed":
				if network, ok := detail[protocol.HostInstallationOutcomeNetworkUnconfirmedPayload](outcome.Data); ok {
					operation.networkFix = &network
					err = unknownConnectionNetwork()
				}
			}
		}
		if err == nil {
			err = errors.New("Host installation response is invalid")
		}
	}
	operation.active = false
	operation.uncertain = faultCode(err) == protocol.ErrorCodeOutcomeUnknown || operation.networkFix != nil
	if !operation.uncertain {
		operation.request = nil
	}
	r.App.Settings.ConnectionPending = false
	key := "host_install_failed"
	if operation.uncertain {
		key = "host_install_unknown"
	}
	r.App.Settings.ConnectionFailure = &settings.ConnectionFailure{Tone: kit.Negative, Title: key, Detail: err.Error()}
	operation.completed(false)
}

func (r *Runtime) checkWSLHost(completed func(bool)) {
	local := r.hostState(r.controller.Target().String())
	if local == nil || local.install == nil || local.install.running {
		completed(false)
		return
	}
	operation := local.install
	operation.completed, operation.active, operation.running = completed, true, true
	r.App.Settings.ConnectionPending = true
	r.async(func() {
		if fix := operation.networkFix; fix != nil {
			err := r.confirmConnectionNetwork(r.ctx, fix.Node, fix.Selection)
			r.enqueue(func(c *ui.Context) {
				operation.running = false
				if operation.active {
					r.completeWSLHost(c, operation, protocol.Output{Kind: "host_installation", Data: protocol.HostInstallationOutcome{Kind: "installed", Data: protocol.HostInstallationOutcomeInstalledPayload{Node: fix.Node}}}, err)
				}
			})
			return
		}
		resolution, err := r.controller.Resolve(r.ctx, *operation.request)
		r.enqueue(func(c *ui.Context) {
			operation.running = false
			if !operation.active {
				return
			}
			if err != nil {
				r.completeWSLHost(c, operation, protocol.Output{}, err)
				return
			}
			switch resolution.State {
			case client.RequestApplied:
				r.completeWSLHost(c, operation, resolution.Output, nil)
			case client.RequestFailed:
				r.completeWSLHost(c, operation, protocol.Output{}, resolution.Fault)
			case client.RequestAbsent:
				// Retry only a request the Node never admitted, with its original ID.
				r.executeWSLHost(operation)
			case client.RequestPending, client.RequestUnknown:
				r.completeWSLHost(c, operation, protocol.Output{}, &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "Host installation must be checked on the target"})
			}
		})
	})
}

func (r *Runtime) wslInstallFrame(c *ui.Context, operation *wslInstallation) {
	if !r.App.Settings.Adding() {
		if operation.active && operation.request != nil {
			request := operation.request.ID
			r.async(func() {
				_, _ = r.controller.Do(r.ctx, protocol.Command{Kind: "cancel_wsl_host_install", Data: protocol.CommandCancelWslHostInstallPayload{Request: request}})
			})
		}
		operation.active = false
		r.App.Settings.ConnectionPending = false
		if !operation.running && operation.state.install == operation {
			operation.state.install = nil
		}
		return
	}
	if !operation.active || !operation.running || operation.request == nil {
		return
	}
	if !operation.polling && !c.Now().Before(operation.nextPoll) {
		request := operation.request.ID
		operation.polling = true
		r.async(func() {
			output, err := r.controller.Do(r.ctx, protocol.Command{Kind: "read_wsl_host_install", Data: protocol.CommandReadWslHostInstallPayload{Request: request}})
			r.enqueue(func(c *ui.Context) {
				operation.polling, operation.nextPoll = false, c.Now().Add(300*time.Millisecond)
				if operation.active && err == nil {
					if progress, ok := detail[protocol.SshInstallProgress](output.Data); ok {
						operation.stage = progress.Kind
					}
				}
			})
		})
	}
	c.After(300 * time.Millisecond)
}

func (r *Runtime) wslInstallProgress() string {
	local := r.hostState(r.controller.Target().String())
	if local != nil && local.install != nil && local.install.active {
		switch local.install.stage {
		case "detecting":
			return "host_install_detecting"
		case "installing":
			return "host_install_installing"
		case "pairing":
			return "host_install_pairing"
		}
		return "connection_connecting"
	}
	return ""
}
