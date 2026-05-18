// Package responses 提供 Responses API 的处理器
package responses

import (
	"bufio"
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
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/providers"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
	"github.com/tidwall/sjson"
)

// Handler Responses API 代理处理器
// 支持多渠道调度：当配置多个渠道时自动启用
func Handler(
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	sessionManager *session.SessionManager,
	channelScheduler *scheduler.ChannelScheduler,
) gin.HandlerFunc {
	return gin.HandlerFunc(func(c *gin.Context) {
		// 先进行认证
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

		// 入口保留原始请求体；按渠道在发往上游前决定是否规范化 metadata.user_id
		c.Set("requestBodyBytes", bodyBytes)

		// 解析 Responses 请求
		var responsesReq types.ResponsesRequest
		if len(bodyBytes) > 0 {
			_ = json.Unmarshal(bodyBytes, &responsesReq)
			responsesReq.RawTools = extractRawToolsFromRequestBody(bodyBytes)
		}

		// 提取统一会话标识用于 Trace 亲和性（保持 metadata.user_id 默认规范化后的既有路由语义）
		affinityBody := common.NormalizeMetadataUserID(bodyBytes)
		userID := utils.ExtractUnifiedSessionID(c, affinityBody)

		// 记录原始请求信息（仅在入口处记录一次）
		common.LogOriginalRequest(c, bodyBytes, envCfg, "Responses")

		// 检查是否为多渠道模式
		isMultiChannel := channelScheduler.IsMultiChannelMode(scheduler.ChannelKindResponses)

		if isMultiChannel {
			handleMultiChannel(c, envCfg, cfgManager, channelScheduler, sessionManager, bodyBytes, responsesReq, userID, startTime)
		} else {
			handleSingleChannel(c, envCfg, cfgManager, channelScheduler, sessionManager, bodyBytes, responsesReq, startTime)
		}
	})
}

// handleMultiChannel 处理多渠道 Responses 请求
func extractRawToolsFromRequestBody(bodyBytes []byte) []interface{} {
	var reqMap map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &reqMap); err != nil {
		return nil
	}
	rawTools, _ := reqMap["tools"].([]interface{})
	return rawTools
}

