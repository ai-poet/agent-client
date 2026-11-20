package main

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"os"
	"strconv"
	"strings"
	"testing"

	"github.com/Masterminds/semver/v3"
	"github.com/ai-poet/agent-client/desktop/internal/core"
	"github.com/ai-poet/agent-client/desktop/internal/link"
	clientupdate "github.com/ai-poet/agent-client/desktop/internal/update"
)

func TestProductIdentityAlignedWithMain(t *testing.T) {
	raw, err := os.ReadFile("mygo.json")
	if err != nil {
		t.Fatal(err)
	}
	var config struct {
		Name       string `json:"name"`
		Slug       string `json:"slug"`
		Identifier string `json:"identifier"`
		Version    string `json:"version"`
		Updates    *struct {
			URL       string `json:"url"`
			PublicKey string `json:"publicKey"`
		} `json:"updates"`
		MacOS struct {
			InfoPlist map[string]any `json:"infoPlist"`
		} `json:"macos"`
		Windows struct {
			DisplayName    string `json:"displayName"`
			InstallDirName string `json:"installDirName"`
		} `json:"windows"`
	}
	if err := json.Unmarshal(raw, &config); err != nil {
		t.Fatal(err)
	}
	if config.Updates == nil || config.Updates.URL != clientupdate.DefaultFeedBaseURL {
		t.Fatalf("mygo.json updates = %+v, want url=%q", config.Updates, clientupdate.DefaultFeedBaseURL)
	}
	// The public key is either the build-time placeholder (empty) or a real
	// base64 Ed25519 key shipped with the release; anything else cannot verify
	// a manifest.
	if key := config.Updates.PublicKey; key != "" {
		decoded, err := base64.StdEncoding.DecodeString(key)
		if err != nil || len(decoded) != ed25519.PublicKeySize {
			t.Fatalf("mygo.json updates publicKey = %q, want empty or a base64 Ed25519 key", key)
		}
	}
	if core.MacOSBundleIdentifier != "sh.waku" || config.Identifier != core.MacOSBundleIdentifier {
		t.Fatalf("mygo.json identifier = %q (constant %q), want %q", config.Identifier, core.MacOSBundleIdentifier, "sh.waku")
	}
	if config.Name != core.AppName || config.Slug != core.AppSlug {
		t.Fatalf("mygo.json name/slug = %q/%q, want %q/%q", config.Name, config.Slug, core.AppName, core.AppSlug)
	}
	if config.Windows.DisplayName != core.AppName || config.Windows.InstallDirName != core.WindowsInstallDirName {
		t.Fatalf("mygo.json windows identity = %+v", config.Windows)
	}
	if config.Version != link.ProductVersion {
		t.Fatalf("mygo.json version %q != link.ProductVersion %q", config.Version, link.ProductVersion)
	}
	current := semver.MustParse(config.Version)
	mainRelease := semver.MustParse("0.3.8")
	if !current.GreaterThan(mainRelease) {
		t.Fatalf("version %s must exceed main release %s", current, mainRelease)
	}

	buildNumber, err := core.DerivedMacOSBuildNumber(config.Version)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(buildNumber, ".") {
		t.Fatalf("CFBundleVersion %q must be strictly numeric", buildNumber)
	}
	numeric, err := strconv.Atoi(buildNumber)
	if err != nil || numeric <= 3008 {
		t.Fatalf("CFBundleVersion %q must exceed main build 3008", buildNumber)
	}
	plistBuild, _ := config.MacOS.InfoPlist["CFBundleVersion"].(string)
	if plistBuild != buildNumber {
		t.Fatalf("mygo.json macos.infoPlist.CFBundleVersion = %q, want %q", plistBuild, buildNumber)
	}
	plistSparkleKey, _ := config.MacOS.InfoPlist["SUPublicEDKey"].(string)
	if plistSparkleKey == "" {
		t.Fatal("mygo.json macos.infoPlist.SUPublicEDKey must be set so Sparkle generate_appcast signs first-hop macOS zip")
	}
	if rootPlist, err := os.ReadFile("../../../resources/Info.plist"); err == nil {
		if !strings.Contains(string(rootPlist), "<string>"+plistSparkleKey+"</string>") {
			t.Fatalf("mygo.json SUPublicEDKey %q does not match resources/Info.plist", plistSparkleKey)
		}
	}

	macScript, err := os.ReadFile("macos/package.sh")
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{"ai.sailry.desktop", "ai.sailry.disk-image", "Set :CFBundleVersion $bundle_version", "Sailry.app"} {
		if strings.Contains(string(macScript), forbidden) {
			t.Fatalf("macos/package.sh still contains stale identity token %q", forbidden)
		}
	}
	winScript, err := os.ReadFile("windows/package.sh")
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{"Sailry.exe", "sailry-setup-", "makensis"} {
		if strings.Contains(string(winScript), forbidden) {
			t.Fatalf("windows/package.sh still contains stale or NSIS token %q", forbidden)
		}
	}
	if !strings.Contains(string(winScript), "-skip-installer") || !strings.Contains(string(winScript), "cheaprouter.iss") {
		t.Fatal("windows/package.sh must use -skip-installer and Inno Setup cheaprouter.iss")
	}
	issRaw, err := os.ReadFile("windows/cheaprouter.iss")
	if err != nil {
		t.Fatal(err)
	}
	issText := string(issRaw)
	for _, required := range []string{
		"AppId={" + core.WindowsInnoAppID,
		"PrivilegesRequired=lowest",
		"UsePreviousAppDir=yes",
		"OutputBaseFilename=CheapRouter-{#AppVersion}-{#Arch}-Setup",
		"CheapRouter.exe",
		"waku.exe",
		"client-daemon.exe",
	} {
		if !strings.Contains(issText, required) {
			t.Fatalf("windows/cheaprouter.iss missing required Inno Setup contract %q", required)
		}
	}
}

func TestDerivedMacOSBuildNumber(t *testing.T) {
	for version, want := range map[string]string{
		"0.3.8":      "3008",
		"0.4.0":      "4000",
		"0.4.0-rc.1": "4000",
		"1.2.3":      "1002003",
	} {
		got, err := core.DerivedMacOSBuildNumber(version)
		if err != nil || got != want {
			t.Fatalf("DerivedMacOSBuildNumber(%q) = %q, %v; want %q", version, got, err, want)
		}
	}
	for _, invalid := range []string{"", "0.4", "1.2.3.4", "a.b.c", "1.1000.0"} {
		if got, err := core.DerivedMacOSBuildNumber(invalid); err == nil {
			t.Fatalf("DerivedMacOSBuildNumber(%q) = %q, want error", invalid, got)
		}
	}
}
