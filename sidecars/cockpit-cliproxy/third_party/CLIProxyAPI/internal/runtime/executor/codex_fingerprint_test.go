package executor

import (
	"encoding/json"
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
)

func testFingerprintAuth(id, mode string) *cliproxyauth.Auth {
	return &cliproxyauth.Auth{
		ID:       id,
		Provider: "codex",
		Metadata: map[string]any{"codex_fingerprint_mode": mode},
	}
}

func testFingerprintPayload() []byte {
	turn := map[string]any{
		"installation_id":       "client-install",
		"session_id":            "client-session",
		"thread_id":             "client-thread",
		"window_id":             "client-thread:0",
		"turn_id":               "client-turn",
		"parent_thread_id":      "client-parent",
		"forked_from_thread_id": "client-parent",
		"parent_turn_id":        "client-root-turn",
		"root_turn_id":          "client-root-turn",
		"cwd":                   "/Users/alice/work/repo",
		"workspaces": map[string]any{
			"/Users/alice/work/repo": map[string]any{
				"cwd":    "/Users/alice/work/repo",
				"remote": "https://github.com/acme/repo.git",
				"sha":    "0123456789abcdef0123456789abcdef01234567",
				"dirty":  true,
			},
		},
	}
	encoded, err := json.Marshal(turn)
	if err != nil {
		panic(err)
	}
	payload := map[string]any{
		"prompt_cache_key": "client-session",
		"thread_id":        "client-thread",
		"client_metadata": map[string]any{
			"x-codex-installation-id":  "client-install",
			"x-codex-window-id":        "client-thread:0",
			"x-codex-parent-thread-id": "client-parent",
			"x-codex-turn-metadata":    string(encoded),
			"thread_id":                "client-thread",
			"session_id":               "client-session",
		},
	}
	raw, err := json.Marshal(payload)
	if err != nil {
		panic(err)
	}
	return raw
}

func TestCodexFingerprintIsolatesAccounts(t *testing.T) {
	t.Parallel()
	cfg := &config.Config{}
	payload := testFingerprintPayload()
	leftBody, left := applyCodexFingerprintBody(cfg, testFingerprintAuth("auth-a", "session"), payload, payload, codexIdentityConfuseState{})
	rightBody, right := applyCodexFingerprintBody(cfg, testFingerprintAuth("auth-b", "session"), payload, payload, codexIdentityConfuseState{})

	if left.installationID == "" || left.installationID == "client-install" {
		t.Fatalf("left installation was not isolated: %q", left.installationID)
	}
	if left.installationID == right.installationID {
		t.Fatal("installation id leaked across accounts")
	}
	if left.sessionID == right.sessionID || left.threadID == right.threadID {
		t.Fatal("session/thread leaked across accounts")
	}
	if left.parentThreadID == "" || left.parentThreadID == "client-parent" {
		t.Fatalf("parent thread was not remapped: %q", left.parentThreadID)
	}
	if left.parentThreadID == right.parentThreadID {
		t.Fatal("parent thread leaked across accounts")
	}
	if left.parentThreadID != mapFingerprintThreadID("session", "auth-a", left.sessionID, "client-parent") {
		t.Fatalf("parent thread mapping drifted: %q", left.parentThreadID)
	}

	leftMeta := gjson.GetBytes(leftBody, "client_metadata.x-codex-turn-metadata").String()
	rightMeta := gjson.GetBytes(rightBody, "client_metadata.x-codex-turn-metadata").String()
	if gjson.Get(leftMeta, "turn_id").String() == "client-turn" {
		t.Fatal("turn id was not remapped")
	}
	if gjson.Get(leftMeta, "turn_id").String() == gjson.Get(rightMeta, "turn_id").String() {
		t.Fatal("turn id leaked across accounts")
	}
	if gjson.Get(leftMeta, "parent_thread_id").String() != left.parentThreadID {
		t.Fatalf("metadata parent thread = %q, want %q", gjson.Get(leftMeta, "parent_thread_id").String(), left.parentThreadID)
	}
	if gjson.Get(leftMeta, "forked_from_thread_id").String() != left.parentThreadID {
		t.Fatal("forked_from_thread_id is not aligned with parent thread")
	}

	leftRoot := fakeFingerprintWorkspaceRoot("auth-a", "/Users/alice/work/repo")
	rightRoot := fakeFingerprintWorkspaceRoot("auth-b", "/Users/alice/work/repo")
	if leftRoot == rightRoot {
		t.Fatal("workspace root leaked across accounts")
	}
	if gjson.Get(leftMeta, "cwd").String() != leftRoot {
		t.Fatalf("cwd = %q, want %q", gjson.Get(leftMeta, "cwd").String(), leftRoot)
	}
	if gjson.Get(rightMeta, "cwd").String() != rightRoot {
		t.Fatalf("right cwd = %q, want %q", gjson.Get(rightMeta, "cwd").String(), rightRoot)
	}
	leftWorkspace := workspaceEntryByRoot(t, leftMeta, leftRoot)
	if workspaceEntryByRoot(t, leftMeta, "/Users/alice/work/repo") != "" {
		t.Fatal("original workspace path still present")
	}
	if gjson.Get(leftWorkspace, "remote").String() == "https://github.com/acme/repo.git" {
		t.Fatal("workspace remote was not rewritten")
	}
	if gjson.Get(leftWorkspace, "sha").String() == "0123456789abcdef0123456789abcdef01234567" {
		t.Fatal("workspace sha was not rewritten")
	}
	if !gjson.Get(leftWorkspace, "dirty").Bool() {
		t.Fatal("workspace dirty flag should stay")
	}
}

