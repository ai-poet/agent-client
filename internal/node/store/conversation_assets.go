package store

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"math"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// Leave the same reply headroom as history pages for metadata and the Link envelope.
const conversationAssetPageBytes = protocol.MAX_FRAME_BYTES - 64*1024

// ListConversationAssets derives bounded inventory pages from canonical history.
// It does not index workspace writes or inspect the filesystem.
func ListConversationAssets(ctx context.Context, db *sql.Conn, session protocol.SessionID, query protocol.ConversationAssetsQuery) (protocol.ConversationAssetsPage, error) {
	page := protocol.DefaultConversationAssetsPage()
	page.Session = session
	if err := requireSession(ctx, db, session); err != nil {
		return page, err
	}
	if query.Limit < 1 || query.Limit > 50 || query.Before != nil && (*query.Before == 0 || *query.Before > math.MaxInt64) {
		return page, invalid("conversation asset bounds are invalid")
	}
	if query.Before != nil {
		var visible bool
		if err := db.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM conversation_events WHERE session=?1 AND sequence=?2)", session.String(), *query.Before).Scan(&visible); err != nil {
			return page, err
		}
		if !visible {
			return page, fault(protocol.ErrorCodeWrongTarget, "asset cursor is not visible in this session")
		}
	}
	var err error
	if page.Revision, err = historyRevision(ctx, db, session); err != nil {
		return page, err
	}
	boundary := uint64(math.MaxInt64)
	if query.Before != nil {
		boundary = *query.Before - 1
	}
	oldest, more, err := conversationAssetBoundary(ctx, db, session, boundary)
	if err != nil || oldest == 0 {
		return page, err
	}
	rows, err := db.QueryContext(ctx, `SELECT h.sequence,e.turn,e.body`+conversationAssetMembers+` AND h.sequence>=?3 ORDER BY h.sequence DESC`, session.String(), boundary, oldest)
	if err != nil {
		return page, err
	}
	defer rows.Close()
	type location struct {
		worktree protocol.WorktreeID
		root     *string
	}
	locations := map[protocol.TurnID]location{}
	var last uint64
	remaining := conversationAssetPageBytes
	for rows.Next() {
		var sequence uint64
		var turnText string
		var body sql.RawBytes
		if err := rows.Scan(&sequence, &turnText, &body); err != nil {
			return page, err
		}
		turn, err := protocol.ParseTurnID(turnText)
		if err != nil {
			return page, err
		}
		var record canonical.Record
		if err = json.Unmarshal(body, &record); err != nil {
			return page, err
		}
		kind := protocol.ConversationAssetsKindArtifact
		if record.Provenance.Author == "user" {
			kind = protocol.ConversationAssetsKindResource
		}
		if query.Kind != nil && *query.Kind != kind {
			last = sequence
			continue
		}
		owner, ok := locations[turn]
		if !ok {
			run, err := visibleAgentRun(ctx, db, session, turn)
			if err != nil {
				return page, err
			}
			owner.worktree = run.Worktree
			var path string
			err = db.QueryRowContext(ctx, "SELECT path FROM worktrees WHERE id=?1", run.Worktree.String()).Scan(&path)
			if err == nil {
				owner.root = &path
			} else if !errors.Is(err, sql.ErrNoRows) {
				return page, err
			}
			locations[turn] = owner
		}
		items, err := conversationAssetItems(ctx, db, record, owner.worktree, owner.root)
		if err != nil {
			return page, err
		}
		if len(items) > 0 {
			group := protocol.ConversationAssetsGroup{Sequence: sequence, Turn: turn, Worktree: owner.worktree, TimestampMs: record.Created.UnixMilli(), Kind: kind, Items: items}
			body, err := encode(group)
			if err != nil {
				return page, err
			}
			size := len(body)
			if size > conversationAssetPageBytes {
				return page, invalid("conversation asset group exceeds the response frame budget")
			}
			if len(page.Groups) > 0 {
				size++ // comma between groups
			}
			if size > remaining {
				more = true
				break
			}
			remaining -= size
			page.Groups = append(page.Groups, group)
		}
		last = sequence
		if len(page.Groups) == int(query.Limit) {
			more = more || last > oldest
			break
		}
	}
	if err := errors.Join(rows.Err(), rows.Close()); err != nil {
		return page, err
	}
	if more {
		page.NextBefore = &last
	}
	return page, nil
}

const conversationAssetMembers = ` FROM conversation_events h CROSS JOIN agent_events e ON e.sequence=h.sequence WHERE h.session=?1 AND h.sequence<=?2`

