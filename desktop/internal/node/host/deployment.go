package host

import (
	"context"
	"crypto/sha256"
	"embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"

	"os"
	"path/filepath"
	"reflect"

	"github.com/ai-poet/agent-client/desktop/internal/client"
	"github.com/ai-poet/agent-client/desktop/internal/link"

	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

var ArtifactRoot string

//go:embed deployment/release.sh deployment/service.sh deployment/identity.py
var deploymentSources embed.FS

type artifact struct {
	file   *os.File
	digest string
	name   string
	size   int64
}

func artifactDirectory(target string) (string, error) {
	root := ArtifactRoot
	if root == "" {
		executable, err := os.Executable()
		if err != nil {
			return "", unavailable("Host build artifacts are unavailable")
		}
		root = filepath.Join(filepath.Dir(executable), "host-artifacts")
	}
	return filepath.Join(root, target), nil
}

func openArtifact(root, name string) (*artifact, error) {
	path := filepath.Join(root, name)
	metadata, err := os.Lstat(path)
	if err != nil || !metadata.Mode().IsRegular() || metadata.Size() == 0 {
		return nil, unavailable("locally built Host artifacts are unavailable for this target")
	}
	file, err := os.Open(path)
	if err != nil {
		return nil, unavailable("locally built Host artifact could not be read")
	}
	digest := sha256.New()
	size, err := io.Copy(digest, file)
	if err == nil {
		_, err = file.Seek(0, io.SeekStart)
	}
	if err != nil {
		_ = file.Close()
		return nil, unavailable("locally built Host artifact could not be verified")
	}
	return &artifact{file: file, digest: hex.EncodeToString(digest.Sum(nil)), name: name, size: size}, nil
}

// stageCommand makes the remote staging directory. The installer script
// needs python3, so its absence fails here, before anything is uploaded,
// and names the cause.
const stageCommand = "umask 077; command -v python3 >/dev/null || { echo 'python3 is required' >&2; exit 1; }; mktemp -d /tmp/sailry-bootstrap-XXXXXX"

// releaseIdentity finds the managed program files of an installed release
// Host and asks its bundled Python for the identity of the running Node.
const releaseIdentity = `install_root="$HOME/.local/lib/sailry"
if test "$(uname -s)" = Linux && test "$(id -u)" = 0; then
  if test -e /usr/local/libexec/sailry; then install_root=/usr/local/libexec/sailry
  elif ! test -L /usr/local/bin/sailry || test "$(readlink /usr/local/bin/sailry)" != "$install_root/sailry"; then
    echo 'Managed Host program files are unavailable' >&2; exit 1
  fi
fi
"$install_root/python/bin/python3.12" -I - "$install_root" "$HOME/.sailry-host" <<'SAILRY_IDENTITY'
`

// pairingAttempts is how many times an installation tries to pair with the
// Host it installed.
const pairingAttempts = 5

func ConfigureNetwork(ctx context.Context, endpoint *link.Link, address link.EndpointAddress, id protocol.RequestID, selected protocol.NetworkSelection) error {
	controller := client.New(endpoint.Remote(address))
	defer controller.Close()
	read := func() (protocol.NetworkSettings, error) {
		output, err := controller.Do(ctx, protocol.Command{Kind: "read_network_settings"})
		if err != nil {
			return protocol.NetworkSettings{}, err
		}
		settings, ok := output.Data.(*protocol.NetworkSettings)
		if output.Kind != "network_settings" || !ok {
			return protocol.NetworkSettings{}, invalid("network settings response expected")
		}
		return *settings, nil
	}
	settings, err := read()
	if err != nil {
		return err
	}
	if settings.Revision > 0 {
		if reflect.DeepEqual(settings.Selection, &selected) && reflect.DeepEqual(settings.Effective, &selected) {
			return nil
		}
		return &protocol.Fault{Code: protocol.ErrorCodeRevisionConflict, Message: "installed Host network changed"}
	}
	request := controller.Prepare(protocol.Command{Kind: "save_network_settings", Data: protocol.CommandSaveNetworkSettingsPayload{ExpectedRevision: 0, Selection: selected}})
	request.ID = id
	output, err := controller.Execute(ctx, request)
	if err != nil {
		var fault *protocol.Fault
		if errors.As(err, &fault) && fault.Code == protocol.ErrorCodeOutcomeUnknown {
			_, _ = controller.Outcome(ctx, request)
		}
		return err
	}
	saved, ok := output.Data.(*protocol.NetworkSettings)
	if output.Kind != "network_settings" || !ok {
		return invalid("network settings response expected")
	}
	var actual link.EndpointAddress
	if json.Unmarshal([]byte(saved.Address), &actual) != nil {
		return invalid("invalid Host network address")
	}
	if actual.ID != address.ID {
		return &protocol.Fault{Code: protocol.ErrorCodeWrongTarget, Message: "network settings belong to another Host"}
	}
	if err := endpoint.UpdatePeer(ctx, actual); err != nil {
		return err
	}
	_ = controller.Close()
	controller = client.New(endpoint.Remote(actual))
	defer controller.Close()
	verified, err := read()
	if err != nil {
		return err
	}
	if verified.Revision == 0 || !reflect.DeepEqual(verified.Selection, &selected) || !reflect.DeepEqual(verified.Effective, &selected) {
		return unknown("Host network change could not be confirmed")
	}
	return nil
}
