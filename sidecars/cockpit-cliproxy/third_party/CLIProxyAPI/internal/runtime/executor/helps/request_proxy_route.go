package helps

import (
	"context"
	"net"
	"net/http"
	"reflect"
	"sync"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/proxyutil"
)

var requestProxyRouteObserver struct {
	sync.RWMutex
	observe func(string, net.Conn) func() *usage.ProxyRoute
}

// SetRequestProxyRouteObserver installs a connection observer. Both the observer
// and its returned snapshot getter must return promptly without network I/O waits.
// An empty proxy URL means confirmed direct; "unknown" means undetermined routing.
func SetRequestProxyRouteObserver(observer func(string, net.Conn) func() *usage.ProxyRoute) {
	requestProxyRouteObserver.Lock()
	requestProxyRouteObserver.observe = observer
	requestProxyRouteObserver.Unlock()
}

// ObserveRequestProxyRoute starts optional side-channel observation of an actual connection.
func ObserveRequestProxyRoute(proxyURL string, conn net.Conn) func() *usage.ProxyRoute {
	requestProxyRouteObserver.RLock()
	observe := requestProxyRouteObserver.observe
	requestProxyRouteObserver.RUnlock()
	if observe == nil || conn == nil {
		return nil
	}
	return observe(proxyURL, conn)
}

// SetProxyRouteGetter records the latest connection used, including retries and reuse.
func (r *UsageReporter) SetProxyRouteGetter(getter func() *usage.ProxyRoute) {
	if r == nil {
		return
	}
	r.proxyRouteMu.Lock()
	r.proxyRouteGetter = getter
	r.proxyRouteMu.Unlock()
}

func (r *UsageReporter) proxyRouteSnapshot() *usage.ProxyRoute {
	r.proxyRouteMu.RLock()
	getter := r.proxyRouteGetter
	r.proxyRouteMu.RUnlock()
	if getter == nil {
		return nil
	}
	route := getter()
	if route == nil {
		return nil
	}
	snapshot := *route
	return &snapshot
}

// proxyRouteConn preserves connection-layer routing metadata (not credentials in usage).
// NetConn allows TLS and side-channel observers to unwrap the socket normally.
type proxyRouteConn struct {
	net.Conn
	proxyURL string
}

func (c *proxyRouteConn) NetConn() net.Conn { return c.Conn }

func annotateProxyTransport(transport *http.Transport, raw string) {
	setting, errParse := proxyutil.Parse(raw)
	if errParse != nil || transport == nil {
		return
	}
	if setting.Mode != proxyutil.ModeDirect && setting.Mode != proxyutil.ModeProxy {
		return
	}
	proxyURL := ""
	if setting.URL != nil {
		proxyURL = setting.URL.String()
	}
	dial := transport.DialContext
	if dial == nil {
		// Do not replace custom legacy dialers or TLS dialing behavior.
		if transport.Dial != nil {
			return
		}
		dial = (&net.Dialer{}).DialContext
	}
	transport.DialContext = func(ctx context.Context, network, addr string) (net.Conn, error) {
		conn, errDial := dial(ctx, network, addr)
		if errDial != nil {
			return conn, errDial
		}
		return &proxyRouteConn{Conn: conn, proxyURL: proxyURL}, nil
	}
}

func effectiveHTTPProxyURL(base http.RoundTripper, req *http.Request, conn net.Conn) string {
	for depth := 0; conn != nil && depth < 8; depth++ {
		if routed, ok := conn.(*proxyRouteConn); ok {
			return routed.proxyURL
		}
		unwrapper, ok := conn.(interface{ NetConn() net.Conn })
		if !ok {
			break
		}
		conn = unwrapper.NetConn()
	}
	if fallback, ok := base.(*fallbackRoundTripper); ok {
		base = fallback.transportForRequest(req)
	}
	transport, ok := base.(*http.Transport)
	if !ok || transport == nil {
		return "unknown"
	}
	if transport.Proxy != nil {
		// Only the standard environment resolver is safe to query again. Custom
		// proxy functions may be stateful and must not be invoked for observation.
		if reflect.ValueOf(transport.Proxy).Pointer() != reflect.ValueOf(http.ProxyFromEnvironment).Pointer() {
			return "unknown"
		}
		selected, errProxy := transport.Proxy(req)
		if errProxy != nil {
			return "unknown"
		}
		if selected != nil {
			return selected.String()
		}
	}
	if transport == http.DefaultTransport || (transport.Dial == nil && transport.DialContext == nil && transport.DialTLS == nil && transport.DialTLSContext == nil) {
		return ""
	}
	return "unknown"
}
