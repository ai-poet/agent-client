package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

const fakeDaemonTimeout = 2 * time.Second

type fakeWireRequest struct {
	Type      string          `json:"type"`
	RequestID string          `json:"requestId"`
	SessionID string          `json:"sessionId"`
	RuntimeID string          `json:"runtimeId"`
	Command   json.RawMessage `json:"command"`
}

type fakeWirePeer struct {
	conn     *websocket.Conn
	hello    helloMessage
	requests chan fakeWireRequest
}

func newFakeDaemon(t *testing.T) (string, <-chan *fakeWirePeer) {
	t.Helper()
	peers := make(chan *fakeWirePeer, 4)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			t.Errorf("accept fake daemon socket: %v", err)
			return
		}
		defer conn.CloseNow()

		var hello helloMessage
		if err := wsjson.Read(r.Context(), conn, &hello); err != nil {
			t.Errorf("read fake daemon hello: %v", err)
			return
		}
		if err := wsjson.Write(r.Context(), conn, map[string]any{
			"type":            "hello",
			"protocolVersion": ProtocolVersion,
			"daemonVersion":   "test-daemon",
		}); err != nil {
			t.Errorf("write fake daemon hello: %v", err)
			return
		}
		peer := &fakeWirePeer{conn: conn, hello: hello, requests: make(chan fakeWireRequest, 16)}
		peers <- peer
		for {
			var request fakeWireRequest
			if err := wsjson.Read(r.Context(), conn, &request); err != nil {
				return
			}
			peer.requests <- request
		}
	}))
	t.Cleanup(server.Close)
	return strings.TrimPrefix(server.URL, "http://"), peers
}

func connectFakeDaemon(t *testing.T, address string, peers <-chan *fakeWirePeer) (*Client, *fakeWirePeer) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), fakeDaemonTimeout)
	defer cancel()
	client, err := Connect(ctx, address, "secret")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = client.Close() })
	return client, awaitFakePeer(t, peers)
}

func awaitFakePeer(t *testing.T, peers <-chan *fakeWirePeer) *fakeWirePeer {
	t.Helper()
	select {
	case peer := <-peers:
		return peer
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("timed out waiting for fake daemon connection")
		return nil
	}
}

func awaitFakeRequest(t *testing.T, peer *fakeWirePeer) fakeWireRequest {
	t.Helper()
	select {
	case request := <-peer.requests:
		return request
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("timed out waiting for client request")
		return fakeWireRequest{}
	}
}

func writeFakeOK(t *testing.T, peer *fakeWirePeer, requestID string, payload any) {
	t.Helper()
	if err := wsjson.Write(context.Background(), peer.conn, map[string]any{
		"type":      "response",
		"requestId": requestID,
		"outcome":   map[string]any{"status": "ok", "payload": payload},
	}); err != nil {
		t.Fatalf("write fake daemon response: %v", err)
	}
}

func writeFakeEvent(t *testing.T, peer *fakeWirePeer, event SequencedEvent) {
	t.Helper()
	if err := wsjson.Write(context.Background(), peer.conn, map[string]any{
		"type":      "event",
		"sessionId": event.SessionID,
		"runtimeId": event.RuntimeID,
		"epoch":     event.Epoch,
		"sequence":  event.Sequence,
		"event":     event.Event,
	}); err != nil {
		t.Fatalf("write fake daemon event: %v", err)
	}
}

func awaitClientEvent(t *testing.T, client *Client) SequencedEvent {
	t.Helper()
	select {
	case event, ok := <-client.Events():
		if !ok {
			t.Fatal("event channel closed before the expected event")
		}
		return event
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("timed out waiting for client event")
		return SequencedEvent{}
	}
}

func decodeFakeCommand(t *testing.T, request fakeWireRequest) map[string]any {
	t.Helper()
	var command map[string]any
	if err := json.Unmarshal(request.Command, &command); err != nil {
		t.Fatalf("decode request command: %v", err)
	}
	return command
}

