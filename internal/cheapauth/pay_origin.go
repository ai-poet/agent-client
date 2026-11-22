package cheapauth

import "path/filepath"

// gatewayOriginForProfile resolves the business API origin from the profile's
// gateway-origin.json, falling back to login when no explicit candidate is
// chosen or the file is absent/unreadable.
func gatewayOriginForProfile(profileDir, login string) (origin string, overridden bool) {
	store, err := NewGatewayOriginStore(profileDir)
	if err != nil {
		return login, false
	}
	cfg := store.Load()
	if selected := cfg.SelectedOrigin(); selected != "" {
		return selected, true
	}
	return login, false
}

// Only an explicit choice still in the profile's candidate list overrides
// the login endpoint. In particular, a self-hosted login never implicitly
// routes account traffic to the managed primary.
func paymentOrigin(credentialPath, login string) string {
	origin, _ := gatewayOriginForProfile(filepath.Dir(credentialPath), login)
	return origin
}
