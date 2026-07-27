package executor

import (
	"bytes"
	"context"
	"crypto"
	"crypto/ed25519"
	"crypto/sha256"
	"crypto/sha512"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"golang.org/x/crypto/curve25519"
	"golang.org/x/crypto/nacl/box"
)

const codexAgentIdentityAuthMode = "agentIdentity"

var codexAgentIdentityAuthAPIBaseURL = "https://auth.openai.com/api/accounts"

type codexAgentIdentityCredential struct {
	runtimeID   string
	privateKey  ed25519.PrivateKey
	taskID      string
	fingerprint string
}

type codexAgentIdentityRuntime struct {
	mu         sync.Mutex
	credential codexAgentIdentityCredential
}

var codexAgentIdentityRuntimes sync.Map

type codexAgentIdentityTaskRegistrationResponse struct {
	TaskID               string `json:"task_id"`
	TaskIDCamel          string `json:"taskId"`
	EncryptedTaskID      string `json:"encrypted_task_id"`
	EncryptedTaskIDCamel string `json:"encryptedTaskId"`
}

func codexAgentIdentityMetadataString(auth *cliproxyauth.Auth, keys ...string) string {
	if auth == nil || auth.Metadata == nil {
		return ""
	}
	for _, key := range keys {
		if value, ok := auth.Metadata[key].(string); ok {
			if value = strings.TrimSpace(value); value != "" {
				return value
			}
		}
	}
	return ""
}

func isCodexAgentIdentityAuth(auth *cliproxyauth.Auth) bool {
	mode := codexAgentIdentityMetadataString(auth, "auth_mode", "openai_auth_mode")
	return strings.EqualFold(mode, codexAgentIdentityAuthMode)
}

func loadCodexAgentIdentityCredential(auth *cliproxyauth.Auth) (codexAgentIdentityCredential, error) {
	if !isCodexAgentIdentityAuth(auth) {
		return codexAgentIdentityCredential{}, errors.New("codex agent identity auth is required")
	}
	runtimeID := codexAgentIdentityMetadataString(auth, "agent_runtime_id", "agentRuntimeId")
	encodedPrivateKey := codexAgentIdentityMetadataString(auth, "agent_private_key", "agentPrivateKey")
	if runtimeID == "" || encodedPrivateKey == "" {
		return codexAgentIdentityCredential{}, errors.New("codex agent identity runtime or private key is missing")
	}
	der, err := base64.StdEncoding.DecodeString(encodedPrivateKey)
	if err != nil {
		return codexAgentIdentityCredential{}, errors.New("codex agent identity private key is not valid base64")
	}
	parsed, err := x509.ParsePKCS8PrivateKey(der)
	if err != nil {
		return codexAgentIdentityCredential{}, errors.New("codex agent identity private key is not valid PKCS#8")
	}
	privateKey, ok := parsed.(ed25519.PrivateKey)
	if !ok || len(privateKey) != ed25519.PrivateKeySize {
		return codexAgentIdentityCredential{}, errors.New("codex agent identity private key is not Ed25519")
	}
	fingerprintBytes := sha256.Sum256([]byte(runtimeID + "\x00" + encodedPrivateKey))
	return codexAgentIdentityCredential{
		runtimeID:   runtimeID,
		privateKey:  privateKey,
		taskID:      codexAgentIdentityMetadataString(auth, "task_id", "taskId"),
		fingerprint: base64.RawURLEncoding.EncodeToString(fingerprintBytes[:]),
	}, nil
}

func codexAgentIdentityRuntimeFor(auth *cliproxyauth.Auth) (*codexAgentIdentityRuntime, error) {
	credential, err := loadCodexAgentIdentityCredential(auth)
	if err != nil {
		return nil, err
	}
	key := strings.TrimSpace(auth.ID)
	if key == "" {
		key = credential.fingerprint
	}
	candidate := &codexAgentIdentityRuntime{credential: credential}
	actual, _ := codexAgentIdentityRuntimes.LoadOrStore(key, candidate)
	runtimeState, ok := actual.(*codexAgentIdentityRuntime)
	if !ok {
		return nil, errors.New("codex agent identity runtime has invalid type")
	}
	runtimeState.mu.Lock()
	if runtimeState.credential.fingerprint != credential.fingerprint {
		runtimeState.credential = credential
	} else if runtimeState.credential.taskID == "" && credential.taskID != "" {
		runtimeState.credential.taskID = credential.taskID
	}
	runtimeState.mu.Unlock()
	return runtimeState, nil
}

