package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
)

var testAudio = []byte{'R', 'I', 'F', 'F', 0, 255, 128, 13, 10}

func audioMultipart(t *testing.T, fields map[string]string) ([]byte, string) {
	t.Helper()
	var body bytes.Buffer
	w := multipart.NewWriter(&body)
	file, err := w.CreateFormFile("file", "voice.wav")
	if err != nil {
		t.Fatal(err)
	}
	_, _ = file.Write(testAudio)
	for k, v := range fields {
		if err := w.WriteField(k, v); err != nil {
			t.Fatal(err)
		}
	}
	if err := w.Close(); err != nil {
		t.Fatal(err)
	}
	return body.Bytes(), w.FormDataContentType()
}

func audioServerFixture(t *testing.T, upstream string, oauth bool) (*relayServer, *apiKeySpec) {
	t.Helper()
	spec := &apiKeySpec{ID: "voice-key", Key: "local-key", Enabled: true, AccountIDs: []string{"voice-account"}, AllowedModels: []string{"gpt-5.5"}}
	account := &accountSpec{ID: "voice-account", AuthID: "voice-auth", AuthKind: "oauth"}
	m := &manifest{
		apiKeyByValue: map[string]*apiKeySpec{spec.Key: spec}, accountByID: map[string]*accountSpec{account.ID: account},
		accountByAuthID:      map[string]*accountSpec{account.AuthID: account},
		GatewayErrorMessages: map[string]map[string]string{"audio_invalid_request": {"en": "Invalid audio request: {{detail}}"}},
	}
	tracker := newRequestUsageTracker()
	s := &relayServer{manifest: m, policy: &requestPolicy{manifest: m, tracker: tracker, tokenLimiter: newAPIKeyTokenLimiter(m)}}
	if oauth {
		manager := coreauth.NewManager(nil, buildCoreAuthSelectorWithConcurrency(nil, &coreauth.RoundRobinSelector{}, m, nil, tracker), nil)
		manager.RegisterExecutor(executor.NewCodexExecutor(&config.Config{}))
		_, err := manager.Register(context.Background(), &coreauth.Auth{ID: "voice-auth", Provider: "codex", Attributes: map[string]string{
			"base_url": upstream + "/backend-api/codex", coreauth.AttributeAuthKind: coreauth.AuthKindOAuth,
		}, Metadata: map[string]any{"access_token": "oauth-secret", "account_id": "chatgpt-account"}})
		if err != nil {
			t.Fatal(err)
		}
		s.authManager = manager
	} else {
		account.AuthKind = "api_key"
		spec.ProviderGateway = &providerGatewaySpec{BaseURL: upstream + "/v1", APIKey: "provider-secret"}
	}
	return s, spec
}

func serveAudio(t *testing.T, router http.Handler, path string, body []byte, contentType, key string) *httptest.ResponseRecorder {
	t.Helper()
	r := httptest.NewRequest(http.MethodPost, path, bytes.NewReader(body))
	r.Header.Set("Content-Type", contentType)
	if key != "" {
		r.Header.Set("Authorization", "Bearer "+key)
	}
	w := httptest.NewRecorder()
	router.ServeHTTP(w, r)
	return w
}

