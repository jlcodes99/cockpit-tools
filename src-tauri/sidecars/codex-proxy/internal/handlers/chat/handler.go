// Package chat 提供 Chat Completions API 的代理处理器
package chat

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/converters"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/middleware"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

// Handler Chat Completions API 代理处理器
// 支持多渠道调度：当配置多个渠道时自动启用
func Handler(
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
) gin.HandlerFunc {
	return gin.HandlerFunc(func(c *gin.Context) {
		// Chat 代理端点统一使用代理访问密钥鉴权（x-api-key / Authorization: Bearer）
		middleware.ProxyAuthMiddleware(envCfg)(c)
		if c.IsAborted() {
			return
		}

		startTime := time.Now()

		// 读取原始请求体
		maxBodySize := envCfg.MaxRequestBodySize
		bodyBytes, err := common.ReadRequestBody(c, maxBodySize)
		if err != nil {
			return
		}
		c.Set("requestBodyBytes", bodyBytes)

		// 解析请求中的关键字段
		var reqMap map[string]interface{}
		if len(bodyBytes) > 0 {
			if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
				c.JSON(400, gin.H{
					"error": gin.H{
						"message": fmt.Sprintf("Invalid request body: %v", err),
						"type":    "invalid_request_error",
						"code":    "invalid_json",
					},
				})
				return
			}
		}

		// 从请求体提取 model
		model, _ := reqMap["model"].(string)
		if model == "" {
			c.JSON(400, gin.H{
				"error": gin.H{
					"message": "model is required",
					"type":    "invalid_request_error",
					"code":    "missing_parameter",
				},
			})
			return
		}

		// 从请求体提取 stream（默认 false）
		isStream, _ := reqMap["stream"].(bool)

		// 提取统一会话标识用于 Trace 亲和性
		userID := utils.ExtractUnifiedSessionID(c, bodyBytes)

		// 记录原始请求信息
		common.LogOriginalRequest(c, bodyBytes, envCfg, "Chat")

		// 检查是否为多渠道模式
		isMultiChannel := channelScheduler.IsMultiChannelMode(scheduler.ChannelKindChat)

		if isMultiChannel {
			handleMultiChannel(c, envCfg, cfgManager, channelScheduler, bodyBytes, model, isStream, userID, startTime)
		} else {
			handleSingleChannel(c, envCfg, cfgManager, channelScheduler, bodyBytes, model, isStream, startTime)
		}
	})
}

// handleMultiChannel 处理多渠道 Chat 请求
func handleMultiChannel(
	c *gin.Context,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	bodyBytes []byte,
	model string,
	isStream bool,
	userID string,
	startTime time.Time,
) {
	metricsManager := channelScheduler.GetChatMetricsManager()
	common.HandleMultiChannelFailover(
		c,
		envCfg,
		channelScheduler,
		scheduler.ChannelKindChat,
		"Chat",
		userID,
		model,
		func(selection *scheduler.SelectionResult) common.MultiChannelAttemptResult {
			upstream := selection.Upstream
			channelIndex := selection.ChannelIndex

			if upstream == nil {
				return common.MultiChannelAttemptResult{}
			}

			baseURLs := upstream.GetAllBaseURLs()
			sortedURLResults := channelScheduler.GetSortedURLsForChannel(scheduler.ChannelKindChat, channelIndex, baseURLs)

			handled, successKey, successBaseURLIdx, failoverErr, usage, lastErr := common.TryUpstreamWithAllKeys(
				c,
				envCfg,
				cfgManager,
				channelScheduler,
				scheduler.ChannelKindChat,
				"Chat",
				metricsManager,
				upstream,
				sortedURLResults,
				bodyBytes,
				isStream,
				func(upstream *config.UpstreamConfig, failedKeys map[string]bool) (string, error) {
					return cfgManager.GetNextChatAPIKey(upstream, failedKeys)
				},
				func(c *gin.Context, upstreamCopy *config.UpstreamConfig, apiKey string) (*http.Request, error) {
					return buildProviderRequest(c, upstreamCopy, upstreamCopy.BaseURL, apiKey, bodyBytes, model, isStream)
				},
				func(apiKey string) {
					_ = cfgManager.DeprioritizeAPIKey(apiKey)
				},
				func(url string) {
					channelScheduler.MarkURLFailure(scheduler.ChannelKindChat, channelIndex, url)
				},
				func(url string) {
					channelScheduler.MarkURLSuccess(scheduler.ChannelKindChat, channelIndex, url)
				},
				func(c *gin.Context, resp *http.Response, upstreamCopy *config.UpstreamConfig, apiKey string, actualRequestBody []byte) (*types.Usage, error) {
					return handleSuccess(c, resp, upstreamCopy.ServiceType, envCfg, startTime, model, isStream, cfgManager.GetFuzzyModeEnabled())
				},
				model,
				"",
				selection.ChannelIndex,
				channelScheduler.GetChannelLogStore(scheduler.ChannelKindChat),
			)

			return common.MultiChannelAttemptResult{
				Handled:           handled,
				Attempted:         true,
				SuccessKey:        successKey,
				SuccessBaseURLIdx: successBaseURLIdx,
				FailoverError:     failoverErr,
				Usage:             usage,
				LastError:         lastErr,
			}
		},
		nil,
		func(ctx *gin.Context, failoverErr *common.FailoverError, lastError error) {
			handleAllChannelsFailed(ctx, failoverErr, lastError)
		},
	)
}