func buildCodexAgentAssertion(credential codexAgentIdentityCredential, now time.Time) (string, error) {
	if credential.runtimeID == "" || credential.taskID == "" {
		return "", errors.New("codex agent identity runtime or task is missing")
	}
	timestamp := now.UTC().Format(time.RFC3339)
	payload := []byte(credential.runtimeID + ":" + credential.taskID + ":" + timestamp)
	signature, err := credential.privateKey.Sign(nil, payload, crypto.Hash(0))
	if err != nil {
		return "", errors.New("failed to sign codex agent assertion")
	}
	envelope, err := json.Marshal(map[string]string{
		"agent_runtime_id": credential.runtimeID,
		"task_id":          credential.taskID,
		"timestamp":        timestamp,
		"signature":        base64.StdEncoding.EncodeToString(signature),
	})
	if err != nil {
		return "", errors.New("failed to serialize codex agent assertion")
	}
	return "AgentAssertion " + base64.RawURLEncoding.EncodeToString(envelope), nil
}

func decryptCodexAgentIdentityTaskID(credential codexAgentIdentityCredential, encoded string) (string, error) {
	ciphertext, err := base64.StdEncoding.DecodeString(strings.TrimSpace(encoded))
	if err != nil {
		return "", errors.New("encrypted codex agent task id is not valid base64")
	}
	digest := sha512.Sum512(credential.privateKey.Seed())
	var curvePrivate [32]byte
	copy(curvePrivate[:], digest[:32])
	curvePrivate[0] &= 248
	curvePrivate[31] &= 127
	curvePrivate[31] |= 64
	curvePublicBytes, err := curve25519.X25519(curvePrivate[:], curve25519.Basepoint)
	if err != nil {
		return "", errors.New("failed to derive codex agent identity decryption key")
	}
	var curvePublic [32]byte
	copy(curvePublic[:], curvePublicBytes)
	plaintext, ok := box.OpenAnonymous(nil, ciphertext, &curvePublic, &curvePrivate)
	if !ok {
		return "", errors.New("failed to decrypt codex agent task id")
	}
	taskID := strings.TrimSpace(string(plaintext))
	if taskID == "" {
		return "", errors.New("decrypted codex agent task id is empty")
	}
	return taskID, nil
}

