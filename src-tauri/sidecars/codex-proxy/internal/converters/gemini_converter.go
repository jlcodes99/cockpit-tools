package converters

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// ============== Gemini -> Claude/OpenAI 转换器 ==============

// GeminiToClaudeRequest 将 Gemini 请求转换为 Claude Messages API 格式
func GeminiToClaudeRequest(geminiReq *types.GeminiRequest, model string) (map[string]interface{}, error) {
	claudeReq := map[string]interface{}{
		"model": model,
	}

	// 1. 转换 systemInstruction -> system
	if geminiReq.SystemInstruction != nil && len(geminiReq.SystemInstruction.Parts) > 0 {
		systemText := extractTextFromGeminiParts(geminiReq.SystemInstruction.Parts)
		if systemText != "" {
			claudeReq["system"] = systemText
		}
	}

	// 2. 转换 contents -> messages
	messages := []map[string]interface{}{}
	for _, content := range geminiReq.Contents {
		msg, err := geminiContentToClaudeMessage(&content)
		if err != nil {
			return nil, err
		}
		if msg != nil {
			messages = append(messages, msg)
		}
	}
	claudeReq["messages"] = messages

	// 3. 转换 generationConfig
	if geminiReq.GenerationConfig != nil {
		cfg := geminiReq.GenerationConfig
		if cfg.MaxOutputTokens > 0 {
			claudeReq["max_tokens"] = cfg.MaxOutputTokens
		}
		if cfg.Temperature != nil {
			claudeReq["temperature"] = *cfg.Temperature
		}
		if cfg.TopP != nil {
			claudeReq["top_p"] = *cfg.TopP
		}
		if cfg.TopK != nil {
			claudeReq["top_k"] = *cfg.TopK
		}
		if len(cfg.StopSequences) > 0 {
			claudeReq["stop_sequences"] = cfg.StopSequences
		}
	}

	// 4. 转换 tools -> tools
	if len(geminiReq.Tools) > 0 {
		claudeTools := []map[string]interface{}{}
		for _, tool := range geminiReq.Tools {
			for _, fn := range tool.FunctionDeclarations {
				claudeTool := map[string]interface{}{
					"name": fn.Name,
				}
				if fn.Description != "" {
					claudeTool["description"] = fn.Description
				}
				if fn.Parameters != nil {
					claudeTool["input_schema"] = fn.Parameters
				} else {
					// Claude 需要 input_schema，提供空 schema
					claudeTool["input_schema"] = map[string]interface{}{
						"type":       "object",
						"properties": map[string]interface{}{},
					}
				}
				claudeTools = append(claudeTools, claudeTool)
			}
		}
		if len(claudeTools) > 0 {
			claudeReq["tools"] = claudeTools
		}
	}

	return claudeReq, nil
}

// GeminiToOpenAIRequest 将 Gemini 请求转换为 OpenAI Chat Completions 格式
func GeminiToOpenAIRequest(geminiReq *types.GeminiRequest, model string) (map[string]interface{}, error) {
	openaiReq := map[string]interface{}{
		"model": model,
	}

	messages := []map[string]interface{}{}

	// 1. 转换 systemInstruction -> system message
	if geminiReq.SystemInstruction != nil && len(geminiReq.SystemInstruction.Parts) > 0 {
		systemText := extractTextFromGeminiParts(geminiReq.SystemInstruction.Parts)
		if systemText != "" {
			messages = append(messages, map[string]interface{}{
				"role":    "system",
				"content": systemText,
			})
		}
	}

	// 2. 转换 contents -> messages
	for _, content := range geminiReq.Contents {
		msg, err := geminiContentToOpenAIMessage(&content)
		if err != nil {
			return nil, err
		}
		if msg != nil {
			messages = append(messages, msg)
		}
	}
	openaiReq["messages"] = messages

	// 3. 转换 generationConfig
	if geminiReq.GenerationConfig != nil {
		cfg := geminiReq.GenerationConfig
		if cfg.MaxOutputTokens > 0 {
			openaiReq["max_tokens"] = cfg.MaxOutputTokens
		}
		if cfg.Temperature != nil {
			openaiReq["temperature"] = *cfg.Temperature
		}
		if cfg.TopP != nil {
			openaiReq["top_p"] = *cfg.TopP
		}
		if len(cfg.StopSequences) > 0 {
			openaiReq["stop"] = cfg.StopSequences
		}
	}

	// 4. 转换 tools -> tools
	if len(geminiReq.Tools) > 0 {
		openaiTools := []map[string]interface{}{}
		for _, tool := range geminiReq.Tools {
			for _, fn := range tool.FunctionDeclarations {
				openaiTool := map[string]interface{}{
					"type": "function",
					"function": map[string]interface{}{
						"name":        fn.Name,
						"description": fn.Description,
						"parameters":  fn.Parameters,
					},
				}
				openaiTools = append(openaiTools, openaiTool)
			}
		}
		if len(openaiTools) > 0 {
			openaiReq["tools"] = openaiTools
		}
	}

	return openaiReq, nil
}