// handleSingleChannel 处理单渠道 Chat 请求
func handleSingleChannel(
	c *gin.Context,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	bodyBytes []byte,
	model string,
	isStream bool,
	startTime time.Time,
) {
	upstream, channelIndex, err := cfgManager.GetCurrentChatUpstreamWithIndex()
	if err != nil {
		chatErrorResponse(c, 503, "No Chat upstream configured", "service_unavailable")
		return
	}

	if len(upstream.APIKeys) == 0 {
		chatErrorResponse(c, 503, fmt.Sprintf("No API keys configured for upstream \"%s\"", upstream.Name), "service_unavailable")
		return
	}

	metricsManager := channelScheduler.GetChatMetricsManager()
	baseURLs := upstream.GetAllBaseURLs()
	urlResults := common.BuildDefaultURLResults(baseURLs)

	handled, _, _, lastFailoverError, _, lastError := common.TryUpstreamWithAllKeys(
		c,
		envCfg,
		cfgManager,
		channelScheduler,
		scheduler.ChannelKindChat,
		"Chat",
		metricsManager,
		upstream,
		urlResults,
		bodyBytes,
		isStream,
		func(upstream *config.UpstreamConfig, failedKeys map[string]bool) (string, error) {
			return cfgManager.GetNextChatAPIKey(upstream, failedKeys)
		},
		func(c *gin.Context, upstreamCopy *config.UpstreamConfig, apiKey string) (*http.Request, error) {
			return buildProviderRequest(c, upstreamCopy, upstreamCopy.BaseURL, apiKey, bodyBytes, model, isStream)
		},
		func(apiKey string) {
			_ = cfgManager.DeprioritizeAPIKey(apiKey)
		},
		nil,
		nil,
		func(c *gin.Context, resp *http.Response, upstreamCopy *config.UpstreamConfig, apiKey string, actualRequestBody []byte) (*types.Usage, error) {
			return handleSuccess(c, resp, upstreamCopy.ServiceType, envCfg, startTime, model, isStream, cfgManager.GetFuzzyModeEnabled())
		},
		model,
		"",
		channelIndex,
		channelScheduler.GetChannelLogStore(scheduler.ChannelKindChat),
	)
	if handled {
		return
	}

	log.Printf("[Chat-Error] 所有 API密钥都失败了")
	handleAllKeysFailed(c, lastFailoverError, lastError)
}

func buildChatCompletionRequestBody(
	bodyBytes []byte,
	model string,
	mappedModel string,
	upstream *config.UpstreamConfig,
	includeAdvancedOptions bool,
) ([]byte, error) {
	needsRewrite := includeAdvancedOptions || mappedModel != model || upstream.NormalizeNonstandardChatRoles
	if !needsRewrite {
		return bodyBytes, nil
	}

	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		return nil, err
	}

	reqMap["model"] = mappedModel

	if includeAdvancedOptions {
		if effort := config.ResolveReasoningEffort(model, upstream); effort != "" {
			if upstream.ReasoningParamStyle == "reasoning_effort" {
				reqMap["reasoning_effort"] = effort
			} else if upstream.ReasoningParamStyle == "thinking" {
				delete(reqMap, "reasoning")
				delete(reqMap, "reasoning_effort")
				if effort != "none" {
					reqMap["thinking"] = map[string]interface{}{"type": "enabled"}
				}
			} else {
				reqMap["reasoning"] = map[string]interface{}{"effort": effort}
			}
		}
		if upstream.TextVerbosity != "" {
			reqMap["text"] = map[string]interface{}{"verbosity": upstream.TextVerbosity}
		}
		if upstream.FastMode {
			reqMap["service_tier"] = "priority"
		}
	}

	if upstream.NormalizeNonstandardChatRoles {
		converters.NormalizeNonstandardChatRolesInRequest(reqMap)
	}

	return json.Marshal(reqMap)
}

