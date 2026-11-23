package imagestudio

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"image/png"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
)

func testPNGBytes(t *testing.T) []byte {
	t.Helper()
	var buf bytes.Buffer
	if err := png.Encode(&buf, image.NewRGBA(image.Rect(0, 0, 2, 2))); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

type fakeAuthProvider struct {
	mu         sync.Mutex
	signedIn   bool
	candidates []cheapauth.ImageGroup
	origin     string
	keys       map[int64]string
	defaultKey string
	remembered map[string]int64
	keyErr     error
}

func (f *fakeAuthProvider) SignedIn() bool {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.signedIn
}

func (f *fakeAuthProvider) ImageRouteCandidates(_ []cheapauth.Model, model string) []cheapauth.ImageGroup {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []cheapauth.ImageGroup
	if gid := f.remembered[strings.ToLower(strings.TrimSpace(model))]; gid > 0 {
		for _, g := range f.candidates {
			if g.GroupID == gid {
				out = append(out, g)
				break
			}
		}
	}
	for _, g := range f.candidates {
		if len(out) > 0 && out[0].GroupID == g.GroupID {
			continue
		}
		out = append(out, g)
	}
	return out
}

func (f *fakeAuthProvider) ImageRouteKey(_ context.Context, groupID *int64) (string, string, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.keyErr != nil {
		return "", "", f.keyErr
	}
	if groupID != nil && *groupID > 0 {
		if k, ok := f.keys[*groupID]; ok {
			return f.origin, k, nil
		}
	}
	return f.origin, f.defaultKey, nil
}

func (f *fakeAuthProvider) RememberImageGroup(_ context.Context, model string, groupID int64) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.remembered == nil {
		f.remembered = map[string]int64{}
	}
	f.remembered[strings.ToLower(strings.TrimSpace(model))] = groupID
	return nil
}

func newTestClient(pollInterval, taskTimeout time.Duration) *Client {
	c := NewClient()
	c.pollInterval = pollInterval
	c.taskTimeout = taskTimeout
	return c
}

func TestURLResolution(t *testing.T) {
	for _, tc := range []struct {
		base    string
		wantGen string
		wantEd  string
	}{
		{"https://api.example.com", "https://api.example.com/v1/images/generations", "https://api.example.com/v1/images/edits"},
		{"https://api.example.com/v1", "https://api.example.com/v1/images/generations", "https://api.example.com/v1/images/edits"},
		{"https://api.example.com/v1/images/generations", "https://api.example.com/v1/images/generations", "https://api.example.com/v1/images/edits"},
		{"https://api.example.com/custom/v1/", "https://api.example.com/custom/v1/images/generations", "https://api.example.com/custom/v1/images/edits"},
		{"https://api.example.com/v1/images/edits", "https://api.example.com/v1/images/edits/images/generations", "https://api.example.com/v1/images/edits"},
	} {
		gen, err := GenerationsURL(tc.base)
		if err != nil || gen != tc.wantGen {
			t.Fatalf("GenerationsURL(%q) = %q, %v; want %q", tc.base, gen, err, tc.wantGen)
		}
		ed, err := EditsURL(tc.base)
		if err != nil || ed != tc.wantEd {
			t.Fatalf("EditsURL(%q) = %q, %v; want %q", tc.base, ed, err, tc.wantEd)
		}
	}
}