func handleMultiChannel(
	c *gin.Context,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	sessionManager *session.SessionManager,
	bodyBytes []byte,
	responsesReq types.ResponsesRequest,
	userID string,
	startTime time.Time,
) {
	provider := &providers.ResponsesProvider{SessionManager: sessionManager}
	metricsManager := channelScheduler.GetResponsesMetricsManager()

	common.HandleMultiChannelFailover(
		c,
		envCfg,
		channelScheduler,
		scheduler.ChannelKindResponses,
		"Responses",
		userID,
		responsesReq.Model,
		func(selection *scheduler.SelectionResult) common.MultiChannelAttemptResult {
			upstream := selection.Upstream
			channelIndex := selection.ChannelIndex

			if upstream == nil {
				return common.MultiChannelAttemptResult{}
			}

			baseURLs := upstream.GetAllBaseURLs()
			sortedURLResults := channelScheduler.GetSortedURLsForChannel(scheduler.ChannelKindResponses, channelIndex, baseURLs)

			handled, successKey, successBaseURLIdx, failoverErr, usage, lastErr := common.TryUpstreamWithAllKeys(
				c,
				envCfg,
				cfgManager,
				channelScheduler,
				scheduler.ChannelKindResponses,
				"Responses",
				metricsManager,
				upstream,
				sortedURLResults,
				bodyBytes,
				responsesReq.Stream,
				func(upstream *config.UpstreamConfig, failedKeys map[string]bool) (string, error) {
					return cfgManager.GetNextResponsesAPIKey(upstream, failedKeys)
				},
				func(c *gin.Context, upstreamCopy *config.UpstreamConfig, apiKey string) (*http.Request, error) {
					req, _, err := provider.ConvertToProviderRequest(c, upstreamCopy, apiKey)
					return req, err
				},
				func(apiKey string) {
					_ = cfgManager.DeprioritizeAPIKey(apiKey)
				},
				func(url string) {
					channelScheduler.MarkURLFailure(scheduler.ChannelKindResponses, channelIndex, url)
				},
				func(url string) {
					channelScheduler.MarkURLSuccess(scheduler.ChannelKindResponses, channelIndex, url)
				},
				func(c *gin.Context, resp *http.Response, upstreamCopy *config.UpstreamConfig, apiKey string, actualRequestBody []byte) (*types.Usage, error) {
					// Inject codex_tool_compat_enabled for response remapping
					if responsesReq.TransformerMetadata == nil {
						responsesReq.TransformerMetadata = make(map[string]interface{})
					}
					responsesReq.TransformerMetadata["codex_tool_compat_enabled"] = upstreamCopy.IsCodexToolCompatEnabled() || upstreamCopy.CodexNativeToolPassthrough
					return handleSuccess(c, resp, provider, upstream.ServiceType, envCfg, sessionManager, startTime, &responsesReq, actualRequestBody, cfgManager.GetFuzzyModeEnabled())
				},
				responsesReq.Model,
				"",
				selection.ChannelIndex,
				channelScheduler.GetChannelLogStore(scheduler.ChannelKindResponses),
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
			common.HandleAllChannelsFailed(ctx, cfgManager.GetFuzzyModeEnabled(), failoverErr, lastError, "Responses")
		},
	)
}

// handleSingleChannel 处理单渠道 Responses 请求
func handleSingleChannel(
	c *gin.Context,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	sessionManager *session.SessionManager,
	bodyBytes []byte,
	responsesReq types.ResponsesRequest,
	startTime time.Time,
) {
	upstream, channelIndex, err := cfgManager.GetCurrentResponsesUpstreamWithIndex()
	if err != nil {
		c.JSON(503, gin.H{
			"error": "未配置任何 Responses 渠道，请先在管理界面添加渠道",
			"code":  "NO_RESPONSES_UPSTREAM",
		})
		return
	}

	if len(upstream.APIKeys) == 0 {
		c.JSON(503, gin.H{
			"error": fmt.Sprintf("当前 Responses 渠道 \"%s\" 未配置API密钥", upstream.Name),
			"code":  "NO_API_KEYS",
		})
		return
	}

	provider := &providers.ResponsesProvider{SessionManager: sessionManager}

	metricsManager := channelScheduler.GetResponsesMetricsManager()
	baseURLs := upstream.GetAllBaseURLs()

	urlResults := common.BuildDefaultURLResults(baseURLs)

	handled, _, _, lastFailoverError, _, lastError := common.TryUpstreamWithAllKeys(
		c,
		envCfg,
		cfgManager,
		channelScheduler,
		scheduler.ChannelKindResponses,
		"Responses",
		metricsManager,
		upstream,
		urlResults,
		bodyBytes,
		responsesReq.Stream,
		func(upstream *config.UpstreamConfig, failedKeys map[string]bool) (string, error) {
			return cfgManager.GetNextResponsesAPIKey(upstream, failedKeys)
		},
		func(c *gin.Context, upstreamCopy *config.UpstreamConfig, apiKey string) (*http.Request, error) {
			req, _, err := provider.ConvertToProviderRequest(c, upstreamCopy, apiKey)
			return req, err
		},
		func(apiKey string) {
			if err := cfgManager.DeprioritizeAPIKey(apiKey); err != nil {
				log.Printf("[Responses-Key] 警告: 密钥降级失败: %v", err)
			}
		},
		nil,
		nil,
		func(c *gin.Context, resp *http.Response, upstreamCopy *config.UpstreamConfig, apiKey string, actualRequestBody []byte) (*types.Usage, error) {
			// Inject codex_tool_compat_enabled for response remapping
			if responsesReq.TransformerMetadata == nil {
				responsesReq.TransformerMetadata = make(map[string]interface{})
			}
			responsesReq.TransformerMetadata["codex_tool_compat_enabled"] = upstreamCopy.IsCodexToolCompatEnabled() || upstreamCopy.CodexNativeToolPassthrough
			return handleSuccess(c, resp, provider, upstream.ServiceType, envCfg, sessionManager, startTime, &responsesReq, actualRequestBody, cfgManager.GetFuzzyModeEnabled())
		},
		responsesReq.Model,
		"",
		channelIndex,
		channelScheduler.GetChannelLogStore(scheduler.ChannelKindResponses),
	)
	if handled {
		return
	}

	log.Printf("[Responses-Error] 所有 Responses API密钥都失败了")
	common.HandleAllKeysFailed(c, cfgManager.GetFuzzyModeEnabled(), lastFailoverError, lastError, "Responses")
}

// handleSuccess 处理成功的 Responses 响应
func handleSuccess(
	c *gin.Context,
	resp *http.Response,
	provider *providers.ResponsesProvider,
	upstreamType string,
	envCfg *config.EnvConfig,
	sessionManager *session.SessionManager,
	startTime time.Time,
	originalReq *types.ResponsesRequest,
	originalRequestJSON []byte,
	fuzzyMode bool,
) (*types.Usage, error) {
	defer resp.Body.Close()

	isStream := originalReq != nil && originalReq.Stream

	// Inject codex_tool_compat_enabled into raw JSON so converters can read it.
	// TransformerMetadata is json:"-" so it does not survive serialization.
	if originalReq != nil && originalReq.TransformerMetadata != nil {
		if enabled, ok := originalReq.TransformerMetadata["codex_tool_compat_enabled"].(bool); ok {
			if injected, err := sjson.SetBytes(originalRequestJSON, "transformer_metadata.codex_tool_compat_enabled", enabled); err == nil {
				originalRequestJSON = injected
			}
		}
	}

	if isStream {
		return handleStreamSuccess(c, resp, upstreamType, envCfg, startTime, originalReq, originalRequestJSON)
	}

	// 非流式响应处理
	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		c.JSON(500, gin.H{"error": "Failed to read response"})
		return nil, err
	}

	if envCfg.EnableResponseLogs {
		responseTime := time.Since(startTime).Milliseconds()
		log.Printf("[Responses-Timing] Responses 响应完成: %dms, 状态: %d", responseTime, resp.StatusCode)
		common.LogUpstreamResponse(resp, bodyBytes, envCfg, "Responses")
	}

	providerResp := &types.ProviderResponse{
		StatusCode: resp.StatusCode,
		Headers:    resp.Header,
		Body:       bodyBytes,
		Stream:     false,
	}

	responsesResp, err := provider.ConvertToResponsesResponse(providerResp, upstreamType, "")
	if err != nil {
		// JSON 解析失败（如上游返回 HTML 错误页面）：不写 Header，返回可 failover 的错误
		preview := bodyBytes
		if len(preview) > 100 {
			preview = preview[:100]
		}
		log.Printf("[Responses-InvalidBody] 响应体解析失败: %v, body前100字节: %s", err, preview)
		return nil, fmt.Errorf("%w: %v", common.ErrInvalidResponseBody, err)
	}

	// 空响应拦截（仅 Fuzzy 模式）：上游 200 但 output 语义为空，
	// Header 未发送，可安全 failover 到下一个 Key/BaseURL/渠道
	if fuzzyMode && common.IsResponsesResponseEmpty(responsesResp) {
		log.Printf("[Responses-EmptyResponse] 上游返回空响应（非流式，upstreamType=%s），触发 failover", upstreamType)
		return nil, common.ErrEmptyNonStreamResponse
	}

	// Remap Codex custom tool proxy function calls to custom_tool_call items.
	if originalReq != nil {
		codexEnabled := false
		if originalReq.TransformerMetadata != nil {
			if v, ok := originalReq.TransformerMetadata["codex_tool_compat_enabled"].(bool); ok {
				codexEnabled = v
			}
		}
		if codexEnabled {
			codexCtx := converters.BuildCodexToolContext(originalReq.Tools)
			if len(originalReq.RawTools) > 0 {
				codexCtx = converters.BuildCodexToolContextFromRaw(originalReq.RawTools)
			}
			codexCtx.RemapCustomToolCallsInResponse(responsesResp)
			codexCtx.RemapNamespaceFunctionCallsInResponse(responsesResp)
		}
	}

	// Token 补全逻辑
	originalUsage := responsesResp.Usage

	patchResponsesUsage(responsesResp, originalRequestJSON, envCfg)

	// 更新会话
	if originalReq.Store == nil || *originalReq.Store {
		sess, err := sessionManager.GetOrCreateSession(originalReq.PreviousResponseID)
		if err == nil {
			inputItems, _ := parseInputToItems(originalReq.Input)
			for _, item := range inputItems {
				sessionManager.AppendMessage(sess.ID, item, 0)
			}

			for _, item := range responsesResp.Output {
				sessionManager.AppendMessage(sess.ID, item, responsesResp.Usage.TotalTokens)
			}

			previousResponseID := sess.LastResponseID
			sessionManager.UpdateLastResponseID(sess.ID, responsesResp.ID)
			sessionManager.RecordResponseMapping(responsesResp.ID, sess.ID)

			if previousResponseID != "" {
				responsesResp.PreviousID = previousResponseID
			}
		}
	}

	utils.ForwardResponseHeaders(resp.Header, c.Writer)
	c.JSON(200, responsesResp)

	// 返回 usage 数据用于指标记录
	promptTokensTotal := promptTokensTotalFromResponsesInput(
		originalUsage.InputTokens,
		upstreamType,
		responsesUsageHasClaudeCache(originalUsage),
	)
	return metricsUsageFromResponsesUsage(responsesResp.Usage, promptTokensTotal), nil
}

func responsesUsageHasClaudeCache(usage types.ResponsesUsage) bool {
	return usage.CacheCreationInputTokens > 0 ||
		usage.CacheReadInputTokens > 0 ||
		usage.CacheCreation5mInputTokens > 0 ||
		usage.CacheCreation1hInputTokens > 0
}

func promptTokensTotalFromResponsesInput(inputTokens int, upstreamType string, hasClaudeCache bool) int {
	if upstreamType != "responses" || inputTokens <= 0 {
		return 0
	}
	if inputTokens <= 1 && !hasClaudeCache {
		return 0
	}
	return inputTokens
}

func metricsUsageFromResponsesUsage(usage types.ResponsesUsage, promptTokensTotal int) *types.Usage {
	cacheReadTokens := usage.CacheReadInputTokens
	if cacheReadTokens == 0 && usage.InputTokensDetails != nil && usage.InputTokensDetails.CachedTokens > 0 {
		cacheReadTokens = usage.InputTokensDetails.CachedTokens
	}

	return &types.Usage{
		InputTokens:                usage.InputTokens,
		OutputTokens:               usage.OutputTokens,
		CacheCreationInputTokens:   usage.CacheCreationInputTokens,
		CacheReadInputTokens:       cacheReadTokens,
		PromptTokensTotal:          promptTokensTotal,
		CacheCreation5mInputTokens: usage.CacheCreation5mInputTokens,
		CacheCreation1hInputTokens: usage.CacheCreation1hInputTokens,
		CacheTTL:                   usage.CacheTTL,
	}
}