// buildProviderRequest 构建上游请求
func buildProviderRequest(
	c *gin.Context,
	upstream *config.UpstreamConfig,
	baseURL string,
	apiKey string,
	bodyBytes []byte,
	model string,
	isStream bool,
) (*http.Request, error) {
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	baseURL = strings.TrimSuffix(strings.TrimRight(baseURL, "/"), "#")
	// 应用模型映射
	mappedModel := config.RedirectModel(model, upstream)

	var requestBody []byte
	var url string

	switch upstream.ServiceType {
	case "openai", "responses", "":
		// OpenAI 兼容上游：透传请求，仅替换 model 并注入高级参数
		var err error
		requestBody, err = buildChatCompletionRequestBody(bodyBytes, model, mappedModel, upstream, true)
		if err != nil {
			return nil, err
		}
		// Gemini 兼容端点配置为 openai serviceType 时，也需要注入 thought_signature
		if strings.Contains(baseURL, "generativelanguage.googleapis.com") && !upstream.StripThoughtSignature {
			requestBody = injectGeminiThoughtSignatures(requestBody)
		}
		if skipVersionPrefix {
			url = fmt.Sprintf("%s/chat/completions", strings.TrimRight(baseURL, "/"))
		} else {
			url = fmt.Sprintf("%s/v1/chat/completions", strings.TrimRight(baseURL, "/"))
		}

	case "claude":
		// Claude 上游：转换 OpenAI Chat 格式为 Claude Messages 格式
		claudeReq, err := convertChatToClaudeRequest(bodyBytes, mappedModel, isStream)
		if err != nil {
			return nil, err
		}
		requestBody, err = json.Marshal(claudeReq)
		if err != nil {
			return nil, err
		}
		if skipVersionPrefix {
			url = fmt.Sprintf("%s/messages", strings.TrimRight(baseURL, "/"))
		} else {
			url = fmt.Sprintf("%s/v1/messages", strings.TrimRight(baseURL, "/"))
		}

	case "gemini":
		// Gemini 上游：透传为 OpenAI Chat 格式（大部分 Gemini 兼容端点支持 OpenAI 格式）
		var err error
		requestBody, err = buildChatCompletionRequestBody(bodyBytes, model, mappedModel, upstream, false)
		if err != nil {
			return nil, err
		}
		// Gemini 3 要求 tool_calls 中包含 thought_signature，注入 dummy 值跳过验证
		// 尊重 stripThoughtSignature 配置：如果渠道明确要求移除 signature 则跳过注入
		if !upstream.StripThoughtSignature {
			requestBody = injectGeminiThoughtSignatures(requestBody)
		}
		if skipVersionPrefix {
			url = fmt.Sprintf("%s/chat/completions", strings.TrimRight(baseURL, "/"))
		} else {
			url = fmt.Sprintf("%s/v1/chat/completions", strings.TrimRight(baseURL, "/"))
		}

	default:
		// 默认当作 OpenAI 兼容处理
		var err error
		requestBody, err = buildChatCompletionRequestBody(bodyBytes, model, mappedModel, upstream, false)
		if err != nil {
			return nil, err
		}
		if skipVersionPrefix {
			url = fmt.Sprintf("%s/chat/completions", strings.TrimRight(baseURL, "/"))
		} else {
			url = fmt.Sprintf("%s/v1/chat/completions", strings.TrimRight(baseURL, "/"))
		}
	}

	req, err := http.NewRequestWithContext(c.Request.Context(), "POST", url, bytes.NewReader(requestBody))
	if err != nil {
		return nil, err
	}

	// 使用统一的头部处理逻辑（透明代理）
	req.Header = utils.PrepareUpstreamHeaders(c, req.URL.Host)

	// 设置 Content-Type
	req.Header.Set("Content-Type", "application/json")

	// 设置认证头
	switch upstream.ServiceType {
	case "claude":
		utils.SetAuthenticationHeader(req.Header, apiKey)
		req.Header.Set("anthropic-version", "2023-06-01")
	default:
		// OpenAI / Gemini / Responses 等都使用 Bearer token
		utils.SetAuthenticationHeader(req.Header, apiKey)
	}

	// 应用自定义请求头
	utils.ApplyCustomHeaders(req.Header, upstream.CustomHeaders)

	return req, nil
}

