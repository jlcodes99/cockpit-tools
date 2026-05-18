package converters

// responses_items.go 收敛 Responses canonical item 的共享解析与 provider 转换辅助逻辑。
// 约定内部优先使用 function_call / function_call_output；legacy tool_* 仅通过兼容路径进入。

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func resolveFunctionCallItem(item types.ResponsesItem) (string, string, string, error) {
	callID := item.CallID
	name := item.Name
	arguments := item.Arguments

	if contentMap, ok := item.Content.(map[string]interface{}); ok {
		if callID == "" {
			callID, _ = contentMap["call_id"].(string)
		}
		if name == "" {
			name, _ = contentMap["name"].(string)
		}
		if arguments == "" {
			arguments, _ = contentMap["arguments"].(string)
		}
		if nestedContent, ok := contentMap["content"].(map[string]interface{}); ok {
			if callID == "" {
				callID, _ = nestedContent["call_id"].(string)
			}
			if name == "" {
				name, _ = nestedContent["name"].(string)
			}
			if arguments == "" {
				arguments, _ = nestedContent["arguments"].(string)
			}
		}
	}

	if name == "" {
		return "", "", "", fmt.Errorf("function_call 缺少 name")
	}
	if callID == "" {
		callID = name
	}

	return callID, name, arguments, nil
}

func resolveFunctionCallOutputItem(item types.ResponsesItem) (string, interface{}, error) {
	callID := item.CallID
	if callID == "" {
		callID = item.Name
	}
	output := item.Output

	if contentMap, ok := item.Content.(map[string]interface{}); ok {
		if callID == "" {
			callID, _ = contentMap["call_id"].(string)
		}
		if callID == "" {
			callID, _ = contentMap["name"].(string)
		}
		if output == nil {
			output = contentMap["output"]
		}
		if nestedContent, ok := contentMap["content"].(map[string]interface{}); ok {
			if callID == "" {
				callID, _ = nestedContent["call_id"].(string)
			}
			if callID == "" {
				callID, _ = nestedContent["name"].(string)
			}
			if output == nil {
				output = nestedContent["output"]
			}
		}
	}

	if callID == "" {
		return "", nil, fmt.Errorf("function_call_output 缺少 call_id")
	}

	return callID, output, nil
}

func parseFunctionCallArguments(arguments string) interface{} {
	input := interface{}(map[string]interface{}{})
	if arguments == "" {
		return input
	}

	var parsed interface{}
	if err := json.Unmarshal([]byte(arguments), &parsed); err == nil {
		return parsed
	}

	return input
}

func resolveResponsesTextItem(item types.ResponsesItem) (string, string) {
	role := item.Role
	if role == "" {
		role = "user"
	}
	return role, extractTextFromContent(item.Content)
}

func resolveResponsesChatMessageContent(item types.ResponsesItem) (string, interface{}) {
	role, text := resolveResponsesTextItem(item)
	contentParts := responsesContentToOpenAIChatParts(item.Content)
	if len(contentParts) == 0 {
		if text == "" {
			return role, nil
		}
		return role, text
	}
	return role, contentParts
}

func responsesContentToOpenAIChatParts(content interface{}) []map[string]interface{} {
	switch v := content.(type) {
	case []interface{}:
		parts := make([]map[string]interface{}, 0, len(v))
		for _, raw := range v {
			block, ok := raw.(map[string]interface{})
			if !ok {
				continue
			}
			part := responsesContentBlockToOpenAIChatPart(block)
			if part == nil {
				continue
			}
			parts = append(parts, part)
		}
		if len(parts) == 0 {
			return nil
		}
		return parts
	default:
		return nil
	}
}

func responsesContentBlockToOpenAIChatPart(block map[string]interface{}) map[string]interface{} {
	blockType, _ := block["type"].(string)
	if blockType == "" {
		blockType = "input_text"
	}

	switch blockType {
	case "input_text", "output_text", "text":
		text, _ := block["text"].(string)
		if text == "" {
			return nil
		}
		return map[string]interface{}{"type": "text", "text": text}
	case "input_image", "image_url":
		imageURL := normalizeResponsesImageURL(block)
		if imageURL == nil {
			return nil
		}
		return map[string]interface{}{"type": "image_url", "image_url": imageURL}
	default:
		return nil
	}
}

func normalizeResponsesImageURL(block map[string]interface{}) map[string]interface{} {
	rawImageURL, ok := block["image_url"]
	if !ok {
		return nil
	}

	switch imageURL := rawImageURL.(type) {
	case string:
		if imageURL == "" {
			return nil
		}
		result := map[string]interface{}{"url": imageURL}
		if detail, _ := block["detail"].(string); detail != "" {
			result["detail"] = detail
		}
		return result
	case map[string]interface{}:
		if _, ok := imageURL["url"].(string); !ok {
			return nil
		}
		result := make(map[string]interface{}, len(imageURL)+1)
		for key, value := range imageURL {
			result[key] = value
		}
		if detail, _ := block["detail"].(string); detail != "" {
			result["detail"] = detail
		}
		return result
	default:
		return nil
	}
}

func extractResponsesReasoningText(item types.ResponsesItem) string {
	if text := extractReasoningTextFromSummary(item.Summary); text != "" {
		return text
	}
	return extractReasoningTextFromSummary(item.Content)
}

func extractReasoningTextFromSummary(raw interface{}) string {
	switch v := raw.(type) {
	case string:
		return v
	case []interface{}:
		parts := make([]string, 0, len(v))
		for _, rawPart := range v {
			if part, ok := rawPart.(map[string]interface{}); ok {
				if text, ok := part["text"].(string); ok && text != "" {
					parts = append(parts, text)
				}
			}
		}
		return strings.Join(parts, "\n")
	case []map[string]interface{}:
		parts := make([]string, 0, len(v))
		for _, part := range v {
			if text, ok := part["text"].(string); ok && text != "" {
				parts = append(parts, text)
			}
		}
		return strings.Join(parts, "\n")
	case []types.ContentBlock:
		parts := make([]string, 0, len(v))
		for _, part := range v {
			if part.Text != "" {
				parts = append(parts, part.Text)
			}
		}
		return strings.Join(parts, "\n")
	default:
		return ""
	}
}

func normalizeGeminiRole(role string) string {
	if role == "assistant" {
		return "model"
	}
	return role
}

func parseGeminiFunctionCallArgs(arguments string) map[string]interface{} {
	if arguments == "" {
		return nil
	}

	var args map[string]interface{}
	_ = JSONUnmarshal([]byte(arguments), &args)
	return args
}

func buildGeminiFunctionResponsePayload(output interface{}) map[string]interface{} {
	switch value := output.(type) {
	case string:
		return map[string]interface{}{"result": value}
	case map[string]interface{}:
		return value
	default:
		return map[string]interface{}{"result": fmt.Sprintf("%v", output)}
	}
}
