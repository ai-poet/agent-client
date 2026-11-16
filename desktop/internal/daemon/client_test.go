package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func TestConnectHandshake(t *testing.T) {
	helloChannel := make(chan helloMessage, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1" {
			t.Errorf("handshake path = %q, want /v1", r.URL.Path)
		}
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			t.Errorf("accept WebSocket: %v", err)
			return
		}
		defer conn.CloseNow()
		var hello helloMessage
		if err := wsjson.Read(r.Context(), conn, &hello); err != nil {
			t.Errorf("read hello: %v", err)
			return
		}
		helloChannel <- hello
		if err := wsjson.Write(r.Context(), conn, map[string]any{
			"type":            "hello",
			"protocolVersion": ProtocolVersion,
			"daemonVersion":   "1.2.3",
		}); err != nil {
			t.Errorf("write hello: %v", err)
		}
	}))
	defer server.Close()

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	client, err := Connect(ctx, strings.TrimPrefix(server.URL, "http://"), "secret")
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	if got := client.DaemonVersion(); got != "1.2.3" {
		t.Fatalf("daemon version = %q, want 1.2.3", got)
	}

	hello := <-helloChannel
	if hello.Type != "hello" || hello.ProtocolVersion != ProtocolVersion || hello.Token != "secret" {
		t.Fatalf("unexpected hello: %+v", hello)
	}
	if hello.ResumeFrom == nil || len(hello.ResumeFrom) != 0 {
		t.Fatalf("resumeFrom = %#v, want an empty JSON array", hello.ResumeFrom)
	}
	if len(hello.ClientID) != 36 || hello.ClientID[14] != '4' {
		t.Fatalf("clientId = %q, want a UUID v4", hello.ClientID)
	}
	wire, err := json.Marshal(hello)
	if err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{"protocolVersion", "clientId", "resumeFrom"} {
		if !strings.Contains(string(wire), `"`+field+`"`) {
			t.Errorf("hello JSON %s is missing %s", wire, field)
		}
	}
}

func TestConnectHandshakeErrors(t *testing.T) {
	tests := []struct {
		name          string
		response      any
		want          string
		protocolError bool
	}{
		{
			name:     "rejected",
			response: map[string]any{"type": "rejected", "message": "bad token"},
			want:     "daemon rejected connection: bad token",
		},
		{
			name: "version mismatch",
			response: map[string]any{
				"type": "hello", "protocolVersion": 5, "daemonVersion": "old",
			},
			want:          "daemon protocol 5 does not match desktop protocol 6",
			protocolError: true,
		},
		{
			name:          "wrong response type",
			response:      map[string]any{"type": "response"},
			want:          `invalid handshake response of type "response"`,
			protocolError: true,
		},
		{
			name: "missing daemon version",
			response: map[string]any{
				"type": "hello", "protocolVersion": 6,
			},
			want:          "daemon hello is missing daemonVersion",
			protocolError: true,
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				conn, err := websocket.Accept(w, r, nil)
				if err != nil {
					return
				}
				defer conn.CloseNow()
				var hello helloMessage
				if wsjson.Read(r.Context(), conn, &hello) == nil {
					_ = wsjson.Write(r.Context(), conn, test.response)
				}
			}))
			defer server.Close()
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			client, err := Connect(ctx, strings.TrimPrefix(server.URL, "http://"), "secret")
			if client != nil {
				_ = client.Close()
			}
			if err == nil || !strings.Contains(err.Error(), test.want) {
				t.Fatalf("error = %v, want text %q", err, test.want)
			}
			var protocolErr *ProtocolError
			if got := errors.As(err, &protocolErr); got != test.protocolError {
				t.Fatalf("errors.As(ProtocolError) = %v, want %v (%v)", got, test.protocolError, err)
			}
		})
	}
}

func TestConnectRejectsMalformedHandshakeJSON(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		var hello helloMessage
		if wsjson.Read(r.Context(), conn, &hello) == nil {
			_ = conn.Write(r.Context(), websocket.MessageText, []byte(`{"type":`))
		}
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, err := Connect(ctx, strings.TrimPrefix(server.URL, "http://"), "secret")
	var protocolErr *ProtocolError
	if !errors.As(err, &protocolErr) || !strings.Contains(err.Error(), "invalid handshake JSON") {
		t.Fatalf("error = %v, want invalid handshake JSON protocol error", err)
	}
}

func TestDaemonURL(t *testing.T) {
	tests := map[string]string{
		"127.0.0.1:4312":                    "ws://127.0.0.1:4312/v1",
		"ws://localhost:4312/ignored?q=1#x": "ws://localhost:4312/v1",
		"wss://waku.example.test/base":      "wss://waku.example.test/v1",
	}
	for input, want := range tests {
		got, err := daemonURL(input)
		if err != nil {
			t.Errorf("daemonURL(%q): %v", input, err)
		} else if got != want {
			t.Errorf("daemonURL(%q) = %q, want %q", input, got, want)
		}
	}
	for _, input := range []string{"", "https://example.test", "ws://"} {
		if _, err := daemonURL(input); err == nil {
			t.Errorf("daemonURL(%q) unexpectedly succeeded", input)
		}
	}
}
