// Command mygo-manifest packages finalized application artifacts into
// MyGo-compatible .tar.gz update archives and signs update-<target>.json
// manifests for the second-hop MyGo updater.
package main

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"net"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/Masterminds/semver/v3"
	"github.com/ai-poet/agent-client/internal/core"
	clientupdate "github.com/ai-poet/agent-client/internal/update"
)

const (
	maxManifestSize = 1 << 20
	maxArchiveSize  = 1 << 30
	maxPreviousKeep = 3
)

// Manifest mirrors github.com/egoist/mygo/internal/update.Manifest.
type Manifest struct {
	Version   string    `json:"version"`
	Notes     string    `json:"notes,omitempty"`
	Date      string    `json:"date,omitempty"`
	URL       string    `json:"url"`
	Size      int64     `json:"size"`
	Signature string    `json:"signature"`
	Deltas    []Delta   `json:"deltas,omitempty"`
	Previous  []Archive `json:"previous,omitempty"`
}

// Delta mirrors github.com/egoist/mygo/internal/update.Delta.
type Delta struct {
	From      string `json:"from"`
	URL       string `json:"url"`
	Size      int64  `json:"size"`
	Signature string `json:"signature"`
}

// Archive mirrors github.com/egoist/mygo/internal/update.Archive.
type Archive struct {
	Version   string `json:"version"`
	URL       string `json:"url"`
	Size      int64  `json:"size"`
	Signature string `json:"signature"`
}

type mygoConfig struct {
	Slug    string `json:"slug"`
	Version string `json:"version"`
	Updates *struct {
		URL       string `json:"url"`
		PublicKey string `json:"publicKey"`
	} `json:"updates"`
}

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "mygo-manifest: %v\n", err)
		os.Exit(1)
	}
}

func run(args []string) error {
	if len(args) == 0 {
		return errors.New("usage: mygo-manifest <pack|sign|verify> [flags]")
	}
	switch args[0] {
	case "pack":
		return runPack(args[1:])
	case "sign":
		return runSign(args[1:])
	case "verify":
		return runVerify(args[1:])
	default:
		return fmt.Errorf("unknown subcommand %q (expected pack, sign, or verify)", args[0])
	}
}

func runPack(args []string) error {
	fsFlags := flag.NewFlagSet("pack", flag.ContinueOnError)
	root := fsFlags.String("root", "", "directory containing the entries to archive")
	outPath := fsFlags.String("out", "", "output .tar.gz archive path")
	excludeRaw := fsFlags.String("exclude", "", "comma-separated top-level glob patterns to skip when packing all entries")
	target := fsFlags.String("target", "", "optional target (e.g. darwin-arm64, windows-amd64) to validate layout")
	if err := fsFlags.Parse(args); err != nil {
		return err
	}
	if strings.TrimSpace(*root) == "" || strings.TrimSpace(*outPath) == "" {
		return errors.New("pack requires -root and -out")
	}
	entries := fsFlags.Args()
	if len(entries) == 0 {
		excludes := splitCSV(*excludeRaw)
		listed, err := listTopLevelEntries(*root, excludes)
		if err != nil {
			return err
		}
		entries = listed
	}
	if len(entries) == 0 {
		return fmt.Errorf("no entries found in %s", *root)
	}
	size, sum, err := packArchive(*root, entries, *outPath)
	if err != nil {
		return err
	}
	if strings.TrimSpace(*target) != "" {
		if err := validateArchive(*outPath, strings.TrimSpace(*target)); err != nil {
			_ = os.Remove(*outPath)
			return err
		}
	}
	fmt.Printf("%s  %d  %s\n", hex.EncodeToString(sum[:]), size, *outPath)
	return nil
}

