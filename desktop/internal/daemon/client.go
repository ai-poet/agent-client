package daemon

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

const (
	handshakeTimeout   = 5 * time.Second
	defaultRPCTimeout  = 120 * time.Second
	eventChannelBuffer = 512
)

// ErrDisconnected reports that the socket closed before an RPC completed.
var ErrDisconnected = errors.New("client-daemon disconnected")

// Connection is the daemon boundary consumed by the native workbench. All
// methods may block on network I/O and must be called away from the UI thread.
type Connection interface {
	DaemonVersion() string
	LoadTaskState(context.Context) (TaskState, error)
	HydrateSession(context.Context, string) (*AgentSession, error)
	AttachSession(context.Context, string) (*SessionRuntime, error)
	Prompt(context.Context, string, string, string) error
	Events() <-chan SequencedEvent
	ReplayCursors() []ReplayCursor
	Done() <-chan struct{}
	Err() error
	Close() error
}

type requestResult struct {
	payload ResponsePayload
	err     error
}

// Client is one connected client-daemon socket. A background reader routes
// responses, deduplicates sequenced events, and closes Done on disconnect.
type Client struct {
	conn          *websocket.Conn
	daemonVersion string

	writeMu sync.Mutex

	requestMu sync.Mutex
	pending   map[string]chan requestResult

	replay *replayTracker
	events chan SequencedEvent
	done   chan struct{}

	doneOnce  sync.Once
	closeOnce sync.Once
	closeErr  error
	errMu     sync.Mutex
	readErr   error
}

// Connect joins the existing JSON WebSocket protocol at /v1 without replay
// cursors. Use ConnectWithResume when replacing a disconnected socket.
func Connect(ctx context.Context, address, token string) (*Client, error) {
	return ConnectWithResume(ctx, address, token, nil)
}

// ConnectWithResume performs the daemon hello and asks it to replay events
// after each supplied session/runtime/epoch cursor.
func ConnectWithResume(ctx context.Context, address, token string, resume []ReplayCursor) (*Client, error) {
	handshakeContext, cancel := context.WithTimeout(ctx, handshakeTimeout)
	defer cancel()

	endpoint, err := daemonURL(address)
	if err != nil {
		return nil, err
	}
	conn, _, err := websocket.Dial(handshakeContext, endpoint, nil)
	if err != nil {
		return nil, fmt.Errorf("could not connect to client-daemon at %s: %w", endpoint, err)
	}
	conn.SetReadLimit(MaxWireMessageBytes)

	clientID, err := newUUID()
	if err != nil {
		_ = conn.CloseNow()
		return nil, fmt.Errorf("could not create client id: %w", err)
	}
	cursors := append([]ReplayCursor{}, resume...)
	hello := helloMessage{
		Type:            "hello",
		ProtocolVersion: ProtocolVersion,
		Token:           token,
		ClientID:        clientID,
		ResumeFrom:      cursors,
	}
	if err := wsjson.Write(handshakeContext, conn, hello); err != nil {
		_ = conn.CloseNow()
		return nil, fmt.Errorf("could not send client-daemon hello: %w", err)
	}

	var response serverHandshake
	if err := wsjson.Read(handshakeContext, conn, &response); err != nil {
		_ = conn.CloseNow()
		var closeErr websocket.CloseError
		if errors.As(err, &closeErr) {
			return nil, fmt.Errorf("client-daemon closed during handshake: %w", err)
		}
		var syntaxErr *json.SyntaxError
		var typeErr *json.UnmarshalTypeError
		if errors.As(err, &syntaxErr) || errors.As(err, &typeErr) {
			return nil, protocolErrorf("daemon sent invalid handshake JSON: %v", err)
		}
		return nil, fmt.Errorf("could not read client-daemon handshake: %w", err)
	}

	switch response.Type {
	case "hello":
		if response.ProtocolVersion != ProtocolVersion {
			_ = conn.CloseNow()
			return nil, protocolErrorf(
				"daemon protocol %d does not match desktop protocol %d",
				response.ProtocolVersion,
				ProtocolVersion,
			)
		}
		if response.DaemonVersion == "" {
			_ = conn.CloseNow()
			return nil, protocolErrorf("daemon hello is missing daemonVersion")
		}
	case "rejected":
		_ = conn.CloseNow()
		return nil, fmt.Errorf("daemon rejected connection: %s", response.Message)
	default:
		_ = conn.CloseNow()
		return nil, protocolErrorf("daemon sent an invalid handshake response of type %q", response.Type)
	}

	client := &Client{
		conn:          conn,
		daemonVersion: response.DaemonVersion,
		pending:       make(map[string]chan requestResult),
		replay:        newReplayTracker(cursors),
		events:        make(chan SequencedEvent, eventChannelBuffer),
		done:          make(chan struct{}),
	}
	go client.readLoop()
	return client, nil
}

