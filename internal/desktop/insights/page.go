package insights

import (
	"fmt"
	"reflect"
	"strconv"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

type Actions struct {
	Apply, More func()
	Open        func(protocol.UsageRequest)
	Host        func(*ui.Context)
}

func Page(c *ui.Context, s *State, a Actions) {
	t := func(k string) string { return i18n.T(c, k) }
	ui.Scroll(c.Key("insights")).Grow(1).MinHeight(0).Padding(20).Gap(12).Children(func() {
		ui.Text(c, t("insights_title")).FontSize(22)
		ui.Row(c.Key("toolbar")).Wrap().Gap(8).AlignItems(ui.Center).Children(func() {
			a.Host(c)
			d := strconv.Itoa(s.Draft.Days)
			labels := []string{"24 h"}
			for _, days := range []int{7, 30} {
				labels = append(labels, i18n.T(c, "usage_range_days", "days", days))
			}
			labels = append(labels, t("pairing_relay_custom"))
			if _, ok := kit.Select(c, "range", t("usage_range"), &d, labels, false, []string{"1", "7", "30", "0"}); ok {
				s.Draft.Days, _ = strconv.Atoi(d)
				if s.Draft.Days == 0 && s.Draft.StartDate == "" {
					now := time.Now()
					s.Draft.StartDate = now.AddDate(0, 0, -6).Format(time.DateOnly)
					s.Draft.EndDate = now.Format(time.DateOnly)
				}
				a.Apply()
			}
			kit.Ghost(c, t("usage_filters"), s.Advanced).OnClick(func() { s.Advanced = !s.Advanced })
			kit.Secondary(c, t("refresh")).OnClick(a.Apply)
		})
		if s.Draft.Days == 0 {
			ui.Row(c.Key("dates")).Wrap().Gap(8).Children(func() {
				for _, v := range []struct {
					k string
					p *string
				}{{"insights_start", &s.Draft.StartDate}, {"insights_until", &s.Draft.EndDate}} {
					ui.Column(c.Key(v.k)).Children(func() {
						ui.Text(c, t(v.k))
						ui.TextInput(c.Key("date"), v.p).Label(t(v.k)).Placeholder("YYYY-MM-DD").Width(152).OnSubmit(a.Apply)
					})
				}
				kit.Secondary(c, t("insights_apply")).OnClick(a.Apply)
			})
		}
		if s.Advanced {
			filterFields(c, &s.Draft, a.Apply)
		}
		kit.Segmented(c, "tabs", &s.Tab, []string{t("usage"), t("insights_index")}, kit.SegmentedProps{Size: kit.SizeSmall})
		if s.Draft.ObservedOnly != (s.Tab == 1) {
			s.Draft.ObservedOnly = s.Tab == 1
			a.Apply()
		}
		if s.Error != "" {
			ui.Text(c, t(s.Error)).TextColor(theme.Of(c).Danger)
		}
		if s.Loading {
			ui.Text(c, t("usage_loading"))
		}
		if s.Report == nil {
			return
		}
		switch s.Tab {
		case 0:
			overview(c, s.Report)
			ui.Text(c, t("insights_groups")).FontSize(16)
			d := string(s.Draft.Dimension)
			if _, changed := kit.Select(c, "grouping", t("insights_groups"), &d, []string{t("model"), t("usage_column_provider"), t("insights_credential"), t("insights_session"), t("usage_column_project"), t("insights_worktree")}, false, []string{"model", "provider", "credential", "session", "project", "worktree"}); changed {
				s.Draft.Dimension = protocol.UsageDimension(d)
				a.Apply()
				return
			}
			if len(s.Report.Groups) == 0 {
				ui.Text(c, t("usage_empty"))
			}
			for _, g := range s.Report.Groups {
				usageGroup(c, s, g, a.Apply)
			}
		case 1:
			if len(s.Report.Requests.Items) == 0 {
				ui.Text(c, t("usage_empty"))
			}
			for _, r := range s.Report.Requests.Items {
				requestRow(c, r, a.Open)
			}
			if s.Next() != nil {
				kit.Secondary(c, t("files_load_more")).Disabled(s.Loading).OnClick(a.More)
			}
		}
	})
}

func filterFields(c *ui.Context, f *Filters, apply func()) {
	t := func(k string) string { return i18n.T(c, k) }
	ui.Row(c.Key("filter-fields")).Wrap().Gap(8).Children(func() {
		kit.Select(c, "outcome", t("insights_outcome"), &f.Outcome, []string{t("insights_all"), t("meter_succeeded"), t("meter_failed"), t("approval_cancelled")}, false, []string{"", "succeeded", "failed", "cancelled"})
		ends := []string{"", "completed", "http-error", "stream-error", "client-abort", "upstream-reset", "stream-truncated", "upstream-timeout", "upstream-unreachable"}
		kit.Select(c, "end", t("insights_end"), &f.EndKind, append([]string{t("insights_all")}, ends[1:]...), false, ends)
		for _, v := range []struct {
			k     string
			value *string
		}{{"usage_column_model", &f.Model}, {"insights_provider_id", &f.Provider}, {"insights_status", &f.Status}, {"insights_session", &f.Session}, {"insights_credential", &f.Credential}, {"usage_column_project", &f.Project}, {"insights_worktree", &f.Worktree}, {"insights_failure", &f.FailureKind}} {
			ui.TextInput(c.Key(v.k), v.value).Width(176).Label(t(v.k)).Placeholder(t(v.k)).OnSubmit(apply)
		}
		kit.Ghost(c, t("insights_unassigned"), f.WithoutCredential).OnClick(func() { f.WithoutCredential = !f.WithoutCredential })
		kit.Ghost(c, t("trace_miss"), f.CacheMissOnly).OnClick(func() { f.CacheMissOnly = !f.CacheMissOnly })
		kit.Primary(c, t("insights_apply"), false).OnClick(apply)
	})
}

// A local transport can return a value; decoded protocol unions hold pointers.
func keyData(k protocol.UsageKey) any {
	v := reflect.Indirect(reflect.ValueOf(k.Data))
	if !v.IsValid() {
		return nil
	}
	return v.Interface()
}

func groupName(g protocol.UsageGroup) string {
	switch k := keyData(g.Key).(type) {
	case protocol.UsageKeyModelPayload:
		return k.Model + " / " + k.Provider.String()
	case protocol.UsageKeyWorktreePayload:
		return k.Worktree.String()
	}
	if g.Name != nil && *g.Name != "" {
		return *g.Name
	}
	return fmt.Sprint(keyData(g.Key))
}

func narrow(f Filters, k protocol.UsageKey) (Filters, bool) {
	switch v := keyData(k).(type) {
	case protocol.ProviderID:
		f.Provider = v.String()
	case protocol.ProjectID:
		f.Project = v.String()
	case protocol.SessionID:
		f.Session = v.String()
	case protocol.CredentialID:
		f.Credential = v.String()
		f.WithoutCredential = false
	case protocol.UsageKeyModelPayload:
		f.Model, f.Provider = v.Model, v.Provider.String()
	case protocol.UsageKeyWorktreePayload:
		f.Worktree, f.Project = v.Worktree.String(), v.Project.String()
	default:
		if k.Kind != "unassigned" {
			return f, false
		}
		f.WithoutCredential = true
		f.Credential = ""
	}
	return f, true
}
