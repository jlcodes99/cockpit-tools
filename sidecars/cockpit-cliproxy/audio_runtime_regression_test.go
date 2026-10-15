package main

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestAudioLateFailuresPreserveNewCredentials(t *testing.T) {
	for _, reRegister := range []bool{false, true} {
		for _, status := range []int{http.StatusUnauthorized, http.StatusForbidden, http.StatusTooManyRequests} {
			t.Run(fmt.Sprintf("register=%v/status=%d", reRegister, status), func(t *testing.T) {
				started, release := make(chan struct{}), make(chan struct{})
				var once sync.Once
				unblock := func() { once.Do(func() { close(release) }) }
				var calls atomic.Int32
				upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					if calls.Add(1) == 1 {
						close(started)
						select {
						case <-release:
						case <-r.Context().Done():
							return
						}
						w.Header().Set("Retry-After", "60")
						w.WriteHeader(status)
						_, _ = io.WriteString(w, `{"error":{"message":"old token rejected"}}`)
						return
					}
					_, _ = io.WriteString(w, `{"text":"new credential works"}`)
				}))
				defer upstream.Close()
				defer unblock()
				s, spec := audioServerFixture(t, upstream.URL, true)
				router := s.router()
				body, ct := audioMultipart(t, map[string]string{"model": "whisper-1"})
				done := make(chan *httptest.ResponseRecorder, 1)
				go func() { done <- serveAudio(t, router, "/v1/audio/transcriptions", body, ct, spec.Key) }()
				select {
				case <-started:
				case <-time.After(5 * time.Second):
					t.Fatal("audio request never reached upstream")
				}
				fresh, _ := s.authManager.GetByID("voice-auth")
				fresh.Metadata["access_token"] = "reauthorized-token"
				var err error
				if reRegister {
					fresh, err = s.authManager.Register(context.Background(), fresh)
				} else {
					fresh, err = s.authManager.Update(context.Background(), fresh)
				}
				if err != nil {
					t.Fatal(err)
				}
				unblock()
				select {
				case response := <-done:
					if response.Code != status {
						t.Fatalf("old request response changed: %d", response.Code)
					}
				case <-time.After(5 * time.Second):
					t.Fatal("old audio request did not finish")
				}
				latest, _ := s.authManager.GetByID("voice-auth")
				if latest.LastError != nil || latest.Unavailable || len(latest.ModelStates) > 0 ||
					latest.CredentialVersion != fresh.CredentialVersion || latest.RegistrationEpoch != fresh.RegistrationEpoch {
					t.Fatalf("late audio failure changed new credentials: %+v", latest)
				}
				response := serveAudio(t, router, "/v1/audio/transcriptions", body, ct, spec.Key)
				if response.Code != http.StatusOK || calls.Load() != 2 {
					t.Fatalf("old audio backoff blocked new credentials: status=%d calls=%d", response.Code, calls.Load())
				}
			})
		}
	}
}

func TestAudioCurrentFailuresStillRecordAccountState(t *testing.T) {
	for _, status := range []int{http.StatusUnauthorized, http.StatusForbidden, http.StatusTooManyRequests} {
		t.Run(fmt.Sprint(status), func(t *testing.T) {
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(status)
			}))
			defer upstream.Close()
			s, spec := audioServerFixture(t, upstream.URL, true)
			body, ct := audioMultipart(t, nil)
			response := serveAudio(t, s.router(), "/v1/transcribe", body, ct, spec.Key)
			latest, _ := s.authManager.GetByID("voice-auth")
			if response.Code != status || latest.LastError == nil || !latest.Unavailable || latest.ModelStates["codex-transcribe"] == nil {
				t.Fatalf("current audio failure was ignored: status=%d auth=%+v", response.Code, latest)
			}
		})
	}
}

func mixedAudioServerFixture(t *testing.T, upstream string) (*relayServer, *apiKeySpec) {
	t.Helper()
	s, spec := audioServerFixture(t, upstream, true)
	account := &accountSpec{ID: "speech-account", AuthID: "speech-auth", AuthKind: "api_key"}
	s.manifest.accountByID[account.ID] = account
	s.manifest.accountByAuthID[account.AuthID] = account
	spec.AccountIDs = append(spec.AccountIDs, account.ID)
	_, err := s.authManager.Register(context.Background(), &coreauth.Auth{
		ID: account.AuthID, Provider: "codex",
		Attributes: map[string]string{coreauth.AttributeAuthKind: coreauth.AuthKindAPIKey,
			coreauth.AttributeAPIKey: "speech-secret", "base_url": upstream + "/v1"},
	})
	if err != nil {
		t.Fatal(err)
	}
	return s, spec
}

func TestAudioMixedPoolUsesCapableCredentials(t *testing.T) {
	for _, operation := range []string{"speech", "translations", "transcriptions"} {
		t.Run(operation, func(t *testing.T) {
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				path, credential := "/v1/audio/"+operation, "speech-secret"
				if operation == "transcriptions" {
					path, credential = "/backend-api/transcribe", "oauth-secret"
				}
				if r.URL.Path != path || r.Header.Get("Authorization") != "Bearer "+credential {
					t.Errorf("wrong audio credential or endpoint: %s %s", r.URL.Path, r.Header.Get("Authorization"))
				}
				if operation == "speech" {
					w.Header().Set("Content-Type", "audio/mpeg")
					_, _ = w.Write(testAudio)
				} else {
					_, _ = io.WriteString(w, `{"text":"hello"}`)
				}
			}))
			defer upstream.Close()
			s, spec := mixedAudioServerFixture(t, upstream.URL)
			body, ct := audioMultipart(t, map[string]string{"model": "whisper-1"})
			if operation == "speech" {
				body, ct = []byte(`{"model":"gpt-4o-mini-tts","input":"hello","voice":"alloy"}`), "application/json"
			}
			response := serveAudio(t, s.router(), "/v1/audio/"+operation, body, ct, spec.Key)
			if response.Code != http.StatusOK || (operation == "speech" && !bytes.Equal(response.Body.Bytes(), testAudio)) {
				t.Fatalf("mixed audio pool failed: %d %s", response.Code, response.Body.String())
			}
		})
	}
}

func TestAudioMixedPoolCannotEscapeOAuthBindingOrKeyScope(t *testing.T) {
	for _, boundOAuth := range []bool{false, true} {
		t.Run(fmt.Sprintf("bound=%v", boundOAuth), func(t *testing.T) {
			var calls atomic.Int32
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				calls.Add(1)
			}))
			defer upstream.Close()
			s, spec := mixedAudioServerFixture(t, upstream.URL)
			spec.BoundOAuth = boundOAuth
			if !boundOAuth {
				// The API-key credential stays loaded but is outside this client's scope.
				spec.AccountIDs = []string{"voice-account"}
			}
			response := serveAudio(t, s.router(), "/v1/audio/speech",
				[]byte(`{"model":"gpt-4o-mini-tts","input":"hello","voice":"alloy"}`), "application/json", spec.Key)
			if response.Code != http.StatusBadRequest || !strings.Contains(response.Body.String(), "audio_not_supported") || calls.Load() != 0 {
				t.Fatalf("audio escaped its OAuth binding or scope: status=%d calls=%d", response.Code, calls.Load())
			}
		})
	}
}