func TestCheapRouterAsyncPollAndDownloadNoAuth(t *testing.T) {
	pngData := testPNGBytes(t)
	var polls atomic.Int32
	var downloadSawAuth atomic.Bool

	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/v1/images/generations/async":
			if r.Header.Get("Authorization") != "Bearer key-group-2" {
				http.Error(w, "bad auth", http.StatusUnauthorized)
				return
			}
			w.WriteHeader(http.StatusAccepted)
			_, _ = w.Write([]byte(`{"task_id":"imgtask_42"}`))
		case r.Method == http.MethodGet && r.URL.Path == "/v1/images/tasks/imgtask_42":
			n := polls.Add(1)
			if n == 1 {
				_, _ = w.Write([]byte(`{"status":"processing"}`))
				return
			}
			_, _ = fmt.Fprintf(w, `{"status":"completed","result":{"data":[{"url":%q,"revised_prompt":"bright harbor"}]}}`, srv.URL+"/files/out.png")
		case r.Method == http.MethodGet && r.URL.Path == "/files/out.png":
			if r.Header.Get("Authorization") != "" {
				downloadSawAuth.Store(true)
			}
			w.Header().Set("Content-Type", "image/png")
			_, _ = w.Write(pngData)
		default:
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()

	client := newTestClient(5*time.Millisecond, 5*time.Second)
	auth := &fakeAuthProvider{
		signedIn:   true,
		origin:     srv.URL,
		candidates: []cheapauth.ImageGroup{{GroupID: 2, Name: "codex-img"}},
		keys:       map[int64]string{2: "key-group-2"},
	}
	runner := NewRunner(client, auth, nil, nil)

	var seenTaskID string
	res, imgErr := runner.Run(t.Context(), GenerateSpec{
		Channel: ChannelCheapRouter,
		Model:   "gpt-image-2",
		Prompt:  "a harbor",
		Size:    "1536x1024",
		Count:   1,
	}, RunOptions{
		OnTaskID: func(id string) { seenTaskID = id },
	})
	if imgErr != nil {
		t.Fatalf("unexpected error: %v", imgErr)
	}
	if seenTaskID != "imgtask_42" || res.TaskID != "imgtask_42" || res.GroupID != 2 {
		t.Fatalf("unexpected result metadata: %+v, seenTaskID=%q", res, seenTaskID)
	}
	if len(res.Outputs) != 1 || res.Outputs[0].RevisedPrompt != "bright harbor" {
		t.Fatalf("unexpected outputs: %+v", res.Outputs)
	}

	outDir := t.TempDir()
	paths, saveErr := client.SaveOutputs(t.Context(), res.Outputs, outDir, "harbor")
	if saveErr != nil {
		t.Fatalf("SaveOutputs failed: %v", saveErr)
	}
	if len(paths) != 1 || filepath.Base(paths[0]) != "harbor-1.png" {
		t.Fatalf("saved paths = %v", paths)
	}
	if downloadSawAuth.Load() {
		t.Fatal("download request leaked Authorization header")
	}
	got, err := os.ReadFile(paths[0])
	if err != nil || !bytes.Equal(got, pngData) {
		t.Fatalf("saved bytes mismatch: err=%v len=%d", err, len(got))
	}
	if _, err := os.Stat(filepath.Join(outDir, "harbor-1.part")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("leftover .part file found: %v", err)
	}
	if auth.remembered["gpt-image-2"] != 2 {
		t.Fatalf("winning group was not remembered: %v", auth.remembered)
	}
}

func TestCheapRouterAsyncFallbackSSEAndSyncGrok(t *testing.T) {
	pngData := testPNGBytes(t)
	b64 := base64.StdEncoding.EncodeToString(pngData)
	var asyncCalls, syncCalls atomic.Int32
	var sawStreamTrue, grokOmittedQuality atomic.Bool

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/images/generations/async":
			asyncCalls.Add(1)
			w.WriteHeader(http.StatusNotFound)
			_, _ = w.Write([]byte(`{"error":{"message":"async image tasks are not enabled"}}`))
		case "/v1/images/generations":
			syncCalls.Add(1)
			var body map[string]any
			_ = json.NewDecoder(r.Body).Decode(&body)
			if body["model"] == "grok-imagine" {
				if _, hasQ := body["quality"]; !hasQ && body["response_format"] == "b64_json" {
					grokOmittedQuality.Store(true)
				}
				_, _ = fmt.Fprintf(w, `{"data":[{"b64_json":%q}]}`, b64)
				return
			}
			if body["stream"] == true {
				sawStreamTrue.Store(true)
			}
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = fmt.Fprintf(w, ": keepalive\n\nevent: image_generation.completed\ndata: {\"type\":\"image_generation.completed\",\"b64_json\":%q}\n\n", b64)
		default:
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()

	client := newTestClient(5*time.Millisecond, 5*time.Second)
	auth := &fakeAuthProvider{
		signedIn:   true,
		origin:     srv.URL,
		candidates: []cheapauth.ImageGroup{{GroupID: 5, Name: "default"}},
		keys:       map[int64]string{5: "key-5"},
	}
	runner := NewRunner(client, auth, nil, nil)

	// First call: tries /async (404 async unavailable) -> falls back to SSE.
	res, err := runner.Run(t.Context(), GenerateSpec{
		Channel: ChannelCheapRouter,
		Model:   "gpt-image-2",
		Prompt:  "mountain lake",
		Count:   1,
	}, RunOptions{})
	if err != nil {
		t.Fatalf("SSE fallback failed: %v", err)
	}
	if !res.AsyncDisabled || !runner.AsyncUnavailable() || !sawStreamTrue.Load() {
		t.Fatalf("expected SSE fallback & async memoization: res=%+v stream=%v", res, sawStreamTrue.Load())
	}

	// Second call (Grok): skips /async completely, sends sync JSON without quality.
	res2, err := runner.Run(t.Context(), GenerateSpec{
		Channel: ChannelCheapRouter,
		Model:   "grok-imagine",
		Prompt:  "neon skyline",
		Quality: "high",
		Count:   1,
	}, RunOptions{})
	if err != nil || len(res2.Outputs) != 1 {
		t.Fatalf("Grok sync JSON failed: res=%+v err=%v", res2, err)
	}
	if asyncCalls.Load() != 1 || syncCalls.Load() != 2 || !grokOmittedQuality.Load() {
		t.Fatalf("asyncCalls=%d syncCalls=%d grokOmittedQuality=%v", asyncCalls.Load(), syncCalls.Load(), grokOmittedQuality.Load())
	}
}

