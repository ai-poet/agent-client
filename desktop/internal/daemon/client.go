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

const handshakeTimeout = 5 * time.Second

// Connection is the part of a daemon connection used by the first-phase UI.
type Connection interface {
	DaemonVersion() string
	Close() error
}

// Client is a connected client-daemon socket. This phase only performs the
// handshake; request routing and sequenced event subscriptions are later work.
type Client struct {
	conn          *websocket.Conn
	daemonVersion string
	closeOnce     sync.Once
	closeErr      error
}

// Connect joins the existing JSON WebSocket protocol at /v1 and waits for the
// daemon's hello response.
func Connect(ctx context.Context, address, token string) (*Client, error) {
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
	hello := helloMessage{
		Type:            "hello",
		ProtocolVersion: ProtocolVersion,
		Token:           token,
		ClientID:        clientID,
		ResumeFrom:      []ReplayCursor{},
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

	return &Client{conn: conn, daemonVersion: response.DaemonVersion}, nil
}

func (c *Client) DaemonVersion() string { return c.daemonVersion }

// Shutdown asks a locally owned daemon to exit, matching the Rust desktop's
// shutdown message. Callers must still wait for or terminate the child.
func (c *Client) Shutdown(ctx context.Context) error {
	if err := wsjson.Write(ctx, c.conn, map[string]string{"type": "shutdown"}); err != nil {
		return fmt.Errorf("could not send client-daemon shutdown: %w", err)
	}
	return nil
}

func (c *Client) Close() error {
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
