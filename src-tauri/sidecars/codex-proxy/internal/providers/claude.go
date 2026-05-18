package providers

import (
	"bufio"
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"regexp"
	"strings"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

// ClaudeProvider Claude 提供商（直接透传）
type ClaudeProvider struct{}

// redirectModelInBody 仅修改请求体中的 model 字段，保持其他内容不变
// 使用 map[string]interface{} 避免结构体字段丢失问题
func redirectModelInBody(bodyBytes []byte, upstream *config.UpstreamConfig) []byte {
	decoder := json.NewDecoder(bytes.NewReader(bodyBytes))
	decoder.UseNumber() // 保留数字精度

	var data map[string]interface{}
	if err := decoder.Decode(&data); err != nil {
		return bodyBytes // 解析失败，返回原始数据
	}

	model, ok := data["model"].(string)
	if !ok {
		return bodyBytes // 没有 model 字段或类型不对
	}

	newModel := config.RedirectModel(model, upstream)
	if newModel == model {
		return bodyBytes // 模型未变，无需重编码
	}

	data["model"] = newModel

	// 使用 Encoder 并禁用 HTML 转义，保持原始格式
	newBytes, err := utils.MarshalJSONNoEscape(data)
	if err != nil {
		return bodyBytes // 编码失败，返回原始数据
	}
	return newBytes
}

// convertThinkingToReasoningContent 将 assistant 消息中的 thinking 内容块转为 reasoning_content 字段
// 用于兼容 mimo 等使用 Claude 协议但要求 OpenAI 风格 reasoning_content 回传的上游
func convertThinkingToReasoningContent(bodyBytes []byte) []byte {
	decoder := json.NewDecoder(bytes.NewReader(bodyBytes))
	decoder.UseNumber()

	var data map[string]interface{}
	if err := decoder.Decode(&data); err != nil {
		return bodyBytes
	}

	messages, ok := data["messages"].([]interface{})
	if !ok {
		return bodyBytes
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

		content, ok := msgMap["content"].([]interface{})
		if !ok {
			continue
		}

		var thinkingTexts []string
		for _, block := range content {
			blockMap, ok := block.(map[string]interface{})
			if !ok {
				continue
			}
			if blockType, _ := blockMap["type"].(string); blockType == "thinking" {
				if thinking, ok := blockMap["thinking"].(string); ok && thinking != "" {
					thinkingTexts = append(thinkingTexts, thinking)
				}
			}
		}

		if len(thinkingTexts) > 0 {
			msgMap["reasoning_content"] = strings.Join(thinkingTexts, "\n")
			modified = true
		}
	}

	if !modified {
		return bodyBytes
	}

	newBytes, err := utils.MarshalJSONNoEscape(data)
	if err != nil {
		return bodyBytes
	}
	return newBytes
}

// convertReasoningContentToThinking 将响应中的 reasoning_content 转为 Claude thinking 内容块
// 用于兼容 mimo 等返回 OpenAI 风格 reasoning_content 的 Claude 协议上游
func convertReasoningContentToThinking(bodyBytes []byte) []byte {
	decoder := json.NewDecoder(bytes.NewReader(bodyBytes))
	decoder.UseNumber()

	var data map[string]interface{}
	if err := decoder.Decode(&data); err != nil {
		return bodyBytes
	}

	modified := false

	// 处理顶层 reasoning_content（如果存在）
	if reasoningContent, ok := data["reasoning_content"].(string); ok && reasoningContent != "" {
		content, ok := data["content"].([]interface{})
		if !ok {
			content = []interface{}{}
		}

		// 在 content 数组开头插入 thinking 块
		thinkingBlock := map[string]interface{}{
			"type":     "thinking",
			"thinking": reasoningContent,
		}
		newContent := append([]interface{}{thinkingBlock}, content...)
		data["content"] = newContent
		delete(data, "reasoning_content")
		modified = true
	}

	// 处理 content 数组中的 reasoning_content（如果存在）
	if content, ok := data["content"].([]interface{}); ok {
		for i, block := range content {
			blockMap, ok := block.(map[string]interface{})
			if !ok {
				continue
			}

			if reasoningContent, exists := blockMap["reasoning_content"].(string); exists && reasoningContent != "" {
				// 将 reasoning_content 转为 thinking 块
				blockMap["type"] = "thinking"
				blockMap["thinking"] = reasoningContent
				delete(blockMap, "reasoning_content")
				content[i] = blockMap
				modified = true
			}
		}
	}

	if !modified {
		return bodyBytes
	}

	newBytes, err := utils.MarshalJSONNoEscape(data)
	if err != nil {
		return bodyBytes
	}
	return newBytes
}

// ConvertToProviderRequest 转换为 Claude 请求（实现真正的透传）
func (p *ClaudeProvider) ConvertToProviderRequest(c *gin.Context, upstream *config.UpstreamConfig, apiKey string) (*http.Request, []byte, error) {
	// 读取原始请求体
	bodyBytes, err := getRequestBodyBytes(c)
	if err != nil {
		return nil, nil, err
	}

	// 模型重定向：仅修改 model 字段，保持其他内容不变
	if upstream.ModelMapping != nil && len(upstream.ModelMapping) > 0 {
		bodyBytes = redirectModelInBody(bodyBytes, upstream)
	}

	// thinking 块 → reasoning_content 转换（兼容 mimo 等要求 OpenAI 风格 reasoning_content 的 Claude 协议上游）
	if upstream.PassbackReasoningContent {
		bodyBytes = convertThinkingToReasoningContent(bodyBytes)
	}

	// 构建目标URL
	// 智能拼接逻辑：
	// 1. 如果 baseURL 以 # 结尾，跳过自动添加 /v1
	// 2. 如果 baseURL 已包含版本号后缀（如 /v1, /v2, /v3），直接拼接端点路径
	// 3. 如果 baseURL 不包含版本号后缀，自动添加 /v1 再拼接端点路径
	// 先剥离 routePrefix（如 /glm），再剥离 /v1，得到纯端点路径（如 /messages）
	path := c.Request.URL.Path
	if routePrefix := c.Param("routePrefix"); routePrefix != "" {
		path = strings.TrimPrefix(path, "/"+routePrefix)
	}
	endpoint := strings.TrimPrefix(path, "/v1")
	baseURL := upstream.GetEffectiveBaseURL()
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	if skipVersionPrefix {
		baseURL = strings.TrimSuffix(baseURL, "#")
	}
	baseURL = strings.TrimSuffix(baseURL, "/")

	// 使用正则表达式检测 baseURL 是否以版本号结尾（/v1, /v2, /v1beta, /v2alpha等）
	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)

	var targetURL string
	if versionPattern.MatchString(baseURL) || skipVersionPrefix {
		// baseURL 已包含版本号或以#结尾，直接拼接
		targetURL = baseURL + endpoint
	} else {
		// baseURL 不包含版本号，添加 /v1
		targetURL = baseURL + "/v1" + endpoint
	}

	if c.Request.URL.RawQuery != "" {
		targetURL += "?" + c.Request.URL.RawQuery
	}

	// 创建请求
	var req *http.Request
	if len(bodyBytes) > 0 {
		req, err = http.NewRequestWithContext(c.Request.Context(), c.Request.Method, targetURL, bytes.NewReader(bodyBytes))
	} else {
		// 如果 bodyBytes 为空（例如 GET 请求或原始请求体为空），则直接使用 nil Body
		req, err = http.NewRequestWithContext(c.Request.Context(), c.Request.Method, targetURL, nil)
	}
	if err != nil {
		return nil, nil, err
	}

	// 使用统一的头部处理逻辑
	req.Header = utils.PrepareUpstreamHeaders(c, req.URL.Host)
	utils.ApplyCustomHeaders(req.Header, upstream.CustomHeaders) // 先应用自定义头，后覆盖认证（不可被自定义头覆盖）
	utils.SetAuthenticationHeader(req.Header, apiKey)
	utils.EnsureCompatibleUserAgent(req.Header, "claude")

	return req, bodyBytes, nil
}

