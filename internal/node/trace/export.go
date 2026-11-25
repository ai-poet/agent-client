// Package trace exports controller-authorized session captures.
package trace

import (
	"archive/zip"
	"bufio"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/ai-poet/cheaprouter-harness/internal/core"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

var tooLarge = &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "trace export exceeds the limit; select a narrower scope"}

func invalid(s string) error {
	return &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: s}
}

type limitedWriter struct {
	w    io.Writer
	left int
}

func (w *limitedWriter) Write(p []byte) (int, error) {
	if len(p) > w.left {
		return 0, tooLarge
	}
	n, e := w.w.Write(p)
	w.left -= n
	return n, e
}

type archive struct {
	z    *zip.Writer
	sums map[string]string
	left int
}

func (a *archive) add(name string, b []byte, method uint16) error {
	if len(b) > a.left {
		return tooLarge
	}
	a.left -= len(b)
	w, err := a.z.CreateHeader(&zip.FileHeader{Name: name, Method: method})
	if err != nil {
		return err
	}
	if _, err = w.Write(b); err != nil {
		return err
	}
	a.sums[name] = digest(b)
	return nil
}
func digest(b []byte) string { return fmt.Sprintf("%x", sha256.Sum256(b)) }

var bodyPath = regexp.MustCompile(`^bodies/[a-zA-Z0-9_.-]{1,150}\.(req|res)\.gz$`)

type bodyRef struct {
	Body string `json:"body,omitempty"`
}
type row struct {
	Version   int                `json:"v"`
	Session   protocol.SessionID `json:"session"`
	Turn      protocol.TurnID    `json:"turn"`
	ID        string             `json:"request_id"`
	Attempt   int                `json:"attempt"`
	Timestamp int64              `json:"ts"`
	Outcome   string             `json:"outcome"`
	Status    int                `json:"status"`
	Purpose   string             `json:"purpose,omitempty"`
	Failure   *struct {
		Kind string `json:"kind"`
	} `json:"failure,omitempty"`
	Bytes struct {
		Req          int64 `json:"req"`
		Res          int64 `json:"res"`
		ReqTruncated bool  `json:"req_truncated"`
		ResTruncated bool  `json:"res_truncated"`
	} `json:"bytes"`
	Retry *struct {
		Index int   `json:"index"`
		Limit int   `json:"limit"`
		Delay int64 `json:"delay_ms"`
	} `json:"retry,omitempty"`
	Request  bodyRef `json:"request"`
	Response struct {
		bodyRef
		End     string `json:"end,omitempty"`
		Gateway string `json:"gateway_request_id,omitempty"`
	} `json:"response"`
	Provider struct {
		Model string `json:"model"`
	} `json:"provider"`
	Timing struct {
		Elapsed *uint64 `json:"elapsed_us,omitempty"`
		First   *uint64 `json:"first_token_us,omitempty"`
	} `json:"timing"`
	Usage struct {
		Input      *uint64 `json:"input,omitempty"`
		Output     *uint64 `json:"output,omitempty"`
		CacheRead  *uint64 `json:"cache_read,omitempty"`
		CacheWrite *uint64 `json:"cache_write,omitempty"`
	} `json:"usage"`
}

func (r row) failed() bool { return r.Outcome == "failed" }
func (r *row) share() {
	// Free-form identifiers and names can themselves contain prompts or secrets.
	if _, err := protocol.ParseRequestID(r.ID); err != nil {
		r.ID = digest([]byte(r.ID))
	}
	if r.Provider.Model != "" {
		r.Provider.Model = digest([]byte(r.Provider.Model))
	}
	switch r.Purpose {
	case "main", "summary", "native_compaction":
	default:
		r.Purpose = ""
	}
	if r.Failure != nil {
		r.Failure.Kind = safeFailure(r.Failure.Kind)
	}
	r.Response.Gateway = ""
	switch r.Outcome {
	case "succeeded", "failed", "cancelled":
	default:
		r.Outcome = "unknown"
	}
	switch r.Response.End {
	case "completed", "client-abort", "http-error", "stream-truncated", "upstream-timeout", "upstream-unreachable", "upstream-reset", "stream-error":
	default:
		r.Response.End = ""
	}
}

