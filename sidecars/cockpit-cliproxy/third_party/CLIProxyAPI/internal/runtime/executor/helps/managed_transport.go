package helps

import (
	"container/list"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"sync"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/proxyutil"
)

// ManagedTransportKey excludes transient Agent Identity assertions. Task changes
// retire connections while signing each request preserves the current pool.
func ManagedTransportKey(auth *coreauth.Auth, proxyURL, protocol string) (string, bool, error) {
	profile, err := ManagedIdentityForAuth(auth)
	if err != nil || profile == nil {
		return "", false, err
	}
	credentials := map[string]any{}
	for _, name := range []string{"api_key", "access_token", "personal_access_token", "at_token", "id_token", "refresh_token", "agent_runtime_id", "agent_private_key", "task_id", "taskId", "agentRuntimeId", "agentPrivateKey", "base_url", "auth_mode", "openai_auth_mode", "account_id", "chatgpt_account_id"} {
		if v := auth.Attributes[name]; v != "" {
			credentials["attribute:"+name] = v
		}
		if v, ok := auth.Metadata[name].(string); ok && v != "" {
			credentials["metadata:"+name] = v
		}
	}
	for name, value := range auth.Attributes {
		if strings.HasPrefix(strings.ToLower(name), "header:") {
			credentials[name] = value
		}
	}
	data, _ := json.Marshal([]any{auth.Attributes["account_id"], auth.Provider, profile, credentials, proxyURL, protocol})
	digest := sha256.Sum256(data)
	return hex.EncodeToString(digest[:]), true, nil
}

type managedPoolEntry struct {
	key, slot string
	transport http.RoundTripper
}

var managedPools = struct {
	sync.Mutex
	entries map[string]*list.Element
	order   *list.List
}{entries: map[string]*list.Element{}, order: list.New()}

const managedPoolLimit = 64

type unavailableManagedTransport struct{ err error }

func (t unavailableManagedTransport) RoundTrip(*http.Request) (*http.Response, error) {
	return nil, t.err
}
func closeManagedIdle(rt http.RoundTripper) {
	if closer, ok := rt.(interface{ CloseIdleConnections() }); ok {
		closer.CloseIdleConnections()
	}
}

// ManagedTransport retains at most 64 pools. Reconfiguration retires idle
// connections only; response bodies and active streams continue to completion.
func ManagedTransport(auth *coreauth.Auth, proxyURL, protocol string, build func() http.RoundTripper) http.RoundTripper {
	key, managed, err := ManagedTransportKey(auth, proxyURL, protocol)
	if err != nil {
		return unavailableManagedTransport{err}
	}
	if !managed {
		return build()
	}
	slot := auth.Provider + "\x00" + auth.Attributes["account_id"] + "\x00" + protocol
	managedPools.Lock()
	defer managedPools.Unlock()
	if found := managedPools.entries[key]; found != nil {
		managedPools.order.MoveToFront(found)
		return found.Value.(*managedPoolEntry).transport
	}
	for e := managedPools.order.Back(); e != nil; {
		previous := e.Prev()
		entry := e.Value.(*managedPoolEntry)
		if entry.slot == slot {
			delete(managedPools.entries, entry.key)
			managedPools.order.Remove(e)
			closeManagedIdle(entry.transport)
		}
		e = previous
	}
	rt := build()
	managedPools.entries[key] = managedPools.order.PushFront(&managedPoolEntry{key, slot, rt})
	for managedPools.order.Len() > managedPoolLimit {
		e := managedPools.order.Back()
		entry := e.Value.(*managedPoolEntry)
		delete(managedPools.entries, entry.key)
		managedPools.order.Remove(e)
		closeManagedIdle(entry.transport)
	}
	return rt
}

// ManagedStandardTransport clones the direct pool as well as explicit proxies.
func ManagedStandardTransport(auth *coreauth.Auth, proxyURL, protocol string) http.RoundTripper {
	return ManagedTransport(auth, proxyURL, protocol, func() http.RoundTripper {
		if strings.TrimSpace(proxyURL) != "" {
			if rt, _, err := proxyutil.BuildHTTPTransport(proxyURL); err == nil && rt != nil {
				annotateProxyTransport(rt, proxyURL)
				return rt
			}
			return unavailableManagedTransport{fmt.Errorf("configured upstream proxy is unavailable")}
		}
		return cloneManagedDefaultTransport()
	})
}

// Clone a standard transport without sharing a mutable default client pool.
func cloneManagedDefaultTransport() *http.Transport {
	if transport, ok := http.DefaultTransport.(*http.Transport); ok {
		return transport.Clone()
	}
	return (&http.Transport{Proxy: http.ProxyFromEnvironment, ForceAttemptHTTP2: true}).Clone()
}

// ManagedWebsocketKey includes stable handshake options but excludes request-level
// session, turn, cache and AgentAssertion values.
func ManagedWebsocketKey(auth *coreauth.Auth, proxyURL, endpoint string, headers http.Header) (string, error) {
	key, managed, err := ManagedTransportKey(auth, proxyURL, "websocket:"+endpoint)
	if err != nil || !managed {
		return key, err
	}
	stable := map[string][]string{}
	for _, name := range []string{"User-Agent", "Originator", "Version", "OpenAI-Beta", "OpenAI-Organization", "OpenAI-Project", "Chatgpt-Account-Id", "X-Codex-Installation-Id"} {
		for actual, values := range headers {
			if strings.EqualFold(name, actual) {
				stable[name] = append(stable[name], values...)
			}
		}
	}
	for actual, values := range headers {
		if strings.EqualFold(actual, "Authorization") {
			for _, value := range values {
				if !strings.HasPrefix(value, "AgentAssertion ") {
					stable["Authorization"] = append(stable["Authorization"], value)
				}
			}
		}
	}
	data, _ := json.Marshal([]any{key, stable})
	digest := sha256.Sum256(data)
	return hex.EncodeToString(digest[:]), nil
}
