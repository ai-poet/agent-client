package store

import (
	"bufio"
	"compress/gzip"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"io/fs"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/core"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func CheckTraceSession(ctx context.Context, db *sql.Conn, session protocol.SessionID) error {
	var exists bool
	err := db.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND state!='removed')", session.String()).Scan(&exists)
	if err == nil && !exists {
		return fault(protocol.ErrorCodeNotFound, "session does not exist on this Node")
	}
	return err
}

// ReadTrace only opens the index for listing. A body request opens exactly
// one regular file under the authorized session's bodies directory.
func ReadTrace(ctx context.Context, profile string, query protocol.TraceQuery) (protocol.TracePage, error) {
	page := protocol.TracePage{Session: query.Session, Body: query.Body}
	name := "trace.jsonl"
	if query.Body != "" {
		base, ok := strings.CutPrefix(query.Body, "bodies/")
		if !ok || !fs.ValidPath(base) || strings.ContainsAny(base, "/\\:") || !(strings.HasSuffix(base, ".req.gz") || strings.HasSuffix(base, ".res.gz")) || query.After != "" || query.Limit != 0 {
			return page, invalid("invalid trace body reference")
		}
		name = query.Body
	} else if query.Limit > 100 {
		return page, invalid("trace page limit exceeds 100")
	}
	files, err := core.OpenFiles(profile, core.RestrictedFiles)
	if err != nil {
		return page, err
	}
	defer files.Close()
	file, err := files.Open("traces/" + query.Session.String() + "/" + name)
	if err != nil {
		var f *protocol.Fault
		if query.Body == "" && query.After == "" && errors.As(err, &f) && f.Code == protocol.ErrorCodeNotFound {
			return page, nil
		}
		return page, err
	}
	defer file.Close()
	if query.Body != "" {
		z, err := gzip.NewReader(file)
		if err != nil {
			return page, err
		}
		defer z.Close()
		const preview = 256 << 10
		content, err := io.ReadAll(io.LimitReader(z, preview+1))
		if err != nil {
			return page, err
		}
		page.Content = content[:min(len(content), preview)]
		page.Truncated = len(content) > preview
		return page, nil
	}
	limit := int(query.Limit)
	if limit == 0 {
		limit = 32
	}
	scan := bufio.NewScanner(file)
	scan.Buffer(make([]byte, 4096), 1<<20)
	// Cursors identify a complete record rather than a row number, so clear
	// followed by fresh capture cannot silently continue an unrelated page.
	found := query.After == ""
	var last string
	var size int
	for scan.Scan() {
		if err := ctx.Err(); err != nil {
			return page, err
		}
		line := scan.Bytes()
		if found && (len(page.Records) == limit || size+len(line) > 2<<20) {
			page.Next = last
			break
		}
		sum := sha256.Sum256(line)
		last = hex.EncodeToString(sum[:])
		if !found {
			found = last == query.After
			continue
		}
		if len(line) == 0 || line[0] != '{' || !json.Valid(line) {
			return page, invalid("invalid trace index record")
		}
		page.Records = append(page.Records, append(json.RawMessage(nil), line...))
		size += len(line)
	}
	if err := scan.Err(); err != nil {
		return page, err
	}
	if !found {
		return page, fault(protocol.ErrorCodeRevisionConflict, "trace cursor no longer exists")
	}
	return page, nil
}