// convertChatToClaudeRequest 将 OpenAI Chat 请求转换为 Claude Messages 格式
func convertChatToClaudeRequest(bodyBytes []byte, model string, isStream bool) (map[string]interface{}, error) {
	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		return nil, err
	}

	claudeReq := map[string]interface{}{
		"model":  model,
		"stream": isStream,
	}

	// 转换 max_tokens
	if maxTokens, ok := reqMap["max_tokens"]; ok {
		claudeReq["max_tokens"] = maxTokens
	} else if maxCompletionTokens, ok := reqMap["max_completion_tokens"]; ok {
		claudeReq["max_tokens"] = maxCompletionTokens
	} else {
		claudeReq["max_tokens"] = 4096
	}

	// 转换 temperature
	if temp, ok := reqMap["temperature"]; ok {
		claudeReq["temperature"] = temp
	}

	// 转换 top_p
	if topP, ok := reqMap["top_p"]; ok {
		claudeReq["top_p"] = topP
	}

	// 转换 messages：提取 system 消息，其余转为 Claude 格式
	if messages, ok := reqMap["messages"].([]interface{}); ok {
		var claudeMessages []map[string]interface{}
		var systemParts []string

		for _, msg := range messages {
			m, ok := msg.(map[string]interface{})
			if !ok {
				continue
			}
			role, _ := m["role"].(string)
			content, _ := m["content"]

			switch role {
			case "system":
				if text, ok := content.(string); ok {
					systemParts = append(systemParts, text)
				}
			case "user":
				claudeMessages = append(claudeMessages, map[string]interface{}{
					"role":    "user",
					"content": content,
				})
			case "assistant":
				// 检查是否包含 tool_calls（OpenAI → Claude tool_use）
				if toolCalls, ok := m["tool_calls"].([]interface{}); ok && len(toolCalls) > 0 {
					var contentBlocks []map[string]interface{}
					if reasoning, ok := m["reasoning_content"].(string); ok && reasoning != "" {
						contentBlocks = append(contentBlocks, map[string]interface{}{
							"type":     "thinking",
							"thinking": reasoning,
						})
					}
					// 先添加文本内容（如有）
					if text, ok := content.(string); ok && text != "" {
						contentBlocks = append(contentBlocks, map[string]interface{}{
							"type": "text",
							"text": text,
						})
					}
					// 转换 tool_calls → tool_use blocks
					for _, tc := range toolCalls {
						tcMap, ok := tc.(map[string]interface{})
						if !ok {
							continue
						}
						fn, _ := tcMap["function"].(map[string]interface{})
						toolID, _ := tcMap["id"].(string)
						toolName, _ := fn["name"].(string)
						argsStr, _ := fn["arguments"].(string)
						var argsObj interface{}
						if json.Unmarshal([]byte(argsStr), &argsObj) != nil {
							argsObj = map[string]interface{}{}
						}
						contentBlocks = append(contentBlocks, map[string]interface{}{
							"type":  "tool_use",
							"id":    toolID,
							"name":  toolName,
							"input": argsObj,
						})
					}
					claudeMessages = append(claudeMessages, map[string]interface{}{
						"role":    "assistant",
						"content": contentBlocks,
					})
				} else {
					if reasoning, ok := m["reasoning_content"].(string); ok && reasoning != "" {
						var contentBlocks []map[string]interface{}
						contentBlocks = append(contentBlocks, map[string]interface{}{
							"type":     "thinking",
							"thinking": reasoning,
						})
						if text, ok := content.(string); ok && text != "" {
							contentBlocks = append(contentBlocks, map[string]interface{}{
								"type": "text",
								"text": text,
							})
						}
						claudeMessages = append(claudeMessages, map[string]interface{}{
							"role":    "assistant",
							"content": contentBlocks,
						})
						continue
					}
					claudeMessages = append(claudeMessages, map[string]interface{}{
						"role":    "assistant",
						"content": content,
					})
				}
			case "tool":
				// OpenAI tool result → Claude tool_result（作为 user 消息）
				toolCallID, _ := m["tool_call_id"].(string)
				contentStr := ""
				if s, ok := content.(string); ok {
					contentStr = s
				}
				claudeMessages = append(claudeMessages, map[string]interface{}{
					"role": "user",
					"content": []map[string]interface{}{
						{
							"type":        "tool_result",
							"tool_use_id": toolCallID,
							"content":     contentStr,
						},
					},
				})
			default:
				claudeMessages = append(claudeMessages, map[string]interface{}{
					"role":    "user",
					"content": content,
				})
			}
		}

		if len(systemParts) > 0 {
			claudeReq["system"] = strings.Join(systemParts, "\n\n")
		}
		claudeReq["messages"] = claudeMessages
	}

	// 转换 tools：OpenAI function → Claude tools
	if tools, ok := reqMap["tools"].([]interface{}); ok && len(tools) > 0 {
		var claudeTools []map[string]interface{}
		for _, tool := range tools {
			t, ok := tool.(map[string]interface{})
			if !ok {
				continue
			}
			fn, ok := t["function"].(map[string]interface{})
			if !ok {
				continue
			}
			claudeTool := map[string]interface{}{
				"name": fn["name"],
			}
			if desc, ok := fn["description"]; ok {
				claudeTool["description"] = desc
			}
			if params, ok := fn["parameters"]; ok {
				claudeTool["input_schema"] = params
			} else {
				claudeTool["input_schema"] = map[string]interface{}{
					"type":       "object",
					"properties": map[string]interface{}{},
				}
			}
			claudeTools = append(claudeTools, claudeTool)
		}
		if len(claudeTools) > 0 {
			claudeReq["tools"] = claudeTools
		}
	}

	return claudeReq, nil
}

