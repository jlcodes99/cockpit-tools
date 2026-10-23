package auth

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"regexp"
	"strings"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

var capabilityToolTypeParam = regexp.MustCompile(`^tools(?:\[\d+\]|\.\d+)\.type$`)

// canFallbackCapability permits only explicit pre-generation protocol rejection
// of a feature actually present in a complete, portable request. It changes no
// account grants, model aliases, routing policy, or credential retry budgets.
func canFallbackCapability(req cliproxyexecutor.Request, opts cliproxyexecutor.Options, err error) bool {
	if !cliproxyexecutor.IsUpstreamRequestRejected(err) || isRequestScopedError(err) || opts.Alt != "" || pinnedAuthIDFromMetadata(opts.Metadata) != "" ||
		strings.TrimSpace(opts.Headers.Get("X-Cockpit-Target-Account-Id")) != "" {
		return false
	}
	status := statusCodeFromError(err)
	if status != 400 && status != 422 {
		return false
	}
	raw := err.Error()
	var authErr *Error
	if errors.As(err, &authErr) {
		raw = authErr.Message
	}
	var envelope struct {
		Error struct{ Code, Type, Param, Capability string }
	}
	if json.Unmarshal([]byte(raw), &envelope) != nil {
		return false
	}
	capabilities, safe := portableRequestCapabilities(req.Payload)
	if !safe {
		return false
	}
	if len(opts.OriginalRequest) != 0 {
		if _, safe := portableRequestCapabilities(opts.OriginalRequest); !safe {
			return false
		}
	}
	detail := envelope.Error
	code := strings.ToLower(strings.TrimSpace(detail.Code))
	if code == "" {
		code = strings.ToLower(strings.TrimSpace(detail.Type))
	}
	switch code {
	case "unsupported_tool", "unsupported_tool_type", "tool_not_supported":
		return capabilities["tools"]
	case "unsupported_value":
		return capabilityToolTypeParam.MatchString(detail.Param) && capabilities["tools"]
	case "unsupported_capability", "capability_not_supported":
		return capabilities[detail.Capability]
	case "vision_not_supported", "image_input_not_supported":
		return capabilities["image_input"]
	case "audio_input_not_supported":
		return capabilities["audio_input"]
	case "video_input_not_supported":
		return capabilities["video_input"]
	default:
		return false
	}
}

