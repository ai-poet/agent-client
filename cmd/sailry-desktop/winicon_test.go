package main

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"image/png"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/ai-poet/agent-client/assets/branding"
	"github.com/ai-poet/agent-client/internal/core"
)

var wantWindowsIconSizes = []int{16, 24, 32, 48, 64, 128, 256}

func TestWindowsIconConfigAndAssets(t *testing.T) {
	raw, err := os.ReadFile("mygo.json")
	if err != nil {
		t.Fatal(err)
	}
	var config struct {
		Identifier string `json:"identifier"`
		Icon       string `json:"icon"`
		Windows    struct {
			Icon string `json:"icon"`
		} `json:"windows"`
	}
	if err := json.Unmarshal(raw, &config); err != nil {
		t.Fatal(err)
	}
	if config.Identifier != core.WindowsAppUserModelID {
		t.Fatalf("mygo.json identifier = %q, want %q", config.Identifier, core.WindowsAppUserModelID)
	}
	if config.Icon == "" {
		t.Fatal("mygo.json top-level icon is empty")
	}
	if config.Windows.Icon == "" || !strings.HasSuffix(strings.ToLower(config.Windows.Icon), ".ico") {
		t.Fatalf("mygo.json windows.icon = %q, want a .ico path", config.Windows.Icon)
	}

	pngBytes, err := os.ReadFile(filepath.FromSlash(config.Icon))
	if err != nil {
		t.Fatalf("read mygo.json icon %q: %v", config.Icon, err)
	}
	if !bytes.Equal(pngBytes, branding.AppIconPNG) {
		t.Fatalf("embedded branding.AppIconPNG (%d bytes) differs from mygo.json icon %q (%d bytes)", len(branding.AppIconPNG), config.Icon, len(pngBytes))
	}
	cfg, err := png.DecodeConfig(bytes.NewReader(pngBytes))
	if err != nil {
		t.Fatalf("decode mygo.json icon PNG: %v", err)
	}
	if cfg.Width != cfg.Height || cfg.Width < 256 {
		t.Fatalf("mygo.json icon PNG is %dx%d, want square >= 256x256", cfg.Width, cfg.Height)
	}

	icoPath := filepath.FromSlash(config.Windows.Icon)
	assertValidMultiSizeICO(t, icoPath, wantWindowsIconSizes)
}

func assertValidMultiSizeICO(t *testing.T, path string, wantSizes []int) []byte {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	if len(data) < 6 {
		t.Fatalf("%s too short (%d bytes)", path, len(data))
	}
	le := binary.LittleEndian
	reserved := le.Uint16(data[0:2])
	iconType := le.Uint16(data[2:4])
	count := int(le.Uint16(data[4:6]))
	if reserved != 0 || iconType != 1 {
		t.Fatalf("%s invalid ICO header: reserved=%d type=%d", path, reserved, iconType)
	}
	if len(data) < 6+count*16 {
		t.Fatalf("%s truncated directory: len=%d count=%d", path, len(data), count)
	}

	gotSizes := make([]int, 0, count)
	for i := range count {
		entry := data[6+i*16 : 6+(i+1)*16]
		w, h := int(entry[0]), int(entry[1])
		if w == 0 {
			w = 256
		}
		if h == 0 {
			h = 256
		}
		planes := le.Uint16(entry[4:6])
		bpp := le.Uint16(entry[6:8])
		size := int(le.Uint32(entry[8:12]))
		offset := int(le.Uint32(entry[12:16]))
		if w != h {
			t.Fatalf("%s entry %d is non-square: %dx%d", path, i, w, h)
		}
		if planes != 1 || bpp != 32 {
			t.Fatalf("%s entry %d (%dx%d) planes=%d bpp=%d, want 1/32", path, i, w, h, planes, bpp)
		}
		if offset < 6+count*16 || size <= 0 || offset > len(data) || size > len(data)-offset {
			t.Fatalf("%s entry %d (%dx%d) out-of-range payload offset=%d size=%d total=%d", path, i, w, h, offset, size, len(data))
		}
		payload := data[offset : offset+size]
		imgCfg, err := png.DecodeConfig(bytes.NewReader(payload))
		if err != nil {
			t.Fatalf("%s entry %d (%dx%d) payload is not valid PNG: %v", path, i, w, h, err)
		}
		if imgCfg.Width != w || imgCfg.Height != h {
			t.Fatalf("%s entry %d declares %dx%d, PNG is %dx%d", path, i, w, h, imgCfg.Width, imgCfg.Height)
		}
		gotSizes = append(gotSizes, w)
	}
	slices.Sort(gotSizes)
	if !slices.Equal(gotSizes, wantSizes) {
		t.Fatalf("%s sizes = %v, want %v", path, gotSizes, wantSizes)
	}
	return data
}

func TestWindowsInstallerIconAndIdentityContract(t *testing.T) {
	issRaw, err := os.ReadFile("windows/cheaprouter.iss")
	if err != nil {
		t.Fatal(err)
	}
	iss := string(issRaw)
	for _, required := range []string{
		`#define IconFile "..\..\..\assets\branding\cheaprouter.ico"`,
		`#define AppUserModelID "` + core.WindowsAppUserModelID + `"`,
		`SetupIconFile={#IconFile}`,
		`UninstallDisplayIcon={app}\CheapRouter.exe,0`,
		`Name: "{group}\CheapRouter"; Filename: "{app}\CheapRouter.exe"; IconFilename: "{app}\CheapRouter.exe"; IconIndex: 0; AppUserModelID: "{#AppUserModelID}"`,
		`Name: "{userdesktop}\CheapRouter"; Filename: "{app}\CheapRouter.exe"; IconFilename: "{app}\CheapRouter.exe"; IconIndex: 0; AppUserModelID: "{#AppUserModelID}"; Tasks: desktopicon`,
	} {
		if !strings.Contains(iss, required) {
			t.Errorf("windows/cheaprouter.iss missing required line %q", required)
		}
	}
	defaultICO := filepath.Join("windows", "..", "..", "..", "assets", "branding", "cheaprouter.ico")
	if _, err := os.Stat(defaultICO); err != nil {
		t.Fatalf("default IconFile in cheaprouter.iss does not resolve to an existing file: %v", err)
	}
}

func TestWindowsPackageScriptDryRun(t *testing.T) {
	if _, err := exec.LookPath("bash"); err != nil {
		t.Skip("bash not available")
	}
	if _, err := exec.LookPath("jq"); err != nil {
		t.Skip("jq not available")
	}
	desktopRoot, err := filepath.Abs("../..")
	if err != nil {
		t.Fatal(err)
	}
	for _, arch := range []string{"amd64", "arm64"} {
		t.Run(arch, func(t *testing.T) {
			cmd := exec.Command("bash", "cmd/sailry-desktop/windows/package.sh", "--dry-run", "-arch", arch)
			cmd.Dir = desktopRoot
			out, err := cmd.CombinedOutput()
			if err != nil {
				t.Fatalf("package.sh --dry-run -arch %s failed: %v\n%s", arch, err, out)
			}
			text := string(out)
			for _, want := range []string{
				"platform          : windows/" + arch,
				"app_user_model_id : " + core.WindowsAppUserModelID,
				"app_icon          : cmd/sailry-desktop/../../assets/branding/cheaprouter.png",
				"windows_icon      : cmd/sailry-desktop/../../assets/branding/cheaprouter.ico",
			} {
				if !strings.Contains(text, want) {
					t.Errorf("dry-run output missing %q:\n%s", want, text)
				}
			}
		})
	}
}