func runSign(args []string) error {
	fsFlags := flag.NewFlagSet("sign", flag.ContinueOnError)
	dir := fsFlags.String("dir", "", "directory containing <slug>-<version>-<target>.tar.gz archives")
	outDir := fsFlags.String("out", "", "output directory for update-<target>.json manifests (default: -dir)")
	prevDir := fsFlags.String("prev-dir", "", "optional directory containing previously published update-<target>.json manifests")
	version := fsFlags.String("version", "", "release semantic version (e.g. 0.4.0)")
	slug := fsFlags.String("slug", core.AppSlug, "archive filename slug")
	feedURL := fsFlags.String("feed-url", "", "base HTTPS URL hosting the MyGo archives and manifests")
	notesFile := fsFlags.String("notes-file", "", "optional Markdown file providing release notes")
	targetsRaw := fsFlags.String("targets", "", "comma-separated targets (e.g. darwin-arm64,darwin-amd64,windows-amd64,windows-arm64)")
	mygoConfigPath := fsFlags.String("mygo-config", "", "optional path to mygo.json for public key and feed cross-check")
	dateStr := fsFlags.String("date", "", "RFC3339 build timestamp (default: current UTC time)")
	if err := fsFlags.Parse(args); err != nil {
		return err
	}
	if strings.TrimSpace(*dir) == "" {
		return errors.New("sign requires -dir")
	}
	if strings.TrimSpace(*outDir) == "" {
		*outDir = *dir
	}
	ver := strings.TrimSpace(*version)
	if ver == "" {
		return errors.New("sign requires -version")
	}
	if _, err := semver.StrictNewVersion(ver); err != nil {
		return fmt.Errorf("invalid -version %q: %w", ver, err)
	}
	targets := splitCSV(*targetsRaw)
	if len(targets) == 0 {
		return errors.New("sign requires -targets")
	}

	var cfg *mygoConfig
	if strings.TrimSpace(*mygoConfigPath) != "" {
		loaded, err := loadMyGoConfig(*mygoConfigPath)
		if err != nil {
			return err
		}
		cfg = loaded
	}

	privRaw := firstNonEmpty(os.Getenv("MYGO_UPDATE_PRIVATE_KEY"), os.Getenv("MYGO_UPDATER_PRIVATE_KEY"))
	if privRaw == "" {
		return errors.New("MYGO_UPDATE_PRIVATE_KEY is required to sign MyGo manifests")
	}
	priv, err := parsePrivateKey(privRaw)
	if err != nil {
		return err
	}
	derivedPub := base64.StdEncoding.EncodeToString(priv.Public().(ed25519.PublicKey))

	envPub := strings.TrimSpace(os.Getenv("MYGO_UPDATE_PUBLIC_KEY"))
	cfgPub := ""
	if cfg != nil && cfg.Updates != nil {
		cfgPub = strings.TrimSpace(cfg.Updates.PublicKey)
	}
	if envPub == "" && cfgPub == "" {
		return errors.New("expected public key must be provided via MYGO_UPDATE_PUBLIC_KEY or -mygo-config")
	}
	if envPub != "" && envPub != derivedPub {
		return fmt.Errorf("MYGO_UPDATE_PRIVATE_KEY does not match MYGO_UPDATE_PUBLIC_KEY (derived %s, expected %s)", derivedPub, envPub)
	}
	if cfgPub != "" && cfgPub != derivedPub {
		return fmt.Errorf("MYGO_UPDATE_PRIVATE_KEY does not match updates.publicKey in %s (derived %s, expected %s)", *mygoConfigPath, derivedPub, cfgPub)
	}

	resolvedFeed := firstNonEmpty(*feedURL, os.Getenv("MYGO_UPDATE_FEED_URL"))
	if resolvedFeed == "" && cfg != nil && cfg.Updates != nil {
		resolvedFeed = strings.TrimSpace(cfg.Updates.URL)
	}
	if resolvedFeed == "" {
		resolvedFeed = clientupdate.DefaultFeedBaseURL
	}
	resolvedFeed = strings.TrimRight(resolvedFeed, "/")
	if err := validateBaseURL(resolvedFeed); err != nil {
		return err
	}

	notes := ""
	if strings.TrimSpace(*notesFile) != "" {
		rawNotes, err := os.ReadFile(*notesFile)
		if err != nil {
			return fmt.Errorf("read notes file: %w", err)
		}
		notes = strings.TrimSpace(string(rawNotes))
	}

	timestamp := time.Now().UTC().Format(time.RFC3339)
	if strings.TrimSpace(*dateStr) != "" {
		parsed, err := time.Parse(time.RFC3339, strings.TrimSpace(*dateStr))
		if err != nil {
			return fmt.Errorf("invalid -date %q: %w", *dateStr, err)
		}
		timestamp = parsed.UTC().Format(time.RFC3339)
	}

	if err := os.MkdirAll(*outDir, 0o755); err != nil {
		return err
	}

	pub := priv.Public().(ed25519.PublicKey)
	for _, target := range targets {
		archiveName := fmt.Sprintf("%s-%s-%s.tar.gz", strings.TrimSpace(*slug), ver, target)
		archivePath := filepath.Join(*dir, archiveName)
		if err := validateArchive(archivePath, target); err != nil {
			return fmt.Errorf("%s: %w", archiveName, err)
		}
		size, digest, err := hashFile(archivePath)
		if err != nil {
			return fmt.Errorf("%s: %w", archiveName, err)
		}
		if size <= 0 || size > maxArchiveSize {
			return fmt.Errorf("%s: invalid archive size %d", archiveName, size)
		}
		sigBytes := ed25519.Sign(priv, digest[:])
		if !ed25519.Verify(pub, digest[:], sigBytes) {
			return fmt.Errorf("%s: signature self-check failed", archiveName)
		}
		sig := base64.StdEncoding.EncodeToString(sigBytes)

		manifest := Manifest{
			Version:   ver,
			Notes:     notes,
			Date:      timestamp,
			URL:       resolvedFeed + "/" + url.PathEscape(archiveName),
			Size:      size,
			Signature: sig,
		}
		if strings.TrimSpace(*prevDir) != "" {
			prevPath := filepath.Join(*prevDir, ManifestName(target))
			if prev, err := readPreviousManifest(prevPath); err == nil && prev != nil {
				manifest.Previous = mergePrevious(ver, prev)
			}
		}

		manifestPath := filepath.Join(*outDir, ManifestName(target))
		if err := writeJSONAtomic(manifestPath, manifest); err != nil {
			return err
		}
		fmt.Printf("signed %s (%d bytes) -> %s\n", archiveName, size, manifestPath)
	}
	return nil
}

