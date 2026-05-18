// Package common 提供 handlers 模块的公共功能
package common

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/providers"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
)

// ErrEmptyStreamResponse 上游返回 HTTP 200 但流式响应内容为空或几乎为空
// 空响应定义：OutputTokens == 0 或 OutputTokens == 1 且内容仅为 "{"
var ErrEmptyStreamResponse = errors.New("upstream returned empty stream response")

// ErrInvalidResponseBody 上游返回 HTTP 200 但响应体不是合法 JSON（如返回 HTML 错误页面）
// Header 未发送，可安全 failover 到下一个 Key/BaseURL/渠道
var ErrInvalidResponseBody = errors.New("upstream returned invalid response body")

// ErrBlacklistKey 上游在 SSE 流中返回了应拉黑 Key 的错误（认证/余额）
// Header 未发送，可安全 failover 到下一个 Key/BaseURL/渠道
type ErrBlacklistKey struct {
	Reason  string // "authentication_error" / "permission_error" / "insufficient_balance"
	Message string
}

func (e *ErrBlacklistKey) Error() string {
	return fmt.Sprintf("upstream stream error requires key blacklist: %s", e.Reason)
}

// StreamPreflightResult 流式预检测结果
type StreamPreflightResult struct {
	BufferedEvents   []string // 缓冲的事件（需要回放）
	IsEmpty          bool     // 是否为空响应
	HasError         bool     // 是否有流错误
	Error            error    // 流错误
	BlacklistReason  string   // 拉黑原因（非空时应拉黑 Key）
	BlacklistMessage string   // 拉黑错误信息
	Diagnostic       string   // 空响应诊断摘要
	UnknownEventType string   // 首个未知 SSE data.type
}

// PreflightStreamEvents 在发送 HTTP Header 之前预检测流式响应是否为空
// 缓冲事件并检查实际输出内容，避免发送 200 后无法撤销
func PreflightStreamEvents(eventChan <-chan string, errChan <-chan error) *StreamPreflightResult {
	result := &StreamPreflightResult{}
	var textBuf bytes.Buffer
	var thinkingBuf bytes.Buffer
	hasNonTextContent := false // tool_use / server_tool_use 等非文本语义内容
	seenEvent := false
	seenMessageStop := false
	seenUsageOnlyEvent := false
	seenUnknownDataType := false
	unknownEventType := ""
	timeout := time.NewTimer(30 * time.Second)
	defer timeout.Stop()

	for {
		select {
		case event, ok := <-eventChan:
			if !ok {
				// eventChan 关闭：流结束
				if hasNonTextContent {
					return result // 有非文本内容，视为非空
				}
				result.IsEmpty = isEmptyStreamContent(textBuf.String(), thinkingBuf.String())
				result.UnknownEventType = unknownEventType
				result.Diagnostic = buildClaudePreflightDiagnostic(seenEvent, seenMessageStop, seenUsageOnlyEvent, seenUnknownDataType, unknownEventType, textBuf.String(), thinkingBuf.String(), result.BufferedEvents)
				return result
			}
			seenEvent = true
			result.BufferedEvents = append(result.BufferedEvents, event)

			// 检测 SSE error 事件中的拉黑条件（认证/余额错误）
			if result.BlacklistReason == "" {
				if reason, msg := DetectStreamBlacklistError(event); reason != "" {
					result.BlacklistReason = reason
					result.BlacklistMessage = msg
				}
			}

			// 检测非文本 content block（tool_use / thinking）
			if !hasNonTextContent && hasNonTextContentBlock(event) {
				hasNonTextContent = true
				return result // 有效内容，立即放行
			}

			seenMessageStop = seenMessageStop || IsMessageStopEvent(event)
			if isUsageOnlySSEEvent(event) {
				seenUsageOnlyEvent = true
			}
			if t, ok := firstUnknownSSEDataType(event); ok {
				seenUnknownDataType = true
				if unknownEventType == "" {
					unknownEventType = t
				}
			}

			// 提取文本内容
			ExtractTextFromEvent(event, &textBuf)
			ExtractThinkingFromEvent(event, &thinkingBuf)

			// 检查是否有有效内容（非空且不是仅 "{"）
			if !isEmptyStreamContent(textBuf.String(), thinkingBuf.String()) {
				// 非空响应，放行
				return result
			}

			// 检查是否为 message_stop 事件（流正常结束）
			if IsMessageStopEvent(event) {
				if hasNonTextContent {
					return result
				}
				result.IsEmpty = isEmptyStreamContent(textBuf.String(), thinkingBuf.String())
				result.UnknownEventType = unknownEventType
				result.Diagnostic = buildClaudePreflightDiagnostic(seenEvent, true, seenUsageOnlyEvent, seenUnknownDataType, unknownEventType, textBuf.String(), thinkingBuf.String(), result.BufferedEvents)
				return result
			}

		case err, ok := <-errChan:
			if !ok {
				// errChan 关闭：置为 nil 防止 select 忙等自旋
				errChan = nil
				continue
			}
			if err != nil {
				result.HasError = true
				result.Error = err
				return result
			}

		case <-timeout.C:
			// 超时：保守放行
			return result
		}
	}
}

func buildClaudePreflightDiagnostic(seenEvent, seenMessageStop, seenUsageOnlyEvent, seenUnknownDataType bool, unknownEventType string, text string, thinking string, events []string) string {
	switch {
	case !seenEvent:
		return "未收到任何 SSE 事件"
	case seenUsageOnlyEvent && isEmptyStreamContent(text, thinking):
		return "仅收到 usage/计数类事件，没有文本或语义内容"
	case seenUnknownDataType && isEmptyStreamContent(text, thinking):
		if unknownEventType != "" {
			return "收到了未识别的 SSE data.type=" + unknownEventType + "，但没有文本或语义内容"
		}
		return "收到了未识别的 SSE data.type，但没有文本或语义内容"
	case seenMessageStop && isEmptyStreamContent(text, thinking):
		return "流正常结束(message_stop)，但未检测到文本或语义内容"
	default:
		return "检测到空流，但未匹配到明确类别"
	}
}

func isUsageOnlySSEEvent(event string) bool {
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}
		if usage, ok := data["usage"].(map[string]interface{}); ok && len(usage) > 0 {
			if _, hasDelta := data["delta"]; !hasDelta && data["type"] != "message_start" {
				return true
			}
		}
	}
	return false
}

func firstUnknownSSEDataType(event string) (string, bool) {
	knownTypes := map[string]struct{}{
		"message_start": {}, "message_delta": {}, "message_stop": {}, "content_block_start": {}, "content_block_delta": {}, "content_block_stop": {}, "ping": {}, "error": {},
	}
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
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

// isEmptyContent 判断流式响应的累积文本是否为空内容
func isEmptyContent(text string) bool {
	return IsEffectivelyEmptyStreamText(text)
}

func isEmptyStreamContent(text string, thinking string) bool {
	return IsEffectivelyEmptyStreamText(text) && IsEffectivelyEmptyStreamText(thinking)
}

// IsEffectivelyEmptyStreamText 判断流式响应文本是否仍可视为“空”
func IsEffectivelyEmptyStreamText(text string) bool {
	return text == "" || strings.TrimSpace(text) == "{"
}

func extractSSEJSONLine(line string) (string, bool) {
	if !strings.HasPrefix(line, "data:") {
		return "", false
	}
	jsonStr := strings.TrimPrefix(line, "data:")
	return strings.TrimPrefix(jsonStr, " "), true
}

// hasNonTextContentBlock 检测 SSE 事件是否包含可立即判定为有效的非文本语义内容（如 tool_use）
// 这些 content block 不产生 delta.text，但属于有效响应内容
func hasNonTextContentBlock(event string) bool {
	return HasClaudeSemanticContent(event)
}

// HasClaudeSemanticContent 判断 Claude/Messages 风格 SSE 是否包含有效语义内容
func HasClaudeSemanticContent(event string) bool {
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		// content_block_start 事件中检查 content_block.type
		if cb, ok := data["content_block"].(map[string]interface{}); ok {
			if cbType, ok := cb["type"].(string); ok {
				switch cbType {
				case "text", "", "thinking", "redacted_thinking":
				default:
					return true
				}
			}
		}

		if delta, ok := data["delta"].(map[string]interface{}); ok {
			if deltaType, _ := delta["type"].(string); deltaType == "input_json_delta" {
				return true
			}
			if stopReason, _ := delta["stop_reason"].(string); stopReason == "tool_use" || stopReason == "server_tool_use" {
				return true
			}
		}
	}
	return false
}