func TestAudioOAuthTranscriptionAliasesPreserveAudioAndScope(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/backend-api/transcribe" || r.Header.Get("Authorization") != "Bearer oauth-secret" || r.Header.Get("Chatgpt-Account-Id") != "chatgpt-account" {
			t.Errorf("wrong upstream binding: path=%s headers=%v", r.URL.Path, r.Header)
		}
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Error(err)
			return
		}
		defer r.MultipartForm.RemoveAll()
		file, header, err := r.FormFile("file")
		if err != nil {
			t.Error(err)
			return
		}
		defer file.Close()
		data, _ := io.ReadAll(file)
		if !bytes.Equal(data, testAudio) || header.Filename != "voice.wav" || r.FormValue("language") != "zh" {
			t.Errorf("audio or language changed: %v %s %s", data, header.Filename, r.FormValue("language"))
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"text":"你好"}`)
	}))
	defer upstream.Close()
	s, spec := audioServerFixture(t, upstream.URL, true)
	router := s.router()
	for _, path := range []string{"/transcribe", "/v1/transcribe", "/backend-api/transcribe", "/backend-api/codex/transcribe", "/v1/audio/transcriptions", "/audio/transcriptions"} {
		body, ct := audioMultipart(t, map[string]string{"model": "whisper-1", "language": "zh"})
		w := serveAudio(t, router, path, body, ct, spec.Key)
		if w.Code != 200 || !strings.Contains(w.Body.String(), "你好") {
			t.Fatalf("%s: %d %s", path, w.Code, w.Body.String())
		}
	}
	// Changing scope must not cause fallback to an unrelated loaded credential.
	spec.AccountIDs = []string{"missing-account"}
	body, ct := audioMultipart(t, nil)
	w := serveAudio(t, router, "/v1/transcribe", body, ct, spec.Key)
	if w.Code == http.StatusOK {
		t.Fatal("out-of-scope OAuth account was used")
	}
}

func TestAudioOAuthFormatsAndUnsupportedOptions(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Error(err)
			return
		}
		defer r.MultipartForm.RemoveAll()
		if r.FormValue("model") != "" || r.FormValue("response_format") != "" {
			t.Error("public API control fields leaked to the backend")
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"text":"hello","extra":true}`)
	}))
	defer upstream.Close()
	s, spec := audioServerFixture(t, upstream.URL, true)
	router := s.router()
	body, ct := audioMultipart(t, map[string]string{"model": "whisper-1", "response_format": "text"})
	w := serveAudio(t, router, "/audio/transcriptions", body, ct, spec.Key)
	if w.Code != 200 || w.Body.String() != "hello" || !strings.HasPrefix(w.Header().Get("Content-Type"), "text/plain") {
		t.Fatalf("text conversion: %d %s %v", w.Code, w.Body.String(), w.Header())
	}
	for _, fields := range []map[string]string{{"response_format": "srt"}, {"stream": "true"}, {"prompt": "unsupported hint"}} {
		body, ct = audioMultipart(t, fields)
		w = serveAudio(t, router, "/audio/transcriptions", body, ct, spec.Key)
		if w.Code != 400 || !strings.Contains(w.Body.String(), "audio_invalid_request") {
			t.Fatalf("unsupported options accepted: %v %d %s", fields, w.Code, w.Body.String())
		}
	}
	if calls.Load() != 1 {
		t.Fatalf("invalid requests reached upstream: %d", calls.Load())
	}
}

func TestAudioProviderNativeTranscriptionAndSpeech(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.Header.Get("Authorization") != "Bearer provider-secret" || r.Header.Get("Chatgpt-Account-Id") != "" {
			t.Error("wrong provider authentication")
		}
		switch r.URL.Path {
		case "/v1/audio/transcriptions":
			if err := r.ParseMultipartForm(1 << 20); err != nil {
				t.Error(err)
				return
			}
			defer r.MultipartForm.RemoveAll()
			if r.FormValue("model") != defaultTranscriptionModel || r.FormValue("language") != "en" {
				t.Error("native dictation was not adapted to the public API")
			}
			_, _ = io.WriteString(w, `{"text":"hello"}`)
		case "/v1/audio/speech":
			payload, _ := io.ReadAll(r.Body)
			if !bytes.Contains(payload, []byte(`"model":"gpt-4o-mini-tts"`)) {
				t.Errorf("speech body changed: %s", payload)
			}
			w.Header().Set("Content-Type", "audio/mpeg")
			_, _ = w.Write(testAudio)
		default:
			t.Errorf("wrong provider path: %s", r.URL.Path)
		}
	}))
	defer upstream.Close()
	s, spec := audioServerFixture(t, upstream.URL, false)
	router := s.router()
	body, ct := audioMultipart(t, map[string]string{"language": "en"})
	w := serveAudio(t, router, "/v1/transcribe", body, ct, spec.Key)
	if w.Code != 200 {
		t.Fatalf("native dictation: %d %s", w.Code, w.Body.String())
	}
	w = serveAudio(t, router, "/v1/audio/speech", []byte(`{"model":"gpt-4o-mini-tts","input":"hello","voice":"alloy"}`), "application/json", spec.Key)
	if w.Code != 200 || !bytes.Equal(w.Body.Bytes(), testAudio) || w.Header().Get("Content-Type") != "audio/mpeg" || calls.Load() != 2 {
		t.Fatalf("binary speech: %d %v %v calls=%d", w.Code, w.Body.Bytes(), w.Header(), calls.Load())
	}
}