// patchResponsesUsage 补全 Responses 响应的 Token 统计
func patchResponsesUsage(resp *types.ResponsesResponse, requestBody []byte, envCfg *config.EnvConfig) {
	// 检查是否有 Claude 原生缓存 token（有时才跳过 input_tokens 修补）
	// 仅检测 Claude 原生字段：cache_creation_input_tokens, cache_read_input_tokens,
	// cache_creation_5m_input_tokens, cache_creation_1h_input_tokens
	// 注意：不检测 input_tokens_details.cached_tokens（OpenAI 格式），避免错误跳过
	hasClaudeCache := resp.Usage.CacheCreationInputTokens > 0 ||
		resp.Usage.CacheReadInputTokens > 0 ||
		resp.Usage.CacheCreation5mInputTokens > 0 ||
		resp.Usage.CacheCreation1hInputTokens > 0

	// 检查是否需要补全
	needInputPatch := resp.Usage.InputTokens <= 1 && !hasClaudeCache
	needOutputPatch := resp.Usage.OutputTokens <= 1

	// 如果 usage 完全为空，进行完整估算
	if resp.Usage.InputTokens == 0 && resp.Usage.OutputTokens == 0 && resp.Usage.TotalTokens == 0 {
		estimatedInput := utils.EstimateResponsesRequestTokens(requestBody)
		estimatedOutput := estimateResponsesOutputFromItems(resp.Output)
		resp.Usage.InputTokens = estimatedInput
		resp.Usage.OutputTokens = estimatedOutput
		resp.Usage.TotalTokens = calculateTotalTokensWithCache(
			estimatedInput,
			estimatedOutput,
			resp.Usage.CacheReadInputTokens,
			resp.Usage.CacheCreationInputTokens,
			resp.Usage.CacheCreation5mInputTokens,
			resp.Usage.CacheCreation1hInputTokens,
		)
		if envCfg.EnableResponseLogs {
			log.Printf("[Responses-Token] 上游无Usage, 本地估算: input=%d, output=%d", estimatedInput, estimatedOutput)
		}
		return
	}

	// 修补虚假值
	originalInput := resp.Usage.InputTokens
	originalOutput := resp.Usage.OutputTokens
	patched := false

	if needInputPatch {
		resp.Usage.InputTokens = utils.EstimateResponsesRequestTokens(requestBody)
		patched = true
	}
	if needOutputPatch {
		resp.Usage.OutputTokens = estimateResponsesOutputFromItems(resp.Output)
		patched = true
	}

	// 重新计算 TotalTokens（修补时或 total_tokens 为 0 但 input/output 有效时）
	if patched || (resp.Usage.TotalTokens == 0 && (resp.Usage.InputTokens > 0 || resp.Usage.OutputTokens > 0)) {
		resp.Usage.TotalTokens = calculateTotalTokensWithCache(
			resp.Usage.InputTokens,
			resp.Usage.OutputTokens,
			resp.Usage.CacheReadInputTokens,
			resp.Usage.CacheCreationInputTokens,
			resp.Usage.CacheCreation5mInputTokens,
			resp.Usage.CacheCreation1hInputTokens,
		)
	}

	if envCfg.EnableResponseLogs {
		if patched {
			log.Printf("[Responses-Token] 虚假值修补: InputTokens=%d->%d, OutputTokens=%d->%d",
				originalInput, resp.Usage.InputTokens, originalOutput, resp.Usage.OutputTokens)
		}
		log.Printf("[Responses-Token] InputTokens=%d, OutputTokens=%d, TotalTokens=%d, CacheCreation=%d, CacheRead=%d, CacheCreation5m=%d, CacheCreation1h=%d, CacheTTL=%s",
			resp.Usage.InputTokens, resp.Usage.OutputTokens, resp.Usage.TotalTokens,
			resp.Usage.CacheCreationInputTokens, resp.Usage.CacheReadInputTokens,
			resp.Usage.CacheCreation5mInputTokens, resp.Usage.CacheCreation1hInputTokens,
			resp.Usage.CacheTTL)
	}
}

// estimateResponsesOutputFromItems 从 ResponsesItem 数组估算输出 token
func estimateResponsesOutputFromItems(output []types.ResponsesItem) int {
	if len(output) == 0 {
		return 0
	}

	total := 0
	for _, item := range output {
		// 处理 content
		if item.Content != nil {
			switch v := item.Content.(type) {
			case string:
				total += utils.EstimateTokens(v)
			case []interface{}:
				for _, block := range v {
					if b, ok := block.(map[string]interface{}); ok {
						if text, ok := b["text"].(string); ok {
							total += utils.EstimateTokens(text)
						}
					}
				}
			case []types.ContentBlock:
				// 处理结构化 ContentBlock 数组
				for _, block := range v {
					if block.Text != "" {
						total += utils.EstimateTokens(block.Text)
					}
				}
			default:
				// 回退：序列化后估算
				data, _ := json.Marshal(v)
				total += utils.EstimateTokens(string(data))
			}
		}

		// 处理 tool_use
		if item.ToolUse != nil {
			if item.ToolUse.Name != "" {
				total += utils.EstimateTokens(item.ToolUse.Name) + 2
			}
			if item.ToolUse.Input != nil {
				data, _ := json.Marshal(item.ToolUse.Input)
				total += utils.EstimateTokens(string(data))
			}
		}

		// 处理 function_call 类型（item.Type == "function_call"）
		if item.Type == "function_call" {
			// 在转换后的响应中，function_call 的参数可能在 Content 中
			if contentStr, ok := item.Content.(string); ok {
				total += utils.EstimateTokens(contentStr)
			}
		}
	}

	return total
}

