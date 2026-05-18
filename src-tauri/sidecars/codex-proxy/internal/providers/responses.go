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

	"github.com/gin-gonic/gin"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/converters"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
)

// ResponsesProvider Responses API 提供商
type ResponsesProvider struct {
	SessionManager *session.SessionManager
}

// ConvertToProviderRequest 将请求转换为上游格式
func (p *ResponsesProvider) ConvertToProviderRequest(
	c *gin.Context,
	upstream *config.UpstreamConfig,
	apiKey string,
) (*http.Request, []byte, error) {
	bodyBytes, err := getRequestBodyBytes(c)
	if err != nil {
		return nil, nil, fmt.Errorf("读取请求体失败: %w", err)
	}

	if p.SessionManager == nil {
		p.SessionManager = newDefaultSessionManager()
	}

	providerReq, reqBodyForURL, err := p.buildProviderRequestBody(c, c.Request.URL.Path, bodyBytes, upstream)
	if err != nil {
		return nil, bodyBytes, err
	}

	reqBody, err := utils.MarshalJSONNoEscape(providerReq)
	if err != nil {
		return nil, bodyBytes, fmt.Errorf("序列化请求失败: %w", err)
	}

	targetURL, err := p.buildRequestURL(upstream, reqBodyForURL)
	if err != nil {
		return nil, bodyBytes, err
	}

	req, err := http.NewRequestWithContext(c.Request.Context(), http.MethodPost, targetURL, bytes.NewReader(reqBody))
	if err != nil {
		return nil, bodyBytes, err
	}

	req.Header = utils.PrepareUpstreamHeaders(c, req.URL.Host)
	req.Header.Del("authorization")
	req.Header.Del("x-api-key")
	req.Header.Del("x-goog-api-key")

	switch upstream.ServiceType {
	case "gemini":
		utils.SetGeminiAuthenticationHeader(req.Header, apiKey)
	default:
		utils.SetAuthenticationHeader(req.Header, apiKey)
	}

	req.Header.Set("Content-Type", "application/json")
	utils.ApplyCustomHeaders(req.Header, upstream.CustomHeaders)

	return req, bodyBytes, nil
}