func TestAudioValidationAndAuthorization(t *testing.T) {
	s, spec := audioServerFixture(t, "http://127.0.0.1:1", false)
	router := s.router()
	for _, path := range []string{"/transcribe", "/v1/audio/transcriptions", "/audio/translations", "/v1/audio/speech"} {
		w := serveAudio(t, router, path, nil, "application/json", "")
		if w.Code != 401 {
			t.Fatalf("%s bypassed authorization: %d", path, w.Code)
		}
	}
	w := serveAudio(t, router, "/v1/audio/transcriptions", []byte(`{}`), "application/json", spec.Key)
	if w.Code != 400 {
		t.Fatalf("bad multipart: %d", w.Code)
	}
	w = serveAudio(t, router, "/v1/audio/transcriptions", bytes.Repeat([]byte("a"), maxAudioRequestBytes+1), "multipart/form-data; boundary=large", spec.Key)
	if w.Code != 413 {
		t.Fatalf("oversized upload: %d", w.Code)
	}
	body, ct := audioMultipart(t, nil)
	w = serveAudio(t, router, "/v1/audio/transcriptions", body, ct, spec.Key)
	if w.Code != 400 || !strings.Contains(w.Body.String(), "model_required") {
		t.Fatalf("public API accepted a missing model: %d %s", w.Code, w.Body.String())
	}
}

func TestAudioProviderRetryAfter(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		w.Header().Set("Retry-After", "60")
		w.WriteHeader(http.StatusTooManyRequests)
		_, _ = io.WriteString(w, `{"error":{"message":"quota"}}`)
	}))
	defer upstream.Close()
	s, spec := audioServerFixture(t, upstream.URL, false)
	router := s.router()
	body, ct := audioMultipart(t, map[string]string{"model": "whisper-1"})
	for i := 0; i < 2; i++ {
		w := serveAudio(t, router, "/audio/transcriptions", body, ct, spec.Key)
		if w.Code != 429 || w.Header().Get("Retry-After") == "" {
			t.Fatalf("Retry-After ignored: %d %s", w.Code, w.Body.String())
		}
	}
	if calls.Load() != 1 {
		t.Fatalf("request sent during backoff: %d", calls.Load())
	}
}

func TestAudioProviderStreamingAndCancellation(t *testing.T) {
	canceled := make(chan struct{})
	finished := make(chan struct{})
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, "data: {\"delta\":\"hello\"}\n\n")
		w.(http.Flusher).Flush()
		select {
		case <-r.Context().Done():
			close(canceled)
		case <-time.After(5 * time.Second):
			t.Error("upstream was not canceled")
		}
	}))
	defer upstream.Close()
	s, spec := audioServerFixture(t, upstream.URL, false)
	s.manifest.MaxAccountConcurrency = 1
	router := s.router()
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("X-Test-Audio-Stream") == "1" {
			defer close(finished)
		}
		router.ServeHTTP(w, r)
	}))
	defer proxy.Close()
	body, ct := audioMultipart(t, map[string]string{"model": "gpt-4o-transcribe", "stream": "true"})
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, http.MethodPost, proxy.URL+"/audio/transcriptions", bytes.NewReader(body))
	req.Header.Set("Content-Type", ct)
	req.Header.Set("Authorization", "Bearer "+spec.Key)
	req.Header.Set("X-Test-Audio-Stream", "1")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	chunk := make([]byte, len("data: {\"delta\":\"hello\"}\n\n"))
	if _, err := io.ReadFull(resp.Body, chunk); err != nil || !strings.Contains(string(chunk), "hello") {
		t.Fatalf("SSE was buffered: %s %v", chunk, err)
	}
	busy := serveAudio(t, router, "/audio/transcriptions", body, ct, spec.Key)
	if busy.Code != http.StatusTooManyRequests {
		t.Fatalf("stream released its concurrency slot early: %d %s", busy.Code, busy.Body.String())
	}
	cancel()
	_ = resp.Body.Close()
	select {
	case <-canceled:
	case <-time.After(3 * time.Second):
		t.Fatal("client cancellation did not reach the audio upstream")
	}
	select {
	case <-finished:
	case <-time.After(3 * time.Second):
		t.Fatal("canceled handler did not return")
	}
	s.policy.tracker.mu.Lock()
	slots := len(s.policy.tracker.accountSlots)
	s.policy.tracker.mu.Unlock()
	if slots != 0 {
		t.Fatalf("cancellation leaked %d account slots", slots)
	}
}

