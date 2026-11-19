package live

import (
	"context"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/client"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/desktop/internal/link"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

type callbackTestTransport struct {
	conversationTestTransport
	history *conversationTestSubscription
}

func (transport *callbackTestTransport) Subscribe(ctx context.Context, topic protocol.Topic) (link.Subscription, error) {
	if topic.Kind == "conversation" {
		return transport.history, nil
	}
	return transport.conversationTestTransport.Subscribe(ctx, topic)
}

func watchCallbackTranscript(t *testing.T, snapshot protocol.ConversationSnapshot) (*Runtime, *viewmodel.Conversation, *callbackTestTransport) {
	t.Helper()
	session := protocol.DefaultSession()
	session.ID, session.Revision = snapshot.Page.Session, 1
	node := protocol.DefaultSnapshot()
	node.Node, node.Sessions = snapshot.Node, []protocol.Session{session}
	transport := &callbackTestTransport{conversationTestTransport: conversationTestTransport{snapshot: node},
		history: &conversationTestSubscription{updates: make(chan protocol.Update, 1), closed: make(chan struct{})}}
	transport.history.updates <- protocol.Update{Kind: "conversation_snapshot", Data: snapshot}
	controller := client.New(transport)
	t.Cleanup(func() { _ = controller.Close() })
	if err := controller.Start(t.Context()); err != nil {
		t.Fatal(err)
	}
	r := &Runtime{controller: controller, ctx: t.Context(), busy: map[string]bool{}, uncertain: map[string]protocol.Request{}}
	cv := &viewmodel.Conversation{Node: node.Node.String(), ID: session.ID.String()}
	changed := make(chan struct{}, 1)
	release, err := controller.WatchConversation(t.Context(), session.ID, func() {
		r.conversationChanged()
		select {
		case changed <- struct{}{}:
		default:
		}
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(release)
	await := func(sequence uint64) {
		t.Helper()
		ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
		defer cancel()
		for {
			got, ok := controller.Conversation(session.ID)
			if ok && got.Snapshot.Sequence == sequence && got.Connected {
				return
			}
			select {
			case <-changed:
			case <-ctx.Done():
				t.Fatalf("Client did not accept frame %d: %v", sequence, got.Err)
			}
		}
	}
	await(snapshot.Sequence)
	return r, cv, transport
}