func TestRequestSuccessAndRPCError(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)

	type requestAnswer struct {
		payload ResponsePayload
		err     error
	}
	answer := make(chan requestAnswer, 1)
	go func() {
		payload, err := client.Request(context.Background(), "session-a", "runtime-a", map[string]any{
			"type": "prompt", "prompt": "你好",
		})
		answer <- requestAnswer{payload: payload, err: err}
	}()
	request := awaitFakeRequest(t, peer)
	if request.Type != "request" || request.RequestID == "" || request.SessionID != "session-a" || request.RuntimeID != "runtime-a" {
		t.Fatalf("unexpected request envelope: %+v", request)
	}
	command := decodeFakeCommand(t, request)
	if command["type"] != "prompt" || command["prompt"] != "你好" {
		t.Fatalf("unexpected command: %#v", command)
	}
	writeFakeOK(t, peer, request.RequestID, map[string]any{"type": "ack"})
	select {
	case got := <-answer:
		if got.err != nil || got.payload.Type != "ack" {
			t.Fatalf("successful request = %+v, %v", got.payload, got.err)
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("successful RPC did not complete")
	}

	go func() {
		_, err := client.Request(context.Background(), NilUUID, NilUUID, map[string]any{"type": "loadTaskState"})
		answer <- requestAnswer{err: err}
	}()
	request = awaitFakeRequest(t, peer)
	if err := wsjson.Write(context.Background(), peer.conn, map[string]any{
		"type":      "response",
		"requestId": request.RequestID,
		"outcome": map[string]any{
			"status": "error",
			"error":  map[string]any{"message": "任务状态不可用"},
		},
	}); err != nil {
		t.Fatal(err)
	}
	select {
	case got := <-answer:
		var rpcErr *RPCError
		if !errors.As(got.err, &rpcErr) || rpcErr.Message != "任务状态不可用" {
			t.Fatalf("RPC error = %#v, want RPCError", got.err)
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("error RPC did not complete")
	}
}

func TestRequestTimeout(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Millisecond)
	defer cancel()
	answer := make(chan error, 1)
	go func() {
		_, err := client.Request(ctx, NilUUID, NilUUID, map[string]any{"type": "loadTaskState"})
		answer <- err
	}()
	_ = awaitFakeRequest(t, peer)
	select {
	case err := <-answer:
		if !errors.Is(err, context.DeadlineExceeded) || !strings.Contains(err.Error(), "timed out") {
			t.Fatalf("timeout error = %v", err)
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("RPC did not honor its context deadline")
	}
}

func TestConcurrentResponsesRouteByRequestID(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)
	const count = 8
	type answer struct {
		index int
		cwd   string
		err   error
	}
	answers := make(chan answer, count)
	for index := 0; index < count; index++ {
		go func() {
			payload, err := client.Request(context.Background(), NilUUID, NilUUID, map[string]any{
				"type": "testRequest", "index": index,
			})
			answers <- answer{index: index, cwd: payload.DefaultCwd, err: err}
		}()
	}

	requests := make([]fakeWireRequest, count)
	for index := range requests {
		requests[index] = awaitFakeRequest(t, peer)
	}
	for index := len(requests) - 1; index >= 0; index-- {
		request := requests[index]
		command := decodeFakeCommand(t, request)
		writeFakeOK(t, peer, request.RequestID, map[string]any{
			"type": "taskState", "defaultCwd": fmt.Sprintf("cwd-%.0f", command["index"]),
		})
	}
	seen := make(map[int]string, count)
	for range count {
		select {
		case got := <-answers:
			if got.err != nil {
				t.Fatalf("request %d failed: %v", got.index, got.err)
			}
			seen[got.index] = got.cwd
		case <-time.After(fakeDaemonTimeout):
			t.Fatal("concurrent RPC did not complete")
		}
	}
	for index := 0; index < count; index++ {
		if want := fmt.Sprintf("cwd-%d", index); seen[index] != want {
			t.Errorf("request %d received payload %q, want %q", index, seen[index], want)
		}
	}
}

