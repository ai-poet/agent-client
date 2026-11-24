package node

import (
	"bytes"
	"context"
	"database/sql"
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
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/imagestudio"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model"
	"github.com/ai-poet/cheaprouter-harness/internal/node/process"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"lukechampine.com/blake3"
)

type stubCheapImageAuth struct {
	mu         sync.Mutex
	signedIn   bool
	origin     string
	candidates []cheapauth.ImageGroup
	keys       map[int64]string
	defaultKey string
	remembered map[string]int64
}

func (s *stubCheapImageAuth) SignedIn() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.signedIn
}

func (s *stubCheapImageAuth) ImageRouteCandidates(_ []cheapauth.Model, _ string) []cheapauth.ImageGroup {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]cheapauth.ImageGroup(nil), s.candidates...)
}

func (s *stubCheapImageAuth) ImageRouteKey(_ context.Context, groupID *int64) (string, string, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if groupID != nil && *groupID > 0 {
		if k, ok := s.keys[*groupID]; ok {
			return s.origin, k, nil
		}
	}
	return s.origin, s.defaultKey, nil
}

func (s *stubCheapImageAuth) RememberImageGroup(_ context.Context, model string, groupID int64) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.remembered == nil {
		s.remembered = map[string]int64{}
	}
	s.remembered[strings.ToLower(model)] = groupID
	return nil
}

