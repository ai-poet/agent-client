//go:build !darwin

package host

import "github.com/ai-poet/agent-client/internal/protocol"

// sampleGPUs lists no accelerator: gopsutil has no GPU sampler, and Linux
// and Windows publish the load only through each vendor's own library or
// tool, which the Node does not load.
func sampleGPUs() []protocol.HostMetricsGpu { return []protocol.HostMetricsGpu{} }
