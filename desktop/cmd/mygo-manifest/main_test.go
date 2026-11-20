package main

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	clientupdate "github.com/ai-poet/agent-client/desktop/internal/update"
)

func TestPackSignVerifyAndClientCompatibility(t *testing.T) {
	pub, priv, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	pubB64 := base64.StdEncoding.EncodeToString(pub)
	privB64 := base64.StdEncoding.EncodeToString(priv)
	t.Setenv("MYGO_UPDATE_PUBLIC_KEY", pubB64)
	t.Setenv("MYGO_UPDATE_PRIVATE_KEY", privB64)

	work := t.TempDir()
	// 1. macOS bundle fixture (with executable bit and relative symlink).
	macRoot := filepath.Join(work, "mac-stage")
	macContents := filepath.Join(macRoot, "CheapRouter.app", "Contents")
	if err := os.MkdirAll(filepath.Join(macContents, "MacOS"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(macContents, "Resources"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(macContents, "MacOS", "CheapRouter"), []byte("#!/bin/sh\necho ok\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(macContents, "Info.plist"), []byte("<plist/>"), 0o644); err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" {
		if err := os.Symlink("../Info.plist", filepath.Join(macContents, "Resources", "Info.plist.link")); err != nil {
			t.Fatal(err)
		}
	}

	// 2. Windows app directory fixture (with .mygo-update-files and excluded setup/zip).
	winRoot := filepath.Join(work, "win-stage")
	if err := os.MkdirAll(filepath.Join(winRoot, "helpers"), 0o755); err != nil {
		t.Fatal(err)
	}
	for name, body := range map[string]string{
		"CheapRouter.exe":                    "MZ-binary",
		"sailry_iroh.dll":                    "dll",
		"native-seal.json":                   `{"version":1}`,
		"build.json":                         `{"version":1}`,
		".mygo-update-files":                 "CheapRouter.exe\nsailry_iroh.dll\n",
		"helpers/agent-desktop.exe":          "helper",
		"CheapRouter-0.4.0-x86_64-Setup.exe": "should-be-excluded",
		"cheaprouter-0.4.0-x86_64-msvc.zip":  "should-be-excluded",
	} {
		p := filepath.Join(winRoot, filepath.FromSlash(name))
		if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}

	outDir := filepath.Join(work, "dist")
	macArchive := filepath.Join(outDir, "cheaprouter-0.4.0-darwin-arm64.tar.gz")
	if err := run([]string{"pack", "-root", macRoot, "-out", macArchive, "-target", "darwin-arm64", "CheapRouter.app"}); err != nil {
		t.Fatalf("pack macOS: %v", err)
	}
	winArchive := filepath.Join(outDir, "cheaprouter-0.4.0-windows-amd64.tar.gz")
	if err := run([]string{"pack", "-root", winRoot, "-out", winArchive, "-target", "windows-amd64", "-exclude", "*-Setup.exe,*.zip,*.tar.gz"}); err != nil {
		t.Fatalf("pack Windows: %v", err)
	}

	notesPath := filepath.Join(work, "notes.md")
	if err := os.WriteFile(notesPath, []byte("- Two-hop release\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	mygoJSONPath := filepath.Join(work, "mygo.json")
	if err := os.WriteFile(mygoJSONPath, []byte(`{"slug":"cheaprouter","version":"0.4.0","updates":{"url":"https://s3.cheaprouter.cc/cheaprouter-releases/mygo","publicKey":"`+pubB64+`"}}`), 0o644); err != nil {
		t.Fatal(err)
	}

	prevDir := filepath.Join(work, "prev")
	if err := os.MkdirAll(prevDir, 0o755); err != nil {
		t.Fatal(err)
	}
	prevManifest := Manifest{
		Version:   "0.3.9",
		URL:       "https://s3.cheaprouter.cc/cheaprouter-releases/mygo/cheaprouter-0.3.9-darwin-arm64.tar.gz",
		Size:      1234,
		Signature: "prev-sig",
	}
	if err := writeJSONAtomic(filepath.Join(prevDir, "update-darwin-arm64.json"), prevManifest); err != nil {
		t.Fatal(err)
	}

	if err := run([]string{
		"sign",
		"-dir", outDir,
		"-out", outDir,
		"-prev-dir", prevDir,
		"-version", "0.4.0",
		"-slug", "cheaprouter",
		"-notes-file", notesPath,
		"-targets", "darwin-arm64,windows-amd64",
		"-mygo-config", mygoJSONPath,
		"-date", "2026-11-20T00:00:00Z",
	}); err != nil {
		t.Fatalf("sign: %v", err)
	}

	if err := run([]string{
		"verify",
		"-dir", outDir,
		"-targets", "darwin-arm64,windows-amd64",
		"-mygo-config", mygoJSONPath,
	}); err != nil {
		t.Fatalf("verify: %v", err)
	}

	// Verify manifest fields match desktop/internal/update & MyGo expectations.
	rawMac, err := os.ReadFile(filepath.Join(outDir, "update-darwin-arm64.json"))
	if err != nil {
		t.Fatal(err)
	}
	var macManifest Manifest
	if err := json.Unmarshal(rawMac, &macManifest); err != nil {
		t.Fatal(err)
	}
	if macManifest.Version != "0.4.0" ||
		macManifest.URL != "https://s3.cheaprouter.cc/cheaprouter-releases/mygo/cheaprouter-0.4.0-darwin-arm64.tar.gz" ||
		macManifest.Notes != "- Two-hop release" ||
		macManifest.Date != "2026-11-20T00:00:00Z" ||
		len(macManifest.Previous) != 1 || macManifest.Previous[0].Version != "0.3.9" {
		t.Fatalf("unexpected macOS manifest: %+v", macManifest)
	}

	// Serve via httptest and verify desktop/internal/update.Source can fetch and verify SHA-256 signature.
	srv := httptest.NewServer(http.FileServer(http.Dir(outDir)))
	defer srv.Close()
	src := &clientupdate.Source{FeedURL: srv.URL + "/update-windows-amd64.json", Version: "0.3.9"}
	rc, err := src.Manifest(context.Background(), false)
	if err != nil {
		t.Fatalf("Source.Manifest: %v", err)
	}
	defer rc.Close()
	var winManifest Manifest
	if err := json.NewDecoder(rc).Decode(&winManifest); err != nil {
		t.Fatalf("decode windows manifest: %v", err)
	}
	archiveRC, err := src.Open(context.Background(), srv.URL+"/cheaprouter-0.4.0-windows-amd64.tar.gz")
	if err != nil {
		t.Fatalf("Source.Open: %v", err)
	}
	defer archiveRC.Close()
	archiveBytes, err := io.ReadAll(archiveRC)
	if err != nil {
		t.Fatal(err)
	}
	if int64(len(archiveBytes)) != winManifest.Size {
		t.Fatalf("downloaded size = %d, want %d", len(archiveBytes), winManifest.Size)
	}
	digest := sha256.Sum256(archiveBytes)
	sig, err := base64.StdEncoding.DecodeString(winManifest.Signature)
	if err != nil || !ed25519.Verify(pub, digest[:], sig) {
		t.Fatal("SHA-256 Ed25519 signature verification failed")
	}
}

func TestSignRejectsMissingOrMismatchedKeys(t *testing.T) {
	pub, priv, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	otherPub, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	work := t.TempDir()

	t.Run("missing private key", func(t *testing.T) {
		t.Setenv("MYGO_UPDATE_PRIVATE_KEY", "")
		t.Setenv("MYGO_UPDATER_PRIVATE_KEY", "")
		t.Setenv("MYGO_UPDATE_PUBLIC_KEY", base64.StdEncoding.EncodeToString(pub))
		err := run([]string{"sign", "-dir", work, "-version", "0.4.0", "-targets", "darwin-arm64"})
		if err == nil || !strings.Contains(err.Error(), "MYGO_UPDATE_PRIVATE_KEY") {
			t.Fatalf("err = %v, want MYGO_UPDATE_PRIVATE_KEY required", err)
		}
	})

	t.Run("mismatched public key", func(t *testing.T) {
		t.Setenv("MYGO_UPDATE_PRIVATE_KEY", base64.StdEncoding.EncodeToString(priv))
		t.Setenv("MYGO_UPDATE_PUBLIC_KEY", base64.StdEncoding.EncodeToString(otherPub))
		err := run([]string{"sign", "-dir", work, "-version", "0.4.0", "-targets", "darwin-arm64"})
		if err == nil || !strings.Contains(err.Error(), "does not match") {
			t.Fatalf("err = %v, want key mismatch error", err)
		}
	})

	t.Run("32-byte seed private key accepted", func(t *testing.T) {
		seed := priv.Seed()
		parsed, err := parsePrivateKey(base64.StdEncoding.EncodeToString(seed))
		if err != nil {
			t.Fatalf("parsePrivateKey(seed): %v", err)
		}
		if got := base64.StdEncoding.EncodeToString(parsed.Public().(ed25519.PublicKey)); got != base64.StdEncoding.EncodeToString(pub) {
			t.Fatalf("derived pub = %s, want %s", got, base64.StdEncoding.EncodeToString(pub))
		}
	})
}
