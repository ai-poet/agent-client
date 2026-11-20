package update

import "errors"

// Stable error classes let the interface explain a failure without exposing
// configuration material or depending on a platform-specific network error.
var (
	ErrDisabled      = errors.New("updates are disabled")
	ErrUnconfigured  = errors.New("update public key is not configured")
	ErrUnavailable   = errors.New("this build cannot install updates")
	ErrConfiguration = errors.New("update configuration is invalid")
	ErrNetwork       = errors.New("update network request failed")
	ErrNotFound      = errors.New("update manifest was not found")
	ErrManifest      = errors.New("update manifest is invalid")
	ErrSignature     = errors.New("update signature does not match")
	ErrInstall       = errors.New("update installation failed")
)
