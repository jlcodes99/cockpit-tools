package executor

import (
	"fmt"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

func codexCallItemRequiresID(itemType string) bool {
	switch itemType {
	case "function_call", "custom_tool_call", "tool_call", "mcp_tool_call":
		return true
	default:
		return false
	}
}

func codexCallOutputRequiresID(itemType string) bool {
	switch itemType {
	case "function_call_output", "custom_tool_call_output", "tool_call_output", "mcp_tool_call_output":
		return true
	default:
		return false
	}
}

func nextGeneratedCodexCallID(prefix string, index int, used map[string]struct{}) string {
	base := fmt.Sprintf("%s_%d", prefix, index)
	if _, exists := used[base]; !exists {
		used[base] = struct{}{}
		return base
	}
	for suffix := 1; ; suffix++ {
		candidate := fmt.Sprintf("%s_%d", base, suffix)
		if _, exists := used[candidate]; !exists {
			used[candidate] = struct{}{}
			return candidate
		}
	}
}

// NormalizeResponsesCallIDs fills the same deterministic tool call defaults
// used by the Codex executor. Continuity must retain opaque orphan history so
// the authoritative owner check can reject or restore it rather than erase it.
func NormalizeResponsesCallIDs(body []byte, preserveOpaque ...bool) []byte {
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() {
		return body
	}
	used := make(map[string]struct{})
	for _, item := range input.Array() {
		if callID := strings.TrimSpace(item.Get("call_id").String()); callID != "" {
			used[callID] = struct{}{}
		}
	}
	type pendingCall struct {
		id   string
		name string
	}
	pending := make([]pendingCall, 0)
	dropInputIndices := make([]int, 0)
	index := -1
	input.ForEach(func(_, item gjson.Result) bool {
		index++
		itemType := strings.ToLower(strings.TrimSpace(item.Get("type").String()))
		isCall := codexCallItemRequiresID(itemType)
		isOutput := codexCallOutputRequiresID(itemType)
		if !isCall && !isOutput {
			return true
		}
		name := strings.TrimSpace(item.Get("name").String())
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" && codexHostResult(item) {
			// These host-injected records have no model-issued call to match.
			// Preserve malformed envelopes too; the shared Rust continuity
			// validator must reject them instead of receiving a fabricated ID.
			return true
		}
		if callID == "" {
			if isCall {
				callID = nextGeneratedCodexCallID("call_missing", index, used)
			} else {
				matched := -1
				if name != "" {
					for i, pendingCall := range pending {
						if pendingCall.name == name {
							matched = i
							break
						}
					}
				}
				if matched < 0 && len(pending) > 0 {
					matched = 0
				}
				if matched >= 0 {
					callID = pending[matched].id
					pending = append(pending[:matched], pending[matched+1:]...)
				} else if codexCallOutputCanStandAlone(itemType, item) {
					return true
				} else if len(preserveOpaque) > 0 && preserveOpaque[0] && responsesCallHasOpaqueContent(item) {
					return true
				} else {
					dropInputIndices = append(dropInputIndices, index)
					return true
				}
			}
			body, _ = sjson.SetBytes(body, fmt.Sprintf("input.%d.call_id", index), callID)
		}
		if isCall {
			pending = append(pending, pendingCall{id: callID, name: name})
			return true
		}
		for i, pendingCall := range pending {
			if pendingCall.id == callID {
				pending = append(pending[:i], pending[i+1:]...)
				break
			}
		}
		return true
	})
	for index := len(dropInputIndices) - 1; index >= 0; index-- {
		body, _ = sjson.DeleteBytes(body, fmt.Sprintf("input.%d", dropInputIndices[index]))
	}
	return body
}

// Keep this envelope aligned with native responses_host_results::is_host_result.
// Payload and explicit call ID validity belong to the continuity validator.
func codexHostResult(item gjson.Result) bool {
	if item.Get("type").String() != "function_call_output" || item.Get("namespace").String() != "codex_app" {
		return false
	}
	switch item.Get("name").String() {
	case "create_thread", "send_message_to_thread":
		return true
	default:
		return false
	}
}

func codexCallOutputCanStandAlone(itemType string, item gjson.Result) bool {
	if itemType != "function_call_output" {
		return false
	}
	name := item.Get("name")
	return name.Type == gjson.String && strings.TrimSpace(name.String()) != ""
}

func responsesCallHasOpaqueContent(value gjson.Result) bool {
	if !value.IsObject() && !value.IsArray() {
		return false
	}
	found := false
	value.ForEach(func(key, item gjson.Result) bool {
		if key.String() == "encrypted_content" && item.Type == gjson.String && item.String() != "" || responsesCallHasOpaqueContent(item) {
			found = true
			return false
		}
		return true
	})
	return found
}
