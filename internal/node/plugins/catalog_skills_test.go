package plugins

import (
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestSkillCatalogSkipsUnsupportedRows(t *testing.T) {
	for _, tc := range []struct {
		source protocol.PluginSkillcatalogSource
		path   string
		body   string
		kind   string
	}{
		{
			source: protocol.PluginSkillcatalogSourceSkillsSh,
			path:   "/api/skills/all-time/2",
			body: `{"skills":[
				{"source":"google-labs-code/stitch-skills","skillId":"react:components","name":"React Components"},
				{"source":"team/..","skillId":"inspect","name":"Inspect"},
				{"source":"team/tools","skillId":"review","name":"Review"}
			],"total":10000,"hasMore":true}`,
			kind: "git",
		},
		{
			source: protocol.PluginSkillcatalogSourceSkillhub,
			path:   "/api/skills?page=3&pageSize=24",
			body: `{"code":0,"data":{"skills":[
				{"namespace":{"handle":"team"},"slug":"invalid-version","name":"Unsupported","version":"latest"},
				{"namespace":{"handle":"team"},"slug":"unnamed","name":" "},
				{"namespace":{"handle":"tools"},"slug":"review","name":"Review","version":"1.2.3"}
			],"total":10000}}`,
			kind: "registry",
		},
	} {
		t.Run(string(tc.source), func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
				if req.URL.RequestURI() != tc.path {
					t.Errorf("request = %s, want %s", req.URL.RequestURI(), tc.path)
					w.WriteHeader(http.StatusBadRequest)
					return
				}
				_, _ = io.WriteString(w, tc.body)
			}))
			defer server.Close()
			manager := NewManager(t.TempDir())
			manager.Client = server.Client()
			manager.SkillsDirectoryURL, manager.RegistryURL = server.URL, server.URL
			page, err := manager.SkillCatalog(t.Context(), tc.source, "", 3, false)
			if err != nil || page.Unavailable || page.Page != 3 || page.Next == nil || *page.Next != 4 || page.Total == nil || *page.Total != 10000 {
				t.Fatalf("pagination after unsupported rows: %#v, %v", page, err)
			}
			if len(page.Entries) != 1 || page.Entries[0].Name != "review" || page.Entries[0].Source == nil || page.Entries[0].Source.Kind != tc.kind {
				t.Fatalf("expected only the installable skill, got %#v", page.Entries)
			}
		})
	}
}

func TestSkillCatalogDistinguishesEmptyAndInvalidSearch(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		if req.URL.Path != "/api/search" {
			t.Errorf("expected catalog search, got %s", req.URL.Path)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		if req.URL.Query().Get("q") == "no-match" {
			_, _ = io.WriteString(w, `{"skills":[]}`)
		} else {
			_, _ = io.WriteString(w, `{"unexpected":[]}`)
		}
	}))
	defer server.Close()
	manager := NewManager(t.TempDir())
	manager.Client, manager.SkillsDirectoryURL = server.Client(), server.URL
	source := protocol.PluginSkillcatalogSourceSkillsSh
	page, err := manager.SkillCatalog(t.Context(), source, "no-match", 1, false)
	if err != nil || page.Unavailable || page.Next != nil || len(page.Entries) != 0 {
		t.Fatalf("valid empty search: %#v, %v", page, err)
	}
	_, err = manager.SkillCatalog(t.Context(), source, "invalid-response", 1, false)
	var fault *protocol.Fault
	if !errors.As(err, &fault) || fault.Code != protocol.ErrorCodeInvalidRequest {
		t.Fatalf("malformed response must not look like an empty search: %v", err)
	}
}
