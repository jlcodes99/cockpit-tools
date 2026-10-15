package executor

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/gorilla/websocket"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

type bootstrapCanceledResultHook struct {
	cliproxyauth.NoopHook
	results chan cliproxyauth.Result
}

func (h *bootstrapCanceledResultHook) OnResult(_ context.Context, result cliproxyauth.Result) {
	h.results <- result
}

func TestCodexManagerObservesQuotaAfterCanceledBootstrap(t *testing.T) {
	for _, replaceCredential := range []bool{false, true} {
		t.Run(map[bool]string{false: "original_credential", true: "replacement_credential"}[replaceCredential], func(t *testing.T) {
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			observerEntered := make(chan struct{})
			observerRelease := make(chan struct{})
			quotaGate := make(chan struct{})
			var releaseOnce sync.Once
			release := func() { releaseOnce.Do(func() { close(observerRelease) }) }
			defer release()
			upgrader := websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				conn, errUpgrade := upgrader.Upgrade(w, r, nil)
				if errUpgrade != nil {
					t.Error(errUpgrade)
					return
				}
				defer conn.Close()
				if _, _, errRead := conn.ReadMessage(); errRead != nil {
					t.Error(errRead)
					return
				}
				if errWrite := conn.WriteMessage(websocket.TextMessage, []byte(codexCreatedEvent)); errWrite != nil {
					t.Error(errWrite)
					return
				}
				select {
				case <-quotaGate:
				case <-ctx.Done():
					return
				}
				if errWrite := conn.WriteMessage(websocket.TextMessage, []byte(`{"type":"error","status":429,"error":{"type":"usage_limit_reached","message":"usage limit reached","resets_in_seconds":600}}`)); errWrite != nil {
					t.Error(errWrite)
				}
			}))
			defer server.Close()
			exec := NewCodexWebsocketsExecutor(codexBufferingConfig(false))
			exec.store = &codexWebsocketSessionStore{sessions: make(map[string]*codexWebsocketSession)}
			sessionID := uuid.NewString()
			defer exec.CloseExecutionSession(sessionID)
			disconnect := exec.UpstreamDisconnectChan(sessionID)
			go func() {
				select {
				case <-disconnect:
					cancel()
				case <-ctx.Done():
				}
			}()
			credential := codexTestAuth(server.URL)
			credential.ID, credential.Provider, credential.Status = uuid.NewString(), "codex", cliproxyauth.StatusActive
			req, opts := codexWebsocketRequest()
			opts.Stream = true
			opts.Metadata = map[string]any{cliproxyexecutor.ExecutionSessionMetadataKey: sessionID, cliproxyexecutor.CallerScopeMetadataKey: "bootstrap-caller"}
			opts.WebSocketResponseObserver = func(_ context.Context, event cliproxyexecutor.WebSocketResponseEvent) {
				if event.EventType == "response.created" {
					close(observerEntered)
					<-observerRelease
				}
			}
			modelRegistry := registry.GetGlobalRegistry()
			modelRegistry.RegisterClient(credential.ID, "codex", []*registry.ModelInfo{{ID: req.Model}})
			defer modelRegistry.UnregisterClient(credential.ID)
			hook := &bootstrapCanceledResultHook{results: make(chan cliproxyauth.Result, 4)}
			manager := cliproxyauth.NewManager(nil, nil, hook)
			manager.SetRetryConfig(0, 0, 0)
			manager.RegisterExecutor(exec)
			original, errRegister := manager.Register(t.Context(), credential)
			if errRegister != nil {
				t.Fatal(errRegister)
			}
			finished := make(chan error, 1)
			go func() {
				_, errStream := manager.ExecuteStream(ctx, []string{"codex"}, req, opts)
				finished <- errStream
			}()
			select {
			case <-observerEntered:
			case <-time.After(3 * time.Second):
				t.Fatal("executor did not reach the first-frame barrier")
			}
			close(quotaGate)
			select {
			case errStream := <-finished:
				if !errors.Is(errStream, context.Canceled) {
					t.Fatalf("bootstrap error = %v, want cancellation", errStream)
				}
			case <-time.After(3 * time.Second):
				t.Fatal("bootstrap waited for the executor's frame delivery")
			}
			if replaceCredential {
				replacement := original.Clone()
				replacement.Attributes["api_key"] = "replacement"
				if _, errUpdate := manager.Update(t.Context(), replacement); errUpdate != nil {
					t.Fatal(errUpdate)
				}
			}
			release()
			var result cliproxyauth.Result
			select {
			case result = <-hook.results:
			case <-time.After(3 * time.Second):
				t.Fatal("confirmed quota was discarded after canceled bootstrap")
			}
			if result.Success || result.RetryAfter == nil || !result.CredentialScope || result.Options.Metadata[cliproxyexecutor.CallerScopeMetadataKey] != "bootstrap-caller" || result.StaleCredential != replaceCredential || result.CredentialVersion != original.CredentialVersion || result.RegistrationEpoch != original.RegistrationEpoch {
				t.Fatalf("incorrect original-attempt result: %#v", result)
			}
			latest, _ := manager.GetByID(credential.ID)
			if replaceCredential {
				if latest.Unavailable || latest.Failed != 0 || latest.Success != 0 {
					t.Fatalf("old quota mutated replacement: %#v", latest)
				}
			} else if !latest.Unavailable || latest.Failed != 1 || latest.Success != 0 || !latest.NextRetryAfter.After(time.Now().Add(5*time.Minute)) {
				t.Fatalf("quota was not recorded exactly once: %#v", latest)
			}
		})
	}
}
