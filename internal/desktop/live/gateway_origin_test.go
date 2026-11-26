package live

import (
	"net/http"
	"slices"
	"strings"
	"testing"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
)

func TestCheapGatewayOriginSwitchUpdatesProvidersAndRollsBackOnFailure(t *testing.T) {
	loginGW := newCheapGateway(t)
	altGW := newCheapGateway(t)
	brokenGW := newCheapGateway(t)
	brokenGW.set(func() { brokenGW.down = true })

	// Seed credentials pointing at loginGW so startup establishes Ready state.
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		store, err := core.NewStore(d.profile)
		if err != nil {
			t.Fatal(err)
		}
		if err := store.Save(core.Credentials{
			AccessToken:  cheapTestAccess,
			RefreshToken: cheapTestRefresh,
			ExpiresAt:    time.Now().Add(time.Hour).Unix(),
			Endpoint:     loginGW.URL,
			APIKey:       cheapTestKey,
		}); err != nil {
			t.Fatal(err)
		}
	})

	d.await("initial sync ready", func() bool {
		return d.account().Stage == cheapauthui.Ready
	})

	// Confirm initial CheapRouter provider points at loginGW.
	prov := d.provider(providers.CheapRouterProviderID)
	if prov == nil || !strings.HasPrefix(prov.Endpoint, loginGW.URL) {
		t.Fatalf("initial provider = %+v, want prefix %s", prov, loginGW.URL)
	}

	// Add altGW and brokenGW as user candidates.
	d.account().AddOrigin(altGW.URL)
	d.account().AddOrigin(brokenGW.URL)
	d.tester.Frame()
	if d.account().OriginError != "" {
		t.Fatalf("unexpected AddOrigin error: %q", d.account().OriginError)
	}

	// Switch to altGW: providers move to altGW while login Endpoint stays loginGW.
	d.account().SelectOrigin(altGW.URL)
	d.await("switched to altGW", func() bool {
		return !d.account().OriginSwitching && d.account().ActiveOrigin() == altGW.URL
	})
	if d.account().OriginError != "" || d.account().Stage != cheapauthui.Ready {
		t.Fatalf("switch to altGW failed: stage=%v err=%q", d.account().Stage, d.account().OriginError)
	}
	if d.account().Endpoint != loginGW.URL {
		t.Fatalf("login Endpoint changed to %q, want %q", d.account().Endpoint, loginGW.URL)
	}
	d.await("provider endpoint updated to altGW", func() bool {
		p := d.provider(providers.CheapRouterProviderID)
		return p != nil && strings.HasPrefix(p.Endpoint, altGW.URL)
	})
	if got := d.runtime.cheapAuth.service.GatewayOriginConfig().SelectedOrigin(); got != altGW.URL {
		t.Fatalf("persisted chosen = %q, want %q", got, altGW.URL)
	}
	if !slices.Contains(altGW.requests(), "GET /models/catalog") {
		t.Fatalf("altGW did not receive catalog request: %v", altGW.requests())
	}

	// Attempting to switch to brokenGW must fail, roll back to altGW, preserve Ready
	// state and provider endpoint, and populate OriginError.
	d.account().SelectOrigin(brokenGW.URL)
	d.await("rollback after brokenGW failure", func() bool {
		return !d.account().OriginSwitching && d.account().OriginError != ""
	})
	if d.account().ActiveOrigin() != altGW.URL {
		t.Fatalf("active origin after failed switch = %q, want rollback to %q", d.account().ActiveOrigin(), altGW.URL)
	}
	if got := d.runtime.cheapAuth.service.GatewayOriginConfig().SelectedOrigin(); got != altGW.URL {
		t.Fatalf("persisted chosen after rollback = %q, want %q", got, altGW.URL)
	}
	if d.account().Stage != cheapauthui.Ready {
		t.Fatalf("stage after rollback = %v, want Ready", d.account().Stage)
	}
	if p := d.provider(providers.CheapRouterProviderID); p == nil || !strings.HasPrefix(p.Endpoint, altGW.URL) {
		t.Fatalf("provider after rollback = %+v, want prefix %s", p, altGW.URL)
	}

	// Logout revocation must still go to loginGW, never altGW.
	d.account().Logout()
	d.await("signed out", func() bool {
		return d.account().Stage == cheapauthui.SignedOut && !d.account().LoggingOut
	})
	if !slices.Contains(loginGW.requests(), "POST /auth/logout") {
		t.Fatalf("logout did not hit loginGW: %v", loginGW.requests())
	}
	if slices.Contains(altGW.requests(), "POST /auth/logout") {
		t.Fatalf("logout leaked to altGW: %v", altGW.requests())
	}
}

func TestCheapGatewayOriginRejectionReturnsToLogin(t *testing.T) {
	login := newCheapGateway(t)
	previous := newCheapGateway(t)
	alt := newCheapGateway(t)
	alt.set(func() { alt.groupsBody = `{"code":401,"message":"sensitive-response-fixture"}` })
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		d.signedIn(login.URL, cheapTestAccess, time.Hour)
	})
	d.await("initial sync ready", func() bool { return d.account().Stage == cheapauthui.Ready })
	d.account().SelectOrigin(previous.URL)
	d.await("routed away from login", func() bool {
		return !d.account().OriginSwitching && d.account().ActiveOrigin() == previous.URL
	})
	d.account().SelectOrigin(alt.URL)
	d.await("rejected origin returns to login", func() bool {
		return !d.account().OriginSwitching && d.account().ActiveOrigin() == login.URL
	})
	p := d.gateway(true)
	if d.account().Stage != cheapauthui.Ready || d.account().OriginError == "" ||
		!strings.HasPrefix(p.Endpoint, login.URL) || d.runtime.cheapAuth.service.GatewayOrigin() != login.URL ||
		d.stored() == nil || d.stored().AccessToken != cheapTestAccess {
		t.Fatal("rollback did not preserve credentials and resync the provider on the login origin")
	}
	if slices.Contains(alt.requests(), "POST /auth/refresh") {
		t.Fatal("sent a refresh token to the rejected origin")
	}
}

func TestCheapGatewayOriginInvalidatesOnConfirmedLoginRejection(t *testing.T) {
	login, alt := newCheapGateway(t), newCheapGateway(t)
	checkLog := captureCheapInvalidation(t)
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		d.signedIn(login.URL, cheapTestAccess, time.Hour)
	})
	d.await("initial sync ready", func() bool { return d.account().Stage == cheapauthui.Ready })
	for _, gw := range []*cheapGateway{alt, login} {
		gw.set(func() { gw.groupsBody = `{"code":401,"message":"sensitive-response-fixture"}` })
	}
	d.account().SelectOrigin(alt.URL)
	d.await("login confirms rejection", func() bool { return d.account().Stage == cheapauthui.SignedOut })
	if !d.account().LoginExpired {
		t.Fatal("origin rejection did not show recovery guidance")
	}
	// The rejected origin was rolled back to the login origin, which renewed
	// once and then confirmed the rejection itself: the session ends, and the
	// invalidation is recorded against the login origin, not the rejected one.
	checkLog("origin_switch", "/groups/available", http.StatusOK, true)
	if d.stored() != nil || d.runtime.cheapAuth.service.GatewayOriginConfig().SelectedOrigin() != "" ||
		!slices.Contains(login.requests(), "POST /auth/refresh") || slices.Contains(alt.requests(), "POST /auth/refresh") {
		t.Fatal("session did not end after rollback and login-only renewal")
	}
	d.gateway(false)
}
