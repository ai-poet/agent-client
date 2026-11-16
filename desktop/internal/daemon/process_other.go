//go:build !windows

package daemon

import "os/exec"

func configureDaemonCommand(_ *exec.Cmd) {}
