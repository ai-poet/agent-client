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
	"fmt"
	"io"
	"regexp"
	"slices"
	"strings"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/core"
	capture "github.com/ai-poet/cheaprouter-harness/internal/node/agent/trace"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

var tooLarge = &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: protocol.TraceExportTooLargeMessage}

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
	Verdict   *protocol.TraceVerdict `json:"verdict,omitempty"`
	Version   int                    `json:"v"`
	Session   protocol.SessionID     `json:"session"`
	Turn      protocol.TurnID        `json:"turn"`
	ID        string                 `json:"request_id"`
	Attempt   int                    `json:"attempt"`
	Timestamp int64                  `json:"ts"`
	Outcome   string                 `json:"outcome"`
	Status    int                    `json:"status"`
	Purpose   string                 `json:"purpose,omitempty"`
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
func Export(ctx context.Context, profile string, q protocol.TraceExportQuery, w io.Writer) error {
	if q.Session == (protocol.SessionID{}) || (q.Mode != "raw" && q.Mode != "shareable") || (q.Scope != "all" && q.Scope != "failed" && q.Scope != "turn") || ((q.Scope == "turn") != (q.Turn != (protocol.TurnID{}))) {
		return invalid("invalid trace export selection")
	}
	release := capture.ReadLease(profile, q.Session.String())
	defer release()
	fs, err := core.OpenFiles(profile, core.RestrictedFiles)
	if err != nil {
		return err
	}
	defer fs.Close()
	prefix := "traces/" + q.Session.String() + "/"
	f, err := fs.Open(prefix + "trace.jsonl")
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
	type entry struct {
		name  string
		data  []byte
		known bool
	}
	bodies := map[string]*entry{}
	var order []string
	var records [][]byte
	secrets := promptSet{}
	retained := protocol.MaxTraceExportBytes
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
		records = append(records, bytes.Clone(line))
		rows = append(rows, r)
		for i, src := range []string{r.Request.Body, r.Response.Body} {
			if src == "" {
				continue
			}
			suffix := []string{".req.gz", ".res.gz"}[i]
			if !bodyPath.MatchString(src) || !strings.HasSuffix(src, suffix) {
				return invalid("invalid trace export body reference")
			}
			if bodies[src] != nil {
				continue
			}
			file, e := fs.Open(prefix + src)
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
			plain, e := inflate(blob, expanded)
			if e != nil {
				return e
			}
			expanded -= len(plain)
			ent := &entry{name: src, data: blob}
			if q.Mode == "shareable" {
				ent.name = fmt.Sprintf("bodies/%04d%s", len(order)+1, suffix)
				ent.data = plain
				if i == 0 {
					ent.data, ent.known, e = secrets.request(plain)
					if e != nil {
						return e
					}
				}
			}
			if len(ent.data) > retained {
				return tooLarge
			}
			retained -= len(ent.data)
			bodies[src] = ent
			order = append(order, src)
		}
	}
	if err = scanner.Err(); err != nil {
		return err
	}
	release()
	// Discover protected text throughout the selection before writing any entry:
	// a later request can contain a prompt echoed by an earlier attempt.
	scrub := secrets.replacer()
	unsafeResponse := map[string]bool{}
	for i, r := range rows {
		if err = ctx.Err(); err != nil {
			return err
		}
		line := records[i]
		if q.Mode == "shareable" {
			req := bodies[r.Request.Body]
			if req == nil || !req.known || r.Bytes.ReqTruncated {
				unsafeResponse[r.Response.Body] = true
			}
			var meta map[string]any
			if !decodeJSON(line, &meta) {
				return invalid("invalid trace metadata")
			}
			delete(meta, "segment_hashes_v1")
			// Detail names can be derived from a protected skill list or tool
			// schema; masking the original full prompt doesn't mask that fragment.
			if verdict, ok := meta["verdict"].(map[string]any); ok {
				if key, ok := verdict["cause_key"].(string); ok {
					family, _, _ := strings.Cut(key, ":")
					switch family {
					case "skill", "tool-def", "tool-call", "tool-result", "message":
						cause := family
						if strings.HasSuffix(key, ":volatile") {
							cause += ":volatile"
						}
						verdict["cause_key"], meta["cause_key"] = cause, cause
					}
				}
			}
			meta = cleanJSON(meta, scrub).(map[string]any)
			for _, v := range []struct{ key, ref string }{{"request", r.Request.Body}, {"response", r.Response.Body}} {
				if ent := bodies[v.ref]; ent != nil {
					side, ok := meta[scrub.Replace(v.key)].(map[string]any)
					if !ok {
						return invalid("invalid trace body metadata")
					}
					side["body"] = ent.name
				}
			}
			if red, ok := meta["redaction"].(map[string]any); ok {
				red["system_prompt"] = "fingerprinted"
			}
			line, err = json.Marshal(meta)
			if err != nil {
				return err
			}
			rows[i].Provider.Model = scrub.Replace(r.Provider.Model)
			rows[i].Response.Gateway = scrub.Replace(r.Response.Gateway)
		}
		if len(line)+index.Len()+1 > a.left {
			return tooLarge
		}
		index.Write(line)
		index.WriteByte('\n')
	}
	sharedBudget := protocol.MaxTraceExportBytes
	for _, src := range order {
		if err = ctx.Err(); err != nil {
			return err
		}
		ent := bodies[src]
		blob := ent.data
		if q.Mode == "shareable" {
			var safe []byte
			switch {
			case strings.HasSuffix(src, ".req.gz") && !ent.known, unsafeResponse[src]:
				safe = []byte(marker("body omitted", blob))
			default:
				safe = safeEvidence(blob, scrub)
			}
			if len(safe) > sharedBudget || len(safe) > 16<<20 {
				return tooLarge
			}
			sharedBudget -= len(safe)
			var out bytes.Buffer
			gz := gzip.NewWriter(&out)
			if _, err = gz.Write(safe); err != nil {
				return err
			}
			if err = gz.Close(); err != nil {
				return err
			}
			blob = out.Bytes()
		}
		if err = a.add(ent.name, blob, zip.Store); err != nil {
			return err
		}
		ent.data = nil
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
func inflate(blob []byte, remaining int) ([]byte, error) {
	z, err := gzip.NewReader(bytes.NewReader(blob))
	if err != nil {
		return nil, err
	}
	defer z.Close()
	limit := min(remaining, 16<<20)
	b, err := io.ReadAll(io.LimitReader(z, int64(limit)+1))
	if len(b) > limit {
		return nil, tooLarge
	}
	if err != nil {
		return nil, err
	}
	return b, nil
}

const readme = `Sailry request trace
trace.jsonl contains one captured HTTP attempt per line:
  session, turn, request_id, attempt: correlation keys; attempt is one-based within a request.
  ts: start time in Unix milliseconds. purpose and provider identify the route.
  status, outcome, response.end: HTTP status (0 if unavailable), model outcome, and transport end.
  failure, retry: observed failure kind and resend decision (delay_ms in milliseconds).
  timing: offsets from dispatch preparation to the reported milestones, in microseconds.
  usage: reported input, output, reasoning, cache read/write tokens (not inferred costs).
  bytes, digest: original captured byte counts, truncation flags and SHA-256 digests.
  request, response: allowed headers, names of dropped headers, optional body reference.
Missing fields are unknown. Paths in request.body and response.body refer to gzip entries.
manifest.json lists SHA-256 (hex) for each other ZIP entry, over the entry's exact bytes.
Raw exports retain local capture redactions but may contain system prompts, code, user data
and credentials missed by capture. Do not share raw exports without inspection.
Shareable exports preserve capture-redacted headers, model, gateway identifiers, ordinary
messages, and response evidence. System/developer instructions and tool schemas are
fingerprinted; matching prompt strings are masked in captured evidence. Unsupported request
formats and responses without a usable captured request receive a whole-body fingerprint.
Original capture digests describe wire bytes; manifest checksums describe exported files.
User code, conversation text, endpoint information and other private data can remain.
Inspect every export before sharing; this is not anonymization.
`
