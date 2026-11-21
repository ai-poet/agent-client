package cheapauth

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"
)

// LoginFlow owns one PKCE attempt and, when available, its ephemeral loopback
// listener. The same verifier redeems browser-delivered and pasted codes.
type LoginFlow struct {
	client   *Client
	pkce     PKCE
	listener net.Listener
	server   *http.Server
	result   chan Credentials
	serveErr chan error
	done     chan struct{}
	// delivery admits one relay request at a time, until one is redeemed.
	delivery  sync.Mutex
	delivered bool
	close     sync.Once
}

// StartManualLogin creates a paste-only attempt without touching the network.
func StartManualLogin(client *Client) (*LoginFlow, error) {
	pkce, err := GeneratePKCE()
	if err != nil {
		return nil, err
	}
	return &LoginFlow{client: client, pkce: pkce, result: make(chan Credentials, 1), serveErr: make(chan error, 1), done: make(chan struct{})}, nil
}

// StartLogin falls back to paste-only mode when loopback cannot be bound.
func StartLogin(ctx context.Context, client *Client) (*LoginFlow, error) {
	flow, err := StartManualLogin(client)
	if err != nil {
		return nil, err
	}
	listener, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		return flow, nil
	}
	flow.listener = listener
	flow.server = &http.Server{
		BaseContext: func(net.Listener) context.Context { return ctx },
		Handler:     http.HandlerFunc(flow.serveHTTP), MaxHeaderBytes: 16 * 1024,
		ReadHeaderTimeout: 10 * time.Second, ReadTimeout: 15 * time.Second, WriteTimeout: 30 * time.Second,
	}
	go func() {
		err := flow.server.Serve(listener)
		if err != nil && err != http.ErrServerClosed {
			flow.serveErr <- err
		}
	}()
	return flow, nil
}

func (f *LoginFlow) PKCE() PKCE       { return f.pkce }
func (f *LoginFlow) Loopback() bool   { return f.listener != nil }
func (f *LoginFlow) Endpoint() string { return f.client.Endpoint() }
func (f *LoginFlow) Port() int {
	if f.listener == nil {
		return 0
	}
	if address, ok := f.listener.Addr().(*net.TCPAddr); ok {
		return address.Port
	}
	return 0
}

func (f *LoginFlow) LoginURL() string {
	if f.listener == nil {
		return ManualLoginURL(f.client.Endpoint(), f.pkce.Challenge())
	}
	return LoginURL(f.client.Endpoint(), f.Port(), f.pkce.Challenge())
}

// Wait waits for the relay page's local POST to be redeemed. Paste-only flows
// are completed by ExchangePasted instead.
func (f *LoginFlow) Wait(ctx context.Context) (Credentials, error) {
	if f.listener == nil {
		<-ctx.Done()
		return Credentials{}, ctx.Err()
	}
	select {
	case <-ctx.Done():
		return Credentials{}, ctx.Err()
	case <-f.done:
		select {
		case credentials := <-f.result:
			return credentials, nil
		default:
			return Credentials{}, context.Canceled
		}
	case <-f.serveErr:
		return Credentials{}, &AuthError{Kind: ErrorNetwork, Op: "loopback"}
	case credentials := <-f.result:
		return credentials, nil
	}
}

// ExchangePasted redeems a code copied from the bridge, including the
// no-loopback flow. Both exchange paths return candidates; only the owner of
// the current attempt can establish an account.
func (f *LoginFlow) ExchangePasted(ctx context.Context, pasted string) (Credentials, error) {
	code, legacy, err := ParsePastedLogin(pasted, f.client.Endpoint())
	if err != nil {
		return Credentials{}, err
	}
	if legacy != nil {
		return *legacy, nil
	}
	return f.client.Exchange(ctx, code, f.pkce.Verifier())
}

