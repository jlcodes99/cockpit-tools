package helps

import (
	"context"
	"crypto/tls"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/http/httptrace"
	"net/url"
	"os"
	"os/exec"
	"sync"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	sdkconfig "github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

func TestRequestProxyRouteHTTPConnectionReuseAndSnapshot(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { _, _ = w.Write([]byte("ok")) }))
	defer server.Close()
	var mu sync.Mutex
	var observed []net.Conn
	SetRequestProxyRouteObserver(func(proxyURL string, conn net.Conn) func() *usage.ProxyRoute {
		if proxyURL != "" {
			t.Errorf("expected explicit direct route, got %q", proxyURL)
		}
		mu.Lock()
		observed = append(observed, conn)
		mu.Unlock()
		return func() *usage.ProxyRoute { return &usage.ProxyRoute{Kind: "direct"} }
	})
	defer SetRequestProxyRouteObserver(nil)
	client := NewProxyAwareHTTPClient(context.Background(), &config.Config{SDKConfig: sdkconfig.SDKConfig{ProxyURL: "http://unused.invalid:1234"}}, &cliproxyauth.Auth{ProxyURL: "direct"}, 0)
	defer client.CloseIdleConnections()
	for i := 0; i < 2; i++ {
		reporter := NewUsageReporter(context.Background(), "test", "test", nil)
		resp, err := reporter.TrackHTTPClient(client).Get(server.URL)
		if err != nil {
			t.Fatal(err)
		}
		_, _ = io.Copy(io.Discard, resp.Body)
		_ = resp.Body.Close()
		if record := reporter.buildRecord(usage.Detail{}, false); record.ProxyRoute == nil || record.ProxyRoute.Kind != "direct" {
			t.Fatalf("route = %+v", record.ProxyRoute)
		}
	}
	mu.Lock()
	defer mu.Unlock()
	if len(observed) != 2 || observed[0] != observed[1] {
		t.Fatalf("expected trace for both requests on reused connection, got %d", len(observed))
	}
}

func TestRequestProxyRouteHTTPProxySelectionAndUnknownTransport(t *testing.T) {
	request, _ := http.NewRequest(http.MethodGet, "https://upstream.example", nil)
	left, right := net.Pipe()
	defer left.Close()
	defer right.Close()
	proxyURL := "http://localhost:1234"
	transport := buildProxyTransport(proxyURL)
	if got := effectiveHTTPProxyURL(transport, request, &proxyRouteConn{Conn: left, proxyURL: proxyURL}); got != proxyURL {
		t.Fatalf("proxy = %q", got)
	}
	// A custom dialer may hide a SOCKS route and must never be guessed direct.
	transport = &http.Transport{DialContext: (&net.Dialer{}).DialContext}
	if got := effectiveHTTPProxyURL(transport, request, left); got != "unknown" {
		t.Fatalf("custom dialer proxy = %q", got)
	}
	// The connection metadata remains authoritative even below TLS.
	tlsConn := tls.Client(&proxyRouteConn{Conn: left, proxyURL: "socks5://localhost:1234"}, &tls.Config{})
	if got := effectiveHTTPProxyURL(transport, request, tlsConn); got != "socks5://localhost:1234" {
		t.Fatalf("SOCKS proxy = %q", got)
	}
}

type proxyTraceRetryTransport struct {
	conns               []net.Conn
	failAfterConnection bool
}

func (t proxyTraceRetryTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	trace := httptrace.ContextClientTrace(req.Context())
	for _, conn := range t.conns {
		trace.GotConn(httptrace.GotConnInfo{Conn: conn})
	}
	if t.failAfterConnection {
		trace.GetConn("retry.example:443")
		return nil, errors.New("retry dial failed")
	}
	return &http.Response{StatusCode: http.StatusOK, Header: make(http.Header), Body: io.NopCloser(http.NoBody)}, nil
}

