package live

import (
	"context"

	"github.com/ai-poet/cheaprouter-harness/internal/client"
	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func (r *Runtime) confirmConnectionNetwork(ctx context.Context, node protocol.NodeID, selection protocol.NetworkSelection) error {
	if r.network == nil || r.network.endpoint == nil {
		return unknownConnectionNetwork()
	}
	endpoint := r.network.endpoint
	peers, err := endpoint.Peers(ctx)
	if err != nil {
		return unknownConnectionNetwork()
	}
	for _, previous := range peers {
		if previous.ID != node {
			continue
		}
		controller := client.New(endpoint.Remote(previous))
		defer controller.Close()
		actual, err := readConnectionNetwork(ctx, controller)
		if err != nil {
			candidate, addressErr := link.CandidateAddress(previous, selection)
			if addressErr != nil {
				return unknownConnectionNetwork()
			}
			if err := endpoint.UpdatePeer(ctx, candidate); err != nil {
				return unknownConnectionNetwork()
			}
			actual, err = readConnectionNetwork(ctx, controller)
		}
		if err != nil || actual.Revision == 0 || !sameConnectionNetwork(actual.Selection, &selection) || !sameConnectionNetwork(actual.Effective, &selection) {
			return unknownConnectionNetwork()
		}
		address, err := connectionNetworkAddress(actual, node)
		if err != nil {
			return unknownConnectionNetwork()
		}
		return endpoint.UpdatePeer(ctx, address)
	}
	return unknownConnectionNetwork()
}
