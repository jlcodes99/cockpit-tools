package providers

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

// OpenAIProvider OpenAI 提供商
type OpenAIProvider struct{}

// ConvertToProviderRequest 转换为 OpenAI 请求
func (p *OpenAIProvider) ConvertToProviderRequest(c *gin.Context, upstream *config.UpstreamConfig, apiKey string) (*http.Request, []byte, error) {
	// 读取和解析原始请求体
	originalBodyBytes, err := getRequestBodyBytes(c)
	if err != nil {
		return nil, nil, fmt.Errorf("读取请求体失败: %w", err)
	}

	var claudeReq types.ClaudeRequest
	if err := json.Unmarshal(originalBodyBytes, &claudeReq); err != nil {
		return nil, originalBodyBytes, fmt.Errorf("解析Claude请求体失败: %w", err)
	}

	// --- 复用旧的转换逻辑 ---
	openaiReq := &types.OpenAIRequest{
		Model:       config.RedirectModel(claudeReq.Model, upstream),
		Messages:    p.convertMessages(&claudeReq),
		Stream:      claudeReq.Stream,
		Temperature: claudeReq.Temperature,
	}

	if claudeReq.MaxTokens > 0 {
		openaiReq.MaxCompletionTokens = claudeReq.MaxTokens
	} else {
		openaiReq.MaxCompletionTokens = 65535
	}

	// 转换工具
	if len(claudeReq.Tools) > 0 {
		openaiReq.Tools = p.convertTools(claudeReq.Tools)
		openaiReq.ToolChoice = "auto"
	}
	// --- 转换逻辑结束 ---

	requestMap := map[string]interface{}{
		"model":                 openaiReq.Model,
		"messages":              openaiReq.Messages,
		"stream":                openaiReq.Stream,
		"temperature":           openaiReq.Temperature,
		"max_completion_tokens": openaiReq.MaxCompletionTokens,
	}
	if len(openaiReq.Tools) > 0 {
		requestMap["tools"] = openaiReq.Tools
		requestMap["tool_choice"] = openaiReq.ToolChoice
	}
	if effort := config.ResolveReasoningEffort(claudeReq.Model, upstream); effort != "" {
		requestMap["reasoning"] = map[string]interface{}{"effort": effort}
	}
	if upstream.TextVerbosity != "" {
		requestMap["text"] = map[string]interface{}{"verbosity": upstream.TextVerbosity}
	}
	if upstream.FastMode {
		requestMap["service_tier"] = "priority"
	}

	reqBodyBytes, err := json.Marshal(requestMap)
	if err != nil {
		return nil, originalBodyBytes, fmt.Errorf("序列化OpenAI请求体失败: %w", err)
	}

	// 构建URL - baseURL可能已包含版本号(如/v1, /v2, /v1beta, /v2alpha等),需要智能拼接
	// 如果 baseURL 以 # 结尾，则跳过自动添加 /v1
	baseURL := upstream.GetEffectiveBaseURL()
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	if skipVersionPrefix {
		baseURL = strings.TrimSuffix(baseURL, "#")
	}
	baseURL = strings.TrimSuffix(baseURL, "/")

	// 检查baseURL是否以版本号结尾(如/v1, /v2, /v1beta, /v2alpha等)
	// 使用正则表达式匹配 /v\d+[a-z]* 的模式(v后跟数字,可选字母后缀)
	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
	hasVersionSuffix := versionPattern.MatchString(baseURL)

	// 如果baseURL已经包含版本号或以#结尾,直接拼接/chat/completions
	// 否则拼接/v1/chat/completions
	endpoint := "/chat/completions"
	if !hasVersionSuffix && !skipVersionPrefix {
		endpoint = "/v1" + endpoint
	}
	url := baseURL + endpoint

	req, err := http.NewRequestWithContext(c.Request.Context(), "POST", url, bytes.NewReader(reqBodyBytes))
	if err != nil {
		return nil, originalBodyBytes, fmt.Errorf("创建OpenAI请求失败: %w", err)
	}

	// 使用统一的头部处理逻辑（透明代理）
	// 保留客户端的大部分 headers，只移除/替换必要的认证和代理相关 headers
	req.Header = utils.PrepareUpstreamHeaders(c, req.URL.Host)
	utils.SetAuthenticationHeader(req.Header, apiKey)
	utils.ApplyCustomHeaders(req.Header, upstream.CustomHeaders)

	return req, originalBodyBytes, nil
}

