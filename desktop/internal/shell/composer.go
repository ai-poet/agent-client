package shell

import (
	"context"
	"errors"
	"log"
	"strings"

	"github.com/ELY-Open-Source/CrychicUI/feedback"
	crychicinput "github.com/ELY-Open-Source/CrychicUI/input"
	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/ai-poet/agent-client/desktop/internal/workbench"
	"github.com/egoist/mygo/ui"
)

func (m *Model) composerView(c *ui.Context, snapshot Snapshot) {
	theme := c.Theme()
	ui.Column(c).Padding(12, 24, 16).Gap(8).BorderWidth(1, 0, 0, 0).BorderColor(theme.Border).Children(func() {
		switch {
		case snapshot.Sending:
			feedback.Busy(c, true, "发送中…", "发送中…")
		case snapshot.SendError != "":
			feedback.Alert(c, feedback.ToneDanger, "", snapshot.SendError).Role(ui.RoleAlert)
		}
		ui.Row(c).Gap(12).AlignItems(ui.End).Children(func() {
			c.Key("composer")
			composerInput := crychicinput.AutoTextArea(c, &m.state.Draft, crychicinput.AutoTextAreaOptions{
				MinLines: 3, MaxLines: 8, Placeholder: "输入消息，Enter 发送，Shift+Enter 换行",
			}).Label("消息输入").Grow(1).
				Disabled(snapshot.Sending || snapshot.SessionLoading || snapshot.SessionError != "")
			composerInput.OnTextKey(func(key ui.TextKey) bool {
				if key.Key != ui.KeyEnter || key.Mods != 0 {
					return false // Shift+Enter and other modified Enter keys insert a newline.
				}
				if m.sendText(key.Text()) {
					key.SetText("")
				}
				return true
			})
			canSend := !snapshot.Sending && !snapshot.SessionLoading && snapshot.SessionError == "" &&
				strings.TrimSpace(m.state.Draft) != ""
			if ui.PrimaryButton(c.Key("send"), "发送").Disabled(!canSend).Clicked() {
				m.Send()
			}
		})
	})
}

// Send submits the current draft asynchronously. The input is cleared and
// disabled while the RPC is in flight; a failure restores the exact draft.
func (m *Model) Send() {
	m.sendText(m.state.Draft)
}

func (m *Model) sendText(text string) bool {
	if m.closed || m.connection == nil {
		return false
	}
	action, ok := m.state.BeginSend(text)
	if !ok {
		return false
	}
	ctx, cancel := context.WithCancel(context.Background())
	m.sendCancel = cancel
	connection := m.connection
	m.invalidate()
	go m.send(ctx, action, connection)
	return true
}

func (m *Model) send(ctx context.Context, action workbench.SendAction, connection daemon.Connection) {
	runtimeID := action.RuntimeID
	if runtimeID == "" {
		runtime, err := connection.AttachSession(ctx, action.SessionID)
		if err != nil {
			m.finishSend(ctx, action, err)
			return
		}
		if runtime != nil {
			runtimeID = runtime.RuntimeID
		}
		if ctx.Err() != nil {
			return
		}
		if runtimeID != "" {
			m.enqueue(func() {
				if ctx.Err() == nil {
					m.state.ApplyAttach(workbench.AttachResult{
						Generation: action.Generation, SessionGeneration: action.SessionGeneration,
						SessionID: action.SessionID, Runtime: runtime,
					})
				}
			})
		}
	}
	if runtimeID == "" {
		runtimeID = daemon.NilUUID
	}
	err := connection.Prompt(ctx, action.SessionID, runtimeID, action.Text)
	m.finishSend(ctx, action, err)
}

func (m *Model) finishSend(ctx context.Context, action workbench.SendAction, err error) {
	if err != nil {
		log.Printf("WARN desktop: send failed; category=%s", sendErrorCategory(err))
	}
	m.enqueue(func() {
		if ctx.Err() != nil {
			return
		}
		if m.state.FinishSend(workbench.SendResult{
			Generation: action.Generation, SessionGeneration: action.SessionGeneration,
			SessionID: action.SessionID, Text: action.Text, Err: err,
		}) {
			m.sendCancel = nil
		}
	})
}

func sendErrorCategory(err error) string {
	if errors.Is(err, context.DeadlineExceeded) {
		return "timeout"
	}
	var rpcError *daemon.RPCError
	if errors.As(err, &rpcError) {
		return "rejected"
	}
	return "other"
}
