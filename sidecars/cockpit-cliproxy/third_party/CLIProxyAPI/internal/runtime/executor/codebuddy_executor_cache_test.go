package executor

import (
	"testing"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

// TestCodebuddyExecutionSessionID verifies the session ID extraction precedence
// for CodeBuddy prompt cache key injection.
func TestCodebuddyExecutionSessionID(t *testing.T) {
	tests := []struct {
		name    string
		req     cliproxyexecutor.Request
		opts    cliproxyexecutor.Options
		want    string
	}{
		{
			name: "opts metadata wins",
			opts: cliproxyexecutor.Options{Metadata: map[string]any{
				cliproxyexecutor.ExecutionSessionMetadataKey: "session-from-opts",
			}},
			want: "session-from-opts",
		},
		{
			name: "req metadata fallback",
			req: cliproxyexecutor.Request{Metadata: map[string]any{
				cliproxyexecutor.ExecutionSessionMetadataKey: "session-from-req",
			}},
			want: "session-from-req",
		},
		{
			name: "payload prompt_cache_key fallback",
			req:  cliproxyexecutor.Request{Payload: []byte(`{"prompt_cache_key":"session-from-payload"}`)},
			want: "session-from-payload",
		},
		{
			name: "opts overrides req",
			req: cliproxyexecutor.Request{Metadata: map[string]any{
				cliproxyexecutor.ExecutionSessionMetadataKey: "session-from-req",
			}},
			opts: cliproxyexecutor.Options{Metadata: map[string]any{
				cliproxyexecutor.ExecutionSessionMetadataKey: "session-from-opts",
			}},
			want: "session-from-opts",
		},
		{
			name: "empty when no session",
			req:  cliproxyexecutor.Request{Payload: []byte(`{"messages":[]}`)},
			want: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := codebuddyExecutionSessionID(tt.req, tt.opts); got != tt.want {
				t.Fatalf("codebuddyExecutionSessionID() = %q, want %q", got, tt.want)
			}
		})
	}
}

// TestApplyCodebuddyPromptCache verifies prompt_cache_key injection and that
// prompt_cache_retention is preserved (CodeBuddy supports it, unlike XAI).
func TestApplyCodebuddyPromptCache(t *testing.T) {
	in := []byte(`{"model":"deepseek-v4-flash","messages":[],"prompt_cache_retention":"24h"}`)
	out := applyCodebuddyPromptCache(in, "session-123")

	if got := gjson.GetBytes(out, "prompt_cache_key").String(); got != "session-123" {
		t.Fatalf("prompt_cache_key = %q, want %q; out=%s", got, "session-123", out)
	}
	if got := gjson.GetBytes(out, "prompt_cache_retention").String(); got != "24h" {
		t.Fatalf("prompt_cache_retention = %q, want %q (must be preserved)", got, "24h")
	}
	if got := gjson.GetBytes(out, "model").String(); got != "deepseek-v4-flash" {
		t.Fatalf("model = %q, want unchanged", got)
	}
}

// TestApplyCodebuddyPromptCacheEmptyKey verifies no mutation on empty session.
func TestApplyCodebuddyPromptCacheEmptyKey(t *testing.T) {
	in := []byte(`{"model":"deepseek-v4-flash"}`)
	out := applyCodebuddyPromptCache(in, "")
	if gjson.GetBytes(out, "prompt_cache_key").Exists() {
		t.Fatalf("prompt_cache_key should not be set for empty session; out=%s", out)
	}
	if string(out) != string(in) {
		t.Fatalf("body should be unchanged; got %s", out)
	}
}