// handleStreamSuccess 处理流式响应
//
// 流程：预读取行 → 检测空响应
//   - 空响应 → return nil, ErrEmptyStreamResponse（Header 未发送，可安全重试）
//   - 非空   → 发送 Header → 回放缓冲行 → 继续读取
func handleStreamSuccess(
	c *gin.Context,
	resp *http.Response,
	upstreamType string,
	envCfg *config.EnvConfig,
	startTime time.Time,
	originalReq *types.ResponsesRequest,
	originalRequestJSON []byte,
) (*types.Usage, error) {
	if envCfg.EnableResponseLogs {
		responseTime := time.Since(startTime).Milliseconds()
		log.Printf("[Responses-Stream] Responses 流式响应开始: %dms, 状态: %d", responseTime, resp.StatusCode)
		common.LogUpstreamResponseHeaders(resp, envCfg, "Responses")
	}

	var synthesizer *utils.StreamSynthesizer
	logBuffer := common.NewLimitedLogBuffer(common.MaxUpstreamResponseLogBytes)
	streamLoggingEnabled := envCfg.IsDevelopment() && envCfg.EnableResponseLogs

	if streamLoggingEnabled {
		synthesizer = utils.NewStreamSynthesizer(upstreamType)
	}

	needConvert := upstreamType != "responses"
	var converterState any

	scanner := bufio.NewScanner(resp.Body)
	const maxCapacity = 1024 * 1024
	buf := make([]byte, 0, 64*1024)
	scanner.Buffer(buf, maxCapacity)

	// 预检测：在发送 HTTP Header 之前缓冲行并检查是否为空响应
	// 使用 goroutine + channel 实现真正的超时控制（scanner.Scan 是阻塞调用）
	type scanLine struct {
		text string
		ok   bool
	}
	lineChan := make(chan scanLine, 1)
	scanDone := make(chan struct{})
	go func() {
		defer close(lineChan)
		for scanner.Scan() {
			select {
			case lineChan <- scanLine{text: normalizeResponsesSSEFieldLine(scanner.Text()), ok: true}:
			case <-scanDone:
				return
			}
		}
		select {
		case lineChan <- scanLine{ok: false}: // scanner 结束
		case <-scanDone:
		}
	}()

	var bufferedLines []string
	var preflightTextBuf bytes.Buffer
	preflightHasNonTextContent := false
	preflightEmpty := false
	preflightDiagnostic := ""
	preflightTimeout := time.NewTimer(30 * time.Second)
	preflightDone := false
	var blacklistReason, blacklistMessage string
	seenConvertedEvent := false
	seenCompletedEvent := false
	seenUsageOnlyEvent := false
	seenUnknownEvent := false
	unknownEventType := ""

	for !preflightDone {
		select {
		case sl := <-lineChan:
			if !sl.ok {
				// scanner 结束
				if preflightHasNonTextContent {
					preflightEmpty = false
				} else {
					preflightEmpty = common.IsEffectivelyEmptyStreamText(preflightTextBuf.String())
				}
				preflightDiagnostic = buildResponsesPreflightDiagnostic(seenConvertedEvent, seenCompletedEvent, seenUsageOnlyEvent, seenUnknownEvent, unknownEventType, preflightTextBuf.String())
				preflightDone = true
				break
			}
			line := sl.text
			bufferedLines = append(bufferedLines, line)

			// 检测 SSE error 事件中的拉黑条件
			if blacklistReason == "" {
				if r, m := common.DetectStreamBlacklistError(line + "\n"); r != "" {
					blacklistReason = r
					blacklistMessage = m
				}
			}

			// 处理转换后的事件用于文本提取
			var eventsToCheck []string
			if needConvert {
				switch upstreamType {
				case "claude":
					eventsToCheck = converters.ConvertClaudeMessagesToResponses(
						c.Request.Context(),
						originalReq.Model,
						originalRequestJSON,
						nil,
						[]byte(line),
						&converterState,
					)
				case "gemini":
					eventsToCheck = converters.ConvertGeminiStreamToResponses(
						c.Request.Context(),
						originalReq.Model,
						originalRequestJSON,
						nil,
						[]byte(line),
						&converterState,
					)
				default:
					eventsToCheck = converters.ConvertOpenAIChatToResponses(
						c.Request.Context(),
						originalReq.Model,
						originalRequestJSON,
						nil,
						[]byte(line),
						&converterState,
					)
				}
			} else {
				eventsToCheck = []string{line + "\n"}
			}

			for _, event := range eventsToCheck {
				seenConvertedEvent = true
				seenCompletedEvent = seenCompletedEvent || isResponsesCompletedEvent(event)
				seenUsageOnlyEvent = seenUsageOnlyEvent || isResponsesUsageOnlyEvent(event)
				if t, ok := firstUnknownResponsesEventType(event); ok {
					seenUnknownEvent = true
					if unknownEventType == "" {
						unknownEventType = t
					}
				}

				if !preflightHasNonTextContent && common.HasResponsesSemanticContent(event) {
					preflightHasNonTextContent = true
					preflightEmpty = false
					preflightDone = true
					break
				}

				extractResponsesTextFromEvent(event, &preflightTextBuf)

				// 检查是否有有效内容 delta 事件
				if !common.IsEffectivelyEmptyStreamText(preflightTextBuf.String()) {
					preflightDone = true
					break
				}

				// 检查是否为 response.completed 事件（流正常结束）
				if isResponsesCompletedEvent(event) {
					preflightDone = true
					// 检查是否有实际内容（文本或工具调用）
					preflightEmpty = !preflightHasNonTextContent && common.IsEffectivelyEmptyStreamText(preflightTextBuf.String())
					// 如果有工具调用，不算空响应
					if preflightEmpty && hasResponsesFunctionCall(event) {
						preflightEmpty = false
					}
					preflightDiagnostic = buildResponsesPreflightDiagnostic(seenConvertedEvent, true, seenUsageOnlyEvent, seenUnknownEvent, unknownEventType, preflightTextBuf.String())
					break
				}
			}

		case <-preflightTimeout.C:
			preflightDone = true // 超时保守放行
		}
	}
	preflightTimeout.Stop()

	// 空响应：Header 未发送，可安全重试
	if preflightEmpty {
		log.Printf("[Responses-EmptyResponse] 上游返回空响应 (缓冲行数: %d, 诊断: %s)，触发重试", len(bufferedLines), preflightDiagnostic)
		close(scanDone) // 通知 scanner goroutine 退出
		if blacklistReason != "" {
			return nil, &common.ErrBlacklistKey{Reason: blacklistReason, Message: blacklistMessage}
		}
		return nil, common.ErrEmptyStreamResponse
	}

	// 流中有拉黑错误但内容非空：仍返回拉黑错误以触发 Key 拉黑
	if blacklistReason != "" {
		close(scanDone)
		return nil, &common.ErrBlacklistKey{Reason: blacklistReason, Message: blacklistMessage}
	}

	// 非空响应：发送 Header 并回放缓冲行
	// 重置 converterState 以便回放时重新转换
	converterState = nil

	utils.ForwardResponseHeaders(resp.Header, c.Writer)
	c.Header("Content-Type", "text/event-stream")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	c.Header("X-Accel-Buffering", "no")

	c.Status(resp.StatusCode)
	flusher, _ := c.Writer.(http.Flusher)

	// Token 统计状态
	var outputTextBuffer bytes.Buffer
	const maxOutputBufferSize = 1024 * 1024 // 1MB 上限，防止内存溢出
	var collectedUsage responsesStreamUsage
	hasUsage := false
	needTokenPatch := false
	clientGone := false
	promptTokensTotal := 0
	completedEventSent := false
	eventsSentCount := 0

	// processLine 处理单行数据（复用于缓冲行回放和后续读取）
	processLine := func(line string) {

		if streamLoggingEnabled {
			logBuffer.WriteString(line + "\n")
			if synthesizer != nil {
				synthesizer.ProcessLine(line)
			}
		}

		// 处理转换后的事件
		var eventsToProcess []string

		if needConvert {
			var events []string
			switch upstreamType {
			case "claude":
				events = converters.ConvertClaudeMessagesToResponses(
					c.Request.Context(),
					originalReq.Model,
					originalRequestJSON,
					nil,
					[]byte(line),
					&converterState,
				)
			case "gemini":
				events = converters.ConvertGeminiStreamToResponses(
					c.Request.Context(),
					originalReq.Model,
					originalRequestJSON,
					nil,
					[]byte(line),
					&converterState,
				)
			default:
				events = converters.ConvertOpenAIChatToResponses(
					c.Request.Context(),
					originalReq.Model,
					originalRequestJSON,
					nil,
					[]byte(line),
					&converterState,
				)
			}
			eventsToProcess = events
		} else {
			eventsToProcess = []string{line + "\n"}
		}

		for _, event := range eventsToProcess {
			// 提取文本内容用于估算（限制缓冲区大小）
			if outputTextBuffer.Len() < maxOutputBufferSize {
				extractResponsesTextFromEvent(event, &outputTextBuffer)
			}

			// 检测并收集 usage
			detected, needPatch, usageData := checkResponsesEventUsage(event, envCfg.EnableResponseLogs && envCfg.ShouldLog("debug"))
			if detected {
				if !hasUsage {
					hasUsage = true
					needTokenPatch = needPatch
					if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") && needPatch {
						log.Printf("[Responses-Stream-Token] 检测到虚假值, 延迟到流结束修补")
					}
				}
				updateResponsesStreamUsage(&collectedUsage, usageData)
				if !needConvert {
					candidatePromptTokensTotal := promptTokensTotalFromResponsesInput(
						usageData.InputTokens,
						upstreamType,
						usageData.HasClaudeCache,
					)
					if candidatePromptTokensTotal > promptTokensTotal {
						promptTokensTotal = candidatePromptTokensTotal
					}
				}
			}

			// 在 response.completed 事件前注入/修补 usage
			eventToSend := event
			if isResponsesCompletedEvent(event) {
				completedEventSent = true
				if !hasUsage {
					// 上游完全没有 usage，注入本地估算
					var injectedInput, injectedOutput int
					eventToSend, injectedInput, injectedOutput = injectResponsesUsageToCompletedEvent(event, originalRequestJSON, outputTextBuffer.String(), envCfg)
					// 更新 collectedUsage 以便最终日志输出
					collectedUsage.InputTokens = injectedInput
					collectedUsage.OutputTokens = injectedOutput
					collectedUsage.TotalTokens = calculateTotalTokensWithCache(
						injectedInput,
						injectedOutput,
						collectedUsage.CacheReadInputTokens,
						collectedUsage.CacheCreationInputTokens,
						collectedUsage.CacheCreation5mInputTokens,
						collectedUsage.CacheCreation1hInputTokens,
					)
					if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
						log.Printf("[Responses-Stream-Token] 上游无usage, 注入本地估算: input=%d, output=%d", injectedInput, injectedOutput)
					}
				} else if needTokenPatch {
					// 需要修补虚假值
					eventToSend = patchResponsesCompletedEventUsage(event, originalRequestJSON, outputTextBuffer.String(), &collectedUsage, envCfg)
				}
			}

			// 转发给客户端
			if !clientGone {
				_, err := c.Writer.Write([]byte(eventToSend))
				if err != nil {
					clientGone = true
					if !isClientDisconnectError(err) {
						log.Printf("[Responses-Stream] 警告: 流式响应传输错误: %v", err)
					} else if envCfg.ShouldLog("info") {
						log.Printf("[Responses-Stream] 客户端中断连接 (正常行为)，继续接收上游数据...")
					}
				} else {
					eventsSentCount++
					if flusher != nil {
						flusher.Flush()
					}
				}
			}
		}
	}

	// 回放预检测期间缓冲的行
	for _, bufferedLine := range bufferedLines {
		processLine(bufferedLine)
	}

	// 继续从 lineChan 读取剩余的流数据
	for sl := range lineChan {
		if !sl.ok {
			break
		}
		processLine(sl.text)
	}

	// 兜底：如果上游未发送终止符（如 MiniMax 不发 [DONE]），补发 response.completed
	if !completedEventSent && !clientGone {
		log.Printf("[Responses-Stream] 上游未发送终止符，补发 response.completed (upstreamType=%s)", upstreamType)

		var fallbackEvents []string
		if needConvert {
			switch upstreamType {
			case "claude", "gemini":
				fallbackEvents = converters.SynthesizeResponsesCompleted(originalRequestJSON, &converterState, upstreamType, eventsSentCount)
			default:
				// OpenAI 格式（包括 MiniMax）：发送合成 [DONE] 触发 converter 正常完成流程
				fallbackEvents = converters.ConvertOpenAIChatToResponses(
					c.Request.Context(),
					originalReq.Model,
					originalRequestJSON,
					nil,
					[]byte("data: [DONE]"),
					&converterState,
				)
			}
		} else {
			fallbackEvents = converters.SynthesizeResponsesCompleted(originalRequestJSON, &converterState, "responses", eventsSentCount)
		}

		for _, event := range fallbackEvents {
			eventToSend := event
			if isResponsesCompletedEvent(event) {
				completedEventSent = true
				if !hasUsage {
					var injectedInput, injectedOutput int
					eventToSend, injectedInput, injectedOutput = injectResponsesUsageToCompletedEvent(event, originalRequestJSON, outputTextBuffer.String(), envCfg)
					collectedUsage.InputTokens = injectedInput
					collectedUsage.OutputTokens = injectedOutput
					collectedUsage.TotalTokens = calculateTotalTokensWithCache(
						injectedInput,
						injectedOutput,
						collectedUsage.CacheReadInputTokens,
						collectedUsage.CacheCreationInputTokens,
						collectedUsage.CacheCreation5mInputTokens,
						collectedUsage.CacheCreation1hInputTokens,
					)
				} else if needTokenPatch {
					eventToSend = patchResponsesCompletedEventUsage(event, originalRequestJSON, outputTextBuffer.String(), &collectedUsage, envCfg)
				}
			}
			if _, err := c.Writer.Write([]byte(eventToSend)); err == nil && flusher != nil {
				flusher.Flush()
			}
		}
	}

	if err := scanner.Err(); err != nil {
		if !isClientDisconnectError(err) {
			log.Printf("[Responses-Stream] 警告: 流式响应读取错误: %v", err)
		} else if envCfg.ShouldLog("info") {
			log.Printf("[Responses-Stream] 上游读取因客户端取消而结束")
		}
	}

	if envCfg.EnableResponseLogs {
		responseTime := time.Since(startTime).Milliseconds()
		log.Printf("[Responses-Stream] Responses 流式响应完成: %dms", responseTime)

		// 输出 Token 统计
		if hasUsage || collectedUsage.InputTokens > 0 || collectedUsage.OutputTokens > 0 {
			log.Printf("[Responses-Stream-Token] InputTokens=%d, OutputTokens=%d, CacheCreation=%d, CacheRead=%d, CacheCreation5m=%d, CacheCreation1h=%d, CacheTTL=%s",
				collectedUsage.InputTokens, collectedUsage.OutputTokens,
				collectedUsage.CacheCreationInputTokens, collectedUsage.CacheReadInputTokens,
				collectedUsage.CacheCreation5mInputTokens, collectedUsage.CacheCreation1hInputTokens,
				collectedUsage.CacheTTL)
		}

		if envCfg.IsDevelopment() {
			if synthesizer != nil {
				synthesizedContent := synthesizer.GetSynthesizedContent()
				parseFailed := synthesizer.IsParseFailed()
				if synthesizedContent != "" && !parseFailed {
					log.Printf("[Responses-Stream] 上游流式响应合成内容:\n%s", strings.TrimSpace(synthesizedContent))
				} else if logBuffer.Len() > 0 {
					log.Printf("[Responses-Stream] 上游流式响应原始内容:\n%s", logBuffer.String())
				}
			} else if logBuffer.Len() > 0 {
				log.Printf("[Responses-Stream] 上游流式响应原始内容:\n%s", logBuffer.String())
			}
		}
	}

	// 返回收集到的 usage 数据
	return metricsUsageFromResponsesUsage(types.ResponsesUsage{
		InputTokens:                collectedUsage.InputTokens,
		OutputTokens:               collectedUsage.OutputTokens,
		CacheCreationInputTokens:   collectedUsage.CacheCreationInputTokens,
		CacheReadInputTokens:       collectedUsage.CacheReadInputTokens,
		CacheCreation5mInputTokens: collectedUsage.CacheCreation5mInputTokens,
		CacheCreation1hInputTokens: collectedUsage.CacheCreation1hInputTokens,
		CacheTTL:                   collectedUsage.CacheTTL,
	}, promptTokensTotal), nil
}

