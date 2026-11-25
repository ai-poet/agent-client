package store

import (
	"bytes"
	"cmp"
	"container/heap"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"math"
	"math/big"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// Only decode scalar diagnostic facts, not the context composition or stream
// annotations. Unlike transport declarations, this also accepts old snapshots
// with missing fields. Usage is only needed if the canonical bill is missing.
type usageAttemptFacts struct {
	Verdict          *protocol.TraceVerdict `json:"verdict"`
	Index            *uint64                `json:"index"`
	Status           *uint16                `json:"status"`
	Outcome          *string                `json:"outcome"`
	EndKind          *string                `json:"end_kind"`
	GatewayRequestID *string                `json:"gateway_request_id"`
	RetryReason      *string                `json:"retry_reason"`
	FailureKind      *string                `json:"failure_kind"`
	usageAttemptTiming
	CacheWrite        *uint64         `json:"cache_write"`
	CacheReported     bool            `json:"cache_reported"`
	ReasoningReported bool            `json:"reasoning_reported"`
	Usage             json.RawMessage `json:"usage"`
}

// Old records can contain negative duration sentinels. As in the original
// scalar SQL projection, those are unknown, not zero-duration samples.
type usageAttemptTiming struct {
	ElapsedUs    *int64 `json:"elapsed_us"`
	FirstTokenUs *int64 `json:"first_token_us"`
}

func usageDuration(v *int64) *uint64 {
	if v == nil || *v < 0 {
		return nil
	}
	us := uint64(*v)
	return &us
}

type usagePriceKey struct {
	scope                                           *usageRowScope
	input, output, cacheRead, cacheWrite, reasoning uint64
}

// Keep the oldest retained row at the root. Aggregates still see every row.
type usageRequestHeap []protocol.UsageRequest

func (h usageRequestHeap) Len() int { return len(h) }
func (h usageRequestHeap) Less(i, j int) bool {
	return compareUsageOrder(h[i].Position, h[j].Position) < 0
}
func (h usageRequestHeap) Swap(i, j int) { h[i], h[j] = h[j], h[i] }
func (h *usageRequestHeap) Push(v any)   { *h = append(*h, v.(protocol.UsageRequest)) }
func (h *usageRequestHeap) Pop() any {
	last := len(*h) - 1
	v := (*h)[last]
	*h = (*h)[:last]
	return v
}

func compareUsageOrder(a, b protocol.UsagePosition) int {
	if a.TimestampMs != b.TimestampMs {
		return cmp.Compare(a.TimestampMs, b.TimestampMs)
	}
	if a.Node != b.Node {
		return bytes.Compare(a.Node[:], b.Node[:])
	}
	return cmp.Or(cmp.Compare(a.Sequence, b.Sequence), cmp.Compare(a.Ordinal, b.Ordinal))
}

func readAgentUsage(ctx context.Context, db *sql.Conn, node protocol.NodeID, query protocol.UsageQuery, observations bool) (protocol.UsageReport, error) {
	report := protocol.DefaultUsageReport()
	report.Node = node
	report.Query = query
	if !observations && (query.CacheMissOnly || query.ObservedOnly || query.WithoutCredential || query.Dimension == protocol.UsageDimensionSession || query.Dimension == protocol.UsageDimensionCredential || len(query.Statuses) != 0 || len(query.FailureKinds) != 0 || len(query.Outcomes) != 0 || len(query.EndKinds) != 0 || len(query.Sessions) != 0 || len(query.Credentials) != 0) {
		return report, invalid("observation filters require read_usage_v2")
	}
	if len(query.Statuses) > 128 || len(query.FailureKinds) > 128 || (query.WithoutCredential && len(query.Credentials) > 0) || query.StartMs < 0 || query.EndMs <= query.StartMs || query.EndMs-query.StartMs > 366*86400000 || query.UtcOffsetMinutes > 840 || query.UtcOffsetMinutes < -840 || query.Before != nil && (query.Before.TimestampMs < query.StartMs || query.Before.TimestampMs >= query.EndMs) || len(query.Projects) > 128 || len(query.Worktrees) > 128 || len(query.Providers) > 128 || len(query.Models) > 128 || len(query.Outcomes) > 128 || len(query.EndKinds) > 128 || len(query.Sessions) > 128 || len(query.Credentials) > 128 {
		return report, invalid("usage time range or filters exceed the query limits")
	}
	for _, status := range query.Statuses {
		if status < 100 || status > 599 {
			return report, invalid("invalid HTTP status")
		}
	}
	for _, value := range slices.Concat(query.Models, query.FailureKinds) {
		if strings.TrimSpace(value) == "" || len(value) > 256 {
			return report, invalid("usage time range or filters exceed the query limits")
		}
	}
	if err := db.QueryRowContext(ctx, "SELECT coalesce(max(cursor),0) FROM events").Scan(&report.Cursor); err != nil {
		return report, err
	}
	offset := int64(query.UtcOffsetMinutes) * 60000
	report.Days = usageBuckets(query.StartMs, query.EndMs, 86400000, offset)
	recentStart := max(query.StartMs, query.EndMs-2*86400000)
	report.Recent = usageBuckets(recentStart, query.EndMs, 1800000, offset)
	args := []any{time.UnixMilli(query.StartMs).UTC().Format("2006-01-02T15:04:05"), time.UnixMilli(query.EndMs).UTC().Format("2006-01-02T15:04:05")}
	for _, filter := range []any{query.Projects, query.Worktrees, query.Providers, query.Models, query.Outcomes, query.EndKinds, query.Sessions, query.Credentials, query.Statuses, query.FailureKinds} {
		data, err := encode(filter)
		if err != nil {
			return report, err
		}
		if string(data) == "null" {
			data = []byte("[]")
		}
		args = append(args, string(data))
	}
	user, err := CurrentUser(ctx, db)
	if err != nil {
		return report, err
	}
	rows, err := db.QueryContext(ctx, agentUsageRows(observations, query.ObservedOnly, len(query.Providers)+len(query.Models)+len(query.Credentials) > 0 || query.WithoutCredential), append(args, 100001, user.ID.String(), query.WithoutCredential, query.ObservedOnly, query.CacheMissOnly)...)
	if err != nil {
		return report, err
	}
	defer rows.Close()
	groups := map[string]*protocol.UsageGroup{}
	resources := map[string]protocol.UsageKey{}
	requests := usageRequestHeap{}
	prices := modelPrices{}
	totalInsights := usageInsightAccumulator{}
	groupInsights := map[string]*usageInsightAccumulator{}
	var lastUsage sql.NullString
	var lastTokens canonical.Usage
	var lastPriceKey usagePriceKey
	var lastPrice *protocol.UsageCost
	for row, e := range streamUsageRows(ctx, db, rows, query, observations, groups, resources) {
		if e != nil {
			return report, e
		}
		ms, scope := row.ms, row.scope
		if scope.groupName != nil {
			scope.group.Name = scope.groupName
		}
		request := scope.base
		var a usageAttemptFacts
		if row.attempt.Valid {
			if e = json.Unmarshal([]byte(row.attempt.String), &a); e != nil {
				return report, e
			}
		}
		if row.timing.Valid && (a.ElapsedUs == nil || a.FirstTokenUs == nil) {
			var timing usageAttemptTiming
			if e = json.Unmarshal([]byte(row.timing.String), &timing); e != nil {
				return report, e
			}
			if a.ElapsedUs == nil {
				a.ElapsedUs = timing.ElapsedUs
			}
			if a.FirstTokenUs == nil {
				a.FirstTokenUs = timing.FirstTokenUs
			}
		}
		elapsed := usageDuration(a.ElapsedUs)
		if row.usage != lastUsage {
			lastTokens = canonical.Usage{}
			if row.usage.Valid {
				if e = json.Unmarshal([]byte(row.usage.String), &lastTokens); e != nil {
					return report, e
				}
			}
			lastUsage = row.usage
		}
		usage := lastTokens
		if len(a.Usage) > 0 && string(a.Usage) != "null" && (usage.Input == nil || usage.Output == nil) {
			var u protocol.ConversationUsage
			if err := json.Unmarshal(a.Usage, &u); err != nil {
				return report, err
			}
			usage.Input, usage.Output = &u.Input, &u.Output
			if a.CacheReported {
				usage.CacheRead = &u.CachedInput
			}
			if a.ReasoningReported {
				usage.Reasoning = &u.Reasoning
			}
		}
		if a.CacheWrite != nil {
			usage.CacheWrite = a.CacheWrite
		}
		var tokens *protocol.ConversationUsage
		var cost *protocol.UsageCost
		if usage.Input != nil && usage.Output != nil {
			tokens = &protocol.ConversationUsage{Input: *usage.Input, Output: *usage.Output, CachedInput: optionalUint(usage.CacheRead), Reasoning: optionalUint(usage.Reasoning)}
			if row.key.media.String != "image" && row.key.media.String != "video" {
				key := usagePriceKey{scope: scope, input: *usage.Input, output: *usage.Output, cacheRead: optionalUint(usage.CacheRead), cacheWrite: optionalUint(usage.CacheWrite), reasoning: optionalUint(usage.Reasoning)}
				if len(usage.Provider) == 0 && key == lastPriceKey {
					cost = lastPrice
				} else {
					cost, e = estimateAgentCost(ctx, db, prices, scope.preset, scope.api, request.Model, usage)
					if e != nil {
						return report, e
					}
					if len(usage.Provider) == 0 {
						lastPriceKey, lastPrice = key, cost
					}
				}
			}
		}
		if tokens == nil && !observations {
			continue
		}
		if tokens != nil {
			if e = addUsageMetrics(&report.Totals, *tokens, cost, elapsed); e != nil {
				return report, e
			}
			if e = addUsageMetrics(&scope.group.Metrics, *tokens, cost, elapsed); e != nil {
				return report, e
			}
			day := int((usageFloor(ms, 86400000, offset) - report.Days[0].StartMs) / 86400000)
			if e = addUsageMetrics(&report.Days[day].Metrics, *tokens, cost, elapsed); e != nil {
				return report, e
			}
			addUsageModel(&report.Days[day], request.Model, *tokens)
			if ms >= recentStart {
				bucket := int((usageFloor(ms, 1800000, offset) - report.Recent[0].StartMs) / 1800000)
				if e = addUsageMetrics(&report.Recent[bucket].Metrics, *tokens, cost, elapsed); e != nil {
					return report, e
				}
				addUsageModel(&report.Recent[bucket], request.Model, *tokens)
			}
		}
		request.Tokens, request.ElapsedUs = tokens, elapsed
		if observations {
			request.RequestID, request.Attempt, request.Status = row.requestID, a.Index, a.Status
			request.Verdict, request.CacheRead = a.Verdict, usage.CacheRead
			request.Outcome, request.EndKind = (*protocol.ConversationObservationOutcome)(a.Outcome), (*protocol.ConversationObservationEndKind)(a.EndKind)
			request.CacheWrite, request.GatewayRequestID, request.RetryReason = usage.CacheWrite, a.GatewayRequestID, a.RetryReason
			request.FailureKind = a.FailureKind
			if request.FailureKind == nil {
				request.FailureKind = a.RetryReason
			}
			if request.FailureKind == nil && request.Outcome != nil && *request.Outcome == protocol.ConversationObservationOutcomeFailed {
				unknown := "unknown"
				request.FailureKind = &unknown
			}
		}
		if first := usageDuration(a.FirstTokenUs); first != nil && (elapsed != nil && *first <= *elapsed || elapsed == nil && observations) {
			request.FirstTokenUs = first
		}
		if cost != nil {
			request.UsdMicros = &cost.UsdMicros
		}
		if observations {
			groupStats := groupInsights[scope.groupKey]
			if groupStats == nil {
				groupStats = &usageInsightAccumulator{}
				groupInsights[scope.groupKey] = groupStats
			}
			for _, stats := range []*usageInsightAccumulator{&totalInsights, groupStats} {
				if err := stats.add(request, usage); err != nil {
					return report, err
				}
			}
		}
		position := protocol.UsagePosition{TimestampMs: ms, Node: node, Sequence: row.sequence, Ordinal: row.ordinal}
		if before := query.Before; before != nil && compareUsageOrder(position, *before) >= 0 {
			continue
		}
		request.Position = position
		if len(requests) < 20 {
			requests = append(requests, request)
			if len(requests) == 20 {
				heap.Init(&requests)
			}
		} else {
			report.Requests.HasMore = true
			if compareUsageOrder(request.Position, requests[0].Position) > 0 {
				requests[0] = request
				heap.Fix(&requests, 0)
			}
		}
	}
	if err = errors.Join(rows.Err(), rows.Close()); err != nil {
		return report, err
	}
	slices.SortFunc(requests, func(a, b protocol.UsageRequest) int { return compareUsageOrder(b.Position, a.Position) })
	titles := map[protocol.SessionID]string{}
	for i := range requests {
		title, known := titles[requests[i].Session]
		if !known {
			full, e := activityTitle(ctx, db, requests[i].Session)
			if e != nil {
				return report, e
			}
			title = shortTitle(full)
			titles[requests[i].Session] = title
		}
		if strings.TrimSpace(title) != "" {
			requests[i].SessionTitle = &title
		}
	}
	if observations {
		report.Insights = totalInsights.finish()
	}
	report.Requests.Items = requests
	groupKeys := []string{}
	for k := range groups {
		groupKeys = append(groupKeys, k)
	}
	slices.Sort(groupKeys)
	for _, k := range groupKeys {
		if observations {
			groups[k].Insights = groupInsights[k].finish()
		}
		report.Groups = append(report.Groups, *groups[k])
	}
	resourceKeys := []string{}
	for k := range resources {
		resourceKeys = append(resourceKeys, k)
	}
	slices.Sort(resourceKeys)
	for _, k := range resourceKeys {
		report.Resources = append(report.Resources, resources[k])
	}
	var updated sql.NullInt64
	err = db.QueryRowContext(ctx, "SELECT json_extract(body,'$.updated_at_ms') FROM model_catalog_status WHERE id=1").Scan(&updated)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return report, err
	}
	if updated.Valid && updated.Int64 >= 0 {
		v := uint64(updated.Int64)
		report.PricingUpdatedAtMs = &v
	}
	encoded, e := encode(report)
	if e != nil {
		return report, e
	}
	if len(encoded) > protocol.MAX_FRAME_BYTES-64*1024 {
		return report, invalid("usage report exceeds the response limit; narrow the filters")
	}
	return report, nil
}
func usageKeyText(key protocol.UsageKey) string {
	order := slices.Index([]string{"provider", "model", "project", "session", "connection", "assistant", "worktree", "credential", "unassigned"}, key.Kind)
	body, _ := encode(key.Data)
	return string(rune('0'+order)) + string(body)
}
func usageFloor(ms, width, offset int64) int64 {
	v := ms + offset
	q := v / width
	if v < 0 && v%width != 0 {
		q--
	}
	return q*width - offset
}
func usageBuckets(start, end, width, offset int64) []protocol.UsagePoint {
	out := []protocol.UsagePoint{}
	for t := usageFloor(start, width, offset); t < end; t += width {
		p := protocol.DefaultUsagePoint()
		p.StartMs = t
		out = append(out, p)
	}
	return out
}
func addUsageModel(point *protocol.UsagePoint, model string, value protocol.ConversationUsage) {
	if point.Models == nil {
		point.Models = map[string]protocol.ConversationUsage{}
	}
	v := point.Models[model]
	v.Input += value.Input
	v.Output += value.Output
	v.CachedInput += value.CachedInput
	v.Reasoning += value.Reasoning
	point.Models[model] = v
}
func addUsageMetrics(target *protocol.UsageMetrics, tokens protocol.ConversationUsage, cost *protocol.UsageCost, elapsed *uint64) error {
	target.Responses++
	if target.Tokens == nil {
		target.Tokens = &protocol.ConversationUsage{}
	}
	target.Tokens.Input += tokens.Input
	target.Tokens.Output += tokens.Output
	target.Tokens.CachedInput += tokens.CachedInput
	target.Tokens.Reasoning += tokens.Reasoning
	if elapsed != nil {
		if target.Generation == nil {
			target.Generation = &protocol.UsageGeneration{}
		}
		if math.MaxUint64-target.Generation.ElapsedUs < *elapsed {
			return invalid("usage duration exceeds the counter limit")
		}
		target.Generation.ElapsedUs += *elapsed
		target.Generation.OutputTokens += tokens.Output
		target.Generation.Responses++
	}
	if cost != nil {
		if target.Cost == nil {
			copy := *cost
			if cost.Breakdown != nil {
				b := *cost.Breakdown
				copy.Breakdown = &b
			}
			target.Cost = &copy
		} else {
			if math.MaxUint64-target.Cost.UsdMicros < cost.UsdMicros {
				return invalid("usage cost exceeds the counter limit")
			}
			target.Cost.UsdMicros += cost.UsdMicros
			target.Cost.Responses += cost.Responses
			if target.Cost.Breakdown != nil && cost.Breakdown != nil {
				a, b := target.Cost.Breakdown, cost.Breakdown
				a.Input += b.Input
				a.Output += b.Output
				a.CacheRead += b.CacheRead
				a.CacheWrite += b.CacheWrite
			} else {
				target.Cost.Breakdown = nil
			}
		}
	}
	return nil
}