// ============== Claude/OpenAI -> Gemini 响应转换 ==============

// ClaudeResponseToGemini 将 Claude 响应转换为 Gemini 格式
func ClaudeResponseToGemini(claudeResp map[string]interface{}) (*types.GeminiResponse, error) {
	geminiResp := &types.GeminiResponse{
		Candidates: []types.GeminiCandidate{},
	}

	// 1. 转换 content -> candidates[0].content.parts
	content, ok := claudeResp["content"].([]interface{})
	if !ok {
		return geminiResp, nil
	}

	parts := []types.GeminiPart{}
	for _, c := range content {
		contentBlock, ok := c.(map[string]interface{})
		if !ok {
			continue
		}

		blockType, _ := contentBlock["type"].(string)
		switch blockType {
		case "thinking":
			if thinking, _ := contentBlock["thinking"].(string); thinking != "" {
				parts = append(parts, types.GeminiPart{
					Text:    thinking,
					Thought: true,
				})
			}
		case "text":
			text, _ := contentBlock["text"].(string)
			parts = append(parts, types.GeminiPart{
				Text: text,
			})
		case "tool_use":
			name, _ := contentBlock["name"].(string)
			args, _ := contentBlock["input"].(map[string]interface{})

			functionCall := &types.GeminiFunctionCall{
				Name: name,
				Args: args,
			}

			// 处理 thought_signature:
			// 1. 如果 Claude 响应中包含 signature，保留原值
			// 2. 否则使用 dummy signature 跳过 Gemini 验证
			if signature, ok := contentBlock["signature"].(string); ok && signature != "" {
				functionCall.ThoughtSignature = signature
			} else {
				functionCall.ThoughtSignature = types.DummyThoughtSignature
			}

			parts = append(parts, types.GeminiPart{
				FunctionCall: functionCall,
			})
		}
	}

	// 2. 转换 stop_reason -> finishReason
	finishReason := "STOP"
	if stopReason, ok := claudeResp["stop_reason"].(string); ok {
		finishReason = claudeStopReasonToGemini(stopReason)
	}

	candidate := types.GeminiCandidate{
		Content: &types.GeminiContent{
			Parts: parts,
			Role:  "model",
		},
		FinishReason: finishReason,
		Index:        0,
	}
	geminiResp.Candidates = append(geminiResp.Candidates, candidate)

	// 3. 转换 usage -> usageMetadata
	if usageRaw, ok := claudeResp["usage"].(map[string]interface{}); ok {
		inputTokens, _ := getIntFromMap(usageRaw, "input_tokens")
		outputTokens, _ := getIntFromMap(usageRaw, "output_tokens")
		cacheRead, _ := getIntFromMap(usageRaw, "cache_read_input_tokens")

		geminiResp.UsageMetadata = &types.GeminiUsageMetadata{
			PromptTokenCount:        inputTokens + cacheRead, // Gemini 格式包含缓存
			CandidatesTokenCount:    outputTokens,
			TotalTokenCount:         inputTokens + cacheRead + outputTokens,
			CachedContentTokenCount: cacheRead,
		}
	}

	return geminiResp, nil
}