func workspaceEntryByRoot(t *testing.T, metadata, root string) string {
	t.Helper()
	var found string
	gjson.Get(metadata, "workspaces").ForEach(func(key, value gjson.Result) bool {
		if key.String() == root {
			found = value.Raw
			return false
		}
		return true
	})
	return found
}

func TestCodexFingerprintDeviceKeepsSessionTree(t *testing.T) {
	t.Parallel()
	payload := testFingerprintPayload()
	body, state := applyCodexFingerprintBody(&config.Config{}, testFingerprintAuth("auth-a", "device"), payload, payload, codexIdentityConfuseState{})
	if state.installationID == "" || state.installationID == "client-install" {
		t.Fatalf("device mode should rewrite installation: %q", state.installationID)
	}
	if got := gjson.GetBytes(body, "client_metadata.session_id").String(); got != "client-session" {
		t.Fatalf("device mode session = %q, want original", got)
	}
	if got := gjson.GetBytes(body, "client_metadata.thread_id").String(); got != "client-thread" {
		t.Fatalf("device mode thread = %q, want original", got)
	}
	meta := gjson.GetBytes(body, "client_metadata.x-codex-turn-metadata").String()
	if gjson.Get(meta, "parent_thread_id").String() != "client-parent" {
		t.Fatalf("device mode should keep parent thread, got %q", gjson.Get(meta, "parent_thread_id").String())
	}
	if gjson.Get(meta, "cwd").String() == "/Users/alice/work/repo" {
		t.Fatal("device mode should still rewrite workspace path")
	}
}

func TestCodexFingerprintOffLeavesPayload(t *testing.T) {
	t.Parallel()
	payload := testFingerprintPayload()
	body, state := applyCodexFingerprintBody(&config.Config{}, testFingerprintAuth("auth-a", "off"), payload, payload, codexIdentityConfuseState{})
	if state.fingerprintMode != "" && state.fingerprintMode != "off" {
		t.Fatalf("off mode state = %q", state.fingerprintMode)
	}
	if got := gjson.GetBytes(body, "client_metadata.x-codex-installation-id").String(); got != "client-install" {
		t.Fatalf("off mode installation = %q", got)
	}
	meta := gjson.GetBytes(body, "client_metadata.x-codex-turn-metadata").String()
	if gjson.Get(meta, "cwd").String() != "/Users/alice/work/repo" {
		t.Fatalf("off mode cwd = %q", gjson.Get(meta, "cwd").String())
	}
}

