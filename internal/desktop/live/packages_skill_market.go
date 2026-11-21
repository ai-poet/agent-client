package live

import (
	"context"
	"errors"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/settings"
	"github.com/ai-poet/agent-client/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/internal/protocol"
	"github.com/egoist/mygo/ui"
)

var skillMarketSources = [...]protocol.PluginSkillcatalogSource{
	protocol.PluginSkillcatalogSourceOfficial,
	protocol.PluginSkillcatalogSourceSkillsSh,
	protocol.PluginSkillcatalogSourceSkillhub,
}

type skillMarketCursor struct {
	next       uint32
	entries    []protocol.PluginSkillcatalogEntry
	fallback   []protocol.PluginSkillcatalogEntry
	issue      string
	pageFilter bool
}

type skillMarketSearch struct {
	query   string
	stop    context.CancelFunc
	sources [len(skillMarketSources)]skillMarketCursor
}

func (r *Runtime) searchSkillMarket(query string) {
	query = strings.TrimSpace(query)
	if current := r.packages.skillMarket; current != nil && current.query == query {
		return
	}
	r.resetSkillMarket()
	search := &skillMarketSearch{query: query}
	for index := range search.sources {
		search.sources[index].next = 1
	}
	r.packages.skillMarket = search
	r.loadSkillMarket(false)
}

func (r *Runtime) resetSkillMarket() {
	if search := r.packages.skillMarket; search != nil && search.stop != nil {
		search.stop()
	}
	r.packages.skillMarket = nil
	r.clearSkillMarketView()
}

func (r *Runtime) clearSkillMarketView() {
	m := r.App.Settings.Model
	for _, row := range m.MarketSkills {
		delete(r.packages.skillCatalog, row.ID)
	}
	m.MarketSkills = nil
	m.SkillMarket = settings.SkillMarketState{}
}

func (r *Runtime) loadSkillMarket(retry bool) {
	search := r.packages.skillMarket
	m := r.App.Settings.Model
	if search == nil || m.SkillMarket.Loading {
		return
	}
	var pages [len(skillMarketSources)]uint32
	requested := false
	for index, cursor := range search.sources {
		if cursor.next != 0 && (cursor.issue != "") == retry {
			pages[index] = cursor.next
			requested = true
		}
	}
	if !requested {
		return
	}
	// Bound a round by the slowest source rather than adding up deadlines.
	ctx, cancel := context.WithTimeout(r.ctx, 10*time.Second)
	search.stop = cancel
	m.SkillMarket.Loading = true
	if !r.async(func() {
		defer cancel()
		var results [len(skillMarketSources)]skillMarketResult
		var work sync.WaitGroup
		for index, page := range pages {
			if page == 0 {
				continue
			}
			work.Go(func() {
				out, err := r.controller.Do(ctx, protocol.Command{Kind: "search_skill_catalog", Data: protocol.CommandSearchSkillCatalogPayload{
					Source: skillMarketSources[index], Query: search.query, Page: page, Force: retry,
				}})
				results[index] = skillMarketPage(ctx, out, err, page)
			})
		}
		work.Wait()
		r.enqueue(func(*ui.Context) {
			if r.packages.skillMarket != search || r.localRuntime().packageOwner() != r {
				return
			}
			search.stop = nil
			for index, page := range pages {
				if page != 0 {
					search.sources[index].apply(results[index])
				}
			}
			r.publishSkillMarket(search)
		})
	}) {
		cancel()
		search.stop = nil
		m.SkillMarket.Loading = false
	}
}

type skillMarketResult struct {
	page  protocol.PluginSkillcatalogPage
	issue string
}

func skillMarketPage(ctx context.Context, out protocol.Output, err error, page uint32) skillMarketResult {
	switch {
	case err != nil && (errors.Is(err, context.DeadlineExceeded) || errors.Is(ctx.Err(), context.DeadlineExceeded)):
		return skillMarketResult{issue: "market_skill_catalog_timeout"}
	case err != nil && faultMessage(err) == "resource host is offline":
		return skillMarketResult{issue: "resource_offline_action"}
	case err != nil:
		switch faultCode(err) {
		case protocol.ErrorCodePermissionDenied:
			return skillMarketResult{issue: "permission_denied"}
		case protocol.ErrorCodeInvalidRequest:
			return skillMarketResult{issue: "market_skill_catalog_invalid_response"}
		case protocol.ErrorCodeUnavailable, protocol.ErrorCodeNotFound:
			return skillMarketResult{issue: "unavailable"}
		default:
			return skillMarketResult{issue: "market_skills_failed"}
		}
	}
	value, valid := detail[protocol.PluginSkillcatalogPage](out.Data)
	if !valid || out.Kind != "skill_catalog" || value.Page != page || value.Next != nil && *value.Next <= page {
		return skillMarketResult{issue: "market_skill_catalog_invalid_response"}
	}
	result := skillMarketResult{page: value}
	if value.Unavailable {
		result.issue = "unavailable"
	}
	return result
}

func (cursor *skillMarketCursor) apply(result skillMarketResult) {
	cursor.issue = result.issue
	cursor.pageFilter = cursor.pageFilter || result.page.QueryScope == protocol.PluginSkillcatalogQueryScopePage
	if cursor.issue != "" {
		if result.page.Unavailable {
			cursor.fallback = result.page.Entries
		}
		return
	}
	cursor.fallback = nil
	cursor.entries = append(cursor.entries, result.page.Entries...)
	cursor.next = 0
	if result.page.Next != nil {
		cursor.next = *result.page.Next
	}
}

func (r *Runtime) publishSkillMarket(search *skillMarketSearch) {
	r.clearSkillMarketView()
	m := r.App.Settings.Model
	locale := i18n.Match(r.packageLocale())
	seen := map[string]bool{}
	for index, cursor := range search.sources {
		if cursor.issue != "" {
			m.SkillMarket.Issues = append(m.SkillMarket.Issues, settings.SkillMarketIssue{Source: skillMarketSources[index], Reason: cursor.issue})
		} else if cursor.next != 0 {
			m.SkillMarket.More = true
		}
		m.SkillMarket.PageFilter = m.SkillMarket.PageFilter || cursor.pageFilter
		for _, entries := range [][]protocol.PluginSkillcatalogEntry{cursor.entries, cursor.fallback} {
			for _, entry := range entries {
				if seen[entry.ID] {
					continue
				}
				seen[entry.ID] = true
				r.packages.skillCatalog[entry.ID] = entry
				row := viewmodel.Skill{ID: entry.ID, Name: entry.Name, Description: stringValue(entry.Description), Origin: entry.Origin, Content: r.packages.documents[entry.ID]}
				if entry.DisplayName != nil {
					row.Name = *entry.DisplayName
				}
				if title := entry.DisplayLocales[locale]; title != "" {
					row.Name = title
				}
				if text := entry.DescriptionLocales[locale]; text != "" {
					row.Description = text
				}
				if entry.Package != nil {
					row.Package = *entry.Package
				}
				m.MarketSkills = append(m.MarketSkills, row)
			}
		}
	}
	r.projectSkillMarket()
	r.readMarketSkillUpdates()
}