func (p *ResponsesProvider) buildProviderRequestBody(c *gin.Context, requestPath string, bodyBytes []byte, upstream *config.UpstreamConfig) (interface{}, []byte, error) {
	if strings.HasSuffix(requestPath, "/v1/messages") {
		responsesReq, err := p.buildResponsesRequestFromClaude(c, bodyBytes, upstream)
		if err != nil {
			return nil, nil, fmt.Errorf("解析 Claude Messages 请求失败: %w", err)
		}
		return responsesReq, bodyBytes, nil
	}

	var providerReq interface{}
	converter := converters.NewConverter(upstream.ServiceType)

	if _, ok := converter.(*converters.ResponsesPassthroughConverter); ok {
		var reqMap map[string]interface{}
		if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
			return nil, nil, fmt.Errorf("透传模式下解析请求失败: %w", err)
		}
		normalizeResponsesInputForPassthrough(reqMap)
		if upstream.CodexNativeToolPassthrough {
			convertCodexToolsForPassthrough(reqMap)
		} else if upstream.IsCodexToolCompatEnabled() {
			stripCodexClientOnlyTools(reqMap)
		}
		if model, ok := reqMap["model"].(string); ok {
			reqMap["model"] = config.RedirectModel(model, upstream)
			if effort := config.ResolveReasoningEffort(model, upstream); effort != "" {
				if upstream.ReasoningParamStyle == "thinking" {
					delete(reqMap, "reasoning")
					delete(reqMap, "reasoning_effort")
					if effort != "none" {
						reqMap["thinking"] = map[string]interface{}{"type": "enabled"}
					}
				} else {
					reqMap["reasoning"] = map[string]interface{}{"effort": effort}
				}
			}
		}
		if upstream.TextVerbosity != "" {
			reqMap["text"] = map[string]interface{}{"verbosity": upstream.TextVerbosity}
		}
		if upstream.FastMode {
			reqMap["service_tier"] = "priority"
		}
		providerReq = reqMap
	} else {
		var responsesReq types.ResponsesRequest
		if err := json.Unmarshal(bodyBytes, &responsesReq); err != nil {
			return nil, nil, fmt.Errorf("解析 Responses 请求失败: %w", err)
		}

		var (
			sess *session.Session
			err  error
		)
		if responsesReq.PreviousResponseID != "" {
			sess, err = p.SessionManager.GetOrCreateSession(responsesReq.PreviousResponseID)
			if err != nil {
				return nil, nil, fmt.Errorf("get session failed: %w", err)
			}
		} else {
			sess = &session.Session{}
		}

		originalModel := responsesReq.Model
		responsesReq.Model = config.RedirectModel(responsesReq.Model, upstream)
		// Inject codex_tool_compat_enabled into TransformerMetadata for the converter.
		if responsesReq.TransformerMetadata == nil {
			responsesReq.TransformerMetadata = make(map[string]interface{})
		}
		responsesReq.TransformerMetadata["codex_tool_compat_enabled"] = upstream.IsCodexToolCompatEnabled() || upstream.CodexNativeToolPassthrough
		responsesReq.RawTools = extractRawToolsFromRequest(bodyBytes)
		convertedReq, err := converter.ToProviderRequest(sess, &responsesReq)
		if err != nil {
			return nil, nil, fmt.Errorf("convert request failed: %w", err)
		}

		// converter 路径：注入 reasoning/thinking 参数
		if reqMap, ok := convertedReq.(map[string]interface{}); ok {
			model := originalModel
			if effort := config.ResolveReasoningEffort(model, upstream); effort != "" {
				if upstream.ReasoningParamStyle == "thinking" {
					delete(reqMap, "reasoning")
					delete(reqMap, "reasoning_effort")
					if effort != "none" {
						reqMap["thinking"] = map[string]interface{}{"type": "enabled"}
					}
				} else if upstream.ReasoningParamStyle == "reasoning_effort" {
					reqMap["reasoning_effort"] = effort
				} else {
					reqMap["reasoning"] = map[string]interface{}{"effort": effort}
				}
			} else {
				// 无 ReasoningMapping 配置时，透传客户端原始 reasoning 并按 style 转换
				var rawReq map[string]interface{}
				if json.Unmarshal(bodyBytes, &rawReq) == nil {
					if reasoning, hasReasoning := rawReq["reasoning"]; hasReasoning {
						if upstream.ReasoningParamStyle == "thinking" {
							delete(reqMap, "reasoning")
							delete(reqMap, "reasoning_effort")
							if effort := extractEffortFromReasoning(reasoning); effort != "" && effort != "none" {
								reqMap["thinking"] = map[string]interface{}{"type": "enabled"}
							}
						} else if upstream.ReasoningParamStyle == "reasoning_effort" {
							delete(reqMap, "reasoning")
							if effort := extractEffortFromReasoning(reasoning); effort != "" {
								reqMap["reasoning_effort"] = effort
							}
						} else {
							reqMap["reasoning"] = reasoning
						}
					}
				}
			}
			if upstream.TextVerbosity != "" {
				reqMap["text"] = map[string]interface{}{"verbosity": upstream.TextVerbosity}
			}
			if upstream.FastMode {
				reqMap["service_tier"] = "priority"
			}
		}

		providerReq = convertedReq
	}

	if upstream.NormalizeNonstandardChatRoles {
		if reqMap, ok := providerReq.(map[string]interface{}); ok {
			converters.NormalizeNonstandardChatRolesInRequest(reqMap)
		}
	}
	if reqMap, ok := providerReq.(map[string]interface{}); ok {
		model, _ := reqMap["model"].(string)
		if config.ShouldOmitVisionContentForCompatibility(upstream, model) {
			converters.OmitOpenAIChatImageContent(reqMap)
		}
	}

	return providerReq, bodyBytes, nil
}

