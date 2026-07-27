package executor

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha512"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"golang.org/x/crypto/curve25519"
	"golang.org/x/crypto/nacl/box"
)

func newCodexAgentIdentityTestAuth(t *testing.T, id string) (*cliproxyauth.Auth, ed25519.PrivateKey) {
	t.Helper()
	_, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatalf("generate ed25519 key: %v", err)
	}
	der, err := x509.MarshalPKCS8PrivateKey(privateKey)
	if err != nil {
		t.Fatalf("marshal PKCS#8 key: %v", err)
	}
	return &cliproxyauth.Auth{
		ID:       id,
		Provider: "codex",
		Metadata: map[string]any{
			"auth_mode":         codexAgentIdentityAuthMode,
			"agent_runtime_id":  "runtime-test",
			"agent_private_key": base64.StdEncoding.EncodeToString(der),
			"task_id":           "task-old",
		},
	}, privateKey
}

func decodeCodexAgentAssertionForTest(t *testing.T, header string) map[string]string {
	t.Helper()
	if !strings.HasPrefix(header, "AgentAssertion ") {
		t.Fatalf("unexpected authorization scheme: %q", header)
	}
	encoded := strings.TrimPrefix(header, "AgentAssertion ")
	payload, err := base64.RawURLEncoding.DecodeString(encoded)
	if err != nil {
		t.Fatalf("decode assertion: %v", err)
	}
	var envelope map[string]string
	if err := json.Unmarshal(payload, &envelope); err != nil {
		t.Fatalf("parse assertion: %v", err)
	}
	return envelope
}

func TestBuildCodexAgentAssertionMatchesOfficialEnvelope(t *testing.T) {
	auth, privateKey := newCodexAgentIdentityTestAuth(t, "agent-assertion")
	credential, err := loadCodexAgentIdentityCredential(auth)
	if err != nil {
		t.Fatalf("load credential: %v", err)
	}
	now := time.Date(2026, 7, 21, 10, 11, 12, 0, time.UTC)
	header, err := buildCodexAgentAssertion(credential, now)
	if err != nil {
		t.Fatalf("build assertion: %v", err)
	}
	envelope := decodeCodexAgentAssertionForTest(t, header)
	if envelope["agent_runtime_id"] != "runtime-test" || envelope["task_id"] != "task-old" || envelope["timestamp"] != "2026-07-21T10:11:12Z" {
		t.Fatalf("unexpected assertion envelope: %#v", envelope)
	}
	signature, err := base64.StdEncoding.DecodeString(envelope["signature"])
	if err != nil {
		t.Fatalf("decode signature: %v", err)
	}
	payload := []byte("runtime-test:task-old:2026-07-21T10:11:12Z")
	if !ed25519.Verify(privateKey.Public().(ed25519.PublicKey), payload, signature) {
		t.Fatal("assertion signature is invalid")
	}
}

func TestCodexAgentIdentityHTTPRecoversInvalidTaskOnce(t *testing.T) {
	auth, _ := newCodexAgentIdentityTestAuth(t, "agent-recovery")
	var responseCalls atomic.Int32
	var registerCalls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/agent/runtime-test/task/register":
			registerCalls.Add(1)
			_, _ = io.WriteString(w, `{"task_id":"task-new"}`)
		case "/responses":
			call := responseCalls.Add(1)
			envelope := decodeCodexAgentAssertionForTest(t, r.Header.Get("Authorization"))
			if call == 1 {
				if envelope["task_id"] != "task-old" {
					t.Fatalf("first task id = %q", envelope["task_id"])
				}
				w.WriteHeader(http.StatusUnauthorized)
				_, _ = io.WriteString(w, `{"error":{"code":"invalid_task_id"}}`)
				return
			}
			if envelope["task_id"] != "task-new" {
				t.Fatalf("retry task id = %q", envelope["task_id"])
			}
			_, _ = io.WriteString(w, `{"ok":true}`)
		default:
			http.NotFound(w, r)
		}
	}))
	defer server.Close()

	originalBaseURL := codexAgentIdentityAuthAPIBaseURL
	codexAgentIdentityAuthAPIBaseURL = server.URL
	t.Cleanup(func() { codexAgentIdentityAuthAPIBaseURL = originalBaseURL })
	client := &http.Client{Transport: &codexAgentIdentityRoundTripper{base: http.DefaultTransport, auth: auth}}
	req, err := http.NewRequestWithContext(context.Background(), http.MethodPost, server.URL+"/responses", strings.NewReader(`{"model":"gpt-5.4"}`))
	if err != nil {
		t.Fatalf("build request: %v", err)
	}
	resp, err := client.Do(req)
	if err != nil {
		t.Fatalf("request failed: %v", err)
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status = %d", resp.StatusCode)
	}
	if responseCalls.Load() != 2 || registerCalls.Load() != 1 {
		t.Fatalf("responses=%d registrations=%d", responseCalls.Load(), registerCalls.Load())
	}
}