// convertMessages 转换消息
func (p *OpenAIProvider) convertMessages(claudeReq *types.ClaudeRequest) []types.OpenAIMessage {
	messages := []types.OpenAIMessage{}

	// 添加系统消息
	if claudeReq.System != nil {
		systemText := extractSystemText(claudeReq.System)
		if systemText != "" {
			messages = append(messages, types.OpenAIMessage{
				Role:    "system",
				Content: systemText,
			})
		}
	}

	// 转换普通消息
	for _, msg := range claudeReq.Messages {
		openaiMsg := p.convertMessage(msg)
		messages = append(messages, openaiMsg...)
	}

	return messages
}

// convertMessage 转换单个消息
func (p *OpenAIProvider) convertMessage(msg types.ClaudeMessage) []types.OpenAIMessage {
	messages := []types.OpenAIMessage{}

	// 如果是字符串内容
	if str, ok := msg.Content.(string); ok {
		if msg.Role != "tool" {
			messages = append(messages, types.OpenAIMessage{
				Role:    normalizeRole(msg.Role),
				Content: str,
			})
		}
		return messages
	}

	// 如果是内容数组
	contents, ok := msg.Content.([]interface{})
	if !ok {
		return messages
	}

	textContents := []string{}
	reasoningContents := []string{}
	toolCalls := []types.OpenAIToolCall{}
	toolResults := []types.OpenAIMessage{}

	for _, c := range contents {
		content, ok := c.(map[string]interface{})
		if !ok {
			continue
		}

		contentType, _ := content["type"].(string)

		switch contentType {
		case "thinking":
			if thinking, ok := content["thinking"].(string); ok && thinking != "" {
				reasoningContents = append(reasoningContents, thinking)
			}

		case "text":
			if text, ok := content["text"].(string); ok {
				textContents = append(textContents, text)
			}

		case "tool_use":
			id, _ := content["id"].(string)
			name, _ := content["name"].(string)
			input := content["input"]

			inputJSON, _ := json.Marshal(input)
			toolCalls = append(toolCalls, types.OpenAIToolCall{
				ID:   id,
				Type: "function",
				Function: types.OpenAIToolCallFunction{
					Name:      name,
					Arguments: string(inputJSON),
				},
			})

		case "tool_result":
			toolUseID, _ := content["tool_use_id"].(string)
			resultContent := content["content"]

			var contentStr string
			if str, ok := resultContent.(string); ok {
				contentStr = str
			} else {
				contentJSON, _ := json.Marshal(resultContent)
				contentStr = string(contentJSON)
			}

			toolResults = append(toolResults, types.OpenAIMessage{
				Role:       "tool",
				ToolCallID: toolUseID,
				Content:    contentStr,
			})
		}
	}

	// 添加工具结果
	messages = append(messages, toolResults...)

	// 添加文本和工具调用
	if len(textContents) > 0 || len(reasoningContents) > 0 || len(toolCalls) > 0 {
		role := normalizeRole(msg.Role)
		if role != "tool" {
			openaiMsg := types.OpenAIMessage{
				Role: role,
			}

			if len(textContents) > 0 {
				openaiMsg.Content = strings.Join(textContents, "\n")
			} else {
				openaiMsg.Content = nil
			}
			if len(reasoningContents) > 0 {
				openaiMsg.ReasoningContent = strings.Join(reasoningContents, "\n")
			}

			if len(toolCalls) > 0 {
				openaiMsg.ToolCalls = toolCalls
			}

			messages = append(messages, openaiMsg)
		}
	}

	return messages
}

// convertTools 转换工具
func (p *OpenAIProvider) convertTools(claudeTools []types.ClaudeTool) []types.OpenAITool {
	tools := []types.OpenAITool{}

	for _, tool := range claudeTools {
		tools = append(tools, types.OpenAITool{
			Type: "function",
			Function: types.OpenAIToolFunction{
				Name:        tool.Name,
				Description: tool.Description,
				Parameters:  cleanJsonSchema(tool.InputSchema),
			},
		})
	}

	return tools
}

