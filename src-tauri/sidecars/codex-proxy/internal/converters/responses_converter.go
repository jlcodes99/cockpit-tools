package converters

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// ============== Responses → Claude Messages ==============

// ResponsesToClaudeMessages 将 Responses 格式转换为 Claude Messages 格式
// instructions 参数会被转换为 Claude API 的 system 参数（不在 messages 中）
func ResponsesToClaudeMessages(sess *session.Session, newInput interface{}, instructions string) ([]types.ClaudeMessage, string, error) {
	messages := []types.ClaudeMessage{}

	// 1. 处理历史消息
	var err error
	messages, err = appendResponsesItemsToClaudeMessages(messages, sess.Messages)
	if err != nil {
		return nil, "", fmt.Errorf("转换历史消息失败: %w", err)
	}

	// 2. 处理新输入（统一在解析阶段完成 legacy tool_* → function_* 归一化）
	newItems, err := parseResponsesInput(newInput)
	if err != nil {
		return nil, "", err
	}

	// 3. 收集被跳过的 legacy tool_call ID，避免输出孤立的 tool_result/function_call_output。
	skippedCallIDs := make(map[string]bool)
	for _, item := range newItems {
		if item.Type == "tool_call" && item.ToolUse == nil {
			callID := item.CallID
			if callID == "" {
				callID = item.ID
			}
			if callID != "" {
				skippedCallIDs[callID] = true
			}
		}
	}

	// 4. 转换新输入，跳过与被跳过 tool_call 对应的结果项。
	filteredNewItems := make([]types.ResponsesItem, 0, len(newItems))
	for _, item := range newItems {
		if item.Type == "function_call_output" && item.CallID != "" && skippedCallIDs[item.CallID] {
			continue
		}
		filteredNewItems = append(filteredNewItems, item)
	}

	messages, err = appendResponsesItemsToClaudeMessages(messages, filteredNewItems)
	if err != nil {
		return nil, "", fmt.Errorf("转换新消息失败: %w", err)
	}

	return messages, instructions, nil
}

func appendResponsesItemsToClaudeMessages(messages []types.ClaudeMessage, items []types.ResponsesItem) ([]types.ClaudeMessage, error) {
	pendingThinking := []types.ClaudeContent{}

	flushThinking := func() {
		if len(pendingThinking) == 0 {
			return
		}
		content := append([]types.ClaudeContent(nil), pendingThinking...)
		messages = append(messages, types.ClaudeMessage{Role: "assistant", Content: content})
		pendingThinking = nil
	}

	for _, item := range items {
		item = types.NormalizeResponsesItem(item)
		if item.Type == "reasoning" {
			thinking := extractResponsesReasoningText(item)
			if thinking != "" {
				pendingThinking = append(pendingThinking, types.ClaudeContent{Type: "thinking", Thinking: thinking})
			}
			continue
		}

		msg, err := responsesItemToClaudeMessage(item)
		if err != nil {
			return nil, err
		}
		if msg == nil {
			continue
		}

		if len(pendingThinking) > 0 {
			if msg.Role == "assistant" {
				switch content := msg.Content.(type) {
				case []types.ClaudeContent:
					merged := append([]types.ClaudeContent(nil), pendingThinking...)
					merged = append(merged, content...)
					msg.Content = merged
					pendingThinking = nil
				case string:
					merged := append([]types.ClaudeContent(nil), pendingThinking...)
					if content != "" {
						merged = append(merged, types.ClaudeContent{Type: "text", Text: content})
					}
					msg.Content = merged
					pendingThinking = nil
				default:
					flushThinking()
				}
			} else {
				flushThinking()
			}
		}

		messages = append(messages, *msg)
	}

	flushThinking()
	return messages, nil
}