// responsesStreamUsage 流式响应 usage 收集结构
type responsesStreamUsage struct {
	InputTokens                int
	OutputTokens               int
	TotalTokens                int // 用于检测 total_tokens 是否需要补全
	CacheCreationInputTokens   int
	CacheReadInputTokens       int
	CacheCreation5mInputTokens int
	CacheCreation1hInputTokens int
	CacheTTL                   string
	HasClaudeCache             bool // 是否检测到 Claude 原生缓存字段（区别于 OpenAI cached_tokens）
}

func normalizeResponsesSSEFieldLine(line string) string {
	for _, prefix := range []string{"data:", "event:", "id:", "retry:"} {
		if strings.HasPrefix(line, prefix) && !strings.HasPrefix(line, prefix+" ") {
			return prefix + " " + line[len(prefix):]
		}
	}
	return line
}

// extractResponsesTextFromEvent 从 Responses SSE 事件中提取文本内容
func extractResponsesTextFromEvent(event string, buf *bytes.Buffer) {
	for _, line := range strings.Split(event, "\n") {
		// 支持 "data:" 和 "data: " 两种格式（有些上游不带空格）
		var jsonStr string
		if strings.HasPrefix(line, "data:") {
			jsonStr = strings.TrimPrefix(line, "data:")
			jsonStr = strings.TrimPrefix(jsonStr, " ") // 移除可能的前导空格
		} else {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		eventType, _ := data["type"].(string)

		// 处理各种 delta 类型
		switch eventType {
		case "response.output_text.delta":
			if delta, ok := data["delta"].(string); ok {
				buf.WriteString(delta)
			}
		case "response.function_call_arguments.delta":
			if delta, ok := data["delta"].(string); ok {
				buf.WriteString(delta)
			}
		case "response.reasoning_summary_text.delta":
			if text, ok := data["text"].(string); ok {
				buf.WriteString(text)
			}
		case "response.output_json.delta":
			// JSON 输出增量
			if delta, ok := data["delta"].(string); ok {
				buf.WriteString(delta)
			}
		case "response.content_part.delta":
			// 内容块增量（通用）
			if delta, ok := data["delta"].(string); ok {
				buf.WriteString(delta)
			} else if text, ok := data["text"].(string); ok {
				buf.WriteString(text)
			}
		case "response.audio.delta", "response.audio_transcript.delta":
			// 音频转录增量
			if delta, ok := data["delta"].(string); ok {
				buf.WriteString(delta)
			}
		}
	}
}

// checkResponsesEventUsage 检测 Responses 事件是否包含 usage
func checkResponsesEventUsage(event string, enableLog bool) (bool, bool, responsesStreamUsage) {
	lines := strings.Split(event, "\n")
	for _, line := range lines {
		// 支持 "data:" 和 "data: " 两种格式（有些上游不带空格）
		var jsonStr string
		if strings.HasPrefix(line, "data:") {
			jsonStr = strings.TrimPrefix(line, "data:")
			jsonStr = strings.TrimPrefix(jsonStr, " ") // 移除可能的前导空格
		} else {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		eventType, _ := data["type"].(string)

		// 检查 response.completed 事件中的 usage
		if eventType == "response.completed" {
			if response, ok := data["response"].(map[string]interface{}); ok {
				if usage, ok := response["usage"].(map[string]interface{}); ok {
					usageData := extractResponsesUsageFromMap(usage)
					needPatch := usageData.InputTokens <= 1 || usageData.OutputTokens <= 1

					// 仅当检测到 Claude 原生缓存字段时，才跳过 input_tokens 补全
					// OpenAI 的 input_tokens_details.cached_tokens 不应阻止补全
					if usageData.HasClaudeCache && usageData.InputTokens <= 1 {
						needPatch = usageData.OutputTokens <= 1 // 有 Claude 缓存时只检查 output
					}

					// 检查 total_tokens 是否需要补全（有效 input/output 但 total=0）
					if !needPatch && usageData.TotalTokens == 0 && (usageData.InputTokens > 0 || usageData.OutputTokens > 0) {
						needPatch = true
					}

					if enableLog {
						log.Printf("[Responses-Stream-Token] response.completed: InputTokens=%d, OutputTokens=%d, TotalTokens=%d, CacheCreation=%d, CacheRead=%d, HasClaudeCache=%v, 需补全=%v",
							usageData.InputTokens, usageData.OutputTokens, usageData.TotalTokens, usageData.CacheCreationInputTokens, usageData.CacheReadInputTokens, usageData.HasClaudeCache, needPatch)
					}
					return true, needPatch, usageData
				} else if enableLog {
					log.Printf("[Responses-Stream-Token] response.completed 事件中无 usage 字段")
				}
			} else if enableLog {
				log.Printf("[Responses-Stream-Token] response.completed 事件中无 response 字段")
			}
		}
	}
	return false, false, responsesStreamUsage{}
}

// extractResponsesUsageFromMap 从 usage map 中提取数据
func extractResponsesUsageFromMap(usage map[string]interface{}) responsesStreamUsage {
	var data responsesStreamUsage

	if v, ok := usage["input_tokens"].(float64); ok {
		data.InputTokens = int(v)
	}
	if v, ok := usage["output_tokens"].(float64); ok {
		data.OutputTokens = int(v)
	}
	if v, ok := usage["total_tokens"].(float64); ok {
		data.TotalTokens = int(v)
	}
	if v, ok := usage["cache_creation_input_tokens"].(float64); ok {
		data.CacheCreationInputTokens = int(v)
		if v > 0 {
			data.HasClaudeCache = true
		}
	}
	if v, ok := usage["cache_read_input_tokens"].(float64); ok {
		data.CacheReadInputTokens = int(v)
		if v > 0 {
			data.HasClaudeCache = true
		}
	}
	if v, ok := usage["cache_creation_5m_input_tokens"].(float64); ok {
		data.CacheCreation5mInputTokens = int(v)
		if v > 0 {
			data.HasClaudeCache = true
		}
	}
	if v, ok := usage["cache_creation_1h_input_tokens"].(float64); ok {
		data.CacheCreation1hInputTokens = int(v)
		if v > 0 {
			data.HasClaudeCache = true
		}
	}

	// 检查 input_tokens_details.cached_tokens (OpenAI 格式，不设置 HasClaudeCache)
	if details, ok := usage["input_tokens_details"].(map[string]interface{}); ok {
		if cached, ok := details["cached_tokens"].(float64); ok && cached > 0 {
			// 仅当 CacheReadInputTokens 未被设置时才使用 OpenAI 的 cached_tokens
			if data.CacheReadInputTokens == 0 {
				data.CacheReadInputTokens = int(cached)
			}
			// 注意：不设置 HasClaudeCache，因为这是 OpenAI 格式
		}
	}

	// 设置 CacheTTL
	var has5m, has1h bool
	if data.CacheCreation5mInputTokens > 0 {
		has5m = true
	}
	if data.CacheCreation1hInputTokens > 0 {
		has1h = true
	}
	if has5m && has1h {
		data.CacheTTL = "mixed"
	} else if has1h {
		data.CacheTTL = "1h"
	} else if has5m {
		data.CacheTTL = "5m"
	}

	return data
}

// updateResponsesStreamUsage 更新收集的 usage 数据
func updateResponsesStreamUsage(collected *responsesStreamUsage, usageData responsesStreamUsage) {
	if usageData.InputTokens > collected.InputTokens {
		collected.InputTokens = usageData.InputTokens
	}
	if usageData.OutputTokens > collected.OutputTokens {
		collected.OutputTokens = usageData.OutputTokens
	}
	if usageData.TotalTokens > collected.TotalTokens {
		collected.TotalTokens = usageData.TotalTokens
	}
	if usageData.CacheCreationInputTokens > 0 {
		collected.CacheCreationInputTokens = usageData.CacheCreationInputTokens
	}
	if usageData.CacheReadInputTokens > 0 {
		collected.CacheReadInputTokens = usageData.CacheReadInputTokens
	}
	if usageData.CacheCreation5mInputTokens > 0 {
		collected.CacheCreation5mInputTokens = usageData.CacheCreation5mInputTokens
	}
	if usageData.CacheCreation1hInputTokens > 0 {
		collected.CacheCreation1hInputTokens = usageData.CacheCreation1hInputTokens
	}
	if usageData.CacheTTL != "" {
		collected.CacheTTL = usageData.CacheTTL
	}
	// 传播 HasClaudeCache 标志
	if usageData.HasClaudeCache {
		collected.HasClaudeCache = true
	}
}

// isResponsesCompletedEvent 检测是否为 response.completed 事件
func isResponsesCompletedEvent(event string) bool {
	return strings.Contains(event, `"type":"response.completed"`) ||
		strings.Contains(event, `"type": "response.completed"`)
}

// isClientDisconnectError 判断是否为客户端断开连接错误
func isClientDisconnectError(err error) bool {
	msg := err.Error()
	return strings.Contains(msg, "broken pipe") ||
		strings.Contains(msg, "connection reset") ||
		strings.Contains(msg, "context canceled")
}

func effectiveCacheCreationTokens(cacheCreation, cacheCreation5m, cacheCreation1h int) int {
	if cacheCreation > 0 {
		return cacheCreation
	}
	return cacheCreation5m + cacheCreation1h
}

func calculateTotalTokensWithCache(inputTokens, outputTokens, cacheRead, cacheCreation, cacheCreation5m, cacheCreation1h int) int {
	return inputTokens + outputTokens + cacheRead + effectiveCacheCreationTokens(cacheCreation, cacheCreation5m, cacheCreation1h)
}

// injectResponsesUsageToCompletedEvent 向 response.completed 事件注入 usage
// 返回: 修改后的事件字符串, 估算的 inputTokens, 估算的 outputTokens
func injectResponsesUsageToCompletedEvent(event string, requestBody []byte, outputText string, envCfg *config.EnvConfig) (string, int, int) {
	inputTokens := utils.EstimateResponsesRequestTokens(requestBody)
	outputTokens := utils.EstimateTokens(outputText)
	totalTokens := calculateTotalTokensWithCache(inputTokens, outputTokens, 0, 0, 0, 0)

	// 调试日志：记录估算开始
	if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
		log.Printf("[Responses-Stream-Token] injectUsage 开始: inputTokens=%d, outputTokens=%d, event长度=%d",
			inputTokens, outputTokens, len(event))
	}

	var result strings.Builder
	lines := strings.Split(event, "\n")
	injected := false

	for _, line := range lines {
		// 跳过 event: 行，但保留它
		if strings.HasPrefix(line, "event:") {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		// 支持 "data:" 和 "data: " 两种格式（有些上游不带空格）
		var jsonStr string
		if strings.HasPrefix(line, "data:") {
			jsonStr = strings.TrimPrefix(line, "data:")
			jsonStr = strings.TrimPrefix(jsonStr, " ") // 移除可能的前导空格
		} else {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			// 调试日志：JSON 解析失败
			if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
				log.Printf("[Responses-Stream-Token] JSON解析失败: %v, 内容前200字符: %.200s", err, jsonStr)
			}
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		eventType, _ := data["type"].(string)

		if eventType == "response.completed" {
			response, ok := data["response"].(map[string]interface{})
			if !ok {
				// response 字段缺失或类型错误，创建一个新的
				if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
					log.Printf("[Responses-Stream-Token] response字段缺失, 创建新的response对象")
				}
				response = make(map[string]interface{})
				data["response"] = response
			}

			response["usage"] = map[string]interface{}{
				"input_tokens":  inputTokens,
				"output_tokens": outputTokens,
				"total_tokens":  totalTokens,
			}
			injected = true

			patchedJSON, err := json.Marshal(data)
			if err != nil {
				if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
					log.Printf("[Responses-Stream-Token] JSON序列化失败: %v", err)
				}
				result.WriteString(line)
				result.WriteString("\n")
				continue
			}

			if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
				log.Printf("[Responses-Stream-Token] 注入本地估算成功: InputTokens=%d, OutputTokens=%d, TotalTokens=%d",
					inputTokens, outputTokens, totalTokens)
			}

			result.WriteString("data: ")
			result.Write(patchedJSON)
			result.WriteString("\n")
		} else {
			result.WriteString(line)
			result.WriteString("\n")
		}
	}

	// 如果没有成功注入，可能是 SSE 格式不同，尝试直接在整个 event 中查找并替换
	if !injected {
		if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
			log.Printf("[Responses-Stream-Token] 逐行解析未找到, 尝试整体解析 event")
		}

		// 尝试从 event 中提取 JSON 部分（可能是多行格式）
		var jsonStart, jsonEnd int
		for i, line := range lines {
			if strings.HasPrefix(line, "data:") {
				jsonStart = i
				break
			}
		}

		// 合并所有 data: 行（支持 "data:" 和 "data: " 两种格式）
		var jsonBuilder strings.Builder
		for i := jsonStart; i < len(lines); i++ {
			line := lines[i]
			if strings.HasPrefix(line, "data:") {
				jsonData := strings.TrimPrefix(line, "data:")
				jsonData = strings.TrimPrefix(jsonData, " ") // 移除可能的前导空格
				jsonBuilder.WriteString(jsonData)
			} else if line == "" {
				jsonEnd = i
				break
			}
		}

		fullJSON := jsonBuilder.String()
		if fullJSON != "" {
			var data map[string]interface{}
			if err := json.Unmarshal([]byte(fullJSON), &data); err == nil {
				eventType, _ := data["type"].(string)
				if eventType == "response.completed" {
					response, ok := data["response"].(map[string]interface{})
					if !ok {
						response = make(map[string]interface{})
						data["response"] = response
					}

					response["usage"] = map[string]interface{}{
						"input_tokens":  inputTokens,
						"output_tokens": outputTokens,
						"total_tokens":  totalTokens,
					}

					patchedJSON, err := json.Marshal(data)
					if err == nil {
						injected = true
						// 重建 event
						result.Reset()
						for i := 0; i < jsonStart; i++ {
							result.WriteString(lines[i])
							result.WriteString("\n")
						}
						result.WriteString("data: ")
						result.Write(patchedJSON)
						result.WriteString("\n")
						for i := jsonEnd; i < len(lines); i++ {
							result.WriteString(lines[i])
							result.WriteString("\n")
						}

						if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
							log.Printf("[Responses-Stream-Token] 整体解析注入成功: InputTokens=%d, OutputTokens=%d",
								inputTokens, outputTokens)
						}
					}
				}
			}
		}
	}

	// 如果仍然没有成功注入，记录警告并打印 event 内容
	if !injected {
		if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
			// 打印 event 的前500个字符帮助调试
			eventPreview := event
			if len(eventPreview) > 500 {
				eventPreview = eventPreview[:500] + "..."
			}
			log.Printf("[Responses-Stream-Token] 警告: 未找到 response.completed 事件进行注入, event内容: %s", eventPreview)
		}
		return event, inputTokens, outputTokens
	}

	return result.String(), inputTokens, outputTokens
}

