package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"iter"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

type usageScopeKey struct {
	turn, session string
	source, media sql.NullString
}

type usageRowScope struct {
	base        protocol.UsageRequest
	preset, api string
	group       *protocol.UsageGroup
	groupKey    string
	groupName   *string
}

type usageScanRow struct {
	key                    usageScopeKey
	scope                  *usageRowScope
	sequence, ordinal      uint64
	ms                     int64
	usage, timing, attempt sql.NullString
	requestID              *string
}

// Batch distinct scope misses, so an interval spanning many turns does not
// require one round trip per turn. Only visible sequences from this same read
// transaction feed the lookup. Neither the cache nor the buffer spans reports.
func streamUsageRows(ctx context.Context, db *sql.Conn, rows *sql.Rows, q protocol.UsageQuery, observations bool, groups map[string]*protocol.UsageGroup, resources map[string]protocol.UsageKey) iter.Seq2[*usageScanRow, error] {
	return func(yield func(*usageScanRow, error) bool) {
		const blockSize = 256
		var block [blockSize]usageScanRow
		scopes := map[usageScopeKey]*usageRowScope{}
		sequences := make([]uint64, 0, blockSize)
		keys := map[uint64]usageScopeKey{}
		var timestamp string
		count := 0
		for {
			n := 0
			sequences = sequences[:0]
			clear(keys)
			if len(scopes) > 4096-blockSize {
				clear(scopes)
			}
			for n < len(block) && rows.Next() {
				count++
				if count > 100000 {
					yield(nil, invalid("usage report exceeds the row limit; narrow the time range or filters"))
					return
				}
				r := &block[n]
				if err := rows.Scan(&r.sequence, &r.key.turn, &r.key.session, &r.key.source, &r.key.media, &timestamp, &r.usage, &r.timing, &r.requestID, &r.ordinal, &r.attempt); err != nil {
					yield(nil, err)
					return
				}
				created, err := time.Parse(time.RFC3339Nano, timestamp)
				if err != nil {
					yield(nil, err)
					return
				}
				r.ms = created.UnixMilli()
				if r.ms < q.StartMs || r.ms >= q.EndMs {
					continue
				}
				if _, ok := scopes[r.key]; !ok {
					scopes[r.key] = nil
					keys[r.sequence] = r.key
					sequences = append(sequences, r.sequence)
				}
				n++
			}
			if err := rows.Err(); err != nil {
				yield(nil, err)
				return
			}
			if n == 0 {
				return
			}
			if len(sequences) != 0 {
				ids, err := json.Marshal(sequences)
				if err != nil {
					yield(nil, err)
					return
				}
				summary, err := db.QueryContext(ctx, agentUsageScope, string(ids))
				if err != nil {
					yield(nil, err)
					return
				}
				for summary.Next() {
					var k usageScopeKey
					var scope *usageRowScope
					k, scope, err = decodeUsageScope(summary, keys, q.Dimension, observations, groups, resources)
					if err != nil {
						break
					}
					scopes[k] = scope
				}
				if err == nil {
					err = summary.Err()
				}
				closeErr := summary.Close()
				if err != nil {
					yield(nil, err)
					return
				}
				if closeErr != nil {
					yield(nil, closeErr)
					return
				}
				if len(groups) > 4096 || len(resources) > 4096*4 {
					yield(nil, invalid("usage report exceeds the group or resource limit; narrow the filters"))
					return
				}
			}
			for i := range n {
				r := &block[i]
				r.scope = scopes[r.key]
				if r.scope == nil {
					yield(nil, invalid("missing usage scope"))
					return
				}
				if !yield(r, nil) {
					return
				}
			}
			if n < blockSize {
				return
			}
		}
	}
}

