package cheapauth

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"net/url"
	"strconv"
	"strings"
	"time"
	"unicode"
)

// PKCE is one attempt's RFC 7636 S256 pair. Its String methods hide the
// verifier while retaining the public challenge for diagnostics if needed.
type PKCE struct {
	verifier  string
	challenge string
}

func GeneratePKCE() (PKCE, error) {
	bytes := make([]byte, 32)
	if _, err := rand.Read(bytes); err != nil {
		return PKCE{}, &AuthError{Kind: ErrorResponse, Op: "pkce"}
	}
	verifier := base64.RawURLEncoding.EncodeToString(bytes)
	return PKCE{verifier: verifier, challenge: PKCEChallenge(verifier)}, nil
}

func PKCEChallenge(verifier string) string {
	digest := sha256.Sum256([]byte(verifier))
	return base64.RawURLEncoding.EncodeToString(digest[:])
}

func (p PKCE) Challenge() string { return p.challenge }
func (p PKCE) Verifier() string  { return p.verifier }
func (p PKCE) String() string    { return "PKCE（verifier 已隐藏）" }
func (p PKCE) GoString() string  { return p.String() }

// ParsePastedLogin accepts a displayed code, a full callback address, or a
// bare callback fragment. Legacy session fragments remain compatible with the
// Rust desktop client.
func ParsePastedLogin(text, expectedEndpoint string) (code string, legacy *Credentials, err error) {
	text = strings.TrimSpace(text)
	if text == "" {
		return "", nil, ErrCodeInvalid
	}
	if !strings.Contains(text, "=") {
		code, err := NormalizeLoginCode(text)
		return code, nil, err
	}
	values := url.Values{}
	before, fragment, hasFragment := strings.Cut(text, "#")
	if _, query, found := strings.Cut(before, "?"); found {
		values = parseQueryLenient(query)
	} else if !hasFragment {
		values = parseQueryLenient(before)
	}
	if hasFragment {
		fragmentValues := parseQueryLenient(fragment)
		for key, entries := range fragmentValues {
			values[key] = entries
		}
	}
	return deliveryFromValues(values, expectedEndpoint, time.Now())
}

func parseDelivery(fragment, expectedEndpoint string, now time.Time) (string, *Credentials, error) {
	fragment = strings.TrimPrefix(strings.TrimSpace(fragment), "#")
	values := parseQueryLenient(fragment)
	return deliveryFromValues(values, expectedEndpoint, now)
}

func deliveryFromValues(values url.Values, expectedEndpoint string, now time.Time) (string, *Credentials, error) {
	if endpoint := strings.TrimSpace(values.Get("endpoint")); endpoint != "" {
		normalized, err := NormalizeEndpoint(endpoint)
		if err != nil || normalized != expectedEndpoint {
			return "", nil, &AuthError{Kind: ErrorResponse, Op: "callback"}
		}
	}
	if values.Has("access_token") {
		expires, parseErr := strconv.ParseInt(strings.TrimSpace(values.Get("expires_in")), 10, 64)
		access, refresh := strings.TrimSpace(values.Get("access_token")), strings.TrimSpace(values.Get("refresh_token"))
		if parseErr != nil || access == "" || refresh == "" || expires <= 0 {
			return "", nil, &AuthError{Kind: ErrorResponse, Op: "callback"}
		}
		return "", &Credentials{
			AccessToken: access, RefreshToken: refresh, ExpiresAt: now.Unix() + expires,
			Endpoint: expectedEndpoint, APIKey: strings.TrimSpace(values.Get("api_key")),
		}, nil
	}
	if values.Has("code") {
		code, err := NormalizeLoginCode(values.Get("code"))
		return code, nil, err
	}
	return "", nil, ErrCodeInvalid
}

func NormalizeLoginCode(raw string) (string, error) {
	var code strings.Builder
	for _, r := range strings.TrimSpace(raw) {
		if unicode.IsSpace(r) || r == '-' {
			continue
		}
		if r >= 'a' && r <= 'z' {
			r -= 'a' - 'A'
		}
		code.WriteRune(r)
	}
	value := code.String()
	if len(value) < 6 || len(value) > 32 {
		return "", ErrCodeInvalid
	}
	for _, r := range value {
		if !(r >= 'A' && r <= 'Z' || r >= '0' && r <= '9') {
			return "", ErrCodeInvalid
		}
	}
	return value, nil
}

func parseQueryLenient(raw string) url.Values {
	values := url.Values{}
	for _, pair := range strings.Split(raw, "&") {
		key, value, found := strings.Cut(pair, "=")
		if !found {
			continue
		}
		decodedKey, keyErr := url.QueryUnescape(key)
		decodedValue, valueErr := url.QueryUnescape(value)
		if keyErr != nil || valueErr != nil {
			continue
		}
		values.Set(decodedKey, decodedValue)
	}
	return values
}
