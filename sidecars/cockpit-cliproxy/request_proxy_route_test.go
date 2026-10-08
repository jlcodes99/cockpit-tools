package main

import (
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
)

type proxyRouteTestConn struct{ local, remote net.Addr }

func (c *proxyRouteTestConn) Read([]byte) (int, error)         { return 0, io.EOF }
func (c *proxyRouteTestConn) Write(p []byte) (int, error)      { return len(p), nil }
func (c *proxyRouteTestConn) Close() error                     { return nil }
func (c *proxyRouteTestConn) LocalAddr() net.Addr              { return c.local }
func (c *proxyRouteTestConn) RemoteAddr() net.Addr             { return c.remote }
func (c *proxyRouteTestConn) SetDeadline(time.Time) error      { return nil }
func (c *proxyRouteTestConn) SetReadDeadline(time.Time) error  { return nil }
func (c *proxyRouteTestConn) SetWriteDeadline(time.Time) error { return nil }
func routeTestConn(port int) *proxyRouteTestConn {
	return &proxyRouteTestConn{
		local: &net.TCPAddr{IP: net.ParseIP("127.0.0.1"), Port: port}, remote: &net.TCPAddr{IP: net.ParseIP("127.0.0.1"), Port: 1080},
	}
}

func TestRequestProxyRouteSnapshotsConnectionLeafAndReusesFrozenResult(t *testing.T) {
	var leaf atomic.Value
	leaf.Store("leaf-a")
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.URL.Path != "/connections" || r.Header.Get("Authorization") != "Bearer private-secret" {
			t.Error("missing private controller authentication")
		}
		fmt.Fprintf(w, `{"connections":[{"metadata":{"sourceIP":"127.0.0.1","sourcePort":"41000"},"chains":["%s","load-balance","account-node"]}]}`, leaf.Load())
	}))
	defer server.Close()
	observe := newRequestProxyRouteObserver([]proxyRouteObserverSpec{{ProxyURL: "socks5h://127.0.0.1:1080", ControllerURL: server.URL, ControllerSecret: "private-secret", ProxyName: "My group", NodeNames: map[string]string{"leaf-a": "Tokyo", "leaf-b": "Singapore"}}})
	conn := routeTestConn(41000)
	started := time.Now()
	get := observe("socks5h://private-user:private-password@127.0.0.1:1080", conn)
	if time.Since(started) > 50*time.Millisecond {
		t.Fatal("observation blocked")
	}
	deadline := time.Now().Add(time.Second)
	for get().Kind != "node" && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if result := get(); result.Kind != "node" || result.Name != "Tokyo" {
		t.Fatalf("wrong leaf: %+v", result)
	}
	get().Name = "mutated externally"
	if get().Name != "Tokyo" {
		t.Fatal("getter exposed shared snapshot")
	}
	leaf.Store("leaf-b")
	if observe("socks5h://other:credentials@127.0.0.1:1080", conn)().Name != "Tokyo" {
		t.Fatal("same connection lost frozen node")
	}
	if requests.Load() != 1 {
		t.Fatalf("same connection triggered new lookup: %d", requests.Load())
	}
	// A distinct connection object may reuse the same tuple; it needs a fresh lookup.
	next := observe("socks5h://127.0.0.1:1080", routeTestConn(41000))
	deadline = time.Now().Add(time.Second)
	for next().Kind != "node" && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if next().Name != "Singapore" {
		t.Fatalf("new connection inherited old cache: %+v", next())
	}
	body, _ := json.Marshal(get())
	if strings.Contains(string(body), "private") {
		t.Fatal("private controller metadata leaked into route")
	}
}