// handleSuccess 处理成功的响应
func handleSuccess(
	c *gin.Context,
	resp *http.Response,
	upstreamType string,
	envCfg *config.EnvConfig,
	startTime time.Time,
	model string,
	isStream bool,
	fuzzyMode bool,
) (*types.Usage, error) {
	defer resp.Body.Close()

	if isStream {
		return handleStreamSuccess(c, resp, upstreamType, envCfg, startTime, model), nil
	}

	// 非流式响应处理
	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		chatErrorResponse(c, 500, "Failed to read response", "server_error")
		return nil, err
	}

	if envCfg.EnableResponseLogs {
		responseTime := time.Since(startTime).Milliseconds()
		log.Printf("[Chat-Timing] 响应完成: %dms, 状态: %d", responseTime, resp.StatusCode)
		common.LogUpstreamResponse(resp, bodyBytes, envCfg, "Chat")
	}

	switch upstreamType {
	case "claude":
		// 转换 Claude 响应为 OpenAI Chat 格式
		var claudeResp map[string]interface{}
		if err := json.Unmarshal(bodyBytes, &claudeResp); err != nil {
			return nil, fmt.Errorf("%w: %v", common.ErrInvalidResponseBody, err)
		}
		// 空响应拦截（仅 Fuzzy 模式）：在原生 Claude 结构上判空，避免
		// convertClaudeResponseToChat 丢失 server_tool_use / redacted_thinking
		// 等语义块导致的误判。Header 未发送，可安全 failover。
		if fuzzyMode {
			var claudeTyped types.ClaudeResponse
			if err := json.Unmarshal(bodyBytes, &claudeTyped); err == nil && common.IsClaudeResponseEmpty(&claudeTyped) {
				log.Printf("[Chat-EmptyResponse] 上游返回空响应（非流式，upstreamType=%s），触发 failover", upstreamType)
				return nil, common.ErrEmptyNonStreamResponse
			}
		}
		openaiResp := convertClaudeResponseToChat(claudeResp, model)
		respBytes, err := json.Marshal(openaiResp)
		if err != nil {
			c.Data(resp.StatusCode, "application/json", bodyBytes)
			return nil, nil
		}
		c.Data(resp.StatusCode, "application/json", respBytes)

		// 提取 usage
		var usage *types.Usage
		if u, ok := claudeResp["usage"].(map[string]interface{}); ok {
			inputTokens, _ := u["input_tokens"].(float64)
			outputTokens, _ := u["output_tokens"].(float64)
			usage = &types.Usage{
				InputTokens:  int(inputTokens),
				OutputTokens: int(outputTokens),
			}
		}
		return usage, nil

	default:
		// 先解析以判断空响应；再决定是 failover 还是透传
		var respMap map[string]interface{}
		if err := json.Unmarshal(bodyBytes, &respMap); err != nil {
			// JSON 不可解析：维持原 ErrInvalidResponseBody 语义
			return nil, fmt.Errorf("%w: %v", common.ErrInvalidResponseBody, err)
		}
		if fuzzyMode && common.IsChatResponseEmpty(respMap) {
			log.Printf("[Chat-EmptyResponse] 上游返回空响应（非流式，upstreamType=%s），触发 failover", upstreamType)
			return nil, common.ErrEmptyNonStreamResponse
		}
		// 透传原始响应体（保留上游字段，避免 marshal 丢失）
		utils.ForwardResponseHeaders(resp.Header, c.Writer)
		c.Data(resp.StatusCode, "application/json", bodyBytes)
		if u, ok := respMap["usage"].(map[string]interface{}); ok {
			promptTokens, _ := u["prompt_tokens"].(float64)
			completionTokens, _ := u["completion_tokens"].(float64)
			return &types.Usage{
				InputTokens:  int(promptTokens),
				OutputTokens: int(completionTokens),
			}, nil
		}
		return nil, nil
	}
}