func (c *Client) DaemonVersion() string         { return c.daemonVersion }
func (c *Client) Events() <-chan SequencedEvent { return c.events }
func (c *Client) Done() <-chan struct{}         { return c.done }
func (c *Client) ReplayCursors() []ReplayCursor { return c.replay.cursors() }

// Err returns the terminal read error after Done closes, or ErrDisconnected
// for a locally initiated close.
func (c *Client) Err() error {
	select {
	case <-c.done:
		c.errMu.Lock()
		defer c.errMu.Unlock()
		if c.readErr != nil {
			return c.readErr
		}
		return ErrDisconnected
	default:
		return nil
	}
}

// Request sends one protocol request and routes its response by requestId.
// The default deadline matches the existing TypeScript client's 120 seconds;
// callers may impose a shorter deadline through ctx.
func (c *Client) Request(ctx context.Context, sessionID, runtimeID string, command any) (ResponsePayload, error) {
	requestContext, cancel := context.WithTimeout(ctx, defaultRPCTimeout)
	defer cancel()

	requestID, err := newUUID()
	if err != nil {
		return ResponsePayload{}, fmt.Errorf("could not create request id: %w", err)
	}
	resultChannel := make(chan requestResult, 1)
	c.requestMu.Lock()
	select {
	case <-c.done:
		c.requestMu.Unlock()
		return ResponsePayload{}, c.Err()
	default:
	}
	c.pending[requestID] = resultChannel
	c.requestMu.Unlock()

	message := struct {
		Type      string `json:"type"`
		RequestID string `json:"requestId"`
		SessionID string `json:"sessionId"`
		RuntimeID string `json:"runtimeId"`
		Command   any    `json:"command"`
	}{
		Type:      "request",
		RequestID: requestID,
		SessionID: sessionID,
		RuntimeID: runtimeID,
		Command:   command,
	}
	if err := c.write(requestContext, message); err != nil {
		c.removePending(requestID)
		return ResponsePayload{}, fmt.Errorf("could not send client-daemon request: %w", err)
	}

	select {
	case result := <-resultChannel:
		return result.payload, result.err
	case <-requestContext.Done():
		c.removePending(requestID)
		if errors.Is(requestContext.Err(), context.DeadlineExceeded) {
			return ResponsePayload{}, fmt.Errorf("timed out waiting for client-daemon: %w", requestContext.Err())
		}
		return ResponsePayload{}, requestContext.Err()
	case <-c.done:
		c.removePending(requestID)
		return ResponsePayload{}, c.Err()
	}
}

func (c *Client) removePending(requestID string) {
	c.requestMu.Lock()
	delete(c.pending, requestID)
	c.requestMu.Unlock()
}

func (c *Client) write(ctx context.Context, message any) error {
	c.writeMu.Lock()
	defer c.writeMu.Unlock()
	return wsjson.Write(ctx, c.conn, message)
}