// OpenAIResponseToGemini 将 OpenAI 响应转换为 Gemini 格式
func OpenAIResponseToGemini(openaiResp map[string]interface{}) (*types.GeminiResponse, error) {
	geminiResp := &types.GeminiResponse{
		Candidates: []types.GeminiCandidate{},
	}

	// 1. 转换 choices[0].message -> candidates[0].content
	choices, ok := openaiResp["choices"].([]interface{})
	if !ok || len(choices) == 0 {
		return geminiResp, nil
	}

	choice, ok := choices[0].(map[string]interface{})
	if !ok {
		return geminiResp, nil
	}

	parts := []types.GeminiPart{}
	finishReason := "STOP"

	// 处理 message
	if message, ok := choice["message"].(map[string]interface{}); ok {
		if reasoning, ok := message["reasoning_content"].(string); ok && reasoning != "" {
			parts = append(parts, types.GeminiPart{
				Text:    reasoning,
				Thought: true,
			})
		}

		// 文本内容
		if content, ok := message["content"].(string); ok && content != "" {
			parts = append(parts, types.GeminiPart{
				Text: content,
			})
		}

		// 工具调用
		if toolCalls, ok := message["tool_calls"].([]interface{}); ok {
			for _, tc := range toolCalls {
				toolCall, ok := tc.(map[string]interface{})
				if !ok {
					continue
				}
				function, ok := toolCall["function"].(map[string]interface{})
				if !ok {
					continue
				}
				name, _ := function["name"].(string)
				argsStr, _ := function["arguments"].(string)
				var args map[string]interface{}
				if argsStr != "" {
					_ = JSONUnmarshal([]byte(argsStr), &args)
				}

				// OpenAI 响应不包含 signature，统一使用 dummy signature
				parts = append(parts, types.GeminiPart{
					FunctionCall: &types.GeminiFunctionCall{
						Name:             name,
						Args:             args,
						ThoughtSignature: types.DummyThoughtSignature,
					},
				})
			}
		}
	}

	// 转换 finish_reason
	if fr, ok := choice["finish_reason"].(string); ok {
		finishReason = openaiFinishReasonToGemini(fr)
	}

	candidate := types.GeminiCandidate{
		Content: &types.GeminiContent{
			Parts: parts,
			Role:  "model",
		},
		FinishReason: finishReason,
		Index:        0,
	}
	geminiResp.Candidates = append(geminiResp.Candidates, candidate)

	// 2. 转换 usage -> usageMetadata
	if usageRaw, ok := openaiResp["usage"].(map[string]interface{}); ok {
		promptTokens, _ := getIntFromMap(usageRaw, "prompt_tokens")
		completionTokens, _ := getIntFromMap(usageRaw, "completion_tokens")

		geminiResp.UsageMetadata = &types.GeminiUsageMetadata{
			PromptTokenCount:     promptTokens,
			CandidatesTokenCount: completionTokens,
			TotalTokenCount:      promptTokens + completionTokens,
		}
	}

	return geminiResp, nil
}

// ============== 辅助函数 ==============

// geminiContentToClaudeMessage 将 Gemini Content 转换为 Claude Message
func geminiContentToClaudeMessage(content *types.GeminiContent) (map[string]interface{}, error) {
	if content == nil || len(content.Parts) == 0 {
		return nil, nil
	}

	// 角色转换: model -> assistant, user -> user
	role := content.Role
	if role == "model" {
		role = "assistant"
	}
	if role == "" {
		role = "user"
	}

	claudeContent := []map[string]interface{}{}

	for _, part := range content.Parts {
		if part.Text != "" {
			if part.Thought && role == "assistant" {
				claudeContent = append(claudeContent, map[string]interface{}{
					"type":     "thinking",
					"thinking": part.Text,
				})
				continue
			}
			claudeContent = append(claudeContent, map[string]interface{}{
				"type": "text",
				"text": part.Text,
			})
		}

		if part.InlineData != nil {
			// 图片转换
			claudeContent = append(claudeContent, map[string]interface{}{
				"type": "image",
				"source": map[string]interface{}{
					"type":       "base64",
					"media_type": part.InlineData.MimeType,
					"data":       part.InlineData.Data,
				},
			})
		}

		if part.FunctionCall != nil {
			// 工具调用
			toolUseID := part.FunctionCall.Name
			if toolUseID == "" {
				continue
			}
			claudeContent = append(claudeContent, map[string]interface{}{
				"type":  "tool_use",
				"id":    toolUseID,
				"name":  part.FunctionCall.Name,
				"input": part.FunctionCall.Args,
			})
		}

		if part.FunctionResponse != nil {
			// 工具结果 - Claude 需要单独的 tool_result 消息
			// 这里简化处理，将其作为 tool_result 内容块
			claudeContent = append(claudeContent, map[string]interface{}{
				"type":        "tool_result",
				"tool_use_id": part.FunctionResponse.Name,
				"content":     part.FunctionResponse.Response,
			})
		}
	}

	if len(claudeContent) == 0 {
		return nil, nil
	}

	return map[string]interface{}{
		"role":    role,
		"content": claudeContent,
	}, nil
}

