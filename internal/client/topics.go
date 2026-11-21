package client

import (
	"context"
	"sync"
	"time"

	"github.com/ai-poet/agent-client/internal/link"
	"github.com/ai-poet/agent-client/internal/protocol"
)

func (client *Client) Subscribe(ctx context.Context, topic protocol.Topic) (link.Subscription, error) {
	client.lifecycle.Lock()
	defer client.lifecycle.Unlock()
	if client.closed {
		return nil, fault(protocol.ErrorCodeUnavailable, "Client is closed")
	}
	operation, cancel := context.WithCancel(ctx)
	// Close must interrupt establishment before it waits for lifecycle. Once
	// registered, the topic keeps operation until its own Close cancels it.
	stop := context.AfterFunc(client.closing, cancel)
	defer stop()
	stream, err := client.transport.Subscribe(operation, copyValue(topic))
	if err != nil {
		cancel()
		return nil, err
	}
	value := &topicSubscription{client: client, topic: copyValue(topic), stream: stream, ctx: operation, cancel: cancel, changed: make(chan struct{})}
	client.topicsMu.Lock()
	if client.topics == nil {
		client.topics = map[*topicSubscription]struct{}{}
	}
	client.topics[value] = struct{}{}
	client.topicsMu.Unlock()
	return value, nil
}

type topicSubscription struct {
	client      *Client
	topic       protocol.Topic
	stream      link.Subscription
	ctx         context.Context
	cancel      context.CancelFunc
	once        sync.Once
	err         error
	next        sync.Mutex
	mu          sync.Mutex
	changed     chan struct{}
	generation  uint64
	bound       uint64
	routeCancel context.CancelFunc
}

func (stream *topicSubscription) Next(ctx context.Context) (protocol.Update, error) {
	stream.next.Lock()
	defer stream.next.Unlock()
	operation, cancel := context.WithCancel(ctx)
	defer cancel()
	stop := context.AfterFunc(stream.ctx, cancel)
	defer stop()
	cancelled := func() error {
		if err := stream.ctx.Err(); err != nil {
			return err
		}
		return operation.Err()
	}
	delay := 250 * time.Millisecond
	wake := stream.client.retried()
	for {
		if err := cancelled(); err != nil {
			return protocol.Update{}, err
		}
		stream.mu.Lock()
		current := stream.stream
		stream.mu.Unlock()
		if current != nil {
			wake = stream.client.retried()
			update, err := current.Next(operation)
			if err == nil {
				return update, nil
			}
			if closed := cancelled(); closed != nil {
				return protocol.Update{}, closed
			}
			if stream.topic.Kind == "status" || stream.topic.Kind == "connector_login" || !transient(err) {
				return update, err
			}
			// Retire the broken route even if this read is cancelled during
			// recovery. The next read can start from a detached subscription.
			retired, cancelRoute := stream.detachRoute()
			if cancelRoute != nil {
				cancelRoute()
			}
			if retired != nil {
				_ = retired.Close()
			}
		}

		stream.mu.Lock()
		changed, fresh := stream.changed, stream.generation != stream.bound
		stream.mu.Unlock()
		woken := false
		if !fresh {
			timer := time.NewTimer(delay)
			select {
			case <-operation.Done():
			case <-changed:
			case <-wake:
				woken = true
			case <-timer.C:
			}
			timer.Stop()
		}
		if err := cancelled(); err != nil {
			return protocol.Update{}, err
		}
		if woken {
			delay = 250 * time.Millisecond
		} else {
			delay = min(delay*2, 5*time.Second)
		}

		wake = stream.client.retried()

		stream.mu.Lock()
		generation := stream.generation
		stream.mu.Unlock()
		routeContext, cancelRoute := context.WithCancel(stream.ctx)
		stopRoute := context.AfterFunc(operation, cancelRoute)
		reconnected, err := stream.client.transport.Subscribe(routeContext, copyValue(stream.topic))
		stopRoute()
		stream.mu.Lock()
		stream.bound = generation
		closed := cancelled()
		if err == nil && closed == nil {
			stream.stream, stream.routeCancel = reconnected, cancelRoute
			stream.mu.Unlock()
			continue
		}
		stream.mu.Unlock()
		cancelRoute()
		if err == nil {
			_ = reconnected.Close()
		}
		if closed != nil {
			return protocol.Update{}, closed
		}
		if !transient(err) {
			return protocol.Update{}, err
		}
	}
}

func (stream *topicSubscription) detachRoute() (link.Subscription, context.CancelFunc) {
	stream.mu.Lock()
	defer stream.mu.Unlock()
	current, cancel := stream.stream, stream.routeCancel
	stream.stream, stream.routeCancel = nil, nil
	return current, cancel
}

func (stream *topicSubscription) Close() error {
	stream.once.Do(func() {
		stream.cancel()
		current, cancelRoute := stream.detachRoute()
		if cancelRoute != nil {
			cancelRoute()
		}
		if current != nil {
			stream.err = current.Close()
		}
		stream.client.topicsMu.Lock()
		delete(stream.client.topics, stream)
		stream.client.topicsMu.Unlock()
	})
	return stream.err
}

func (client *Client) resumeTopics() {
	client.topicsMu.Lock()
	defer client.topicsMu.Unlock()
	for stream := range client.topics {
		stream.mu.Lock()
		stream.generation++
		close(stream.changed)
		stream.changed = make(chan struct{})
		stream.mu.Unlock()
	}
}

func (client *Client) closeTopics() {
	client.topicsMu.Lock()
	streams := make([]*topicSubscription, 0, len(client.topics))
	for stream := range client.topics {
		streams = append(streams, stream)
	}
	client.topicsMu.Unlock()
	for _, stream := range streams {
		_ = stream.Close()
	}
}