func TestDisconnectFailsPendingRequest(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)
	answer := make(chan error, 1)
	go func() {
		_, err := client.Request(context.Background(), NilUUID, NilUUID, map[string]any{"type": "loadTaskState"})
		answer <- err
	}()
	_ = awaitFakeRequest(t, peer)
	_ = peer.conn.CloseNow()
	select {
	case err := <-answer:
		if err == nil {
			t.Fatal("pending RPC succeeded after disconnect")
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("pending RPC remained blocked after disconnect")
	}
	select {
	case <-client.Done():
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("Done was not closed after disconnect")
	}
}

func TestEventSubscriptionDeduplicatesExactStream(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)
	base := SequencedEvent{
		SessionID: "session-a", RuntimeID: "runtime-a", Epoch: "epoch-a", Sequence: 1,
		Event: WireDriverEvent{Kind: "textDelta", Payload: json.RawMessage(`"一"`)},
	}
	otherEpoch := base
	otherEpoch.Epoch = "epoch-b"
	otherEpoch.Event.Payload = json.RawMessage(`"另一纪元"`)
	newer := base
	newer.Sequence = 2
	newer.Event.Payload = json.RawMessage(`"二"`)

	writeFakeEvent(t, peer, base)
	duplicate := base
	duplicate.Event.Payload = json.RawMessage(`"重复不应投递"`)
	writeFakeEvent(t, peer, duplicate)
	older := base
	older.Sequence = 0
	writeFakeEvent(t, peer, older)
	writeFakeEvent(t, peer, otherEpoch)
	writeFakeEvent(t, peer, newer)

	for _, want := range []SequencedEvent{base, otherEpoch, newer} {
		got := awaitClientEvent(t, client)
		if got.SessionID != want.SessionID || got.RuntimeID != want.RuntimeID || got.Epoch != want.Epoch || got.Sequence != want.Sequence || got.Event.Kind != want.Event.Kind || string(got.Event.Payload) != string(want.Event.Payload) {
			t.Fatalf("event = %+v, want %+v", got, want)
		}
	}
	wantCursors := []ReplayCursor{
		{SessionID: "session-a", RuntimeID: "runtime-a", Epoch: "epoch-a", Sequence: 2},
		{SessionID: "session-a", RuntimeID: "runtime-a", Epoch: "epoch-b", Sequence: 1},
	}
	if got := client.ReplayCursors(); !reflect.DeepEqual(got, wantCursors) {
		t.Fatalf("replay cursors = %#v, want %#v", got, wantCursors)
	}
}

func TestReconnectSendsResumeFromAndSkipsReplay(t *testing.T) {
	address, peers := newFakeDaemon(t)
	first, firstPeer := connectFakeDaemon(t, address, peers)
	event := SequencedEvent{
		SessionID: "session-a", RuntimeID: "runtime-a", Epoch: "epoch-a", Sequence: 7,
		Event: WireDriverEvent{Kind: "turnStarted", Payload: json.RawMessage("null")},
	}
	writeFakeEvent(t, firstPeer, event)
	_ = awaitClientEvent(t, first)
	cursors := first.ReplayCursors()
	_ = firstPeer.conn.CloseNow()
	select {
	case <-first.Done():
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("first connection did not disconnect")
	}

	ctx, cancel := context.WithTimeout(context.Background(), fakeDaemonTimeout)
	defer cancel()
	second, err := ConnectWithResume(ctx, address, "secret", cursors)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = second.Close() })
	secondPeer := awaitFakePeer(t, peers)
	if !reflect.DeepEqual(secondPeer.hello.ResumeFrom, cursors) {
		t.Fatalf("resumeFrom = %#v, want %#v", secondPeer.hello.ResumeFrom, cursors)
	}
	writeFakeEvent(t, secondPeer, event)
	newer := event
	newer.Sequence = 8
	newer.Event = WireDriverEvent{Kind: "textDelta", Payload: json.RawMessage(`"继续"`)}
	writeFakeEvent(t, secondPeer, newer)
	if got := awaitClientEvent(t, second); got.Sequence != 8 || got.Event.Kind != "textDelta" {
		t.Fatalf("reconnected event = %+v, want only sequence 8", got)
	}
}