func Export(ctx context.Context, profile string, q protocol.TraceExportQuery, w io.Writer) error {
	if q.Session == (protocol.SessionID{}) || (q.Mode != "raw" && q.Mode != "shareable") || (q.Scope != "all" && q.Scope != "failed" && q.Scope != "turn") || ((q.Scope == "turn") != (q.Turn != (protocol.TurnID{}))) {
		return invalid("invalid trace export selection")
	}
	fs, err := core.OpenFiles(filepath.Join(profile, "traces", q.Session.String()), core.RestrictedFiles)
	if err != nil {
		return err
	}
	defer fs.Close()
	f, err := fs.Open("trace.jsonl")
	if err != nil {
		return err
	}
	defer f.Close()
	stat, err := f.Stat()
	if err != nil {
		return err
	}
	if stat.Size() > protocol.MaxTraceExportBytes {
		return tooLarge
	}
	// Freeze the index length: concurrent appends cannot extend this selection.
	scanner := bufio.NewScanner(io.LimitReader(f, stat.Size()))
	scanner.Buffer(make([]byte, 4096), 1<<20)
	a := archive{z: zip.NewWriter(&limitedWriter{w, protocol.MaxTraceExportBytes}), sums: map[string]string{}, left: protocol.MaxTraceExportBytes}
	var index bytes.Buffer
	var rows []row
	names := map[string]string{}
	expanded, compressed := protocol.MaxTraceExportBytes, protocol.MaxTraceExportBytes
	for count := 0; scanner.Scan(); count++ {
		if err = ctx.Err(); err != nil {
			return err
		}
		if count >= 200 {
			return tooLarge
		}
		line := scanner.Bytes()
		var r row
		if err = json.Unmarshal(line, &r); err != nil || r.Version != 1 || r.Session != q.Session {
			return invalid("invalid trace export record")
		}
		if q.Scope == "failed" && !r.failed() || q.Scope == "turn" && q.Turn != r.Turn {
			continue
		}
		if q.Mode == "shareable" {
			r.share()
		}
		for i, ref := range []*string{&r.Request.Body, &r.Response.Body} {
			src := *ref
			if src == "" {
				continue
			}
			suffix := []string{".req.gz", ".res.gz"}[i]
			if !bodyPath.MatchString(src) || !strings.HasSuffix(src, suffix) {
				return invalid("invalid trace export body reference")
			}
			if existing, ok := names[src]; ok {
				*ref = existing
				continue
			}
			file, e := fs.Open(src)
			if e != nil {
				return e
			}
			limit := min((16<<20)+(1<<16), compressed)
			blob, e := io.ReadAll(io.LimitReader(file, int64(limit)+1))
			file.Close()
			if e != nil {
				return e
			}
			if len(blob) > limit {
				return tooLarge
			}
			compressed -= len(blob)
			blob, used, e := exportBody(blob, q.Mode, expanded, i == 0)
			if e != nil {
				return e
			}
			expanded -= used
			if q.Mode == "shareable" {
				*ref = fmt.Sprintf("bodies/%04d%s", len(names)+1, suffix)
			}
			names[src] = *ref
			if err = a.add(*ref, blob, zip.Store); err != nil {
				return err
			}
		}
		if q.Mode == "shareable" {
			line, err = json.Marshal(r)
			if err != nil {
				return err
			}
		}
		if len(line)+index.Len()+1 > a.left {
			return tooLarge
		}
		index.Write(line)
		index.WriteByte('\n')
		rows = append(rows, r)
	}
	if err = scanner.Err(); err != nil {
		return err
	}
	if err = a.add("trace.jsonl", index.Bytes(), zip.Deflate); err != nil {
		return err
	}
	if err = a.add("summary.md", summary(rows), zip.Deflate); err != nil {
		return err
	}
	if err = a.add("README.txt", []byte(readme), zip.Deflate); err != nil {
		return err
	}
	models, gateways := map[string]bool{}, map[string]bool{}
	for _, r := range rows {
		if r.Provider.Model != "" {
			models[r.Provider.Model] = true
		}
		if r.Response.Gateway != "" {
			gateways[r.Response.Gateway] = true
		}
	}
	manifest := map[string]any{"schemaVersion": 1, "session": q.Session, "scope": q.Scope, "exported_at": time.Now().UTC(), "redaction": q.Mode, "model": keys(models), "gateway_request_ids": keys(gateways), "files": a.sums}
	if q.Scope == "turn" {
		manifest["turn"] = q.Turn
	}
	b, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return err
	}
	if err = a.add("manifest.json", b, zip.Deflate); err != nil {
		return err
	}
	if err = ctx.Err(); err != nil {
		return err
	}
	return a.z.Close()
}
func keys(m map[string]bool) []string {
	out := make([]string, 0, len(m))
	for s := range m {
		out = append(out, s)
	}
	slices.Sort(out)
	return out
}
func exportBody(blob []byte, mode string, remaining int, request bool) ([]byte, int, error) {
	z, err := gzip.NewReader(bytes.NewReader(blob))
	if err != nil {
		return nil, 0, err
	}
	defer z.Close()
	limit := min(remaining, 16<<20)
	b, err := io.ReadAll(io.LimitReader(z, int64(limit)+1))
	if len(b) > limit {
		return nil, 0, tooLarge
	}
	if err != nil {
		return nil, 0, err
	}
	if mode == "raw" {
		return blob, len(b), nil
	}
	// Unknown provider formats, echoed requests, SSE and arbitrary tool results
	// cannot be safely scrubbed by looking for particular field names.
	var out bytes.Buffer
	g := gzip.NewWriter(&out)
	_, err = fmt.Fprintf(g, "<body omitted: %d chars, sha256:%s>", utf8.RuneCount(b), digest(b))
	if err == nil && request {
		_, err = g.Write(promptMarkers(b))
	}
	err = errors.Join(err, g.Close())
	return out.Bytes(), len(b), err
}

const readme = `Sailry request trace
trace.jsonl contains one captured HTTP attempt per line. Timing is in microseconds.
Missing fields are unknown. Paths in request.body and response.body refer to gzip entries.
manifest.json lists SHA-256 (hex) for each other ZIP entry, over the entry's exact bytes.
Raw exports retain local capture redactions but may contain system prompts, code, user data
and credentials missed by capture. Do not share raw exports without inspection.
Shareable exports omit headers, hash model names, and replace each complete captured body
(including system prompts, tool definitions, user messages and responses) with its character
count and SHA-256. Timing, identifiers and hashes remain correlatable; review before sharing.
`