// modelPrices holds the catalog entries one report has read, keyed by supplier
// and model; the empty supplier keys the entry every supplier agrees on. An
// entry the catalog lacks is kept as nil.
type modelPrices map[[2]string][]byte

// catalogSuppliers names the models.dev supplier of a preset or brand that
// models.dev lists under another identifier.
var catalogSuppliers = map[string]string{"gemini": "google", "qwen": "alibaba", "moonshot": "moonshotai", "fireworks": "fireworks-ai", "together": "togetherai", "zhipu": "zhipuai", "doubao": "volcengine"}

var presetBrands = sync.OnceValue(func() map[string]string {
	brands := map[string]string{}
	for _, preset := range providers.Bundled().Builtin {
		if preset.Brand != nil {
			brands[preset.ID] = *preset.Brand
		}
	}
	return brands
})

// priceSuppliers lists, in order, the suppliers whose price applies to a
// response: the vendor of its channel (the preset, then the preset's brand),
// then the vendor its wire protocol belongs to.
func priceSuppliers(preset, api string) []string {
	var suppliers []string
	for _, name := range []string{preset, presetBrands()[preset], map[string]string{"responses": "openai", "chat_completions": "openai", "anthropic": "anthropic", "gemini": "google"}[api]} {
		if alias, ok := catalogSuppliers[name]; ok {
			name = alias
		}
		if name != "" && !slices.Contains(suppliers, name) {
			suppliers = append(suppliers, name)
		}
	}
	return suppliers
}