// responsesItemToClaudeMessage 单个 ResponsesItem 转换为 Claude Message
func responsesItemToClaudeMessage(item types.ResponsesItem) (*types.ClaudeMessage, error) {
	item = types.NormalizeResponsesItem(item)

	if item.Type == "tool_call" {
		return nil, nil
	}
	if item.Type == "tool_result" {
		return nil, fmt.Errorf("tool_result 缺少 tool_use_id")
	}

	switch item.Type {
	case "reasoning":
		thinking := extractResponsesReasoningText(item)
		if thinking == "" {
			return nil, nil
		}
		return &types.ClaudeMessage{
			Role: "assistant",
			Content: []types.ClaudeContent{{
				Type:     "thinking",
				Thinking: thinking,
			}},
		}, nil

	case "message":
		// 新格式：嵌套结构（type=message, role=user/assistant, content=[]ContentBlock）
		role, contentText := resolveResponsesTextItem(item)
		if contentText == "" {
			return nil, nil // 空内容，跳过
		}

		return &types.ClaudeMessage{
			Role: role,
			Content: []types.ClaudeContent{
				{
					Type: "text",
					Text: contentText,
				},
			},
		}, nil

	case "text":
		// 旧格式：简单 string（向后兼容）
		role, contentStr := resolveResponsesTextItem(item)
		if contentStr == "" {
			return nil, fmt.Errorf("text 类型的 content 不能为空")
		}

		return &types.ClaudeMessage{
			Role: role,
			Content: []types.ClaudeContent{
				{
					Type: "text",
					Text: contentStr,
				},
			},
		}, nil

	case "custom_tool_call":
		callID := item.CallID
		name := item.Name
		input := customToolInputFromItem(item)
		if callID == "" {
			callID = name
		}
		if name == "" {
			return nil, nil
		}
		return &types.ClaudeMessage{
			Role: "assistant",
			Content: []types.ClaudeContent{{
				Type:  "tool_use",
				ID:    callID,
				Name:  name,
				Input: input,
			}},
		}, nil

	case "custom_tool_call_output":
		callID := item.CallID
		output := item.Output
		if callID == "" {
			callID = item.Name
		}
		return &types.ClaudeMessage{
			Role: "user",
			Content: []types.ClaudeContent{{
				Type:      "tool_result",
				ToolUseID: callID,
				Content:   output,
			}},
		}, nil

	case "function_call":
		callID, name, arguments, err := resolveFunctionCallItem(item)
		if err != nil {
			return nil, err
		}

		return &types.ClaudeMessage{
			Role: "assistant",
			Content: []types.ClaudeContent{{
				Type:  "tool_use",
				ID:    callID,
				Name:  name,
				Input: parseFunctionCallArguments(arguments),
			}},
		}, nil

	case "function_call_output":
		callID, output, err := resolveFunctionCallOutputItem(item)
		if err != nil {
			return nil, err
		}

		return &types.ClaudeMessage{
			Role: "user",
			Content: []types.ClaudeContent{{
				Type:      "tool_result",
				ToolUseID: callID,
				Content:   output,
			}},
		}, nil

	default:
		return nil, fmt.Errorf("未知的 item type: %s", item.Type)
	}
}

// ============== Claude Response → Responses ==============

// ClaudeResponseToResponses 将 Claude 响应转换为 Responses 格式
func ClaudeResponseToResponses(claudeResp map[string]interface{}, sessionID string) (*types.ResponsesResponse, error) {
	// 提取字段
	model, _ := claudeResp["model"].(string)
	content, _ := claudeResp["content"].([]interface{})

	// 转换 output
	output := []types.ResponsesItem{}
	for _, c := range content {
		contentBlock, ok := c.(map[string]interface{})
		if !ok {
			continue
		}

		blockType, _ := contentBlock["type"].(string)
		switch blockType {
		case "thinking":
			thinking, _ := contentBlock["thinking"].(string)
			if thinking != "" {
				output = append(output, types.ResponsesItem{
					Type:    "reasoning",
					Status:  "completed",
					Summary: []interface{}{map[string]interface{}{"type": "summary_text", "text": thinking}},
				})
			}
		case "text":
			text, _ := contentBlock["text"].(string)
			output = append(output, types.ResponsesItem{
				Type:    "text",
				Content: text,
			})
		case "tool_use":
			id, _ := contentBlock["id"].(string)
			name, _ := contentBlock["name"].(string)
			arguments := ""
			if input, ok := contentBlock["input"]; ok {
				if argsJSON, err := JSONMarshal(input); err == nil {
					arguments = string(argsJSON)
				}
			}
			output = append(output, types.ResponsesItem{
				Type:      "function_call",
				CallID:    id,
				Name:      name,
				Arguments: arguments,
			})
		case "tool_result":
			toolUseID, _ := contentBlock["tool_use_id"].(string)
			output = append(output, types.ResponsesItem{
				Type:   "function_call_output",
				CallID: toolUseID,
				Output: contentBlock["content"],
			})
		}
	}

	// 提取 usage（使用统一入口自动检测格式）
	usage := ExtractUsageMetrics(claudeResp["usage"])

	// 生成 response ID
	responseID := generateResponseID()

	return &types.ResponsesResponse{
		ID:         responseID,
		Model:      model,
		Output:     output,
		Status:     "completed",
		PreviousID: "", // 将在外部设置
		Usage:      usage,
	}, nil
}

// ============== Responses → OpenAI Chat ==============

// ResponsesToOpenAIChatMessages 将 Responses 格式转换为 OpenAI Chat 格式
func ResponsesToOpenAIChatMessages(sess *session.Session, newInput interface{}, instructions string) ([]map[string]interface{}, error) {
	messages := []map[string]interface{}{}

	// 1. 处理 instructions（如果存在）
	if instructions != "" {
		messages = append(messages, map[string]interface{}{
			"role":    "system",
			"content": instructions,
		})
	}

	// 2. 处理历史消息
	messages = appendResponsesItemsToOpenAIMessages(messages, sess.Messages)

	// 3. 处理新输入
	newItems, err := parseResponsesInput(newInput)
	if err != nil {
		return nil, err
	}

	messages = appendResponsesItemsToOpenAIMessages(messages, newItems)

	return normalizeOpenAIToolCallMessages(messages), nil
}

