package core

import (
	"fmt"
	"strconv"
	"strings"
)

// Product identity constants aligned with the released main (Rust/GPUI) desktop
// package so Sparkle (macOS) and Inno Setup (Windows) replace the existing app
// in place instead of installing a second application beside it.
const (
	AppName                  = "CheapRouter"
	AppSlug                  = "cheaprouter"
	MacOSBundleIdentifier    = "sh.waku"
	MacOSDiskImageIdentifier = "sh.waku.disk-image"
	MacOSBundleName          = "CheapRouter.app"
	MacOSExecutableName      = "CheapRouter"
	MacOSExecutablePath      = "Contents/MacOS/CheapRouter"
	MacOSInstallPath         = "/Applications/CheapRouter.app"
	WindowsInstallDirName    = "CheapRouter"
	WindowsExecutableName    = "CheapRouter.exe"
	WindowsInnoAppID         = "{7DC6C35B-FA40-4A95-B37A-626BF64556C5}"
	WindowsAppUserModelID    = "sh.waku"
	SigningIdentityEnv       = "APPLE_SIGNING_IDENTITY"
	SigningIdentityFallback  = "WAKU_SIGNING_IDENTITY"
	SigningTeamIDEnv         = "APPLE_TEAM_ID"
)

// DerivedMacOSBuildNumber converts a SemVer version ("major.minor.patch[-pre]")
// into the strictly numeric CFBundleVersion used by scripts/release.ts and
// Sparkle (major*1_000_000 + minor*1_000 + patch). For example, "0.3.8" ->
// "3008" and "0.4.0" -> "4000".
func DerivedMacOSBuildNumber(version string) (string, error) {
	coreVersion := strings.TrimSpace(version)
	if cut, _, ok := strings.Cut(coreVersion, "+"); ok {
		coreVersion = cut
	}
	if cut, _, ok := strings.Cut(coreVersion, "-"); ok {
		coreVersion = cut
	}
	parts := strings.Split(coreVersion, ".")
	if len(parts) != 3 {
		return "", fmt.Errorf("invalid semantic version %q", version)
	}
	var numbers [3]int64
	for i, part := range parts {
		if len(part) == 0 || len(part) > 3 {
			return "", fmt.Errorf("invalid version segment %q in %q", part, version)
		}
		value, err := strconv.ParseInt(part, 10, 64)
		if err != nil || value < 0 || value > 999 {
			return "", fmt.Errorf("invalid version segment %q in %q", part, version)
		}
		numbers[i] = value
	}
	return strconv.FormatInt(numbers[0]*1_000_000+numbers[1]*1_000+numbers[2], 10), nil
}