func TestRequestProxyRouteMatchesSourceAndProxyEndpoint(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		io.WriteString(w, `{"connections":[{"metadata":{"sourceIP":"127.0.0.2","sourcePort":41000},"chains":["wrong"]},{"metadata":{"sourceIP":"127.0.0.1","sourcePort":41001},"chains":["wrong"]},{"metadata":{"sourceIP":"127.0.0.1","sourcePort":41000},"chains":["right","group"]}]}`)
	}))
	defer server.Close()
	observe := newRequestProxyRouteObserver([]proxyRouteObserverSpec{{ProxyURL: "socks5h://127.0.0.1:1080", ControllerURL: server.URL, NodeNames: map[string]string{"right": "Correct", "wrong": "Wrong"}}})
	wrongConn := routeTestConn(41000)
	wrongConn.remote = &net.TCPAddr{IP: net.ParseIP("127.0.0.1"), Port: 9999}
	if got := observe("socks5h://127.0.0.1:1080", wrongConn)(); got.Kind != "proxy" {
		t.Fatalf("unexpected route: %+v", got)
	}
	if requests.Load() != 0 {
		t.Fatal("queried unrelated proxy controller")
	}
	get := observe("socks5h://127.0.0.1:1080", routeTestConn(41000))
	deadline := time.Now().Add(time.Second)
	for get().Kind != "node" && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if get().Name != "Correct" {
		t.Fatalf("wrong connection matched: %+v", get())
	}
}

func TestRequestProxyRouteFallbackAndControllerValidation(t *testing.T) {
	for _, raw := range []string{"https://127.0.0.1:90", "http://example.com:90", "http://127.0.0.1:90/?token=x", "http://u:p@127.0.0.1:90", "http://127.0.0.1:90/other"} {
		if requestProxyControllerURL(raw) != "" {
			t.Fatalf("unsafe controller accepted: %s", raw)
		}
	}
	observe := newRequestProxyRouteObserver(nil)
	if got := observe("", nil)(); got.Kind != "direct" {
		t.Fatalf("explicit direct lost: %+v", got)
	}
	if got := observe("http://user:secret@[::1]:8888/private?secret=x#fragment", nil)(); got.Kind != "proxy" || got.Name != "http://[::1]:8888" {
		t.Fatalf("fallback leaked credentials: %+v", got)
	}
	if got := observe("invalid secret", nil)(); got.Kind != "unknown" || got.Name != "" {
		t.Fatalf("invalid URL exposed: %+v", got)
	}
}

func TestRequestProxyRouteSlowControllerNeverBlocksAndWorkersAreBounded(t *testing.T) {
	release := make(chan struct{})
	var active, maximum atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := active.Add(1)
		defer active.Add(-1)
		for old := maximum.Load(); n > old && !maximum.CompareAndSwap(old, n); old = maximum.Load() {
		}
		select {
		case <-release:
		case <-r.Context().Done():
		}
	}))
	defer server.Close()
	observe := newRequestProxyRouteObserver([]proxyRouteObserverSpec{{ProxyURL: "socks5h://127.0.0.1:1080", ControllerURL: server.URL, ProxyName: "Safe label"}})
	started := time.Now()
	for i := 0; i < requestProxyRouteCacheLimit+40; i++ {
		if got := observe("socks5h://u:p@127.0.0.1:1080", routeTestConn(41000+i))(); got.Kind != "proxy" || got.Name != "Safe label" {
			t.Fatalf("wrong fallback: %+v", got)
		}
	}
	if time.Since(started) > 100*time.Millisecond {
		t.Fatal("slow controller blocked observe/getter")
	}
	deadline := time.Now().Add(time.Second)
	for maximum.Load() == 0 && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	close(release)
	if maximum.Load() > requestProxyRouteWorkerLimit {
		t.Fatalf("too many controller workers: %d", maximum.Load())
	}
}

