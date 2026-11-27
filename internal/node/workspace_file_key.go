package node

import "path/filepath"

// workspaceFileKey identifies a queue, not a path authorized for file access.
func workspaceFileKey(root, name string) string {
	if !filepath.IsAbs(name) {
		name = filepath.Join(root, filepath.FromSlash(name))
	}
	return filepath.ToSlash(workspacePathKey(resolvedPathParts(filepath.Clean(name))))
}