// cleanJsonSchema 清理 JSON Schema，移除某些上游不支持的字段
func cleanJsonSchema(schema interface{}) interface{} {
	if schema == nil {
		return schema
	}

	// 如果是 map，递归清理
	if schemaMap, ok := schema.(map[string]interface{}); ok {
		cleaned := make(map[string]interface{})

		for key, value := range schemaMap {
			// 移除不需要的字段
			if key == "$schema" || key == "title" || key == "examples" || key == "additionalProperties" {
				continue
			}
			// 移除 format 字段（当类型为 string 时）
			if key == "format" {
				if schemaType, hasType := schemaMap["type"]; hasType && schemaType == "string" {
					continue
				}
			}
			// 递归处理嵌套对象
			if key == "properties" || key == "items" {
				cleaned[key] = cleanJsonSchema(value)
			} else if valueMap, isMap := value.(map[string]interface{}); isMap {
				cleaned[key] = cleanJsonSchema(valueMap)
			} else if valueSlice, isSlice := value.([]interface{}); isSlice {
				cleanedSlice := make([]interface{}, len(valueSlice))
				for i, item := range valueSlice {
					cleanedSlice[i] = cleanJsonSchema(item)
				}
				cleaned[key] = cleanedSlice
			} else {
				cleaned[key] = value
			}
		}

		return cleaned
	}

	// 如果是数组，递归清理每个元素
	if schemaSlice, ok := schema.([]interface{}); ok {
		cleaned := make([]interface{}, len(schemaSlice))
		for i, item := range schemaSlice {
			cleaned[i] = cleanJsonSchema(item)
		}
		return cleaned
	}

	// 其他类型直接返回
	return schema
}

// ConvertToClaudeResponse 转换为 Claude 响应
func (p *OpenAIProvider) ConvertToClaudeResponse(providerResp *types.ProviderResponse) (*types.ClaudeResponse, error) {
	var openaiResp types.OpenAIResponse
	if err := json.Unmarshal(providerResp.Body, &openaiResp); err != nil {
		return nil, err
	}

	claudeResp := &types.ClaudeResponse{
		ID:      generateID(),
		Type:    "message",
		Role:    "assistant",
		Content: []types.ClaudeContent{},
	}

	if len(openaiResp.Choices) > 0 {
		choice := openaiResp.Choices[0]
		msg := choice.Message

		// 添加文本内容
		if msg.ReasoningContent != "" {
			claudeResp.Content = append(claudeResp.Content, types.ClaudeContent{
				Type:     "thinking",
				Thinking: msg.ReasoningContent,
			})
		}

		if str, ok := msg.Content.(string); ok && str != "" {
			claudeResp.Content = append(claudeResp.Content, types.ClaudeContent{
				Type: "text",
				Text: str,
			})
		}

		// 添加工具调用
		for _, toolCall := range msg.ToolCalls {
			var input interface{}
			json.Unmarshal([]byte(toolCall.Function.Arguments), &input)
			input = sanitizeClaudeToolInput(toolCall.Function.Name, input)

			claudeResp.Content = append(claudeResp.Content, types.ClaudeContent{
				Type:  "tool_use",
				ID:    toolCall.ID,
				Name:  toolCall.Function.Name,
				Input: input,
			})
		}

		// 设置停止原因
		if len(msg.ToolCalls) > 0 {
			claudeResp.StopReason = "tool_use"
		} else if choice.FinishReason == "length" {
			claudeResp.StopReason = "max_tokens"
		} else {
			claudeResp.StopReason = "end_turn"
		}
	}

	// 添加使用统计
	if openaiResp.Usage != nil {
		claudeResp.Usage = &types.Usage{
			InputTokens:  openaiResp.Usage.PromptTokens,
			OutputTokens: openaiResp.Usage.CompletionTokens,
		}
	}

	return claudeResp, nil
}

