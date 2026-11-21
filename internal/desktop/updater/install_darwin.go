package updater

import (
	"os"
	"path/filepath"
	"strings"

	"github.com/ai-poet/agent-client/internal/core"
)

// installedBundle is the application bundle the running executable belongs to.
func installedBundle() (string, error) {
	executable, err := os.Executable()
	if err != nil {
		return "", ioFailure(err)
	}
	if executable, err = filepath.EvalSymlinks(executable); err != nil {
		return "", ioFailure(err)
	}
	for root := executable; root != filepath.Dir(root); root = filepath.Dir(root) {
		if !strings.HasSuffix(root, ".app") {
			continue
		}
		if filepath.Join(root, filepath.FromSlash(core.MacOSExecutablePath)) != executable {
			return "", failure("updates_bundle_required", "the executable is outside the expected application bundle")
		}
		return root, nil
	}
	return "", failure("updates_bundle_required", "updates require a packaged macOS application")
}