func TestMultiGroupFallbackDenialAndExpiry(t *testing.T) {
	pngData := testPNGBytes(t)
	b64 := base64.StdEncoding.EncodeToString(pngData)
	var tried []string
	var mu sync.Mutex

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		key := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
		mu.Lock()
		tried = append(tried, key)
		mu.Unlock()
		switch key {
		case "key-g1":
			w.WriteHeader(http.StatusForbidden)
			_, _ = w.Write([]byte(`{"error":{"message":"image generation is not enabled for this group"}}`))
		case "key-g2":
			w.WriteHeader(http.StatusNotFound)
			_, _ = w.Write([]byte(`{"error":{"message":"endpoint is not supported for this platform"}}`))
		case "key-g3":
			_, _ = fmt.Fprintf(w, `{"data":[{"b64_json":%q}]}`, b64)
		default:
			http.Error(w, "unknown key", http.StatusUnauthorized)
		}
	}))
	defer srv.Close()

	profileDir := t.TempDir()
	idxStore, err := NewIndexStore(profileDir)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Unix(1_700_000_000, 0)
	client := newTestClient(5*time.Millisecond, 5*time.Second)
	auth := &fakeAuthProvider{
		signedIn: true,
		origin:   srv.URL,
		candidates: []cheapauth.ImageGroup{
			{GroupID: 10, Name: "no-perm"},
			{GroupID: 20, Name: "wrong-platform"},
			{GroupID: 30, Name: "working"},
		},
		keys: map[int64]string{10: "key-g1", 20: "key-g2", 30: "key-g3"},
	}
	runner := NewRunner(client, auth, nil, idxStore)
	runner.SetAsyncUnavailable(true)
	runner.now = func() time.Time { return now }

	res, imgErr := runner.Run(t.Context(), GenerateSpec{
		Channel: ChannelCheapRouter,
		Model:   "grok-imagine",
		Prompt:  "test",
		Count:   1,
	}, RunOptions{PersistDenials: true})
	if imgErr != nil {
		t.Fatalf("expected group 30 to succeed, got %v", imgErr)
	}
	if res.GroupID != 30 || len(res.NewDenials) != 2 {
		t.Fatalf("group=%d newDenials=%v", res.GroupID, res.NewDenials)
	}
	if auth.remembered["grok-imagine"] != 30 {
		t.Fatalf("did not remember group 30: %v", auth.remembered)
	}

	// Verify denials persisted to index.json and skip groups 10 & 20 within 24h.
	denials, err := idxStore.LoadDenials(now.Add(time.Hour))
	if err != nil || len(denials) != 2 {
		t.Fatalf("persisted denials = %v, err=%v", denials, err)
	}
	auth.remembered = nil // Clear remembered group to prove denials alone skip g1/g2.
	mu.Lock()
	tried = nil
	mu.Unlock()
	res2, imgErr := runner.Run(t.Context(), GenerateSpec{
		Channel: ChannelCheapRouter,
		Model:   "grok-imagine",
		Prompt:  "test 2",
		Count:   1,
	}, RunOptions{PersistDenials: true})
	if imgErr != nil || res2.GroupID != 30 {
		t.Fatalf("second run failed: res=%+v err=%v", res2, imgErr)
	}
	mu.Lock()
	gotTried := append([]string(nil), tried...)
	mu.Unlock()
	if len(gotTried) != 1 || gotTried[0] != "key-g3" {
		t.Fatalf("expected denied groups to be skipped, tried=%v", gotTried)
	}

	// Advance clock past 24h DenialTTL -> denials expire and group 10 is probed again.
	now = now.Add(DenialTTL + time.Minute)
	auth.remembered = nil
	mu.Lock()
	tried = nil
	mu.Unlock()
	_, imgErr = runner.Run(t.Context(), GenerateSpec{
		Channel: ChannelCheapRouter,
		Model:   "grok-imagine",
		Prompt:  "test 3",
		Count:   1,
	}, RunOptions{PersistDenials: true})
	if imgErr != nil {
		t.Fatalf("post-expiry run failed: %v", imgErr)
	}
	mu.Lock()
	gotTried = append([]string(nil), tried...)
	mu.Unlock()
	if len(gotTried) != 3 || gotTried[0] != "key-g1" {
		t.Fatalf("expected expired denials to be retried, tried=%v", gotTried)
	}
}

