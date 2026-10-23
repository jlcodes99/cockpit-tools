package helps

import (
	"bytes"
	"context"
	"crypto/sha256"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const (
	codexRejectedContentTTL     = 30 * time.Minute
	codexRejectedMaxSessions    = 256
	codexRejectedMaxDigests     = 64
	codexRecoveryErrorBodyLimit = 1 << 20
)

type codexRejectedContentEntry struct {
	expires time.Time
	digests map[[32]byte]struct{}
}

// Keep only bounded, expiring hashes. Never retain request bodies, tokens, or
// plaintext session identifiers in recovery state.
var codexRejectedContent = struct {
	sync.Mutex
	entries map[[32]byte]codexRejectedContentEntry
}{entries: make(map[[32]byte]codexRejectedContentEntry)}

// IsCodexOAuthAuth excludes API-key accounts even when they use the strict
// Codex protocol or also carry legacy OAuth metadata.
func IsCodexOAuthAuth(auth *cliproxyauth.Auth) bool {
	if auth == nil || strings.TrimSpace(auth.Attributes["api_key"]) != "" || auth.AuthKind() != cliproxyauth.AuthKindOAuth {
		return false
	}
	token, _ := auth.Metadata["access_token"].(string)
	return strings.TrimSpace(token) != ""
}

// IsCodexEncryptedContentRejection deliberately does not classify arbitrary
// signature errors as recoverable: they can mean unrelated protocol failures.
func IsCodexEncryptedContentRejection(status int, body []byte) bool {
	if status != http.StatusBadRequest || !gjson.ValidBytes(body) {
		return false
	}
	code := gjson.GetBytes(body, "error.code").String()
	if code == "invalid_encrypted_content" {
		return true
	}
	message := strings.ToLower(gjson.GetBytes(body, "error.message").String())
	return code == "thinking_signature_invalid" &&
		strings.Contains(message, "the encrypted content") &&
		strings.Contains(message, "could not be verified") &&
		strings.Contains(message, "could not be decrypted or parsed")
}

type CodexHTTPEncryptedRecovery struct {
	enabled    bool
	scope      [32]byte
	storeScope bool
	auth       *cliproxyauth.Auth
}

func NewCodexHTTPEncryptedRecovery(auth *cliproxyauth.Auth, isCompat bool, req *http.Request, body []byte) *CodexHTTPEncryptedRecovery {
	r := &CodexHTTPEncryptedRecovery{enabled: !isCompat && IsCodexOAuthAuth(auth), auth: auth}
	if !r.enabled || req == nil || req.URL == nil || strings.TrimSpace(auth.ID) == "" {
		return r
	}
	session := strings.TrimSpace(req.Header.Get("Session-Id"))
	if session == "" {
		session = strings.TrimSpace(gjson.GetBytes(body, "prompt_cache_key").String())
	}
	if session != "" {
		token, _ := auth.Metadata["access_token"].(string)
		accountID, _ := auth.Metadata["account_id"].(string)
		credentialDigest := sha256.Sum256([]byte(token))
		// Account header overrides can change the wire identity without changing metadata.
		wireCredentialDigest := sha256.Sum256([]byte(req.Header.Get("Authorization") + "\x00" + req.Header.Get("Chatgpt-Account-Id")))
		// A reauthorized account can reuse its ID and session while an older
		// request is still in flight. Scope by the immutable credential snapshot,
		// and keep compact and regular Responses endpoint histories independent.
		identity := fmt.Sprintf("%s\x00%s\x00%d\x00%d\x00%x\x00%x", auth.ID, accountID, auth.RegistrationEpoch, auth.CredentialVersion, credentialDigest, wireCredentialDigest)
		r.scope = sha256.Sum256([]byte(identity + "\x00" + req.URL.Scheme + "://" + req.URL.Host + req.URL.EscapedPath() + "\x00" + gjson.GetBytes(body, "model").String() + "\x00" + session))
		r.storeScope = true
	}
	return r
}

// PrepareRequest removes only previously rejected digests within this exact
// credential/endpoint/model/session. Other credentials and sessions remain untouched.
func (r *CodexHTTPEncryptedRecovery) PrepareRequest(req *http.Request, body []byte) ([]byte, error) {
	if !r.enabled || !r.storeScope {
		return body, nil
	}
	digests := r.knownDigests()
	if len(digests) == 0 {
		return body, nil
	}
	next, _, err := stripCodexRejectedContent(body, digests)
	if err != nil {
		return body, err
	}
	if !bytes.Equal(next, body) {
		resetCodexRecoveryRequest(req, next)
	}
	return next, nil
}

// Do retries a rejected HTTP request once, using the same client, headers,
// credentials and context. It never inspects HTTP 200/SSE events or replays any
// request after downstream output has been delivered.
func (r *CodexHTTPEncryptedRecovery) Do(ctx context.Context, cfg *config.Config, client *http.Client, req *http.Request, body []byte, beforeRetry func(int, []byte) error) (*http.Response, []byte, error) {
	resp, err := DoGuardedUpstreamHTTP(ctx, r.auth, client, req)
	if err != nil || !r.enabled || resp.StatusCode != http.StatusBadRequest {
		return resp, body, err
	}
	// Limit speculative error inspection and restore the exact response on every
	// non-recovery path, including oversized error bodies.
	data, readErr := io.ReadAll(io.LimitReader(resp.Body, codexRecoveryErrorBodyLimit+1))
	resp.Body = &codexRecoveryResponseBody{Reader: io.MultiReader(bytes.NewReader(data), resp.Body), closer: resp.Body}
	if readErr != nil {
		_ = resp.Body.Close()
		return nil, body, fmt.Errorf("read rejected Codex response: %w", readErr)
	}
	if len(data) > codexRecoveryErrorBodyLimit || !IsCodexEncryptedContentRejection(resp.StatusCode, data) {
		return resp, body, nil
	}
	digests := rejectedCodexContentDigests(body, data)
	next, removed, err := stripCodexRejectedContent(body, digests)
	if err != nil {
		_ = resp.Body.Close()
		return nil, body, err
	}
	if len(removed) == 0 {
		return resp, body, nil
	}
	if err = ctx.Err(); err != nil {
		_ = resp.Body.Close()
		return nil, body, err
	}
	if beforeRetry != nil {
		if err = beforeRetry(resp.StatusCode, data); err != nil {
			_ = resp.Body.Close()
			return nil, body, err
		}
	}
	r.remember(removed)
	RecordAPIResponseMetadata(ctx, cfg, resp.StatusCode, resp.Header.Clone())
	AppendAPIResponseChunk(ctx, cfg, data)
	if errClose := resp.Body.Close(); errClose != nil {
		return nil, body, fmt.Errorf("close rejected Codex response: %w", errClose)
	}
	resetCodexRecoveryRequest(req, next)
	authType, authValue := r.auth.AccountInfo()
	transport := ""
	if strings.Contains(req.Header.Get("Accept"), "text/event-stream") {
		transport = "sse"
	}
	RecordAPIRequest(ctx, cfg, UpstreamRequestLog{Transport: transport, URL: req.URL.String(), Method: req.Method, Headers: req.Header.Clone(), Body: next,
		Provider: "codex", AuthID: r.auth.ID, AuthLabel: r.auth.Label, AuthType: authType, AuthValue: authValue})
	LogWithRequestID(ctx).Debug("codex HTTP encrypted content rejected; retrying once with rejected digests removed")
	resp, err = DoGuardedUpstreamHTTP(ctx, r.auth, client, req)
	return resp, next, err
}

type codexRecoveryResponseBody struct {
	io.Reader
	closer io.Closer
}

func (b *codexRecoveryResponseBody) Close() error { return b.closer.Close() }

func resetCodexRecoveryRequest(req *http.Request, body []byte) {
	req.Body = io.NopCloser(bytes.NewReader(body))
	req.ContentLength = int64(len(body))
	req.GetBody = func() (io.ReadCloser, error) { return io.NopCloser(bytes.NewReader(body)), nil }
}

func rejectedCodexContentDigests(body, errorBody []byte) map[[32]byte]struct{} {
	all := make(map[[32]byte]struct{})
	targeted := make(map[[32]byte]struct{})
	param := strings.TrimSpace(gjson.GetBytes(errorBody, "error.param").String())
	namedContent, hasNamedContent := codexRejectedContentName(gjson.GetBytes(errorBody, "error.message").String())
	// A field-level location is authoritative. Unknown/unrelated fields and
	// absent indexes must never turn a specific rejection into a full purge.
	specificParam := param != "" && param != "input"
	for index, item := range gjson.GetBytes(body, "input").Array() {
		if !codexRecoverableEncryptedItem(item) {
			continue
		}
		content := item.Get("encrypted_content").String()
		digest := sha256.Sum256([]byte(content))
		all[digest] = struct{}{}
		if specificParam {
			if param == "input["+strconv.Itoa(index)+"].encrypted_content" || param == "input."+strconv.Itoa(index)+".encrypted_content" {
				targeted[digest] = struct{}{}
			}
		} else if hasNamedContent && content == namedContent {
			targeted[digest] = struct{}{}
		}
	}
	if specificParam || hasNamedContent {
		return targeted
	}
	// Only a recognized rejection with no specific location can use generic recovery.
	return all
}

// Canonical upstream errors may name a ciphertext instead of reporting error.param.
// Keep a named-but-unmatched target distinct from a generic verification failure.
var codexRejectedContentNamePattern = regexp.MustCompile(`(?i)encrypted content\s*:?\s*(.*?)\s+could not be verified`)

func codexRejectedContentName(message string) (string, bool) {
	// RE2 returns captures from the original text. Unicode case folding can
	// change byte lengths, so offsets from a lowercased copy are never safe here.
	match := codexRejectedContentNamePattern.FindStringSubmatch(message)
	if len(match) != 2 {
		return "", false
	}
	name := strings.TrimSpace(match[1])
	if name == "" {
		return "", false
	}
	if len(name) >= 2 && ((name[0] == '\'' && name[len(name)-1] == '\'') || (name[0] == '"' && name[len(name)-1] == '"')) {
		name = name[1 : len(name)-1]
	}
	return name, true
}

func codexRecoverableEncryptedItem(item gjson.Result) bool {
	switch item.Get("type").String() {
	case "reasoning", "compaction", "compaction_summary":
		content := item.Get("encrypted_content")
		return content.Type == gjson.String && content.String() != ""
	default:
		return false
	}
}

func stripCodexRejectedContent(body []byte, digests map[[32]byte]struct{}) ([]byte, [][32]byte, error) {
	input := gjson.GetBytes(body, "input")
	if len(digests) == 0 || !input.IsArray() {
		return body, nil, nil
	}
	items := input.Array()
	kept := make([][]byte, 0, len(items))
	var removed [][32]byte
	for _, item := range items {
		digest := sha256.Sum256([]byte(item.Get("encrypted_content").String()))
		if _, rejected := digests[digest]; !rejected || !codexRecoverableEncryptedItem(item) {
			kept = append(kept, []byte(item.Raw))
			continue
		}
		removed = append(removed, digest)
		// Compaction's opaque payload cannot be replayed after rejection,
		// but a supplied plaintext summary still carries useful context.
		if item.Get("type").String() != "reasoning" {
			summary := item.Get("summary")
			var next []byte
			var err error
			if summary.IsArray() && len(summary.Array()) > 0 {
				next, err = sjson.SetRawBytes([]byte(`{"type":"reasoning"}`), "summary", []byte(summary.Raw))
			} else if summary.Type == gjson.String && strings.TrimSpace(summary.String()) != "" {
				next, err = sjson.SetBytes([]byte(`{"type":"reasoning","summary":[{"type":"summary_text"}]}`), "summary.0.text", summary.String())
			}
			if err != nil {
				return body, nil, err
			}
			if len(next) > 0 {
				kept = append(kept, next)
			}
			continue
		}
		// An empty reasoning item cannot contribute context once its opaque
		// payload is removed; dropping it also avoids strict-schema rejection.
		if len(item.Get("summary").Array()) == 0 && len(item.Get("content").Array()) == 0 {
			continue
		}
		next, err := sjson.DeleteBytes([]byte(item.Raw), "encrypted_content")
		if err != nil {
			return body, nil, err
		}
		// Prevent store=false from turning the stripped reasoning into an ID lookup.
		next, err = sjson.DeleteBytes(next, "id")
		if err != nil {
			return body, nil, err
		}
		kept = append(kept, next)
	}
	if len(removed) == 0 {
		return body, nil, nil
	}
	array := append([]byte{'['}, bytes.Join(kept, []byte{','})...)
	array = append(array, ']')
	next, err := sjson.SetRawBytes(body, "input", array)
	return next, removed, err
}

func (r *CodexHTTPEncryptedRecovery) knownDigests() map[[32]byte]struct{} {
	codexRejectedContent.Lock()
	defer codexRejectedContent.Unlock()
	entry, ok := codexRejectedContent.entries[r.scope]
	if !ok {
		return nil
	}
	if !time.Now().Before(entry.expires) {
		delete(codexRejectedContent.entries, r.scope)
		return nil
	}
	copyDigests := make(map[[32]byte]struct{}, len(entry.digests))
	for digest := range entry.digests {
		copyDigests[digest] = struct{}{}
	}
	return copyDigests
}

func (r *CodexHTTPEncryptedRecovery) remember(digests [][32]byte) {
	if !r.storeScope {
		return
	}
	codexRejectedContent.Lock()
	defer codexRejectedContent.Unlock()
	now := time.Now()
	for key, entry := range codexRejectedContent.entries {
		if !now.Before(entry.expires) {
			delete(codexRejectedContent.entries, key)
		}
	}
	entry, ok := codexRejectedContent.entries[r.scope]
	if !ok {
		if len(codexRejectedContent.entries) >= codexRejectedMaxSessions {
			var oldestKey [32]byte
			var oldest time.Time
			for key, candidate := range codexRejectedContent.entries {
				if oldest.IsZero() || candidate.expires.Before(oldest) {
					oldestKey, oldest = key, candidate.expires
				}
			}
			delete(codexRejectedContent.entries, oldestKey)
		}
		entry.digests = make(map[[32]byte]struct{})
	}
	for _, digest := range digests {
		if len(entry.digests) >= codexRejectedMaxDigests {
			break
		}
		entry.digests[digest] = struct{}{}
	}
	entry.expires = now.Add(codexRejectedContentTTL)
	codexRejectedContent.entries[r.scope] = entry
}
