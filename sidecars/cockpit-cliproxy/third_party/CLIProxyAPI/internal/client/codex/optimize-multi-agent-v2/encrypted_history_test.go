package multiagentv2

import (
	"bytes"
	"context"
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// Native mode preserves opaque history. Enabling the CPA optimizer explicitly
// retypes agent content while preserving its bytes and every other history item.
func TestOfficialCodexOptimizationKeepsMixedParentChildHistory(t *testing.T) {
	payload := []byte(`{"tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"type":"object","properties":{"message":{"type":"string","encrypted":true}}}}]}],"input":[
		{"type":"function_call","namespace":"collaboration","name":"spawn_agent","call_id":"bad-call","arguments":"{\"message\":"},
		{"type":"function_call_output","call_id":"bad-call","output":"Error parsing function arguments: EOF"},
		{"type":"agent_message","id":"amsg-cipher","author":"/root/child","recipient":"/root","content":[{"type":"encrypted_content","encrypted_content":"gAAAA-real-cipher-fixture"}]},
		{"type":"agent_message","id":"amsg-legacy","author":"/root","recipient":"/root/child","content":[{"type":"encrypted_content","encrypted_content":"legacy text whose source cannot be inferred"}]},
		{"type":"agent_message","id":"amsg-text","author":"/root","recipient":"/root/child","content":[{"type":"input_text","text":"known plaintext"}]},
		{"type":"function_call","namespace":"collaboration","name":"send_message","call_id":"plain-call","arguments":"{\"message\":\"known plaintext\"}","encrypted_function_args":[]},
		{"type":"function_call_output","call_id":"plain-call","output":"delivered"}
	]}`)
	original := bytes.Clone(payload)
	headers := http.Header{"User-Agent": {"Codex Desktop/0.146.0-alpha.3"}}
	for _, enabled := range []bool{false, true} {
		got, optimized := OptimizeCodexMultiAgentV2Request(context.Background(), headers, payload,
			&config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: enabled}})
		if optimized != enabled {
			t.Fatalf("optimized = %v, enabled = %v", optimized, enabled)
		}
		expected := bytes.Clone(original)
		if enabled {
			for _, index := range []string{"2", "3"} {
				path := "input." + index + ".content.0"
				value := gjson.GetBytes(expected, path+".encrypted_content").String()
				expected, _ = sjson.SetBytes(expected, path+".type", "input_text")
				expected, _ = sjson.SetBytes(expected, path+".text", value)
				expected, _ = sjson.DeleteBytes(expected, path+".encrypted_content")
			}
		}
		if !bytes.Equal(payload, original) || gjson.GetBytes(got, "input").Raw != gjson.GetBytes(expected, "input").Raw {
			t.Fatalf("history changed outside the explicitly enabled CPA content rewrite: %s", got)
		}
	}
}
