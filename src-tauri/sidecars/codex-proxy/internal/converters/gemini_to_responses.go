package converters

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// ============== Gemini → Responses 请求转换 ==============

// GeminiToResponsesRequest 将 Gemini 请求转换为 Responses 格式
func GeminiToResponsesRequest(geminiReq *types.GeminiRequest, modelName string) (map[string]interface{}, error) {
	responsesReq := map[string]interface{}{
		"model": modelName,
	}

	// 1. 转换 systemInstruction → instructions
	if geminiReq.SystemInstruction != nil && len(geminiReq.SystemInstruction.Parts) > 0 {
		systemText := extractTextFromGeminiParts(geminiReq.SystemInstruction.Parts)
		if systemText != "" {
			responsesReq["instructions"] = systemText
		}
	}

	// 2. 转换 contents → input
	input := []types.ResponsesItem{}
	for _, content := range geminiReq.Contents {
		items := geminiContentToResponsesItems(&content)
		input = append(input, items...)
	}
	responsesReq["input"] = input

	// 3. 转换 generationConfig
	if geminiReq.GenerationConfig != nil {
		cfg := geminiReq.GenerationConfig
		if cfg.MaxOutputTokens > 0 {
			responsesReq["max_output_tokens"] = cfg.MaxOutputTokens
		}
		if cfg.Temperature != nil {
			responsesReq["temperature"] = *cfg.Temperature
		}
		if cfg.TopP != nil {
			responsesReq["top_p"] = *cfg.TopP
		}
	}

	// 4. 转换 tools
	if len(geminiReq.Tools) > 0 {
		responsesTools := []map[string]interface{}{}
		for _, tool := range geminiReq.Tools {
			for _, fn := range tool.FunctionDeclarations {
				responsesTool := map[string]interface{}{
					"type": "function",
					"name": fn.Name,
				}
				if fn.Description != "" {
					responsesTool["description"] = fn.Description
				}
				if fn.Parameters != nil {
					responsesTool["parameters"] = fn.Parameters
				}
				responsesTools = append(responsesTools, responsesTool)
			}
		}
		if len(responsesTools) > 0 {
			responsesReq["tools"] = responsesTools
		}
	}

	return responsesReq, nil
}

// geminiContentToResponsesItems 将 Gemini Content 转换为 Responses Items
func geminiContentToResponsesItems(content *types.GeminiContent) []types.ResponsesItem {
	if content == nil || len(content.Parts) == 0 {
		return nil
	}

	items := []types.ResponsesItem{}

	// 角色转换: model → assistant, user → user
	role := content.Role
	if role == "model" {
		role = "assistant"
	}
	if role == "" {
		role = "user"
	}

	// 收集文本内容
	var textParts []string
	for _, part := range content.Parts {
		if part.Text != "" {
			if part.Thought && role == "assistant" {
				items = append(items, types.ResponsesItem{
					Type: "reasoning",
					Summary: []interface{}{map[string]interface{}{
						"type": "summary_text",
						"text": part.Text,
					}},
				})
				continue
			}
			textParts = append(textParts, part.Text)
		}
	}

	// 如果有文本，创建 message item
	if len(textParts) > 0 {
		items = append(items, types.ResponsesItem{
			Type: "message",
			Role: role,
			Content: []types.ContentBlock{
				{
					Type: "input_text",
					Text: strings.Join(textParts, "\n"),
				},
			},
		})
	}

	// 处理 function call
	for _, part := range content.Parts {
		if part.FunctionCall != nil {
			argsJSON, _ := JSONMarshal(part.FunctionCall.Args)
			items = append(items, types.ResponsesItem{
				Type:      "function_call",
				Role:      role,
				CallID:    part.FunctionCall.Name,
				Name:      part.FunctionCall.Name,
				Arguments: string(argsJSON),
			})
		}
	}

	// 处理 function response
	for _, part := range content.Parts {
		if part.FunctionResponse != nil {
			items = append(items, types.ResponsesItem{
				Type:   "function_call_output",
				Role:   role,
				CallID: part.FunctionResponse.Name,
				Output: part.FunctionResponse.Response,
			})
		}
	}

	return items
}

// ============== Responses → Gemini 响应转换 ==============

