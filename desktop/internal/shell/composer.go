package shell

import (
	"context"
	"errors"
	"fmt"
	"log"
	"strings"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/egoist/mygo/ui"
)

func (m *Model) composerView(c *ui.Context, snapshot Snapshot) {
	theme := c.Theme()
	ui.Column(c).Padding(12, 24, 16).Gap(8).BorderWidth(1, 0, 0, 0).BorderColor(theme.Border).Children(func() {
		switch {
		case snapshot.Sending:
			ui.Text(c, "发送中…").FontSize(12).TextColor(theme.TextMuted).Role(ui.RoleStatus)
		case snapshot.SendError != "":
			ui.Text(c, snapshot.SendError).FontSize(12).TextColor(theme.Danger).Role(ui.RoleAlert)
		}
		ui.Row(c).Gap(12).AlignItems(ui.End).Children(func() {
			input := ui.TextArea(c.Key("composer"), &m.draft).
				Placeholder("输入消息，Enter 发送，Shift+Enter 换行").
				Label("消息输入").Lines(3, 8).Grow(1).
				Disabled(snapshot.Sending || snapshot.SessionLoading || snapshot.SessionError != "")
			input.OnTextKey(func(key ui.TextKey) bool {
				if key.Key != ui.KeyEnter || key.Mods != 0 {
					return false // Shift+Enter and other modified Enter keys insert a newline.
				}
				if m.sendText(key.Text()) {
					key.SetText("")
				}
				return true
			})
			canSend := !snapshot.Sending && !snapshot.SessionLoading && snapshot.SessionError == "" &&
				strings.TrimSpace(m.draft) != ""
			if ui.PrimaryButton(c.Key("send"), "发送").Disabled(!canSend).Clicked() {
				m.Send()
			}
		})
	})
}

// Send submits the current draft asynchronously. The input is cleared and
// disabled while the RPC is in flight; a failure restores the exact draft.
func (m *Model) Send() {
	m.sendText(m.draft)
}

func (m *Model) sendText(text string) bool {
	if strings.TrimSpace(text) == "" {
		return false
	}

	sessionID := m.snapshot.SelectedSessionID
	if m.closed || m.snapshot.Status != Connected || m.connection == nil || m.snapshot.Sending ||
		sessionID == "" || m.hydratedGeneration != m.sessionGeneration || m.hydratedSessionID != sessionID {
		return false
	}
	ctx, cancel := context.WithCancel(context.Background())
	m.sendCancel = cancel
	m.sendingText = text
	m.draft = ""
	m.setSessionDraft(sessionID, "")
	m.snapshot.Draft = ""
	m.snapshot.Sending = true
	m.snapshot.SendError = ""
	generation, sessionGeneration := m.generation, m.sessionGeneration
	connection, runtimeID := m.connection, m.attachedRuntimeID
	m.invalidate()
	go m.send(ctx, generation, sessionGeneration, connection, sessionID, runtimeID, text)
	return true
}

func (m *Model) send(ctx context.Context, generation, sessionGeneration uint64, connection daemon.Connection, sessionID, runtimeID, text string) {
	if runtimeID == "" {
		runtime, err := connection.AttachSession(ctx, sessionID)
		if err != nil {
			m.finishSend(ctx, generation, sessionGeneration, sessionID, text, err)
			return
		}
		if runtime != nil {
			runtimeID = runtime.RuntimeID
		}
		if ctx.Err() != nil {
			return
		}
		m.enqueue(func() {
			if m.sessionCurrent(generation, sessionGeneration, sessionID) && ctx.Err() == nil && runtimeID != "" {
				m.attachedRuntimeID = runtimeID
			}
		})
	}
	if runtimeID == "" {
		runtimeID = daemon.NilUUID
	}
	err := connection.Prompt(ctx, sessionID, runtimeID, text)
	m.finishSend(ctx, generation, sessionGeneration, sessionID, text, err)
}

func (m *Model) finishSend(ctx context.Context, generation, sessionGeneration uint64, sessionID, text string, err error) {
	if err != nil {
		log.Printf("WARN desktop: send failed; category=%s", sendErrorCategory(err))
	}
	m.enqueue(func() {
		if !m.sessionCurrent(generation, sessionGeneration, sessionID) || ctx.Err() != nil {
			return
		}
		m.sendCancel = nil
		m.sendingText = ""
		m.snapshot.Sending = false
		if err != nil {
			m.draft = text
			m.setSessionDraft(sessionID, text)
			m.snapshot.Draft = text
			m.snapshot.SendError = sendErrorMessage(err)
		} else {
			m.snapshot.SendError = ""
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

func sendErrorMessage(err error) string {
	detail := strings.Join(strings.Fields(err.Error()), " ")
	if detail == "" {
		detail = "未知错误"
	}
	const detailLimit = 240
	runes := []rune(detail)
	if len(runes) > detailLimit {
		detail = string(runes[:detailLimit]) + "…"
	}
	return fmt.Sprintf("发送失败，已保留输入：%s", detail)
}