func responseItemCarriesSemanticContent(item map[string]interface{}) bool {
	itemType, _ := item["type"].(string)
	switch itemType {
	case "function_call", "reasoning":
		return true
	}
	return strings.HasSuffix(itemType, "_call")
}

// HasResponsesSemanticContent 判断 Responses 风格 SSE 是否包含有效语义内容
func HasResponsesSemanticContent(event string) bool {
	lines := strings.Split(event, "\n")
	for _, line := range lines {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		switch data["type"] {
		case "response.function_call_arguments.delta", "response.function_call_arguments.done",
			"response.reasoning_summary_part.added", "response.reasoning_summary_part.done",
			"response.reasoning_summary_text.done":
			return true
		case "response.output_item.added", "response.output_item.done":
			item, _ := data["item"].(map[string]interface{})
			if responseItemCarriesSemanticContent(item) {
				return true
			}
		case "response.completed":
			if response, ok := data["response"].(map[string]interface{}); ok {
				if output, ok := response["output"].([]interface{}); ok {
					for _, item := range output {
						if itemMap, ok := item.(map[string]interface{}); ok && responseItemCarriesSemanticContent(itemMap) {
							return true
						}
					}
				}
			}
		}
	}
	return false
}

// drainChannels 排空 eventChan 和 errChan，防止 provider goroutine 泄漏
// 使用超时保护，避免在 channel 未关闭时永久阻塞
func drainChannels(eventChan <-chan string, errChan <-chan error) {
	go func() {
		timeout := time.After(60 * time.Second)
		for {
			select {
			case _, ok := <-eventChan:
				if !ok {
					return
				}
			case <-timeout:
				return
			}
		}
	}()
	go func() {
		timeout := time.After(60 * time.Second)
		for {
			select {
			case _, ok := <-errChan:
				if !ok {
					return
				}
			case <-timeout:
				return
			}
		}
	}()
}

// StreamContext 流处理上下文
type StreamContext struct {
	LogBuffer            *LimitedLogBuffer
	OutputTextBuffer     bytes.Buffer
	Synthesizer          *utils.StreamSynthesizer
	LoggingEnabled       bool
	ClientGone           bool
	HasUsage             bool
	HasMessageDeltaUsage bool
	NeedTokenPatch       bool
	// 累积的 token 统计
	CollectedUsage CollectedUsageData
	// 用于日志的"续写前缀"（不参与真实转发，只影响 Stream-Synth 输出可读性）
	LogPrefillText string
	// SSE 事件调试追踪
	EventCount        int            // 事件总数
	ContentBlockCount int            // content block 计数
	ContentBlockTypes map[int]string // 每个 block 的类型
	// 低质量渠道处理
	RequestModel string // 请求中的 model（用于一致性检查）
	LowQuality   bool   // 是否为低质量渠道
	// 隐式缓存推断
	MessageStartInputTokens int // message_start 事件中的 input_tokens（用于推断隐式缓存）
}

// CollectedUsageData 从流事件中收集的 usage 数据
type CollectedUsageData struct {
	InputTokens              int
	OutputTokens             int
	CacheCreationInputTokens int
	CacheReadInputTokens     int
	// 缓存 TTL 细分
	CacheCreation5mInputTokens int
	CacheCreation1hInputTokens int
	CacheTTL                   string // "5m" | "1h" | "mixed"
}

// NewStreamContext 创建流处理上下文
func NewStreamContext(envCfg *config.EnvConfig) *StreamContext {
	ctx := &StreamContext{
		LoggingEnabled:    envCfg.IsDevelopment() && envCfg.EnableResponseLogs,
		ContentBlockTypes: make(map[int]string),
		LogBuffer:         NewLimitedLogBuffer(MaxUpstreamResponseLogBytes),
	}
	if ctx.LoggingEnabled {
		ctx.Synthesizer = utils.NewStreamSynthesizer("claude")
	}
	return ctx
}

// seedSynthesizerFromRequest 将请求里预置的 assistant 文本拼接进合成器（仅用于日志可读性）
//
// Claude Code 的部分内部调用会在 messages 里预置一条 assistant 内容（例如 "{"），让模型只输出“续写”部分。
// 这会导致我们仅基于 SSE delta 合成的日志缺失开头。这里用请求体做一次轻量补齐。
func seedSynthesizerFromRequest(ctx *StreamContext, requestBody []byte) {
	if ctx == nil || ctx.Synthesizer == nil || len(requestBody) == 0 {
		return
	}

	var req struct {
		Messages []struct {
			Role    string `json:"role"`
			Content []struct {
				Type string `json:"type"`
				Text string `json:"text"`
			} `json:"content"`
		} `json:"messages"`
	}
	if err := json.Unmarshal(requestBody, &req); err != nil {
		return
	}

	// 只取最后一条 assistant，避免把历史上下文都拼进日志
	for i := len(req.Messages) - 1; i >= 0; i-- {
		msg := req.Messages[i]
		if msg.Role != "assistant" {
			continue
		}
		var b strings.Builder
		for _, c := range msg.Content {
			if c.Type == "text" && c.Text != "" {
				b.WriteString(c.Text)
			}
		}
		prefill := b.String()
		// 防止把很长的预置内容刷进日志
		if len(prefill) > 0 && len(prefill) <= 256 {
			ctx.LogPrefillText = prefill
		}
		return
	}
}

// SetupStreamHeaders 设置流式响应头
func SetupStreamHeaders(c *gin.Context, resp *http.Response, envCfg *config.EnvConfig, apiType string) {
	LogUpstreamResponseHeaders(resp, envCfg, apiType)
	utils.ForwardResponseHeaders(resp.Header, c.Writer)
	c.Header("Content-Type", "text/event-stream")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	c.Header("X-Accel-Buffering", "no")
	c.Status(200)
}

// ProcessStreamEvents 处理流事件循环
// 返回值: error 表示流处理过程中是否发生错误（用于调用方决定是否记录失败指标）
func ProcessStreamEvents(
	c *gin.Context,
	w gin.ResponseWriter,
	flusher http.Flusher,
	eventChan <-chan string,
	errChan <-chan error,
	ctx *StreamContext,
	envCfg *config.EnvConfig,
	startTime time.Time,
	requestBody []byte,
) (*types.Usage, error) {
	for {
		select {
		case event, ok := <-eventChan:
			if !ok {
				usage := logStreamCompletion(ctx, envCfg, startTime)
				return usage, nil
			}
			ProcessStreamEvent(c, w, flusher, event, ctx, envCfg, requestBody)

		case err, ok := <-errChan:
			if !ok {
				continue
			}
			if err != nil {
				log.Printf("[Messages-Stream] 错误: 流式传输错误: %v", err)
				logPartialResponse(ctx, envCfg)

				// 向客户端发送错误事件（如果连接仍然有效）
				if !ctx.ClientGone {
					errorEvent := BuildStreamErrorEvent(err)
					w.Write([]byte(errorEvent))
					flusher.Flush()
				}

				return nil, err
			}
		}
	}
}

