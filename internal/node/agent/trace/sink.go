package trace

import (
	"bytes"
	"compress/gzip"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/core"
)

func component(s string) bool {
	if !filepath.IsLocal(s) || len(s) > 128 || s == "." || strings.HasSuffix(s, ".") {
		return false
	}
	for _, c := range s {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_' || c == '-' || c == '.') {
			return false
		}
	}
	return true
}

func Clear(profile, session string) error {
	if profile == "" || !component(session) {
		return fs.ErrInvalid
	}
	profile, err := filepath.Abs(profile)
	if err != nil {
		return err
	}
	publication.Lock()
	defer publication.Unlock()
	for a := range publication.active {
		if a.scope.Profile == profile && a.scope.Session == session {
			delete(publication.active, a)
		}
	}
	base := filepath.Join(profile, "traces")
	if _, err := os.Lstat(base); errors.Is(err, os.ErrNotExist) {
		return nil
	} else if err != nil {
		return err
	}
	if err := core.PrivateDirectory(base); err != nil {
		return err
	}
	root, err := os.OpenRoot(base)
	if err != nil {
		return err
	}
	defer root.Close()
	return root.RemoveAll(session)
}

func (a *Attempt) prepare(c *capture, response bool) []byte {
	if !c.seen || c.truncated {
		return nil
	}
	var data []byte
	var ok bool
	ct := strings.ToLower(c.headers.Headers.Get("Content-Type"))
	if response && a.encoding != "" && a.encoding != "identity" && a.encoding != "identity(decompressed)" {
		c.truncated = true
		return nil
	}
	if strings.HasPrefix(ct, "text/event-stream") || bytes.HasPrefix(bytes.TrimSpace(c.data), []byte("data:")) || bytes.Contains(c.data, []byte("\ndata:")) || bytes.Contains(c.data, []byte("\rdata:")) {
		data, ok = redactSSE(c.data, a.secrets)
	} else {
		data, ok = payload(c.data, a.secrets)
	}
	if !ok || len(data) > maxBody {
		c.truncated = true
		return nil
	}
	var b bytes.Buffer
	z := gzip.NewWriter(&b)
	_, err := z.Write(data)
	if z.Close() != nil || err != nil {
		c.truncated = true
		return nil
	}
	return b.Bytes()
}

type sizes struct{ total, session int64 }

func diskUsage(base, session string) (s sizes, err error) {
	err = filepath.WalkDir(base, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		i, err := d.Info()
		if err != nil {
			return err
		}
		s.total += i.Size()
		if strings.HasPrefix(path, session+string(filepath.Separator)) {
			s.session += i.Size()
		}
		return nil
	})
	return
}

type rowKey struct {
	RequestID string            `json:"request_id"`
	Request   headerSnapshot    `json:"request"`
	Digest    map[string]string `json:"digest"`
}