func conversationAssetBoundary(ctx context.Context, db *sql.Conn, session protocol.SessionID, through uint64) (oldest uint64, more bool, err error) {
	rows, err := db.QueryContext(ctx, `SELECT h.sequence,`+historyStoredBytes+conversationAssetMembers+` ORDER BY h.sequence DESC LIMIT 257`, session.String(), through)
	if err != nil {
		return 0, false, err
	}
	defer rows.Close()
	var storedBytes int64
	for count := 0; rows.Next(); count++ {
		var sequence uint64
		var size int64
		if err := rows.Scan(&sequence, &size); err != nil {
			return 0, false, err
		}
		// One oversized stored event must advance on its own. Only fetch bodies
		// inside this window, even if the lookahead event is very large.
		if count == 256 || count > 0 && size > 4*1024*1024-storedBytes {
			more = true
			break
		}
		oldest = sequence
		storedBytes += size
	}
	return oldest, more, errors.Join(rows.Err(), rows.Close())
}

type conversationAssetTargets struct {
	items []protocol.ConversationAssetsTarget
	seen  map[string]bool
	err   error
}

func (targets *conversationAssetTargets) push(kind string, data any) {
	if targets.err != nil {
		return
	}
	value := protocol.ConversationAssetsTarget{Kind: kind, Data: data}
	body, err := encode(value)
	if err != nil {
		targets.err = err
		return
	}
	if targets.seen == nil {
		targets.seen = map[string]bool{}
	}
	if !targets.seen[string(body)] {
		targets.items = append(targets.items, value)
		targets.seen[string(body)] = true
	}
}
func (targets *conversationAssetTargets) file(path string) {
	targets.push("file", &protocol.ConversationAssetsTargetFilePayload{Path: path})
}
func (targets *conversationAssetTargets) image(record canonical.Record, worktree protocol.WorktreeID, part, index uint64, asset canonical.AssetRef) error {
	if !supportedAgentImage(asset) {
		return nil
	}
	value, err := describeAgentImage(record.Turn, worktree, record.ID, part, index, asset)
	if err == nil {
		targets.push("image", &value)
	}
	return err
}
func conversationAssetItems(ctx context.Context, db *sql.Conn, record canonical.Record, worktree protocol.WorktreeID, root *string) ([]protocol.ConversationAssetsTarget, error) {
	targets := &conversationAssetTargets{items: []protocol.ConversationAssetsTarget{}}
	message := record.Message()
	if message == nil {
		return targets.items, nil
	}
	hidden := record.Provenance.Author == "system" && record.Metadata.Context != nil && record.Metadata.Context.Captured
	if !hidden {
		skipped := uint64(0)
		if input := record.Metadata.Input; input != nil {
			if len(input.References) > 0 {
				skipped++
			}
		}
		for part := skipped; part < message.PartCount; part++ {
			if input := record.Metadata.Input; input != nil && ((input.CapabilityPart != nil && part == *input.CapabilityPart) || (input.NoticePart != nil && part == *input.NoticePart)) {
				continue
			}
			for _, block := range message.Content {
				if block.Index != part {
					continue
				}
				switch value := block.Data.(type) {
				case *canonical.TextContent:
					conversationAssetMarkdown(targets, value.Text, root, record.Provenance.Author == "user")
				case *canonical.AssetContent:
					if block.Kind == "image" {
						if err := targets.image(record, worktree, part, 0, value.Asset); err != nil {
							return nil, err
						}
					}
				case *canonical.FileContent:
					if strings.HasPrefix(value.File.URI, "sailry-attachment://") {
						id, err := protocol.ParseAttachmentID(strings.TrimPrefix(value.File.URI, "sailry-attachment://"))
						if err != nil {
							return nil, err
						}
						attachment, err := GetTurnAttachment(ctx, db, record.Turn, id)
						if err != nil {
							return nil, err
						}
						if value.File.Mime == nil || *value.File.Mime != attachment.Spec.MediaType {
							return nil, invalid("attachment media type mismatch")
						}
						targets.push("attachment", &attachment)
					}
				case *canonical.ToolResponse:
					if paths, ok := generatedImageAssetFiles(value); ok {
						for _, p := range paths {
							targets.file(p)
						}
						break
					}
					for index, asset := range conversationAssetImages(value) {
						if err := targets.image(record, worktree, part, uint64(index), asset); err != nil {
							return nil, err
						}
					}
				}
				break
			}
		}
		if input := record.Metadata.Input; input != nil {
			for _, reference := range input.References {
				if reference.Target.Kind == "file" {
					if path, ok := filePayload[string](reference.Target.Data); ok {
						targets.file(path)
					}
				}
			}
		}
	}
	for _, block := range message.Content {
		response, ok := block.Data.(*canonical.ToolResponse)
		if !ok || !conversationAssetResultMayContain(response.Result.Value, `"sailry_content"`) {
			continue
		}
		// Only file and artifact blocks contribute targets here. Preserve the full
		// decoder for escaped discriminators and values supplied by binary slots.
		body := response.Result.Value
		if len(response.Result.Assets) == 0 && !bytes.Contains(body, []byte(`"file"`)) && !bytes.Contains(body, []byte(`"artifact"`)) && !bytes.Contains(body, []byte(`\u`)) {
			continue
		}
		hydrated, err := HydrateCanonicalPayload(ctx, db, response.Result)
		if err != nil {
			return nil, err
		}
		raw := rawAgentTool{Presentation: protocol.ToolPresentationContent, Result: &hydrated, Format: response.Format}
		for index, asset := range conversationAssetImages(response) {
			if supportedAgentImage(asset) {
				raw.Images = append(raw.Images, protocol.ConversationImage{Index: uint64(index)})
			}
		}
		if content := raw.content(); content != nil {
			for _, item := range content.Blocks {
				switch item.Kind {
				case "file":
					if file, ok := filePayload[protocol.ToolContentFile](item.Data); ok {
						targets.file(file.Path)
					}
				case "artifact":
					if value, ok := filePayload[protocol.ToolContentBlockArtifactPayload](item.Data); ok {
						targets.push("document", &value.Artifact)
					}
				}
			}
		}
	}
	return targets.items, targets.err
}

