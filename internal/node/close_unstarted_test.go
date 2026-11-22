package node

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/knowledge/service"
)

// A Node that was constructed but never started holds no knowledge service.
// Shutdown used to bind a method value on the nil service and panic in
// Service.Close, so closing an unstarted node crashed the process.
func TestCloseKnowledgeToleratesUnstartedNode(t *testing.T) {
	if err := (&Node{}).closeKnowledge(); err != nil {
		t.Fatalf("unstarted node close: %v", err)
	}
	var missing *Node
	if err := missing.closeKnowledge(); err != nil {
		t.Fatalf("nil node close: %v", err)
	}
}

func TestServiceCloseToleratesNilReceiver(t *testing.T) {
	var service *service.Service
	service.Close()
}