func (a *Attempt) publish(f Facts, req, res []byte, sessionLimit, profileLimit int64, roundLimit int) {
	base := filepath.Join(a.scope.Profile, "traces")
	dir := filepath.Join(base, a.scope.Session)
	for _, path := range []string{base, dir, filepath.Join(dir, "bodies")} {
		if core.PrivateDirectory(path) != nil {
			return
		}
	}
	root, err := os.OpenRoot(dir)
	if err != nil {
		return
	}
	defer root.Close()
	usage, err := diskUsage(base, dir)
	if err != nil {
		return
	}
	budget := min(profileLimit-usage.total, sessionLimit-usage.session)
	if budget <= 0 {
		return
	}
	const index = "trace.jsonl"
	if i, err := root.Lstat(index); err == nil {
		if !i.Mode().IsRegular() || (runtime.GOOS != "windows" && i.Mode().Perm()&0077 != 0) {
			return
		}
	} else if !errors.Is(err, fs.ErrNotExist) {
		return
	}

	log, err := root.OpenFile(index, os.O_RDWR|os.O_CREATE|os.O_APPEND, 0600)
	if err != nil {
		return
	}
	defer log.Close()
	digest := a.req.digest()
	dec := json.NewDecoder(log)
	for count := 0; ; count++ {
		if count >= roundLimit {
			return
		}
		var old rowKey
		err := dec.Decode(&old)
		if err == io.EOF {
			break
		}
		if err != nil {
			return
		}
		if req != nil && old.RequestID == a.request && digest == old.Digest["req"] && old.Request.Body != "" {
			// Secrets can change between retries. Never reuse a blob sanitized
			// under a different set of credentials unless the result is identical.
			if filepath.Dir(filepath.FromSlash(old.Request.Body)) == "bodies" && sameBlob(root, old.Request.Body, req) {
				a.req.headers.Body = old.Request.Body
				req = nil
			}
		}
	}
	stem := fmt.Sprintf("bodies/%s-%d", a.request, a.index)
	if req != nil {
		a.req.headers.Body = stem + ".req.gz"
	}
	if res != nil {
		a.res.headers.Body = stem + ".res.gz"
	}
	if f.Status == 0 {
		f.Status = a.status
	}

	// Reserve the JSONL line as well as both compressed sidecars. If bytes
	// are scarce, prefer a truthful metadata-only record to orphan bodies.
	var line []byte
	for {
		line, err = a.line(f)
		if err != nil {
			return
		}
		if int64(len(line)+len(req)+len(res)) <= budget {
			break
		}
		if res != nil {
			res = nil
			a.res.headers.Body = ""
			a.res.truncated = true
			continue
		}
		if req != nil {
			req = nil
			a.req.headers.Body = ""
			a.req.truncated = true
			continue
		}
		return
	}
	var created []string
	defer func() {
		for _, p := range created {
			_ = root.Remove(p)
		}
	}()
	for _, part := range []struct {
		c    *capture
		data []byte
	}{{&a.req, req}, {&a.res, res}} {
		if part.data == nil {
			continue
		}
		name := part.c.headers.Body
		file, err := root.OpenFile(name, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
		if err == nil {
			created = append(created, name)
			_, err = file.Write(part.data)
			err = errors.Join(err, file.Close())
			if err != nil && root.Remove(name) != nil {
				return
			}
		}
		if err != nil {
			// The index may still be writable (e.g. a body name collision).
			// Never publish a dangling pointer or abort the model attempt.
			part.c.headers.Body = ""
			part.c.truncated = true
			line, err = a.line(f)
			if err != nil {
				return
			}
		}
	}
	offset, err := log.Seek(0, io.SeekEnd)
	if err != nil {
		return
	}
	if _, err := log.Write(line); err != nil {
		_ = log.Truncate(offset)
		return
	}
	created = nil
}

func sameBlob(root *os.Root, path string, want []byte) bool {
	i, err := root.Lstat(path)
	if err != nil || !i.Mode().IsRegular() || i.Size() != int64(len(want)) {
		return false
	}
	f, err := root.Open(path)
	if err != nil {
		return false
	}
	defer f.Close()
	got, err := io.ReadAll(io.LimitReader(f, int64(len(want))+1))
	return err == nil && bytes.Equal(got, want)
}

func (a *Attempt) line(f Facts) ([]byte, error) {
	// Keep the session identifier distinct from arbitrary JSON "session"
	// fields, which are always sensitive inside provider bodies and failures.
	m := map[string]any{
		"v": 1, "ts": f.StartedMs, "turn": a.scope.Turn, "request_id": a.request, "attempt": a.index, "purpose": a.scope.Purpose,
		"provider": a.scope.Provider, "outcome": f.Outcome, "status": f.Status,
		"timing": f.Timing, "usage": f.Usage, "retry": f.Retry, "failure": f.Failure,
		"request": a.req.headers,
		"response": struct {
			headerSnapshot
			End         string `json:"end,omitempty"`
			Gateway     string `json:"gateway_request_id,omitempty"`
			Encoding    string `json:"encoding,omitempty"`
			ContentType string `json:"content_type,omitempty"`
		}{a.res.headers, f.EndKind, f.GatewayRequestID, a.encoding, a.res.headers.Headers.Get("Content-Type")},
		"bytes":     map[string]any{"req": a.req.total, "res": a.res.total, "req_truncated": a.req.truncated, "res_truncated": a.res.truncated},
		"digest":    map[string]string{"req": a.req.digest(), "res": a.res.digest()},
		"redaction": map[string]string{"headers": "allowlist", "body": "keys+secrets", "system_prompt": "included"},
	}
	if a.scope.Record != "" {
		m["record"] = a.scope.Record
	}
	raw, err := json.Marshal(m)
	if err != nil {
		return nil, err
	}
	raw = redactJSON(raw, a.secrets)
	session, _ := json.Marshal(scrub(a.scope.Session, a.secrets))
	raw = append(raw[:len(raw)-1], `,"session":`...)
	raw = append(raw, session...)
	return append(raw, '}', '\n'), nil
}
