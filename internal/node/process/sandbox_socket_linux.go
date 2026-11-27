package process

import (
	"encoding/binary"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"golang.org/x/sys/unix"
)

// A read-only root mount still permits connections to pathname Unix sockets.
// Load the filter before the requested program, including for workspace PTYs
// whose launcher cannot pass extra file descriptors to bubblewrap.
func socketFilter(privateHome string) (string, error) {
	arch := map[string]uint32{"amd64": unix.AUDIT_ARCH_X86_64, "arm64": unix.AUDIT_ARCH_AARCH64}[runtime.GOARCH]
	if arch == 0 {
		return "", fmt.Errorf("unsupported socket filter architecture: %s", runtime.GOARCH)
	}
	filter := []unix.SockFilter{
		{Code: unix.BPF_LD | unix.BPF_W | unix.BPF_ABS, K: 4}, // seccomp_data.arch
		{Code: unix.BPF_JMP | unix.BPF_JEQ | unix.BPF_K, Jt: 1, K: arch},
		{Code: unix.BPF_RET | unix.BPF_K, K: unix.SECCOMP_RET_KILL_PROCESS},
		{Code: unix.BPF_LD | unix.BPF_W | unix.BPF_ABS},                        // seccomp_data.nr
		{Code: unix.BPF_JMP | unix.BPF_JGE | unix.BPF_K, Jt: 3, K: 0x40000000}, // x32 ABI
		{Code: unix.BPF_JMP | unix.BPF_JEQ | unix.BPF_K, Jt: 2, K: unix.SYS_SOCKET},
		{Code: unix.BPF_JMP | unix.BPF_JEQ | unix.BPF_K, Jt: 1, K: unix.SYS_SOCKETPAIR},
		{Code: unix.BPF_JMP | unix.BPF_JEQ | unix.BPF_K, Jf: 1, K: unix.SYS_IO_URING_SETUP},
		{Code: unix.BPF_RET | unix.BPF_K, K: unix.SECCOMP_RET_ERRNO | uint32(unix.EPERM)},
		{Code: unix.BPF_RET | unix.BPF_K, K: unix.SECCOMP_RET_ALLOW},
	}
	data, err := binary.Append(nil, binary.NativeEndian, filter)
	if err != nil {
		return "", err
	}
	path := filepath.Join(privateHome, "socket-filter.bpf")
	return path, os.WriteFile(path, data, 0600)
}
