package daemon

import (
	"bufio"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

const (
	daemonStartTimeout    = 15 * time.Second
	daemonShutdownTimeout = time.Second
)

// LaunchOptions configures a desktop-owned daemon child.
type LaunchOptions struct {
	Executable string
	Stderr     io.Writer
}

// LocalProcess owns a child started with the existing client-daemon CLI.
type LocalProcess struct {
	Endpoint Endpoint
	command  *exec.Cmd
	done     chan error
}

type daemonReady struct {
	Address         string `json:"address"`
	ProtocolVersion uint32 `json:"protocolVersion"`
	PID             uint32 `json:"pid"`
}

// StartLocal launches the existing Rust daemon on an ephemeral loopback port,
// passes this process as its parent, and consumes its one-line readiness JSON.
func StartLocal(ctx context.Context, options LaunchOptions) (*LocalProcess, error) {
	return startLocal(ctx, options, nil)
}

func startLocal(ctx context.Context, options LaunchOptions, prefixArgs []string) (*LocalProcess, error) {
	if options.Executable == "" {
		return nil, fmt.Errorf("client-daemon executable is empty; set %s or place client-daemon next to the desktop executable", DaemonPathEnv)
	}
	appExecutable, err := os.Executable()
	if err != nil {
		return nil, fmt.Errorf("could not locate desktop executable: %w", err)
	}
	token, err := newToken()
	if err != nil {
		return nil, fmt.Errorf("could not create daemon token: %w", err)
	}

	args := append([]string{}, prefixArgs...)
	args = append(args, "--bind", "127.0.0.1:0", "--parent-pid", strconv.Itoa(os.Getpid()))
	command := exec.CommandContext(ctx, options.Executable, args...)
	configureDaemonCommand(command)
	command.Env = replaceEnv(os.Environ(), map[string]string{
		DaemonTokenEnv:   token,
		AppExecutableEnv: appExecutable,
	})
	command.Stdin = nil
	command.Stderr = options.Stderr
	stdout, err := command.StdoutPipe()
	if err != nil {
		return nil, fmt.Errorf("could not open client-daemon readiness stream: %w", err)
	}
	if err := command.Start(); err != nil {
		return nil, fmt.Errorf("could not launch %s: %w", options.Executable, err)
	}

	type readyResult struct {
		line string
		err  error
	}
	readyChannel := make(chan readyResult, 1)
	go func() {
		line, err := bufio.NewReader(stdout).ReadString('\n')
		readyChannel <- readyResult{line: line, err: err}
	}()

	timer := time.NewTimer(daemonStartTimeout)
	defer timer.Stop()
	var result readyResult
	select {
	case result = <-readyChannel:
	case <-ctx.Done():
		_ = command.Process.Kill()
		_ = command.Wait()
		return nil, fmt.Errorf("client-daemon startup canceled: %w", ctx.Err())
	case <-timer.C:
		_ = command.Process.Kill()
		_ = command.Wait()
		return nil, fmt.Errorf("timed out waiting for client-daemon readiness")
	}
	if result.err != nil && result.line == "" {
		_ = command.Process.Kill()
		_ = command.Wait()
		return nil, fmt.Errorf("client-daemon exited before becoming ready: %w", result.err)
	}

	var ready daemonReady
	if err := json.Unmarshal([]byte(result.line), &ready); err != nil {
		_ = command.Process.Kill()
		_ = command.Wait()
		return nil, fmt.Errorf("client-daemon returned invalid readiness JSON: %w", err)
	}
	if ready.ProtocolVersion != ProtocolVersion {
		_ = command.Process.Kill()
		_ = command.Wait()
		return nil, protocolErrorf(
			"daemon protocol %d does not match desktop protocol %d",
			ready.ProtocolVersion,
			ProtocolVersion,
		)
	}
	address, err := desktopClientAddress(ready.Address)
	if err != nil {
		_ = command.Process.Kill()
		_ = command.Wait()
		return nil, err
	}

	process := &LocalProcess{
		Endpoint: Endpoint{Address: address, Token: token},
		command:  command,
		done:     make(chan error, 1),
	}
	go func() { process.done <- command.Wait() }()
	return process, nil
}

// Stop asks the child to shut down, then terminates it if it does not exit
// within the same one-second grace period used by the Rust desktop.
func (p *LocalProcess) Stop(ctx context.Context, client *Client) {
	if p == nil {
		return
	}
	if client != nil {
		writeContext, cancel := context.WithTimeout(ctx, daemonShutdownTimeout)
		_ = client.Shutdown(writeContext)
		cancel()
	}
	timer := time.NewTimer(daemonShutdownTimeout)
	defer timer.Stop()
	select {
	case <-p.done:
		return
	case <-timer.C:
	}
	_ = p.command.Process.Kill()
	<-p.done
}

func desktopClientAddress(address string) (string, error) {
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return "", fmt.Errorf("client-daemon returned an invalid address %q: %w", address, err)
	}
	ip := net.ParseIP(host)
	if ip == nil {
		return "", fmt.Errorf("client-daemon returned an invalid address %q", address)
	}
	if ip.IsUnspecified() {
		if ip.To4() != nil {
			host = "127.0.0.1"
		} else {
			host = "::1"
		}
	}
	return net.JoinHostPort(host, port), nil
}

func newToken() (string, error) {
	var token [16]byte
	if _, err := rand.Read(token[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(token[:]), nil
}

func replaceEnv(environment []string, replacements map[string]string) []string {
	result := make([]string, 0, len(environment)+len(replacements))
	for _, entry := range environment {
		key, _, ok := strings.Cut(entry, "=")
		if _, replaced := replacements[key]; !ok || !replaced {
			result = append(result, entry)
		}
	}
	for key, value := range replacements {
		result = append(result, key+"="+value)
	}
	return result
}