func registerCodexAgentIdentityTask(ctx context.Context, transport http.RoundTripper, credential codexAgentIdentityCredential) (string, error) {
	if transport == nil {
		transport = http.DefaultTransport
	}
	timestamp := time.Now().UTC().Format(time.RFC3339)
	signature, err := credential.privateKey.Sign(nil, []byte(credential.runtimeID+":"+timestamp), crypto.Hash(0))
	if err != nil {
		return "", errors.New("failed to sign codex agent task registration")
	}
	body, err := json.Marshal(map[string]string{
		"timestamp": timestamp,
		"signature": base64.StdEncoding.EncodeToString(signature),
	})
	if err != nil {
		return "", errors.New("failed to serialize codex agent task registration")
	}
	requestCtx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	url := strings.TrimRight(codexAgentIdentityAuthAPIBaseURL, "/") + "/v1/agent/" + credential.runtimeID + "/task/register"
	req, err := http.NewRequestWithContext(requestCtx, http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return "", errors.New("failed to build codex agent task registration request")
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	resp, err := transport.RoundTrip(req)
	if err != nil {
		return "", fmt.Errorf("codex agent task registration failed: %w", err)
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode < http.StatusOK || resp.StatusCode >= http.StatusMultipleChoices {
		return "", fmt.Errorf("codex agent task registration returned status %d", resp.StatusCode)
	}
	var result codexAgentIdentityTaskRegistrationResponse
	if err := json.NewDecoder(io.LimitReader(resp.Body, 64*1024)).Decode(&result); err != nil {
		return "", errors.New("codex agent task registration response is invalid")
	}
	if taskID := strings.TrimSpace(result.TaskID); taskID != "" {
		return taskID, nil
	}
	if taskID := strings.TrimSpace(result.TaskIDCamel); taskID != "" {
		return taskID, nil
	}
	encrypted := strings.TrimSpace(result.EncryptedTaskID)
	if encrypted == "" {
		encrypted = strings.TrimSpace(result.EncryptedTaskIDCamel)
	}
	if encrypted == "" {
		return "", errors.New("codex agent task registration response omitted task id")
	}
	return decryptCodexAgentIdentityTaskID(credential, encrypted)
}

func ensureCodexAgentIdentityAssertion(ctx context.Context, auth *cliproxyauth.Auth, transport http.RoundTripper, expectedTaskID string) (string, error) {
	runtimeState, err := codexAgentIdentityRuntimeFor(auth)
	if err != nil {
		return "", err
	}
	runtimeState.mu.Lock()
	defer runtimeState.mu.Unlock()
	if runtimeState.credential.taskID == "" || (expectedTaskID != "" && runtimeState.credential.taskID == expectedTaskID) {
		taskID, errRegister := registerCodexAgentIdentityTask(ctx, transport, runtimeState.credential)
		if errRegister != nil {
			return "", errRegister
		}
		if errPersist := persistCodexAgentIdentityTask(auth, runtimeState.credential, taskID); errPersist != nil {
			return "", errPersist
		}
		runtimeState.credential.taskID = taskID
		CloseCodexWebsocketSessionsForAuthID(auth.ID, "agent_identity_task_recovered")
	}
	return buildCodexAgentAssertion(runtimeState.credential, time.Now())
}

func isCodexAgentIdentityTaskInvalid(statusCode int, body []byte) bool {
	if statusCode != http.StatusUnauthorized {
		return false
	}
	lower := strings.ToLower(string(body))
	compact := strings.NewReplacer(" ", "", "\t", "", "\r", "", "\n", "").Replace(lower)
	for _, marker := range []string{`"code":"invalid_task_id"`, `"code":"task_not_found"`, `"code":"task_expired"`, `"error":"invalid_task_id"`} {
		if strings.Contains(compact, marker) {
			return true
		}
	}
	for _, marker := range []string{"invalid task_id", "invalid task id", "task_id is invalid", "task id is invalid", "task not found", "task expired", "unknown task_id", "unknown task id"} {
		if strings.Contains(lower, marker) {
			return true
		}
	}
	return false
}

type codexAgentIdentityRoundTripper struct {
	base http.RoundTripper
	auth *cliproxyauth.Auth
}

func persistCodexAgentIdentityTask(auth *cliproxyauth.Auth, credential codexAgentIdentityCredential, taskID string) error {
	taskID = strings.TrimSpace(taskID)
	if auth == nil || taskID == "" || auth.Attributes == nil {
		return nil
	}
	path := strings.TrimSpace(auth.Attributes["path"])
	if path == "" {
		return nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return fmt.Errorf("read codex agent identity auth file: %w", err)
	}
	var payload map[string]any
	if err = json.Unmarshal(data, &payload); err != nil {
		return errors.New("parse codex agent identity auth file failed")
	}
	if runtimeID := firstAgentIdentityMapString(payload, "agent_runtime_id", "agentRuntimeId"); runtimeID != credential.runtimeID {
		return errors.New("codex agent identity runtime changed before task persistence")
	}
	encodedPrivateKey := firstAgentIdentityMapString(payload, "agent_private_key", "agentPrivateKey")
	fingerprintBytes := sha256.Sum256([]byte(credential.runtimeID + "\x00" + encodedPrivateKey))
	if encodedPrivateKey == "" || base64.RawURLEncoding.EncodeToString(fingerprintBytes[:]) != credential.fingerprint {
		return errors.New("codex agent identity private key changed before task persistence")
	}
	payload["task_id"] = taskID
	encoded, err := json.MarshalIndent(payload, "", "  ")
	if err != nil {
		return errors.New("serialize codex agent identity auth file failed")
	}
	encoded = append(encoded, '\n')
	temp, err := os.CreateTemp(filepath.Dir(path), ".agent-identity-*.tmp")
	if err != nil {
		return fmt.Errorf("create codex agent identity auth temp file: %w", err)
	}
	tempPath := temp.Name()
	defer func() { _ = os.Remove(tempPath) }()
	if err = temp.Chmod(0o600); err == nil {
		_, err = temp.Write(encoded)
	}
	if err == nil {
		err = temp.Sync()
	}
	if closeErr := temp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return fmt.Errorf("write codex agent identity auth temp file: %w", err)
	}
	if err = os.Rename(tempPath, path); err != nil {
		return fmt.Errorf("replace codex agent identity auth file: %w", err)
	}
	if err = os.Chmod(path, 0o600); err != nil {
		return fmt.Errorf("harden codex agent identity auth file: %w", err)
	}
	return nil
}

func firstAgentIdentityMapString(payload map[string]any, keys ...string) string {
	for _, key := range keys {
		if value, ok := payload[key].(string); ok {
			if value = strings.TrimSpace(value); value != "" {
				return value
			}
		}
	}
	return ""
}

func redactCodexAgentIdentitySensitiveBody(auth *cliproxyauth.Auth, body []byte) []byte {
	if !isCodexAgentIdentityAuth(auth) || len(body) == 0 {
		return body
	}
	redacted := string(body)
	for _, key := range []string{"agent_private_key", "agent_runtime_id", "task_id", "access_token", "refresh_token", "id_token", "api_key", "session_key", "cookie"} {
		if value := codexAgentIdentityMetadataString(auth, key); value != "" {
			redacted = strings.ReplaceAll(redacted, value, "[redacted]")
		}
	}
	if runtimeState, err := codexAgentIdentityRuntimeFor(auth); err == nil {
		runtimeState.mu.Lock()
		currentTaskID := runtimeState.credential.taskID
		runtimeState.mu.Unlock()
		if currentTaskID != "" {
			redacted = strings.ReplaceAll(redacted, currentTaskID, "[redacted]")
		}
	}
	const prefix = "AgentAssertion "
	for offset := 0; offset < len(redacted); {
		relativeStart := strings.Index(redacted[offset:], prefix)
		if relativeStart < 0 {
			break
		}
		start := offset + relativeStart
		valueStart := start + len(prefix)
		end := valueStart
		for end < len(redacted) && !strings.ContainsRune(" \t\r\n\"',}", rune(redacted[end])) {
			end++
		}
		redacted = redacted[:valueStart] + "[redacted]" + redacted[end:]
		offset = valueStart + len("[redacted]")
	}
	return []byte(redacted)
}

func bufferCodexAgentIdentityErrorResponse(auth *cliproxyauth.Auth, resp *http.Response) ([]byte, error) {
	if resp == nil || resp.StatusCode < http.StatusBadRequest || resp.Body == nil {
		return nil, nil
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	_ = resp.Body.Close()
	redacted := redactCodexAgentIdentitySensitiveBody(auth, body)
	resp.Body = io.NopCloser(bytes.NewReader(redacted))
	resp.ContentLength = int64(len(redacted))
	return body, nil
}

func cloneCodexAgentIdentityRequest(req *http.Request) (*http.Request, error) {
	cloned := req.Clone(req.Context())
	cloned.Header = req.Header.Clone()
	if req.GetBody != nil {
		body, err := req.GetBody()
		if err != nil {
			return nil, err
		}
		cloned.Body = body
	} else if req.Body != nil {
		body, err := io.ReadAll(req.Body)
		if err != nil {
			return nil, err
		}
		_ = req.Body.Close()
		replay := append([]byte(nil), body...)
		req.Body = io.NopCloser(bytes.NewReader(replay))
		req.GetBody = func() (io.ReadCloser, error) {
			return io.NopCloser(bytes.NewReader(replay)), nil
		}
		cloned.Body = io.NopCloser(bytes.NewReader(replay))
		cloned.GetBody = req.GetBody
	}
	return cloned, nil
}

func (t *codexAgentIdentityRoundTripper) RoundTrip(req *http.Request) (*http.Response, error) {
	if t == nil || t.base == nil || !isCodexAgentIdentityAuth(t.auth) {
		return http.DefaultTransport.RoundTrip(req)
	}
	assertion, err := ensureCodexAgentIdentityAssertion(req.Context(), t.auth, t.base, "")
	if err != nil {
		return nil, err
	}
	first, err := cloneCodexAgentIdentityRequest(req)
	if err != nil {
		return nil, err
	}
	first.Header.Set("Authorization", assertion)
	resp, err := t.base.RoundTrip(first)
	if err != nil || resp == nil {
		return resp, err
	}
	body, errRead := bufferCodexAgentIdentityErrorResponse(t.auth, resp)
	if errRead != nil {
		return resp, nil
	}
	if resp.StatusCode != http.StatusUnauthorized || !isCodexAgentIdentityTaskInvalid(resp.StatusCode, body) {
		return resp, nil
	}
	runtimeState, err := codexAgentIdentityRuntimeFor(t.auth)
	if err != nil {
		return nil, err
	}
	runtimeState.mu.Lock()
	expectedTaskID := runtimeState.credential.taskID
	runtimeState.mu.Unlock()
	assertion, err = ensureCodexAgentIdentityAssertion(req.Context(), t.auth, t.base, expectedTaskID)
	if err != nil {
		return nil, err
	}
	retry, err := cloneCodexAgentIdentityRequest(req)
	if err != nil {
		return nil, err
	}
	retry.Header.Set("Authorization", assertion)
	retryResp, retryErr := t.base.RoundTrip(retry)
	if retryErr == nil && retryResp != nil {
		_, _ = bufferCodexAgentIdentityErrorResponse(t.auth, retryResp)
	}
	return retryResp, retryErr
}

func newCodexAuthenticatedHTTPClient(ctx context.Context, cfg *config.Config, auth *cliproxyauth.Auth) *http.Client {
	client := helps.NewUtlsHTTPClient(ctx, cfg, auth, 0)
	if !isCodexAgentIdentityAuth(auth) {
		return client
	}
	base := client.Transport
	if base == nil {
		base = http.DefaultTransport
	}
	client.Transport = &codexAgentIdentityRoundTripper{base: base, auth: auth}
	return client
}

func codexAgentIdentityAssertionForWebsocket(ctx context.Context, cfg *config.Config, auth *cliproxyauth.Auth, expectedTaskID string) (string, error) {
	client := helps.NewUtlsHTTPClient(ctx, cfg, auth, 0)
	transport := client.Transport
	if transport == nil {
		transport = http.DefaultTransport
	}
	return ensureCodexAgentIdentityAssertion(ctx, auth, transport, expectedTaskID)
}
