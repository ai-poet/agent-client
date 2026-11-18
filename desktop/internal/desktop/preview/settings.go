package preview

import (
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"time"
)

// The settings pages' preview data: what a Node and the app would report
// for each section. Credentials are never part of it; pages show them as
// masked placeholders.

// EnvVar is a terminal environment variable.
type EnvVar = viewmodel.EnvVar

type Shell = viewmodel.Shell

// Device is a paired host (Execution) or mobile controller.
type Device = viewmodel.Device

type HookPhase = viewmodel.HookPhase

const (
	BeforeTool = viewmodel.BeforeTool
	AfterTool  = viewmodel.AfterTool
	TurnEnd    = viewmodel.TurnEnd
)

type Hook = viewmodel.Hook

type Permission = viewmodel.Permission

type Archived = viewmodel.Archived

// Settings is the settings pages' dataset.
type Settings struct {
	Version      string
	PreventSleep bool
	Errors       bool // telemetry: error reports
	Metrics      bool // telemetry: usage metrics
	Sandbox      bool
	Env          []EnvVar
	Shells       []Shell
	Devices      []Device
	Relays       []string // the named relay servers
	Hooks        []Hook
	HooksOn      bool
	Python       string // the bundled interpreter's version
	Indexes      []string
	Permissions  []Permission
	Archived     []Archived
	Sessions     int // sessions that are not archived
}

// SettingsData builds the settings' dataset, fresh each call.
func SettingsData() *Settings {
	return &Settings{
		Version: "0.1.0", Errors: true, Metrics: true, Sandbox: true,
		Env:    []EnvVar{{Key: "EDITOR", Value: "nvim"}, {Key: "LANG", Value: "en_US.UTF-8"}},
		Shells: []Shell{{}, {Path: "/bin/zsh"}, {Path: "/bin/bash"}, {Path: "/opt/homebrew/bin/fish"}},
		Devices: []Device{
			{ID: "7f3a9c21", Name: "MacBook Pro", Platform: "macos", Execution: true, Online: true, Local: true, OS: "15.5", Version: "0.1.0"},
			{ID: "c48e0d55", Name: "build-server", Platform: "linux", Execution: true, Online: true, Route: "p2p", RTT: 18, OS: "24.04", Version: "0.1.0"},
			{ID: "a1b2c3d4", Name: "Studio PC", Platform: "windows", Execution: true, Route: ""},
			{ID: "5e6f7a8b", Name: "iPhone", Platform: "ios", Online: true, Route: "relay", RTT: 64, Version: "0.1.0"},
		},
		Relays: []string{"Sailry Asia", "Sailry Europe"},
		Hooks: []Hook{
			{ID: "fmt", Name: "Format", Command: "gofmt -l .", Phase: AfterTool, TimeoutMS: 30000, Enabled: true},
			{ID: "guard", Command: "./scripts/check-paths.sh", Phase: BeforeTool, TimeoutMS: 5000},
			{ID: "notes", Name: "Notes", Command: "git status --short > .notes", Phase: TurnEnd, TimeoutMS: 10000},
		},
		HooksOn: true,
		Python:  "3.12.7",
		Indexes: []string{"pypi.org", "mirrors.aliyun.com", "mirrors.tencent.com", "pypi.tuna.tsinghua.edu.cn", "custom"},
		Permissions: []Permission{
			{Resource: "permission_accessibility", Status: "permission_granted", Settings: true},
			{Resource: "permission_screen", Status: "permission_required", Settings: true},
			{Resource: "permission_microphone", Status: "permission_granted", Settings: true},
			{Resource: "permission_full_disk", Status: "permission_unknown", Settings: true},
			{Resource: "permission_keychain", Status: "permission_not_needed", Settings: false},
		},
		Archived: []Archived{
			{ID: "arch-1", Title: "Refactor the updater", At: Now.Add(-26 * time.Hour)},
			{ID: "arch-2", Title: "Plan the relay probe", At: Now.Add(-9 * 24 * time.Hour)},
		},
		Sessions: 12,
	}
}