func normalizeOpenAIToolCallMessages(messages []map[string]interface{}) []map[string]interface{} {
	return downgradeOrphanOpenAIToolMessages(normalizeOpenAIToolCallMessageOrder(messages))
}

func normalizeOpenAIToolCallMessageOrder(messages []map[string]interface{}) []map[string]interface{} {
	normalized := append([]map[string]interface{}(nil), messages...)

	for i := 0; i < len(normalized); i++ {
		pendingIDs := openAIToolCallIDs(normalized[i])
		if len(pendingIDs) == 0 {
			continue
		}

		toolMessages := make([]map[string]interface{}, 0, len(pendingIDs))
		deferredMessages := make([]map[string]interface{}, 0)
		end := i
		for j := i + 1; j < len(normalized) && len(pendingIDs) > 0; j++ {
			msg := normalized[j]
			if id := openAIToolMessageID(msg); id != "" {
				if _, ok := pendingIDs[id]; ok {
					toolMessages = append(toolMessages, msg)
					delete(pendingIDs, id)
				} else {
					deferredMessages = append(deferredMessages, msg)
				}
			} else {
				deferredMessages = append(deferredMessages, msg)
			}
			end = j
		}

		if len(pendingIDs) > 0 || len(deferredMessages) == 0 {
			continue
		}

		reordered := make([]map[string]interface{}, 0, len(normalized))
		reordered = append(reordered, normalized[:i+1]...)
		reordered = append(reordered, toolMessages...)
		reordered = append(reordered, deferredMessages...)
		reordered = append(reordered, normalized[end+1:]...)
		normalized = reordered
		i += len(toolMessages)
	}

	return normalized
}

func downgradeOrphanOpenAIToolMessages(messages []map[string]interface{}) []map[string]interface{} {
	normalized := make([]map[string]interface{}, 0, len(messages))
	var pendingIDs map[string]struct{}

	for _, msg := range messages {
		if role, _ := msg["role"].(string); role == "tool" {
			id := openAIToolMessageID(msg)
			if id != "" {
				if _, ok := pendingIDs[id]; ok {
					normalized = append(normalized, msg)
					delete(pendingIDs, id)
					continue
				}
			}

			// DeepSeek 等上游会拒绝孤立 tool 消息；降级为普通历史文本保留上下文。
			normalized = append(normalized, orphanOpenAIToolMessageAsUser(msg))
			pendingIDs = nil
			continue
		}

		normalized = append(normalized, msg)
		if ids := openAIToolCallIDs(msg); len(ids) > 0 {
			pendingIDs = ids
		} else {
			pendingIDs = nil
		}
	}

	return normalized
}

func orphanOpenAIToolMessageAsUser(msg map[string]interface{}) map[string]interface{} {
	callID, _ := msg["tool_call_id"].(string)
	content := stringifyOpenAIToolMessageContent(msg["content"])
	if callID != "" {
		content = fmt.Sprintf("Function call output (%s): %s", callID, content)
	} else {
		content = fmt.Sprintf("Function call output: %s", content)
	}
	return map[string]interface{}{
		"role":    "user",
		"content": content,
	}
}

func stringifyOpenAIToolMessageContent(content interface{}) string {
	switch v := content.(type) {
	case string:
		return v
	case nil:
		return ""
	default:
		data, err := JSONMarshal(v)
		if err != nil {
			return fmt.Sprintf("%v", v)
		}
		return string(data)
	}
}

func openAIToolCallIDs(msg map[string]interface{}) map[string]struct{} {
	if role, _ := msg["role"].(string); role != "assistant" {
		return nil
	}

	ids := map[string]struct{}{}
	switch toolCalls := msg["tool_calls"].(type) {
	case []map[string]interface{}:
		for _, toolCall := range toolCalls {
			if id, _ := toolCall["id"].(string); id != "" {
				ids[id] = struct{}{}
			}
		}
	case []interface{}:
		for _, rawToolCall := range toolCalls {
			toolCall, ok := rawToolCall.(map[string]interface{})
			if !ok {
				continue
			}
			if id, _ := toolCall["id"].(string); id != "" {
				ids[id] = struct{}{}
			}
		}
	case []types.OpenAIToolCall:
		for _, toolCall := range toolCalls {
			if toolCall.ID != "" {
				ids[toolCall.ID] = struct{}{}
			}
		}
	}

	if len(ids) == 0 {
		return nil
	}
	return ids
}

func openAIToolMessageID(msg map[string]interface{}) string {
	if role, _ := msg["role"].(string); role != "tool" {
		return ""
	}
	id, _ := msg["tool_call_id"].(string)
	return id
}