// ConvertToClaudeResponse 转换为 Claude 响应（直接透传）
func (p *ClaudeProvider) ConvertToClaudeResponse(providerResp *types.ProviderResponse) (*types.ClaudeResponse, error) {
	var claudeResp types.ClaudeResponse
	if err := json.Unmarshal(providerResp.Body, &claudeResp); err != nil {
		return nil, err
	}

	// 检查响应中是否包含 reasoning_content（mimo 等上游可能返回此字段）
	// 如果存在，转换为 Claude thinking 内容块
	var rawResp map[string]interface{}
	if err := json.Unmarshal(providerResp.Body, &rawResp); err == nil {
		if content, ok := rawResp["content"].([]interface{}); ok {
			// 检查是否有 reasoning_content 需要转换
			hasReasoningContent := false
			for _, block := range content {
				if blockMap, ok := block.(map[string]interface{}); ok {
					if _, exists := blockMap["reasoning_content"]; exists {
						hasReasoningContent = true
						break
					}
				}
			}

			// 或者检查顶层是否有 reasoning_content
			if !hasReasoningContent {
				if _, exists := rawResp["reasoning_content"]; exists {
					hasReasoningContent = true
				}
			}

			if hasReasoningContent {
				convertedBody := convertReasoningContentToThinking(providerResp.Body)
				if err := json.Unmarshal(convertedBody, &claudeResp); err == nil {
					return &claudeResp, nil
				}
			}
		}
	}

	return &claudeResp, nil
}