func tinyPNG(t *testing.T) []byte {
	t.Helper()
	var buf bytes.Buffer
	if err := png.Encode(&buf, image.NewRGBA(image.Rect(0, 0, 2, 2))); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

func TestImageRoutingPriorityMatrix(t *testing.T) {
	mediaModel := func(id string, generates ...protocol.MediaGeneration) protocol.ConversationModel {
		m := protocol.DefaultConversationModel()
		m.ID, m.Generates = id, generates
		return m
	}
	provider := protocol.DefaultConversationProvider()
	provider.ID, provider.Api, provider.Authentication = protocol.NewProviderID(), protocol.ConversationModelApiResponses, protocol.AuthenticationApiKey
	provider.Models = []protocol.ConversationModel{
		mediaModel("plain"),
		mediaModel("conv-img", protocol.MediaGenerationImage),
		mediaModel("assigned-img", protocol.MediaGenerationImage),
	}

	makeNode := func(t *testing.T, cheapSignedIn bool, customURL, customKey, customModel string) *Node {
		t.Helper()
		profile := filepath.Join(t.TempDir(), "profile")
		customStore, err := imagestudio.NewCustomStore(profile)
		if err != nil {
			t.Fatal(err)
		}
		if customURL != "" {
			if _, err := customStore.Save(customURL, customKey, false, customModel, ""); err != nil {
				t.Fatal(err)
			}
		}
		auth := &stubCheapImageAuth{
			signedIn: cheapSignedIn,
			origin:   "https://cheap.example.test",
		}
		env := process.NewEnvironment([]string{"PATH=/usr/bin:/bin"})
		n, err := StartWithOptions(t.Context(), profile, Options{
			Environment: &env,
			ImageStudio: &ImageStudioOptions{
				Auth:        auth,
				CustomStore: customStore,
				StaticCatalog: []cheapauth.Model{
					{ID: "gpt-image-2", BillingMode: "image"},
					{ID: "grok-imagine", BillingMode: "image"},
				},
			},
		})
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { _ = n.Stop(context.Background()) })
		return n
	}

	t.Run("default priority chain", func(t *testing.T) {
		// 1) Assigned Media model > CheapRouter > Custom > Conversation model
		nAll := makeNode(t, true, "https://custom.example.test/v1", "sk-secret-custom-999", "flux-custom")
		invAssigned := &store.AgentInvocation{
			Provider: &provider,
			Media:    store.MediaCapture{protocol.MediaKindImage: {Model: "assigned-img", Provider: &provider}},
		}
		invAssigned.Turn.Config.Model = "conv-img"
		st := nAll.captureImageBackend(invAssigned)
		ch, mdl, _, err := st.resolveImageRoute(imageChannelAuto, "")
		if err != nil || ch != imageChannelMedia || mdl != "assigned-img" {
			t.Fatalf("expected assigned Media priority, got ch=%s model=%s err=%v", ch, mdl, err)
		}

		// Tool description contains host/models but NEVER leaks the secret key.
		desc := st.descriptionSuffix()
		if strings.Contains(desc, "sk-secret-custom-999") {
			t.Fatalf("tool description leaked custom API key: %q", desc)
		}
		for _, want := range []string{"cheaprouter", "gpt-image-2", "grok-imagine", "custom.example.test", "flux-custom"} {
			if !strings.Contains(desc, want) {
				t.Fatalf("tool description %q missing %q", desc, want)
			}
		}

		// 2) Without assigned Media: CheapRouter > Custom > Conversation model
		invUnassigned := &store.AgentInvocation{Provider: &provider}
		invUnassigned.Turn.Config.Model = "conv-img"
		st2 := nAll.captureImageBackend(invUnassigned)
		ch, mdl, _, err = st2.resolveImageRoute(imageChannelAuto, "")
		if err != nil || ch != imageChannelCheapRouter || mdl != "gpt-image-2" {
			t.Fatalf("expected CheapRouter priority, got ch=%s model=%s err=%v", ch, mdl, err)
		}

		// 3) Without CheapRouter: Custom > Conversation model
		nCustom := makeNode(t, false, "https://custom.example.test/v1", "sk-secret", "flux-custom")
		st3 := nCustom.captureImageBackend(invUnassigned)
		ch, mdl, _, err = st3.resolveImageRoute(imageChannelAuto, "")
		if err != nil || ch != imageChannelCustom || mdl != "flux-custom" {
			t.Fatalf("expected Custom priority, got ch=%s model=%s err=%v", ch, mdl, err)
		}

		// 4) Without CheapRouter or Custom: Conversation model fallback
		nBare := makeNode(t, false, "", "", "")
		st4 := nBare.captureImageBackend(invUnassigned)
		ch, mdl, _, err = st4.resolveImageRoute(imageChannelAuto, "")
		if err != nil || ch != imageChannelMedia || mdl != "conv-img" {
			t.Fatalf("expected conversation model fallback, got ch=%s model=%s err=%v", ch, mdl, err)
		}

		// 5) Nothing available -> tool is NOT offered
		invPlain := &store.AgentInvocation{Provider: &provider}
		invPlain.Turn.Config.Model = "plain"
		st5 := nBare.captureImageBackend(invPlain)
		if st5.offered() {
			t.Fatalf("expected tool not offered when nothing configured, got %+v", st5)
		}
		tools, err := nBare.mediaTools(invPlain)
		if err != nil {
			t.Fatal(err)
		}
		for _, tool := range tools {
			if tool.Definition.Name == "generate_image" {
				t.Fatal("generate_image was offered when no backend is configured")
			}
		}
	})

	t.Run("explicit channel and model override without silent switch", func(t *testing.T) {
		nAll := makeNode(t, true, "https://custom.example.test/v1", "sk-secret", "flux-custom")
		invAssigned := &store.AgentInvocation{
			Provider: &provider,
			Media:    store.MediaCapture{protocol.MediaKindImage: {Model: "assigned-img", Provider: &provider}},
		}
		invAssigned.Turn.Config.Model = "plain"
		st := nAll.captureImageBackend(invAssigned)

		// Explicit channel=custom overrides assigned Media & CheapRouter
		ch, mdl, _, err := st.resolveImageRoute(imageChannelCustom, "")
		if err != nil || ch != imageChannelCustom || mdl != "flux-custom" {
			t.Fatalf("explicit custom = %s/%s err=%v", ch, mdl, err)
		}
		// Explicit model=grok-imagine selects CheapRouter
		ch, mdl, _, err = st.resolveImageRoute(imageChannelAuto, "grok-imagine")
		if err != nil || ch != imageChannelCheapRouter || mdl != "grok-imagine" {
			t.Fatalf("explicit grok-imagine = %s/%s err=%v", ch, mdl, err)
		}

		// Explicit channel=cheaprouter when NOT signed in MUST fail (never silently switch to custom)
		nCustomOnly := makeNode(t, false, "https://custom.example.test/v1", "sk-secret", "flux-custom")
		stCustom := nCustomOnly.captureImageBackend(invAssigned)
		_, _, _, err = stCustom.resolveImageRoute(imageChannelCheapRouter, "")
		var fault *protocol.Fault
		if !errors.As(err, &fault) || fault.Code != protocol.ErrorCodeNotConfigured {
			t.Fatalf("expected NotConfigured for unconfigured explicit cheaprouter, got %v", err)
		}

		// Explicit channel=custom when NOT configured MUST fail (never silently switch to cheaprouter)
		nCheapOnly := makeNode(t, true, "", "", "")
		stCheap := nCheapOnly.captureImageBackend(invAssigned)
		_, _, _, err = stCheap.resolveImageRoute(imageChannelCustom, "")
		if !errors.As(err, &fault) || fault.Code != protocol.ErrorCodeNotConfigured {
			t.Fatalf("expected NotConfigured for unconfigured explicit custom, got %v", err)
		}
	})
}