func appendResponsesItemsToOpenAIMessages(messages []map[string]interface{}, items []types.ResponsesItem) []map[string]interface{} {
	var pendingReasoning []string
	var pendingToolCalls []map[string]interface{}

	flushReasoning := func() {
		if len(pendingReasoning) == 0 {
			return
		}
		reasoning := strings.Join(pendingReasoning, "\n")
		if len(messages) > 0 {
			last := messages[len(messages)-1]
			if role, _ := last["role"].(string); role == "assistant" {
				if existing, ok := last["reasoning_content"].(string); ok && existing != "" {
					last["reasoning_content"] = existing + "\n" + reasoning
				} else {
					last["reasoning_content"] = reasoning
				}
				pendingReasoning = nil
				return
			}
		}
		messages = append(messages, map[string]interface{}{
			"role":              "assistant",
			"content":           "",
			"reasoning_content": reasoning,
		})
		pendingReasoning = nil
	}

	flushToolCalls := func() {
		if len(pendingToolCalls) == 0 {
			return
		}
		// 若最后一条消息是 assistant，将 tool_calls 合并进去（去重）
		// 避免产生连续 assistant 消息（DeepSeek 等上游不允许）
		if len(messages) > 0 {
			last := messages[len(messages)-1]
			if role, _ := last["role"].(string); role == "assistant" {
				if existing, hasTC := last["tool_calls"].([]map[string]interface{}); hasTC {
					// 已有 tool_calls，合并新 tool_calls（按 id 去重）
					last["tool_calls"] = mergeToolCalls(existing, pendingToolCalls)
				} else {
					last["tool_calls"] = pendingToolCalls
				}
				pendingToolCalls = nil
				return
			}
		}
		msg := map[string]interface{}{
			"role":       "assistant",
			"content":    "",
			"tool_calls": pendingToolCalls,
		}
		if len(pendingReasoning) > 0 {
			msg["reasoning_content"] = strings.Join(pendingReasoning, "\n")
			msg["content"] = ""
			pendingReasoning = nil
		}
		messages = append(messages, msg)
		pendingToolCalls = nil
	}

	for _, item := range items {
		item = types.NormalizeResponsesItem(item)
		if item.Type == "reasoning" {
			reasoning := extractResponsesReasoningText(item)
			if reasoning != "" {
				pendingReasoning = append(pendingReasoning, reasoning)
			}
			continue
		}

		msg := responsesItemToOpenAIMessage(item)
		if msg == nil {
			continue
		}

		// function_call 产生的 assistant+tool_calls 消息需要合并
		if role, _ := msg["role"].(string); role == "assistant" {
			if tc, ok := msg["tool_calls"].([]map[string]interface{}); ok && len(tc) > 0 {
				pendingToolCalls = append(pendingToolCalls, tc...)
				continue
			}
		}

		// 非 function_call 消息：先刷出待处理的 reasoning 和 tool_calls
		if len(pendingReasoning) > 0 {
			role, _ := msg["role"].(string)
			if role == "assistant" && len(pendingToolCalls) == 0 {
				msg["reasoning_content"] = strings.Join(pendingReasoning, "\n")
				if _, ok := msg["content"]; !ok {
					msg["content"] = ""
				}
				pendingReasoning = nil
			} else {
				flushToolCalls()
				flushReasoning()
			}
		} else {
			flushToolCalls()
		}

		messages = append(messages, msg)
	}

	flushToolCalls()
	flushReasoning()
	return messages
}

// mergeToolCalls 合并两组 tool_calls，按 id 去重（保留 existing 优先）
func mergeToolCalls(existing, incoming []map[string]interface{}) []map[string]interface{} {
	seen := make(map[string]bool, len(existing))
	for _, tc := range existing {
		if id, _ := tc["id"].(string); id != "" {
			seen[id] = true
		}
	}
	for _, tc := range incoming {
		id, _ := tc["id"].(string)
		if id == "" || seen[id] {
			continue
		}
		existing = append(existing, tc)
		seen[id] = true
	}
	return existing
}

