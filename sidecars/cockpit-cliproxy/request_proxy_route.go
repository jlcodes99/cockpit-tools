package main

import (
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/url"
	"reflect"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
)

// Controller credentials are private manifest data and never enter a usage event.
type proxyRouteObserverSpec struct {
	ProxyURL         string            `json:"proxyUrl"`
	MappingURL       string            `json:"mappingUrl"`
	ControllerURL    string            `json:"controllerUrl"`
	ControllerSecret string            `json:"controllerSecret"`
	NodeNames        map[string]string `json:"nodeNames"`
	ProxyName        string            `json:"proxyName"`
}

const requestProxyRouteCacheLimit = 256
const requestProxyRouteWorkerLimit = 8

type proxyRouteSnapshot struct {
	value atomic.Pointer[usage.ProxyRoute]
}

func (s *proxyRouteSnapshot) get() *usage.ProxyRoute {
	value := s.value.Load()
	if value == nil {
		return nil
	}
	copy := *value
	return &copy
}

// All potentially slow work happens after acquiring a bounded worker slot. Neither
// observing a connection nor getting its current snapshot waits for controller I/O.
func newRequestProxyRouteObserver(specs []proxyRouteObserverSpec) func(string, net.Conn) func() *usage.ProxyRoute {
	observers := make(map[string]proxyRouteObserverSpec)
	for _, spec := range specs {
		endpoint := requestProxyEndpoint(spec.ProxyURL)
		controller := requestProxyControllerURL(spec.ControllerURL)
		mapping := requestProxyMappingURL(spec.MappingURL, endpoint)
		if endpoint == "" || (controller == "" && mapping == "") {
			continue
		}
		spec.ProxyURL, spec.ControllerURL, spec.MappingURL = endpoint, controller, mapping
		names := make(map[string]string, len(spec.NodeNames))
		for tag, name := range spec.NodeNames {
			if tag != "" && name != "" {
				names[tag] = name
			}
		}
		spec.NodeNames = names
		observers[endpoint] = spec
	}
	client := &http.Client{
		Timeout:       160 * time.Millisecond,
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
		Transport: &http.Transport{Proxy: nil, DialContext: (&net.Dialer{Timeout: 150 * time.Millisecond}).DialContext,
			MaxIdleConns: 8, MaxIdleConnsPerHost: 2, IdleConnTimeout: time.Minute},
	}
	workers := make(chan struct{}, requestProxyRouteWorkerLimit)
	cache := make(map[net.Conn]*proxyRouteSnapshot)
	var mu sync.Mutex
	return func(proxyURL string, conn net.Conn) func() *usage.ProxyRoute {
		endpoint := requestProxyEndpoint(proxyURL)
		fallback := usage.ProxyRoute{Kind: "proxy", Name: endpoint}
		if value := strings.ToLower(strings.TrimSpace(proxyURL)); value == "" || value == "direct" || value == "none" {
			fallback.Kind = "direct"
		}
		if endpoint == "" && fallback.Kind != "direct" {
			fallback.Kind = "unknown"
		}
		spec, observed := observers[endpoint]
		if observed && spec.ProxyName != "" {
			fallback.Name = spec.ProxyName
		}
		state := &proxyRouteSnapshot{}
		state.value.Store(&fallback)
		conn = requestProxyBaseConn(conn)
		if !observed || conn == nil || !reflect.TypeOf(conn).Comparable() || !requestProxyRemoteMatches(conn, endpoint) {
			return state.get
		}
		mu.Lock()
		if previous := cache[conn]; previous != nil {
			mu.Unlock()
			return previous.get
		}
		if len(cache) >= requestProxyRouteCacheLimit {
			// Never evict a live connection: its node must remain frozen across
			// keep-alive reuse. Closed TCP identities can be safely reclaimed.
			for key := range cache {
				if !requestProxyConnOpen(key) {
					delete(cache, key)
				}
			}
			if len(cache) >= requestProxyRouteCacheLimit {
				mu.Unlock()
				return state.get
			}
		}
		cache[conn] = state
		mu.Unlock()
		select {
		case workers <- struct{}{}:
			go func() {
				defer func() { <-workers }()
				if route := captureRequestProxyRoute(client, spec, conn); route != nil {
					state.value.Store(route)
				}
			}()
		default:
			// Saturation must never queue work or delay the upstream request.
		}
		return state.get
	}
}