func (p *ResponsesProvider) buildResponsesRequestFromClaude(c *gin.Context, bodyBytes []byte, upstream *config.UpstreamConfig) (map[string]interface{}, error) {
	var claudeReq types.ClaudeRequest
	if err := json.Unmarshal(bodyBytes, &claudeReq); err != nil {
		return nil, err
	}

	input := make([]map[string]interface{}, 0, len(claudeReq.Messages))
	for _, msg := range claudeReq.Messages {
		role := normalizeRole(msg.Role)
		contentBlocks := make([]map[string]interface{}, 0)
		flushMessage := func() {
			if len(contentBlocks) == 0 {
				return
			}
			input = append(input, map[string]interface{}{
				"type":    "message",
				"role":    role,
				"content": contentBlocks,
			})
			contentBlocks = make([]map[string]interface{}, 0)
		}
		switch content := msg.Content.(type) {
		case string:
			if content != "" {
				contentBlocks = append(contentBlocks, map[string]interface{}{
					"type": responsesTextContentType(role),
					"text": content,
				})
			}
		case []interface{}:
			for _, rawBlock := range content {
				block, ok := rawBlock.(map[string]interface{})
				if !ok {
					continue
				}
				switch block["type"] {
				case "thinking":
					if thinking, ok := block["thinking"].(string); ok && thinking != "" {
						flushMessage()
						input = append(input, map[string]interface{}{
							"type": "reasoning",
							"summary": []map[string]interface{}{{
								"type": "summary_text",
								"text": thinking,
							}},
						})
					}
				case "text":
					if text, ok := block["text"].(string); ok && text != "" {
						contentBlocks = append(contentBlocks, map[string]interface{}{
							"type": responsesTextContentType(role),
							"text": text,
						})
					}
				case "tool_use":
					flushMessage()
					arguments, _ := utils.MarshalJSONNoEscape(block["input"])
					input = append(input, map[string]interface{}{
						"type":      "function_call",
						"call_id":   block["id"],
						"name":      block["name"],
						"arguments": string(arguments),
					})
				case "tool_result":
					flushMessage()
					resultText := extractClaudeToolResult(block["content"])
					input = append(input, map[string]interface{}{
						"type":    "function_call_output",
						"call_id": block["tool_use_id"],
						"output":  resultText,
					})
				}
			}
		}

		flushMessage()
	}

	responsesReq := map[string]interface{}{
		"model":  config.RedirectModel(claudeReq.Model, upstream),
		"input":  input,
		"stream": claudeReq.Stream,
	}
	if effort := config.ResolveReasoningEffort(claudeReq.Model, upstream); effort != "" {
		if upstream.ReasoningParamStyle == "thinking" {
			if effort != "none" {
				responsesReq["thinking"] = map[string]interface{}{"type": "enabled"}
			}
		} else {
			responsesReq["reasoning"] = map[string]interface{}{"effort": effort}
		}
	}
	if instructions := extractResponsesInstructions(claudeReq.System); instructions != "" {
		responsesReq["instructions"] = instructions
	}
	if claudeReq.MaxTokens > 0 {
		responsesReq["max_output_tokens"] = claudeReq.MaxTokens
	}
	if claudeReq.Temperature > 0 {
		responsesReq["temperature"] = claudeReq.Temperature
	}
	if claudeReq.TopP > 0 {
		responsesReq["top_p"] = claudeReq.TopP
	}
	if claudeReq.ToolChoice != nil {
		responsesReq["tool_choice"] = claudeReq.ToolChoice
	}
	if len(claudeReq.Tools) > 0 {
		tools := make([]map[string]interface{}, 0, len(claudeReq.Tools))
		for _, tool := range claudeReq.Tools {
			item := map[string]interface{}{
				"type":       "function",
				"name":       tool.Name,
				"parameters": tool.InputSchema,
			}
			if tool.Description != "" {
				item["description"] = tool.Description
			}
			tools = append(tools, item)
		}
		responsesReq["tools"] = tools
		if claudeReq.ParallelToolCalls != nil {
			responsesReq["parallel_tool_calls"] = *claudeReq.ParallelToolCalls
		} else {
			responsesReq["parallel_tool_calls"] = true
		}
	}
	if cacheKey := utils.ExtractUnifiedSessionID(c, bodyBytes); cacheKey != "" {
		responsesReq["prompt_cache_key"] = cacheKey
	}
	return responsesReq, nil
}

func extractResponsesInstructions(system interface{}) string {
	arr, ok := system.([]interface{})
	if !ok || len(arr) == 0 {
		return extractSystemText(system)
	}

	first, ok := arr[0].(map[string]interface{})
	if !ok || first["type"] != "text" {
		return extractSystemText(system)
	}

	text, ok := first["text"].(string)
	if !ok || !strings.HasPrefix(text, "x-anthropic-billing-header:") {
		return extractSystemText(system)
	}

	return extractSystemTextBlocks(system, 1)
}

func responsesTextContentType(role string) string {
	if role == "assistant" {
		return "output_text"
	}
	return "input_text"
}

func extractClaudeToolResult(content interface{}) string {
	switch v := content.(type) {
	case string:
		return v
	case []interface{}:
		parts := make([]string, 0, len(v))
		for _, item := range v {
			block, ok := item.(map[string]interface{})
			if !ok {
				continue
			}
			if text, ok := block["text"].(string); ok && text != "" {
				parts = append(parts, text)
			}
		}
		return strings.Join(parts, "\n")
	default:
		bytes, _ := utils.MarshalJSONNoEscape(v)
		return string(bytes)
	}
}

