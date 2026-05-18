package types

import (
	"encoding/json"
	"fmt"
)

// ParseResponsesInput 解析 Responses input，并将 legacy tool_* 归一化为 function_*。
func ParseResponsesInput(input interface{}) ([]ResponsesItem, error) {
	switch v := input.(type) {
	case string:
		return []ResponsesItem{{Type: "text", Content: v}}, nil
	case []interface{}:
		items := make([]ResponsesItem, 0, len(v))
		for _, rawItem := range v {
			itemMap, ok := rawItem.(map[string]interface{})
			if !ok {
				continue
			}
			items = append(items, NormalizeResponsesItem(responsesItemFromMap(itemMap)))
		}
		return items, nil
	case []ResponsesItem:
		return NormalizeResponsesItems(v), nil
	default:
		return nil, fmt.Errorf("不支持的 input 类型: %T", input)
	}
}

// NormalizeResponsesItems 将多条 ResponsesItem 归一化为内部推荐表示。
func NormalizeResponsesItems(items []ResponsesItem) []ResponsesItem {
	normalized := make([]ResponsesItem, len(items))
	for i, item := range items {
		normalized[i] = NormalizeResponsesItem(item)
	}
	return normalized
}

// NormalizeResponsesItem 将 legacy tool_* 归一化为 function_*。
// 内部中间态推荐使用 function_call / function_call_output；tool_* 仅作为兼容输入保留。
func NormalizeResponsesItem(item ResponsesItem) ResponsesItem {
	switch item.Type {
	case "tool_call":
		return normalizeToolCallItem(item)
	case "tool_result":
		return normalizeToolResultItem(item)
	default:
		return item
	}
}

func responsesItemFromMap(itemMap map[string]interface{}) ResponsesItem {
	item := ResponsesItem{
		ID:        stringFromMap(itemMap, "id"),
		Type:      stringFromMap(itemMap, "type"),
		Role:      stringFromMap(itemMap, "role"),
		Status:    stringFromMap(itemMap, "status"),
		Content:   itemMap["content"],
		Summary:   itemMap["summary"],
		CallID:    stringFromMap(itemMap, "call_id"),
		Name:      stringFromMap(itemMap, "name"),
		Namespace: stringFromMap(itemMap, "namespace"),
		Input:     stringFromMap(itemMap, "input"),
		Arguments: stringFromMap(itemMap, "arguments"),
		Output:    itemMap["output"],
	}

	if item.Type == "" && item.Role != "" {
		item.Type = "message"
	}

	if toolUseMap, ok := itemMap["tool_use"].(map[string]interface{}); ok {
		item.ToolUse = &ToolUse{
			ID:    stringFromMap(toolUseMap, "id"),
			Name:  stringFromMap(toolUseMap, "name"),
			Input: toolUseMap["input"],
		}
	}

	return item
}

func normalizeToolCallItem(item ResponsesItem) ResponsesItem {
	callID := firstNonEmpty(item.CallID, item.ID)
	name := item.Name
	arguments := item.Arguments

	if item.ToolUse != nil {
		callID = firstNonEmpty(callID, item.ToolUse.ID)
		name = firstNonEmpty(name, item.ToolUse.Name)
		if arguments == "" {
			arguments = marshalJSONString(item.ToolUse.Input)
		}
	}

	if contentMap, ok := item.Content.(map[string]interface{}); ok {
		callID = firstNonEmpty(callID, stringFromMap(contentMap, "call_id"), stringFromMap(contentMap, "id"))
		name = firstNonEmpty(name, stringFromMap(contentMap, "name"))
		if arguments == "" {
			arguments = firstNonEmpty(arguments, stringFromMap(contentMap, "arguments"))
			if arguments == "" {
				arguments = marshalJSONString(contentMap["input"])
			}
		}

		if nestedContent, ok := contentMap["content"].(map[string]interface{}); ok {
			callID = firstNonEmpty(callID, stringFromMap(nestedContent, "call_id"), stringFromMap(nestedContent, "id"))
			name = firstNonEmpty(name, stringFromMap(nestedContent, "name"))
			if arguments == "" {
				arguments = firstNonEmpty(arguments, stringFromMap(nestedContent, "arguments"))
				if arguments == "" {
					arguments = marshalJSONString(nestedContent["input"])
				}
			}
		}
	}

	if item.CallID == "" {
		item.CallID = callID
	}
	if item.Name == "" {
		item.Name = name
	}
	if item.Arguments == "" {
		item.Arguments = arguments
	}

	if name == "" {
		return item
	}
	if callID == "" {
		callID = name
	}

	item.Type = "function_call"
	item.CallID = callID
	item.Name = name
	item.Arguments = arguments
	item.ToolUse = nil
	item.Content = nil
	return item
}

func normalizeToolResultItem(item ResponsesItem) ResponsesItem {
	callID := item.CallID
	output := item.Output

	if contentMap, ok := item.Content.(map[string]interface{}); ok {
		callID = firstNonEmpty(callID, stringFromMap(contentMap, "call_id"), stringFromMap(contentMap, "tool_use_id"), stringFromMap(contentMap, "name"))
		if output == nil {
			if rawOutput, ok := contentMap["output"]; ok {
				output = rawOutput
			} else if rawContent, ok := contentMap["content"]; ok {
				if nestedContent, ok := rawContent.(map[string]interface{}); ok {
					if nestedOutput, ok := nestedContent["output"]; ok {
						output = nestedOutput
					} else {
						output = rawContent
					}
				} else {
					output = rawContent
				}
			}
		}
	}

	if item.CallID == "" {
		item.CallID = callID
	}
	if item.Output == nil {
		item.Output = output
	}

	if callID == "" {
		return item
	}

	item.Type = "function_call_output"
	item.CallID = callID
	item.Output = output
	item.Content = nil
	return item
}

func stringFromMap(data map[string]interface{}, key string) string {
	value, _ := data[key].(string)
	return value
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

func marshalJSONString(value interface{}) string {
	if value == nil {
		return "{}"
	}
	data, err := json.Marshal(value)
	if err != nil {
		return "{}"
	}
	return string(data)
}