// ProcessStreamEvent 处理单个流事件
func ProcessStreamEvent(
	c *gin.Context,
	w gin.ResponseWriter,
	flusher http.Flusher,
	event string,
	ctx *StreamContext,
	envCfg *config.EnvConfig,
	requestBody []byte,
) {
	// SSE 事件调试日志
	ctx.EventCount++
	if envCfg.SSEDebugLevel == "full" || envCfg.SSEDebugLevel == "summary" {
		eventType, blockIndex, blockType := extractSSEEventInfo(event)
		if eventType == "content_block_start" {
			ctx.ContentBlockCount++
			if blockType != "" {
				ctx.ContentBlockTypes[blockIndex] = blockType
			}
		}
		if envCfg.SSEDebugLevel == "full" {
			log.Printf("[Messages-Stream-Event] #%d 类型=%s 长度=%d block_index=%d block_type=%s",
				ctx.EventCount, eventType, len(event), blockIndex, blockType)
			// 对于 content_block 相关事件，记录详细内容
			if strings.Contains(event, "content_block") {
				log.Printf("[Messages-Stream-Event] 详情: %s", truncateForLog(event, 500))
			}
		}
	}

	// 提取文本用于估算 token
	ExtractTextFromEvent(event, &ctx.OutputTextBuffer)

	// 检测并收集 usage
	hasUsage, needInputPatch, needOutputPatch, usageData := CheckEventUsageStatus(event, envCfg.EnableResponseLogs && envCfg.ShouldLog("debug"))
	needPatch := needInputPatch || needOutputPatch
	// 保存原始 usageData 用于后续 PatchMessageStartInputTokensIfNeeded
	originalUsageData := usageData
	if hasUsage {
		if !ctx.HasUsage {
			ctx.HasUsage = true
			ctx.NeedTokenPatch = needPatch || ctx.LowQuality
			if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") && needPatch && !IsMessageDeltaEvent(event) {
				log.Printf("[Messages-Stream-Token] 检测到虚假值, 延迟到流结束修补")
			}
		}
		// 对于 message_start 事件，不累积 input_tokens 到 CollectedUsage
		// 因为 message_start 的 input_tokens 是请求总 token，而非最终计费值
		// CollectedUsage.InputTokens 应该只记录 message_delta 的最终计费值
		if IsMessageStartEvent(event) && usageData.InputTokens > 0 {
			usageData.InputTokens = 0
		}
		// 累积收集 usage 数据
		updateCollectedUsage(&ctx.CollectedUsage, usageData)

		if IsMessageDeltaEvent(event) {
			ctx.HasMessageDeltaUsage = true
		}
	}

	// 日志缓存
	if ctx.LoggingEnabled {
		ctx.LogBuffer.WriteString(event)
		if ctx.Synthesizer != nil {
			for _, line := range strings.Split(event, "\n") {
				ctx.Synthesizer.ProcessLine(line)
			}
		}
	}

	// 在 message_stop 前注入 usage（message_delta 未携带 usage 的兜底场景）
	if !ctx.HasMessageDeltaUsage && !ctx.ClientGone && IsMessageStopEvent(event) {
		usageEvent := BuildUsageEvent(requestBody, ctx.OutputTextBuffer.String())
		if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
			log.Printf("[Messages-Stream-Token] message_delta 缺少 usage, 在 message_stop 前注入兜底 usage 事件")
		}
		w.Write([]byte(usageEvent))
		flusher.Flush()
		ctx.HasUsage = true
		ctx.HasMessageDeltaUsage = true
	}

	// 修补 token
	eventToSend := event

	// 处理 message_start 事件：补全空 id 和检查 model 一致性（可选）
	if IsMessageStartEvent(event) && ctx.RequestModel != "" {
		eventToSend = PatchMessageStartEvent(eventToSend, ctx.RequestModel, envCfg.RewriteResponseModel, envCfg.EnableResponseLogs && envCfg.ShouldLog("debug"))
	}

	// 处理 message_start 事件：尽早补全 input_tokens（部分客户端只读取首个 usage 来累计）
	// 注意：使用 originalUsageData 而非被清零后的 usageData，避免误判
	if hasUsage {
		eventToSend = PatchMessageStartInputTokensIfNeeded(eventToSend, requestBody, needInputPatch, originalUsageData, envCfg.EnableResponseLogs && envCfg.ShouldLog("debug"), ctx.LowQuality)
	}

	// 对严格客户端做协议兜底：任何 message_delta 都应带顶层 usage。
	if IsMessageDeltaEvent(eventToSend) && !HasEventWithUsage(eventToSend) {
		inputTokens := ctx.CollectedUsage.InputTokens
		outputTokens := ctx.CollectedUsage.OutputTokens

		estimatedInputTokens := utils.EstimateRequestTokens(requestBody)
		estimatedOutputTokens := utils.EstimateTokens(ctx.OutputTextBuffer.String())

		if inputTokens <= 0 && estimatedInputTokens > 0 {
			inputTokens = estimatedInputTokens
		}
		if outputTokens <= 0 && estimatedOutputTokens > 0 {
			outputTokens = estimatedOutputTokens
		}

		eventToSend = EnsureMessageDeltaUsage(eventToSend, inputTokens, outputTokens)

		if inputTokens > ctx.CollectedUsage.InputTokens {
			ctx.CollectedUsage.InputTokens = inputTokens
		}
		if outputTokens > ctx.CollectedUsage.OutputTokens {
			ctx.CollectedUsage.OutputTokens = outputTokens
		}

		ctx.HasUsage = true
		ctx.HasMessageDeltaUsage = true
		if envCfg.EnableResponseLogs && envCfg.ShouldLog("debug") {
			log.Printf("[Messages-Stream-Token] message_delta 缺少 usage, 已就地补齐 input=%d output=%d", inputTokens, outputTokens)
		}
	}

	// 记录 message_start 中的 input_tokens（用于后续推断隐式缓存）
	// 注意：必须在 PatchMessageStartInputTokensIfNeeded 之后执行，因为原始值可能是 0 被修补成估算值
	if IsMessageStartEvent(event) && ctx.MessageStartInputTokens == 0 {
		if patchedInputTokens := ExtractInputTokensFromEvent(eventToSend); patchedInputTokens > 0 {
			ctx.MessageStartInputTokens = patchedInputTokens
		}
	}

	if ctx.NeedTokenPatch && HasEventWithUsage(eventToSend) {
		if IsMessageDeltaEvent(eventToSend) || IsMessageStopEvent(eventToSend) {
			hasCacheTokens := ctx.CollectedUsage.CacheCreationInputTokens > 0 ||
				ctx.CollectedUsage.CacheReadInputTokens > 0 ||
				ctx.CollectedUsage.CacheCreation5mInputTokens > 0 ||
				ctx.CollectedUsage.CacheCreation1hInputTokens > 0

			// 在转发前执行隐式缓存推断，确保下游能收到推断的 cache_read_input_tokens
			if !hasCacheTokens {
				inferImplicitCacheRead(ctx, envCfg.EnableResponseLogs && envCfg.ShouldLog("debug"))
				// 重新检查是否有缓存 token（可能刚被推断出来）
				hasCacheTokens = ctx.CollectedUsage.CacheReadInputTokens > 0
			}

			// 检测隐式缓存信号：message_start 的 input_tokens 远大于最终值
			// 这种情况下不应该用本地估算值覆盖，因为低 input_tokens 是缓存命中的正常结果
			hasImplicitCacheSignal := ctx.MessageStartInputTokens > 0 &&
				ctx.CollectedUsage.InputTokens > 0 &&
				ctx.MessageStartInputTokens > ctx.CollectedUsage.InputTokens

			inputTokens := ctx.CollectedUsage.InputTokens
			estimatedInputTokens := utils.EstimateRequestTokens(requestBody)
			// 仅在无缓存信号（显式或隐式）且 input_tokens 异常小时才用估算值修补
			if !hasCacheTokens && !hasImplicitCacheSignal && inputTokens < 10 && estimatedInputTokens > inputTokens {
				inputTokens = estimatedInputTokens
			}

			outputTokens := ctx.CollectedUsage.OutputTokens
			estimatedOutputTokens := utils.EstimateTokens(ctx.OutputTextBuffer.String())
			if outputTokens <= 1 && estimatedOutputTokens > outputTokens {
				outputTokens = estimatedOutputTokens
			}

			if inputTokens > ctx.CollectedUsage.InputTokens {
				ctx.CollectedUsage.InputTokens = inputTokens
			}
			if outputTokens > ctx.CollectedUsage.OutputTokens {
				ctx.CollectedUsage.OutputTokens = outputTokens
			}

			// 修补事件，包括推断的 cache_read_input_tokens
			eventToSend = PatchTokensInEventWithCache(eventToSend, inputTokens, outputTokens, ctx.CollectedUsage.CacheReadInputTokens, hasCacheTokens, envCfg.EnableResponseLogs && envCfg.ShouldLog("debug"), ctx.LowQuality)
			ctx.NeedTokenPatch = false
		}
	}

	if IsMessageDeltaEvent(eventToSend) && HasEventWithUsage(eventToSend) {
		ctx.HasUsage = true
		ctx.HasMessageDeltaUsage = true
	}

	// 转发给客户端
	if !ctx.ClientGone {
		if _, err := w.Write([]byte(eventToSend)); err != nil {
			ctx.ClientGone = true
			if !IsClientDisconnectError(err) {
				log.Printf("[Messages-Stream] 警告: 写入错误: %v", err)
			} else if envCfg.ShouldLog("info") {
				log.Printf("[Messages-Stream] 客户端中断连接 (正常行为)，继续接收上游数据...")
			}
		} else {
			flusher.Flush()
		}
	}
}