func TestCustomAPIExtraParamsAndSecretRedaction(t *testing.T) {
	pngData := testPNGBytes(t)
	b64 := base64.StdEncoding.EncodeToString(pngData)
	secret := "sk-custom-secret-987654321"
	var sawBody map[string]any

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/v1/images/generations" {
			_ = json.NewDecoder(r.Body).Decode(&sawBody)
			if sawBody["prompt"] == "trigger-error" {
				w.WriteHeader(http.StatusUnauthorized)
				_, _ = fmt.Fprintf(w, `{"error":{"message":"invalid api key %s"}}`, secret)
				return
			}
			_, _ = fmt.Fprintf(w, `{"data":[{"b64_json":%q}]}`, b64)
			return
		}
		http.NotFound(w, r)
	}))
	defer srv.Close()

	profileDir := t.TempDir()
	customStore, err := NewCustomStore(profileDir)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := customStore.Save(srv.URL, secret, false, "flux-pro", `{"aspect_ratio":"16:9"}`); err != nil {
		t.Fatal(err)
	}

	runner := NewRunner(newTestClient(5*time.Millisecond, 5*time.Second), nil, customStore, nil)
	res, imgErr := runner.Run(t.Context(), GenerateSpec{
		Channel: ChannelCustom,
		Prompt:  "forest sunrise",
		Count:   1,
	}, RunOptions{})
	if imgErr != nil {
		t.Fatalf("custom generation failed: %v", imgErr)
	}
	if len(res.Outputs) != 1 || sawBody["model"] != "flux-pro" || sawBody["aspect_ratio"] != "16:9" {
		t.Fatalf("custom body = %v, outputs = %v", sawBody, res.Outputs)
	}

	_, errRes := runner.Run(t.Context(), GenerateSpec{
		Channel: ChannelCustom,
		Prompt:  "trigger-error",
		Count:   1,
	}, RunOptions{})
	if errRes == nil || errRes.Kind != ErrUnauthorized {
		t.Fatalf("expected ErrUnauthorized, got %+v", errRes)
	}
	if strings.Contains(errRes.Message, secret) || strings.Contains(errRes.Error(), secret) || !strings.Contains(errRes.Message, "[redacted]") {
		t.Fatalf("secret leaked in error: %q", errRes.Error())
	}
}