func (p *ResponsesProvider) buildRequestURL(upstream *config.UpstreamConfig, bodyBytes []byte) (string, error) {
	if upstream.ServiceType == "gemini" {
		var responsesReq types.ResponsesRequest
		if err := json.Unmarshal(bodyBytes, &responsesReq); err != nil {
			return "", fmt.Errorf("解析 Responses 请求失败: %w", err)
		}
		model := config.RedirectModel(responsesReq.Model, upstream)
		action := "generateContent"
		if responsesReq.Stream {
			action = "streamGenerateContent?alt=sse"
		}
		baseURL := strings.TrimSuffix(upstream.GetEffectiveBaseURL(), "/")
		versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
		if !versionPattern.MatchString(baseURL) && !strings.HasSuffix(upstream.BaseURL, "#") {
			baseURL += "/v1beta"
		}
		return fmt.Sprintf("%s/models/%s:%s", baseURL, model, action), nil
	}
	return p.buildTargetURL(upstream), nil
}

// buildTargetURL 根据上游类型构建目标 URL
func (p *ResponsesProvider) buildTargetURL(upstream *config.UpstreamConfig) string {
	baseURL := upstream.BaseURL
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	if skipVersionPrefix {
		baseURL = strings.TrimSuffix(baseURL, "#")
	}
	baseURL = strings.TrimSuffix(baseURL, "/")

	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
	hasVersionSuffix := versionPattern.MatchString(baseURL)

	var endpoint string
	switch upstream.ServiceType {
	case "responses":
		endpoint = "/responses"
	case "claude":
		endpoint = "/messages"
	case "gemini":
		endpoint = ""
	default:
		endpoint = "/chat/completions"
	}

	if hasVersionSuffix || skipVersionPrefix {
		return baseURL + endpoint
	}
	return baseURL + "/v1" + endpoint
}

// ConvertToClaudeResponse 将上游响应转换为 Claude 响应
func (p *ResponsesProvider) ConvertToClaudeResponse(providerResp *types.ProviderResponse) (*types.ClaudeResponse, error) {
	var responsesResp map[string]interface{}
	if err := json.Unmarshal(providerResp.Body, &responsesResp); err != nil {
		return nil, err
	}

	claudeResp := &types.ClaudeResponse{
		ID:      generateID(),
		Type:    "message",
		Role:    "assistant",
		Content: []types.ClaudeContent{},
	}

	if id, ok := responsesResp["id"].(string); ok && id != "" {
		claudeResp.ID = id
	}

	if output, ok := responsesResp["output"].([]interface{}); ok {
		for _, rawItem := range output {
			item, ok := rawItem.(map[string]interface{})
			if !ok {
				continue
			}
			switch item["type"] {
			case "reasoning":
				if thinking := responsesReasoningText(item); thinking != "" {
					claudeResp.Content = append(claudeResp.Content, types.ClaudeContent{Type: "thinking", Thinking: thinking})
				}
			case "message":
				if content, ok := item["content"].([]interface{}); ok {
					for _, rawBlock := range content {
						block, ok := rawBlock.(map[string]interface{})
						if !ok {
							continue
						}
						if text, ok := block["text"].(string); ok && text != "" {
							claudeResp.Content = append(claudeResp.Content, types.ClaudeContent{Type: "text", Text: text})
						}
					}
				}
			case "function_call":
				var input interface{}
				if args, ok := item["arguments"].(string); ok && args != "" {
					_ = json.Unmarshal([]byte(args), &input)
				}
				input = sanitizeClaudeToolInput(toString(item["name"]), input)
				claudeResp.Content = append(claudeResp.Content, types.ClaudeContent{
					Type:  "tool_use",
					ID:    toString(item["call_id"]),
					Name:  toString(item["name"]),
					Input: input,
				})
			}
		}
	}

	if usageRaw, ok := responsesResp["usage"].(map[string]interface{}); ok {
		claudeResp.Usage = &types.Usage{}
		if v, ok := usageRaw["input_tokens"].(float64); ok {
			claudeResp.Usage.InputTokens = int(v)
			claudeResp.Usage.PromptTokensTotal = int(v)
		}
		if v, ok := usageRaw["output_tokens"].(float64); ok {
			claudeResp.Usage.OutputTokens = int(v)
		}
		if cacheCreation, ok := usageRaw["cache_creation_input_tokens"].(float64); ok {
			claudeResp.Usage.CacheCreationInputTokens = int(cacheCreation)
		}
		if cacheRead, ok := usageRaw["cache_read_input_tokens"].(float64); ok {
			claudeResp.Usage.CacheReadInputTokens = int(cacheRead)
		} else {
			claudeResp.Usage.CacheReadInputTokens = extractResponsesCacheReadTokens(usageRaw)
		}
		if cacheCreation5m, ok := usageRaw["cache_creation_5m_input_tokens"].(float64); ok {
			claudeResp.Usage.CacheCreation5mInputTokens = int(cacheCreation5m)
		}
		if cacheCreation1h, ok := usageRaw["cache_creation_1h_input_tokens"].(float64); ok {
			claudeResp.Usage.CacheCreation1hInputTokens = int(cacheCreation1h)
		}
		if cacheTTL, ok := usageRaw["cache_ttl"].(string); ok {
			claudeResp.Usage.CacheTTL = cacheTTL
		}
	}

	hasToolUse := false
	for _, block := range claudeResp.Content {
		if block.Type == "tool_use" {
			hasToolUse = true
			break
		}
	}
	if hasToolUse {
		claudeResp.StopReason = "tool_use"
	} else if status, _ := responsesResp["status"].(string); status == "incomplete" {
		claudeResp.StopReason = "max_tokens"
	} else {
		claudeResp.StopReason = "end_turn"
	}

	return claudeResp, nil
}