func requestProxyBaseConn(conn net.Conn) net.Conn {
	for i := 0; i < 8 && conn != nil; i++ {
		wrapper, ok := conn.(interface{ NetConn() net.Conn })
		if !ok {
			break
		}
		next := wrapper.NetConn()
		if next == nil {
			break
		}
		conn = next
	}
	return conn
}

func requestProxyEndpoint(raw string) string {
	parsed, err := url.Parse(strings.TrimSpace(raw))
	if err != nil || parsed.Hostname() == "" {
		return ""
	}
	port := parsed.Port()
	switch parsed.Scheme {
	case "http":
		if port == "" {
			port = "80"
		}
	case "https":
		if port == "" {
			port = "443"
		}
	case "socks5", "socks5h", "socks":
		if port == "" {
			port = "1080"
		}
	default:
		return ""
	}
	number, err := strconv.Atoi(port)
	if err != nil || number < 1 || number > 65535 {
		return ""
	}
	return parsed.Scheme + "://" + net.JoinHostPort(strings.ToLower(parsed.Hostname()), strconv.Itoa(number))
}

func requestProxyControllerURL(raw string) string {
	parsed, err := url.Parse(raw)
	if err != nil || parsed.Scheme != "http" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" || (parsed.Path != "" && parsed.Path != "/") {
		return ""
	}
	host := parsed.Hostname()
	if host == "localhost" {
		host = "127.0.0.1"
	}
	ip := net.ParseIP(host)
	port, err := strconv.Atoi(parsed.Port())
	if ip == nil || !ip.IsLoopback() || err != nil || port < 1 || port > 65535 {
		return ""
	}
	return "http://" + net.JoinHostPort(ip.String(), strconv.Itoa(port)) + "/connections"
}

// Mapping shares the exact stable proxy endpoint; no redirects or external hosts.
func requestProxyMappingURL(raw, endpoint string) string {
	parsed, err := url.Parse(raw)
	proxy, proxyErr := url.Parse(endpoint)
	if err != nil || proxyErr != nil || parsed.Scheme != "http" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" || parsed.Path != "/_cockpit/proxy-route" || parsed.Host != proxy.Host {
		return ""
	}
	ip := net.ParseIP(parsed.Hostname())
	port, err := strconv.Atoi(parsed.Port())
	if ip == nil || !ip.IsLoopback() || err != nil || port < 1 || port > 65535 {
		return ""
	}
	return parsed.String()
}

func queryMappedRequestProxyRoute(ctx context.Context, client *http.Client, spec proxyRouteObserverSpec, sourceIP, sourcePort string) *usage.ProxyRoute {
	if spec.MappingURL == "" {
		return queryRequestProxyRoute(ctx, client, spec, sourceIP, sourcePort)
	}
	query := url.Values{"sourceIP": {sourceIP}, "sourcePort": {sourcePort}}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, spec.MappingURL+"?"+query.Encode(), nil)
	if err != nil {
		return nil
	}
	request.Header.Set("Authorization", "Bearer "+spec.ControllerSecret)
	response, err := client.Do(request)
	if err != nil {
		return nil
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil
	}
	const maxBody = 2 * 1024 * 1024
	body, err := io.ReadAll(io.LimitReader(response.Body, maxBody+1))
	if err != nil || len(body) > maxBody {
		return nil
	}
	var mapping struct {
		SourceIP   string                 `json:"sourceIP"`
		SourcePort string                 `json:"sourcePort"`
		Observer   proxyRouteObserverSpec `json:"observer"`
		Route      *usage.ProxyRoute      `json:"route"`
	}
	if json.Unmarshal(body, &mapping) != nil {
		return nil
	}
	if mapping.Route != nil {
		switch mapping.Route.Kind {
		case "direct":
			return &usage.ProxyRoute{Kind: "direct"}
		case "proxy":
			if endpoint := requestProxyEndpoint(mapping.Route.Name); endpoint != "" {
				return &usage.ProxyRoute{Kind: "proxy", Name: endpoint}
			}
		}
		return nil
	}
	ip := net.ParseIP(mapping.SourceIP)
	port, err := strconv.Atoi(mapping.SourcePort)
	controller := requestProxyControllerURL(mapping.Observer.ControllerURL)
	if ip == nil || !ip.IsLoopback() || err != nil || port < 1 || port > 65535 || controller == "" {
		return nil
	}
	mapping.Observer.ControllerURL = controller
	return queryRequestProxyRoute(ctx, client, mapping.Observer, ip.String(), strconv.Itoa(port))
}

