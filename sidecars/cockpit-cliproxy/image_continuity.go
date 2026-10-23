package main

import (
	"encoding/json"
	"net/http"
)

// Images endpoints cannot carry Responses ciphertext or upstream resource IDs.
// Reject that conversion before extracting a prompt, instead of losing history.
func imageAPIHistoryIsOpaque(payload map[string]any, headers http.Header) bool {
	for _, name := range []string{"previous_response_id", "conversation", "conversation_id"} {
		if payload[name] != nil {
			return true
		}
	}
	if len(headers.Values("X-Codex-Turn-State")) > 0 || len(headers.Values("Conversation_id")) > 0 {
		return true
	}
	var opaque func(any) bool
	opaque = func(value any) bool {
		switch value := value.(type) {
		case []any:
			for _, item := range value {
				if opaque(item) {
					return true
				}
			}
		case map[string]any:
			if value["encrypted_content"] != nil || value["type"] == "item_reference" {
				return true
			}
			arguments, declared := value["encrypted_function_args"]
			plain := false
			if declared && arguments != nil {
				values, valid := arguments.([]any)
				if !valid || len(values) > 0 {
					return true
				}
				plain = true
			}
			if !plain && value["type"] == "function_call" && value["namespace"] == "collaboration" {
				switch value["name"] {
				case "spawn_agent", "send_message", "followup_task":
					var args map[string]any
					if raw, ok := value["arguments"].(string); ok && json.Unmarshal([]byte(raw), &args) == nil {
						if message, ok := args["message"].(string); ok && message != "" {
							return true
						}
					}
				}
			}
			for _, item := range value {
				if opaque(item) {
					return true
				}
			}
		}
		return false
	}
	return opaque(payload["input"])
}