func runVerify(args []string) error {
	fsFlags := flag.NewFlagSet("verify", flag.ContinueOnError)
	dir := fsFlags.String("dir", "", "directory containing update-<target>.json and its archives")
	targetsRaw := fsFlags.String("targets", "", "comma-separated targets to verify")
	mygoConfigPath := fsFlags.String("mygo-config", "", "optional path to mygo.json providing updates.publicKey")
	if err := fsFlags.Parse(args); err != nil {
		return err
	}
	if strings.TrimSpace(*dir) == "" {
		return errors.New("verify requires -dir")
	}
	targets := splitCSV(*targetsRaw)
	if len(targets) == 0 {
		return errors.New("verify requires -targets")
	}
	pubText := strings.TrimSpace(os.Getenv("MYGO_UPDATE_PUBLIC_KEY"))
	if pubText == "" && strings.TrimSpace(*mygoConfigPath) != "" {
		cfg, err := loadMyGoConfig(*mygoConfigPath)
		if err != nil {
			return err
		}
		if cfg.Updates != nil {
			pubText = strings.TrimSpace(cfg.Updates.PublicKey)
		}
	}
	pub, err := parsePublicKey(pubText)
	if err != nil {
		return err
	}
	for _, target := range targets {
		manifestPath := filepath.Join(*dir, ManifestName(target))
		raw, err := os.ReadFile(manifestPath)
		if err != nil {
			return fmt.Errorf("read %s: %w", manifestPath, err)
		}
		if len(raw) > maxManifestSize {
			return fmt.Errorf("%s exceeds max manifest size", manifestPath)
		}
		var m Manifest
		if err := json.Unmarshal(raw, &m); err != nil {
			return fmt.Errorf("decode %s: %w", manifestPath, err)
		}
		if m.Version == "" || m.URL == "" || m.Signature == "" || m.Size <= 0 {
			return fmt.Errorf("%s is incomplete", manifestPath)
		}
		if _, err := semver.StrictNewVersion(m.Version); err != nil {
			return fmt.Errorf("%s invalid version %q: %w", manifestPath, m.Version, err)
		}
		if m.Date != "" {
			if _, err := time.Parse(time.RFC3339, m.Date); err != nil {
				return fmt.Errorf("%s invalid RFC3339 date %q: %w", manifestPath, m.Date, err)
			}
		}
		parsedURL, err := url.Parse(m.URL)
		if err != nil || validateBaseURL(m.URL) != nil {
			return fmt.Errorf("%s invalid archive url %q", manifestPath, m.URL)
		}
		archiveName := path.Base(parsedURL.Path)
		archivePath := filepath.Join(*dir, archiveName)
		if err := validateArchive(archivePath, target); err != nil {
			return fmt.Errorf("%s: %w", archiveName, err)
		}
		size, digest, err := hashFile(archivePath)
		if err != nil {
			return fmt.Errorf("%s: %w", archiveName, err)
		}
		if size != m.Size {
			return fmt.Errorf("%s size mismatch: manifest=%d actual=%d", archiveName, m.Size, size)
		}
		sig, err := base64.StdEncoding.DecodeString(m.Signature)
		if err != nil || !ed25519.Verify(pub, digest[:], sig) {
			return fmt.Errorf("%s signature verification failed", archiveName)
		}
		fmt.Printf("verified %s (%s, %d bytes)\n", ManifestName(target), m.Version, size)
	}
	return nil
}