func (c *Client) readLoop() {
	var readErr error
	defer func() {
		if readErr != nil && !errors.Is(readErr, context.Canceled) {
			c.errMu.Lock()
			c.readErr = fmt.Errorf("client-daemon read failed: %w", readErr)
			c.errMu.Unlock()
		}
		c.finish()
		close(c.events)
	}()

	for {
		var envelope struct {
			Type      string          `json:"type"`
			RequestID string          `json:"requestId"`
			Outcome   json.RawMessage `json:"outcome"`

			SessionID string          `json:"sessionId"`
			RuntimeID string          `json:"runtimeId"`
			Epoch     string          `json:"epoch"`
			Sequence  uint64          `json:"sequence"`
			Event     WireDriverEvent `json:"event"`
		}
		if err := wsjson.Read(context.Background(), c.conn, &envelope); err != nil {
			readErr = err
			return
		}

		switch envelope.Type {
		case "response":
			var outcome struct {
				Status  string          `json:"status"`
				Payload json.RawMessage `json:"payload"`
				Error   *RPCError       `json:"error"`
			}
			if err := json.Unmarshal(envelope.Outcome, &outcome); err != nil {
				readErr = protocolErrorf("daemon sent invalid response outcome: %v", err)
				return
			}
			result := requestResult{}
			switch outcome.Status {
			case "ok":
				if err := json.Unmarshal(outcome.Payload, &result.payload); err != nil {
					readErr = protocolErrorf("daemon sent invalid response payload: %v", err)
					return
				}
			case "error":
				if outcome.Error == nil {
					readErr = protocolErrorf("daemon error response is missing error")
					return
				}
				result.err = outcome.Error
			default:
				readErr = protocolErrorf("daemon sent unknown response status %q", outcome.Status)
				return
			}

			c.requestMu.Lock()
			waiter := c.pending[envelope.RequestID]
			delete(c.pending, envelope.RequestID)
			c.requestMu.Unlock()
			if waiter != nil {
				waiter <- result
			}
		case "event":
			event := SequencedEvent{
				SessionID: envelope.SessionID,
				RuntimeID: envelope.RuntimeID,
				Epoch:     envelope.Epoch,
				Sequence:  envelope.Sequence,
				Event:     envelope.Event,
			}
			if !c.replay.advance(event.cursor()) {
				continue
			}
			select {
			case c.events <- event:
			case <-c.done:
				return
			}
		case "shuttingDown":
			_ = c.conn.Close(websocket.StatusNormalClosure, "daemon shutting down")
			return
		case "taskStateChanged":
			// The workbench refreshes on its initial load and session selection.
			// Catalog fan-out can be added without changing sequenced replay.
		}
	}
}

func (c *Client) finish() {
	c.doneOnce.Do(func() {
		c.requestMu.Lock()
		pending := c.pending
		c.pending = make(map[string]chan requestResult)
		c.requestMu.Unlock()
		for _, waiter := range pending {
			waiter <- requestResult{err: ErrDisconnected}
		}
		close(c.done)
	})
}

// Shutdown asks a locally owned daemon to exit, matching the Rust desktop's
// shutdown message. Callers must still wait for or terminate the child.
func (c *Client) Shutdown(ctx context.Context) error {
	if err := c.write(ctx, map[string]string{"type": "shutdown"}); err != nil {
		return fmt.Errorf("could not send client-daemon shutdown: %w", err)
	}
	return nil
}

func (c *Client) Close() error {
	c.finish()
	c.closeOnce.Do(func() {
		c.closeErr = c.conn.Close(websocket.StatusNormalClosure, "desktop closing")
	})
	return c.closeErr
}

func daemonURL(address string) (string, error) {
	address = strings.TrimSpace(address)
	if address == "" {
		return "", fmt.Errorf("client-daemon address is empty")
	}
	normalized := address
	if !strings.HasPrefix(address, "ws://") && !strings.HasPrefix(address, "wss://") {
		if strings.Contains(address, "://") {
			return "", fmt.Errorf("client-daemon address must use ws:// or wss://: %q", address)
		}
		normalized = "ws://" + address
	}
	u, err := url.Parse(normalized)
	if err != nil {
		return "", fmt.Errorf("client-daemon address is invalid: %w", err)
	}
	if (u.Scheme != "ws" && u.Scheme != "wss") || u.Host == "" {
		return "", fmt.Errorf("client-daemon address is invalid: %q", address)
	}
	u.Path = "/v1"
	u.RawPath = ""
	u.RawQuery = ""
	u.Fragment = ""
	return u.String(), nil
}

func newUUID() (string, error) {
	var id [16]byte
	if _, err := rand.Read(id[:]); err != nil {
		return "", err
	}
	id[6] = (id[6] & 0x0f) | 0x40
	id[8] = (id[8] & 0x3f) | 0x80
	var encoded [36]byte
	hex.Encode(encoded[0:8], id[0:4])
	encoded[8] = '-'
	hex.Encode(encoded[9:13], id[4:6])
	encoded[13] = '-'
	hex.Encode(encoded[14:18], id[6:8])
	encoded[18] = '-'
	hex.Encode(encoded[19:23], id[8:10])
	encoded[23] = '-'
	hex.Encode(encoded[24:36], id[10:16])
	return string(encoded[:]), nil
}
