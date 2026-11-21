//go:build !windows

package main

import "github.com/egoist/mygo"

func initWindowsAppIdentity() {}

func applyWindowsAppIcon() {}

func applyWindowsWindowIcon(*mygo.Window) {}