// patchResponsesCompletedEventUsage 修补 response.completed 事件中的 usage
func patchResponsesCompletedEventUsage(event string, requestBody []byte, outputText string, collected *responsesStreamUsage, envCfg *config.EnvConfig) string {
	var result strings.Builder
	lines := strings.Split(event, "\n")

	for _, line := range lines {
		// 支持 "data:" 和 "data: " 两种格式（有些上游不带空格）
		var jsonStr string
		if strings.HasPrefix(line, "data:") {
			jsonStr = strings.TrimPrefix(line, "data:")
			jsonStr = strings.TrimPrefix(jsonStr, " ") // 移除可能的前导空格
		} else {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		if data["type"] == "response.completed" {
			if response, ok := data["response"].(map[string]interface{}); ok {
				if usage, ok := response["usage"].(map[string]interface{}); ok {
					originalInput := collected.InputTokens
					originalOutput := collected.OutputTokens
					patched := false

					// 修补 input_tokens（仅当没有 Claude 原生缓存时）
					// OpenAI 的 cached_tokens 不应阻止 input_tokens 补全
					if collected.InputTokens <= 1 && !collected.HasClaudeCache {
						estimatedInput := utils.EstimateResponsesRequestTokens(requestBody)
						usage["input_tokens"] = estimatedInput
						collected.InputTokens = estimatedInput
						patched = true
					}

					// 修补 output_tokens
					if collected.OutputTokens <= 1 {
						estimatedOutput := utils.EstimateTokens(outputText)
						usage["output_tokens"] = estimatedOutput
						collected.OutputTokens = estimatedOutput
						patched = true
					}

					// 重新计算 total_tokens（修补时或 total_tokens 为 0 但 input/output 有效时）
					currentTotal := 0
					if t, ok := usage["total_tokens"].(float64); ok {
						currentTotal = int(t)
					}
					if patched || (currentTotal == 0 && (collected.InputTokens > 0 || collected.OutputTokens > 0)) {
						usage["total_tokens"] = calculateTotalTokensWithCache(
							collected.InputTokens,
							collected.OutputTokens,
							collected.CacheReadInputTokens,
							collected.CacheCreationInputTokens,
							collected.CacheCreation5mInputTokens,
							collected.CacheCreation1hInputTokens,
						)
					}

					if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") && patched {
						log.Printf("[Responses-Stream-Token] 虚假值修补: InputTokens=%d->%d, OutputTokens=%d->%d",
							originalInput, collected.InputTokens, originalOutput, collected.OutputTokens)
					}
				}
			}

			patchedJSON, err := json.Marshal(data)
			if err != nil {
				result.WriteString(line)
				result.WriteString("\n")
				continue
			}

			result.WriteString("data: ")
			result.Write(patchedJSON)
			result.WriteString("\n")
		} else {
			result.WriteString(line)
			result.WriteString("\n")
		}
	}

	return result.String()
}