func TestImageStudioFaultMapping(t *testing.T) {
	checkCode := func(t *testing.T, ie *imagestudio.ImageError, ch imageChannelKind, wantCode protocol.ErrorCode, wantSubstr string) {
		t.Helper()
		err := imageStudioFault(ie, ch)
		var fault *protocol.Fault
		if !errors.As(err, &fault) {
			t.Fatalf("expected *protocol.Fault, got %T: %v", err, err)
		}
		if fault.Code != wantCode {
			t.Fatalf("kind=%s code=%s, want %s (msg=%q)", ie.Kind, fault.Code, wantCode, fault.Message)
		}
		if wantSubstr != "" && !strings.Contains(fault.Message, wantSubstr) {
			t.Fatalf("msg=%q does not contain %q", fault.Message, wantSubstr)
		}
	}

	checkCode(t, &imagestudio.ImageError{Kind: imagestudio.ErrUnauthorized, Message: "expired"}, imageChannelCheapRouter, protocol.ErrorCodeUnavailable, "CheapRouter")
	checkCode(t, &imagestudio.ImageError{Kind: imagestudio.ErrNoImagePermission, Message: "forbidden"}, imageChannelCheapRouter, protocol.ErrorCodePermissionDenied, "group")
	checkCode(t, &imagestudio.ImageError{Kind: imagestudio.ErrWrongPlatform, Message: "wrong platform"}, imageChannelCheapRouter, protocol.ErrorCodePermissionDenied, "group")
	checkCode(t, &imagestudio.ImageError{Kind: imagestudio.ErrOwnBalance, Message: "low balance"}, imageChannelCheapRouter, protocol.ErrorCodeUnavailable, "balance")
	checkCode(t, &imagestudio.ImageError{Kind: imagestudio.ErrContentPolicy, Message: "blocked"}, imageChannelCheapRouter, protocol.ErrorCodeInvalidRequest, "safety")
	checkCode(t, &imagestudio.ImageError{Kind: imagestudio.ErrTimeout, Message: "timed out"}, imageChannelCheapRouter, protocol.ErrorCodeUnavailable, "")
	checkCode(t, &imagestudio.ImageError{Kind: imagestudio.ErrTimeout, Message: "timed out", TaskID: "imgtask_99"}, imageChannelCheapRouter, protocol.ErrorCodeOutcomeUnknown, "imgtask_99")
	checkCode(t, &imagestudio.ImageError{Kind: imagestudio.ErrCancelled, Message: "cancelled", TaskID: "imgtask_99"}, imageChannelCheapRouter, protocol.ErrorCodeCancelled, "imgtask_99")
}