// EnsureMessageDeltaUsage 确保 message_delta 事件包含顶层 usage 字段。
func EnsureMessageDeltaUsage(event string, inputTokens, outputTokens int) string {
	if inputTokens < 0 {
		inputTokens = 0
	}
	if outputTokens < 0 {
		outputTokens = 0
	}

	var result strings.Builder
	lines := strings.Split(event, "\n")

	for _, line := range lines {
		if !strings.HasPrefix(line, "data: ") {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		jsonStr := strings.TrimPrefix(line, "data: ")
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		if data["type"] == "message_delta" {
			if _, exists := data["usage"].(map[string]interface{}); !exists {
				data["usage"] = map[string]int{
					"input_tokens":  inputTokens,
					"output_tokens": outputTokens,
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
	}

	return result.String()
}

// updateCollectedUsage 更新收集的 usage 数据
func updateCollectedUsage(collected *CollectedUsageData, usageData CollectedUsageData) {
	if usageData.InputTokens > collected.InputTokens {
		collected.InputTokens = usageData.InputTokens
	}
	if usageData.OutputTokens > collected.OutputTokens {
		collected.OutputTokens = usageData.OutputTokens
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
}

// inferImplicitCacheRead 推断隐式缓存读取
//
// 当 message_start 中的 input_tokens 与 message_delta 中的最终 input_tokens 存在显著差异时，
// 差额可能是上游 prompt caching 命中但未明确返回 cache_read_input_tokens 的情况。
// 触发条件：差额 > 10% 或差额 > 10000 tokens，且上游未返回 cache_read_input_tokens。
func inferImplicitCacheRead(ctx *StreamContext, enableLog bool) {
	// 前置条件检查
	if ctx.MessageStartInputTokens == 0 || ctx.CollectedUsage.InputTokens == 0 {
		return
	}

	// 上游已明确返回 cache_read，无需推断
	if ctx.CollectedUsage.CacheReadInputTokens > 0 {
		return
	}

	// 计算差额
	diff := ctx.MessageStartInputTokens - ctx.CollectedUsage.InputTokens
	if diff <= 0 {
		return
	}

	// 计算差额比例
	ratio := float64(diff) / float64(ctx.MessageStartInputTokens)

	// 触发条件：差额 > 10% 或差额 > 10000 tokens
	if ratio > 0.10 || diff > 10000 {
		ctx.CollectedUsage.CacheReadInputTokens = diff
		if enableLog {
			log.Printf("[Messages-Stream-Token] 推断隐式缓存: message_start=%d, final=%d, cache_read=%d (%.1f%%)",
				ctx.MessageStartInputTokens, ctx.CollectedUsage.InputTokens, diff, ratio*100)
		}
	}
}

// logStreamCompletion 记录流完成日志
func logStreamCompletion(ctx *StreamContext, envCfg *config.EnvConfig, startTime time.Time) *types.Usage {
	if envCfg.EnableResponseLogs {
		log.Printf("[Messages-Stream] 流式响应完成: %dms", time.Since(startTime).Milliseconds())
	}
	if ctx.ClientGone && envCfg.ShouldLog("info") {
		log.Printf("[Messages-Stream] 客户端已提前断开；上游流仍已完整接收（仅服务端日志可见）")
	}

	// SSE 事件统计日志
	if envCfg.SSEDebugLevel == "full" || envCfg.SSEDebugLevel == "summary" {
		blockTypeSummary := make(map[string]int)
		for _, bt := range ctx.ContentBlockTypes {
			blockTypeSummary[bt]++
		}
		log.Printf("[Messages-Stream-Summary] 总事件数=%d, content_blocks=%d, 类型分布=%v",
			ctx.EventCount, ctx.ContentBlockCount, blockTypeSummary)
	}

	if envCfg.IsDevelopment() {
		logSynthesizedContent(ctx)
	}

	// 推断隐式缓存读取
	inferImplicitCacheRead(ctx, envCfg.EnableResponseLogs && envCfg.ShouldLog("debug"))

	// 将累积的 usage 数据转换为 *types.Usage
	var usage *types.Usage
	hasUsageData := ctx.CollectedUsage.InputTokens > 0 ||
		ctx.CollectedUsage.OutputTokens > 0 ||
		ctx.CollectedUsage.CacheCreationInputTokens > 0 ||
		ctx.CollectedUsage.CacheReadInputTokens > 0 ||
		ctx.CollectedUsage.CacheCreation5mInputTokens > 0 ||
		ctx.CollectedUsage.CacheCreation1hInputTokens > 0
	if hasUsageData {
		usage = &types.Usage{
			InputTokens:                ctx.CollectedUsage.InputTokens,
			OutputTokens:               ctx.CollectedUsage.OutputTokens,
			CacheCreationInputTokens:   ctx.CollectedUsage.CacheCreationInputTokens,
			CacheReadInputTokens:       ctx.CollectedUsage.CacheReadInputTokens,
			CacheCreation5mInputTokens: ctx.CollectedUsage.CacheCreation5mInputTokens,
			CacheCreation1hInputTokens: ctx.CollectedUsage.CacheCreation1hInputTokens,
			CacheTTL:                   ctx.CollectedUsage.CacheTTL,
		}
	}
	return usage
}

// logPartialResponse 记录部分响应日志
func logPartialResponse(ctx *StreamContext, envCfg *config.EnvConfig) {
	if envCfg.EnableResponseLogs && envCfg.IsDevelopment() {
		logSynthesizedContent(ctx)
	}
}

// logSynthesizedContent 记录合成内容
func logSynthesizedContent(ctx *StreamContext) {
	if ctx.Synthesizer != nil {
		content := ctx.Synthesizer.GetSynthesizedContent()
		if content != "" && !ctx.Synthesizer.IsParseFailed() {
			trimmed := strings.TrimSpace(content)

			// 仅在“明显是 JSON 续写”的情况下拼接预置前缀，避免出现 "{OK" 这类误导日志
			if ctx.LogPrefillText == "{" && !strings.HasPrefix(strings.TrimLeft(trimmed, " \t\r\n"), "{") {
				left := strings.TrimLeft(trimmed, " \t\r\n")
				if strings.HasPrefix(left, "\"") {
					trimmed = ctx.LogPrefillText + trimmed
				}
			}

			log.Printf("[Messages-Stream] 上游流式响应合成内容:\n%s", strings.TrimSpace(trimmed))
			return
		}
	}
	if ctx.LogBuffer.Len() > 0 {
		log.Printf("[Messages-Stream] 上游流式响应原始内容:\n%s", ctx.LogBuffer.String())
	}
}

// IsClientDisconnectError 判断是否为客户端断开连接错误
func IsClientDisconnectError(err error) bool {
	msg := err.Error()
	return strings.Contains(msg, "broken pipe") ||
		strings.Contains(msg, "connection reset") ||
		strings.Contains(msg, "context canceled")
}

// HandleStreamResponse 处理流式响应（Messages API）
//
// 流程：provider.HandleStreamResponse → PreflightStreamEvents（预检测）
//   - 空响应 → return nil, ErrEmptyStreamResponse（Header 未发送，可安全重试）
//   - 非空   → SetupStreamHeaders → 回放缓冲事件 → ProcessStreamEvents
func HandleStreamResponse(
	c *gin.Context,
	resp *http.Response,
	provider providers.Provider,
	envCfg *config.EnvConfig,
	startTime time.Time,
	upstream *config.UpstreamConfig,
	requestBody []byte,
	requestModel string,
) (*types.Usage, error) {
	defer resp.Body.Close()

	eventChan, errChan, err := provider.HandleStreamResponse(resp.Body)
	if err != nil {
		c.JSON(500, gin.H{"error": "Failed to handle stream response"})
		return nil, err
	}

	// 预检测：在发送 HTTP Header 之前缓冲事件并检查是否为空响应
	preflight := PreflightStreamEvents(eventChan, errChan)

	// 流错误：排空 channel 后返回错误
	if preflight.HasError {
		drainChannels(eventChan, errChan)
		return nil, preflight.Error
	}

	// 空响应：Header 未发送，可安全重试
	if preflight.IsEmpty {
		log.Printf("[Messages-EmptyResponse] 上游返回空响应 (缓冲事件数: %d, 诊断: %s)，触发重试", len(preflight.BufferedEvents), preflight.Diagnostic)
		drainChannels(eventChan, errChan)
		// 如果同时检测到拉黑条件，优先返回拉黑错误
		if preflight.BlacklistReason != "" {
			return nil, &ErrBlacklistKey{Reason: preflight.BlacklistReason, Message: preflight.BlacklistMessage}
		}
		return nil, ErrEmptyStreamResponse
	}

	// 流中有拉黑错误但内容非空（如错误前有部分输出）：仍返回拉黑错误以触发 Key 拉黑
	if preflight.BlacklistReason != "" {
		drainChannels(eventChan, errChan)
		return nil, &ErrBlacklistKey{Reason: preflight.BlacklistReason, Message: preflight.BlacklistMessage}
	}

	// 非空响应：正常流程
	SetupStreamHeaders(c, resp, envCfg, "Messages")

	w := c.Writer
	flusher, ok := w.(http.Flusher)
	if !ok {
		log.Printf("[Messages-Stream] 警告: ResponseWriter不支持Flush接口")
		drainChannels(eventChan, errChan)
		return nil, fmt.Errorf("ResponseWriter不支持Flush接口")
	}
	flusher.Flush()

	ctx := NewStreamContext(envCfg)
	ctx.RequestModel = requestModel
	ctx.LowQuality = upstream.LowQuality
	seedSynthesizerFromRequest(ctx, requestBody)

	// 回放预检测期间缓冲的事件
	for _, bufferedEvent := range preflight.BufferedEvents {
		ProcessStreamEvent(c, w, flusher, bufferedEvent, ctx, envCfg, requestBody)
	}

	usage, err := ProcessStreamEvents(c, w, flusher, eventChan, errChan, ctx, envCfg, startTime, requestBody)
	if err != nil {
		return nil, err
	}
	return annotatePromptTokensTotalForProvider(provider, usage), nil
}

func annotatePromptTokensTotalForProvider(provider providers.Provider, usage *types.Usage) *types.Usage {
	if usage == nil {
		return nil
	}
	if _, ok := provider.(*providers.ResponsesProvider); ok && usage.InputTokens > 0 {
		usage.PromptTokensTotal = usage.InputTokens
	}
	return usage
}

// ========== Token 检测和修补相关函数 ==========

// CheckEventUsageStatus 检测事件是否包含 usage 字段
func CheckEventUsageStatus(event string, enableLog bool) (bool, bool, bool, CollectedUsageData) {
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		// 检查顶层 usage 字段
		if hasUsage, needInputPatch, needOutputPatch := checkUsageFieldsWithPatch(data["usage"]); hasUsage {
			var usageData CollectedUsageData
			if usage, ok := data["usage"].(map[string]interface{}); ok {
				if enableLog {
					logUsageDetection("顶层usage", usage, needInputPatch || needOutputPatch)
				}
				usageData = extractUsageFromMap(usage)
			}
			return true, needInputPatch, needOutputPatch, usageData
		}

		// 检查 message.usage
		if msg, ok := data["message"].(map[string]interface{}); ok {
			if hasUsage, needInputPatch, needOutputPatch := checkUsageFieldsWithPatch(msg["usage"]); hasUsage {
				var usageData CollectedUsageData
				if usage, ok := msg["usage"].(map[string]interface{}); ok {
					if enableLog {
						logUsageDetection("message.usage", usage, needInputPatch || needOutputPatch)
					}
					usageData = extractUsageFromMap(usage)
				}
				return true, needInputPatch, needOutputPatch, usageData
			}
		}
	}
	return false, false, false, CollectedUsageData{}
}

// checkUsageFieldsWithPatch 检查 usage 对象是否包含 token 字段
func checkUsageFieldsWithPatch(usage interface{}) (bool, bool, bool) {
	if u, ok := usage.(map[string]interface{}); ok {
		inputTokens, hasInput := u["input_tokens"]
		outputTokens, hasOutput := u["output_tokens"]
		if hasInput || hasOutput {
			needInputPatch := false
			needOutputPatch := false

			cacheCreation, _ := u["cache_creation_input_tokens"].(float64)
			cacheRead, _ := u["cache_read_input_tokens"].(float64)
			hasCacheTokens := cacheCreation > 0 || cacheRead > 0

			if hasInput {
				if inputTokens == nil {
					// input_tokens 为 nil 时需要修补
					needInputPatch = true
				} else if v, ok := inputTokens.(float64); ok && v <= 1 && !hasCacheTokens {
					needInputPatch = true
				}
			}
			if hasOutput {
				if v, ok := outputTokens.(float64); ok && v <= 1 {
					needOutputPatch = true
				}
			}
			return true, needInputPatch, needOutputPatch
		}
	}
	return false, false, false
}

// extractUsageFromMap 从 usage map 中提取 token 数据
func extractUsageFromMap(usage map[string]interface{}) CollectedUsageData {
	var data CollectedUsageData

	if v, ok := usage["input_tokens"].(float64); ok {
		data.InputTokens = int(v)
	}
	if v, ok := usage["output_tokens"].(float64); ok {
		data.OutputTokens = int(v)
	}
	if v, ok := usage["cache_creation_input_tokens"].(float64); ok {
		data.CacheCreationInputTokens = int(v)
	}
	if v, ok := usage["cache_read_input_tokens"].(float64); ok {
		data.CacheReadInputTokens = int(v)
	}

	var has5m, has1h bool
	if v, ok := usage["cache_creation_5m_input_tokens"].(float64); ok {
		data.CacheCreation5mInputTokens = int(v)
		has5m = data.CacheCreation5mInputTokens > 0
	}
	if v, ok := usage["cache_creation_1h_input_tokens"].(float64); ok {
		data.CacheCreation1hInputTokens = int(v)
		has1h = data.CacheCreation1hInputTokens > 0
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

// logUsageDetection 统一格式输出 usage 检测日志
func logUsageDetection(location string, usage map[string]interface{}, needPatch bool) {
	inputTokens := usage["input_tokens"]
	outputTokens := usage["output_tokens"]
	cacheCreation, _ := usage["cache_creation_input_tokens"].(float64)
	cacheRead, _ := usage["cache_read_input_tokens"].(float64)

	log.Printf("[Messages-Stream-Token] %s: InputTokens=%v, OutputTokens=%v, CacheCreation=%.0f, CacheRead=%.0f, 需补全=%v",
		location, inputTokens, outputTokens, cacheCreation, cacheRead, needPatch)
}

// HasEventWithUsage 检查事件是否包含 usage 字段
func HasEventWithUsage(event string) bool {
	for _, line := range strings.Split(event, "\n") {
		if !strings.HasPrefix(line, "data: ") {
			continue
		}
		jsonStr := strings.TrimPrefix(line, "data: ")

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		if _, ok := data["usage"].(map[string]interface{}); ok {
			return true
		}

		if msg, ok := data["message"].(map[string]interface{}); ok {
			if _, ok := msg["usage"].(map[string]interface{}); ok {
				return true
			}
		}
	}
	return false
}

// PatchTokensInEvent 修补事件中的 token 字段
func PatchTokensInEvent(event string, estimatedInputTokens, estimatedOutputTokens int, hasCacheTokens bool, enableLog bool, lowQuality bool) string {
	var result strings.Builder
	lines := strings.Split(event, "\n")

	for _, line := range lines {
		if !strings.HasPrefix(line, "data: ") {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		jsonStr := strings.TrimPrefix(line, "data: ")
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		// 修补顶层 usage
		if usage, ok := data["usage"].(map[string]interface{}); ok {
			patchUsageFieldsWithLog(usage, estimatedInputTokens, estimatedOutputTokens, hasCacheTokens, enableLog, "顶层usage", lowQuality)
		}

		// 修补 message.usage
		if msg, ok := data["message"].(map[string]interface{}); ok {
			if usage, ok := msg["usage"].(map[string]interface{}); ok {
				patchUsageFieldsWithLog(usage, estimatedInputTokens, estimatedOutputTokens, hasCacheTokens, enableLog, "message.usage", lowQuality)
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
	}

	return result.String()
}

// PatchTokensInEventWithCache 修补事件中的 token 字段，并写入推断的 cache_read_input_tokens
// 当 inferredCacheRead > 0 且事件中没有 cache_read_input_tokens 时，将推断值写入
func PatchTokensInEventWithCache(event string, estimatedInputTokens, estimatedOutputTokens, inferredCacheRead int, hasCacheTokens bool, enableLog bool, lowQuality bool) string {
	var result strings.Builder
	lines := strings.Split(event, "\n")

	for _, line := range lines {
		if !strings.HasPrefix(line, "data: ") {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		jsonStr := strings.TrimPrefix(line, "data: ")
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		// 修补顶层 usage
		if usage, ok := data["usage"].(map[string]interface{}); ok {
			patchUsageFieldsWithLog(usage, estimatedInputTokens, estimatedOutputTokens, hasCacheTokens, enableLog, "顶层usage", lowQuality)
			// 写入推断的 cache_read_input_tokens（仅当字段不存在时）
			if inferredCacheRead > 0 {
				if _, exists := usage["cache_read_input_tokens"]; !exists {
					usage["cache_read_input_tokens"] = inferredCacheRead
					if enableLog {
						log.Printf("[Messages-Stream-Token] 顶层usage: 写入推断的 cache_read_input_tokens=%d", inferredCacheRead)
					}
				}
			}
		}

		// 修补 message.usage
		if msg, ok := data["message"].(map[string]interface{}); ok {
			if usage, ok := msg["usage"].(map[string]interface{}); ok {
				patchUsageFieldsWithLog(usage, estimatedInputTokens, estimatedOutputTokens, hasCacheTokens, enableLog, "message.usage", lowQuality)
				// 写入推断的 cache_read_input_tokens（仅当字段不存在时）
				if inferredCacheRead > 0 {
					if _, exists := usage["cache_read_input_tokens"]; !exists {
						usage["cache_read_input_tokens"] = inferredCacheRead
						if enableLog {
							log.Printf("[Messages-Stream-Token] message.usage: 写入推断的 cache_read_input_tokens=%d", inferredCacheRead)
						}
					}
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
	}

	return result.String()
}

// PatchMessageStartInputTokensIfNeeded 在首个 message_start 事件中尽早补全 input_tokens。
//
// 部分客户端（例如终端工具）只读取首个 usage 来累计 prompt tokens；如果 message_start 的 input_tokens 为 0/极小值，
// 即便后续顶层 usage 给出正确值，也可能导致累计失败。
func PatchMessageStartInputTokensIfNeeded(event string, requestBody []byte, needInputPatch bool, usageData CollectedUsageData, enableLog bool, lowQuality bool) string {
	if !IsMessageStartEvent(event) {
		return event
	}
	if !HasEventWithUsage(event) {
		return event
	}

	hasCacheTokens := usageData.CacheCreationInputTokens > 0 ||
		usageData.CacheReadInputTokens > 0 ||
		usageData.CacheCreation5mInputTokens > 0 ||
		usageData.CacheCreation1hInputTokens > 0

	// 仅在 input_tokens 明显异常时提前补齐；缓存命中场景不应强行补 input_tokens（除非上游返回 nil）
	// 低质量渠道模式下，即使 input_tokens >= 10 也需要进行偏差检测
	if !lowQuality && !needInputPatch && (hasCacheTokens || usageData.InputTokens >= 10) {
		return event
	}

	estimatedInputTokens := utils.EstimateRequestTokens(requestBody)
	if estimatedInputTokens <= 0 {
		return event
	}

	return PatchTokensInEvent(event, estimatedInputTokens, 0, hasCacheTokens, enableLog, lowQuality)
}

// patchUsageFieldsWithLog 修补 usage 对象中的 token 字段
// lowQuality 模式：偏差 > 5% 时使用本地估算值
func patchUsageFieldsWithLog(usage map[string]interface{}, estimatedInput, estimatedOutput int, hasCacheTokens bool, enableLog bool, location string, lowQuality bool) {
	originalInput := usage["input_tokens"]
	originalOutput := usage["output_tokens"]
	inputPatched := false
	outputPatched := false

	cacheCreation, _ := usage["cache_creation_input_tokens"].(float64)
	cacheRead, _ := usage["cache_read_input_tokens"].(float64)
	cacheCreation5m, _ := usage["cache_creation_5m_input_tokens"].(float64)
	cacheCreation1h, _ := usage["cache_creation_1h_input_tokens"].(float64)
	cacheTTL, _ := usage["cache_ttl"].(string)

	// 低质量渠道模式：偏差 > 5% 时使用本地估算值
	if lowQuality {
		if v, ok := usage["input_tokens"].(float64); ok && estimatedInput > 0 {
			currentInput := int(v)
			if currentInput > 0 {
				deviation := float64(abs(currentInput-estimatedInput)) / float64(estimatedInput)
				if deviation > 0.05 {
					usage["input_tokens"] = estimatedInput
					inputPatched = true
					if enableLog {
						log.Printf("[Messages-Stream-Token-LowQuality] %s: input_tokens %d -> %d (偏差 %.1f%% > 5%%)",
							location, currentInput, estimatedInput, deviation*100)
					}
				} else if enableLog {
					log.Printf("[Messages-Stream-Token-LowQuality] %s: input_tokens %d ≈ %d (偏差 %.1f%% ≤ 5%%, 保留上游值)",
						location, currentInput, estimatedInput, deviation*100)
				}
			}
		} else if enableLog && estimatedInput > 0 {
			log.Printf("[Messages-Stream-Token-LowQuality] %s: input_tokens=%v (上游无效值, 本地估算=%d)",
				location, usage["input_tokens"], estimatedInput)
		}
		if v, ok := usage["output_tokens"].(float64); ok && estimatedOutput > 0 {
			currentOutput := int(v)
			if currentOutput > 0 {
				deviation := float64(abs(currentOutput-estimatedOutput)) / float64(estimatedOutput)
				if deviation > 0.05 {
					usage["output_tokens"] = estimatedOutput
					outputPatched = true
					if enableLog {
						log.Printf("[Messages-Stream-Token-LowQuality] %s: output_tokens %d -> %d (偏差 %.1f%% > 5%%)",
							location, currentOutput, estimatedOutput, deviation*100)
					}
				} else if enableLog {
					log.Printf("[Messages-Stream-Token-LowQuality] %s: output_tokens %d ≈ %d (偏差 %.1f%% ≤ 5%%, 保留上游值)",
						location, currentOutput, estimatedOutput, deviation*100)
				}
			}
		} else if enableLog && estimatedOutput > 0 {
			log.Printf("[Messages-Stream-Token-LowQuality] %s: output_tokens=%v (上游无效值, 本地估算=%d)",
				location, usage["output_tokens"], estimatedOutput)
		}
	}

	// 常规修补逻辑（非 lowQuality 模式或 lowQuality 模式下未修补的情况）
	if !inputPatched {
		if v, ok := usage["input_tokens"].(float64); ok {
			currentInput := int(v)
			if !hasCacheTokens && ((currentInput <= 1) || (estimatedInput > currentInput && estimatedInput > 1)) {
				usage["input_tokens"] = estimatedInput
				inputPatched = true
			}
		} else if usage["input_tokens"] == nil && estimatedInput > 0 {
			// input_tokens 为 nil 时，用收集到的值修补
			usage["input_tokens"] = estimatedInput
			inputPatched = true
		}
	}

	if !outputPatched && estimatedOutput > 0 {
		if v, ok := usage["output_tokens"].(float64); ok {
			currentOutput := int(v)
			if currentOutput <= 1 || (estimatedOutput > currentOutput && estimatedOutput > 1) {
				usage["output_tokens"] = estimatedOutput
				outputPatched = true
			}
		}
	}

	if enableLog {
		if inputPatched || outputPatched {
			log.Printf("[Messages-Stream-Token-Patch] %s: InputTokens=%v -> %v, OutputTokens=%v -> %v",
				location, originalInput, usage["input_tokens"], originalOutput, usage["output_tokens"])
		}
		log.Printf("[Messages-Stream-Token] %s: InputTokens=%v, OutputTokens=%v, CacheCreationInputTokens=%.0f, CacheReadInputTokens=%.0f, CacheCreation5m=%.0f, CacheCreation1h=%.0f, CacheTTL=%s",
			location, usage["input_tokens"], usage["output_tokens"], cacheCreation, cacheRead, cacheCreation5m, cacheCreation1h, cacheTTL)
	}
}

// abs 返回整数的绝对值
func abs(x int) int {
	if x < 0 {
		return -x
	}
	return x
}

// BuildStreamErrorEvent 构建流错误 SSE 事件
func BuildStreamErrorEvent(err error) string {
	errorEvent := map[string]interface{}{
		"type": "error",
		"error": map[string]interface{}{
			"type":    "stream_error",
			"message": fmt.Sprintf("Stream processing error: %v", err),
		},
	}
	eventJSON, _ := json.Marshal(errorEvent)
	return fmt.Sprintf("event: error\ndata: %s\n\n", eventJSON)
}

// BuildUsageEvent 构建带 usage 的 message_delta SSE 事件
func BuildUsageEvent(requestBody []byte, outputText string) string {
	inputTokens := utils.EstimateRequestTokens(requestBody)
	outputTokens := utils.EstimateTokens(outputText)

	event := map[string]interface{}{
		"type": "message_delta",
		"usage": map[string]int{
			"input_tokens":  inputTokens,
			"output_tokens": outputTokens,
		},
	}
	eventJSON, _ := json.Marshal(event)
	return fmt.Sprintf("event: message_delta\ndata: %s\n\n", eventJSON)
}

// IsMessageStartEvent 检测是否为 message_start 事件
func IsMessageStartEvent(event string) bool {
	return strings.Contains(event, "\"type\":\"message_start\"") ||
		strings.Contains(event, "\"type\": \"message_start\"")
}

// PatchMessageStartEvent 修补 message_start 事件中的 id 和 model 字段
func PatchMessageStartEvent(event string, requestModel string, rewriteModel bool, enableLog bool) string {
	if !IsMessageStartEvent(event) {
		return event
	}

	var result strings.Builder
	lines := strings.Split(event, "\n")
	patched := false

	for _, line := range lines {
		if !strings.HasPrefix(line, "data: ") {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		jsonStr := strings.TrimPrefix(line, "data: ")
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		msg, ok := data["message"].(map[string]interface{})
		if !ok {
			result.WriteString(line)
			result.WriteString("\n")
			continue
		}

		// 补全空 id
		if id, _ := msg["id"].(string); id == "" {
			msg["id"] = fmt.Sprintf("msg_%s", uuid.New().String())
			patched = true
			if enableLog {
				log.Printf("[Messages-Stream-Patch] 补全空 message.id: %s", msg["id"])
			}
		}

		// 检查 model 一致性（仅在配置启用时改写）
		if rewriteModel {
			if responseModel, _ := msg["model"].(string); responseModel != "" && requestModel != "" && responseModel != requestModel {
				msg["model"] = requestModel
				patched = true
				if enableLog {
					log.Printf("[Messages-Stream-Patch] 改写 message.model: %s -> %s", responseModel, requestModel)
				}
			}
		}

		if patched {
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

// IsMessageStopEvent 检测是否为 message_stop 事件
func IsMessageStopEvent(event string) bool {
	if strings.Contains(event, "event: message_stop") {
		return true
	}

	for _, line := range strings.Split(event, "\n") {
		if !strings.HasPrefix(line, "data: ") {
			continue
		}
		jsonStr := strings.TrimPrefix(line, "data: ")

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		if data["type"] == "message_stop" {
			return true
		}
	}
	return false
}

// IsMessageDeltaEvent 检测是否为 message_delta 事件
func IsMessageDeltaEvent(event string) bool {
	if strings.Contains(event, "event: message_delta") {
		return true
	}
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}
		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}
		if data["type"] == "message_delta" {
			return true
		}
	}
	return false
}

// ExtractInputTokensFromEvent 从 SSE 事件中提取 input_tokens
// 支持 message_start 事件的 message.usage.input_tokens 和顶层 usage.input_tokens
func ExtractInputTokensFromEvent(event string) int {
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		// 检查 message.usage.input_tokens (message_start 事件)
		if msg, ok := data["message"].(map[string]interface{}); ok {
			if usage, ok := msg["usage"].(map[string]interface{}); ok {
				if v, ok := usage["input_tokens"].(float64); ok && v > 0 {
					return int(v)
				}
			}
		}

		// 检查顶层 usage.input_tokens (message_delta 事件)
		if usage, ok := data["usage"].(map[string]interface{}); ok {
			if v, ok := usage["input_tokens"].(float64); ok && v > 0 {
				return int(v)
			}
		}
	}
	return 0
}

// ExtractTextFromEvent 从 SSE 事件中提取文本内容
func ExtractTextFromEvent(event string, buf *bytes.Buffer) {
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		// Claude SSE: delta.text
		if delta, ok := data["delta"].(map[string]interface{}); ok {
			if text, ok := delta["text"].(string); ok {
				buf.WriteString(text)
			}
			if partialJSON, ok := delta["partial_json"].(string); ok {
				buf.WriteString(partialJSON)
			}
		}

		// content_block_start 中的初始文本
		if cb, ok := data["content_block"].(map[string]interface{}); ok {
			if text, ok := cb["text"].(string); ok {
				buf.WriteString(text)
			}
		}
	}
}

// ExtractThinkingFromEvent 从 SSE 事件中提取 thinking 内容
func ExtractThinkingFromEvent(event string, buf *bytes.Buffer) {
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		if cb, ok := data["content_block"].(map[string]interface{}); ok {
			if cbType, _ := cb["type"].(string); cbType == "thinking" || cbType == "redacted_thinking" {
				if thinking, ok := cb["thinking"].(string); ok {
					buf.WriteString(thinking)
				}
			}
		}

		if delta, ok := data["delta"].(map[string]interface{}); ok {
			if deltaType, _ := delta["type"].(string); deltaType == "thinking_delta" || deltaType == "redacted_thinking_delta" {
				if thinking, ok := delta["thinking"].(string); ok {
					buf.WriteString(thinking)
				}
				if text, ok := delta["text"].(string); ok {
					buf.WriteString(text)
				}
			}
		}
	}
}

// DetectStreamBlacklistError 检测 SSE error 事件中是否包含应拉黑 Key 的错误
// 返回 (reason, message)，reason 非空表示应拉黑
func DetectStreamBlacklistError(event string) (reason string, message string) {
	// 检查是否为 error 事件
	isErrorEvent := false
	for _, line := range strings.Split(event, "\n") {
		if strings.HasPrefix(line, "event: ") {
			if strings.TrimPrefix(line, "event: ") == "error" {
				isErrorEvent = true
			}
			break
		}
	}

	// 即使不是显式的 event: error，也检查 data 中的 type == "error"
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		// Claude 格式: {"type":"error","error":{"type":"authentication_error","message":"..."}}
		if dataType, _ := data["type"].(string); dataType == "error" || isErrorEvent {
			if errObj, ok := data["error"].(map[string]interface{}); ok {
				errType, _ := errObj["type"].(string)
				errMsg, _ := errObj["message"].(string)
				errCode, _ := errObj["code"].(string)

				typeLower := strings.ToLower(errType)

				// 认证错误
				if typeLower == "authentication_error" || typeLower == "invalid_api_key" {
					return "authentication_error", truncateMsg(errMsg)
				}
				if isAuthenticationMessage(errMsg) {
					return "authentication_error", truncateMsg(errMsg)
				}
				// 权限错误
				if typeLower == "permission_error" || typeLower == "permission_denied" {
					return "permission_error", truncateMsg(errMsg)
				}
				if isPermissionMessage(errMsg) {
					return "permission_error", truncateMsg(errMsg)
				}
				// 余额不足（明确的错误类型或错误码）
				if typeLower == "insufficient_balance" || typeLower == "insufficient_quota" || typeLower == "billing_error" {
					return "insufficient_balance", truncateMsg(errMsg)
				}
				// 已知的余额不足错误码（如 Kimi 的 1113）
				if isInsufficientBalanceCode(errCode) || isInsufficientBalanceMessage(errMsg) {
					return "insufficient_balance", truncateMsg(errMsg)
				}
			}
			if errStr, ok := data["error"].(string); ok {
				if isAuthenticationMessage(errStr) {
					return "authentication_error", truncateMsg(errStr)
				}
				if isPermissionMessage(errStr) {
					return "permission_error", truncateMsg(errStr)
				}
				if isInsufficientBalanceMessage(errStr) {
					return "insufficient_balance", truncateMsg(errStr)
				}
			}
			if msg, ok := data["message"].(string); ok {
				if isAuthenticationMessage(msg) {
					return "authentication_error", truncateMsg(msg)
				}
				if isPermissionMessage(msg) {
					return "permission_error", truncateMsg(msg)
				}
				if isInsufficientBalanceMessage(msg) {
					return "insufficient_balance", truncateMsg(msg)
				}
			}
		}
	}
	return "", ""
}

// isInsufficientBalanceCode 检查错误码是否为已知的余额不足代码
func isInsufficientBalanceCode(code string) bool {
	knownCodes := []string{
		"1113",                 // Kimi: 余额不足或无可用资源包
		"INSUFFICIENT_BALANCE", // 通用余额不足
		"INSUFFICIENT_QUOTA",   // 通用额度不足
		"USAGE_LIMIT_EXCEEDED", // 当日/周期额度耗尽
		"DAILY_LIMIT_EXCEEDED", // 当日额度耗尽
	}
	for _, c := range knownCodes {
		if strings.EqualFold(code, c) {
			return true
		}
	}
	return false
}

// truncateMsg 截断消息（最多200字符）
func truncateMsg(msg string) string {
	if len(msg) > 200 {
		return msg[:200]
	}
	return msg
}

// extractSSEEventInfo 从 SSE 事件中提取事件类型、block 索引和 block 类型
func extractSSEEventInfo(event string) (eventType string, blockIndex int, blockType string) {
	for _, line := range strings.Split(event, "\n") {
		jsonStr, ok := extractSSEJSONLine(line)
		if !ok {
			continue
		}

		var data map[string]interface{}
		if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
			continue
		}

		eventType, _ = data["type"].(string)
		if idx, ok := data["index"].(float64); ok {
			blockIndex = int(idx)
		}

		// 从 content_block 中提取类型
		if cb, ok := data["content_block"].(map[string]interface{}); ok {
			blockType, _ = cb["type"].(string)
		}

		return
	}
	return
}

// truncateForLog 截断字符串用于日志输出
func truncateForLog(s string, maxLen int) string {
	if len(s) <= maxLen {
		return s
	}
	return s[:maxLen] + "..."
}