func portableRequestCapabilities(payload []byte) (map[string]bool, bool) {
	decoded, ok := decodePortableRequest(payload)
	body, isObject := decoded.(map[string]any)
	if !ok || !isObject || body == nil {
		return nil, false
	}
	// No continuation lookup or provider-owned session can cross credentials.
	for _, key := range []string{"previous_response_id", "conversation", "conversation_id"} {
		if value, exists := body[key]; exists && value != nil && value != "" {
			return nil, false
		}
	}
	hasHistory := false
	for _, key := range []string{"input", "messages", "contents"} {
		switch history := body[key].(type) {
		case string:
			hasHistory = hasHistory || strings.TrimSpace(history) != ""
		case []any:
			hasHistory = hasHistory || len(history) > 0
		}
	}
	if !hasHistory {
		return nil, false
	}
	capabilities := make(map[string]bool)
	var toolCapabilities func([]any)
	toolCapabilities = func(tools []any) {
		for _, value := range tools {
			tool, ok := value.(map[string]any)
			if !ok {
				continue
			}
			kind, _ := tool["type"].(string)
			if kind != "" {
				capabilities[kind] = true
			}
			if nested, ok := tool["tools"].([]any); ok {
				toolCapabilities(nested)
			}
		}
	}
	calls := make(map[string]int)
	valid := true
	var visit func(any)
	visit = func(value any) {
		if !valid {
			return
		}
		switch value := value.(type) {
		case []any:
			for _, item := range value {
				visit(item)
			}
		case map[string]any:
			for _, key := range []string{"functionCall", "functionResponse", "function_call"} {
				if value[key] != nil {
					valid = false
					return
				}
			}
			for _, key := range []string{"encrypted_content", "signature", "file_id", "upload_id", "container_id"} {
				if field, exists := value[key]; exists && field != nil && field != "" {
					valid = false
					return
				}
			}
			kind, _ := value["type"].(string)
			text := func(key string) string { result, _ := value[key].(string); return result }
			id := ""
			switch kind {
			case "item_reference", "file_search_call", "web_search_call", "image_generation_call", "computer_call", "computer_call_output", "server_tool_use", "compaction", "compaction_summary", "mcp_call", "mcp_list_tools", "mcp_approval_request", "mcp_approval_response":
				valid = false
				return
			case "reasoning":
				if text("id") != "" {
					valid = false
					return
				}
			case "input_image", "image_url", "image":
				capabilities["image_input"] = true
			case "input_audio", "audio":
				capabilities["audio_input"] = true
			case "input_video", "video":
				capabilities["video_input"] = true
			case "function_call", "custom_tool_call":
				id = text("call_id")
				if id == "" {
					valid = false
					return
				}
				calls[id]++
			case "tool_use":
				id = text("id")
				if id == "" {
					valid = false
					return
				}
				calls[id]++
			case "function_call_output", "custom_tool_call_output":
				id = text("call_id")
				if id == "" || calls[id] == 0 {
					valid = false
					return
				}
				calls[id]--
			case "tool_result":
				id = text("tool_use_id")
				if id == "" || calls[id] == 0 {
					valid = false
					return
				}
				calls[id]--
			}
			if role, _ := value["role"].(string); role == "tool" {
				id = text("tool_call_id")
				if id == "" || calls[id] == 0 {
					valid = false
					return
				}
				calls[id]--
			}
			if tools, ok := value["tools"].([]any); ok && len(tools) > 0 {
				capabilities["tools"] = true
				toolCapabilities(tools)
			}
			if toolCalls, ok := value["tool_calls"].([]any); ok {
				for _, call := range toolCalls {
					call, ok := call.(map[string]any)
					if !ok {
						valid = false
						return
					}
					id, _ := call["id"].(string)
					if id == "" {
						valid = false
						return
					}
					calls[id]++
				}
			}
			// Map order must not reorder transcript calls and their outputs.
			// Input/messages arrays retain their exact chronological order.
			for key, child := range value {
				if key == "arguments" || key == "function" || key == "tool_calls" || key == "tools" {
					continue
				}
				visit(child)
			}
		}
	}
	visit(body)
	for _, count := range calls {
		if count != 0 {
			valid = false
		}
	}
	return capabilities, valid
}

// Ambiguous duplicate fields must not hide a private continuation or a second
// history from executors that use a different JSON parser. The original request
// error is retained when parsing is ambiguous; this is not an ingress validator.
func decodePortableRequest(payload []byte) (any, bool) {
	decoder := json.NewDecoder(bytes.NewReader(payload))
	decoder.UseNumber()
	var value func(int) (any, error)
	value = func(depth int) (any, error) {
		if depth > 128 {
			return nil, errors.New("portable history nesting limit")
		}
		token, err := decoder.Token()
		if err != nil {
			return nil, err
		}
		delimiter, isDelimiter := token.(json.Delim)
		if !isDelimiter {
			return token, nil
		}
		switch delimiter {
		case '{':
			object := make(map[string]any)
			seen := make(map[string]bool)
			for decoder.More() {
				keyToken, err := decoder.Token()
				if err != nil {
					return nil, err
				}
				key, ok := keyToken.(string)
				if !ok {
					return nil, errors.New("invalid object key")
				}
				folded := strings.ToLower(key)
				if seen[folded] {
					return nil, errors.New("ambiguous duplicate object key")
				}
				seen[folded] = true
				child, err := value(depth + 1)
				if err != nil {
					return nil, err
				}
				object[key] = child
			}
			closing, err := decoder.Token()
			if err != nil || closing != json.Delim('}') {
				return nil, errors.New("invalid object ending")
			}
			return object, nil
		case '[':
			var array []any
			for decoder.More() {
				child, err := value(depth + 1)
				if err != nil {
					return nil, err
				}
				array = append(array, child)
			}
			closing, err := decoder.Token()
			if err != nil || closing != json.Delim(']') {
				return nil, errors.New("invalid array ending")
			}
			return array, nil
		default:
			return nil, errors.New("invalid JSON delimiter")
		}
	}
	result, err := value(0)
	if err != nil {
		return nil, false
	}
	if _, err = decoder.Token(); err != io.EOF {
		return nil, false
	}
	return result, true
}
