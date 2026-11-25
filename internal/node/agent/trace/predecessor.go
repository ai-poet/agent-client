package trace

import (
	"crypto/sha256"
	"encoding/hex"
	"strconv"
	"strings"
)

// Private index material, never a preview. Each 128-bit digest represents one
// complete, capture-redacted segment. An absent sketch is not a partial prefix.
const maxSegments = 2048
const fingerprintWidth = 32

func fingerprint(s []segment) string {
	if len(s) == 0 || len(s) > maxSegments {
		return ""
	}
	var b strings.Builder
	b.Grow(len(s) * fingerprintWidth)
	for _, v := range s {
		h := sha256.New()
		for _, part := range []string{v.kind, v.role, strconv.Itoa(v.slot), v.text} {
			h.Write([]byte(strconv.Itoa(len(part)) + ":"))
			h.Write([]byte(part))
		}
		var raw [32]byte
		var encoded [fingerprintWidth]byte
		hex.Encode(encoded[:], h.Sum(raw[:0])[:16])
		b.Write(encoded[:])
	}
	return b.String()
}

type similarity struct{ prefix, matches, total int }

func (s similarity) eligible() bool { return s.prefix > 0 || s.total >= 3 && s.matches*2 >= s.total }
func (s similarity) compare(other similarity) int {
	if s.prefix != other.prefix {
		return s.prefix - other.prefix
	}
	if s.prefix > 0 {
		return 0
	}
	return s.matches*other.total - other.matches*s.total
}

type branchMatcher struct {
	current string
	present map[string]bool
}

func matchBranches(current string) branchMatcher {
	m := branchMatcher{current: current, present: map[string]bool{}}
	for i := 0; i+fingerprintWidth <= len(current); i += fingerprintWidth {
		m.present[current[i:i+fingerprintWidth]] = true
	}
	return m
}
func (m branchMatcher) score(other string) similarity {
	if len(other) == 0 || len(other)%fingerprintWidth != 0 || len(other) > maxSegments*fingerprintWidth {
		return similarity{}
	}
	s := similarity{total: len(other) / fingerprintWidth}
	for i := 0; i < len(other); i += fingerprintWidth {
		d := other[i : i+fingerprintWidth]
		if i == s.prefix*fingerprintWidth && i+fingerprintWidth <= len(m.current) && d == m.current[i:i+fingerprintWidth] {
			s.prefix++
		}
		if m.present[d] {
			s.matches++
		}
	}
	return s
}

// On one current request each successive identical segment has positive length,
// so ranking by prefix segment count also ranks by shared prefix characters.
type predecessorChoice struct {
	matcher      branchMatcher
	best, legacy rowKey
	score        similarity
}

func (c *predecessorChoice) consider(r rowKey) {
	if r.Fingerprint == "" {
		if c.legacy.Request.Body == "" || r.StartedMs >= c.legacy.StartedMs {
			c.legacy = r
		}
		return
	}
	s := c.matcher.score(r.Fingerprint)
	if !s.eligible() {
		return
	}
	if delta := s.compare(c.score); c.best.Request.Body == "" || delta > 0 || delta == 0 && r.StartedMs >= c.best.StartedMs {
		c.best, c.score = r, s
	}
}
func (c *predecessorChoice) selected() rowKey {
	if c.best.Request.Body != "" {
		return c.best
	}
	return c.legacy
}
