// Package insights presents the selected Node's observed model attempts.
package insights

import (
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

type Filters struct {
	StartDate, EndDate                                                                string
	Days                                                                              int
	Dimension                                                                         protocol.UsageDimension
	ObservedOnly, WithoutCredential, CacheMissOnly                                    bool
	FailureKind                                                                       string
	Outcome, EndKind, Model, Provider, Status, Session, Credential, Project, Worktree string
}

// Query fixes the end of the window on apply, not on each page read.
func (f Filters) Query(now time.Time) (protocol.UsageQuery, error) {
	q := protocol.DefaultUsageQuery()
	q.Dimension = f.Dimension
	if q.Dimension == "" {
		q.Dimension = protocol.UsageDimensionModel
	}
	q.ObservedOnly, q.WithoutCredential = f.ObservedOnly, f.WithoutCredential
	q.CacheMissOnly = f.CacheMissOnly
	if v := strings.TrimSpace(f.FailureKind); v != "" {
		q.FailureKinds = []string{v}
	}
	_, offset := now.Zone()
	q.UtcOffsetMinutes = int32(offset / 60)
	q.StartMs, q.EndMs = now.AddDate(0, 0, -f.Days).UnixMilli(), now.UnixMilli()
	if f.Days == 1 {
		q.StartMs = now.Add(-24 * time.Hour).UnixMilli()
	}
	if f.Days == 0 {
		start, err := time.ParseInLocation(time.DateOnly, f.StartDate, now.Location())
		if err != nil {
			return q, err
		}
		end, err := time.ParseInLocation(time.DateOnly, f.EndDate, now.Location())
		if err != nil {
			return q, err
		}
		q.StartMs, q.EndMs = start.UnixMilli(), end.AddDate(0, 0, 1).UnixMilli()
		if q.EndMs <= q.StartMs || q.EndMs-q.StartMs > 366*86400000 {
			return q, fmt.Errorf("invalid custom range")
		}
	}
	if f.Outcome != "" {
		q.Outcomes = []protocol.ConversationObservationOutcome{protocol.ConversationObservationOutcome(f.Outcome)}
	}
	if f.EndKind != "" {
		q.EndKinds = []protocol.ConversationObservationEndKind{protocol.ConversationObservationEndKind(f.EndKind)}
	}
	if v := strings.TrimSpace(f.Model); v != "" {
		q.Models = []string{v}
	}
	if v := strings.TrimSpace(f.Status); v != "" {
		n, err := strconv.ParseUint(v, 10, 16)
		if err != nil || n < 100 || n > 599 {
			return q, fmt.Errorf("invalid HTTP status")
		}
		q.Statuses = []uint16{uint16(n)}
	}
	var err error
	if q.Sessions, err = oneID(f.Session, protocol.ParseSessionID); err != nil {
		return q, err
	}
	if q.Providers, err = oneID(f.Provider, protocol.ParseProviderID); err != nil {
		return q, err
	}
	if q.Credentials, err = oneID(f.Credential, protocol.ParseCredentialID); err != nil {
		return q, err
	}
	if len(q.Sessions) == 0 {
		q.Sessions = nil
	}
	if len(q.Credentials) == 0 {
		q.Credentials = nil
	}
	if q.WithoutCredential && len(q.Credentials) > 0 {
		return q, fmt.Errorf("conflicting credential filters")
	}
	if q.Projects, err = oneID(f.Project, protocol.ParseProjectID); err != nil {
		return q, err
	}
	if q.Worktrees, err = oneID(f.Worktree, protocol.ParseWorktreeID); err != nil {
		return q, err
	}
	return q, nil
}

func oneID[T any](raw string, parse func(string) (T, error)) ([]T, error) {
	if raw = strings.TrimSpace(raw); raw == "" {
		return []T{}, nil
	}
	v, err := parse(raw)
	if err != nil {
		return nil, err
	}
	return []T{v}, nil
}

type State struct {
	Expanded          string
	Draft             Filters
	Tab               int
	Advanced, Loading bool
	Error             string // translation key
	Report            *protocol.UsageReport
}

func (s *State) Next() *protocol.UsagePosition {
	if s.Report == nil || !s.Report.Requests.HasMore || len(s.Report.Requests.Items) == 0 {
		return nil
	}
	last := s.Report.Requests.Items[len(s.Report.Requests.Items)-1].Position
	return &last
}
