package helps

import (
	"context"
	"crypto/x509"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"strings"
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

func TestUtlsRequestProxyRouteActualHTTP2Connection(t *testing.T) {
	// Root certificate state is process-global, so isolate the local TLS fixture.
	if os.Getenv("CLIPROXY_UTLS_ROUTE_CHILD") != "1" {
		command := exec.Command(os.Args[0], "-test.run=^TestUtlsRequestProxyRouteActualHTTP2Connection$")
		command.Env = append(os.Environ(), "CLIPROXY_UTLS_ROUTE_CHILD=1", "GODEBUG=x509usefallbackroots=1")
		if output, err := command.CombinedOutput(); err != nil {
			t.Fatalf("TLS route subprocess: %v\n%s", err, output)
		}
		return
	}
	upstream := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.ProtoMajor != 2 {
			t.Errorf("protocol = %s, want HTTP/2", r.Proto)
		}
		_, _ = w.Write([]byte("ok"))
	}))
	upstream.EnableHTTP2 = true
	upstream.StartTLS()
	defer upstream.Close()
	roots := x509.NewCertPool()
	roots.AddCert(upstream.Certificate())
	x509.SetFallbackRoots(roots)
	proxyServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodConnect {
			t.Errorf("proxy method = %s", r.Method)
			http.Error(w, "CONNECT required", 400)
			return
		}
		target, err := net.Dial("tcp", upstream.Listener.Addr().String())
		if err != nil {
			t.Error(err)
			http.Error(w, "dial failed", 502)
			return
		}
		client, _, err := w.(http.Hijacker).Hijack()
		if err != nil {
			target.Close()
			t.Error(err)
			return
		}
		defer client.Close()
		defer target.Close()
		_, _ = io.WriteString(client, "HTTP/1.1 200 Connection Established\r\n\r\n")
		done := make(chan struct{})
		go func() { _, _ = io.Copy(target, client); target.Close(); close(done) }()
		_, _ = io.Copy(client, target)
		client.Close()
		<-done
	}))
	defer proxyServer.Close()
	for _, test := range []struct {
		proxyURL  string
		streaming bool
	}{
		{"direct", false}, {"direct", true}, {proxyServer.URL, false}, {proxyServer.URL, true},
	} {
		proxyURL := test.proxyURL
		t.Run(fmt.Sprintf("%s/streaming=%v", proxyURL, test.streaming), func(t *testing.T) {
			calls := 0
			SetRequestProxyRouteObserver(func(actual string, conn net.Conn) func() *usage.ProxyRoute {
				calls++
				want := proxyURL
				if want == "direct" {
					want = ""
				}
				if actual != want {
					t.Errorf("actual proxy = %q, want %q", actual, want)
				}
				remote := upstream.Listener.Addr().String()
				if proxyURL != "direct" {
					remote = proxyServer.Listener.Addr().String()
				}
				if conn.RemoteAddr().String() != remote {
					t.Errorf("actual socket remote = %s, want %s", conn.RemoteAddr(), remote)
				}
				return func() *usage.ProxyRoute { return &usage.ProxyRoute{Kind: "node", Name: "frozen actual route"} }
			})
			defer SetRequestProxyRouteObserver(nil)
			reporter := NewUsageReporter(t.Context(), "codex", "test", nil)
			client := &http.Client{Transport: newUtlsRoundTripper(proxyURL)}
			if test.streaming {
				client = reporter.TrackHTTPClientRoundTripOnly(client)
			} else {
				client = reporter.TrackHTTPClient(client)
			}
			response, err := client.Get(upstream.URL)
			if err != nil {
				t.Fatal(err)
			}
			_, _ = io.Copy(io.Discard, response.Body)
			response.Body.Close()
			record := reporter.buildRecord(usage.Detail{}, false)
			if calls != 1 || record.ProxyRoute == nil || record.ProxyRoute.Name != "frozen actual route" {
				t.Fatalf("observers = %d, route = %+v", calls, record.ProxyRoute)
			}
		})
	}
}

func TestUtlsFallbackProxyRoutePreservesActualDirectTransport(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { _, _ = w.Write([]byte("ok")) }))
	defer server.Close()
	SetRequestProxyRouteObserver(func(actual string, conn net.Conn) func() *usage.ProxyRoute {
		if actual != "" {
			t.Errorf("direct fallback route = %q", actual)
		}
		return func() *usage.ProxyRoute { return &usage.ProxyRoute{Kind: "direct"} }
	})
	defer SetRequestProxyRouteObserver(nil)
	for _, proxyURL := range []string{"", "direct"} {
		t.Run(strings.ReplaceAll(proxyURL, ":", "_"), func(t *testing.T) {
			reporter := NewUsageReporter(t.Context(), "codex", "test", nil)
			client := reporter.TrackHTTPClient(NewUtlsHTTPClient(t.Context(), nil, &cliproxyauth.Auth{ProxyURL: proxyURL}, 0))
			defer client.CloseIdleConnections()
			response, err := client.Get(server.URL)
			if err != nil {
				t.Fatal(err)
			}
			_, _ = io.Copy(io.Discard, response.Body)
			response.Body.Close()
			if record := reporter.buildRecord(usage.Detail{}, false); record.ProxyRoute == nil {
				t.Fatal("missing fallback route")
			}
		})
	}
}

func TestUtlsCustomContextTransportKeepsUnknownProxyRoute(t *testing.T) {
	left, right := net.Pipe()
	defer left.Close()
	defer right.Close()
	SetRequestProxyRouteObserver(func(actual string, conn net.Conn) func() *usage.ProxyRoute {
		if actual != "unknown" {
			t.Errorf("custom transport proxy = %q, want unknown", actual)
		}
		return func() *usage.ProxyRoute { return &usage.ProxyRoute{Kind: "unknown"} }
	})
	defer SetRequestProxyRouteObserver(nil)
	ctx := context.WithValue(t.Context(), "cliproxy.roundtripper", proxyTraceRetryTransport{conns: []net.Conn{left}})
	reporter := NewUsageReporter(ctx, "codex", "test", nil)
	response, err := reporter.TrackHTTPClient(NewUtlsHTTPClient(ctx, nil, nil, 0)).Get("https://chatgpt.com/backend-api/codex/responses")
	if err != nil {
		t.Fatal(err)
	}
	response.Body.Close()
	if route := reporter.buildRecord(usage.Detail{}, false).ProxyRoute; route == nil || route.Kind != "unknown" {
		t.Fatalf("custom transport route = %+v", route)
	}
}