func TestCancelAndOutcomeUnknownWithTaskID(t *testing.T) {
	var submitCount atomic.Int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/v1/images/generations/async":
			id := fmt.Sprintf("imgtask_slow_%d", submitCount.Add(1))
			w.WriteHeader(http.StatusAccepted)
			_, _ = fmt.Fprintf(w, `{"task_id":%q}`, id)
		case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/images/tasks/"):
			_, _ = w.Write([]byte(`{"status":"processing"}`))
		default:
			http.NotFound(w, r)
		}
	}))
	defer srv.Close()

	auth := &fakeAuthProvider{
		signedIn: true,
		origin:   srv.URL,
		candidates: []cheapauth.ImageGroup{
			{GroupID: 1, Name: "g1"},
			{GroupID: 2, Name: "g2"},
		},
		keys: map[int64]string{1: "k1", 2: "k2"},
	}

	t.Run("timeout after task accepted preserves task_id and does not retry", func(t *testing.T) {
		submitCount.Store(0)
		client := newTestClient(10*time.Millisecond, 35*time.Millisecond)
		runner := NewRunner(client, auth, nil, nil)
		res, err := runner.Run(t.Context(), GenerateSpec{
			Channel: ChannelCheapRouter,
			Model:   "gpt-image-2",
			Prompt:  "slow image",
			Count:   1,
		}, RunOptions{})
		if err == nil || err.Kind != ErrTimeout || err.TaskID != "imgtask_slow_1" || err.TaskEnded || res.TaskID != "imgtask_slow_1" {
			t.Fatalf("timeout result = %+v, err = %+v", res, err)
		}
		if submitCount.Load() != 1 {
			t.Fatalf("runner retried after task submission: submitCount=%d", submitCount.Load())
		}
	})

	t.Run("ctx cancellation stops polling", func(t *testing.T) {
		submitCount.Store(0)
		client := newTestClient(15*time.Millisecond, 10*time.Second)
		runner := NewRunner(client, auth, nil, nil)
		ctx, cancel := context.WithCancel(t.Context())
		go func() {
			time.Sleep(30 * time.Millisecond)
			cancel()
		}()
		_, err := runner.Run(ctx, GenerateSpec{
			Channel: ChannelCheapRouter,
			Model:   "gpt-image-2",
			Prompt:  "cancel me",
			Count:   1,
		}, RunOptions{})
		if err == nil || err.Kind != ErrCancelled || err.TaskID == "" {
			t.Fatalf("expected ErrCancelled with TaskID, got %+v", err)
		}
	})
}