// HandleStreamResponse 处理流式响应
func (p *OpenAIProvider) HandleStreamResponse(body io.ReadCloser) (<-chan string, <-chan error, error) {
	eventChan := make(chan string, 100)
	errChan := make(chan error, 1)

	go func() {
		defer close(eventChan)
		// defer close(errChan) // 移除此行，避免竞态条件
		defer body.Close()

		scanner := bufio.NewScanner(body)
		// 设置更大的 buffer (1MB) 以处理大 JSON chunk，避免默认 64KB 限制
		const maxScannerBufferSize = 1024 * 1024 // 1MB
		scanner.Buffer(make([]byte, 0, 64*1024), maxScannerBufferSize)

		nextBlockIndex := 0
		toolCallAccumulator := make(map[int]*ToolCallAccumulator)
		toolUseStopEmitted := false
		messageStartSent := false
		stopReason := ""

		emitContentBlockStop := func(index int) {
			stopEvent := map[string]interface{}{
				"type":  "content_block_stop",
				"index": index,
			}
			stopJSON, _ := json.Marshal(stopEvent)
			eventChan <- fmt.Sprintf("event: content_block_stop\ndata: %s\n\n", stopJSON)
		}

		// thinking / 文本块状态跟踪
		thinkingBlockStarted := false
		thinkingBlockIndex := -1
		textBlockStarted := false
		textBlockIndex := -1

		for scanner.Scan() {
			line := normalizeSSEFieldLine(scanner.Text())
			line = strings.TrimSpace(line)

			if line == "" || line == "data: [DONE]" {
				continue
			}

			if !strings.HasPrefix(line, "data: ") {
				continue
			}

			jsonStr := strings.TrimPrefix(line, "data: ")

			var chunk map[string]interface{}
			if err := json.Unmarshal([]byte(jsonStr), &chunk); err != nil {
				continue
			}

			// 检查是否有错误
			if errObj, ok := chunk["error"]; ok {
				errChan <- fmt.Errorf("upstream error: %v", errObj)
				return
			}

			choices, ok := chunk["choices"].([]interface{})
			if !ok || len(choices) == 0 {
				continue
			}

			choice, ok := choices[0].(map[string]interface{})
			if !ok {
				continue
			}

			delta, ok := choice["delta"].(map[string]interface{})
			if !ok {
				continue
			}

			// 提取模型名称（用于 message_start）
			model := ""
			if m, ok := chunk["model"].(string); ok {
				model = m
			}

			// 处理 reasoning_content（DeepSeek Chat / OpenAI 兼容推理内容）
			if reasoning, ok := delta["reasoning_content"].(string); ok && reasoning != "" {
				if !messageStartSent {
					eventChan <- buildMessageStartEvent(model)
					messageStartSent = true
				}
				if textBlockStarted {
					emitContentBlockStop(textBlockIndex)
					textBlockStarted = false
				}
				if !thinkingBlockStarted {
					thinkingBlockIndex = nextBlockIndex
					nextBlockIndex++
					startEvent := map[string]interface{}{
						"type":  "content_block_start",
						"index": thinkingBlockIndex,
						"content_block": map[string]string{
							"type":     "thinking",
							"thinking": "",
						},
					}
					startJSON, _ := json.Marshal(startEvent)
					eventChan <- fmt.Sprintf("event: content_block_start\ndata: %s\n\n", startJSON)
					thinkingBlockStarted = true
				}

				deltaEvent := map[string]interface{}{
					"type":  "content_block_delta",
					"index": thinkingBlockIndex,
					"delta": map[string]string{
						"type":     "thinking_delta",
						"thinking": reasoning,
					},
				}
				deltaJSON, _ := json.Marshal(deltaEvent)
				eventChan <- fmt.Sprintf("event: content_block_delta\ndata: %s\n\n", deltaJSON)
			}

			// 处理文本内容
			if content, ok := delta["content"].(string); ok && content != "" {
				// 在第一个 content_block 之前发送 message_start
				if !messageStartSent {
					eventChan <- buildMessageStartEvent(model)
					messageStartSent = true
				}
				if thinkingBlockStarted {
					emitContentBlockStop(thinkingBlockIndex)
					thinkingBlockStarted = false
				}
				// 如果是第一个文本块,发送 content_block_start
				if !textBlockStarted {
					textBlockIndex = nextBlockIndex
					nextBlockIndex++
					startEvent := map[string]interface{}{
						"type":  "content_block_start",
						"index": textBlockIndex,
						"content_block": map[string]string{
							"type": "text",
							"text": "",
						},
					}
					startJSON, _ := json.Marshal(startEvent)
					eventChan <- fmt.Sprintf("event: content_block_start\ndata: %s\n\n", startJSON)
					textBlockStarted = true
				}

				// 发送 content_block_delta
				deltaEvent := map[string]interface{}{
					"type":  "content_block_delta",
					"index": textBlockIndex,
					"delta": map[string]string{
						"type": "text_delta",
						"text": content,
					},
				}
				deltaJSON, _ := json.Marshal(deltaEvent)
				eventChan <- fmt.Sprintf("event: content_block_delta\ndata: %s\n\n", deltaJSON)
			}

			// 处理工具调用
			if toolCalls, ok := delta["tool_calls"].([]interface{}); ok {
				// 在第一个 content_block 之前发送 message_start
				if !messageStartSent {
					eventChan <- buildMessageStartEvent(model)
					messageStartSent = true
				}
				// 如果有 thinking / 文本块正在进行,先关闭它
				if thinkingBlockStarted {
					emitContentBlockStop(thinkingBlockIndex)
					thinkingBlockStarted = false
				}
				if textBlockStarted {
					emitContentBlockStop(textBlockIndex)
					textBlockStarted = false
				}

				for _, tc := range toolCalls {
					toolCall, ok := tc.(map[string]interface{})
					if !ok {
						continue
					}

					index := 0
					if idx, ok := toolCall["index"].(float64); ok {
						index = int(idx)
					}

					// 获取或创建累加器
					if _, exists := toolCallAccumulator[index]; !exists {
						toolCallAccumulator[index] = &ToolCallAccumulator{}
					}
					acc := toolCallAccumulator[index]

					// 累积数据
					if id, ok := toolCall["id"].(string); ok {
						acc.ID = id
					}

					if function, ok := toolCall["function"].(map[string]interface{}); ok {
						if name, ok := function["name"].(string); ok {
							acc.Name = name
						}
						if args, ok := function["arguments"].(string); ok {
							acc.Arguments += args
						}
					}

					// 检查是否完整
					if acc.ID != "" && acc.Name != "" && acc.Arguments != "" {
						var args interface{}
						if err := json.Unmarshal([]byte(acc.Arguments), &args); err == nil {
							args = sanitizeClaudeToolInput(acc.Name, args)
							toolUseBlockIndex := nextBlockIndex
							nextBlockIndex++
							events := processToolUsePart(acc.ID, acc.Name, args, toolUseBlockIndex)
							for _, event := range events {
								eventChan <- event
							}
							delete(toolCallAccumulator, index)
						}
					}
				}
			}

			// 处理结束原因
			if finishReason, ok := choice["finish_reason"].(string); ok {
				// 如果有未关闭的 thinking / 文本块,先关闭它
				if thinkingBlockStarted {
					emitContentBlockStop(thinkingBlockIndex)
					thinkingBlockStarted = false
				}
				if textBlockStarted {
					emitContentBlockStop(textBlockIndex)
					textBlockStarted = false
				}

				if !toolUseStopEmitted && (finishReason == "tool_calls" || finishReason == "function_call") {
					stopReason = "tool_use"
					toolUseStopEmitted = true
				} else if finishReason == "stop" {
					stopReason = "end_turn"
				} else if finishReason == "length" {
					stopReason = "max_tokens"
				}
			}
		}

		// 确保流结束时关闭任何未关闭的 thinking / 文本块
		if thinkingBlockStarted {
			emitContentBlockStop(thinkingBlockIndex)
		}
		if textBlockStarted {
			emitContentBlockStop(textBlockIndex)
		}

		// 发送 message_delta（含 stop_reason）和 message_stop
		// 注意：必须先检查 scanner 错误，避免流读取异常时发送矛盾的正常结束事件
		if err := scanner.Err(); err != nil {
			// 在 tool_use 场景下，客户端主动断开是正常行为
			// 如果已经发送了 tool_use stop 事件，并且错误是连接断开相关的，则忽略该错误
			errMsg := err.Error()
			if toolUseStopEmitted && (strings.Contains(errMsg, "broken pipe") ||
				strings.Contains(errMsg, "connection reset") ||
				strings.Contains(errMsg, "EOF")) {
				// 这是预期的客户端行为，不报告错误
				return
			}
			errChan <- err
			return
		}

		if messageStartSent {
			if stopReason == "" {
				stopReason = "end_turn"
			}
			deltaEvent := map[string]interface{}{
				"type": "message_delta",
				"delta": map[string]string{
					"stop_reason": stopReason,
				},
			}
			deltaJSON, _ := json.Marshal(deltaEvent)
			eventChan <- fmt.Sprintf("event: message_delta\ndata: %s\n\n", deltaJSON)

			stopEvent := map[string]interface{}{
				"type": "message_stop",
			}
			stopJSON, _ := json.Marshal(stopEvent)
			eventChan <- fmt.Sprintf("event: message_stop\ndata: %s\n\n", stopJSON)
		}
	}()

	return eventChan, errChan, nil
}