// responsesItemToOpenAIMessage 单个 ResponsesItem 转换为 OpenAI Message
func responsesItemToOpenAIMessage(item types.ResponsesItem) map[string]interface{} {
	item = types.NormalizeResponsesItem(item)

	if item.Type == "tool_call" || item.Type == "tool_result" {
		return nil
	}

	switch item.Type {
	case "reasoning":
		reasoning := extractResponsesReasoningText(item)
		if reasoning == "" {
			return nil
		}
		return map[string]interface{}{
			"role":              "assistant",
			"content":           "",
			"reasoning_content": reasoning,
		}

	case "message":
		role, content := resolveResponsesChatMessageContent(item)
		if content == nil {
			return nil
		}

		return map[string]interface{}{
			"role":    role,
			"content": content,
		}

	case "text":
		role, content := resolveResponsesChatMessageContent(item)
		if content == nil {
			return nil
		}

		return map[string]interface{}{
			"role":    role,
			"content": content,
		}

	case "custom_tool_call":
		callID := item.CallID
		name := item.Name
		input := customToolInputFromItem(item)
		if callID == "" {
			callID = name
		}
		if name == "" {
			return nil
		}
		upstreamName, argsJSON := replayCustomToolCall(name, input)
		return map[string]interface{}{
			"role": "assistant",
			"tool_calls": []map[string]interface{}{{
				"id":   callID,
				"type": "function",
				"function": map[string]interface{}{
					"name":      upstreamName,
					"arguments": argsJSON,
				},
			}},
		}

	case "custom_tool_call_output":
		callID := item.CallID
		output := item.Output
		if callID == "" {
			callID = item.Name
		}
		contentStr := ""
		switch v := output.(type) {
		case string:
			contentStr = v
		case nil:
			contentStr = ""
		default:
			b, err := JSONMarshal(v)
			if err != nil {
				return nil
			}
			contentStr = string(b)
		}
		return map[string]interface{}{
			"role":         "tool",
			"tool_call_id": callID,
			"content":      contentStr,
		}

	case "function_call":
		callID, name, arguments, err := resolveFunctionCallItem(item)
		if err != nil {
			return nil
		}
		toolName := name
		if item.Namespace != "" {
			toolName = flattenNamespaceToolName(item.Namespace, name)
		}
		return map[string]interface{}{
			"role": "assistant",
			"tool_calls": []map[string]interface{}{{
				"id":   callID,
				"type": "function",
				"function": map[string]interface{}{
					"name":      toolName,
					"arguments": arguments,
				},
			}},
		}

	case "function_call_output":
		callID, output, err := resolveFunctionCallOutputItem(item)
		if err != nil {
			return nil
		}

		contentStr := ""
		switch output := output.(type) {
		case string:
			contentStr = output
		case nil:
			contentStr = ""
		default:
			contentJSON, err := JSONMarshal(output)
			if err != nil {
				return nil
			}
			contentStr = string(contentJSON)
		}

		return map[string]interface{}{
			"role":         "tool",
			"tool_call_id": callID,
			"content":      contentStr,
		}
	}

	return nil
}

// ============== OpenAI Chat Response → Responses ==============

// OpenAIChatResponseToResponses 将 OpenAI Chat 响应转换为 Responses 格式
func OpenAIChatResponseToResponses(openaiResp map[string]interface{}, sessionID string) (*types.ResponsesResponse, error) {
	// 提取字段
	model, _ := openaiResp["model"].(string)
	choices, _ := openaiResp["choices"].([]interface{})

	// 提取第一个 choice 的 message
	output := []types.ResponsesItem{}
	if len(choices) > 0 {
		choice, ok := choices[0].(map[string]interface{})
		if ok {
			message, _ := choice["message"].(map[string]interface{})
			if reasoning, _ := message["reasoning_content"].(string); reasoning != "" {
				output = append(output, types.ResponsesItem{
					Type:   "reasoning",
					Status: "completed",
					Summary: []interface{}{map[string]interface{}{
						"type": "summary_text",
						"text": reasoning,
					}},
				})
			}
			content, _ := message["content"].(string)
			if content != "" {
				output = append(output, types.ResponsesItem{
					Type: "message",
					Role: "assistant",
					Content: []types.ContentBlock{{
						Type: "output_text",
						Text: content,
					}},
				})
			}
			if toolCalls, ok := message["tool_calls"].([]interface{}); ok {
				for _, rawToolCall := range toolCalls {
					toolCall, ok := rawToolCall.(map[string]interface{})
					if !ok {
						continue
					}
					callID, _ := toolCall["id"].(string)
					function, _ := toolCall["function"].(map[string]interface{})
					name, _ := function["name"].(string)
					arguments, _ := function["arguments"].(string)
					output = append(output, types.ResponsesItem{
						Type:      "function_call",
						Status:    "completed",
						CallID:    callID,
						Name:      name,
						Arguments: arguments,
					})
				}
			}
		}
	}

	// 提取 usage（使用统一入口自动检测格式）
	usage := ExtractUsageMetrics(openaiResp["usage"])
	if shouldNormalizeOpenAICachedUsageForResponses(openaiResp["usage"]) {
		normalizeOpenAICachedUsageForResponses(&usage)
	}

	// 生成 response ID
	responseID := generateResponseID()

	return &types.ResponsesResponse{
		ID:         responseID,
		Model:      model,
		Output:     output,
		Status:     "completed",
		PreviousID: "",
		Usage:      usage,
	}, nil
}

// ============== 工具函数 ==============

// extractTextFromContent 从 content 中提取文本内容
// 支持三种格式：
// 1. string - 直接返回
// 2. []ContentBlock - 提取 input_text/output_text 类型的 text 字段
// 3. []interface{} - 动态解析为 ContentBlock
func extractTextFromContent(content interface{}) string {
	// 1. 如果是 string，直接返回
	if str, ok := content.(string); ok {
		return str
	}

	// 2. 如果是 []ContentBlock（已解析类型）
	if blocks, ok := content.([]types.ContentBlock); ok {
		texts := []string{}
		for _, block := range blocks {
			if block.Type == "input_text" || block.Type == "output_text" {
				texts = append(texts, block.Text)
			}
		}
		return strings.Join(texts, "\n")
	}

	// 3. 如果是 []interface{}（未解析类型）
	if arr, ok := content.([]interface{}); ok {
		texts := []string{}
		for _, c := range arr {
			if block, ok := c.(map[string]interface{}); ok {
				blockType, _ := block["type"].(string)
				if blockType == "input_text" || blockType == "output_text" {
					if text, ok := block["text"].(string); ok {
						texts = append(texts, text)
					}
				}
			}
		}
		return strings.Join(texts, "\n")
	}

	return ""
}