func TestGenerateImageStudioEndToEndAndEdits(t *testing.T) {
	imgBytes := tinyPNG(t)
	b64 := base64.StdEncoding.EncodeToString(imgBytes)

	var (
		mu              sync.Mutex
		sawEditFiles    [][]byte
		sawEditPrompt   string
		sawCustomEdits  int
		asyncTaskPolls  atomic.Int32
		blockPoll       = make(chan struct{})
		pollStarted     = make(chan struct{}, 1)
		slowMode        atomic.Bool
		customNoEdits   atomic.Bool
	)

	cheapSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == http.MethodPost && r.URL.Path == "/v1/images/generations/async":
			w.WriteHeader(http.StatusAccepted)
			if slowMode.Load() {
				_, _ = w.Write([]byte(`{"task_id":"imgtask_cancel"}`))
				return
			}
			_, _ = w.Write([]byte(`{"task_id":"imgtask_multi"}`))
		case r.Method == http.MethodGet && r.URL.Path == "/v1/images/tasks/imgtask_multi":
			if asyncTaskPolls.Add(1) == 1 {
				_, _ = w.Write([]byte(`{"status":"processing"}`))
				return
			}
			_, _ = fmt.Fprintf(w, `{"status":"completed","result":{"data":[{"b64_json":%q,"revised_prompt":"two sunset images"},{"b64_json":%q}]}}`, b64, b64)
		case r.Method == http.MethodGet && r.URL.Path == "/v1/images/tasks/imgtask_cancel":
			select {
			case pollStarted <- struct{}{}:
			default:
			}
			select {
			case <-blockPoll:
			case <-r.Context().Done():
				return
			}
			_, _ = w.Write([]byte(`{"status":"processing"}`))
		case r.Method == http.MethodPost && r.URL.Path == "/v1/images/edits/async":
			if err := r.ParseMultipartForm(16 << 20); err != nil {
				http.Error(w, err.Error(), http.StatusBadRequest)
				return
			}
			mu.Lock()
			sawEditPrompt = r.FormValue("prompt")
			sawEditFiles = nil
			for _, fh := range r.MultipartForm.File["image[]"] {
				f, _ := fh.Open()
				data, _ := io.ReadAll(f)
				_ = f.Close()
				sawEditFiles = append(sawEditFiles, data)
			}
			mu.Unlock()
			w.WriteHeader(http.StatusAccepted)
			_, _ = w.Write([]byte(`{"task_id":"imgtask_edit"}`))
		case r.Method == http.MethodGet && r.URL.Path == "/v1/images/tasks/imgtask_edit":
			_, _ = fmt.Fprintf(w, `{"status":"completed","result":{"data":[{"b64_json":%q}]}}`, b64)
		default:
			http.NotFound(w, r)
		}
	}))
	defer cheapSrv.Close()

	customSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/images/edits":
			if customNoEdits.Load() {
				w.WriteHeader(http.StatusMethodNotAllowed)
				_, _ = w.Write([]byte(`{"error":{"message":"edits unsupported"}}`))
				return
			}
			if err := r.ParseMultipartForm(16 << 20); err != nil {
				http.Error(w, err.Error(), http.StatusBadRequest)
				return
			}
			mu.Lock()
			sawCustomEdits += len(r.MultipartForm.File["image"])
			mu.Unlock()
			_, _ = fmt.Fprintf(w, `{"data":[{"b64_json":%q}]}`, b64)
		default:
			http.NotFound(w, r)
		}
	}))
	defer customSrv.Close()

	profile := filepath.Join(t.TempDir(), "profile")
	customStore, err := imagestudio.NewCustomStore(profile)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := customStore.Save(customSrv.URL, "sk-custom-999", false, "custom-edit-v1", ""); err != nil {
		t.Fatal(err)
	}
	auth := &stubCheapImageAuth{
		signedIn:   true,
		origin:     cheapSrv.URL,
		candidates: []cheapauth.ImageGroup{{GroupID: 1, Name: "img-group"}},
		keys:       map[int64]string{1: "sk-cheap-1"},
	}

	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	refFile := filepath.Join(root, "draft.png")
	if err := os.WriteFile(refFile, imgBytes, 0o644); err != nil {
		t.Fatal(err)
	}

	var script *mediaScript
	env := process.NewEnvironment([]string{"PATH=/usr/bin:/bin"})
	node, err := StartWithOptions(t.Context(), profile, Options{
		Environment: &env,
		Model: func(_ context.Context, _ model.Binding) (fantasy.LanguageModel, error) {
			return script, nil
		},
		ImageStudio: &ImageStudioOptions{
			Client:      imagestudio.NewClient().WithTimings(10*time.Millisecond, 5*time.Second),
			Auth:        auth,
			CustomStore: customStore,
			StaticCatalog: []cheapauth.Model{
				{ID: "gpt-image-2", BillingMode: "image"},
			},
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer node.Stop(context.Background())

	// Seed a plain chat provider (no native image capability!) so CheapRouter is selected automatically.
	provider := protocol.DefaultConversationProvider()
	provider.ID, provider.Name, provider.Enabled = protocol.NewProviderID(), "Chat Only", true
	provider.Api, provider.Authentication, provider.Endpoint, provider.DefaultModel = protocol.ConversationModelApiChatCompletions, protocol.AuthenticationApiKey, cheapSrv.URL+"/v1", "chat-plain"
	chatModel := protocol.DefaultConversationModel()
	chatModel.ID, chatModel.Context, chatModel.Output, chatModel.Tools, chatModel.CustomCapabilities = "chat-plain", 32000, 8000, true, true
	provider.Models = []protocol.ConversationModel{chatModel}
	secret := protocol.NewSecret("chat-key")
	commandFixture(t, node, protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{Provider: provider, Secret: &secret}})

	project := commandFixture(t, node, protocol.Command{Kind: "create_project", Data: protocol.ProjectsDraft{
		Name: "Image studio test", Path: root, Source: protocol.ProjectsSource{Kind: "local"}, Appearance: protocol.DefaultProjectsAppearance(),
	}}).Data.(protocol.Project)
	config := protocol.DefaultSessionConfig()
	config.Provider, config.Model, config.Mode, config.Permission = provider.ID, "chat-plain", protocol.WorkModeCode, protocol.PermissionFull
	session := commandFixture(t, node, protocol.Command{Kind: "create_session", Data: protocol.CommandCreateSessionPayload{Project: &project.ID, Config: &config}}).Data.(protocol.Session)

	// Upload one image attachment to test editing with BOTH worktree file and conversation attachment ID!
	attSpec := protocol.AttachmentSpec{
		Worktree:  session.Worktree,
		Name:      "attached.png",
		MediaType: "image/png",
		Size:      uint64(len(imgBytes)),
		Revision:  fmt.Sprintf("%x", blake3.Sum256(imgBytes)),
	}
	upload := commandFixture(t, node, protocol.Command{Kind: "upload_attachment", Data: &attSpec}).Data.(protocol.AttachmentUpload)
	stream, err := node.Open(t.Context(), node.ID(), upload.Stream)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := stream.Write(imgBytes); err != nil {
		t.Fatal(err)
	}
	if err := stream.CloseWrite(); err != nil {
		t.Fatal(err)
	}
	_, _ = io.ReadFull(stream, make([]byte, 1))
	_ = stream.Close()
	attached := commandFixture(t, node, protocol.Command{Kind: "finish_attachment_upload", Data: &protocol.CommandFinishAttachmentUploadPayload{Worktree: session.Worktree, Stream: upload.Stream}}).Data.(protocol.Attachment)

	// Step 1: discover generate_image -> call multi-image generation + edit with worktree file & attachment + custom edit!
	discover := [3]string{"discover", "tool_search", `{"query":"select:generate_image"}`}
	editArgs, _ := json.Marshal(map[string]any{
		"prompt":     "combine both into a night scene",
		"file_name":  "edited.png",
		"references": []string{"draft.png", attached.ID.String()},
	})
	customEditArgs, _ := json.Marshal(map[string]any{
		"prompt":     "restyle in watercolor",
		"channel":    "custom",
		"file_name":  "custom-edited.png",
		"references": []string{"draft.png"},
	})
	script = &mediaScript{
		results: map[string]string{},
		steps: [][][3]string{
			{discover},
			{{"multi", "generate_image", `{"prompt":"two sunset islands","file_name":"sunset.png","size":"1536x1024","quality":"high","count":2}`}},
			{{"edit", "generate_image", string(editArgs)}},
			{{"custom_edit", "generate_image", string(customEditArgs)}},
		},
	}

	subscription, err := node.Subscribe(t.Context(), node.ID(), protocol.Topic{Kind: "node"})
	if err != nil {
		t.Fatal(err)
	}
	defer subscription.Close()

	commandFixture(t, node, protocol.Command{
		Kind: "submit_turn",
		Data: protocol.CommandSubmitTurnPayload{
			Session:          session.ID,
			ExpectedRevision: session.Revision,
			Message: protocol.ConversationInput{
				Text:        "generate and edit images",
				Attachments: []protocol.AttachmentID{attached.ID},
			},
		},
	})

	// Wait for turn completion.
	for completed := false; !completed; {
		var history protocol.ConversationHistory
		if err := node.profile.Store.Node.Read(t.Context(), func(db *sql.Conn) error {
			var err error
			history, err = store.ReadConversation(t.Context(), db, session.ID, nil, 100)
			return err
		}); err != nil {
			t.Fatal(err)
		}
		for _, run := range history.Page.Runs {
			switch run.Status {
			case protocol.ConversationStatusCompleted:
				completed = true
			case protocol.ConversationStatusFailed, protocol.ConversationStatusInterrupted, protocol.ConversationStatusCancelled:
				t.Fatalf("turn ended with %s: results=%v", run.Status, script.results)
			}
		}
		if !completed {
			if _, err := subscription.Next(t.Context()); err != nil {
				t.Fatal(err)
			}
		}
	}

	// Verify multi-image files on disk & JSON result (no base64!).
	for _, rel := range []string{"assets/generated/sunset-1.png", "assets/generated/sunset-2.png", "assets/generated/edited.png", "assets/generated/custom-edited.png"} {
		data, err := os.ReadFile(filepath.Join(root, filepath.FromSlash(rel)))
		if err != nil || !bytes.Equal(data, imgBytes) {
			t.Fatalf("expected saved file %s: err=%v len=%d", rel, err, len(data))
		}
	}
	multiJSON := script.results["multi"]
	for _, want := range []string{"assets/generated/sunset-1.png", "assets/generated/sunset-2.png", "imgtask_multi", "cheaprouter", "gpt-image-2"} {
		if !strings.Contains(multiJSON, want) {
			t.Fatalf("multi result %q missing %q", multiJSON, want)
		}
	}
	if strings.Contains(multiJSON, b64) {
		t.Fatal("multi result JSON leaked base64 bytes")
	}

	// Verify CheapRouter edits received BOTH reference images (file + attachment).
	mu.Lock()
	gotFiles, gotPrompt, gotCustom := len(sawEditFiles), sawEditPrompt, sawCustomEdits
	mu.Unlock()
	if gotFiles != 2 || gotPrompt != "combine both into a night scene" || gotCustom != 1 {
		t.Fatalf("edits received cheapFiles=%d prompt=%q customFiles=%d", gotFiles, gotPrompt, gotCustom)
	}

	// Verify conversation asset inventory registered BOTH multi-image outputs.
	var assetsPage protocol.ConversationAssetsPage
	if err := node.profile.Store.Node.Read(t.Context(), func(db *sql.Conn) error {
		var err error
		assetsPage, err = store.ListConversationAssets(t.Context(), db, session.ID, protocol.ConversationAssetsQuery{Limit: 50})
		return err
	}); err != nil {
		t.Fatal(err)
	}
	var foundFiles []string
	for _, grp := range assetsPage.Groups {
		for _, item := range grp.Items {
			if item.Kind == "file" {
				if p, ok := item.Data.(*protocol.ConversationAssetsTargetFilePayload); ok {
					foundFiles = append(foundFiles, p.Path)
				}
			}
		}
	}
	for _, want := range []string{"assets/generated/sunset-1.png", "assets/generated/sunset-2.png", "assets/generated/edited.png", "assets/generated/custom-edited.png"} {
		if !slices.Contains(foundFiles, want) {
			t.Fatalf("conversation assets missing %q: got %v", want, foundFiles)
		}
	}

	// Verify Custom API unsupported edits (405) returns InvalidRequest cleanly.
	customNoEdits.Store(true)
	specs, err := node.mediaTools(&store.AgentInvocation{
		Session:  session,
		Provider: &provider,
		Worktree: protocol.Worktree{ID: session.Worktree, Path: root},
	})
	if err != nil {
		t.Fatal(err)
	}
	var genToolIdx = -1
	for i := range specs {
		if specs[i].Definition.Name == "generate_image" {
			genToolIdx = i
			break
		}
	}
	if genToolIdx < 0 {
		t.Fatal("generate_image tool missing")
	}
	genTool := specs[genToolIdx]

	// Bad references (path traversal) and multi-image existing file collision rejected in Prepare.
	for _, bad := range []string{
		`{"prompt":"edit","references":["../secret.png"]}`,
		`{"prompt":"edit","references":[]}`,
		`{"prompt":"two","file_name":"sunset.png","count":2}`, // sunset-1.png already exists!
	} {
		if err := genTool.Prepare(t.Context(), canonical.ToolRequest{ID: "bad", Arguments: json.RawMessage(bad)}); err == nil {
			t.Fatalf("expected Prepare to reject %s", bad)
		}
	}

	// Custom 405 unsupported edits returns ErrorCodeInvalidRequest.
	req405 := canonical.ToolRequest{ID: "c405", Arguments: json.RawMessage(`{"prompt":"edit","channel":"custom","file_name":"c405.png","references":["draft.png"]}`)}
	if err := genTool.Prepare(t.Context(), req405); err != nil {
		t.Fatal(err)
	}
	res405 := genTool.Execute(t.Context(), req405)
	if res405.Failure == nil || res405.Failure.Code != protocol.ErrorCodeInvalidRequest || !strings.Contains(res405.Failure.Message, "/v1/images/edits") {
		t.Fatalf("expected InvalidRequest for unsupported custom edits, got %+v", res405.Failure)
	}

	// Cancellation during async polling returns ErrorCodeCancelled promptly.
	slowMode.Store(true)
	reqCancel := canonical.ToolRequest{ID: "cancel1", Arguments: json.RawMessage(`{"prompt":"slow","file_name":"cancel.png"}`)}
	if err := genTool.Prepare(t.Context(), reqCancel); err != nil {
		t.Fatal(err)
	}
	execCtx, cancelExec := context.WithCancel(t.Context())
	go func() {
		select {
		case <-pollStarted:
			cancelExec()
		case <-time.After(5 * time.Second):
			cancelExec()
		}
	}()
	resCancel := genTool.Execute(execCtx, reqCancel)
	close(blockPoll)
	if resCancel.Failure == nil || resCancel.Failure.Code != protocol.ErrorCodeCancelled {
		t.Fatalf("expected Cancelled on ctx cancel, got %+v", resCancel.Failure)
	}
}