func TestRequestProxyRouteControllerRedirectCannotLeakSecret(t *testing.T) {
	var targetCalls atomic.Int32
	target := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { targetCalls.Add(1) }))
	defer target.Close()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, target.URL, http.StatusFound) }))
	defer server.Close()
	observe := newRequestProxyRouteObserver([]proxyRouteObserverSpec{{ProxyURL: "socks5h://127.0.0.1:1080", ControllerURL: server.URL, ControllerSecret: "secret"}})
	get := observe("socks5h://127.0.0.1:1080", routeTestConn(41000))
	time.Sleep(180 * time.Millisecond)
	if get().Kind != "proxy" || targetCalls.Load() != 0 {
		t.Fatal("redirect followed or route fabricated")
	}
}

func TestRequestProxyRouteDirectRequiresAnExplicitUnambiguousChain(t *testing.T) {
	for _, test := range []struct {
		chains string
		kind   string
	}{
		{`["DIRECT","group"]`, "direct"},
		{`["REJECT","group"]`, "proxy"},
		{`["REJECT-DROP","group"]`, "proxy"},
		{`["DIRECT","leaf","group"]`, "proxy"},
		{`["group"]`, "proxy"},
	} {
		t.Run(test.chains, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				fmt.Fprintf(w, `{"connections":[{"metadata":{"sourceIP":"127.0.0.1","sourcePort":41000},"chains":%s}]}`, test.chains)
			}))
			defer server.Close()
			client := server.Client()
			spec := proxyRouteObserverSpec{ControllerURL: server.URL + "/connections", NodeNames: map[string]string{"leaf": "Tokyo"}}
			route := queryRequestProxyRoute(t.Context(), client, spec, "127.0.0.1", "41000")
			if test.kind == "proxy" {
				if route != nil {
					t.Fatalf("fabricated route: %+v", route)
				}
			} else if route == nil || route.Kind != "direct" {
				t.Fatalf("explicit direct missing: %+v", route)
			}
		})
	}
	observe := newRequestProxyRouteObserver(nil)
	for _, raw := range []string{"direct", "none", " DIRECT "} {
		if observe(raw, nil)().Kind != "direct" {
			t.Fatalf("sentinel %q lost", raw)
		}
	}
}

func TestRequestProxyRoutePublishedFallbackCopyDoesNotChangeAfterLookup(t *testing.T) {
	release := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		<-release
		io.WriteString(w, `{"connections":[{"metadata":{"sourceIP":"127.0.0.1","sourcePort":41000},"chains":["leaf"]}]}`)
	}))
	defer server.Close()
	observe := newRequestProxyRouteObserver([]proxyRouteObserverSpec{{ProxyURL: "socks5h://127.0.0.1:1080", ControllerURL: server.URL, ProxyName: "Safe fallback", NodeNames: map[string]string{"leaf": "Tokyo"}}})
	get := observe("socks5h://127.0.0.1:1080", routeTestConn(41000))
	published := get()
	close(release)
	deadline := time.Now().Add(time.Second)
	for get().Kind != "node" && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if get().Name != "Tokyo" {
		t.Fatalf("capture did not resolve: %+v", get())
	}
	if published.Kind != "proxy" || published.Name != "Safe fallback" {
		t.Fatalf("already published event mutated: %+v", published)
	}
}

func TestRequestProxyRouteClosedTCPConnectionCannotMatchReusedSourcePort(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	conn, err := net.Dial("tcp", listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	peer, err := listener.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer peer.Close()
	if !requestProxyConnOpen(conn) {
		t.Fatal("live TCP socket rejected")
	}
	conn.Close()
	if requestProxyConnOpen(conn) {
		t.Fatal("closed TCP socket may reuse another connection's node")
	}
}

func TestRequestProxyRouteRejectsOversizedAndAmbiguousControllerResponses(t *testing.T) {
	for _, body := range []string{
		strings.Repeat(" ", 2*1024*1024+1),
		`{"connections":[{"metadata":{"sourceIP":"127.0.0.1","sourcePort":41000},"chains":["leaf"]},{"metadata":{"sourceIP":"127.0.0.1","sourcePort":41000},"chains":["leaf"]}]}`,
		`{"connections":[{"metadata":{"sourceIP":"127.0.0.1","sourcePort":41000},"chains":["leaf","other-leaf"]}]}`,
	} {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.WriteString(w, body) }))
		spec := proxyRouteObserverSpec{ControllerURL: server.URL + "/connections", NodeNames: map[string]string{"leaf": "Tokyo", "other-leaf": "Singapore"}}
		if route := queryRequestProxyRoute(t.Context(), server.Client(), spec, "127.0.0.1", "41000"); route != nil {
			t.Fatalf("ambiguous response became node: %+v", route)
		}
		server.Close()
	}
}