// ManifestName returns update-<target>.json matching mygo/internal/update.ManifestName.
func ManifestName(target string) string {
	return "update-" + target + ".json"
}

func listTopLevelEntries(root string, excludes []string) ([]string, error) {
	items, err := os.ReadDir(root)
	if err != nil {
		return nil, err
	}
	var names []string
	for _, item := range items {
		name := item.Name()
		excluded := false
		for _, pattern := range excludes {
			if ok, _ := filepath.Match(pattern, name); ok {
				excluded = true
				break
			}
		}
		if !excluded {
			names = append(names, name)
		}
	}
	slices.Sort(names)
	return names, nil
}

// packArchive writes entries under root into outPath using the exact tar.gz
// conventions expected by mygo/internal/update.ExtractArchive (no leading ".",
// deterministic uid/gid=0, preserved symlinks and modes, gzip BestCompression).
func packArchive(root string, entries []string, outPath string) (int64, [sha256.Size]byte, error) {
	var zero [sha256.Size]byte
	if err := os.MkdirAll(filepath.Dir(outPath), 0o755); err != nil {
		return 0, zero, err
	}
	tmp, err := os.CreateTemp(filepath.Dir(outPath), ".mygo-archive-*")
	if err != nil {
		return 0, zero, err
	}
	tmpName := tmp.Name()
	cleanup := true
	defer func() {
		if cleanup {
			_ = os.Remove(tmpName)
		}
	}()

	hasher := sha256.New()
	mw := io.MultiWriter(tmp, hasher)
	gz, err := gzip.NewWriterLevel(mw, gzip.BestCompression)
	if err != nil {
		tmp.Close()
		return 0, zero, err
	}
	tw := tar.NewWriter(gz)

	for _, entry := range entries {
		entryPath := filepath.Join(root, entry)
		err := filepath.WalkDir(entryPath, func(p string, d fs.DirEntry, walkErr error) error {
			if walkErr != nil {
				return walkErr
			}
			info, err := d.Info()
			if err != nil {
				return err
			}
			rel, err := filepath.Rel(root, p)
			if err != nil {
				return err
			}
			slash := filepath.ToSlash(rel)
			clean := path.Clean(slash)
			if !fs.ValidPath(clean) || clean == "." {
				return fmt.Errorf("invalid archive path %q", slash)
			}
			link := ""
			if info.Mode()&fs.ModeSymlink != 0 {
				if link, err = os.Readlink(p); err != nil {
					return err
				}
				resolved := path.Join(path.Dir(clean), link)
				if path.IsAbs(link) || !fs.ValidPath(resolved) {
					return fmt.Errorf("symlink %q -> %q escapes archive root", clean, link)
				}
			}
			hdr, err := tar.FileInfoHeader(info, link)
			if err != nil {
				return err
			}
			hdr.Name = clean
			if d.IsDir() {
				hdr.Name += "/"
			}
			hdr.Uid, hdr.Gid, hdr.Uname, hdr.Gname = 0, 0, "", ""
			if err := tw.WriteHeader(hdr); err != nil {
				return err
			}
			if info.Mode().IsRegular() {
				f, err := os.Open(p)
				if err != nil {
					return err
				}
				_, copyErr := io.Copy(tw, f)
				closeErr := f.Close()
				if copyErr != nil {
					return copyErr
				}
				return closeErr
			}
			return nil
		})
		if err != nil {
			tw.Close()
			gz.Close()
			tmp.Close()
			return 0, zero, err
		}
	}
	if err := tw.Close(); err != nil {
		gz.Close()
		tmp.Close()
		return 0, zero, err
	}
	if err := gz.Close(); err != nil {
		tmp.Close()
		return 0, zero, err
	}
	if err := tmp.Close(); err != nil {
		return 0, zero, err
	}
	info, err := os.Stat(tmpName)
	if err != nil {
		return 0, zero, err
	}
	if err := os.Rename(tmpName, outPath); err != nil {
		return 0, zero, err
	}
	cleanup = false
	var digest [sha256.Size]byte
	copy(digest[:], hasher.Sum(nil))
	return info.Size(), digest, nil
}

