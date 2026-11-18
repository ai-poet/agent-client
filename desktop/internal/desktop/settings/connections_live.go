package settings

import (
	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/egoist/mygo/ui"
)

type ConnectionDraft struct {
	Method                                    Method
	Relay                                     int
	Address, Code string
	Distribution                              string // WSL
}

type ConnectionActions struct {
	// Inspect opens a device monitor; it does not select a configuration Node.
	Inspect     func(viewmodel.Device)
	Add         func(ConnectionDraft, func(bool))
	CheckStatus func(func(bool))
	Revoke      func(viewmodel.Device)
	Narrow      func(viewmodel.Device, string)
	EditNetwork func(viewmodel.Device)
	SaveNetwork func(*NetworkEditor)
	RetryShare  func()
}

type ShareState struct {
	Kind, Code string
}

type RelayChoice struct {
	Label, LabelKey string
	Custom, Pending bool
	Millis          *uint64
}

type NetworkEditor struct {
	Device      viewmodel.Device
	Revision    uint64
	Relay       int
	Address     string
	Relays      []RelayChoice
	Loading     bool
	Pending     bool
	Locked      bool
	Retry       bool
	Unavailable bool
}

func (m *Model) ConnectionDraft() ConnectionDraft {
	value := m.pairing
	return ConnectionDraft{Method: Method(value.method), Relay: value.relay, Address: value.address, Code: value.code, Distribution: value.distributionName()}
}

func (m *Model) Sharing() bool { return m.pairing.sharing }

func (m *Model) Adding() bool { return m.pairing.adding }

func (m *Model) CloseShare() { m.pairing.sharing = false }

func (m *Model) CloseConnectionDialogs() {
	m.pairing.adding, m.pairing.sharing = false, false
	m.NetworkEditor = nil
	m.FollowEditor = nil
}

func (m *Model) SetConnectionNetwork(relay int, address string) {
	m.pairing.relay, m.pairing.address = relay, address
}

func connectionDialogs(c *ui.Context, m *Model) {
	configurationFollowDialog(c, m)
	if editor := m.NetworkEditor; editor != nil {
		open := true
		title := i18n.T(c, "host_network_edit")
		kit.Dialog(c.Key("network-editor"), &open, func() {
			ui.Text(c, title).FontSize(16).FontWeight(600)
			ui.Text(c, deviceName(c, editor.Device)).FontSize(14).TextColor(theme.Of(c).MutedForeground)
			if editor.Loading {
				kit.Shimmer(c, i18n.T(c, "host_runtime_loading"), 14, theme.Of(c).MutedForeground)
			} else {
				draft := pairing{relay: editor.Relay, address: editor.Address}
				connectionRelayFields(c, &draft, m.Relays, editor.Relays, editor.Pending || editor.Locked || editor.Unavailable, false)
				editor.Relay, editor.Address = draft.relay, draft.address
			}
			ui.Row(c.Key("footer")).Gap(8).Justify(ui.End).Children(func() {
				kit.Secondary(c.Key("cancel"), i18n.T(c, "settings_cancel")).Disabled(editor.Pending).OnClick(func() { m.NetworkEditor = nil })
				label := i18n.T(c, "settings_save")
				if editor.Retry {
					label = i18n.T(c, "settings_retry")
				}
				kit.PrimaryBusy(c.Key("save"), label, editor.Loading || editor.Unavailable, editor.Pending).OnClick(func() {
					if m.ConnectionActions != nil && m.ConnectionActions.SaveNetwork != nil {
						m.ConnectionActions.SaveNetwork(editor)
					}
				})
			})
		}).Width(min(420, width(c)-48)).Gap(16).Label(title)
		// A save under way keeps its editor, as its Cancel says.
		if !open && !editor.Pending {
			m.NetworkEditor = nil
		}
	}

}
