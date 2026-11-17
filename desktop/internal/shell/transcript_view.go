package shell

import (
	"time"

	"github.com/ELY-Open-Source/CrychicUI/chat"
	"github.com/ELY-Open-Source/CrychicUI/feedback"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

func (m *Model) transcriptView(c *ui.Context, snapshot Snapshot) {
	theme := c.Theme()
	switch {
	case snapshot.SessionLoading:
		ui.Column(c).Grow(1).Center().Gap(12).Children(func() {
			ui.Text(c, "载入会话…").TextColor(theme.TextMuted).Role(ui.RoleStatus)
			feedback.IndeterminateBar(c, "正在载入会话").Width(240)
		})
		return
	case snapshot.SessionError != "":
		ui.Column(c).Grow(1).Center().Gap(12).Padding(24).Children(func() {
			feedback.Alert(c, feedback.ToneDanger, "载入失败，可重试", snapshot.SessionError).Role(ui.RoleAlert)
			if ui.Button(c.Key("retry-session"), "重试").Clicked() {
				m.SelectSession(snapshot.SelectedSessionID)
			}
		})
		return
	case len(snapshot.Transcript) == 0:
		ui.Column(c).Grow(1).Center().Children(func() {
			feedback.EmptyState(c, "message-circle", "这个会话还没有消息", "", "")
		})
		return
	}

	// List keys keep each Markdown document and its incremental parser attached
	// to the same entry. FollowEnd follows additions and growing rows only while
	// the user is at the end; scrolling away stops follow until they return.
	if m.transcriptSessionID != snapshot.SelectedSessionID {
		m.transcriptList = ui.ListState{}
		m.transcriptSessionID = snapshot.SelectedSessionID
	}
	m.transcriptList.Key = func(index int) any { return snapshot.Transcript[index].ID }
	m.transcriptList.FollowEnd = true
	ui.List(c, &m.transcriptList, len(snapshot.Transcript), func(index int) {
		m.transcriptEntryView(c, snapshot.Transcript[index])
	}).Grow(1).Padding(16, 24).Gap(18).Justify(ui.End).Label("会话转录")
}

func (m *Model) transcriptEntryView(c *ui.Context, entry TranscriptEntry) {
	theme := c.Theme()
	ui.Column(c).Gap(6).Children(func() {
		ui.Row(c).Gap(8).Children(func() {
			ui.Text(c, entry.Title).FontSize(12).Bold().TextColor(theme.TextMuted)
			switch {
			case entry.Failed:
				ui.Text(c, "失败").FontSize(12).TextColor(theme.Danger)
			case entry.Streaming:
				ui.Text(c, "正在生成").FontSize(12).TextColor(theme.TextMuted).Role(ui.RoleStatus)
			case entry.Kind == TranscriptActivity && !entry.Complete:
				ui.Text(c, "进行中").FontSize(12).TextColor(theme.TextMuted).Role(ui.RoleStatus)
			}
		})
		if entry.Text != "" {
			m.transcriptMarkdown(c, entry, entry.Text, entry.Streaming)
		}
		if entry.Detail != "" {
			m.transcriptMarkdown(c, entry, entry.Detail, false)
		}
	})
}

func (m *Model) transcriptMarkdown(c *ui.Context, entry TranscriptEntry, text string, streaming bool) {
	if streaming {
		markdown.Stream(c, text, &m.markdown)
		return
	}
	chat.MarkdownMessage(c, chat.Message{
		Sender: entry.Title,
		Text:   text,
		Mine:   entry.Kind == TranscriptUser,
		At:     time.Unix(0, 0),
	})
}
