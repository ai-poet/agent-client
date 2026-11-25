// Package trace stores opt-in private snapshots of provider HTTP attempts.
package trace

import (
	"crypto/sha256"
	"encoding/hex"
	"hash"
	"io"
	"net/http"
	"path/filepath"
	"slices"
	"strings"
	"sync"
)

const ProviderKey = "sailry.trace"

const (
	maxBody    = 16 << 20
	maxSession = 64 << 20
	maxProfile = 512 << 20
	maxRounds  = 200
)

type Scope struct {
	Profile, Session, Turn, Purpose string
	Record                          string // Current assistant draft, if one exists.
	Enabled                         func() bool
	GatewayEnabled                  func() bool
	Provider                        Provider
}

// Credential identifies the credential record, not its secret.
type Provider struct {
	ID           string `json:"id"`
	API          string `json:"api"`
	Auth         string `json:"auth"`
	Model        string `json:"model"`
	EndpointHost string `json:"endpoint_host"`
	Credential   string `json:"credential"`
}

type Facts struct {
	Round                              *Round
	Verdict                            *Verdict
	StartedMs                          int64
	Outcome, EndKind, GatewayRequestID string
	Status                             int
	Timing, Usage, Retry, Failure      any
	Categories                         any // measured context composition of this attempt
	CompleteContext                    bool
}

type Attempt struct {
	segments    []segment
	fingerprint string
	mu          sync.Mutex
	scope       Scope
	request     string
	index       int
	secrets     []string
	done        bool
	req, res    capture
	status      int
	encoding    string
}

type capture struct {
	headers   headerSnapshot
	data      []byte
	total     int64
	sum       hash.Hash
	truncated bool
	eof       bool
	expected  int64
	seen      bool
}

func (c *capture) add(p []byte) {
	if c.sum == nil {
		c.sum = sha256.New()
	}
	c.sum.Write(p)
	c.total += int64(len(p))
	// Never write an arbitrary prefix of a secret or partially parsed JSON value.
	if c.total > maxBody {
		c.truncated = true
		c.data = nil
	}
	if !c.truncated {
		c.data = append(c.data, p...)
	}
}
func (c *capture) digest() string {
	if c.sum == nil {
		return ""
	}
	return "sha256:" + hex.EncodeToString(c.sum.Sum(nil))
}

// Clear and publication share the admission lock. In-flight attempts invalidated by
// Clear cannot recreate the directory, even when they finish after a new Begin.
var publication = struct {
	sync.RWMutex
	active map[*Attempt]bool
}{active: make(map[*Attempt]bool)}

func (s *Scope) Begin(request string, index int, secrets ...string) *Attempt {
	if s == nil || s.Enabled != nil && !s.Enabled() || s.Profile == "" || !component(s.Session) || !component(request) || index < 0 {
		return nil
	}
	if scrub(request, secrets) != request || scrub(s.Session, secrets) != s.Session {
		return nil
	}
	root, err := filepath.Abs(s.Profile)
	if err != nil {
		return nil
	}
	a := &Attempt{scope: *s, request: request, index: index, secrets: append([]string(nil), secrets...)}
	slices.SortFunc(a.secrets, func(x, y string) int { return len(y) - len(x) })
	a.scope.Profile = root
	publication.Lock()
	publication.active[a] = true
	publication.Unlock()
	return a
}

func (a *Attempt) Request(h http.Header, p []byte) {
	if a == nil {
		return
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.done {
		return
	}
	// Capture the actual transport credentials as well as configuration
	// secrets: a protocol adapter may have substituted a per-request token.
	for k, values := range h {
		if !keySensitive(k) {
			continue
		}
		for _, value := range values {
			a.secrets = append(a.secrets, value)
			if strings.EqualFold(k, "Authorization") || strings.EqualFold(k, "Proxy-Authorization") {
				if _, token, ok := strings.Cut(value, " "); ok {
					a.secrets = append(a.secrets, strings.TrimSpace(token))
				}
			}
		}
	}
	for _, cookie := range (&http.Request{Header: h}).Cookies() {
		a.secrets = append(a.secrets, cookie.Value)
	}
	slices.SortFunc(a.secrets, func(a, b string) int { return len(b) - len(a) })
	a.secrets = slices.Compact(a.secrets)
	a.req = capture{headers: headers(h, a.secrets), seen: true}
	a.req.add(p)
}

func (a *Attempt) Response(r *http.Response) {
	if a == nil || r == nil {
		return
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.done {
		return
	}
	a.status = r.StatusCode
	a.res = capture{headers: headers(r.Header, a.secrets), seen: r.Body != nil, expected: r.ContentLength}
	a.encoding = r.Header.Get("Content-Encoding")
	if r.Uncompressed {
		a.encoding = "identity(decompressed)"
	}
	if r.Body != nil {
		r.Body = &body{ReadCloser: r.Body, a: a}
	}
}

type body struct {
	io.ReadCloser
	a *Attempt
}

func (b *body) Read(p []byte) (int, error) {
	n, err := b.ReadCloser.Read(p)
	b.a.mu.Lock()
	if !b.a.done {
		b.a.res.add(p[:n])
		if err == io.EOF {
			b.a.res.eof = true
		}
		if err != nil && err != io.EOF {
			b.a.res.truncated = true
		}
	}
	b.a.mu.Unlock()
	return n, err
}

func (a *Attempt) Finish(f Facts) {
	if a == nil {
		return
	}
	a.mu.Lock()
	if a.done {
		a.mu.Unlock()
		return
	}
	a.done = true
	a.mu.Unlock()

	defer func() { a.req.data = nil; a.res.data = nil; a.secrets = nil; a.segments = nil; a.fingerprint = "" }()
	// Compress outside the filesystem lock; transport readers are already detached.
	req := a.prepare(&a.req, false)
	res := a.prepare(&a.res, true)
	// SDKs may return on a terminal event without reading the HTTP stream
	// through EOF. Keep safe bytes, but don't claim the unread suffix.
	if a.res.seen && !a.res.eof && (a.res.expected <= 0 || a.res.expected != a.res.total) {
		a.res.truncated = true
	}
	publication.Lock()
	defer publication.Unlock()
	if !publication.active[a] {
		return
	}
	delete(publication.active, a)
	if a.scope.Enabled != nil && !a.scope.Enabled() {
		return
	}
	if scrub(a.scope.Session, a.secrets) != a.scope.Session || scrub(a.request, a.secrets) != a.request {
		return
	}
	a.publish(f, req, res, maxSession, maxProfile, maxRounds)
}