func TestGenerateEditsMultipartCustomAndCheapRouter(t *testing.T) {
	ref1 := testPNGBytes(t)
	ref2 := append([]byte{0xFF, 0xD8, 0xFF, 0xE0}, []byte("fake-jpeg-bytes")...)
	outB64 := base64.StdEncoding.EncodeToString(ref1)

	t.Run("custom edits multipart multi-image and unsupported 405", func(t *testing.T) {
		var (
			gotPrompt, gotModel, gotSize, gotQuality, gotRespFmt, gotExtra string
			gotFiles                                                        [][]byte
			gotNames                                                        []string
			gotMimes                                                        []string
		)
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if r.URL.Path != "/v1/images/edits" {
				http.NotFound(w, r)
				return
			}
			if r.Header.Get("X-Test-Mode") == "unsupported" || strings.Contains(r.URL.RawQuery, "unsupported") {
				w.WriteHeader(http.StatusMethodNotAllowed)
				_, _ = w.Write([]byte(`{"error":"method not allowed"}`))
				return
			}
			if err := r.ParseMultipartForm(16 << 20); err != nil {
				http.Error(w, err.Error(), http.StatusBadRequest)
				return
			}
			gotPrompt = r.FormValue("prompt")
			gotModel = r.FormValue("model")
			gotSize = r.FormValue("size")
			gotQuality = r.FormValue("quality")
			gotRespFmt = r.FormValue("response_format")
			gotExtra = r.FormValue("seed")
			for _, fh := range r.MultipartForm.File["image"] {
				f, err := fh.Open()
				if err != nil {
					t.Fatal(err)
				}
				data, _ := io.ReadAll(f)
				_ = f.Close()
				gotFiles = append(gotFiles, data)
				gotNames = append(gotNames, fh.Filename)
				gotMimes = append(gotMimes, fh.Header.Get("Content-Type"))
			}
			_, _ = fmt.Fprintf(w, `{"data":[{"b64_json":%q}]}`, outB64)
		}))
		defer srv.Close()

		client := newTestClient(5*time.Millisecond, 5*time.Second)
		cfg := CustomConfig{
			BaseURL:     srv.URL + "/v1/images/generations", // verifies EditsURL rewrites /generations -> /edits
			APIKey:      "sk-edit-key",
			Model:       "custom-edit-model",
			ExtraParams: `{"seed":42}`,
		}
		outputs, err := client.GenerateCustom(t.Context(), cfg, GenerateSpec{
			Channel:        ChannelCustom,
			Prompt:         "merge these two",
			Size:           "1024x1024",
			Quality:        "high",
			ResponseFormat: "b64_json",
			Count:          1,
			References: []ReferenceImage{
				{Filename: "one.png", MimeType: "image/png", Data: ref1},
				{Filename: "two.jpg", MimeType: "image/jpeg", Data: ref2},
			},
		})
		if err != nil {
			t.Fatalf("custom edits failed: %v", err)
		}
		if len(outputs) != 1 || gotPrompt != "merge these two" || gotModel != "custom-edit-model" ||
			gotSize != "1024x1024" || gotQuality != "high" || gotRespFmt != "b64_json" || gotExtra != "42" {
			t.Fatalf("unexpected multipart fields: prompt=%q model=%q size=%q quality=%q rf=%q extra=%q",
				gotPrompt, gotModel, gotSize, gotQuality, gotRespFmt, gotExtra)
		}
		if len(gotFiles) != 2 || !bytes.Equal(gotFiles[0], ref1) || !bytes.Equal(gotFiles[1], ref2) {
			t.Fatalf("multipart image bytes mismatch: count=%d", len(gotFiles))
		}
		if gotNames[0] != "one.png" || gotMimes[0] != "image/png" || gotNames[1] != "two.jpg" || gotMimes[1] != "image/jpeg" {
			t.Fatalf("multipart file metadata mismatch: names=%v mimes=%v", gotNames, gotMimes)
		}

		// Custom endpoint returning 404/405 for /v1/images/edits gives clear error.
		unsupportedSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			w.WriteHeader(http.StatusMethodNotAllowed)
			_, _ = w.Write([]byte(`{"error":{"message":"edits not supported"}}`))
		}))
		defer unsupportedSrv.Close()
		_, editErr := client.GenerateCustom(t.Context(), CustomConfig{
			BaseURL: unsupportedSrv.URL,
			Model:   "no-edits",
		}, GenerateSpec{
			Channel:    ChannelCustom,
			Prompt:     "edit",
			References: []ReferenceImage{{Filename: "a.png", MimeType: "image/png", Data: ref1}},
		})
		if editErr == nil || editErr.Kind != ErrBadRequest || !strings.Contains(editErr.Message, "/v1/images/edits") {
			t.Fatalf("expected clear edits-unsupported error, got %+v", editErr)
		}
	})

	t.Run("cheaprouter edits async and fallback", func(t *testing.T) {
		var gotFieldCount int
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			switch {
			case r.Method == http.MethodPost && r.URL.Path == "/v1/images/edits/async":
				if err := r.ParseMultipartForm(16 << 20); err != nil {
					http.Error(w, err.Error(), http.StatusBadRequest)
					return
				}
				gotFieldCount = len(r.MultipartForm.File["image[]"])
				w.WriteHeader(http.StatusAccepted)
				_, _ = w.Write([]byte(`{"task_id":"edit_task_9"}`))
			case r.Method == http.MethodGet && r.URL.Path == "/v1/images/tasks/edit_task_9":
				_, _ = fmt.Fprintf(w, `{"status":"completed","result":{"data":[{"b64_json":%q}]}}`, outB64)
			default:
				http.NotFound(w, r)
			}
		}))
		defer srv.Close()

		client := newTestClient(5*time.Millisecond, 5*time.Second)
		auth := &fakeAuthProvider{
			signedIn:   true,
			origin:     srv.URL,
			candidates: []cheapauth.ImageGroup{{GroupID: 7, Name: "img-grp"}},
			keys:       map[int64]string{7: "key-7"},
		}
		runner := NewRunner(client, auth, nil, nil)
		res, err := runner.Run(t.Context(), GenerateSpec{
			Channel: ChannelCheapRouter,
			Model:   "gpt-image-2",
			Prompt:  "make sky golden",
			Count:   1,
			References: []ReferenceImage{
				{Filename: "src1.png", MimeType: "image/png", Data: ref1},
				{Filename: "src2.jpg", MimeType: "image/jpeg", Data: ref2},
			},
		}, RunOptions{})
		if err != nil {
			t.Fatalf("cheaprouter edits failed: %v", err)
		}
		if res.TaskID != "edit_task_9" || gotFieldCount != 2 || len(res.Outputs) != 1 {
			t.Fatalf("unexpected edits result: res=%+v gotFieldCount=%d", res, gotFieldCount)
		}
	})
}