func TestRequestProxyRouteLatestConnectionAndImmutableSnapshot(t *testing.T) {
	left, right := net.Pipe()
	defer left.Close()
	defer right.Close()
	route := &usage.ProxyRoute{Kind: "node", Name: "first"}
	SetRequestProxyRouteObserver(func(proxyURL string, conn net.Conn) func() *usage.ProxyRoute {
		if conn == right {
			route.Name = "second"
		}
		return func() *usage.ProxyRoute { return route }
	})
	defer SetRequestProxyRouteObserver(nil)
	reporter := NewUsageReporter(context.Background(), "test", "test", nil)
	client := reporter.TrackHTTPClient(&http.Client{Transport: proxyTraceRetryTransport{conns: []net.Conn{left, right}}})
	resp, err := client.Get("http://example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	record := reporter.buildRecord(usage.Detail{}, false)
	if record.ProxyRoute == nil || record.ProxyRoute.Name != "second" {
		t.Fatalf("latest route = %+v", record.ProxyRoute)
	}
	route.Name = "changed later"
	if record.ProxyRoute.Name != "second" {
		t.Fatalf("published snapshot mutated: %+v", record.ProxyRoute)
	}
	reporter.SetProxyRouteGetter(nil)
	if reporter.buildRecord(usage.Detail{}, false).ProxyRoute != nil {
		t.Fatal("unknown latest connection retained previous route")
	}
}

func TestRequestProxyRouteFailedRetryClearsPreviousConnection(t *testing.T) {
	left, right := net.Pipe()
	defer left.Close()
	defer right.Close()
	SetRequestProxyRouteObserver(func(string, net.Conn) func() *usage.ProxyRoute {
		return func() *usage.ProxyRoute { return &usage.ProxyRoute{Kind: "node", Name: "previous"} }
	})
	defer SetRequestProxyRouteObserver(nil)
	reporter := NewUsageReporter(context.Background(), "test", "test", nil)
	client := reporter.TrackHTTPClient(&http.Client{Transport: proxyTraceRetryTransport{conns: []net.Conn{left}, failAfterConnection: true}})
	if _, err := client.Get("http://example.invalid"); err == nil {
		t.Fatal("expected retry failure")
	}
	if route := reporter.buildRecord(usage.Detail{}, true).ProxyRoute; route != nil {
		t.Fatalf("stale route after failed retry: %+v", route)
	}
}

func TestRequestProxyRouteDoesNotReinvokeCustomProxySelector(t *testing.T) {
	calls := 0
	transport := &http.Transport{Proxy: func(*http.Request) (*url.URL, error) { calls++; return nil, nil }}
	request, _ := http.NewRequest(http.MethodGet, "https://upstream.example", nil)
	if got := effectiveHTTPProxyURL(transport, request, nil); got != "unknown" || calls != 0 {
		t.Fatalf("route=%q selector calls=%d", got, calls)
	}
}

func TestRequestProxyRouteEnvironmentAndNoProxy(t *testing.T) {
	// ProxyFromEnvironment caches its first environment; use an isolated process.
	if os.Getenv("CLIPROXY_ROUTE_ENV_TEST") != "1" {
		command := exec.Command(os.Args[0], "-test.run=^TestRequestProxyRouteEnvironmentAndNoProxy$")
		command.Env = append(os.Environ(), "CLIPROXY_ROUTE_ENV_TEST=1", "HTTP_PROXY=http://proxy.example:1234", "HTTPS_PROXY=http://proxy.example:2345", "NO_PROXY=bypass.example", "REQUEST_METHOD=")
		if output, err := command.CombinedOutput(); err != nil {
			t.Fatalf("environment subprocess: %v\n%s", err, output)
		}
		return
	}
	for _, tc := range []struct{ target, want string }{
		{"http://upstream.example", "http://proxy.example:1234"},
		{"https://upstream.example", "http://proxy.example:2345"},
		{"https://bypass.example", ""},
	} {
		request, _ := http.NewRequest(http.MethodGet, tc.target, nil)
		if got := effectiveHTTPProxyURL(http.DefaultTransport, request, nil); got != tc.want {
			t.Fatalf("target=%s route=%q want=%q", tc.target, got, tc.want)
		}
	}
}
