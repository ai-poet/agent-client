package trace

import (
	"net/http"
	"path/filepath"
	"strings"
	"testing"
)

func TestGatewayCorrelationStaysOnTrustedTransportClone(t *testing.T) {
	const header = "X-Sailry-Trace-Id"
	enabled := false
	a := Attempt{request: "request-1", scope: Scope{GatewayEnabled: func() bool { return enabled }}}
	for _, tc := range []struct {
		url, host          string
		on, redirect, want bool
	}{
		{"https://cheaprouter.cc/v1", "", false, false, false},
		{"https://cheaprouter.cc/v1", "", true, false, true},
		{"https://CDN.cheaprouter.cc:443/path", "", true, false, true},
		{"http://cheaprouter.cc", "", true, false, false},
		{"https://cheaprouter.cc:8443", "", true, false, false},
		{"https://cheaprouter.cc.example.org", "", true, false, false},
		{"https://user@cheaprouter.cc", "", true, false, false},
		{"https://cheaprouter.cc", "other.example", true, false, false},
		{"https://cheaprouter.cc", "", true, true, false},
	} {
		enabled = tc.on
		r, err := http.NewRequest("POST", tc.url, nil)
		if err != nil {
			t.Fatal(err)
		}
		if tc.host != "" {
			r.Host = tc.host
		}
		if tc.redirect {
			r.Response = &http.Response{}
		}
		wire := a.GatewayRequest(r)
		if r.Header.Get(header) != "" || (wire.Header.Get(header) == a.request) != tc.want {
			t.Errorf("%+v: %v", tc, wire.Header)
		}
	}
}

func TestCredentialShapesAndTransportEchoes(t *testing.T) {
	for _, v := range []string{"Basic Zm9vOmJhcg==", "github_pat_sentinel", "gho_sentinel", "glpat-sentinel", "xoxb-sentinel", "AIzaSy_example", "eyJhbGciOiJub25lIn0.eyJzdWIiOiIxIn0.signature"} {
		if scrub(v, nil) != mask {
			t.Errorf("unmasked shape %q", v)
		}
	}
	scope := Scope{Profile: t.TempDir(), Session: "s"}
	a := scope.Begin("r", 1)
	body := `{"message":"adapter-value opaque-cookie", "private_key":"bare-value"}`
	a.Request(http.Header{"Authorization": {"Bearer adapter-value"}, "Cookie": {"sid=opaque-cookie"}}, []byte(body))
	a.Finish(Facts{})
	got := readGZ(t, filepath.Join(scope.Profile, "traces", "s", "bodies", "r-1.req.gz"))
	for _, v := range []string{"adapter-value", "opaque-cookie", "bare-value"} {
		if strings.Contains(got, v) {
			t.Fatalf("secret persisted: %s", got)
		}
	}
}

func TestFragmentedStreamCredentialsOmitBody(t *testing.T) {
	for _, parts := range [][2]string{{"actual-", "credential"}, {"github_", "pat_value"}} {
		b := []byte("data: {\"delta\":{\"content\":\"" + parts[0] + "\"}}\n\ndata: {\"delta\":{\"content\":\"" + parts[1] + "\"}}\n\n")
		if _, ok := redactSSE(b, []string{"actual-credential"}); ok {
			t.Fatal("reconstructible credential retained")
		}
	}
}
