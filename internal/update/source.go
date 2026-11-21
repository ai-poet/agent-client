package update

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"time"
)

const (
	manifestTimeout = 20 * time.Second
	downloadTimeout = 30 * time.Minute
	userAgentPrefix = "Sailry/"
)

// Source is a MyGo UpdateSource backed by the configured HTTPS feed. Plain
// HTTP is admitted only on loopback for local fixtures and tests.
type Source struct {
	FeedURL string
	Version string
	Client  *http.Client
}

// Manifest fetches the configured manifest. MyGo passes the user's prerelease
// choice here; a static manifest serves the same document for either choice.
func (s *Source) Manifest(ctx context.Context, _ bool) (io.ReadCloser, error) {
	return s.get(ctx, s.FeedURL, manifestTimeout)
}

// Open fetches an archive or delta named by the authenticated manifest.
func (s *Source) Open(ctx context.Context, address string) (io.ReadCloser, error) {
	return s.get(ctx, address, downloadTimeout)
}

func (s *Source) get(ctx context.Context, address string, timeout time.Duration) (io.ReadCloser, error) {
	if err := validURL(address); err != nil {
		return nil, err
	}
	requestCtx, cancel := context.WithTimeout(ctx, timeout)
	req, err := http.NewRequestWithContext(requestCtx, http.MethodGet, address, nil)
	if err != nil {
		cancel()
		return nil, fmt.Errorf("%w: create request: %v", ErrNetwork, err)
	}
	req.Header.Set("User-Agent", userAgentPrefix+strings.TrimSpace(s.Version))
	req.Header.Set("Accept-Encoding", "identity")
	client := s.Client
	if client == nil {
		client = &http.Client{CheckRedirect: safeRedirect}
	}
	resp, err := client.Do(req)
	if err != nil {
		cancel()
		return nil, fmt.Errorf("%w: %v", ErrNetwork, err)
	}
	if resp.StatusCode != http.StatusOK {
		resp.Body.Close()
		cancel()
		if resp.StatusCode == http.StatusNotFound {
			return nil, fmt.Errorf("%w: HTTP 404", ErrNotFound)
		}
		return nil, fmt.Errorf("%w: server returned HTTP %d", ErrNetwork, resp.StatusCode)
	}
	return &responseBody{ReadCloser: resp.Body, cancel: cancel}, nil
}

type responseBody struct {
	io.ReadCloser
	cancel context.CancelFunc
}

func (b *responseBody) Close() error {
	err := b.ReadCloser.Close()
	b.cancel()
	return err
}

func validURL(address string) error {
	u, err := url.Parse(address)
	if err != nil || u.Hostname() == "" || u.User != nil || u.Fragment != "" {
		return fmt.Errorf("%w: invalid feed or package URL", ErrConfiguration)
	}
	if u.Scheme == "https" {
		return nil
	}
	ip := net.ParseIP(u.Hostname())
	if u.Scheme == "http" && (u.Hostname() == "localhost" || ip != nil && ip.IsLoopback()) {
		return nil
	}
	return fmt.Errorf("%w: updates require HTTPS", ErrConfiguration)
}

func safeRedirect(next *http.Request, via []*http.Request) error {
	if len(via) >= 5 {
		return fmt.Errorf("%w: too many redirects", ErrNetwork)
	}
	next.Header.Del("Referer")
	if err := validURL(next.URL.String()); err != nil {
		return err
	}
	if via[len(via)-1].URL.Scheme == "https" && next.URL.Scheme != "https" {
		return fmt.Errorf("%w: redirect downgraded HTTPS", ErrNetwork)
	}
	return nil
}