// validateArchive checks that archivePath can be safely extracted by
// mygo/internal/update.ExtractArchive and matches the expected platform layout.
func validateArchive(archivePath, target string) error {
	f, err := os.Open(archivePath)
	if err != nil {
		return err
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		return fmt.Errorf("invalid gzip: %w", err)
	}
	defer gz.Close()
	tr := tar.NewReader(gz)

	topEntries := map[string]struct{}{}
	hasExe := false
	entryCount := 0
	for {
		hdr, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return fmt.Errorf("invalid tar: %w", err)
		}
		entryCount++
		name := path.Clean(hdr.Name)
		if !fs.ValidPath(name) || name == "." {
			return fmt.Errorf("invalid tar entry path %q", hdr.Name)
		}
		switch hdr.Typeflag {
		case tar.TypeDir, tar.TypeReg:
		case tar.TypeSymlink:
			resolved := path.Join(path.Dir(name), hdr.Linkname)
			if path.IsAbs(hdr.Linkname) || !fs.ValidPath(resolved) {
				return fmt.Errorf("symlink %q -> %q escapes archive root", hdr.Name, hdr.Linkname)
			}
		default:
			return fmt.Errorf("unsupported tar entry type %v for %q", hdr.Typeflag, hdr.Name)
		}
		top := strings.SplitN(name, "/", 2)[0]
		topEntries[top] = struct{}{}
		if strings.HasSuffix(strings.ToLower(name), ".exe") && !strings.Contains(name, "/") {
			hasExe = true
		}
	}
	if entryCount == 0 {
		return errors.New("archive is empty")
	}
	if strings.HasPrefix(target, "darwin-") {
		if len(topEntries) != 1 {
			return fmt.Errorf("macOS archive must contain exactly one top-level .app bundle, got %d entries", len(topEntries))
		}
		for name := range topEntries {
			if !strings.HasSuffix(name, ".app") {
				return fmt.Errorf("macOS archive top-level entry %q is not an .app bundle", name)
			}
		}
	}
	if strings.HasPrefix(target, "windows-") && !hasExe {
		return errors.New("Windows archive contains no top-level .exe")
	}
	return nil
}

func parsePublicKey(raw string) (ed25519.PublicKey, error) {
	trimmed := strings.TrimSpace(raw)
	if trimmed == "" {
		return nil, errors.New("update public key is required")
	}
	decoded, err := base64.StdEncoding.DecodeString(trimmed)
	if err != nil || len(decoded) != ed25519.PublicKeySize {
		return nil, errors.New("invalid base64 Ed25519 public key")
	}
	return ed25519.PublicKey(decoded), nil
}

