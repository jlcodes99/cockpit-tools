package helps

import (
	"bytes"
	"context"
	"crypto/sha256"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
)

func TestCodexHTTPEncryptedRecoveryTargetsNamedDigest(t *testing.T) {
	body := []byte(`{"input":[{"type":"reasoning","id":"rs_bad","encrypted_content":"bad","summary":[{"text":"keep"}]},{"type":"reasoning","encrypted_content":"good","summary":[]},{"type":"function_call_output","encrypted_content":"bad","output":"keep output"},{"type":"compaction","encrypted_content":"compact"}]}`)
	for _, errbody := range []string{`{"error":{"param":"input[0].encrypted_content"}}`, `{"error":{"message":"The encrypted content bad could not be verified"}}`} {
		next, removed, err := stripCodexRejectedContent(body, rejectedCodexContentDigests(body, []byte(errbody)))
		if err != nil || len(removed) != 1 {
			t.Fatalf("removed=%v err=%v", removed, err)
		}
		if gjson.GetBytes(next, "input.0.id").Exists() || gjson.GetBytes(next, "input.0.encrypted_content").Exists() || gjson.GetBytes(next, "input.0.summary.0.text").String() != "keep" {
			t.Fatalf("bad recovery: %s", next)
		}
		if gjson.GetBytes(next, "input.1.encrypted_content").String() != "good" || gjson.GetBytes(next, "input.2.encrypted_content").String() != "bad" || gjson.GetBytes(next, "input.3.encrypted_content").String() != "compact" {
			t.Fatalf("unrelated content stripped: %s", next)
		}
	}
}
func TestCodexHTTPEncryptedRecoveryLineageBoundsAndExpiration(t *testing.T) {
	codexRejectedContent.Lock()
	saved := codexRejectedContent.entries
	codexRejectedContent.entries = make(map[[32]byte]codexRejectedContentEntry)
	codexRejectedContent.Unlock()
	defer func() {
		codexRejectedContent.Lock()
		codexRejectedContent.entries = saved
		codexRejectedContent.Unlock()
	}()
	var digests [][32]byte
	for i := 0; i < codexRejectedMaxDigests+10; i++ {
		digests = append(digests, sha256.Sum256([]byte{byte(i)}))
	}
	for i := 0; i < codexRejectedMaxSessions+10; i++ {
		r := &CodexHTTPEncryptedRecovery{storeScope: true, scope: sha256.Sum256([]byte{byte(i), byte(i >> 8)})}
		r.remember(digests)
	}
	codexRejectedContent.Lock()
	if len(codexRejectedContent.entries) > codexRejectedMaxSessions {
		t.Fatal("session cap exceeded")
	}
	for _, entry := range codexRejectedContent.entries {
		if len(entry.digests) > codexRejectedMaxDigests {
			t.Fatal("digest cap exceeded")
		}
	}
	scope := sha256.Sum256([]byte("expired"))
	codexRejectedContent.entries[scope] = codexRejectedContentEntry{expires: time.Now().Add(-time.Second), digests: map[[32]byte]struct{}{digests[0]: {}}}
	codexRejectedContent.Unlock()
	r := &CodexHTTPEncryptedRecovery{enabled: true, storeScope: true, scope: scope}
	if len(r.knownDigests()) != 0 {
		t.Fatal("expired lineage survived")
	}
}
func TestCodexHTTPEncryptedRecoveryRestoresNonRecoverableErrorBody(t *testing.T) {
	for _, body := range []string{`{"error":{"code":"thinking_signature_invalid","message":"unrelated"}}`, strings.Repeat("x", codexRecoveryErrorBodyLimit+100)} {
		t.Run(stringLenLabel(body), func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(400); _, _ = io.WriteString(w, body) }))
			defer server.Close()
			req, _ := http.NewRequestWithContext(context.Background(), "POST", server.URL, bytes.NewReader([]byte(`{"input":[]}`)))
			auth := &cliproxyauth.Auth{Metadata: map[string]any{"access_token": "test"}}
			recovery := NewCodexHTTPEncryptedRecovery(auth, false, req, []byte(`{"input":[]}`))
			resp, _, err := recovery.Do(context.Background(), nil, server.Client(), req, []byte(`{"input":[]}`), nil)
			if err != nil {
				t.Fatal(err)
			}
			defer resp.Body.Close()
			actual, err := io.ReadAll(resp.Body)
			if err != nil || string(actual) != body {
				t.Fatalf("response body not preserved: bytes=%d err=%v", len(actual), err)
			}
		})
	}
}
func stringLenLabel(s string) string {
	if len(s) > codexRecoveryErrorBodyLimit {
		return "oversized"
	}
	return "unrelated"
}

func TestCodexHTTPEncryptedRecoveryRetainsCompactionSummaryAndDropsEmptyReasoning(t *testing.T) {
	body := []byte(`{"input":[{"type":"compaction","encrypted_content":"opaque","summary":[{"type":"summary_text","text":"context"}]},{"type":"compaction_summary","encrypted_content":"opaque","summary":"plain context"},{"type":"reasoning","id":"empty","encrypted_content":"opaque","summary":[]},{"type":"message","role":"user","content":"keep"}]}`)
	next, _, err := stripCodexRejectedContent(body, map[[32]byte]struct{}{sha256.Sum256([]byte("opaque")): {}})
	if err != nil {
		t.Fatal(err)
	}
	input := gjson.GetBytes(next, "input").Array()
	if len(input) != 3 || input[0].Get("type").String() != "reasoning" || input[0].Get("summary.0.text").String() != "context" || input[1].Get("summary.0.text").String() != "plain context" || input[2].Get("content").String() != "keep" {
		t.Fatalf("bad summary recovery: %s", next)
	}
	for _, item := range input {
		if item.Get("encrypted_content").Exists() || item.Get("id").Exists() {
			t.Fatalf("opaque content or store lookup survived: %s", item.Raw)
		}
	}
}

func TestCodexHTTPEncryptedRecoveryEndpointScopesIndependent(t *testing.T) {
	auth := &cliproxyauth.Auth{ID: t.Name(), Metadata: map[string]any{"access_token": "test"}}
	body := []byte(`{"model":"gpt-5.4","prompt_cache_key":"same","input":[{"type":"reasoning","encrypted_content":"bad","summary":[{"text":"keep"}]}]}`)
	regular, _ := http.NewRequest("POST", "http://example.test/responses", bytes.NewReader(body))
	compact, _ := http.NewRequest("POST", "http://example.test/responses/compact", bytes.NewReader(body))
	a := NewCodexHTTPEncryptedRecovery(auth, false, regular, body)
	b := NewCodexHTTPEncryptedRecovery(auth, false, compact, body)
	a.remember([][32]byte{sha256.Sum256([]byte("bad"))})
	result, err := b.PrepareRequest(compact, body)
	if err != nil || !bytes.Equal(result, body) || a.scope == b.scope {
		t.Fatalf("endpoint lineage crossed: %s err=%v", result, err)
	}
}
