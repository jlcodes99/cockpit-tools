package executor

import (
	"crypto/sha1"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"strings"

	"github.com/google/uuid"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

var fingerprintThreadMetadataKeys = []string{
	"thread_id",
	"parent_thread_id",
	"forked_from_thread_id",
	"forked_from_id",
	"root_thread_id",
}

var fingerprintTurnMetadataKeys = []string{
	"turn_id",
	"parent_turn_id",
	"root_turn_id",
}

var fingerprintWorkspacePathKeys = map[string]struct{}{
	"cwd": {}, "root": {}, "path": {}, "git_root": {}, "workspace": {},
}

var fingerprintWorkspaceRemoteKeys = map[string]struct{}{
	"remote": {}, "git_remote": {}, "origin_url": {}, "url": {}, "origin": {},
}

var fingerprintWorkspaceSHAKeys = map[string]struct{}{
	"sha": {}, "head": {}, "commit": {}, "commit_hash": {}, "oid": {}, "head_sha": {},
}

func codexFingerprintMode(cfg *config.Config, auth *cliproxyauth.Auth) string {
	if cfg == nil || auth == nil || codexAuthUsesAPIKey(auth) {
		return "off"
	}
	mode := ""
	if auth.Metadata != nil {
		if raw, ok := auth.Metadata["codex_fingerprint_mode"].(string); ok {
			mode = strings.ToLower(strings.TrimSpace(raw))
		}
	}
	if mode == "" {
		mode = "off"
	}
	switch mode {
	case "device", "session", "full", "off":
		return mode
	default:
		return "session"
	}
}

func codexFingerprintUUID(authID, kind, value string) string {
	name := strings.Join([]string{
		"cli-proxy-api", "codex", "fingerprint", kind,
		strings.TrimSpace(authID), strings.TrimSpace(value),
	}, ":")
	return uuid.NewSHA1(uuid.NameSpaceOID, []byte(name)).String()
}

func fingerprintShortHash(authID, kind, value string) string {
	sum := sha1.Sum([]byte(strings.Join([]string{
		"cli-proxy-api", "codex", "fingerprint", kind,
		strings.TrimSpace(authID), strings.TrimSpace(value),
	}, ":")))
	return hex.EncodeToString(sum[:4])
}

func mapFingerprintSessionID(authID, original string) string {
	original = strings.TrimSpace(original)
	if original == "" {
		// API clients without a conversation id must not collapse onto one
		// per-account session; that serializes every later turn behind the
		// previous upstream stream.
		original = uuid.NewString()
	}
	return codexFingerprintUUID(authID, "session", original)
}

func mapFingerprintThreadID(mode, authID, sessionID, original string) string {
	original = strings.TrimSpace(original)
	if mode == "full" || original == "" {
		return sessionID
	}
	return codexFingerprintUUID(authID, "thread", original)
}

func mapFingerprintTurnID(authID, original string) string {
	original = strings.TrimSpace(original)
	if original == "" {
		return ""
	}
	return codexFingerprintUUID(authID, "turn", original)
}

func fakeFingerprintWorkspaceRoot(authID, original string) string {
	return "/Users/codex/" + fingerprintShortHash(authID, "account", "root") + "/ws/" + fingerprintShortHash(authID, "workspace", original)
}

func fakeFingerprintRemote(authID, original string) string {
	return "https://git.invalid/" + fingerprintShortHash(authID, "account", "root") + "/" + fingerprintShortHash(authID, "remote", original) + ".git"
}

func fakeFingerprintGitSHA(authID, original string) string {
	sum := sha1.Sum([]byte(strings.Join([]string{
		"cli-proxy-api", "codex", "fingerprint", "gitsha",
		strings.TrimSpace(authID), strings.TrimSpace(original),
	}, ":")))
	return hex.EncodeToString(sum[:])
}

func looksLikeAbsPath(value string) bool {
	value = strings.TrimSpace(value)
	if value == "" {
		return false
	}
	if strings.HasPrefix(value, "/") {
		return true
	}
	if len(value) >= 3 && value[1] == ':' && (value[2] == '\\' || value[2] == '/') {
		return true
	}
	return strings.HasPrefix(value, "\\\\")
}

func (state *codexIdentityConfuseState) recordFingerprintReplacement(from, to string) {
	from = strings.TrimSpace(from)
	to = strings.TrimSpace(to)
	if state == nil || from == "" || to == "" || from == to {
		return
	}
	for _, item := range state.fingerprintReplacements {
		if item.original == from {
			return
		}
	}
	state.fingerprintReplacements = append(state.fingerprintReplacements, codexIdentityReplacement{
		original: from,
		confused: to,
	})
}

func codexFingerprintOriginalSessionID(userPayload []byte, state *codexIdentityConfuseState) string {
	for _, path := range []string{
		"session_id",
		"session-id",
		"client_metadata.session_id",
		"client_metadata.x-codex-window-id",
		"prompt_cache_key",
	} {
		if value := strings.TrimSpace(gjson.GetBytes(userPayload, path).String()); value != "" {
			return value
		}
	}
	if state != nil {
		return strings.TrimSpace(state.originalPromptCacheKey)
	}
	return ""
}

func applyCodexFingerprintBody(cfg *config.Config, auth *cliproxyauth.Auth, userPayload []byte, rawJSON []byte, state codexIdentityConfuseState) ([]byte, codexIdentityConfuseState) {
	mode := codexFingerprintMode(cfg, auth)
	if mode == "off" || auth == nil || strings.TrimSpace(auth.ID) == "" || len(rawJSON) == 0 {
		return rawJSON, state
	}
	authID := strings.TrimSpace(auth.ID)
	if strings.TrimSpace(state.authID) == "" {
		state.authID = authID
	}
	state.fingerprintMode = mode
	state.originalInstallationID = strings.TrimSpace(gjson.GetBytes(userPayload, "client_metadata.x-codex-installation-id").String())
	state.installationID = codexFingerprintUUID(authID, "installation", "account")
	state.originalSessionID = codexFingerprintOriginalSessionID(userPayload, &state)
	state.originalThreadID = strings.TrimSpace(gjson.GetBytes(userPayload, "client_metadata.thread_id").String())
	if state.originalThreadID == "" {
		state.originalThreadID = strings.TrimSpace(gjson.GetBytes(userPayload, "thread_id").String())
	}
	if state.originalThreadID == "" {
		state.originalThreadID = state.originalSessionID
	}
	state.originalWindowID = strings.TrimSpace(gjson.GetBytes(userPayload, "client_metadata.x-codex-window-id").String())
	turnMetadata := firstFingerprintTurnMetadata(userPayload, rawJSON)
	state.originalParentThreadID = firstNonEmpty(
		gjson.Get(turnMetadata, "parent_thread_id").String(),
		gjson.GetBytes(userPayload, "client_metadata.x-codex-parent-thread-id").String(),
		gjson.GetBytes(userPayload, "parent_thread_id").String(),
	)

	if mode != "device" {
		state.sessionID = mapFingerprintSessionID(authID, firstNonEmpty(state.originalSessionID, state.originalThreadID))
		state.threadID = mapFingerprintThreadID(mode, authID, state.sessionID, state.originalThreadID)
		state.windowID = state.threadID + ":0"
		if state.originalParentThreadID != "" {
			state.parentThreadID = mapFingerprintThreadID(mode, authID, state.sessionID, state.originalParentThreadID)
		}
	}

	if state.installationID != "" {
		rawJSON, _ = sjson.SetBytes(rawJSON, "client_metadata.x-codex-installation-id", state.installationID)
		state.recordFingerprintReplacement(state.originalInstallationID, state.installationID)
	}
	if mode != "device" {
		for path, value := range map[string]string{
			"client_metadata.session_id":               state.sessionID,
			"client_metadata.thread_id":                state.threadID,
			"client_metadata.x-codex-window-id":        state.windowID,
			"client_metadata.x-codex-parent-thread-id": state.parentThreadID,
		} {
			if value != "" {
				rawJSON, _ = sjson.SetBytes(rawJSON, path, value)
			}
		}
		if gjson.GetBytes(rawJSON, "session_id").Exists() && state.sessionID != "" {
			rawJSON, _ = sjson.SetBytes(rawJSON, "session_id", state.sessionID)
		}
		if gjson.GetBytes(rawJSON, "thread_id").Exists() && state.threadID != "" {
			rawJSON, _ = sjson.SetBytes(rawJSON, "thread_id", state.threadID)
		}
		state.recordFingerprintReplacement(state.originalSessionID, state.sessionID)
		state.recordFingerprintReplacement(state.originalThreadID, state.threadID)
		state.recordFingerprintReplacement(state.originalWindowID, state.windowID)
		state.recordFingerprintReplacement(state.originalParentThreadID, state.parentThreadID)
	}

	if raw := strings.TrimSpace(gjson.GetBytes(rawJSON, "client_metadata.x-codex-turn-metadata").String()); raw != "" {
		updated := rewriteFingerprintTurnMetadata(raw, &state, mode, authID)
		rawJSON, _ = sjson.SetBytes(rawJSON, "client_metadata.x-codex-turn-metadata", updated)
	}
	return rawJSON, state
}

func applyCodexFingerprintHeaders(headers http.Header, state *codexIdentityConfuseState) {
	if headers == nil || state == nil || state.fingerprintMode == "" || state.fingerprintMode == "off" {
		return
	}
	if headerSession := strings.TrimSpace(codexSessionHeaderValue(headers)); headerSession != "" {
		if state.originalSessionID == "" || state.originalSessionID == headerSession {
			state.originalSessionID = headerSession
			if state.fingerprintMode != "device" && strings.TrimSpace(state.authID) != "" {
				state.sessionID = mapFingerprintSessionID(state.authID, headerSession)
			}
		}
	}
	if state.installationID != "" {
		setHeaderCasePreserved(headers, "X-Codex-Installation-Id", state.installationID)
	}
	if raw := strings.TrimSpace(headerValueCaseInsensitive(headers, "X-Codex-Parent-Thread-Id")); raw != "" && state.originalParentThreadID == "" {
		state.originalParentThreadID = raw
		if state.fingerprintMode != "device" && state.sessionID != "" {
			state.parentThreadID = mapFingerprintThreadID(state.fingerprintMode, state.authID, state.sessionID, raw)
		}
	}
	if state.fingerprintMode == "device" {
		if raw := strings.TrimSpace(headerValueCaseInsensitive(headers, "X-Codex-Turn-Metadata")); raw != "" {
			headers.Set("X-Codex-Turn-Metadata", rewriteFingerprintTurnMetadata(raw, state, state.fingerprintMode, headerFingerprintAuthID(state)))
		}
		return
	}
	if state.sessionID != "" {
		setCodexSessionHeaderCasePreserved(headers, "Session-Id", state.sessionID)
	}
	if state.threadID != "" {
		setHeaderCasePreserved(headers, "Thread-Id", state.threadID)
		setHeaderCasePreserved(headers, "X-Client-Request-Id", state.threadID)
	}
	if state.windowID != "" {
		setHeaderCasePreserved(headers, "X-Codex-Window-Id", state.windowID)
	}
	if state.parentThreadID != "" {
		setHeaderCasePreserved(headers, "X-Codex-Parent-Thread-Id", state.parentThreadID)
	}
	if raw := strings.TrimSpace(headerValueCaseInsensitive(headers, "X-Codex-Turn-Metadata")); raw != "" {
		headers.Set("X-Codex-Turn-Metadata", rewriteFingerprintTurnMetadata(raw, state, state.fingerprintMode, headerFingerprintAuthID(state)))
	}
}

func headerFingerprintAuthID(state *codexIdentityConfuseState) string {
	if state == nil {
		return ""
	}
	if authID := strings.TrimSpace(state.authID); authID != "" {
		return authID
	}
	return "account"
}

func applyCodexFingerprintResponsePayload(payload []byte, state codexIdentityConfuseState, exposeClient bool) []byte {
	if state.fingerprintMode == "" || state.fingerprintMode == "off" {
		return payload
	}
	pairs := [][2]string{
		{state.installationID, state.originalInstallationID},
		{state.sessionID, state.originalSessionID},
		{state.threadID, state.originalThreadID},
		{state.windowID, state.originalWindowID},
		{state.parentThreadID, state.originalParentThreadID},
	}
	for _, item := range state.fingerprintReplacements {
		pairs = append(pairs, [2]string{item.confused, item.original})
	}
	if !exposeClient {
		for i := range pairs {
			pairs[i][0], pairs[i][1] = pairs[i][1], pairs[i][0]
		}
	}
	for _, pair := range pairs {
		payload = replaceCodexIdentityResponsePayload(payload, pair[0], pair[1])
	}
	return payload
}

func firstFingerprintTurnMetadata(userPayload, rawJSON []byte) string {
	for _, source := range [][]byte{userPayload, rawJSON} {
		if raw := strings.TrimSpace(gjson.GetBytes(source, "client_metadata.x-codex-turn-metadata").String()); raw != "" {
			return raw
		}
	}
	return ""
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}

func rewriteFingerprintTurnMetadata(raw string, state *codexIdentityConfuseState, mode, authID string) string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return raw
	}
	var obj map[string]any
	if err := json.Unmarshal([]byte(raw), &obj); err != nil {
		return raw
	}
	if state != nil && state.installationID != "" {
		obj["installation_id"] = state.installationID
	}
	if mode != "device" && state != nil {
		if state.sessionID != "" {
			obj["session_id"] = state.sessionID
		}
		if state.threadID != "" {
			obj["thread_id"] = state.threadID
		}
		if state.windowID != "" {
			obj["window_id"] = state.windowID
		}
		for _, key := range fingerprintThreadMetadataKeys {
			if key == "thread_id" {
				continue
			}
			remapFingerprintStringField(obj, key, func(original string) string {
				return mapFingerprintThreadID(mode, authID, state.sessionID, original)
			}, state)
		}
		for _, key := range fingerprintTurnMetadataKeys {
			remapFingerprintStringField(obj, key, func(original string) string {
				return mapFingerprintTurnID(authID, original)
			}, state)
		}
	}
	if workspaces, ok := obj["workspaces"]; ok {
		obj["workspaces"] = rewriteFingerprintWorkspaces(workspaces, authID, state)
	}
	if cwd, ok := obj["cwd"].(string); ok && looksLikeAbsPath(cwd) {
		mapped := fakeFingerprintWorkspaceRoot(authID, cwd)
		if state != nil {
			state.recordFingerprintReplacement(cwd, mapped)
		}
		obj["cwd"] = mapped
	}
	encoded, err := json.Marshal(obj)
	if err != nil {
		return raw
	}
	return string(encoded)
}