// parseResponsesInput 解析 input 字段（可能是 string 或 []ResponsesItem）
func parseResponsesInput(input interface{}) ([]types.ResponsesItem, error) {
	return types.ParseResponsesInput(input)
}

// generateResponseID 生成响应ID
func generateResponseID() string {
	return fmt.Sprintf("resp_%d", getCurrentTimestamp())
}

// getCurrentTimestamp 获取当前时间戳（毫秒）
func getCurrentTimestamp() int64 {
	return time.Now().UnixNano() / int64(time.Millisecond)
}

// ExtractTextFromResponses 从 Responses 消息中提取纯文本（用于 OpenAI Completions）
func ExtractTextFromResponses(sess *session.Session, newInput interface{}) (string, error) {
	texts := []string{}

	// 历史消息
	for _, item := range sess.Messages {
		if item.Type == "text" {
			if text, ok := item.Content.(string); ok {
				texts = append(texts, text)
			}
		}
	}

	// 新输入
	newItems, err := parseResponsesInput(newInput)
	if err != nil {
		return "", err
	}

	for _, item := range newItems {
		if item.Type == "text" {
			if text, ok := item.Content.(string); ok {
				texts = append(texts, text)
			}
		}
	}

	return strings.Join(texts, "\n"), nil
}

// OpenAICompletionsResponseToResponses OpenAI Completions 响应转 Responses
func OpenAICompletionsResponseToResponses(completionsResp map[string]interface{}, sessionID string) (*types.ResponsesResponse, error) {
	model, _ := completionsResp["model"].(string)
	choices, _ := completionsResp["choices"].([]interface{})

	output := []types.ResponsesItem{}
	if len(choices) > 0 {
		choice, ok := choices[0].(map[string]interface{})
		if ok {
			text, _ := choice["text"].(string)
			output = append(output, types.ResponsesItem{
				Type:    "text",
				Content: text,
			})
		}
	}

	// 提取 usage（使用统一入口自动检测格式）
	usage := ExtractUsageMetrics(completionsResp["usage"])

	responseID := generateResponseID()

	return &types.ResponsesResponse{
		ID:         responseID,
		Model:      model,
		Output:     output,
		Status:     "completed",
		PreviousID: "",
		Usage:      usage,
	}, nil
}

// JSONToMap 将 JSON 字节转为 map
func JSONToMap(data []byte) (map[string]interface{}, error) {
	var result map[string]interface{}
	err := json.Unmarshal(data, &result)
	return result, err
}

// getIntFromMap 从 map 中安全提取整数值
// 支持 float64（JSON 反序列化）和 int/int64（内部构造）两种类型
func getIntFromMap(m map[string]interface{}, key string) (int, bool) {
	v, exists := m[key]
	if !exists {
		return 0, false
	}
	switch val := v.(type) {
	case float64:
		return int(val), true
	case int:
		return val, true
	case int64:
		return int(val), true
	case int32:
		return int(val), true
	default:
		return 0, false
	}
}

func effectiveCacheCreationTokensInt(cacheCreation, cacheCreation5m, cacheCreation1h int) int {
	if cacheCreation > 0 {
		return cacheCreation
	}
	return cacheCreation5m + cacheCreation1h
}

func calculateClaudeTotalTokensInt(inputTokens, outputTokens, cacheRead, cacheCreation, cacheCreation5m, cacheCreation1h int) int {
	return inputTokens + outputTokens + cacheRead + effectiveCacheCreationTokensInt(cacheCreation, cacheCreation5m, cacheCreation1h)
}

func normalizeInputTokensWithCacheInt(inputTokens, cacheRead, cacheCreation, cacheCreation5m, cacheCreation1h int) int {
	cacheTokens := cacheRead + effectiveCacheCreationTokensInt(cacheCreation, cacheCreation5m, cacheCreation1h)
	if cacheTokens <= 0 {
		return inputTokens
	}
	normalized := inputTokens - cacheTokens
	if normalized < 0 {
		return 0
	}
	return normalized
}

func normalizeOpenAICachedUsageForResponses(usage *types.ResponsesUsage) {
	if usage == nil || usage.InputTokensDetails == nil || usage.InputTokensDetails.CachedTokens <= 0 {
		return
	}
	usage.InputTokens = normalizeInputTokensWithCacheInt(usage.InputTokens, usage.InputTokensDetails.CachedTokens, 0, 0, 0)
	usage.TotalTokens = calculateClaudeTotalTokensInt(usage.InputTokens, usage.OutputTokens, usage.InputTokensDetails.CachedTokens, 0, 0, 0)
}

