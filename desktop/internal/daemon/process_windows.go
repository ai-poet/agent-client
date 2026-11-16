//go:build windows

package daemon

import (
	"os/exec"
	"syscall"
)

func configureDaemonCommand(command *exec.Cmd) {
	// The desktop must not open a console window for its daemon child.
	command.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
}
