// Package appresources reads the data files shipped outside the executable.
package appresources

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/shipped"
)

// Read loads name from the user's resource directory, falling back to the
// files packaged with the application. The final source-directory fallback
// keeps go run and package tests working without a packaging step.
func Read(name string) ([]byte, error) {
	if name == "" || path.IsAbs(name) || path.Clean(name) != name || strings.HasPrefix(name, "../") {
		return nil, fmt.Errorf("invalid application resource path %q", name)
	}
	for _, directory := range directories() {
		raw, err := os.ReadFile(filepath.Join(directory, filepath.FromSlash(name)))
		if err == nil {
			return raw, nil
		}
		if !errors.Is(err, fs.ErrNotExist) {
			return nil, err
		}
	}
	return nil, fmt.Errorf("application resource %s: %w", name, fs.ErrNotExist)
}

func directories() []string {
	result := []string{}
	if directory := os.Getenv("SAILRY_RESOURCES_DIR"); directory != "" {
		result = append(result, directory)
	}
	if directory, err := os.UserConfigDir(); err == nil {
		result = append(result, filepath.Join(directory, "Sailry", "resources"))
	}
	if directory, err := shipped.Directory(); err == nil {
		result = append(result, directory)
	}
	_, source, _, ok := runtime.Caller(0)
	if ok {
		result = append(result, filepath.Clean(filepath.Join(filepath.Dir(source), "..", "..", "cmd", "sailry-desktop", "resources")))
	}
	return result
}