// parseInputToItems 解析 input 为 ResponsesItem 数组
func parseInputToItems(input interface{}) ([]types.ResponsesItem, error) {
	return types.ParseResponsesInput(input)
}

// hasResponsesFunctionCall 检查 Responses 事件中是否包含工具调用
func hasResponsesFunctionCall(event string) bool {
	lines := strings.Split(event, "\n")
	for _, line := range lines {
		var jsonStr string
		if strings.HasPrefix(line, "data:") {
			jsonStr = strings.TrimPrefix(line, "data:")
			jsonStr = strings.TrimPrefix(jsonStr, " ")
		} else {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		// 检查 response.output 中是否有 function_call 类型
		if response, ok := data["response"].(map[string]interface{}); ok {
			if output, ok := response["output"].([]interface{}); ok {
				for _, item := range output {
					if itemMap, ok := item.(map[string]interface{}); ok {
						if itemType, ok := itemMap["type"].(string); ok && itemType == "function_call" {
							return true
						}
					}
				}
			}
		}
	}
	return false
}

func buildResponsesPreflightDiagnostic(seenEvent, seenCompleted, seenUsageOnly, seenUnknown bool, unknownEventType, text string) string {
	switch {
	case !seenEvent:
		return "未收到任何转换后的 Responses 事件"
	case seenUsageOnly && common.IsEffectivelyEmptyStreamText(text):
		return "仅收到 usage/计数类 Responses 事件，没有文本或语义内容"
	case seenUnknown && common.IsEffectivelyEmptyStreamText(text):
		if unknownEventType != "" {
			return "收到了未识别的 Responses 事件类型=" + unknownEventType + "，但没有文本或语义内容"
		}
		return "收到了未识别的 Responses 事件类型，但没有文本或语义内容"
	case seenCompleted && common.IsEffectivelyEmptyStreamText(text):
		return "流正常结束(response.completed)，但未检测到文本或语义内容"
	default:
		return "检测到空的 Responses 流，但未匹配到明确类别"
	}
}

func isResponsesUsageOnlyEvent(event string) bool {
	lines := strings.Split(event, "\n")
	for _, line := range lines {
		var jsonStr string
		if strings.HasPrefix(line, "data:") {
			jsonStr = strings.TrimPrefix(line, "data:")
			jsonStr = strings.TrimPrefix(jsonStr, " ")
		} else {
			continue
		}
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}
		if data["type"] == "response.completed" {
			if response, ok := data["response"].(map[string]interface{}); ok {
				if usage, ok := response["usage"].(map[string]interface{}); ok && len(usage) > 0 {
					if output, ok := response["output"].([]interface{}); !ok || len(output) == 0 {
						return true
					}
				}
			}
		}
	}
	return false
}

func firstUnknownResponsesEventType(event string) (string, bool) {
	knownTypes := map[string]struct{}{
		"response.output_text.delta": {}, "response.function_call_arguments.delta": {}, "response.function_call_arguments.done": {},
		"response.reasoning_summary_text.delta": {}, "response.reasoning_summary_text.done": {}, "response.reasoning_summary_part.added": {}, "response.reasoning_summary_part.done": {},
		"response.output_json.delta": {}, "response.content_part.delta": {}, "response.audio.delta": {}, "response.audio_transcript.delta": {},
		"response.output_item.added": {}, "response.output_item.done": {}, "response.completed": {},
	}
	lines := strings.Split(event, "\n")
	for _, line := range lines {
		var jsonStr string
		if strings.HasPrefix(line, "data:") {
			jsonStr = strings.TrimPrefix(line, "data:")
			jsonStr = strings.TrimPrefix(jsonStr, " ")
		} else {
			continue
		}
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}
		if t, _ := data["type"].(string); t != "" {
			if _, ok := knownTypes[t]; !ok {
				return t, true
			}
		}
	}
	return "", false
}
