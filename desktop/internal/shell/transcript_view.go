package shell

import (
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

func (m *Model) transcriptView(c *ui.Context, snapshot Snapshot) {
	theme := c.Theme()
	switch {
	case snapshot.SessionLoading:
		ui.Column(c).Grow(1).Center().Children(func() {
			ui.Text(c, "载入会话…").TextColor(theme.TextMuted).Role(ui.RoleStatus)
		})
		return
	case snapshot.SessionError != "":
		ui.Column(c).Grow(1).Center().Gap(12).Children(func() {
			ui.Text(c, "载入失败，可重试").TextColor(theme.Danger).Role(ui.RoleAlert)
			ui.Text(c, snapshot.SessionError).TextColor(theme.TextMuted)
			if ui.Button(c.Key("retry-session"), "重试").Clicked() {
				m.SelectSession(snapshot.SelectedSessionID)
			}
		})
		return
	case len(snapshot.Transcript) == 0:
		ui.Column(c).Grow(1).Center().Children(func() {
			ui.Text(c, "这个会话还没有消息").TextColor(theme.TextMuted)
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
			m.transcriptMarkdown(c, entry.Text, entry.Streaming)
		}
		if entry.Detail != "" {
			m.transcriptMarkdown(c, entry.Detail, false)
		}
	})
}

func (m *Model) transcriptMarkdown(c *ui.Context, text string, streaming bool) {
	if streaming {
		markdown.Stream(c, text, &m.markdown)
		return
	}
	markdown.View(c, text, &m.markdown)
}