func requestProxyRemoteMatches(conn net.Conn, endpoint string) bool {
	if conn.RemoteAddr() == nil {
		return false
	}
	host, port, err := net.SplitHostPort(conn.RemoteAddr().String())
	if err != nil {
		return false
	}
	parsed, err := url.Parse(endpoint)
	if err != nil || port != parsed.Port() {
		return false
	}
	expected := parsed.Hostname()
	if expected == "localhost" {
		return net.ParseIP(host).IsLoopback()
	}
	return net.ParseIP(host) != nil && net.ParseIP(host).Equal(net.ParseIP(expected))
}

// A closed TCP socket must not pick up a newer connection that reused its source
// port. RawConn.Control checks the original socket's lifetime without consuming I/O.
func requestProxyConnOpen(conn net.Conn) bool {
	socket, ok := conn.(syscall.Conn)
	if !ok {
		return true
	}
	raw, err := socket.SyscallConn()
	return err == nil && raw.Control(func(uintptr) {}) == nil
}

func captureRequestProxyRoute(client *http.Client, spec proxyRouteObserverSpec, conn net.Conn) *usage.ProxyRoute {
	if conn.LocalAddr() == nil {
		return nil
	}
	host, port, err := net.SplitHostPort(conn.LocalAddr().String())
	if err != nil || net.ParseIP(host) == nil {
		return nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 500*time.Millisecond)
	defer cancel()
	for attempt := 0; attempt < 4; attempt++ {
		if !requestProxyConnOpen(conn) {
			return nil
		}
		route := queryMappedRequestProxyRoute(ctx, client, spec, host, port)
		if !requestProxyConnOpen(conn) {
			return nil
		}
		if route != nil {
			return route
		}
		if attempt < 3 {
			select {
			case <-ctx.Done():
				return nil
			case <-time.After(20 * time.Millisecond):
			}
		}
	}
	return nil
}

func queryRequestProxyRoute(ctx context.Context, client *http.Client, spec proxyRouteObserverSpec, sourceIP, sourcePort string) *usage.ProxyRoute {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, spec.ControllerURL, nil)
	if err != nil {
		return nil
	}
	request.Header.Set("Authorization", "Bearer "+spec.ControllerSecret)
	response, err := client.Do(request)
	if err != nil {
		return nil
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil
	}
	const maxBody = 2 * 1024 * 1024
	body, err := io.ReadAll(io.LimitReader(response.Body, maxBody+1))
	if err != nil || len(body) > maxBody {
		return nil
	}
	var payload struct {
		Connections []struct {
			Metadata struct {
				SourceIP   string      `json:"sourceIP"`
				SourcePort json.Number `json:"sourcePort"`
			} `json:"metadata"`
			Chains []string `json:"chains"`
		} `json:"connections"`
	}
	if json.Unmarshal(body, &payload) != nil {
		return nil
	}
	var found *usage.ProxyRoute
	for _, connection := range payload.Connections {
		if connection.Metadata.SourcePort.String() != sourcePort || !net.ParseIP(connection.Metadata.SourceIP).Equal(net.ParseIP(sourceIP)) {
			continue
		}
		var route *usage.ProxyRoute
		for _, tag := range connection.Chains {
			if tag == "REJECT" || tag == "REJECT-DROP" {
				return nil
			}
			var candidate *usage.ProxyRoute
			if tag == "DIRECT" {
				candidate = &usage.ProxyRoute{Kind: "direct"}
			} else if name, ok := spec.NodeNames[tag]; ok {
				candidate = &usage.ProxyRoute{Kind: "node", Name: name}
			}
			if candidate != nil {
				if route != nil && *route != *candidate {
					return nil
				}
				route = candidate
			}
		}
		if route == nil {
			return nil
		}
		if found != nil {
			return nil
		}
		found = route
	}
	return found
}