// convertClaudeResponseToChat 将 Claude 非流式响应转换为 OpenAI Chat 格式
func convertClaudeResponseToChat(claudeResp map[string]interface{}, model string) map[string]interface{} {
	// 提取文本内容和 tool_use blocks
	var text string
	var reasoningParts []string
	var toolCalls []map[string]interface{}
	toolCallIndex := 0

	if content, ok := claudeResp["content"].([]interface{}); ok {
		for _, block := range content {
			b, ok := block.(map[string]interface{})
			if !ok {
				continue
			}
			blockType, _ := b["type"].(string)
			switch blockType {
			case "thinking":
				if thinking, ok := b["thinking"].(string); ok && thinking != "" {
					reasoningParts = append(reasoningParts, thinking)
				}
			case "text":
				if t, ok := b["text"].(string); ok {
					text += t
				}
			case "tool_use":
				// Claude tool_use → OpenAI tool_calls
				toolID, _ := b["id"].(string)
				toolName, _ := b["name"].(string)
				inputRaw, _ := json.Marshal(b["input"])
				toolCalls = append(toolCalls, map[string]interface{}{
					"index": toolCallIndex,
					"id":    toolID,
					"type":  "function",
					"function": map[string]interface{}{
						"name":      toolName,
						"arguments": string(inputRaw),
					},
				})
				toolCallIndex++
			default:
				// 其他类型（如 image）提取 text 字段（如有）
				if t, ok := b["text"].(string); ok {
					text += t
				}
			}
		}
	}

	// 映射 stop_reason
	finishReason := "stop"
	if stopReason, ok := claudeResp["stop_reason"].(string); ok {
		switch stopReason {
		case "max_tokens":
			finishReason = "length"
		case "tool_use":
			finishReason = "tool_calls"
		default: // end_turn, stop_sequence
			finishReason = "stop"
		}
	}

	// 构建 message
	message := map[string]interface{}{
		"role": "assistant",
	}
	if text != "" {
		message["content"] = text
	} else {
		message["content"] = nil
	}
	if len(toolCalls) > 0 {
		message["tool_calls"] = toolCalls
	}
	if len(reasoningParts) > 0 {
		message["reasoning_content"] = strings.Join(reasoningParts, "\n")
	}

	// 构建 OpenAI Chat 格式响应
	result := map[string]interface{}{
		"id":      claudeResp["id"],
		"object":  "chat.completion",
		"created": time.Now().Unix(),
		"model":   model,
		"choices": []map[string]interface{}{
			{
				"index":         0,
				"message":       message,
				"finish_reason": finishReason,
			},
		},
	}

	// 转换 usage
	if u, ok := claudeResp["usage"].(map[string]interface{}); ok {
		inputTokens, _ := u["input_tokens"].(float64)
		outputTokens, _ := u["output_tokens"].(float64)
		result["usage"] = map[string]interface{}{
			"prompt_tokens":     int(inputTokens),
			"completion_tokens": int(outputTokens),
			"total_tokens":      int(inputTokens + outputTokens),
		}
	}

	return result
}

// handleStreamSuccess 处理流式响应
func handleStreamSuccess(
	c *gin.Context,
	resp *http.Response,
	upstreamType string,
	envCfg *config.EnvConfig,
	startTime time.Time,
	model string,
) *types.Usage {
	// 设置 SSE 响应头
	c.Header("Content-Type", "text/event-stream")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	c.Header("X-Accel-Buffering", "no")

	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		log.Printf("[Chat-Stream] 警告: ResponseWriter 不支持 Flusher")
	}

	var totalUsage *types.Usage
	logBuffer := common.NewLimitedLogBuffer(common.MaxUpstreamResponseLogBytes)
	streamLoggingEnabled := envCfg.EnableResponseLogs && envCfg.IsDevelopment()

	common.LogUpstreamResponseHeaders(resp, envCfg, "Chat")

	switch upstreamType {
	case "claude":
		totalUsage = streamClaudeToChat(c, resp, flusher, model, logBuffer, streamLoggingEnabled)
	default:
		// OpenAI / Gemini / Responses 等：直接透传 SSE 流
		totalUsage = streamPassthrough(c, resp, flusher, logBuffer, streamLoggingEnabled)
	}

	if envCfg.EnableResponseLogs {
		responseTime := time.Since(startTime).Milliseconds()
		log.Printf("[Chat-Stream-Timing] 流式响应完成: %dms", responseTime)
		if logBuffer.Len() > 0 {
			log.Printf("[Chat-Stream] 上游流式响应原始内容:\n%s", logBuffer.String())
		}
	}

	return totalUsage
}