func parsePrivateKey(raw string) (ed25519.PrivateKey, error) {
	trimmed := strings.TrimSpace(raw)
	if trimmed == "" {
		return nil, errors.New("update private key is required")
	}
	decoded, err := base64.StdEncoding.DecodeString(trimmed)
	if err != nil {
		return nil, fmt.Errorf("invalid base64 Ed25519 private key: %w", err)
	}
	switch len(decoded) {
	case ed25519.PrivateKeySize:
		derived := ed25519.NewKeyFromSeed(decoded[:ed25519.SeedSize])
		if !bytes.Equal(decoded[ed25519.SeedSize:], derived[ed25519.SeedSize:]) {
			return nil, errors.New("corrupted 64-byte Ed25519 private key (public suffix mismatch)")
		}
		return ed25519.PrivateKey(decoded), nil
	case ed25519.SeedSize:
		return ed25519.NewKeyFromSeed(decoded), nil
	default:
		return nil, fmt.Errorf("invalid Ed25519 private key length %d (expected 32 or 64 bytes)", len(decoded))
	}
}

func loadMyGoConfig(path string) (*mygoConfig, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var cfg mygoConfig
	if err := json.Unmarshal(raw, &cfg); err != nil {
		return nil, fmt.Errorf("decode %s: %w", path, err)
	}
	return &cfg, nil
}

func validateBaseURL(raw string) error {
	u, err := url.Parse(raw)
	if err != nil || u.Hostname() == "" || u.User != nil || u.Fragment != "" {
		return fmt.Errorf("invalid feed URL %q", raw)
	}
	if u.Scheme == "https" {
		return nil
	}
	ip := net.ParseIP(u.Hostname())
	if u.Scheme == "http" && (u.Hostname() == "localhost" || (ip != nil && ip.IsLoopback())) {
		return nil
	}
	return fmt.Errorf("feed URL %q must use https", raw)
}

func hashFile(path string) (int64, [sha256.Size]byte, error) {
	var zero [sha256.Size]byte
	f, err := os.Open(path)
	if err != nil {
		return 0, zero, err
	}
	defer f.Close()
	h := sha256.New()
	n, err := io.Copy(h, f)
	if err != nil {
		return 0, zero, err
	}
	var sum [sha256.Size]byte
	copy(sum[:], h.Sum(nil))
	return n, sum, nil
}

func readPreviousManifest(path string) (*Manifest, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	if len(raw) == 0 || len(raw) > maxManifestSize {
		return nil, errors.New("invalid previous manifest size")
	}
	var m Manifest
	if err := json.Unmarshal(raw, &m); err != nil {
		return nil, err
	}
	if m.Version == "" || m.URL == "" || m.Signature == "" || m.Size <= 0 {
		return nil, errors.New("incomplete previous manifest")
	}
	return &m, nil
}

func mergePrevious(currentVersion string, prev *Manifest) []Archive {
	current, err := semver.StrictNewVersion(currentVersion)
	if err != nil {
		return nil
	}
	candidates := append([]Archive{{
		Version:   prev.Version,
		URL:       prev.URL,
		Size:      prev.Size,
		Signature: prev.Signature,
	}}, prev.Previous...)
	var kept []Archive
	seen := map[string]bool{}
	for _, item := range candidates {
		if seen[item.Version] || item.URL == "" || item.Signature == "" || item.Size <= 0 {
			continue
		}
		v, err := semver.StrictNewVersion(item.Version)
		if err != nil || !v.LessThan(current) {
			continue
		}
		seen[item.Version] = true
		kept = append(kept, item)
		if len(kept) >= maxPreviousKeep {
			break
		}
	}
	return kept
}

func writeJSONAtomic(path string, value any) error {
	data, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	tmp, err := os.CreateTemp(filepath.Dir(path), ".manifest-*")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	cleanup := true
	defer func() {
		if cleanup {
			_ = os.Remove(tmpName)
		}
	}()
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Chmod(0o644); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(tmpName, path); err != nil {
		return err
	}
	cleanup = false
	return nil
}

func splitCSV(raw string) []string {
	var out []string
	for _, part := range strings.Split(raw, ",") {
		if trimmed := strings.TrimSpace(part); trimmed != "" {
			out = append(out, trimmed)
		}
	}
	return out
}

func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if trimmed := strings.TrimSpace(v); trimmed != "" {
			return trimmed
		}
	}
	return ""
}
