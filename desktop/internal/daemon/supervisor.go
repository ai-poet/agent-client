package daemon

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Endpoint is the address and bearer capability for one daemon.
type Endpoint struct {
	Address string
	Token   string
}

// EndpointFromEnv follows the existing desktop contract: both variables select
// an already-running daemon; setting only one is an error; neither selects a
// desktop-owned local child.
func EndpointFromEnv(getenv func(string) string) (Endpoint, bool, error) {
	address := strings.TrimSpace(getenv(DaemonAddressEnv))
	token := getenv(DaemonTokenEnv)
	switch {
	case address != "" && token != "":
		return Endpoint{Address: address, Token: token}, true, nil
	case address != "":
		return Endpoint{}, false, fmt.Errorf("%s is set but %s is missing", DaemonAddressEnv, DaemonTokenEnv)
	case token != "":
		return Endpoint{}, false, fmt.Errorf("%s is set but %s is missing", DaemonTokenEnv, DaemonAddressEnv)
	default:
		return Endpoint{}, false, nil
	}
}

// DaemonExecutable resolves WAKU_DAEMON_PATH first, then the sibling binary
// used by the packaged Rust desktop.
func DaemonExecutable(getenv func(string) string, executable func() (string, error)) (string, error) {
	if path := getenv(DaemonPathEnv); path != "" {
		return path, nil
	}
	app, err := executable()
	if err != nil {
		return "", fmt.Errorf("could not locate desktop executable: %w", err)
	}
	name := "client-daemon"
	if strings.HasSuffix(strings.ToLower(app), ".exe") {
		name += ".exe"
	}
	return filepath.Join(filepath.Dir(app), name), nil
}

// Supervisor selects an external daemon or starts a desktop-owned child. Its
// Close method never shuts down an external daemon.
type Supervisor struct {
	getenv     func() map[string]string
	executable func() (string, error)
	process    *LocalProcess
	client     *Client
}

func NewSupervisor() *Supervisor {
	return &Supervisor{
		getenv:     environmentMap,
		executable: os.Executable,
	}
}

// Connect implements shell.Connector. Reconnects reuse a still-running local
// child and pass the last consumed event cursors to the daemon.
func (s *Supervisor) Connect(ctx context.Context, resume []ReplayCursor) (Connection, error) {
	if s.client != nil {
		_ = s.client.Close()
		s.client = nil
	}
	env := s.getenv()
	endpoint, external, err := EndpointFromEnv(func(key string) string { return env[key] })
	if err != nil {
		return nil, err
	}
	if external {
		client, err := ConnectWithResume(ctx, endpoint.Address, endpoint.Token, resume)
		if err != nil {
			return nil, err
		}
		s.client = client
		return client, nil
	}

	if s.process != nil {
		select {
		case <-s.process.done:
			s.process = nil
		default:
			client, err := ConnectWithResume(ctx, s.process.Endpoint.Address, s.process.Endpoint.Token, resume)
			if err != nil {
				return nil, err
			}
			s.client = client
			return client, nil
		}
	}

	path, err := DaemonExecutable(func(key string) string { return env[key] }, s.executable)
	if err != nil {
		return nil, err
	}
	process, err := StartLocal(ctx, LaunchOptions{Executable: path, Stderr: os.Stderr})
	if err != nil {
		return nil, err
	}
	client, err := ConnectWithResume(ctx, process.Endpoint.Address, process.Endpoint.Token, resume)
	if err != nil {
		process.Stop(context.Background(), nil)
		return nil, err
	}
	s.process = process
	s.client = client
	return client, nil
}

func (s *Supervisor) Close() error {
	var err error
	if s.client != nil {
		if s.process != nil {
			s.process.Stop(context.Background(), s.client)
		}
		err = s.client.Close()
		s.client = nil
	} else if s.process != nil {
		s.process.Stop(context.Background(), nil)
	}
	s.process = nil
	return err
}

func environmentMap() map[string]string {
	env := make(map[string]string)
	for _, entry := range os.Environ() {
		key, value, ok := strings.Cut(entry, "=")
		if ok {
			env[key] = value
		}
	}
	return env
}
