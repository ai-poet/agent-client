package settings

import (
	"slices"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preview"
)

func TestProviderKeyAndHeaderDraftsStayWithBrandAndMethod(t *testing.T) {
	m := NewModel(preview.SettingsData())
	e := &m.provider
	e.add(m)
	first, second := preview.Brand{ID: "first"}, preview.Brand{ID: "second"}
	method := preview.Method{ID: "keyed", Keyed: true}
	pick := func(b *preview.Brand) { m.chooseProvider(b, []preview.Method{method}) }
	pick(&first)
	e.key, e.keyShown, e.headers = "first-key", true, []header{{"X-First", "one"}}
	e.back()
	pick(&second)
	if e.keyUpdate() != "" || e.keyShown || len(e.headers) != 0 {
		t.Fatal("another brand inherited the first brand's authentication draft")
	}
	e.key, e.headers = "second-key", []header{{"X-Second", "two"}}
	e.back()
	pick(&first)
	if e.keyUpdate() != "first-key" || e.keyShown || !slices.Equal(e.headers, []header{{"X-First", "one"}}) {
		t.Fatal("returning to the first brand did not restore only its own masked key and headers")
	}
}

func TestProviderOAuthSaveIgnoresHiddenKeyHeaderDraft(t *testing.T) {
	m := NewModel(preview.SettingsData())
	m.Live = true
	var saved *ProviderDraft
	m.ProviderActions.Save = func(draft ProviderDraft, done func(bool)) {
		saved = &draft
		done(false)
	}
	e := &m.provider
	e.add(m)
	keyed := preview.Method{ID: "keyed", Name: "Key channel", Keyed: true, Routes: []preview.Route{{ID: "keyed/0", API: "responses", Endpoint: "https://example.test/v1"}}}
	e.configure(keyed)
	e.key, e.headers = "fixture-key", []header{{"", "invalid header draft"}}
	if problem := e.Check(); problem != "provider_headers_invalid" {
		t.Fatalf("visible API-key header was not checked: %q", problem)
	}
	e.back()
	e.configure(preview.Method{ID: "oauth", Name: "OAuth channel", Login: "fixture-login", Routes: []preview.Route{{ID: "oauth/0", API: "responses", Endpoint: "https://example.test/v1"}}})
	if problem := e.save(m); problem != "" {
		t.Fatalf("a hidden API-key draft prevented OAuth save: %s", problem)
	}
	if saved == nil || saved.Channel.Auth != preview.AuthOAuth || len(saved.Channel.Headers) != 0 {
		t.Fatalf("OAuth save retained API-key headers: %+v", saved)
	}
	e.configure(keyed)
	if problem := e.Check(); problem != "provider_headers_invalid" {
		t.Fatalf("returning to the API-key method lost its header validation: %q", problem)
	}
}
