package trace

import (
	"net/http"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
)

// Only the transport clone carries this header: net/http cannot copy it
// from the original request to a redirect. SDK clients also refuse redirects.
func (a *Attempt) GatewayRequest(r *http.Request) *http.Request {
	if a == nil || a.scope.GatewayEnabled == nil || !a.scope.GatewayEnabled() || a.scope.Enabled != nil && !a.scope.Enabled() {
		return r
	}
	u := r.URL
	if u == nil || u.Scheme != "https" || u.User != nil || u.Opaque != "" || r.Response != nil || r.Host != "" && !strings.EqualFold(r.Host, u.Host) {
		return r
	}
	origin := "https://" + strings.ToLower(strings.TrimSuffix(u.Host, ":443"))
	if origin != cheapauth.PrimaryEndpoint && origin != cheapauth.CDNEndpoint {
		return r
	}
	copy := r.Clone(r.Context())
	copy.Header.Set("X-Sailry-Trace-Id", a.request)
	return copy
}
