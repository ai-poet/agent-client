package cheapauth

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
)

// Only an explicit choice still in the profile's candidate list overrides
// the login endpoint. In particular, a self-hosted login never implicitly
// routes account traffic to the managed primary.
func paymentOrigin(credentialPath, login string) string {
	raw, err := os.ReadFile(filepath.Join(filepath.Dir(credentialPath), "gateway-origin.json"))
	if err != nil {
		return login
	}
	var cfg struct {
		Chosen      string
		Candidates  []string
		ServerNodes []struct{ Origin string } `json:"server_nodes"`
	}
	if json.Unmarshal(raw, &cfg) != nil || cfg.Chosen == "" {
		return login
	}
	nodes := append([]string{PrimaryEndpoint, CDNEndpoint}, cfg.Candidates...)
	for _, node := range cfg.ServerNodes {
		nodes = append(nodes, node.Origin)
	}
	for _, node := range nodes {
		node = strings.TrimSpace(node)
		if node == "" {
			continue
		}
		if !strings.Contains(node, "://") {
			node = "https://" + node
		}
		node = strings.TrimRight(node, "/")
		if strings.HasSuffix(strings.ToLower(node), "/v1") {
			node = node[:len(node)-3]
		}
		if origin, err := NormalizeEndpoint(node); err == nil && origin == cfg.Chosen {
			return origin
		}
	}
	return login
}