type routeTrackedConn struct {
	*proxyRouteTestConn
	closed atomic.Bool
}

func (c *routeTrackedConn) Close() error                          { c.closed.Store(true); return nil }
func (c *routeTrackedConn) SyscallConn() (syscall.RawConn, error) { return routeRawConn{c}, nil }

type routeRawConn struct{ conn *routeTrackedConn }

func (r routeRawConn) Control(f func(uintptr)) error {
	if r.conn.closed.Load() {
		return net.ErrClosed
	}
	f(1)
	return nil
}
func (r routeRawConn) Read(f func(uintptr) bool) error  { return r.Control(func(fd uintptr) { f(fd) }) }
func (r routeRawConn) Write(f func(uintptr) bool) error { return r.Control(func(fd uintptr) { f(fd) }) }

func TestRequestProxyRouteCacheKeepsLiveSnapshotsAndReclaimsClosedConnections(t *testing.T) {
	var requests atomic.Int32
	var leaf atomic.Value
	leaf.Store("a")
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		io.WriteString(w, `{"connections":[`)
		for i := 0; i <= requestProxyRouteCacheLimit; i++ {
			if i > 0 {
				io.WriteString(w, ",")
			}
			fmt.Fprintf(w, `{"metadata":{"sourceIP":"127.0.0.1","sourcePort":%d},"chains":["%s"]}`, 41000+i, leaf.Load())
		}
		io.WriteString(w, `]}`)
	}))
	defer server.Close()
	observe := newRequestProxyRouteObserver([]proxyRouteObserverSpec{{ProxyURL: "socks5h://127.0.0.1:1080", ControllerURL: server.URL, NodeNames: map[string]string{"a": "Tokyo", "b": "Singapore"}}})
	first := &routeTrackedConn{proxyRouteTestConn: routeTestConn(41000)}
	get := observe("socks5h://127.0.0.1:1080", first)
	deadline := time.Now().Add(time.Second)
	for get().Kind != "node" && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if get().Name != "Tokyo" {
		t.Fatalf("first connection not captured: %+v", get())
	}
	var last *routeTrackedConn
	for i := 1; i < requestProxyRouteCacheLimit; i++ {
		last = &routeTrackedConn{proxyRouteTestConn: routeTestConn(41000 + i)}
		observe("socks5h://127.0.0.1:1080", last)
	}
	time.Sleep(100 * time.Millisecond)
	before := requests.Load()
	extra := &routeTrackedConn{proxyRouteTestConn: routeTestConn(41000 + requestProxyRouteCacheLimit)}
	if result := observe("socks5h://127.0.0.1:1080", extra)(); result.Kind != "proxy" {
		t.Fatalf("capacity did not fall back: %+v", result)
	}
	time.Sleep(30 * time.Millisecond)
	if requests.Load() != before {
		t.Fatal("full live cache scheduled extra controller work")
	}
	leaf.Store("b")
	if observe("socks5h://127.0.0.1:1080", first)().Name != "Tokyo" {
		t.Fatal("live connection snapshot was evicted")
	}
	last.Close()
	next := observe("socks5h://127.0.0.1:1080", extra)
	deadline = time.Now().Add(time.Second)
	for next().Kind != "node" && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if next().Name != "Singapore" {
		t.Fatalf("closed identity was not reclaimed: %+v", next())
	}
}