func conversationAssetResultMayContain(value json.RawMessage, key string) bool {
	value = bytes.TrimSpace(value)
	// Binary slots replace an existing value with a string or byte array. They
	// cannot introduce a key or turn a non-object into a result envelope.
	// A Unicode escape can spell a key without the literal marker.
	return len(value) > 0 && value[0] == '{' && (bytes.Contains(value, []byte(key)) || bytes.Contains(value, []byte(`\u`)))
}

func conversationAssetImages(response *canonical.ToolResponse) []canonical.AssetRef {
	if response.Format == canonical.JSON && len(response.Result.Assets) != 0 && !conversationAssetResultMayContain(response.Result.Value, `"inline_data"`) {
		attachments := *response
		attachments.Result.Assets = nil
		return agentImageAssets(&attachments)
	}
	return agentImageAssets(response)
}

func generatedImageAssetFile(response *canonical.ToolResponse) (string, bool) {
	paths, ok := generatedImageAssetFiles(response)
	if !ok || len(paths) != 1 {
		return "", false
	}
	return paths[0], true
}

func generatedImageAssetFiles(response *canonical.ToolResponse) ([]string, bool) {
	if response.Name != "generate_image" || response.Status != canonical.ToolSuccess || response.Format != canonical.JSON || len(response.Attachments) == 0 || len(response.Result.Assets) != 0 {
		return nil, false
	}
	for _, attachment := range response.Attachments {
		if !supportedAgentImage(attachment.Asset) {
			return nil, false
		}
	}
	var result struct {
		Path      string   `json:"path"`
		Paths     []string `json:"paths"`
		MediaType string   `json:"media_type"`
		Size      uint64   `json:"size"`
	}
	if json.Unmarshal(response.Result.Value, &result) != nil {
		return nil, false
	}
	first := response.Attachments[0].Asset
	if len(response.Attachments) == 1 {
		if !validAssetRelative(result.Path) || result.MediaType != first.Mime || result.Size != first.Size {
			return nil, false
		}
		if len(result.Paths) > 0 && (len(result.Paths) != 1 || result.Paths[0] != result.Path) {
			return nil, false
		}
		// Native generation stores the same bytes in its file and preview attachment.
		// Prefer its literal workspace path so a later Markdown image publication
		// refers to one inventory target while canonical image access stays intact.
		return []string{result.Path}, true
	}
	if len(result.Paths) != len(response.Attachments) {
		return nil, false
	}
	if result.Path != "" && result.Path != result.Paths[0] {
		return nil, false
	}
	if result.MediaType != first.Mime || result.Size != first.Size {
		return nil, false
	}
	seen := make(map[string]bool, len(result.Paths))
	for _, p := range result.Paths {
		if !validAssetRelative(p) || seen[p] {
			return nil, false
		}
		seen[p] = true
	}
	return result.Paths, true
}