// streamPassthrough 直接透传 SSE 流（用于 OpenAI 兼容上游）
func streamPassthrough(
	c *gin.Context,
	resp *http.Response,
	flusher http.Flusher,
	logBuffer *common.LimitedLogBuffer,
	loggingEnabled bool,
) *types.Usage {
	var totalUsage *types.Usage
	buf := make([]byte, 32*1024)
	var remainder string

	for {
		n, err := resp.Body.Read(buf)
		if n > 0 {
			if loggingEnabled {
				logBuffer.Write(buf[:n])
			}
			// 使用行缓冲机制避免跨 chunk 截断
			data := remainder + string(buf[:n])
			lines := strings.Split(data, "\n")
			remainder = lines[len(lines)-1]
			completeLines := lines[:len(lines)-1]

			// 尝试从完整行中提取 usage
			for _, line := range completeLines {
				if !strings.HasPrefix(line, "data: ") {
					continue
				}
				jsonData := strings.TrimPrefix(line, "data: ")
				if jsonData == "[DONE]" {
					continue
				}
				var parsed map[string]interface{}
				if json.Unmarshal([]byte(jsonData), &parsed) == nil {
					if u, ok := parsed["usage"].(map[string]interface{}); ok {
						promptTokens, _ := u["prompt_tokens"].(float64)
						completionTokens, _ := u["completion_tokens"].(float64)
						totalUsage = &types.Usage{
							InputTokens:  int(promptTokens),
							OutputTokens: int(completionTokens),
						}
					}
				}
			}

			c.Writer.Write(buf[:n])
			if flusher != nil {
				flusher.Flush()
			}
		}
		if err != nil {
			break
		}
	}

	return totalUsage
}

// streamClaudeToChat Claude 流式响应转换为 OpenAI Chat 格式
func streamClaudeToChat(
	c *gin.Context,
	resp *http.Response,
	flusher http.Flusher,
	model string,
	logBuffer *common.LimitedLogBuffer,
	loggingEnabled bool,
) *types.Usage {
	var totalUsage *types.Usage
	var doneSent bool
	buf := make([]byte, 32*1024)
	var remainder string

	for {
		n, readErr := resp.Body.Read(buf)
		if n > 0 {
			if loggingEnabled {
				logBuffer.Write(buf[:n])
			}
			data := remainder + string(buf[:n])
			lines := strings.Split(data, "\n")
			// 最后一行可能不完整，保留到下次
			remainder = lines[len(lines)-1]
			lines = lines[:len(lines)-1]

			for _, line := range lines {
				if !strings.HasPrefix(line, "data: ") {
					continue
				}
				jsonData := strings.TrimPrefix(line, "data: ")
				if jsonData == "[DONE]" {
					fmt.Fprintf(c.Writer, "data: [DONE]\n\n")
					if flusher != nil {
						flusher.Flush()
					}
					doneSent = true
					continue
				}

				var event map[string]interface{}
				if err := json.Unmarshal([]byte(jsonData), &event); err != nil {
					continue
				}

				eventType, _ := event["type"].(string)

				switch eventType {
				case "content_block_delta":
					delta, ok := event["delta"].(map[string]interface{})
					if !ok {
						continue
					}
					deltaType, _ := delta["type"].(string)
					switch deltaType {
					case "thinking_delta":
						thinking, _ := delta["thinking"].(string)
						if thinking == "" {
							continue
						}
						chatChunk := map[string]interface{}{
							"id":      "chatcmpl-claude",
							"object":  "chat.completion.chunk",
							"created": time.Now().Unix(),
							"model":   model,
							"choices": []map[string]interface{}{
								{
									"index": 0,
									"delta": map[string]interface{}{
										"reasoning_content": thinking,
									},
									"finish_reason": nil,
								},
							},
						}
						chunkBytes, _ := json.Marshal(chatChunk)
						fmt.Fprintf(c.Writer, "data: %s\n\n", string(chunkBytes))
						if flusher != nil {
							flusher.Flush()
						}
					case "text_delta":
						text, _ := delta["text"].(string)
						chatChunk := map[string]interface{}{
							"id":      "chatcmpl-claude",
							"object":  "chat.completion.chunk",
							"created": time.Now().Unix(),
							"model":   model,
							"choices": []map[string]interface{}{
								{
									"index": 0,
									"delta": map[string]interface{}{
										"content": text,
									},
									"finish_reason": nil,
								},
							},
						}
						chunkBytes, _ := json.Marshal(chatChunk)
						fmt.Fprintf(c.Writer, "data: %s\n\n", string(chunkBytes))
						if flusher != nil {
							flusher.Flush()
						}
					}

				case "message_delta":
					// 消息完成
					stopChunk := map[string]interface{}{
						"id":      "chatcmpl-claude",
						"object":  "chat.completion.chunk",
						"created": time.Now().Unix(),
						"model":   model,
						"choices": []map[string]interface{}{
							{
								"index":         0,
								"delta":         map[string]interface{}{},
								"finish_reason": "stop",
							},
						},
					}

					// 提取 usage
					if usage, ok := event["usage"].(map[string]interface{}); ok {
						inputTokens, _ := usage["input_tokens"].(float64)
						outputTokens, _ := usage["output_tokens"].(float64)
						totalUsage = &types.Usage{
							InputTokens:  int(inputTokens),
							OutputTokens: int(outputTokens),
						}
						stopChunk["usage"] = map[string]interface{}{
							"prompt_tokens":     int(inputTokens),
							"completion_tokens": int(outputTokens),
							"total_tokens":      int(inputTokens + outputTokens),
						}
					}

					chunkBytes, _ := json.Marshal(stopChunk)
					fmt.Fprintf(c.Writer, "data: %s\n\n", string(chunkBytes))
					if flusher != nil {
						flusher.Flush()
					}

				case "message_start":
					// 提取初始 usage（input_tokens）
					if msg, ok := event["message"].(map[string]interface{}); ok {
						if usage, ok := msg["usage"].(map[string]interface{}); ok {
							inputTokens, _ := usage["input_tokens"].(float64)
							totalUsage = &types.Usage{
								InputTokens:  int(inputTokens),
								OutputTokens: 0,
							}
						}
					}
				}
			}
		}
		if readErr != nil {
			break
		}
	}

	// 确保发送 [DONE]（仅在未发送过时）
	if !doneSent {
		fmt.Fprintf(c.Writer, "data: [DONE]\n\n")
		if flusher != nil {
			flusher.Flush()
		}
	}

	return totalUsage
}

