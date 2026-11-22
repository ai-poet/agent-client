//go:build windows

package main

import (
	"log"
	"unsafe"

	"github.com/ai-poet/cheaprouter-harness/assets/branding"
	"github.com/ai-poet/cheaprouter-harness/internal/core"
	"github.com/egoist/mygo"
	"golang.org/x/sys/windows"
)

var setCurrentProcessExplicitAppUserModelID = windows.NewLazySystemDLL("shell32.dll").NewProc("SetCurrentProcessExplicitAppUserModelID")

// initWindowsAppIdentity binds the process to CheapRouter's stable
// AppUserModelID before any window is created so Windows groups running
// windows with the Inno Setup shortcuts and pinned taskbar icon.
func initWindowsAppIdentity() {
	if err := setCurrentProcessExplicitAppUserModelID.Find(); err != nil {
		log.Printf("windows AppUserModelID: %v", err)
		return
	}
	id, err := windows.UTF16PtrFromString(core.WindowsAppUserModelID)
	if err != nil {
		log.Printf("windows AppUserModelID: %v", err)
		return
	}
	hr, _, _ := setCurrentProcessExplicitAppUserModelID.Call(uintptr(unsafe.Pointer(id)))
	if int32(hr) < 0 {
		log.Printf("SetCurrentProcessExplicitAppUserModelID(%q): HRESULT 0x%08x", core.WindowsAppUserModelID, uint32(hr))
	}
}

// MyGo's Windows backend uses the Dock API for its default window icon.
func applyWindowsAppIcon() {
	if err := mygo.App.Dock.SetIcon(branding.AppIconPNG); err != nil {
		log.Printf("windows app icon: %v", err)
	}
}

func applyWindowsWindowIcon(window *mygo.Window) {
	if err := window.SetIcon(branding.AppIconPNG); err != nil {
		log.Printf("windows window icon: %v", err)
	}
}
