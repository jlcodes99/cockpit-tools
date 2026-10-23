package helps

import (
	"bytes"
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestCodexEncryptedRecoveryResumesTTFTAtSuccessfulBody(t *testing.T) {
	ctx := context.Background()
	body := []byte(`{"input":[{"type":"reasoning","encrypted_content":"bad","summary":[{"text":"keep"}]}]}`)
	var attempts atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if attempts.Add(1) == 1 {
			w.WriteHeader(400)
			_, _ = io.WriteString(w, `{"error":{"code":"invalid_encrypted_content"}}`)
			return
		}
		_, _ = io.WriteString(w, "success")
	}))
	defer server.Close()
	req, _ := http.NewRequestWithContext(ctx, http.MethodPost, server.URL, bytes.NewReader(body))
	recovery := NewCodexHTTPEncryptedRecovery(&cliproxyauth.Auth{Metadata: map[string]any{"access_token": "test"}}, false, req, body)
	reporter := NewUsageReporter(ctx, "codex", "test", nil)
	client := reporter.TrackHTTPClient(server.Client())
	resp, _, err := recovery.Do(ctx, nil, client, req, body, func(int, []byte) error {
		if !reporter.IsTTFTSet() {
			t.Fatal("rejection did not provide first-byte measurement")
		}
		start := reporter.ttftFirstStart
		rejectedTTFT := reporter.ttft
		reporter.ResumeResponseTTFTAfterRetry()
		if reporter.IsTTFTSet() || !reporter.ttftStart.Equal(start) || reporter.ttftDuration() != rejectedTTFT {
			t.Fatal("retry lost original start or error fallback")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if reporter.IsTTFTSet() {
		t.Fatal("effective TTFT set before success body read")
	}
	_, err = io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if !reporter.IsTTFTSet() || reporter.ttftDuration() <= reporter.firstPacketDuration || attempts.Load() != 2 {
		t.Fatalf("TTFT=%v rejection=%v attempts=%d", reporter.ttftDuration(), reporter.firstPacketDuration, attempts.Load())
	}
}

func TestCodexEncryptedRecoveryOrdinaryFailuresKeepTTFT(t *testing.T) {
	for _, status := range []int{400, 500} {
		t.Run(http.StatusText(status), func(t *testing.T) {
			ctx := context.Background()
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(status)
				_, _ = io.WriteString(w, `{"error":{"code":"ordinary_error"}}`)
			}))
			defer server.Close()
			body := []byte(`{"input":[]}`)
			req, _ := http.NewRequestWithContext(ctx, http.MethodPost, server.URL, bytes.NewReader(body))
			recovery := NewCodexHTTPEncryptedRecovery(&cliproxyauth.Auth{Metadata: map[string]any{"access_token": "test"}}, false, req, body)
			reporter := NewUsageReporter(ctx, "codex", "test", nil)
			resp, _, err := recovery.Do(ctx, nil, reporter.TrackHTTPClient(server.Client()), req, body, func(int, []byte) error { t.Fatal("ordinary error selected retry"); return nil })
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			_, err = io.ReadAll(resp.Body)
			if err != nil {
				t.Fatal(err)
			}
			if !reporter.IsTTFTSet() || reporter.ttftDuration() <= 0 || !reporter.ttftStart.IsZero() || reporter.IsFirstPacketSet() {
				t.Fatal("ordinary failure TTFT changed")
			}
		})
	}
}