func TestRegisterCodexAgentIdentityTaskAcceptsEncryptedTaskID(t *testing.T) {
	auth, _ := newCodexAgentIdentityTestAuth(t, "agent-encrypted-task")
	credential, err := loadCodexAgentIdentityCredential(auth)
	if err != nil {
		t.Fatalf("load credential: %v", err)
	}
	digest := sha512.Sum512(credential.privateKey.Seed())
	var curvePrivate [32]byte
	copy(curvePrivate[:], digest[:32])
	curvePrivate[0] &= 248
	curvePrivate[31] &= 127
	curvePrivate[31] |= 64
	curvePublicBytes, err := curve25519.X25519(curvePrivate[:], curve25519.Basepoint)
	if err != nil {
		t.Fatalf("derive curve public key: %v", err)
	}
	var curvePublic [32]byte
	copy(curvePublic[:], curvePublicBytes)
	ciphertext, err := box.SealAnonymous(nil, []byte("task-encrypted"), &curvePublic, rand.Reader)
	if err != nil {
		t.Fatalf("seal task id: %v", err)
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(map[string]string{
			"encrypted_task_id": base64.StdEncoding.EncodeToString(ciphertext),
		})
	}))
	defer server.Close()
	originalBaseURL := codexAgentIdentityAuthAPIBaseURL
	codexAgentIdentityAuthAPIBaseURL = server.URL
	t.Cleanup(func() { codexAgentIdentityAuthAPIBaseURL = originalBaseURL })

	taskID, err := registerCodexAgentIdentityTask(context.Background(), http.DefaultTransport, credential)
	if err != nil {
		t.Fatalf("register encrypted task: %v", err)
	}
	if taskID != "task-encrypted" {
		t.Fatalf("task id = %q", taskID)
	}
}

func TestPersistCodexAgentIdentityTaskWritesMatchingAuthFile(t *testing.T) {
	auth, _ := newCodexAgentIdentityTestAuth(t, "agent-persist")
	credential, err := loadCodexAgentIdentityCredential(auth)
	if err != nil {
		t.Fatalf("load credential: %v", err)
	}
	path := filepath.Join(t.TempDir(), "agent-persist.json")
	payload := map[string]any{
		"auth_mode":         codexAgentIdentityAuthMode,
		"agent_runtime_id":  auth.Metadata["agent_runtime_id"],
		"agent_private_key": auth.Metadata["agent_private_key"],
		"task_id":           "task-old",
	}
	encoded, err := json.Marshal(payload)
	if err != nil {
		t.Fatalf("marshal auth file: %v", err)
	}
	if err = os.WriteFile(path, encoded, 0o600); err != nil {
		t.Fatalf("write auth file: %v", err)
	}
	auth.Attributes = map[string]string{"path": path}
	if err = persistCodexAgentIdentityTask(auth, credential, "task-new"); err != nil {
		t.Fatalf("persist task: %v", err)
	}
	updated, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read updated auth file: %v", err)
	}
	var updatedPayload map[string]any
	if err = json.Unmarshal(updated, &updatedPayload); err != nil {
		t.Fatalf("parse updated auth file: %v", err)
	}
	if updatedPayload["task_id"] != "task-new" {
		t.Fatalf("task_id = %#v", updatedPayload["task_id"])
	}
	if info, statErr := os.Stat(path); statErr != nil {
		t.Fatalf("stat auth file: %v", statErr)
	} else if info.Mode().Perm()&0o077 != 0 {
		t.Fatalf("auth file permissions = %o", info.Mode().Perm())
	}
}

func TestRedactCodexAgentIdentitySensitiveBodyUsesRecoveredTask(t *testing.T) {
	auth, _ := newCodexAgentIdentityTestAuth(t, "agent-redaction")
	runtimeState, err := codexAgentIdentityRuntimeFor(auth)
	if err != nil {
		t.Fatalf("load runtime: %v", err)
	}
	runtimeState.mu.Lock()
	runtimeState.credential.taskID = "task-recovered"
	runtimeState.mu.Unlock()
	body := []byte(`{"old":"task-old","new":"task-recovered","auth":"AgentAssertion secret-one","again":"AgentAssertion secret-two"}`)
	redacted := string(redactCodexAgentIdentitySensitiveBody(auth, body))
	for _, leaked := range []string{"task-old", "task-recovered", "secret-one", "secret-two"} {
		if strings.Contains(redacted, leaked) {
			t.Fatalf("redacted body leaked %q: %s", leaked, redacted)
		}
	}
}
