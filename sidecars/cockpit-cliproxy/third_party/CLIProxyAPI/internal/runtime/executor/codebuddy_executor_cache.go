package executor

import (
	"strings"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// codebuddyExecutionSessionID resolves a stable, long-lived execution session ID
// for CodeBuddy prompt cache key injection. The final fallback reuses the routing
// layer's ExtractSessionID (the same source as session affinity), so the cache key
// and the account binding derive from one session identity:
//
//	同一会话 -> 同一 cache key -> 同一账号 -> 命中缓存
//
// Precedence: opts.Metadata[execution_session_id] -> req.Metadata[...] ->
// req.Payload.prompt_cache_key (client-provided) -> routing-layer ExtractSessionID.
func codebuddyExecutionSessionID(req cliproxyexecutor.Request, opts cliproxyexecutor.Options) string {
	if value := xaiMetadataString(opts.Metadata, cliproxyexecutor.ExecutionSessionMetadataKey); value != "" {
		return value
	}
	if value := xaiMetadataString(req.Metadata, cliproxyexecutor.ExecutionSessionMetadataKey); value != "" {
		return value
	}
	if promptCacheKey := gjson.GetBytes(req.Payload, "prompt_cache_key"); promptCacheKey.Exists() {
		return strings.TrimSpace(promptCacheKey.String())
	}
	// Fall back to the routing layer's session extraction, so that cache key and
	// session affinity share the same identity even on plain /v1/chat/completions.
	if sessionID := cliproxyauth.ExtractSessionID(opts.Headers, req.Payload, req.Metadata); sessionID != "" {
		return sessionID
	}
	return ""
}

// applyCodebuddyPromptCache injects the prompt cache key into the upstream body.
// Unlike XAI, CodeBuddy preserves prompt_cache_retention (the backend accepts it).
// It is a no-op when sessionID is empty.
func applyCodebuddyPromptCache(body []byte, sessionID string) []byte {
	if sessionID == "" {
		return body
	}
	out, err := sjson.SetBytes(body, "prompt_cache_key", sessionID)
	if err != nil {
		return body
	}
	return out
}