// entry returns the catalog entry that prices a model: the first of the
// suppliers that lists the exact identifier, else the entry of any other
// supplier when all that list it state one price. Resellers price the same
// identifier differently, so a model they disagree on stays unpriced. The
// supplier identifies which vendor-specific pricing rules can be applied.
func (prices modelPrices) entry(ctx context.Context, db *sql.Conn, suppliers []string, model string) ([]byte, string, error) {
	for _, supplier := range append(suppliers, "") {
		key := [2]string{supplier, model}
		body, known := prices[key]
		if !known {
			var err error
			if supplier != "" {
				err = db.QueryRowContext(ctx, "SELECT body FROM model_catalog WHERE provider=?1 AND id=?2", supplier, model).Scan(&body)
				if errors.Is(err, sql.ErrNoRows) {
					err = nil
				}
			} else {
				body, err = agreedPrice(ctx, db, model)
			}
			if err != nil {
				return nil, "", err
			}
			prices[key] = body
		}
		if body != nil {
			return body, supplier, nil
		}
	}
	return nil, "", nil
}

func agreedPrice(ctx context.Context, db *sql.Conn, model string) ([]byte, error) {
	rows, err := db.QueryContext(ctx, "SELECT body FROM model_catalog WHERE id=?1 ORDER BY provider", model)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var agreed []byte
	price := ""
	for rows.Next() {
		var body []byte
		if err = rows.Scan(&body); err != nil {
			return nil, err
		}
		var entry struct {
			Cost map[string]any `json:"cost"`
		}
		if json.Unmarshal(body, &entry) != nil || entry.Cost == nil {
			continue
		}
		// Encoding a decoded object orders its keys, so equal prices compare equal.
		cost, _ := json.Marshal(entry.Cost)
		if agreed != nil && string(cost) != price {
			return nil, rows.Err()
		}
		agreed, price = body, string(cost)
	}
	return agreed, rows.Err()
}