// geminiContentToOpenAIMessage 将 Gemini Content 转换为 OpenAI Message
func geminiContentToOpenAIMessage(content *types.GeminiContent) (map[string]interface{}, error) {
	if content == nil || len(content.Parts) == 0 {
		return nil, nil
	}

	// 角色转换: model -> assistant, user -> user
	role := content.Role
	if role == "model" {
		role = "assistant"
	}
	if role == "" {
		role = "user"
	}

	// 检查是否有工具调用
	var toolCalls []map[string]interface{}
	var textParts []string
	var reasoningParts []string
	var hasToolResponse bool
	var toolResponseName string
	var toolResponseContent interface{}

	for idx, part := range content.Parts {
		if part.Text != "" {
			if part.Thought && role == "assistant" {
				reasoningParts = append(reasoningParts, part.Text)
				continue
			}
			textParts = append(textParts, part.Text)
		}

		if part.FunctionCall != nil {
			functionName := part.FunctionCall.Name
			if functionName == "" {
				continue
			}
			toolCallID := fmt.Sprintf("%s_%d", functionName, idx)
			argsJSON, _ := JSONMarshal(part.FunctionCall.Args)
			toolCalls = append(toolCalls, map[string]interface{}{
				"id":   toolCallID,
				"type": "function",
				"function": map[string]interface{}{
					"name":      functionName,
					"arguments": string(argsJSON),
				},
			})
		}

		if part.FunctionResponse != nil {
			hasToolResponse = true
			toolResponseName = part.FunctionResponse.Name
			toolResponseContent = part.FunctionResponse.Response
		}
	}

	// 如果是工具响应，返回 tool role 的消息
	if hasToolResponse {
		contentStr := ""
		if str, ok := toolResponseContent.(string); ok {
			contentStr = str
		} else {
			contentBytes, _ := JSONMarshal(toolResponseContent)
			contentStr = string(contentBytes)
		}
		return map[string]interface{}{
			"role":         "tool",
			"tool_call_id": toolResponseName,
			"content":      contentStr,
		}, nil
	}

	msg := map[string]interface{}{
		"role": role,
	}

	// 设置内容
	if len(toolCalls) > 0 {
		// 助手消息带工具调用
		if len(textParts) > 0 {
			msg["content"] = strings.Join(textParts, "\n")
		} else {
			msg["content"] = nil
		}
		msg["tool_calls"] = toolCalls
	} else {
		// 普通消息
		msg["content"] = strings.Join(textParts, "\n")
	}
	if len(reasoningParts) > 0 && role == "assistant" {
		msg["reasoning_content"] = strings.Join(reasoningParts, "\n")
		if _, ok := msg["content"]; !ok {
			msg["content"] = nil
		}
	}

	return msg, nil
}

// extractTextFromGeminiParts 从 Gemini Parts 中提取文本
func extractTextFromGeminiParts(parts []types.GeminiPart) string {
	texts := []string{}
	for _, part := range parts {
		if part.Text != "" {
			texts = append(texts, part.Text)
		}
	}
	return strings.Join(texts, "\n")
}

// claudeStopReasonToGemini 将 Claude 停止原因转换为 Gemini 格式
func claudeStopReasonToGemini(stopReason string) string {
	switch stopReason {
	case "end_turn", "stop_sequence":
		return "STOP"
	case "max_tokens":
		return "MAX_TOKENS"
	case "tool_use":
		return "STOP" // Gemini 使用相同的 STOP 表示工具调用
	default:
		return "STOP"
	}
}

// openaiFinishReasonToGemini 将 OpenAI 停止原因转换为 Gemini 格式
func openaiFinishReasonToGemini(finishReason string) string {
	switch finishReason {
	case "stop":
		return "STOP"
	case "length":
		return "MAX_TOKENS"
	case "tool_calls":
		return "STOP"
	case "content_filter":
		return "SAFETY"
	default:
		return "STOP"
	}
}

// geminiFinishReasonToClaude 将 Gemini 停止原因转换为 Claude 格式
func geminiFinishReasonToClaude(finishReason string) string {
	switch finishReason {
	case "STOP":
		return "end_turn"
	case "MAX_TOKENS":
		return "max_tokens"
	case "SAFETY", "RECITATION":
		return "end_turn"
	default:
		return "end_turn"
	}
}

// geminiFinishReasonToOpenAI 将 Gemini 停止原因转换为 OpenAI 格式
func geminiFinishReasonToOpenAI(finishReason string) string {
	switch finishReason {
	case "STOP":
		return "stop"
	case "MAX_TOKENS":
		return "length"
	case "SAFETY":
		return "content_filter"
	default:
		return "stop"
	}
}

// JSONMarshal JSON 序列化包装函数
func JSONMarshal(v interface{}) ([]byte, error) {
	return json.Marshal(v)
}

// JSONUnmarshal JSON 反序列化包装函数
func JSONUnmarshal(data []byte, v interface{}) error {
	return json.Unmarshal(data, v)
}