func remapFingerprintStringField(obj map[string]any, key string, mapper func(string) string, state *codexIdentityConfuseState) {
	raw, ok := obj[key].(string)
	if !ok {
		return
	}
	original := strings.TrimSpace(raw)
	if original == "" {
		return
	}
	mapped := strings.TrimSpace(mapper(original))
	if mapped == "" {
		return
	}
	obj[key] = mapped
	if state != nil {
		state.recordFingerprintReplacement(original, mapped)
	}
}

func rewriteFingerprintWorkspaces(value any, authID string, state *codexIdentityConfuseState) any {
	raw, err := json.Marshal(value)
	if err != nil {
		return value
	}
	var obj map[string]json.RawMessage
	if err := json.Unmarshal(raw, &obj); err != nil {
		return value
	}
	out := make(map[string]json.RawMessage, len(obj))
	for key, item := range obj {
		mappedKey := fakeFingerprintWorkspaceRoot(authID, key)
		if state != nil {
			state.recordFingerprintReplacement(key, mappedKey)
		}
		out[mappedKey] = rewriteFingerprintWorkspaceEntry(item, authID, state)
	}
	return out
}

func rewriteFingerprintWorkspaceEntry(raw json.RawMessage, authID string, state *codexIdentityConfuseState) json.RawMessage {
	var obj map[string]any
	if err := json.Unmarshal(raw, &obj); err != nil {
		return raw
	}
	rewriteFingerprintWorkspaceMap(obj, authID, state)
	encoded, err := json.Marshal(obj)
	if err != nil {
		return raw
	}
	return encoded
}