func estimateAgentCost(ctx context.Context, db *sql.Conn, prices modelPrices, preset, api, model string, usage canonical.Usage) (*protocol.UsageCost, error) {
	if usage.Input == nil || usage.Output == nil {
		return nil, nil
	}
	body, supplier, err := prices.entry(ctx, db, priceSuppliers(preset, api), model)
	if err != nil || body == nil {
		return nil, err
	}
	var object map[string]json.RawMessage
	if json.Unmarshal(body, &object) != nil {
		return nil, nil
	}
	var cost map[string]json.RawMessage
	if json.Unmarshal(object["cost"], &cost) != nil {
		return nil, nil
	}
	if raw, ok := cost["tiers"]; ok {
		var tiers []map[string]json.RawMessage
		if json.Unmarshal(raw, &tiers) != nil {
			return nil, nil
		}
		seen := map[uint64]bool{}
		threshold := uint64(0)
		for _, tier := range tiers {
			var criteria struct {
				Type string `json:"type"`
				Size uint64 `json:"size"`
			}
			if json.Unmarshal(tier["tier"], &criteria) != nil || criteria.Type != "context" || seen[criteria.Size] {
				return nil, nil
			}
			seen[criteria.Size] = true
			if *usage.Input > criteria.Size && criteria.Size >= threshold {
				cost = tier
				threshold = criteria.Size
			}
		}
	} else if *usage.Input > 200000 {
		if raw, ok := cost["context_over_200k"]; ok {
			if json.Unmarshal(raw, &cost) != nil {
				return nil, nil
			}
		}
	}
	if _, ok := cost["input_audio"]; ok {
		return nil, nil
	}
	if _, ok := cost["output_audio"]; ok {
		return nil, nil
	}
	read, write, reasoning := optionalUint(usage.CacheRead), optionalUint(usage.CacheWrite), optionalUint(usage.Reasoning)
	if *usage.Input < read || *usage.Input-read < write || *usage.Output < reasoning {
		return nil, nil
	}
	hourlyWrite := uint64(0)
	if api == "anthropic" && write > 0 && len(usage.Provider) > 0 {
		var reported struct {
			Creation *struct {
				FiveMinutes *uint64 `json:"ephemeral_5m_input_tokens"`
				OneHour     *uint64 `json:"ephemeral_1h_input_tokens"`
			} `json:"cache_creation"`
		}
		if json.Unmarshal(usage.Provider, &reported) != nil {
			return nil, nil
		}
		if creation := reported.Creation; creation != nil {
			if creation.FiveMinutes == nil || creation.OneHour == nil || *creation.FiveMinutes > write || *creation.OneHour != write-*creation.FiveMinutes {
				return nil, nil
			}
			hourlyWrite = *creation.OneHour
			if hourlyWrite > 0 && supplier != "anthropic" {
				// A compatible API does not imply Anthropic's cache pricing.
				return nil, nil
			}
		}
	}
	rate := func(field string) (uint64, bool) {
		var v float64
		if json.Unmarshal(cost[field], &v) != nil || math.IsNaN(v) || math.IsInf(v, 0) || v < 0 || v*1000000 >= float64(math.MaxUint64) {
			return 0, false
		}
		return uint64(math.Round(v * 1000000)), true
	}
	if _, ok := rate("input"); !ok {
		return nil, nil
	}
	if _, ok := rate("output"); !ok {
		return nil, nil
	}
	categories := [4]*big.Int{new(big.Int), new(big.Int), new(big.Int), new(big.Int)}
	for _, item := range []struct {
		count           uint64
		field, fallback string
		category        int
		multiplier      int64
	}{
		{*usage.Input - read - write, "input", "", 0, 1},
		{*usage.Output - reasoning, "output", "", 1, 1},
		{read, "cache_read", "", 2, 1},
		{write - hourlyWrite, "cache_write", "", 3, 1},
		// Anthropic's catalog cache_write rate is for five minutes. It bills
		// one-hour writes at twice the base input rate, including context tiers.
		{hourlyWrite, "input", "", 3, 2},
		{reasoning, "reasoning", "output", 1, 1},
	} {
		if item.count == 0 {
			continue
		}
		r, ok := rate(item.field)
		if !ok && item.fallback != "" {
			r, ok = rate(item.fallback)
		}
		if !ok {
			return nil, nil
		}
		product := new(big.Int).Mul(new(big.Int).SetUint64(r), new(big.Int).SetUint64(item.count))
		product.Mul(product, big.NewInt(item.multiplier))
		categories[item.category].Add(categories[item.category], product)
	}
	cumulative := new(big.Int)
	total := uint64(0)
	amounts := [4]uint64{}
	for i, category := range categories {
		cumulative.Add(cumulative, category)
		rounded := new(big.Int).Add(cumulative, big.NewInt(500000))
		rounded.Div(rounded, big.NewInt(1000000))
		if !rounded.IsUint64() {
			return nil, nil
		}
		next := rounded.Uint64()
		amounts[i] = next - total
		total = next
	}
	return &protocol.UsageCost{UsdMicros: total, Responses: 1, Breakdown: &protocol.UsageCostBreakdown{Input: amounts[0], Output: amounts[1], CacheRead: amounts[2], CacheWrite: amounts[3]}}, nil
}
