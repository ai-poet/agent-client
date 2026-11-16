package daemon

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"
)

func TestEndpointFromEnv(t *testing.T) {
	tests := []struct {
		name         string
		address      string
		token        string
		wantExternal bool
		wantError    string
	}{
		{name: "local by default"},
		{name: "external", address: " 127.0.0.1:4312 ", token: "secret", wantExternal: true},
		{name: "missing token", address: "127.0.0.1:4312", wantError: DaemonTokenEnv},
		{name: "missing address", token: "secret", wantError: DaemonAddressEnv},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			env := map[string]string{DaemonAddressEnv: test.address, DaemonTokenEnv: test.token}
			endpoint, external, err := EndpointFromEnv(func(key string) string { return env[key] })
			if test.wantError != "" {
				if err == nil || !strings.Contains(err.Error(), test.wantError) {
					t.Fatalf("error = %v, want %s", err, test.wantError)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if external != test.wantExternal {
				t.Fatalf("external = %v, want %v", external, test.wantExternal)
			}
			if external && (endpoint.Address != "127.0.0.1:4312" || endpoint.Token != "secret") {
				t.Fatalf("endpoint = %+v", endpoint)
			}
		})
	}
}

func TestDaemonExecutable(t *testing.T) {
	getenv := func(key string) string {
		if key == DaemonPathEnv {
			return "/custom/client-daemon"
		}
		return ""
	}
	executable := func() (string, error) { return "/app/CheapRouter", nil }
	path, err := DaemonExecutable(getenv, executable)
	if err != nil || path != "/custom/client-daemon" {
		t.Fatalf("path = %q, err = %v", path, err)
	}
	path, err = DaemonExecutable(func(string) string { return "" }, executable)
	if err != nil || path != filepath.Join("/app", "client-daemon") {
		t.Fatalf("sibling path = %q, err = %v", path, err)
	}
	path, err = DaemonExecutable(func(string) string { return "" }, func() (string, error) {
		return `C:\CheapRouter\CheapRouter.exe`, nil
	})
	if err != nil || !strings.HasSuffix(strings.ToLower(path), "client-daemon.exe") {
		t.Fatalf("Windows path = %q, err = %v", path, err)
	}
}

func TestDesktopClientAddress(t *testing.T) {
	for input, want := range map[string]string{
		"127.0.0.1:4312": "127.0.0.1:4312",
		"0.0.0.0:4312":   "127.0.0.1:4312",
		"[::]:4312":      "[::1]:4312",
		"[::1]:4312":     "[::1]:4312",
	} {
		got, err := desktopClientAddress(input)
		if err != nil || got != want {
			t.Errorf("desktopClientAddress(%q) = %q, %v; want %q", input, got, err, want)
		}
	}
	for _, input := range []string{"", "localhost:4312", "not an address"} {
		if _, err := desktopClientAddress(input); err == nil {
			t.Errorf("desktopClientAddress(%q) unexpectedly succeeded", input)
		}
	}
}

func TestReplaceEnv(t *testing.T) {
	got := replaceEnv(
		[]string{"PATH=/bin", DaemonTokenEnv + "=old", "EMPTY"},
		map[string]string{DaemonTokenEnv: "new"},
	)
	want := []string{"PATH=/bin", "EMPTY", DaemonTokenEnv + "=new"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("environment = %#v, want %#v", got, want)
	}
}

func TestStartLocalUsesExistingProcessContract(t *testing.T) {
	process, err := startLocal(context.Background(), LaunchOptions{
		Executable: os.Args[0],
		Stderr:     os.Stderr,
	}, []string{"-test.run=TestDaemonHelperProcess", "--"})
	if err != nil {
		t.Fatal(err)
	}
	if process.Endpoint.Address != "127.0.0.1:34123" {
		t.Fatalf("address = %q", process.Endpoint.Address)
	}
	if len(process.Endpoint.Token) != 32 {
		t.Fatalf("token length = %d, want 32", len(process.Endpoint.Token))
	}
	process.Stop(context.Background(), nil)
}

func TestStartLocalChecksReadinessProtocol(t *testing.T) {
	t.Setenv("CHEAPROUTER_HELPER_PROTOCOL", "5")
	process, err := startLocal(context.Background(), LaunchOptions{
		Executable: os.Args[0],
		Stderr:     os.Stderr,
	}, []string{"-test.run=TestDaemonHelperProcess", "--"})
	if process != nil {
		process.Stop(context.Background(), nil)
		t.Fatal("startLocal returned a process for an incompatible daemon")
	}
	var protocolErr *ProtocolError
	if !errors.As(err, &protocolErr) || !strings.Contains(err.Error(), "protocol 5") {
		t.Fatalf("error = %v, want protocol mismatch", err)
	}
}

// TestDaemonHelperProcess is executed in a second copy of the test binary by
// StartLocal. It checks the real CLI arguments and environment, then emits the
// same readiness record as client-daemon.
func TestDaemonHelperProcess(t *testing.T) {
	if os.Getenv(DaemonTokenEnv) == "" {
		return
	}
	if os.Getenv(AppExecutableEnv) == "" {
		t.Error("WAKU_APP_EXECUTABLE is missing")
	}
	arguments := strings.Join(os.Args, " ")
	for _, want := range []string{"--bind 127.0.0.1:0", "--parent-pid " + strconv.Itoa(os.Getppid())} {
		if !strings.Contains(arguments, want) {
			t.Errorf("arguments %q do not contain %q", arguments, want)
		}
	}
	protocol := ProtocolVersion
	if value := os.Getenv("CHEAPROUTER_HELPER_PROTOCOL"); value != "" {
		protocol64, err := strconv.ParseUint(value, 10, 32)
		if err != nil {
			t.Fatal(err)
		}
		protocol = int(protocol64)
	}
	fmt.Printf("{\"address\":\"0.0.0.0:34123\",\"protocolVersion\":%d,\"pid\":%d}\n", protocol, os.Getpid())
}