// ConvertToResponsesResponse 将上游响应转换为 Responses 格式
func (p *ResponsesProvider) ConvertToResponsesResponse(
	providerResp *types.ProviderResponse,
	upstreamType string,
	sessionID string,
) (*types.ResponsesResponse, error) {
	respMap, err := converters.JSONToMap(providerResp.Body)
	if err != nil {
		return nil, fmt.Errorf("解析响应失败: %w", err)
	}
	converter := converters.NewConverter(upstreamType)
	return converter.FromProviderResponse(respMap, sessionID)
}

// HandleStreamResponse 处理流式响应
func (p *ResponsesProvider) HandleStreamResponse(body io.ReadCloser) (<-chan string, <-chan error, error) {
	eventChan := make(chan string, 100)
	errChan := make(chan error, 1)

	go func() {
		defer close(eventChan)
		defer body.Close()

		scanner := bufio.NewScanner(body)
		scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)

		pendingEventType := ""
		messageStartSent := false
		textBlockStarted := false
		textBlockIndex := 0
		toolBlockIndex := 1
		currentTool := map[string]string{}
		var currentToolArgs strings.Builder
		latestInputTokens := 0
		latestOutputTokens := 0
		latestCacheCreationTokens := 0
		latestCacheReadTokens := 0
		latestCacheCreation5mTokens := 0
		latestCacheCreation1hTokens := 0
		latestCacheTTL := ""
		stopReason := "end_turn"

		emitJSON := func(eventName string, payload map[string]interface{}) {
			payload["type"] = eventName
			b, _ := json.Marshal(payload)
			eventChan <- fmt.Sprintf("event: %s\ndata: %s\n\n", eventName, string(b))
		}

		for scanner.Scan() {
			line := strings.TrimSpace(normalizeSSEFieldLine(scanner.Text()))
			if line == "" {
				continue
			}
			if strings.HasPrefix(line, "event: ") {
				pendingEventType = strings.TrimPrefix(line, "event: ")
				continue
			}
			if !strings.HasPrefix(line, "data: ") {
				continue
			}

			var data map[string]interface{}
			if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &data); err != nil {
				continue
			}

			eventType := pendingEventType
			if eventType == "" {
				eventType = toString(data["type"])
			}
			pendingEventType = ""

			switch eventType {
			case "response.output_text.delta":
				if !messageStartSent {
					eventChan <- buildMessageStartEvent("responses")
					messageStartSent = true
				}
				if !textBlockStarted {
					emitJSON("content_block_start", map[string]interface{}{
						"index":         textBlockIndex,
						"content_block": map[string]interface{}{"type": "text", "text": ""},
					})
					textBlockStarted = true
				}
				delta := toString(data["delta"])
				if delta != "" {
					emitJSON("content_block_delta", map[string]interface{}{
						"index": textBlockIndex,
						"delta": map[string]interface{}{"type": "text_delta", "text": delta},
					})
				}
			case "response.output_item.added":
				item, _ := data["item"].(map[string]interface{})
				if toString(item["type"]) != "function_call" {
					continue
				}
				if !messageStartSent {
					eventChan <- buildMessageStartEvent("responses")
					messageStartSent = true
				}
				if textBlockStarted {
					emitJSON("content_block_stop", map[string]interface{}{"index": textBlockIndex})
					textBlockStarted = false
				}
				currentTool = map[string]string{
					"id":   toString(item["call_id"]),
					"name": toString(item["name"]),
				}
				currentToolArgs.Reset()
				if currentTool["id"] == "" {
					currentTool["id"] = currentTool["name"]
				}
				emitJSON("content_block_start", map[string]interface{}{
					"index": toolBlockIndex,
					"content_block": map[string]interface{}{
						"type": "tool_use",
						"id":   currentTool["id"],
						"name": currentTool["name"],
					},
				})
			case "response.function_call_arguments.delta":
				if currentTool["id"] == "" {
					continue
				}
				// 先聚合完整 arguments，再一次性发给下游（便于做 JSON 级别清洗）。
				currentToolArgs.WriteString(toString(data["delta"]))
			case "response.output_item.done":
				item, _ := data["item"].(map[string]interface{})
				if toString(item["type"]) == "function_call" && currentTool["id"] != "" {
					argsJSON := currentToolArgs.String()
					if strings.TrimSpace(argsJSON) == "" {
						argsJSON = toString(item["arguments"])
					}
					if strings.TrimSpace(argsJSON) == "" {
						argsJSON = "{}"
					}
					argsJSON = sanitizeClaudeToolArgsJSON(currentTool["name"], argsJSON)

					emitJSON("content_block_delta", map[string]interface{}{
						"index": toolBlockIndex,
						"delta": map[string]interface{}{"type": "input_json_delta", "partial_json": argsJSON},
					})
					emitJSON("content_block_stop", map[string]interface{}{"index": toolBlockIndex})
					toolBlockIndex++
					stopReason = "tool_use"
					currentTool = map[string]string{}
					currentToolArgs.Reset()
				}
			case "response.completed":
				response, _ := data["response"].(map[string]interface{})
				usage, _ := response["usage"].(map[string]interface{})
				if v, ok := usage["input_tokens"].(float64); ok {
					latestInputTokens = int(v)
				}
				if v, ok := usage["output_tokens"].(float64); ok {
					latestOutputTokens = int(v)
				}
				if v, ok := usage["cache_creation_input_tokens"].(float64); ok {
					latestCacheCreationTokens = int(v)
				}
				if v, ok := usage["cache_read_input_tokens"].(float64); ok {
					latestCacheReadTokens = int(v)
				} else {
					latestCacheReadTokens = extractResponsesCacheReadTokens(usage)
				}
				if v, ok := usage["cache_creation_5m_input_tokens"].(float64); ok {
					latestCacheCreation5mTokens = int(v)
				}
				if v, ok := usage["cache_creation_1h_input_tokens"].(float64); ok {
					latestCacheCreation1hTokens = int(v)
				}
				if v, ok := usage["cache_ttl"].(string); ok {
					latestCacheTTL = v
				}
				status := toString(response["status"])
				if status == "incomplete" {
					stopReason = "max_tokens"
				}
				if textBlockStarted {
					emitJSON("content_block_stop", map[string]interface{}{"index": textBlockIndex})
					textBlockStarted = false
				}
				if !messageStartSent {
					eventChan <- buildMessageStartEvent("responses")
					messageStartSent = true
				}
				usagePayload := map[string]interface{}{
					"input_tokens":  latestInputTokens,
					"output_tokens": latestOutputTokens,
				}
				if latestCacheCreationTokens > 0 {
					usagePayload["cache_creation_input_tokens"] = latestCacheCreationTokens
				}
				if latestCacheReadTokens > 0 {
					usagePayload["cache_read_input_tokens"] = latestCacheReadTokens
				}
				if latestCacheCreation5mTokens > 0 {
					usagePayload["cache_creation_5m_input_tokens"] = latestCacheCreation5mTokens
				}
				if latestCacheCreation1hTokens > 0 {
					usagePayload["cache_creation_1h_input_tokens"] = latestCacheCreation1hTokens
				}
				if latestCacheTTL != "" {
					usagePayload["cache_ttl"] = latestCacheTTL
				}
				emitJSON("message_delta", map[string]interface{}{
					"delta": map[string]interface{}{"stop_reason": stopReason, "stop_sequence": nil},
					"usage": usagePayload,
				})
				emitJSON("message_stop", map[string]interface{}{})
			}
		}

		if err := scanner.Err(); err != nil {
			errChan <- err
		}
	}()

	return eventChan, errChan, nil
}