func TestCodexFingerprintHeadersRewriteLineage(t *testing.T) {
	t.Parallel()
	payload := testFingerprintPayload()
	_, state := applyCodexFingerprintBody(&config.Config{}, testFingerprintAuth("auth-a", "session"), payload, payload, codexIdentityConfuseState{})
	headers := http.Header{}
	headers.Set("X-Codex-Turn-Metadata", gjson.GetBytes(payload, "client_metadata.x-codex-turn-metadata").String())
	headers.Set("X-Codex-Parent-Thread-Id", "client-parent")
	applyCodexFingerprintHeaders(headers, &state)
	if got := headers.Get("X-Codex-Parent-Thread-Id"); got != state.parentThreadID {
		t.Fatalf("parent header = %q, want %q", got, state.parentThreadID)
	}
	if got := headers.Get("X-Codex-Installation-Id"); got != state.installationID {
		t.Fatalf("installation header = %q, want %q", got, state.installationID)
	}
	meta := headers.Get("X-Codex-Turn-Metadata")
	if gjson.Get(meta, "parent_thread_id").String() != state.parentThreadID {
		t.Fatalf("header metadata parent = %q, want %q", gjson.Get(meta, "parent_thread_id").String(), state.parentThreadID)
	}
}

func TestCodexFingerprintKeepsConversationsApartOnSameAccount(t *testing.T) {
	t.Parallel()
	cfg := &config.Config{}
	auth := testFingerprintAuth("auth-a", "session")
	first := []byte(`{"client_metadata":{"session_id":"conversation-1","thread_id":"thread-1"}}`)
	second := []byte(`{"client_metadata":{"session_id":"conversation-2","thread_id":"thread-2"}}`)
	repeat := []byte(`{"client_metadata":{"session_id":"conversation-1","thread_id":"thread-1"}}`)

	_, firstState := applyCodexFingerprintBody(cfg, auth, first, first, codexIdentityConfuseState{})
	_, secondState := applyCodexFingerprintBody(cfg, auth, second, second, codexIdentityConfuseState{})
	_, repeatState := applyCodexFingerprintBody(cfg, auth, repeat, repeat, codexIdentityConfuseState{})

	if firstState.sessionID == "" || firstState.sessionID == "conversation-1" {
		t.Fatalf("conversation session was not remapped: %q", firstState.sessionID)
	}
	if firstState.sessionID == secondState.sessionID {
		t.Fatal("different conversations were collapsed onto one official session")
	}
	if firstState.sessionID != repeatState.sessionID {
		t.Fatalf("same conversation must keep the same official session: first=%q repeat=%q", firstState.sessionID, repeatState.sessionID)
	}
	if firstState.sessionID == mapFingerprintSessionID("auth-a", "account") {
		t.Fatal("conversation session still uses the old per-account seed")
	}
}

func TestCodexFingerprintEmptySessionDoesNotReuseAccountSeed(t *testing.T) {
	t.Parallel()
	cfg := &config.Config{}
	auth := testFingerprintAuth("auth-a", "session")
	payload := []byte(`{"client_metadata":{}}`)
	_, first := applyCodexFingerprintBody(cfg, auth, payload, payload, codexIdentityConfuseState{})
	_, second := applyCodexFingerprintBody(cfg, auth, payload, payload, codexIdentityConfuseState{})
	if first.sessionID == "" || second.sessionID == "" {
		t.Fatal("empty session should still receive an official session id")
	}
	if first.sessionID == second.sessionID {
		t.Fatal("requests without a conversation id must not share one official session")
	}
	if first.sessionID == mapFingerprintSessionID("auth-a", "account") || second.sessionID == mapFingerprintSessionID("auth-a", "account") {
		t.Fatal("empty session still uses the old per-account seed")
	}
}

func TestCodexFingerprintFullCollapsesThread(t *testing.T) {
	t.Parallel()
	payload := testFingerprintPayload()
	_, state := applyCodexFingerprintBody(&config.Config{}, testFingerprintAuth("auth-a", "full"), payload, payload, codexIdentityConfuseState{})
	if state.threadID != state.sessionID {
		t.Fatalf("full thread = %q, want session %q", state.threadID, state.sessionID)
	}
	if state.parentThreadID != state.sessionID {
		t.Fatalf("full parent = %q, want session %q", state.parentThreadID, state.sessionID)
	}
}