func shouldNormalizeOpenAICachedUsageForResponses(usageRaw interface{}) bool {
	usageMap, ok := usageRaw.(map[string]interface{})
	if !ok {
		return false
	}
	if details, ok := usageMap["prompt_tokens_details"].(map[string]interface{}); ok {
		if cached, ok := getIntFromMap(details, "cached_tokens"); ok && cached > 0 {
			return true
		}
	}
	if _, hasCacheRead := usageMap["cache_read_input_tokens"]; hasCacheRead {
		return false
	}
	if _, hasCacheCreation := usageMap["cache_creation_input_tokens"]; hasCacheCreation {
		return false
	}
	if _, hasCacheCreation5m := usageMap["cache_creation_5m_input_tokens"]; hasCacheCreation5m {
		return false
	}
	if _, hasCacheCreation1h := usageMap["cache_creation_1h_input_tokens"]; hasCacheCreation1h {
		return false
	}
	if details, ok := usageMap["input_tokens_details"].(map[string]interface{}); ok {
		if cached, ok := getIntFromMap(details, "cached_tokens"); ok && cached > 0 {
			return true
		}
	}
	return false
}

// parseResponsesUsage 解析 Responses API 的 usage 字段
// 完整支持 OpenAI Responses API 的详细 usage 结构
func parseResponsesUsage(usageRaw interface{}) types.ResponsesUsage {
	usage := types.ResponsesUsage{}

	usageMap, ok := usageRaw.(map[string]interface{})
	if !ok {
		return usage
	}

	// 解析基础字段（兼容两种命名风格）
	// OpenAI Responses API: input_tokens / output_tokens
	// OpenAI Chat API: prompt_tokens / completion_tokens
	if v, ok := getIntFromMap(usageMap, "input_tokens"); ok {
		usage.InputTokens = v
	} else if v, ok := getIntFromMap(usageMap, "prompt_tokens"); ok {
		usage.InputTokens = v
	}

	if v, ok := getIntFromMap(usageMap, "output_tokens"); ok {
		usage.OutputTokens = v
	} else if v, ok := getIntFromMap(usageMap, "completion_tokens"); ok {
		usage.OutputTokens = v
	}

	// 解析 input_tokens_details（兼容 prompt_tokens_details）
	inputDetailsRaw := usageMap["input_tokens_details"]
	if inputDetailsRaw == nil {
		inputDetailsRaw = usageMap["prompt_tokens_details"]
	}
	if detailsMap, ok := inputDetailsRaw.(map[string]interface{}); ok {
		usage.InputTokensDetails = &types.InputTokensDetails{}
		if v, ok := getIntFromMap(detailsMap, "cached_tokens"); ok {
			usage.InputTokensDetails.CachedTokens = v
		}
	}

	if v, ok := getIntFromMap(usageMap, "total_tokens"); ok {
		usage.TotalTokens = v
	} else {
		usage.TotalTokens = usage.InputTokens + usage.OutputTokens
	}

	// 解析 output_tokens_details（兼容 completion_tokens_details）
	outputDetailsRaw := usageMap["output_tokens_details"]
	if outputDetailsRaw == nil {
		outputDetailsRaw = usageMap["completion_tokens_details"]
	}
	if detailsMap, ok := outputDetailsRaw.(map[string]interface{}); ok {
		usage.OutputTokensDetails = &types.OutputTokensDetails{}
		if v, ok := getIntFromMap(detailsMap, "reasoning_tokens"); ok {
			usage.OutputTokensDetails.ReasoningTokens = v
		}
	}

	return usage
}