func TestAudioStoredAPIKeyAndProviderTranslation(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/audio/translations" || r.Header.Get("Authorization") != "Bearer stored-secret" || r.Header.Get("Chatgpt-Account-Id") != "" {
			t.Errorf("stored API key was sent to the wrong endpoint: %s %v", r.URL.Path, r.Header)
		}
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Error(err)
			return
		}
		defer r.MultipartForm.RemoveAll()
		if r.FormValue("model") != "whisper-1" || r.FormValue("response_format") != "srt" || r.FormValue("prompt") != "context" {
			t.Error("provider options changed")
		}
		w.Header().Set("Content-Type", "text/plain")
		_, _ = io.WriteString(w, "1\n00:00:00,000 --> 00:00:01,000\nhello\n")
	}))
	defer upstream.Close()
	s, spec := audioServerFixture(t, upstream.URL, true)
	credential, _ := s.authManager.GetByID("voice-auth")
	credential.Attributes = map[string]string{coreauth.AttributeAuthKind: coreauth.AuthKindAPIKey, coreauth.AttributeAPIKey: "stored-secret", "base_url": upstream.URL + "/v1"}
	credential.Metadata = nil
	if _, err := s.authManager.Update(context.Background(), credential); err != nil {
		t.Fatal(err)
	}
	body, ct := audioMultipart(t, map[string]string{"model": "whisper-1", "response_format": "srt", "prompt": "context"})
	w := serveAudio(t, s.router(), "/audio/translations", body, ct, spec.Key)
	if w.Code != 200 || !strings.Contains(w.Body.String(), "00:00:00,000") {
		t.Fatalf("subtitle forwarding: %d %s", w.Code, w.Body.String())
	}
}

func TestAudioExplicitProviderRouteDoesNotFallback(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if err := r.ParseMultipartForm(1 << 20); err != nil {
			t.Error(err)
			return
		}
		defer r.MultipartForm.RemoveAll()
		if r.FormValue("model") != "whisper-1" || len(r.MultipartForm.Value["model"]) != 1 {
			t.Error("route model was not rewritten exactly once")
		}
		_, _ = io.WriteString(w, `{"text":"routed"}`)
	}))
	defer upstream.Close()
	s, spec := audioServerFixture(t, upstream.URL, false)
	gateway := spec.ProviderGateway
	spec.ProviderGateway = nil
	spec.ModelRouting = &modelRoutingSpec{Routes: []modelRouteSpec{{Namespace: "voice", ProviderAccountID: "voice-account", ProviderGateway: gateway}}}
	gateway.UpstreamModels = []string{"whisper-1"}
	router := s.router()
	body, ct := audioMultipart(t, map[string]string{"model": "voice/whisper-1"})
	w := serveAudio(t, router, "/v1/audio/transcriptions", body, ct, spec.Key)
	if w.Code != 200 {
		t.Fatalf("explicit route: %d %s", w.Code, w.Body.String())
	}
	body, ct = audioMultipart(t, map[string]string{"model": "missing/whisper-1"})
	w = serveAudio(t, router, "/v1/audio/transcriptions", body, ct, spec.Key)
	if w.Code != 400 || calls.Load() != 1 {
		t.Fatalf("unknown route fell back: %d calls=%d", w.Code, calls.Load())
	}
}

func TestAudioLocalizedError(t *testing.T) {
	s, spec := audioServerFixture(t, "http://127.0.0.1:1", false)
	s.manifest.Locale = "zh-CN"
	s.manifest.GatewayErrorMessages["audio_invalid_request"]["zh-cn"] = "音频请求无效：{{detail}}"
	w := serveAudio(t, s.router(), "/v1/audio/transcriptions", []byte(`{}`), "application/json", spec.Key)
	if w.Code != 400 || !strings.Contains(w.Body.String(), "音频请求无效：multipart_required") {
		t.Fatalf("localized error missing: %d %s", w.Code, w.Body.String())
	}
}

func TestAudioOAuthMalformedResponseIsNotSuccess(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.WriteString(w, `{"unexpected":true}`)
	}))
	defer upstream.Close()
	s, spec := audioServerFixture(t, upstream.URL, true)
	body, ct := audioMultipart(t, nil)
	w := serveAudio(t, s.router(), "/audio/transcriptions", body, ct, spec.Key)
	var response map[string]any
	_ = json.Unmarshal(w.Body.Bytes(), &response)
	if w.Code != 502 || response["error"] == nil {
		t.Fatalf("malformed transcript reported success: %d %s", w.Code, w.Body.String())
	}
}
