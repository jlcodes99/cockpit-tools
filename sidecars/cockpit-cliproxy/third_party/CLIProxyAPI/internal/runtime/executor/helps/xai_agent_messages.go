package helps

import (
	"encoding/json"
	"strconv"

	multiagentv2 "github.com/router-for-me/CLIProxyAPI/v7/internal/client/codex/optimize-multi-agent-v2"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// RewriteXAIAgentMessages is protocol adaptation, not an opt-in optimization.
// The unmodified caller history remains owned by the continuity layer.
func RewriteXAIAgentMessages(payload []byte) []byte {
	return multiagentv2.RewriteCodexAgentMessagesForCompatibility(payload)
}

// NormalizeXAICollaborationToolCalls marks ordinary xAI message arguments as
// plaintext for the official client. Missing metadata otherwise means encrypted
// messages, even though a third-party function returned ordinary JSON text.
func NormalizeXAICollaborationToolCalls(payload []byte) []byte {
	payload = NormalizeCodexCollaborationToolCalls(payload)
	root := gjson.ParseBytes(payload)
	paths := make([]string, 0)
	if root.Get("type").String() == "response.output_item.done" {
		paths = append(paths, "item")
	}
	for _, prefix := range []string{"output", "response.output"} {
		for index := range root.Get(prefix).Array() {
			paths = append(paths, prefix+"."+strconv.Itoa(index))
		}
	}
	for _, path := range paths {
		item := gjson.GetBytes(payload, path)
		if item.Get("type").String() != "function_call" || item.Get("namespace").String() != "collaboration" {
			continue
		}
		switch item.Get("name").String() {
		case "spawn_agent", "send_message", "followup_task":
		default:
			continue
		}
		if item.Get("encrypted_content").Type != gjson.Null || item.Get("encrypted_function_args").Type != gjson.Null {
			continue
		}
		if !json.Valid([]byte(item.Get("arguments").String())) {
			continue
		}
		if updated, err := sjson.SetRawBytes(payload, path+".encrypted_function_args", []byte("[]")); err == nil {
			payload = updated
		}
	}
	return payload
}
