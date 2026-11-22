package cheapauth

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"
)

const helpImageMaxBytes = 4 << 20

type HelpImage struct {
	Bytes     []byte
	Extension string
}

func parseHelpImage(b []byte) (HelpImage, error) {
	if len(b) <= helpImageMaxBytes {
		ext := ""
		switch http.DetectContentType(b) {
		case "image/png":
			ext = "png"
		case "image/jpeg":
			ext = "jpg"
		case "image/gif":
			ext = "gif"
		case "image/webp":
			ext = "webp"
		}
		if ext != "" {
			return HelpImage{b, ext}, nil
		}
	}
	return HelpImage{}, errors.New("invalid payment help image")
}

// LoadHelpImage fetches public contact artwork without a session token.
// Only verified image bytes are cached, under a digest rather than a remote filename.
func (p *PayClient) LoadHelpImage(ctx context.Context, profileDir, raw string) (HelpImage, error) {
	raw = p.AbsoluteURL(raw)
	if strings.HasPrefix(raw, "data:") {
		_, encoded, ok := strings.Cut(raw, ";base64,")
		if !ok || len(encoded) > base64.StdEncoding.EncodedLen(helpImageMaxBytes) {
			return HelpImage{}, errors.New("invalid image data URL")
		}
		b, err := base64.StdEncoding.DecodeString(encoded)
		if err != nil {
			return HelpImage{}, errors.New("invalid image data URL")
		}
		return parseHelpImage(b)
	}
	u, err := url.Parse(raw)
	if err != nil || u.User != nil || u.Hostname() == "" || (u.Scheme != "http" && u.Scheme != "https") {
		return HelpImage{}, errors.New("invalid image URL")
	}
	var path string
	if profileDir != "" {
		digest := sha256.Sum256([]byte(raw))
		path = filepath.Join(profileDir, "pay-help", hex.EncodeToString(digest[:8])+".img")
		if st, err := os.Stat(path); err == nil && st.Mode().IsRegular() && st.Size() <= helpImageMaxBytes && time.Since(st.ModTime()) < 24*time.Hour {
			if b, err := os.ReadFile(path); err == nil {
				if img, e := parseHelpImage(b); e == nil {
					return img, nil
				}
			}
		}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, raw, nil)
	if err != nil {
		return HelpImage{}, errors.New("invalid image URL")
	}
	h := *p.client.http
	h.Jar = nil
	res, err := h.Do(req)
	if err != nil {
		if ctx.Err() != nil {
			return HelpImage{}, ctx.Err()
		}
		return HelpImage{}, ErrNetwork
	}
	defer res.Body.Close()
	if res.StatusCode/100 != 2 {
		return HelpImage{}, &PayError{Status: res.StatusCode}
	}
	b, err := io.ReadAll(io.LimitReader(res.Body, helpImageMaxBytes+1))
	if err != nil {
		return HelpImage{}, ErrNetwork
	}
	img, err := parseHelpImage(b)
	if err == nil && path != "" {
		_ = WritePrivateFile(path, img.Bytes)
	}
	return img, err
}