func decodeUsageScope(rows *sql.Rows, bySequence map[uint64]usageScopeKey, dimension protocol.UsageDimension, observations bool, groups map[string]*protocol.UsageGroup, resources map[string]protocol.UsageKey) (usageScopeKey, *usageRowScope, error) {
	var providerText, model, worktreeText, scopeName string
	var projectText, api, providerLabel, resourceText, assistantText, preset sql.NullString
	var credentialText *string
	var seq uint64
	if err := rows.Scan(&seq, &providerText, &model, &projectText, &worktreeText, &api, &credentialText, &providerLabel, &scopeName, &resourceText, &assistantText, &preset); err != nil {
		return usageScopeKey{}, nil, err
	}
	k := bySequence[seq]
	provider, e := protocol.ParseProviderID(providerText)
	if e != nil {
		return k, nil, e
	}
	worktree, e := protocol.ParseWorktreeID(worktreeText)
	if e != nil {
		return k, nil, e
	}
	session, e := protocol.ParseSessionID(k.session)
	if e != nil {
		return k, nil, e
	}
	turn, e := protocol.ParseTurnID(k.turn)
	if e != nil {
		return k, nil, e
	}
	var project *protocol.ProjectID
	if projectText.Valid {
		id, e := protocol.ParseProjectID(projectText.String)
		if e != nil {
			return k, nil, e
		}
		project = &id
	}
	providerName := providerText
	if providerLabel.Valid {
		providerName = providerLabel.String
	}
	var credential *protocol.CredentialID
	if observations && credentialText != nil {
		id, err := protocol.ParseCredentialID(*credentialText)
		if err != nil {
			return k, nil, err
		}
		credential = &id
	}
	owner := protocol.UsageKey{Kind: "session", Data: session}
	location := owner
	if project != nil {
		owner = protocol.UsageKey{Kind: "project", Data: *project}
		location = protocol.UsageKey{Kind: "worktree", Data: protocol.UsageKeyWorktreePayload{Project: *project, Worktree: worktree}}
	} else if resourceText.Valid {
		var resource protocol.ConnectionResource
		if e = json.Unmarshal([]byte(resourceText.String), &resource); e != nil {
			return k, nil, e
		}
		owner = protocol.UsageKey{Kind: "connection", Data: resource}
		location = owner
	} else if assistantText.Valid {
		var assistant protocol.PluginConversationBinding
		if e = json.Unmarshal([]byte(assistantText.String), &assistant); e != nil {
			return k, nil, e
		}
		owner = protocol.UsageKey{Kind: "assistant", Data: protocol.UsageKeyAssistantPayload{Package: assistant.Package.Name, ID: assistant.ID}}
		location = owner
	}
	keys := []protocol.UsageKey{{Kind: "provider", Data: provider}, {Kind: "model", Data: protocol.UsageKeyModelPayload{Provider: provider, Model: model}}, owner, location}
	index := 0
	switch dimension {
	case protocol.UsageDimensionModel:
		index = 1
	case protocol.UsageDimensionProject:
		index = 2
	case protocol.UsageDimensionWorktree:
		index = 3
	}
	key := keys[index]
	if dimension == protocol.UsageDimensionSession {
		key = protocol.UsageKey{Kind: "session", Data: session}
	}
	if dimension == protocol.UsageDimensionCredential {
		key = protocol.UsageKey{Kind: "unassigned", Data: struct{}{}}
		if credential != nil {
			key = protocol.UsageKey{Kind: "credential", Data: *credential}
		}
	}
	keyText := usageKeyText(key)
	group := groups[keyText]
	if group == nil {
		g := protocol.DefaultUsageGroup()
		g.Key = key
		group = &g
		groups[keyText] = group
	}
	var groupName *string
	name := providerName
	if index == 2 {
		name = scopeName
	}
	if index < 3 && name != "" && dimension != protocol.UsageDimensionSession && dimension != protocol.UsageDimensionCredential {
		v := name
		groupName = &v
	}
	for _, key := range keys {
		resources[usageKeyText(key)] = key
	}
	return k, &usageRowScope{base: protocol.UsageRequest{Session: session, Turn: turn, Project: project, ScopeName: scopeName, Worktree: worktree, Provider: provider, ProviderName: providerName, Model: model, Credential: credential}, preset: preset.String, api: api.String, group: group, groupKey: keyText, groupName: groupName}, nil
}