// HandleStreamResponse 处理流式响应（直接透传）
func (p *ClaudeProvider) HandleStreamResponse(body io.ReadCloser) (<-chan string, <-chan error, error) {
	eventChan := make(chan string, 100)
	errChan := make(chan error, 1)

	go func() {
		defer close(eventChan)
		defer close(errChan)
		defer body.Close()

		scanner := bufio.NewScanner(body)
		// 设置更大的 buffer (1MB) 以处理大 JSON chunk，避免默认 64KB 限制
		const maxScannerBufferSize = 1024 * 1024 // 1MB
		scanner.Buffer(make([]byte, 0, 64*1024), maxScannerBufferSize)

		toolUseStopEmitted := false

		// 注意：为了让下游的 token 注入/修补逻辑保持正确，这里必须按「完整 SSE 事件」转发。
		// 上游以空行分隔事件：event/data/id/retry/... + "\n"，空行 => 事件结束。
		var eventBuf strings.Builder

		flushEvent := func() {
			if eventBuf.Len() == 0 {
				return
			}
			eventChan <- eventBuf.String()
			eventBuf.Reset()
		}

		for scanner.Scan() {
			line := normalizeSSEFieldLine(scanner.Text())

			// 检测是否发送了 tool_use 相关的 stop_reason（通常在 data 行中）
			if strings.Contains(line, `"stop_reason":"tool_use"`) ||
				strings.Contains(line, `"stop_reason": "tool_use"`) {
				toolUseStopEmitted = true
			}

			// 转换流式响应中的 reasoning_content → thinking（兼容 mimo 等上游）
			if strings.HasPrefix(line, "data: ") && strings.Contains(line, `"reasoning_content"`) {
				dataJSON := strings.TrimPrefix(line, "data: ")
				if dataJSON != "[DONE]" {
					var eventData map[string]interface{}
					if err := json.Unmarshal([]byte(dataJSON), &eventData); err == nil {
						// 检查是否包含 reasoning_content
						if delta, ok := eventData["delta"].(map[string]interface{}); ok {
							if reasoningContent, exists := delta["reasoning_content"].(string); exists && reasoningContent != "" {
								// 将 reasoning_content 转为 thinking_delta
								delta["type"] = "thinking_delta"
								delta["thinking"] = reasoningContent
								delete(delta, "reasoning_content")

								// 重新序列化
								if newJSON, err := json.Marshal(eventData); err == nil {
									line = "data: " + string(newJSON)
								}
							}
						}
					}
				}
			}

			// 透传所有 SSE 字段（包括注释、id、retry 等）
			eventBuf.WriteString(line)
			eventBuf.WriteString("\n")

			// 空行表示一个 SSE event 结束
			if line == "" {
				flushEvent()
			}
		}

		// 若上游未以空行结尾，仍尝试把最后的残留事件发出去
		flushEvent()

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
		}
	}()

	return eventChan, errChan, nil
}