func extractResponsesCacheReadTokens(usage map[string]interface{}) int {
	if cacheRead, ok := usage["cache_read_input_tokens"].(float64); ok {
		return int(cacheRead)
	}
	inputDetails, ok := usage["input_tokens_details"].(map[string]interface{})
	if !ok {
		return 0
	}
	if cachedTokens, ok := inputDetails["cached_tokens"].(float64); ok {
		return int(cachedTokens)
	}
	return 0
}

func responsesReasoningText(item map[string]interface{}) string {
	return reasoningTextFromRaw(firstNonNil(item["summary"], item["content"]))
}

func firstNonNil(values ...interface{}) interface{} {
	for _, value := range values {
		if value != nil {
			return value
		}
	}
	return nil
}

func reasoningTextFromRaw(raw interface{}) string {
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
	default:
		return ""
	}
}

func toString(v interface{}) string {
	if s, ok := v.(string); ok {
		return s
	}
	return ""
}

func stripCodexClientOnlyToolsFromBody(bodyBytes []byte) []byte {
	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		return bodyBytes
	}
	stripCodexClientOnlyTools(reqMap)
	updated, err := json.Marshal(reqMap)
	if err != nil {
		return bodyBytes
	}
	return updated
}

// stripCodexClientOnlyTools 在 /v1/responses 中剥离仅对官方 Codex 有效的工具条目。
// Codex CLI 0.130+ 会在 tools 数组里混入字符串简写（如 "exec_command"、"mcp__chrome_devtools__"）
// 以及 type=namespace/custom/web_search 等客户端侧约定对象，第三方 Responses 镜像通常不认识，
// 会直接 400（例如 anyrouter 报 "Missing required parameter: 'tools[15].tools'"）。
// 这里只保留第三方上游普遍支持的对象型工具（function/tool 等），其它条目连同 tool_choice 一起剥掉。
func stripCodexClientOnlyTools(reqMap map[string]interface{}) {
	rawTools, ok := reqMap["tools"].([]interface{})
	if !ok || len(rawTools) == 0 {
		return
	}

	kept := make([]interface{}, 0, len(rawTools))
	removed := 0
	for _, item := range rawTools {
		switch v := item.(type) {
		case string:
			removed++
		case map[string]interface{}:
			if shouldDropResponsesToolObject(v) {
				removed++
				continue
			}
			kept = append(kept, v)
		default:
			removed++
		}
	}

	if removed == 0 {
		return
	}

	if len(kept) == 0 {
		delete(reqMap, "tools")
		delete(reqMap, "tool_choice")
		delete(reqMap, "parallel_tool_calls")
		return
	}
	reqMap["tools"] = kept
	normalizeToolChoiceAfterToolStrip(reqMap, kept)
}

