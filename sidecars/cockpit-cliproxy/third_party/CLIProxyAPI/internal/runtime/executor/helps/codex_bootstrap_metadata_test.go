package helps

import (
	"testing"

	"github.com/tidwall/sjson"
)

func TestCodexHandshakeMetadataRejectsExecutionEvidence(t *testing.T) {
	for _, kind := range []string{"response.created", "response.in_progress", "response.metadata", "codex.response.metadata", "codex.rate_limits"} {
		for _, path := range []string{"", "metadata.", "response.", "response.metadata."} {
			base, _ := sjson.SetBytes([]byte(`{}`), "type", kind)
			for _, field := range []string{"delta", "item", "content", "arguments", "tool_calls", "function_call", "function_call_output", "output_text", "audio", "image", "partial_json", "encrypted_content", "reasoning_text", "summary"} {
				t.Run(kind+"/"+path+field, func(t *testing.T) {
					null, _ := sjson.SetBytes(base, path+field, nil)
					if !CodexHandshakeMetadataEvent(null) {
						t.Fatal("null placeholder incorrectly treated as execution")
					}
					executed, _ := sjson.SetBytes(base, path+field, "executed")
					if CodexHandshakeMetadataEvent(executed) {
						t.Fatal("execution field permitted transparent replay")
					}
				})
			}
			for _, field := range []string{"output", "usage.output_tokens", "usage.completion_tokens", "usage.output_tokens_details.reasoning_tokens", "usage.completion_tokens_details.reasoning_tokens"} {
				for _, value := range []any{1, -1, "0", map[string]any{"type": "function_call"}} {
					event, _ := sjson.SetBytes(base, path+field, value)
					if CodexHandshakeMetadataEvent(event) {
						t.Fatalf("unsafe/malformed output accepted: %s", event)
					}
				}
			}
		}
	}
}

func TestCodexHandshakeAllowsConfigurationButNotGeneratedReasoning(t *testing.T) {
	for _, event := range []string{
		`{"type":"response.created","response":{"output":[],"usage":null,"text":{"format":{"type":"text"},"verbosity":"medium"},"reasoning":{"effort":"low","summary":null}}}`,
		`{"type":"response.metadata","response":{"output":[],"usage":{"output_tokens":0},"text":{"format":{"type":"json_schema","schema":{"properties":{"text":{"type":"string"}}}}},"tools":[{"type":"function","parameters":{"properties":{"arguments":{"type":"string"},"content":{"type":"string"}}}}]}}`,
		`{"type":"codex.response.metadata","metadata":{"output":[],"output_tokens":0}}`,
	} {
		if !CodexHandshakeMetadataEvent([]byte(event)) {
			t.Fatalf("request configuration mistaken for execution: %s", event)
		}
	}
	for _, event := range []string{
		`{"type":"response.metadata","response":{"reasoning":{"summary":[{"type":"summary_text","text":"already reasoned"}]}}}`,
		`{"type":"response.metadata","response":{"reasoning":{"encrypted_content":"ciphertext"}}}`,
		`{"type":"response.metadata","response":{"text":{"delta":"hello"}}}`,
		`{"type":"response.metadata","metadata":{"text":{"format":{"type":"text"}}}}`,
		`{"type":"response.unknown_metadata","metadata":{}}`,
		`{"type":"response.metadata","response":{"usage":"unknown"}}`,
		`{"type":"response.metadata","response":[{"type":"message"}]}`,
	} {
		if CodexHandshakeMetadataEvent([]byte(event)) {
			t.Fatalf("unknown execution accepted: %s", event)
		}
	}
}
