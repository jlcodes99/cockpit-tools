package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func TestGatewayIdentityDirectProxyProjectionAndAccountIsolation(t *testing.T) {
	gin.SetMode(gin.TestMode)
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if r.URL.Path == imagesGenerationsPath {
			_, _ = io.WriteString(w, `{"data":[{"b64_json":"`+imageAPITestPNGBase64+`"}]}`)
			return
		}
		_, _ = io.WriteString(w, `{"id":"r","object":"response","status":"completed","output":[]}`)
	}))
	defer upstream.Close()
	forward := http.DefaultTransport.(*http.Transport).Clone()
	forward.Proxy = nil
	defer forward.CloseIdleConnections()
	var mu sync.Mutex
	connections := map[string][]string{}
	proxyCalls := 0
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !r.URL.IsAbs() {
			t.Error("request did not use HTTP proxy absolute form")
		}
		mu.Lock()
		proxyCalls++
		key := r.URL.Path + "|" + r.Header.Get("Authorization")
		connections[key] = append(connections[key], r.RemoteAddr)
		mu.Unlock()
		request := r.Clone(r.Context())
		request.RequestURI = ""
		request.Header.Del("Proxy-Connection")
		response, err := forward.RoundTrip(request)
		if err != nil {
			t.Error(err)
			http.Error(w, "fixture proxy error", 502)
			return
		}
		defer response.Body.Close()
		for name, values := range response.Header {
			w.Header()[name] = values
		}
		w.WriteHeader(response.StatusCode)
		if _, err := io.Copy(w, response.Body); err != nil {
			t.Error(err)
		}
	}))
	defer proxy.Close()
	secondProfile := json.RawMessage(`{"version":1,"scope":"` + strings.Repeat("b", 64) + `","installationId":"22222222-2222-4222-8222-222222222222","revision":1}`)
	fixture := manifest{Accounts: []accountSpec{
		{ID: "proxy-a", UpstreamAPIKey: "key-a", UpstreamProxyURL: proxy.URL, GatewayIdentity: gatewayIdentityFixture()},
		{ID: "proxy-b", UpstreamAPIKey: "key-b", UpstreamProxyURL: proxy.URL, GatewayIdentity: secondProfile},
	}}
	raw, err := json.Marshal(fixture)
	if err != nil {
		t.Fatal(err)
	}
	manifestPath := filepath.Join(t.TempDir(), "manifest.json")
	if err := os.WriteFile(manifestPath, raw, 0600); err != nil {
		t.Fatal(err)
	}
	loaded, err := loadManifest(manifestPath)
	if err != nil {
		t.Fatal(err)
	}
	s := &relayServer{manifest: loaded}
	call := func(accountID, kind string) {
		t.Helper()
		account := loaded.accountByID[accountID]
		gateway := &providerGatewaySpec{BaseURL: upstream.URL, APIKey: account.UpstreamAPIKey, WireAPI: "responses", UpstreamModel: "test", UpstreamModels: []string{"test"}}
		body := []byte(`{"model":"test","input":"hello"}`)
		w := httptest.NewRecorder()
		c, _ := gin.CreateTestContext(w)
		c.Request = httptest.NewRequest("POST", "/v1/responses", nil).WithContext(context.WithValue(context.Background(), providerGatewayAccountContextKey{}, accountID))
		if kind == "provider" {
			s.handleProviderGatewayRequest(c, gateway, body, "test", sdktranslator.FormatOpenAIResponse, "")
			if w.Code != 200 {
				t.Fatal(w.Code, w.Body.String())
			}
			return
		}
		response, _, err := s.forwardProviderAudio(c, gateway, &audioRequest{body: body, contentType: "application/json", operation: "speech"}, "test")
		if err != nil || response == nil {
			t.Fatal("audio fixture request failed", err)
		}
		if _, err := io.Copy(io.Discard, response.Body); err != nil {
			t.Error(err)
		}
		if err := response.Body.Close(); err != nil {
			t.Error(err)
		}
	}
	for _, kind := range []string{"provider", "audio"} {
		call("proxy-a", kind)
		call("proxy-a", kind)
		call("proxy-b", kind)
	}
	mu.Lock()
	for _, path := range []string{"/v1/responses", "/v1/audio/speech"} {
		first := connections[path+"|Bearer key-a"]
		second := connections[path+"|Bearer key-b"]
		if len(first) != 2 || len(second) != 1 {
			t.Errorf("proxy route missing: %s a=%v b=%v", path, first, second)
			continue
		}
		if first[0] != first[1] {
			t.Error("same-account pool was not reused", path)
		}
		if first[0] == second[0] {
			t.Error("different accounts shared proxy connection", path)
		}
	}
	beforeDirect := proxyCalls
	mu.Unlock()

	// Force the inherited default through the proxy. Explicit direct must override
	// it, while a manifest without an identity must retain the legacy default.
	previousDefault := http.DefaultTransport
	inherited := previousDefault.(*http.Transport).Clone()
	proxyURL, _ := url.Parse(proxy.URL)
	inherited.Proxy = http.ProxyURL(proxyURL)
	http.DefaultTransport = inherited
	defer func() { http.DefaultTransport = previousDefault; inherited.CloseIdleConnections() }()
	loaded.accountByID["proxy-a"].UpstreamProxyURL = "direct"
	call("proxy-a", "provider")
	call("proxy-a", "audio")
	mu.Lock()
	afterDirect := proxyCalls
	mu.Unlock()
	if afterDirect != beforeDirect {
		t.Fatal("explicit direct reached the proxy")
	}

	// Images retain their own upstream proxy override ahead of the account route.
	images := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	imageAccount := images.manifest.accountByID["images-key"]
	imageAccount.GatewayIdentity = gatewayIdentityFixture()
	imageAccount.UpstreamProxyURL = proxy.URL
	imageAccount.ImageGenerationUpstream.ProxyURL = "direct"
	if response := imageAPITestRequest(images, imagesGenerationsPath, `{"model":"gpt-image-2.5","prompt":"draw"}`); response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	mu.Lock()
	afterImage := proxyCalls
	mu.Unlock()
	if afterImage != beforeDirect {
		t.Fatal("image-specific direct override was lost")
	}

	loaded.accountByID["proxy-a"].GatewayIdentity = nil
	call("proxy-a", "provider")
	call("proxy-a", "audio")
	mu.Lock()
	afterLegacy := proxyCalls
	mu.Unlock()
	if afterLegacy != beforeDirect+2 {
		t.Fatal("legacy manifest changed inherited proxy behavior", afterLegacy, beforeDirect)
	}
}
