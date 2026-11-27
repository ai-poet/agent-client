package git

import (
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/go-git/go-billy/v5"
	"github.com/go-git/go-billy/v5/osfs"
	gogit "github.com/go-git/go-git/v5"
	"github.com/go-git/go-git/v5/config"
	"github.com/go-git/go-git/v5/plumbing/cache"
	"github.com/go-git/go-git/v5/storage/filesystem"
	"github.com/go-git/go-git/v5/storage/filesystem/dotgit"
)

// UnsupportedExtension is the message of a repository whose configuration
// enables a Git extension that changes what go-git reads.
const UnsupportedExtension = "repository uses an unsupported Git extension"

// readableExtension reports an extension that leaves the objects, references
// and index go-git reads unchanged. go-git v5 refuses every extension except
// its test ones, and compares the lowercase names it reads with a camel-case
// list for format-0 repositories, while native Git opens these repositories:
// sparse checkouts enable worktreeConfig and filtered clones partialClone.
func readableExtension(name, value string) bool {
	switch name {
	case "noop", "noop-v1", "worktreeconfig", "preciousobjects", "partialclone":
		return true
	case "objectformat":
		return value == "sha1"
	case "refstorage":
		return value == "files"
	}
	return false
}

// openExtended opens the repository go-git refused for its extensions, with
// the storage PlainOpen would build for this exact root. The extension check
// reads a view without the extensions section; the repository then uses its
// real storage, so a configuration it writes keeps the extensions.
func openExtended(root string) (*gogit.Repository, error) {
	worktree := osfs.New(root)
	dot, err := dotGitFilesystem(root, worktree)
	if err != nil {
		return nil, mapError(err)
	}
	repository := dot
	common, err := commonDirFilesystem(dot)
	if err != nil {
		return nil, mapError(err)
	}
	if common != nil {
		repository = dotgit.NewRepositoryFilesystem(dot, common)
	}
	storage := filesystem.NewStorage(repository, cache.NewObjectLRUDefault())
	settings, err := storage.Config()
	if err != nil {
		return nil, mapError(err)
	}
	if settings.Raw.HasSection("extensions") {
		for _, option := range settings.Raw.Section("extensions").Options {
			if !readableExtension(strings.ToLower(option.Key), strings.ToLower(option.Value)) {
				return nil, invalid(UnsupportedExtension)
			}
		}
	}
	r, err := gogit.Open(extensionsHidden{storage}, worktree)
	if err != nil {
		return nil, mapError(err)
	}
	r.Storer = storage
	return r, nil
}

// extensionsHidden is the storage go-git checks while opening.
type extensionsHidden struct{ *filesystem.Storage }

func (s extensionsHidden) Config() (*config.Config, error) {
	settings, err := s.Storage.Config()
	if err == nil {
		settings.Raw.RemoveSection("extensions")
	}
	return settings, err
}

func dotGitFilesystem(root string, worktree billy.Filesystem) (billy.Filesystem, error) {
	info, err := worktree.Stat(gogit.GitDirName)
	if err != nil {
		return nil, err
	}
	if info.IsDir() {
		return worktree.Chroot(gogit.GitDirName)
	}
	line, err := smallFile(filepath.Join(root, gogit.GitDirName))
	if err != nil {
		return nil, err
	}
	gitdir, found := strings.CutPrefix(line, "gitdir: ")
	if !found {
		return nil, invalid("Git metadata file has no gitdir")
	}
	gitdir = strings.TrimSpace(strings.Split(gitdir, "\n")[0])
	if !filepath.IsAbs(gitdir) {
		gitdir = filepath.Join(root, gitdir)
	}
	return osfs.New(gitdir), nil
}

func commonDirFilesystem(dot billy.Filesystem) (billy.Filesystem, error) {
	value, err := smallFile(filepath.Join(dot.Root(), "commondir"))
	if errors.Is(err, os.ErrNotExist) || err == nil && strings.TrimSpace(value) == "" {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	path := strings.TrimSpace(value)
	if !filepath.IsAbs(path) {
		path = filepath.Join(dot.Root(), path)
	}
	if _, err := os.Stat(path); err != nil {
		return nil, err
	}
	return osfs.New(path), nil
}

// smallFile reads a Git metadata pointer, which holds one path.
func smallFile(path string) (string, error) {
	file, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer file.Close()
	value, err := io.ReadAll(io.LimitReader(file, 64*1024))
	return string(value), err
}