// convertCodexToolsForPassthrough 将 Codex 原生工具（custom/namespace/web_search 等）
// 转换为 OpenAI function 格式，用于透传分支。转换失败时回退到剥离逻辑。
func convertCodexToolsForPassthrough(reqMap map[string]interface{}) {
	rawTools, ok := reqMap["tools"].([]interface{})
	if !ok || len(rawTools) == 0 {
		return
	}

	converted := converters.ConvertRawToolsToOpenAI(rawTools)
	if len(converted) == 0 {
		stripCodexClientOnlyTools(reqMap)
		return
	}

	tools := make([]interface{}, 0, len(converted))
	for _, t := range converted {
		tools = append(tools, t)
	}
	reqMap["tools"] = tools

	ctx := converters.BuildCodexToolContextFromRaw(rawTools)
	reqMap["tool_choice"] = converters.ConvertToolChoiceForCodex(reqMap["tool_choice"], ctx)
}

func normalizeToolChoiceAfterToolStrip(reqMap map[string]interface{}, keptTools []interface{}) {
	choice, ok := reqMap["tool_choice"].(map[string]interface{})
	if !ok {
		return
	}
	choiceName := extractToolChoiceName(choice)
	if choiceName == "" {
		return
	}
	if hasToolName(keptTools, choiceName) {
		return
	}
	reqMap["tool_choice"] = "auto"
}

func extractToolChoiceName(choice map[string]interface{}) string {
	if name := toString(choice["name"]); name != "" {
		return name
	}
	function, ok := choice["function"].(map[string]interface{})
	if !ok {
		return ""
	}
	return toString(function["name"])
}

