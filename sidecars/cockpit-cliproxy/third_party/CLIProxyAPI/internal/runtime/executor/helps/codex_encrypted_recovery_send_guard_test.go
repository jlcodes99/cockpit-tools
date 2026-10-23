package helps

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestCodexHTTPRecoveryChecksSendGuardForInitialAndRecoveryAttempts(t *testing.T) {
	for _, denyAttempt := range []int{1, 2} {
		var requests atomic.Int32
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
			requests.Add(1)
			w.WriteHeader(http.StatusBadRequest)
			_, _ = io.WriteString(w, `{"error":{"code":"invalid_encrypted_content"}}`)
		}))
		body := []byte(`{"input":[{"type":"reasoning","encrypted_content":"opaque","summary":[]}]}`)
		req, _ := http.NewRequest(http.MethodPost, server.URL, bytes.NewReader(body))
		auth := &cliproxyauth.Auth{ID: "selected-auth", Metadata: map[string]any{"access_token": "synthetic"}}
		rejected := errors.New("quota changed while preparing")
		guards := 0
		ctx := cliproxyexecutor.WithUpstreamSendGuard(context.Background(), func(_ context.Context, authID string) error {
			guards++
			if authID != auth.ID {
				t.Fatalf("authID=%q", authID)
			}
			if guards == denyAttempt {
				return rejected
			}
			return nil
		})
		recovery := NewCodexHTTPEncryptedRecovery(auth, false, req, body)
		response, _, err := recovery.Do(ctx, nil, server.Client(), req, body, nil)
		if response != nil {
			_ = response.Body.Close()
		}
		server.Close()
		if !errors.Is(err, rejected) || guards != denyAttempt || requests.Load() != int32(denyAttempt-1) {
			t.Fatalf("deny=%d guards=%d requests=%d err=%v", denyAttempt, guards, requests.Load(), err)
		}
	}
}