// ResponsesResponseToGemini 将 Responses 响应转换为 Gemini 格式
func ResponsesResponseToGemini(responsesResp map[string]interface{}) (*types.GeminiResponse, error) {
	geminiResp := &types.GeminiResponse{
		Candidates: []types.GeminiCandidate{},
	}

	// 1. 转换 output → candidates[0].content.parts
	output, ok := responsesResp["output"].([]interface{})
	if !ok {
		return geminiResp, nil
	}

	parts := []types.GeminiPart{}
	for _, o := range output {
		item, ok := o.(map[string]interface{})
		if !ok {
			continue
		}

		itemType, _ := item["type"].(string)
		switch itemType {
		case "message":
			// 提取文本内容
			content := item["content"]
			if contentArr, ok := content.([]interface{}); ok {
				for _, c := range contentArr {
					contentBlock, ok := c.(map[string]interface{})
					if !ok {
						continue
					}
					blockType, _ := contentBlock["type"].(string)
					if blockType == "output_text" || blockType == "text" {
						if text, ok := contentBlock["text"].(string); ok {
							parts = append(parts, types.GeminiPart{
								Text: text,
							})
						}
					}
				}
			}

		case "function_call":
			name, _ := item["name"].(string)
			argsStr, _ := item["arguments"].(string)
			var args map[string]interface{}
			if argsStr != "" {
				_ = JSONUnmarshal([]byte(argsStr), &args)
			}

			parts = append(parts, types.GeminiPart{
				FunctionCall: &types.GeminiFunctionCall{
					Name:             name,
					Args:             args,
					ThoughtSignature: types.DummyThoughtSignature,
				},
			})

		case "function_call_output":
			name, _ := item["call_id"].(string)
			output := item["output"]
			if name == "" {
				continue
			}

			response := map[string]interface{}{"result": output}
			if outputMap, ok := output.(map[string]interface{}); ok {
				response = outputMap
			}

			parts = append(parts, types.GeminiPart{
				FunctionResponse: &types.GeminiFunctionResponse{
					Name:     name,
					Response: response,
				},
			})
		}
	}

	// 2. 转换 status → finishReason
	finishReason := "STOP"
	if status, ok := responsesResp["status"].(string); ok {
		finishReason = responsesStatusToGemini(status)
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

	// 3. 转换 usage → usageMetadata
	if usageRaw, ok := responsesResp["usage"].(map[string]interface{}); ok {
		inputTokens, _ := getIntFromMap(usageRaw, "input_tokens")
		outputTokens, _ := getIntFromMap(usageRaw, "output_tokens")
		cachedTokens, _ := getIntFromMap(usageRaw, "cache_read_input_tokens")
		if cachedTokens == 0 {
			if details, ok := usageRaw["input_tokens_details"].(map[string]interface{}); ok {
				cachedTokens, _ = getIntFromMap(details, "cached_tokens")
			}
		}

		geminiResp.UsageMetadata = &types.GeminiUsageMetadata{
			PromptTokenCount:        inputTokens + cachedTokens,
			CandidatesTokenCount:    outputTokens,
			TotalTokenCount:         inputTokens + cachedTokens + outputTokens,
			CachedContentTokenCount: cachedTokens,
		}
	}

	return geminiResp, nil
}

// responsesStatusToGemini 将 Responses status 转换为 Gemini finishReason
func responsesStatusToGemini(status string) string {
	switch status {
	case "completed":
		return "STOP"
	case "incomplete":
		return "MAX_TOKENS"
	case "failed":
		return "SAFETY"
	default:
		return "STOP"
	}
}

// ============== Responses → Gemini 流式转换 ==============

// responsesToGeminiStreamState 流式转换状态
type responsesToGeminiStreamState struct {
	Seq              int
	CurrentCandidate *types.GeminiCandidate
	TextBuf          strings.Builder
	FunctionCalls    []*types.GeminiFunctionCall
	FirstChunk       bool
	PendingEventType string // 缓存上一行的 event: 类型
	// 工具调用收集
	CurrentFuncName string
	CurrentFuncArgs strings.Builder
}

// ConvertResponsesToGeminiStream 将 Responses SSE 转换为 Gemini SSE
func ConvertResponsesToGeminiStream(ctx context.Context, modelName string, rawJSON []byte, param *any) []string {
	if *param == nil {
		*param = &responsesToGeminiStreamState{
			FirstChunk: true,
		}
	}
	st := (*param).(*responsesToGeminiStreamState)

	// 解析 SSE 行（逐行输入，需要缓存 event: 类型）
	line := string(rawJSON)
	line = strings.TrimSpace(line)

	// 处理 event: 行
	if strings.HasPrefix(line, "event: ") {
		st.PendingEventType = strings.TrimPrefix(line, "event: ")
		return []string{} // event: 行不产生输出，等待后续 data: 行
	}

	// 处理 data: 行
	if !strings.HasPrefix(line, "data: ") {
		return []string{}
	}

	eventData := strings.TrimPrefix(line, "data: ")
	if eventData == "" {
		return []string{}
	}

	root := gjson.Parse(eventData)
	var out []string

	// 使用缓存的 eventType（如果没有则从 data 中提取 type 字段）
	eventType := st.PendingEventType
	if eventType == "" {
		eventType = root.Get("type").String()
	}
	st.PendingEventType = "" // 清空缓存

	switch eventType {
	case "response.output_item.added":
		// 检查是否为 function_call 类型
		if root.Get("item.type").String() == "function_call" {
			st.CurrentFuncName = root.Get("item.name").String()
			st.CurrentFuncArgs.Reset()
		}

	case "response.output_text.delta":
		// 文本增量
		delta := root.Get("delta").String()
		if delta != "" {
			st.TextBuf.WriteString(delta)

			// 构建 Gemini 流式响应（发送增量，不是累计文本）
			chunk := buildGeminiStreamChunk(delta, "", false, st.Seq)
			st.Seq++
			out = append(out, chunk)
		}

	case "response.reasoning_summary_text.delta":
		delta := root.Get("text").String()
		if delta != "" {
			chunk := buildGeminiStreamChunk(delta, "", true, st.Seq)
			st.Seq++
			out = append(out, chunk)
		}

	case "response.function_call_arguments.delta":
		// 工具调用参数增量
		delta := root.Get("delta").String()
		if delta != "" {
			st.CurrentFuncArgs.WriteString(delta)
		}

	case "response.function_call_arguments.done":
		// 工具调用参数完成，收集到 FunctionCalls
		if st.CurrentFuncName != "" {
			var args map[string]interface{}
			argsStr := st.CurrentFuncArgs.String()
			if argsStr != "" {
				_ = json.Unmarshal([]byte(argsStr), &args)
			}
			st.FunctionCalls = append(st.FunctionCalls, &types.GeminiFunctionCall{
				Name:             st.CurrentFuncName,
				Args:             args,
				ThoughtSignature: types.DummyThoughtSignature,
			})
			st.CurrentFuncName = ""
			st.CurrentFuncArgs.Reset()
		}

	case "response.completed":
		// 响应完成，发送最终 chunk
		finalChunk := buildGeminiFinalChunk(st.TextBuf.String(), st.FunctionCalls, root)
		out = append(out, finalChunk)
	}

	return out
}

// buildGeminiStreamChunk 构建 Gemini 流式 chunk
func buildGeminiStreamChunk(text, finishReason string, thought bool, seq int) string {
	chunk := `{"candidates":[{"content":{"parts":[{"text":""}],"role":"model"},"finishReason":"","index":0}]}`

	if text != "" {
		chunk, _ = sjson.Set(chunk, "candidates.0.content.parts.0.text", text)
	}
	if thought {
		chunk, _ = sjson.Set(chunk, "candidates.0.content.parts.0.thought", true)
	}
	if finishReason != "" {
		chunk, _ = sjson.Set(chunk, "candidates.0.finishReason", finishReason)
	}

	return "data: " + chunk + "\n\n"
}

// buildGeminiFinalChunk 构建 Gemini 最终 chunk
func buildGeminiFinalChunk(text string, functionCalls []*types.GeminiFunctionCall, completedEvent gjson.Result) string {
	chunk := `{"candidates":[{"content":{"parts":[],"role":"model"},"finishReason":"STOP","index":0}]}`

	parts := []interface{}{}

	// 不添加文本（文本已通过 delta 发送）
	// 最终 chunk 只包含 finishReason、usage 和 function calls

	// 添加工具调用
	for _, fc := range functionCalls {
		parts = append(parts, map[string]interface{}{
			"functionCall": map[string]interface{}{
				"name":             fc.Name,
				"args":             fc.Args,
				"thoughtSignature": fc.ThoughtSignature,
			},
		})
	}

	chunk, _ = sjson.Set(chunk, "candidates.0.content.parts", parts)

	// 转换 status → finishReason
	status := completedEvent.Get("response.status").String()
	finishReason := responsesStatusToGemini(status)
	chunk, _ = sjson.Set(chunk, "candidates.0.finishReason", finishReason)

	// 添加 usage
	if usage := completedEvent.Get("response.usage"); usage.Exists() {
		inputTokens := usage.Get("input_tokens").Int()
		outputTokens := usage.Get("output_tokens").Int()
		cachedTokens := usage.Get("input_tokens_details.cached_tokens").Int()

		usageMetadata := map[string]interface{}{
			"promptTokenCount":        inputTokens + cachedTokens,
			"candidatesTokenCount":    outputTokens,
			"totalTokenCount":         inputTokens + cachedTokens + outputTokens,
			"cachedContentTokenCount": cachedTokens,
		}
		chunk, _ = sjson.Set(chunk, "usageMetadata", usageMetadata)
	}

	return "data: " + chunk + "\n\n"
}