func hasToolName(tools []interface{}, name string) bool {
	for _, raw := range tools {
		tool, ok := raw.(map[string]interface{})
		if !ok {
			continue
		}
		if toolName := toString(tool["name"]); toolName == name {
			return true
		}
		function, ok := tool["function"].(map[string]interface{})
		if !ok {
			continue
		}
		if functionName := toString(function["name"]); functionName == name {
			return true
		}
	}
	return false
}

// shouldDropResponsesToolObject 判断对象型工具条目是否为 Codex 客户端专属。
// 仅识别明确的客户端约定类型，未知类型保守保留，避免误伤上游自定义扩展。
func shouldDropResponsesToolObject(tool map[string]interface{}) bool {
	toolType := strings.ToLower(toString(tool["type"]))
	switch toolType {
	case "namespace", "custom", "web_search", "local_shell", "computer_use":
		return true
	}
	return false
}

func normalizeResponsesInputForPassthrough(reqMap map[string]interface{}) {
	input, ok := reqMap["input"].([]interface{})
	if !ok {
		return
	}

	stateless := toString(reqMap["previous_response_id"]) == ""
	if stateless {
		input = normalizeStatelessResponsesToolHistory(input)
		reqMap["input"] = input
	}

	for _, rawItem := range input {
		item, ok := rawItem.(map[string]interface{})
		if !ok {
			continue
		}

		delete(item, "status")

		if toString(item["type"]) != "message" {
			continue
		}

		role := normalizeRole(toString(item["role"]))
		targetTextType := responsesTextContentType(role)
		content, ok := item["content"].([]interface{})
		if !ok {
			continue
		}

		for _, rawBlock := range content {
			block, ok := rawBlock.(map[string]interface{})
			if !ok {
				continue
			}
			blockType := toString(block["type"])
			if blockType == "input_text" || blockType == "output_text" {
				block["type"] = targetTextType
			}
		}
	}
}

func normalizeStatelessResponsesToolHistory(input []interface{}) []interface{} {
	knownCalls := make(map[string]struct{}, len(input))
	for _, rawItem := range input {
		item, ok := rawItem.(map[string]interface{})
		if !ok {
			continue
		}
		if toString(item["type"]) == "function_call" {
			if id := toString(item["call_id"]); id != "" {
				knownCalls[id] = struct{}{}
			}
		}
	}

	normalized := make([]interface{}, 0, len(input))
	for _, rawItem := range input {
		item, ok := rawItem.(map[string]interface{})
		if !ok {
			normalized = append(normalized, rawItem)
			continue
		}

		switch toString(item["type"]) {
		case "function_call":
			normalized = append(normalized, rawItem)
		case "function_call_output":
			callID := toString(item["call_id"])
			if _, paired := knownCalls[callID]; paired {
				normalized = append(normalized, rawItem)
			} else {
				normalized = append(normalized, responsesToolHistoryMessage("user", formatFunctionCallOutputHistory(item)))
			}
		default:
			normalized = append(normalized, rawItem)
		}
	}
	return normalized
}

func responsesToolHistoryMessage(role, text string) map[string]interface{} {
	return map[string]interface{}{
		"type": "message",
		"role": role,
		"content": []interface{}{
			map[string]interface{}{
				"type": responsesTextContentType(role),
				"text": text,
			},
		},
	}
}

func formatFunctionCallHistory(item map[string]interface{}) string {
	name := toString(item["name"])
	callID := toString(item["call_id"])
	arguments := toString(item["arguments"])
	if name == "" {
		name = "function_call"
	}
	if callID != "" {
		return fmt.Sprintf("Function call %s (%s): %s", name, callID, arguments)
	}
	return fmt.Sprintf("Function call %s: %s", name, arguments)
}

func formatFunctionCallOutputHistory(item map[string]interface{}) string {
	callID := toString(item["call_id"])
	output := toString(item["output"])
	if output == "" && item["output"] != nil {
		if outputJSON, err := json.Marshal(item["output"]); err == nil {
			output = string(outputJSON)
		}
	}
	if callID != "" {
		return fmt.Sprintf("Function call output (%s): %s", callID, output)
	}
	return fmt.Sprintf("Function call output: %s", output)
}

func extractEffortFromReasoning(reasoning interface{}) string {
	switch v := reasoning.(type) {
	case map[string]interface{}:
		if effort, ok := v["effort"].(string); ok {
			return effort
		}
	case string:
		return v
	}
	return ""
}

func extractRawToolsFromRequest(bodyBytes []byte) []interface{} {
	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		return nil
	}
	rawTools, _ := reqMap["tools"].([]interface{})
	return rawTools
}