// ToolCallAccumulator 工具调用累加器
type ToolCallAccumulator struct {
	ID        string
	Name      string
	Arguments string
}

// buildMessageStartEvent 构建 Claude Messages API 的 message_start SSE 事件
func buildMessageStartEvent(model string) string {
	if model == "" {
		model = "unknown"
	}
	event := map[string]interface{}{
		"type": "message_start",
		"message": map[string]interface{}{
			"id":      fmt.Sprintf("msg_%d", time.Now().UnixNano()),
			"type":    "message",
			"role":    "assistant",
			"content": []interface{}{},
			"model":   model,
			"usage": map[string]int{
				"input_tokens":  0,
				"output_tokens": 0,
			},
		},
	}
	eventJSON, _ := json.Marshal(event)
	return fmt.Sprintf("event: message_start\ndata: %s\n\n", eventJSON)
}

// processToolUsePart 处理工具使用部分
func processToolUsePart(id, name string, input interface{}, index int) []string {
	events := []string{}

	// content_block_start
	startEvent := map[string]interface{}{
		"type":  "content_block_start",
		"index": index,
		"content_block": map[string]interface{}{
			"type": "tool_use",
			"id":   id,
			"name": name,
		},
	}
	startJSON, _ := json.Marshal(startEvent)
	events = append(events, fmt.Sprintf("event: content_block_start\ndata: %s\n\n", startJSON))

	// content_block_delta
	inputJSON, _ := json.Marshal(input)
	deltaEvent := map[string]interface{}{
		"type":  "content_block_delta",
		"index": index,
		"delta": map[string]string{
			"type":         "input_json_delta",
			"partial_json": string(inputJSON),
		},
	}
	deltaJSON, _ := json.Marshal(deltaEvent)
	events = append(events, fmt.Sprintf("event: content_block_delta\ndata: %s\n\n", deltaJSON))

	// content_block_stop
	stopEvent := map[string]interface{}{
		"type":  "content_block_stop",
		"index": index,
	}
	stopJSON, _ := json.Marshal(stopEvent)
	events = append(events, fmt.Sprintf("event: content_block_stop\ndata: %s\n\n", stopJSON))

	return events
}

// 辅助函数

func extractSystemText(system interface{}) string {
	return extractSystemTextBlocks(system, 0)
}

func extractSystemTextBlocks(system interface{}, skipLeadingTextBlocks int) string {
	if str, ok := system.(string); ok {
		return str
	}

	arr, ok := system.([]interface{})
	if !ok {
		return ""
	}

	parts := []string{}
	for index, item := range arr {
		obj, ok := item.(map[string]interface{})
		if !ok {
			continue
		}

		if obj["type"] != "text" {
			continue
		}
		if index < skipLeadingTextBlocks {
			continue
		}
		if text, ok := obj["text"].(string); ok {
			parts = append(parts, text)
		}
	}

	return strings.Join(parts, "\n")
}

func normalizeRole(role string) string {
	role = strings.ToLower(role)
	switch role {
	case "user", "assistant", "system", "tool":
		return role
	default:
		return "user"
	}
}

func generateID() string {
	return fmt.Sprintf("msg_%d", time.Now().UnixNano())
}