func rewriteFingerprintWorkspaceMap(obj map[string]any, authID string, state *codexIdentityConfuseState) {
	for key, value := range obj {
		switch typed := value.(type) {
		case string:
			lower := strings.ToLower(strings.TrimSpace(key))
			switch {
			case hasFingerprintKey(fingerprintWorkspacePathKeys, lower) && looksLikeAbsPath(typed):
				mapped := fakeFingerprintWorkspaceRoot(authID, typed)
				obj[key] = mapped
				if state != nil {
					state.recordFingerprintReplacement(typed, mapped)
				}
			case hasFingerprintKey(fingerprintWorkspaceRemoteKeys, lower) && strings.TrimSpace(typed) != "":
				mapped := fakeFingerprintRemote(authID, typed)
				obj[key] = mapped
				if state != nil {
					state.recordFingerprintReplacement(typed, mapped)
				}
			case hasFingerprintKey(fingerprintWorkspaceSHAKeys, lower) && looksLikeGitSHA(typed):
				mapped := fakeFingerprintGitSHA(authID, typed)
				obj[key] = mapped
				if state != nil {
					state.recordFingerprintReplacement(typed, mapped)
				}
			}
		case map[string]any:
			rewriteFingerprintWorkspaceMap(typed, authID, state)
		}
	}
}

func hasFingerprintKey(set map[string]struct{}, key string) bool {
	_, ok := set[key]
	return ok
}

func looksLikeGitSHA(value string) bool {
	value = strings.TrimSpace(value)
	if len(value) != 40 && len(value) != 64 {
		return false
	}
	for _, ch := range value {
		if (ch < '0' || ch > '9') && (ch < 'a' || ch > 'f') && (ch < 'A' || ch > 'F') {
			return false
		}
	}
	return true
}