func TestRequestProxyRouteStableEntryMapsSourceToFrozenController(t *testing.T) {
	var controllerCalls atomic.Int32
	controller := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		controllerCalls.Add(1)
		if r.Header.Get("Authorization") != "Bearer engine-secret" {
			t.Error("wrong controller secret")
		}
		io.WriteString(w, `{"connections":[{"metadata":{"sourceIP":"127.0.0.1","sourcePort":52000},"chains":["leaf-a"]},{"metadata":{"sourceIP":"127.0.0.1","sourcePort":41000},"chains":["wrong"]}]}`)
	}))
	defer controller.Close()
	var mappingCalls atomic.Int32
	mapping := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mappingCalls.Add(1)
		if r.URL.Path != "/_cockpit/proxy-route" || r.URL.Query().Get("sourcePort") != "41000" || r.Header.Get("Authorization") != "Bearer mapping-secret" {
			t.Error("incorrect authenticated source lookup")
		}
		json.NewEncoder(w).Encode(map[string]any{"sourceIP": "127.0.0.1", "sourcePort": "52000", "observer": proxyRouteObserverSpec{ControllerURL: controller.URL, ControllerSecret: "engine-secret", NodeNames: map[string]string{"leaf-a": "Tokyo", "wrong": "Wrong"}}})
	}))
	defer mapping.Close()
	addr := mapping.Listener.Addr().(*net.TCPAddr)
	proxy := fmt.Sprintf("socks5h://127.0.0.1:%d", addr.Port)
	observe := newRequestProxyRouteObserver([]proxyRouteObserverSpec{{ProxyURL: proxy, MappingURL: mapping.URL + "/_cockpit/proxy-route", ControllerSecret: "mapping-secret"}})
	conn := routeTestConn(41000)
	conn.remote = addr
	get := observe(proxy, conn)
	deadline := time.Now().Add(time.Second)
	for get().Kind != "node" && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if result := get(); result.Kind != "node" || result.Name != "Tokyo" {
		t.Fatalf("source mapping lost: %+v", result)
	}
	if observe(proxy, conn)().Name != "Tokyo" || mappingCalls.Load() != 1 || controllerCalls.Load() != 1 {
		t.Fatal("stable connection did not reuse frozen route")
	}
}

func TestRequestProxyRouteMappingValidatesEndpointAndExplicitDirect(t *testing.T) {
	endpoint := "socks5h://127.0.0.1:1080"
	for _, raw := range []string{"http://127.0.0.1:1081/_cockpit/proxy-route", "http://example.com:1080/_cockpit/proxy-route", "http://127.0.0.1:1080/other", "http://user:secret@127.0.0.1:1080/_cockpit/proxy-route", "http://127.0.0.1:1080/_cockpit/proxy-route?secret=x"} {
		if requestProxyMappingURL(raw, endpoint) != "" {
			t.Fatalf("unsafe mapping accepted %q", raw)
		}
	}
	for _, body := range []string{`{"route":{"kind":"direct"}}`, `{"route":{"kind":"proxy","name":"http://user:secret@proxy.example:8080/private"}}`, `{"sourceIP":"127.0.0.1","sourcePort":"52000","observer":{"controllerUrl":"http://example.com:90"}}`} {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.WriteString(w, body) }))
		spec := proxyRouteObserverSpec{MappingURL: server.URL + "/_cockpit/proxy-route"}
		route := queryMappedRequestProxyRoute(t.Context(), server.Client(), spec, "127.0.0.1", "41000")
		if strings.Contains(body, `"kind":"direct"`) && (route == nil || route.Kind != "direct") {
			t.Fatal("explicit direct mapping lost")
		}
		if strings.Contains(body, `"kind":"proxy"`) && (route == nil || route.Name != "http://proxy.example:8080") {
			t.Fatal("proxy mapping leaked private credentials")
		}
		if strings.Contains(body, "controllerUrl") && route != nil {
			t.Fatal("external controller accepted")
		}
		server.Close()
	}
}
