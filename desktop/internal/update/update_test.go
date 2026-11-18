package update

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"sync/atomic"
	"testing"

	"github.com/Masterminds/semver/v3"
	"github.com/egoist/mygo"
)

type fakeNative struct {
	enabled   bool
	checks    atomic.Int32
	checkFunc func(context.Context, mygo.UpdateSource, bool) (*nativeRelease, error)
}

func (f *fakeNative) Enabled() bool { return f.enabled }
func (f *fakeNative) check(ctx context.Context, source mygo.UpdateSource, prereleases bool) (*nativeRelease, error) {
	f.checks.Add(1)
	return f.checkFunc(ctx, source, prereleases)
}

func fixtureClient(feed, version string, native nativeUpdater) *Client {
	public, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		panic(err)
	}
	return &Client{config: Config{FeedURL: feed, PublicKey: base64.StdEncoding.EncodeToString(public), Enabled: true}, version: version, native: native}
}

// manifestCheck is a test double for MyGo's manifest and version phase.
func manifestCheck(current string) func(context.Context, mygo.UpdateSource, bool) (*nativeRelease, error) {
	return func(ctx context.Context, source mygo.UpdateSource, prereleases bool) (*nativeRelease, error) {
		body, err := source.Manifest(ctx, prereleases)
		if err != nil {
			return nil, err
		}
		defer body.Close()
		var manifest struct {
			Version string `json:"version"`
		}
		if err := json.NewDecoder(body).Decode(&manifest); err != nil {
			return nil, err
		}
		newer, err := semver.StrictNewVersion(manifest.Version)
		if err != nil {
			return nil, err
		}
		installed, err := semver.StrictNewVersion(current)
		if err != nil {
			return nil, err
		}
		if newer.Compare(installed) <= 0 {
			return nil, nil
		}
		return &nativeRelease{Version: manifest.Version, install: func(context.Context, func(int64, int64)) error { return nil }}, nil
	}
}

func TestCheckComparesManifestVersion(t *testing.T) {
	for _, test := range []struct {
		name, manifest string
		want           string
	}{
		{name: "newer", manifest: `{"version":"1.2.0","url":"https://example.test/app.tar.gz","size":1,"signature":"unused"}`, want: "1.2.0"},
		{name: "same", manifest: `{"version":"1.1.0","url":"https://example.test/app.tar.gz","size":1,"signature":"unused"}`},
	} {
		t.Run(test.name, func(t *testing.T) {
			var requests atomic.Int32
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				requests.Add(1)
				if r.UserAgent() != "Sailry/1.1.0" {
					t.Errorf("User-Agent = %q", r.UserAgent())
				}
				w.Header().Set("Content-Type", "application/json")
				_, _ = io.WriteString(w, test.manifest)
			}))
			defer server.Close()
			native := &fakeNative{enabled: true}
			native.checkFunc = manifestCheck("1.1.0")
			client := fixtureClient(server.URL, "1.1.0", native)
			found, err := client.Check(context.Background())
			if err != nil {
				t.Fatal(err)
			}
			if test.want == "" {
				if found != nil {
					t.Fatalf("same version returned %+v", found)
				}
			} else if found == nil || found.Version != test.want {
				t.Fatalf("found %+v, want %s", found, test.want)
			}
			if requests.Load() != 1 {
				t.Fatalf("requests = %d", requests.Load())
			}
		})
	}
}

func TestCheckManifestNotFound(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		http.NotFound(w, nil)
	}))
	defer server.Close()
	native := &fakeNative{enabled: true, checkFunc: func(ctx context.Context, source mygo.UpdateSource, pre bool) (*nativeRelease, error) {
		body, err := source.Manifest(ctx, pre)
		if body != nil {
			body.Close()
		}
		return nil, err
	}}
	client := fixtureClient(server.URL, "1.0.0", native)
	if !client.Enabled() {
		t.Fatal("a configured client must remain usable at startup")
	}
	if _, err := client.Check(context.Background()); !errors.Is(err, ErrNotFound) {
		t.Fatalf("404 error = %v, want ErrNotFound", err)
	}
}