// parseClaudeUsage 解析 Claude API 的 usage 字段
// 完整支持 Claude 的缓存统计，包括 TTL 细分 (5m/1h)
// 参考 claude-code-hub 的 extractUsageMetrics 实现
func parseClaudeUsage(usageRaw interface{}) types.ResponsesUsage {
	usage := types.ResponsesUsage{}

	usageMap, ok := usageRaw.(map[string]interface{})
	if !ok {
		return usage
	}

	// 基础字段
	if v, ok := getIntFromMap(usageMap, "input_tokens"); ok {
		usage.InputTokens = v
	}
	if v, ok := getIntFromMap(usageMap, "output_tokens"); ok {
		usage.OutputTokens = v
	}

	// Claude 缓存创建统计（区分 TTL）
	var cacheCreation, cacheCreation5m, cacheCreation1h int
	var has5m, has1h bool

	// 总缓存创建量
	if v, ok := getIntFromMap(usageMap, "cache_creation_input_tokens"); ok {
		cacheCreation = v
		usage.CacheCreationInputTokens = cacheCreation
	}

	// 5分钟 TTL 缓存创建
	if v, ok := getIntFromMap(usageMap, "cache_creation_5m_input_tokens"); ok {
		cacheCreation5m = v
		usage.CacheCreation5mInputTokens = cacheCreation5m
		has5m = cacheCreation5m > 0
	}

	// 1小时 TTL 缓存创建
	if v, ok := getIntFromMap(usageMap, "cache_creation_1h_input_tokens"); ok {
		cacheCreation1h = v
		usage.CacheCreation1hInputTokens = cacheCreation1h
		has1h = cacheCreation1h > 0
	}

	// 缓存读取
	var cacheRead int
	if v, ok := getIntFromMap(usageMap, "cache_read_input_tokens"); ok {
		cacheRead = v
		usage.CacheReadInputTokens = cacheRead
	}

	// 设置缓存 TTL 标识
	if has5m && has1h {
		usage.CacheTTL = "mixed"
	} else if has1h {
		usage.CacheTTL = "1h"
	} else if has5m {
		usage.CacheTTL = "5m"
	}

	// 同时设置 InputTokensDetails（兼容 OpenAI 格式）
	// CachedTokens = cache_read（仅缓存读取，不包含缓存创建）
	// 注意：cache_creation 是新创建的缓存，不是"已缓存的 token"
	if cacheRead > 0 {
		usage.InputTokensDetails = &types.InputTokensDetails{
			CachedTokens: cacheRead,
		}
	}

	usage.TotalTokens = calculateClaudeTotalTokensInt(
		usage.InputTokens,
		usage.OutputTokens,
		cacheRead,
		cacheCreation,
		cacheCreation5m,
		cacheCreation1h,
	)

	return usage
}

// parseGeminiUsage 解析 Gemini API 的 usage 字段
// Gemini 使用 promptTokenCount/candidatesTokenCount，需要特殊处理缓存去重
// 参考 claude-code-hub: Gemini 的 promptTokenCount 已包含 cachedContentTokenCount，需要扣除避免重复计费
func parseGeminiUsage(usageRaw interface{}) types.ResponsesUsage {
	usage := types.ResponsesUsage{}

	usageMap, ok := usageRaw.(map[string]interface{})
	if !ok {
		return usage
	}

	var promptTokens, cachedTokens, outputTokens int

	// Gemini 字段名
	if v, ok := getIntFromMap(usageMap, "promptTokenCount"); ok {
		promptTokens = v
	}
	if v, ok := getIntFromMap(usageMap, "cachedContentTokenCount"); ok {
		cachedTokens = v
	}
	if v, ok := getIntFromMap(usageMap, "candidatesTokenCount"); ok {
		outputTokens = v
	}

	// 关键处理：Gemini 的 promptTokenCount 已包含 cachedContentTokenCount
	// 为避免重复计费，实际输入 token = promptTokenCount - cachedContentTokenCount
	actualInputTokens := promptTokens - cachedTokens
	if actualInputTokens < 0 {
		actualInputTokens = 0
	}

	usage.InputTokens = actualInputTokens
	usage.OutputTokens = outputTokens
	usage.TotalTokens = actualInputTokens + outputTokens

	// 缓存读取统计
	if cachedTokens > 0 {
		usage.CacheReadInputTokens = cachedTokens
		usage.InputTokensDetails = &types.InputTokensDetails{
			CachedTokens: cachedTokens,
		}
	}

	return usage
}

// ExtractUsageMetrics 多格式 Token 提取统一入口
// 自动检测并解析 Claude/Gemini/OpenAI 三种格式的 usage
// 参考 claude-code-hub 的 extractUsageMetrics 实现
func ExtractUsageMetrics(usageRaw interface{}) types.ResponsesUsage {
	usageMap, ok := usageRaw.(map[string]interface{})
	if !ok {
		return types.ResponsesUsage{}
	}

	_, hasInputDetails := usageMap["input_tokens_details"]
	if !hasInputDetails {
		_, hasInputDetails = usageMap["prompt_tokens_details"]
	}

	// 1. 检测 Claude 格式：有 cache_creation_input_tokens 或没有 OpenAI details 的 cache_read_input_tokens
	if _, hasCacheCreation := usageMap["cache_creation_input_tokens"]; hasCacheCreation {
		return parseClaudeUsage(usageRaw)
	}
	if _, hasCacheRead := usageMap["cache_read_input_tokens"]; hasCacheRead {
		if !hasInputDetails {
			return parseClaudeUsage(usageRaw)
		}
	}
	if _, hasCacheCreation5m := usageMap["cache_creation_5m_input_tokens"]; hasCacheCreation5m {
		return parseClaudeUsage(usageRaw)
	}
	if _, hasCacheCreation1h := usageMap["cache_creation_1h_input_tokens"]; hasCacheCreation1h {
		return parseClaudeUsage(usageRaw)
	}

	// 2. 检测 Gemini 格式：有 promptTokenCount
	if _, hasPromptTokenCount := usageMap["promptTokenCount"]; hasPromptTokenCount {
		return parseGeminiUsage(usageRaw)
	}

	// 3. 默认 OpenAI 格式
	return parseResponsesUsage(usageRaw)
}