func (f *LoginFlow) serveHTTP(writer http.ResponseWriter, request *http.Request) {
	switch {
	case request.Method == http.MethodGet && request.URL.Path == "/callback":
		writer.Header().Set("Content-Type", "text/html; charset=utf-8")
		writer.Header().Set("Cache-Control", "no-store")
		writer.Header().Set("Content-Security-Policy", "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'")
		_, _ = writer.Write([]byte(relayPage))
	case request.Method == http.MethodPost && request.URL.Path == "/deliver":
		if origin := strings.TrimRight(strings.TrimSpace(request.Header.Get("Origin")), "/"); origin != "" &&
			origin != "http://127.0.0.1:"+strconv.Itoa(f.Port()) && origin != "http://localhost:"+strconv.Itoa(f.Port()) {
			http.Error(writer, "禁止跨站提交", http.StatusForbidden)
			return
		}
		if request.ContentLength < 0 {
			http.Error(writer, "回跳内容无效", http.StatusBadRequest)
			return
		}
		if request.ContentLength > 64*1024 {
			http.Error(writer, "回跳内容过大", http.StatusRequestEntityTooLarge)
			return
		}
		request.Body = http.MaxBytesReader(writer, request.Body, 64*1024)
		raw := make([]byte, request.ContentLength)
		if _, err := io.ReadFull(request.Body, raw); err != nil {
			http.Error(writer, "回跳内容无效", http.StatusBadRequest)
			return
		}
		f.delivery.Lock()
		defer f.delivery.Unlock()
		if f.delivered {
			http.Error(writer, "登录回跳已处理", http.StatusConflict)
			return
		}
		code, legacy, err := parseDelivery(string(raw), f.client.Endpoint(), f.client.now())
		var credentials Credentials
		if err == nil {
			if legacy != nil {
				credentials = *legacy
			} else {
				credentials, err = f.client.Exchange(request.Context(), code, f.pkce.Verifier())
			}
		}
		if err != nil {
			// A delivery that is not redeemed is that request's problem: a
			// stale tab or a probe must not cost the sign-in its listener.
			http.Error(writer, "登录码无效、已过期或与本次登录不匹配，请返回应用重试或粘贴登录码", http.StatusBadRequest)
			return
		}
		f.delivered = true
		writer.Header().Set("Cache-Control", "no-store")
		writer.WriteHeader(http.StatusOK)
		_, _ = writer.Write([]byte("ok"))
		// The owner may close the listener as soon as it gets the result.
		_ = http.NewResponseController(writer).Flush()
		f.result <- credentials
	default:
		http.NotFound(writer, request)
	}
}

func (f *LoginFlow) Close() {
	f.close.Do(func() {
		close(f.done)
		if f.server != nil {
			_ = f.server.Close()
		} else if f.listener != nil {
			_ = f.listener.Close()
		}
	})
}

const relayPage = `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>正在完成 CheapRouter 登录…</title>
<style>body{font:15px/1.6 system-ui,-apple-system,sans-serif;margin:0;display:flex;align-items:center;justify-content:center;height:100vh;background:#101216;color:#e8eaee}main{text-align:center;max-width:32rem;padding:2rem}h1{font-size:1.15rem}p{color:#aab0bb}</style></head>
<body><main><h1 id="title">正在完成登录…</h1><p id="detail">请稍候，随后即可返回应用。</p></main>
<script>
(function(){
  var title=document.getElementById('title'),detail=document.getElementById('detail');
  var hash=window.location.hash||'';
  if(!hash||hash.length<2){title.textContent='登录未完成';detail.textContent='浏览器没有返回登录码，请返回应用重试。';return;}
  fetch('/deliver',{method:'POST',body:hash}).then(function(response){
    if(response.ok){title.textContent='登录成功';detail.textContent='现在可以关闭此页并返回应用。';return;}
    title.textContent='自动回传失败';detail.textContent='请复制登录页上的一次性登录码，粘贴到应用的登录窗口。';
  }).catch(function(){title.textContent='无法连接应用';detail.textContent='请复制登录页上的一次性登录码，粘贴到应用的登录窗口。';});
})();
</script></body></html>`

func (f *LoginFlow) String() string {
	return fmt.Sprintf("CheapRouter 登录流程（loopback=%t）", f.Loopback())
}
