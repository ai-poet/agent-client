package live

import "testing"

func TestLoginExpiryBannerWindowLifetime(t *testing.T) {
	var process loginExpiry
	var first, second, unrelated int
	closeFirst := process.watch("profile", func() { first++ })
	defer process.watch("profile", func() { second++ })()
	defer process.watch("other", func() { unrelated++ })()
	process.set("profile", true)
	if first != 1 || second != 1 || unrelated != 0 {
		t.Fatal("credential rejection did not notify the profile's windows")
	}
	closeFirst()
	process.dismiss()
	if first != 1 || second != 2 || unrelated != 1 {
		t.Fatal("dismissal did not notify exactly the remaining windows")
	}
	if expired, dismissed := process.snapshot("profile"); !expired || !dismissed {
		t.Fatal("dismissal cleared the underlying account status")
	}
	process.set("profile", false)
	if expired, dismissed := process.snapshot("profile"); expired || !dismissed {
		t.Fatal("new windows should inherit dismissal, but not a resolved expiry")
	}
	process.set("profile", true)
	if expired, dismissed := process.snapshot("profile"); !expired || !dismissed {
		t.Fatal("a later rejection must not show a dismissed banner")
	}
	if expired, dismissed := new(loginExpiry).snapshot("profile"); expired || dismissed {
		t.Fatal("dismissal survived a new process")
	}
}