func TestInstallRejectsSignatureMismatch(t *testing.T) {
	_, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	otherPublic, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	archive := []byte("a package signed by another key")
	digest := sha256.Sum256(archive)
	signature := base64.StdEncoding.EncodeToString(ed25519.Sign(private, digest[:]))
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/manifest.json" {
			_ = json.NewEncoder(w).Encode(map[string]any{"version": "2.0.0", "url": serverURL(r) + "/app.tar.gz", "size": len(archive), "signature": signature})
			return
		}
		_, _ = w.Write(archive)
	}))
	defer server.Close()

	native := &fakeNative{enabled: true}
	native.checkFunc = func(ctx context.Context, source mygo.UpdateSource, pre bool) (*nativeRelease, error) {
		body, err := source.Manifest(ctx, pre)
		if err != nil {
			return nil, err
		}
		defer body.Close()
		var manifest struct {
			Version, URL, Signature string
			Size                    int64
		}
		if err := json.NewDecoder(body).Decode(&manifest); err != nil {
			return nil, err
		}
		return &nativeRelease{Version: manifest.Version, install: func(ctx context.Context, progress func(int64, int64)) error {
			asset, err := source.Open(ctx, manifest.URL)
			if err != nil {
				return err
			}
			defer asset.Close()
			downloaded, err := io.ReadAll(asset)
			if err != nil {
				return err
			}
			sum := sha256.Sum256(downloaded)
			sig, _ := base64.StdEncoding.DecodeString(manifest.Signature)
			if !ed25519.Verify(otherPublic, sum[:], sig) {
				return errors.New("mygo: installing the update: the update is not signed with the app's key")
			}
			return nil
		}}, nil
	}
	client := fixtureClient(server.URL+"/manifest.json", "1.0.0", native)
	found, err := client.Check(context.Background())
	if err != nil || found == nil {
		t.Fatalf("Check = %+v, %v", found, err)
	}
	if err := found.Install(context.Background(), nil); !errors.Is(err, ErrSignature) {
		t.Fatalf("Install error = %v, want ErrSignature", err)
	}
}

func TestEmptyConfigDoesNotRequest(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { requests.Add(1) }))
	defer server.Close()
	native := &fakeNative{enabled: true, checkFunc: func(context.Context, mygo.UpdateSource, bool) (*nativeRelease, error) {
		requests.Add(1)
		return nil, nil
	}}
	client := &Client{version: "1.0.0", native: native}
	if client.Enabled() {
		t.Fatal("empty config enabled updates")
	}
	if _, err := client.Check(context.Background()); !errors.Is(err, ErrDisabled) {
		t.Fatalf("Check error = %v", err)
	}
	if requests.Load() != 0 || native.checks.Load() != 0 {
		t.Fatalf("disabled check sent %d requests and %d native checks", requests.Load(), native.checks.Load())
	}
}

func TestInstallForwardsMonotonicProgress(t *testing.T) {
	native := &fakeNative{enabled: true, checkFunc: func(context.Context, mygo.UpdateSource, bool) (*nativeRelease, error) {
		return &nativeRelease{Version: "2.0.0", install: func(_ context.Context, progress func(int64, int64)) error {
			for _, downloaded := range []int64{0, 3, 3, 8} {
				progress(downloaded, 8)
			}
			return nil
		}}, nil
	}}
	client := fixtureClient("https://updates.example.test/manifest.json", "1.0.0", native)
	found, err := client.Check(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	var progress []int64
	if err := found.Install(context.Background(), func(downloaded, total int64) {
		if total != 8 {
			t.Errorf("total = %d", total)
		}
		progress = append(progress, downloaded)
	}); err != nil {
		t.Fatal(err)
	}
	if !slices.IsSorted(progress) || !slices.Equal(progress, []int64{0, 3, 3, 8}) {
		t.Fatalf("progress = %v", progress)
	}
}

func TestConfigStoreDefaultsAndPermissions(t *testing.T) {
	path := filepath.Join(t.TempDir(), "profile", ConfigName)
	store, err := OpenStore(path)
	if err != nil || store.Config().Configured() {
		t.Fatalf("missing config = %+v, %v", store.Config(), err)
	}
	public, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	want := Config{FeedURL: "https://updates.example.test/manifest.json", PublicKey: base64.StdEncoding.EncodeToString(public), Enabled: true, Automatic: true, Prereleases: true}
	if err := store.Save(want); err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		if got := info.Mode().Perm(); got != 0o600 {
			t.Fatalf("config mode = %o", got)
		}
	}
	reopened, err := OpenStore(path)
	if err != nil || reopened.Config() != want {
		t.Fatalf("reopened = %+v, %v", reopened.Config(), err)
	}
}

func TestMalformedManifestIsClassified(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = io.WriteString(w, `{not json`)
	}))
	defer server.Close()
	native := &fakeNative{enabled: true}
	native.checkFunc = func(ctx context.Context, source mygo.UpdateSource, pre bool) (*nativeRelease, error) {
		body, err := source.Manifest(ctx, pre)
		if err != nil {
			return nil, err
		}
		defer body.Close()
		var value any
		if err := json.NewDecoder(body).Decode(&value); err != nil {
			return nil, fmt.Errorf("decode: %w", err)
		}
		return nil, nil
	}
	client := fixtureClient(server.URL, "1.0.0", native)
	if _, err := client.Check(context.Background()); !errors.Is(err, ErrManifest) {
		t.Fatalf("error = %v", err)
	}
}

func serverURL(r *http.Request) string { return "http://" + r.Host }