// chatErrorResponse 返回 OpenAI 格式的错误响应
func chatErrorResponse(c *gin.Context, statusCode int, message string, code string) {
	c.JSON(statusCode, gin.H{
		"error": gin.H{
			"message": message,
			"type":    "server_error",
			"code":    code,
		},
	})
}

// handleAllChannelsFailed 处理所有渠道失败的情况
func handleAllChannelsFailed(c *gin.Context, failoverErr *common.FailoverError, lastError error) {
	if failoverErr != nil {
		c.Data(failoverErr.Status, "application/json", failoverErr.Body)
		return
	}

	errMsg := "All channels failed"
	if lastError != nil {
		errMsg = lastError.Error()
	}

	chatErrorResponse(c, 503, errMsg, "service_unavailable")
}

// handleAllKeysFailed 处理所有 Key 失败的情况
func handleAllKeysFailed(c *gin.Context, failoverErr *common.FailoverError, lastError error) {
	if failoverErr != nil {
		c.Data(failoverErr.Status, "application/json", failoverErr.Body)
		return
	}

	errMsg := "All API keys failed"
	if lastError != nil {
		errMsg = lastError.Error()
	}

	chatErrorResponse(c, 503, errMsg, "service_unavailable")
}

// injectGeminiThoughtSignatures 为 Gemini 上游注入 thought_signature
// Gemini 3 模型要求 assistant message 中每个 step 的第一个 tool_call 必须包含 thought_signature，
// 否则返回 400。对于没有 thought_signature 的 tool_calls，注入 dummy 值跳过验证。
// 参考: https://ai.google.dev/gemini-api/docs/thought-signatures
func injectGeminiThoughtSignatures(body []byte) []byte {
	var reqMap map[string]interface{}
	if err := json.Unmarshal(body, &reqMap); err != nil {
		return body
	}

	messages, ok := reqMap["messages"].([]interface{})
	if !ok {
		return body
	}

	modified := false
	for _, msg := range messages {
		msgMap, ok := msg.(map[string]interface{})
		if !ok {
			continue
		}

		role, _ := msgMap["role"].(string)
		if role != "assistant" {
			continue
		}

		toolCalls, ok := msgMap["tool_calls"].([]interface{})
		if !ok || len(toolCalls) == 0 {
			continue
		}

		// 只需要为第一个 tool_call 注入（parallel FC 只有第一个需要 signature）
		firstTC, ok := toolCalls[0].(map[string]interface{})
		if !ok {
			continue
		}

		// 检查是否已有 extra_content.google.thought_signature
		if hasThoughtSignature(firstTC) {
			continue
		}

		// 注入 dummy thought_signature，保留已有的 extra_content 字段
		extraContent, ok := firstTC["extra_content"].(map[string]interface{})
		if !ok {
			extraContent = map[string]interface{}{}
		}
		google, ok := extraContent["google"].(map[string]interface{})
		if !ok {
			google = map[string]interface{}{}
		}
		google["thought_signature"] = types.DummyThoughtSignature
		extraContent["google"] = google
		firstTC["extra_content"] = extraContent
		modified = true
	}

	if !modified {
		return body
	}

	result, err := json.Marshal(reqMap)
	if err != nil {
		return body
	}
	return result
}

// hasThoughtSignature 检查 tool_call 是否已包含 thought_signature
func hasThoughtSignature(toolCall map[string]interface{}) bool {
	extraContent, ok := toolCall["extra_content"].(map[string]interface{})
	if !ok {
		return false
	}
	google, ok := extraContent["google"].(map[string]interface{})
	if !ok {
		return false
	}
	sig, ok := google["thought_signature"].(string)
	return ok && sig != ""
}
