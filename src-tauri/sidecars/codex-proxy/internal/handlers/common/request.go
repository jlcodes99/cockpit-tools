// Package common 提供 handlers 模块的公共功能
package common

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"net/http/httptrace"
	"regexp"
	"sort"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/httpclient"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/logger"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

const MaxUpstreamResponseLogBytes = 1024 * 1024

// logToFile 向日志文件写入一行（原始格式）。
// 所有请求/响应日志都通过此函数写入文件，确保文件始终包含完整日志。
func logToFile(format string, args ...interface{}) {
	logger.RawFileLog().Printf(format, args...)
}

type LimitedLogBuffer struct {
	buf       bytes.Buffer
	limit     int
	truncated bool
	total     int
}

func NewLimitedLogBuffer(limit int) *LimitedLogBuffer {
	if limit <= 0 {
		limit = MaxUpstreamResponseLogBytes
	}
	return &LimitedLogBuffer{limit: limit}
}

func (b *LimitedLogBuffer) Write(p []byte) (int, error) {
	if b == nil {
		return len(p), nil
	}
	b.total += len(p)
	remaining := b.limit - b.buf.Len()
	if remaining <= 0 {
		b.truncated = true
		return len(p), nil
	}
	if len(p) > remaining {
		b.buf.Write(p[:remaining])
		b.truncated = true
		return len(p), nil
	}
	b.buf.Write(p)
	return len(p), nil
}

func (b *LimitedLogBuffer) WriteString(s string) (int, error) {
	return b.Write([]byte(s))
}

func (b *LimitedLogBuffer) Len() int {
	if b == nil {
		return 0
	}
	return b.buf.Len()
}

func (b *LimitedLogBuffer) Bytes() []byte {
	if b == nil {
		return nil
	}
	return b.buf.Bytes()
}

func (b *LimitedLogBuffer) String() string {
	if b == nil {
		return ""
	}
	result := b.buf.String()
	if b.truncated {
		result += fmt.Sprintf("\n...[truncated upstream stream log at %d/%d bytes]", b.limit, b.total)
	}
	return result
}

// RequestLifecycleTrace 用于记录上游 HTTP 请求生命周期关键节点。
type RequestLifecycleTrace struct {
	OnConnected         func()
	OnFirstResponseByte func()
}

// ReadRequestBody 读取并验证请求体大小
// 返回: (bodyBytes, error)
// 如果请求体过大，会自动返回 413 错误并排空剩余数据
func ReadRequestBody(c *gin.Context, maxBodySize int64) ([]byte, error) {
	limitedReader := io.LimitReader(c.Request.Body, maxBodySize+1)
	bodyBytes, err := io.ReadAll(limitedReader)
	if err != nil {
		c.JSON(400, gin.H{"error": "Failed to read request body"})
		return nil, err
	}

	if int64(len(bodyBytes)) > maxBodySize {
		// 排空剩余请求体，避免 keep-alive 连接污染
		io.Copy(io.Discard, c.Request.Body)
		c.JSON(413, gin.H{"error": fmt.Sprintf("Request body too large, maximum size is %d MB", maxBodySize/1024/1024)})
		return nil, fmt.Errorf("request body too large")
	}

	// 恢复请求体供后续使用
	c.Request.Body = io.NopCloser(bytes.NewReader(bodyBytes))
	return bodyBytes, nil
}

// RestoreRequestBody 恢复请求体供后续使用
func RestoreRequestBody(c *gin.Context, bodyBytes []byte) {
	c.Request.Body = io.NopCloser(bytes.NewReader(bodyBytes))
}

// PassthroughResponse 直接将上游响应转发给客户端，不在内存中整包缓存。
func PassthroughResponse(c *gin.Context, resp *http.Response) error {
	utils.ForwardResponseHeaders(resp.Header, c.Writer)
	c.Status(resp.StatusCode)
	_, err := io.Copy(c.Writer, resp.Body)
	return err
}

// PassthroughJSONResponse 在透传响应给客户端的同时，用流式 Decoder 尝试解析 JSON。
// 解析失败时会继续排空剩余响应体，确保客户端仍收到完整响应。
func PassthroughJSONResponse(c *gin.Context, resp *http.Response, target interface{}) error {
	if target == nil {
		return PassthroughResponse(c, resp)
	}

	utils.ForwardResponseHeaders(resp.Header, c.Writer)
	c.Status(resp.StatusCode)

	tee := io.TeeReader(resp.Body, c.Writer)
	decoder := json.NewDecoder(tee)
	if err := decoder.Decode(target); err != nil {
		if _, copyErr := io.Copy(c.Writer, resp.Body); copyErr != nil {
			return copyErr
		}
		return err
	}

	_, err := io.Copy(c.Writer, resp.Body)
	return err
}

func LogUpstreamResponseHeaders(resp *http.Response, envCfg *config.EnvConfig, apiType string) {
	if !envCfg.EnableResponseLogs || !envCfg.IsDevelopment() || resp == nil {
		return
	}

	respHeaders := make(map[string]string)
	for key, values := range resp.Header {
		if len(values) > 0 {
			respHeaders[key] = values[0]
		}
	}
	respHeadersJSON, _ := json.MarshalIndent(respHeaders, "", "  ")
	log.Printf("[%s-Response] 响应头:\n%s", apiType, string(respHeadersJSON))
	rawHeadersJSON, _ := json.Marshal(respHeaders)
	logToFile("[%s-Response] 响应头:\n%s", apiType, string(rawHeadersJSON))
}

func LogUpstreamResponseBody(bodyBytes []byte, envCfg *config.EnvConfig, apiType string) {
	if !envCfg.EnableResponseLogs || !envCfg.IsDevelopment() {
		return
	}

	log.Printf("[%s-Response] 响应体:\n%s", apiType, utils.FormatJSONBytesForLog(bodyBytes, 0))
	logToFile("[%s-Response] 响应体:\n%s", apiType, utils.FormatJSONBytesRaw(bodyBytes))
}

func LogUpstreamResponse(resp *http.Response, bodyBytes []byte, envCfg *config.EnvConfig, apiType string) {
	LogUpstreamResponseHeaders(resp, envCfg, apiType)
	LogUpstreamResponseBody(bodyBytes, envCfg, apiType)
}

// SendRequest 发送 HTTP 请求到上游
// isStream: 是否为流式请求（流式请求使用无超时客户端）
// apiType: 接口类型（Messages/Responses/Gemini），用于日志标签前缀
func SendRequest(req *http.Request, upstream *config.UpstreamConfig, envCfg *config.EnvConfig, isStream bool, apiType string) (*http.Response, error) {
	return SendRequestWithLifecycleTrace(req, upstream, envCfg, isStream, apiType, nil)
}

// SendRequestWithLifecycleTrace 发送 HTTP 请求到上游，并可记录连接取得与首个响应字节时间。
func SendRequestWithLifecycleTrace(req *http.Request, upstream *config.UpstreamConfig, envCfg *config.EnvConfig, isStream bool, apiType string, lifecycleTrace *RequestLifecycleTrace) (*http.Response, error) {
	clientManager := httpclient.GetManager()

	var client *http.Client
	if isStream {
		client = clientManager.GetStreamClient(upstream.InsecureSkipVerify, upstream.ProxyURL)
	} else {
		timeout := time.Duration(envCfg.RequestTimeout) * time.Millisecond
		client = clientManager.GetStandardClient(timeout, upstream.InsecureSkipVerify, upstream.ProxyURL)
	}

	if upstream.InsecureSkipVerify && envCfg.EnableRequestLogs {
		log.Printf("[%s-Request-TLS] 警告: 正在跳过对 %s 的TLS证书验证", apiType, req.URL.String())
		logToFile("[%s-Request-TLS] 警告: 正在跳过对 %s 的TLS证书验证", apiType, req.URL.String())
	}

	if envCfg.EnableRequestLogs {
		log.Printf("[%s-Request-URL] 实际请求URL: %s", apiType, req.URL.String())
		log.Printf("[%s-Request-Method] 请求方法: %s", apiType, req.Method)
		logToFile("[%s-Request-URL] 实际请求URL: %s", apiType, req.URL.String())
		logToFile("[%s-Request-Method] 请求方法: %s", apiType, req.Method)
		if upstream.ProxyURL != "" {
			// 对代理 URL 进行脱敏处理，避免泄露凭证
			redactedProxyURL := utils.RedactURLCredentials(upstream.ProxyURL)
			log.Printf("[%s-Request-Proxy] 使用代理: %s", apiType, redactedProxyURL)
			logToFile("[%s-Request-Proxy] 使用代理: %s", apiType, redactedProxyURL)
		}
		if envCfg.IsDevelopment() {
			logRequestDetails(req, envCfg, apiType)
		}
	}

	req = withLifecycleTrace(req, lifecycleTrace)
	return client.Do(req)
}

func withLifecycleTrace(req *http.Request, lifecycleTrace *RequestLifecycleTrace) *http.Request {
	if lifecycleTrace == nil || (lifecycleTrace.OnConnected == nil && lifecycleTrace.OnFirstResponseByte == nil) {
		return req
	}

	trace := &httptrace.ClientTrace{
		GotConn: func(_ httptrace.GotConnInfo) {
			if lifecycleTrace.OnConnected != nil {
				lifecycleTrace.OnConnected()
			}
		},
		GotFirstResponseByte: func() {
			if lifecycleTrace.OnFirstResponseByte != nil {
				lifecycleTrace.OnFirstResponseByte()
			}
		},
	}
	return req.WithContext(httptrace.WithClientTrace(req.Context(), trace))
}

// logRequestDetails 记录请求详情（仅开发模式）
// apiType: 接口类型（Messages/Responses/Gemini），用于日志标签前缀
func logRequestDetails(req *http.Request, envCfg *config.EnvConfig, apiType string) {
	// 对请求头做敏感信息脱敏
	reqHeaders := make(map[string]string)
	for key, values := range req.Header {
		if len(values) > 0 {
			reqHeaders[key] = values[0]
		}
	}
	maskedReqHeaders := utils.MaskSensitiveHeaders(reqHeaders)
	reqHeadersJSON, _ := json.MarshalIndent(maskedReqHeaders, "", "  ")
	log.Printf("[%s-Request-Headers] 实际请求头:\n%s", apiType, string(reqHeadersJSON))
	rawReqHeadersJSON, _ := json.Marshal(maskedReqHeaders)
	logToFile("[%s-Request-Headers] 实际请求头:\n%s", apiType, string(rawReqHeadersJSON))

	if req.Body != nil {
		contentType := req.Header.Get("Content-Type")
		if strings.HasPrefix(strings.ToLower(contentType), "multipart/form-data") {
			log.Printf("[%s-Request-Body] 实际请求体: [multipart/form-data omitted]", apiType)
			logToFile("[%s-Request-Body] 实际请求体: [multipart/form-data omitted]", apiType)
			return
		}
		bodyBytes, err := io.ReadAll(req.Body)
		if err == nil {
			req.Body = io.NopCloser(bytes.NewReader(bodyBytes))
			log.Printf("[%s-Request-Body] 实际请求体:\n%s", apiType, utils.FormatJSONBytesForLog(bodyBytes, 0))
			logToFile("[%s-Request-Body] 实际请求体:\n%s", apiType, utils.FormatJSONBytesRaw(bodyBytes))
		}
	}
}

// LogOriginalRequest 记录原始请求信息
func LogOriginalRequest(c *gin.Context, bodyBytes []byte, envCfg *config.EnvConfig, apiType string) {
	if !envCfg.EnableRequestLogs {
		return
	}

	log.Printf("[Request-Receive] 收到%s请求: %s %s", apiType, c.Request.Method, c.Request.URL.Path)
	logToFile("[Request-Receive] 收到%s请求: %s %s", apiType, c.Request.Method, c.Request.URL.Path)

	if envCfg.IsDevelopment() {
		contentType := c.GetHeader("Content-Type")
		if strings.HasPrefix(strings.ToLower(contentType), "multipart/form-data") {
			log.Printf("[Request-OriginalBody] 原始请求体: [multipart/form-data omitted]")
			logToFile("[Request-OriginalBody] 原始请求体: [multipart/form-data omitted]")
		} else {
			log.Printf("[Request-OriginalBody] 原始请求体:\n%s", utils.FormatJSONBytesForLog(bodyBytes, 0))
			logToFile("[Request-OriginalBody] 原始请求体:\n%s", utils.FormatJSONBytesRaw(bodyBytes))
		}

		sanitizedHeaders := make(map[string]string)
		for key, values := range c.Request.Header {
			if len(values) > 0 {
				sanitizedHeaders[key] = values[0]
			}
		}
		maskedHeaders := utils.MaskSensitiveHeaders(sanitizedHeaders)
		headersJSON, _ := json.MarshalIndent(maskedHeaders, "", "  ")
		log.Printf("[Request-OriginalHeaders] 原始请求头:\n%s", string(headersJSON))
		rawHeadersJSON, _ := json.Marshal(maskedHeaders)
		logToFile("[Request-OriginalHeaders] 原始请求头:\n%s", string(rawHeadersJSON))
	}
}

// AreAllKeysSuspended 检查渠道的所有 Key 是否都处于熔断状态
// 用于判断是否需要启用强制探测模式
func AreAllKeysSuspended(metricsManager *metrics.MetricsManager, baseURL string, apiKeys []string, serviceType string) bool {
	if len(apiKeys) == 0 {
		return false
	}

	for _, apiKey := range apiKeys {
		if !metricsManager.ShouldSuspendKey(baseURL, apiKey, serviceType) {
			return false
		}
	}
	return true
}

// RemoveEmptySignatures 移除请求体中 messages[*].content[*].signature 的空值
// 用于预防 Claude API 返回 400 错误
// 仅处理已知路径：messages 数组中各消息的 content 数组中的 signature 字段
// enableLog: 是否输出日志（由 envCfg.EnableRequestLogs 控制）
// apiType: 接口类型（Messages/Responses/Gemini），用于日志标签前缀
func RemoveEmptySignatures(bodyBytes []byte, enableLog bool, apiType string) ([]byte, bool) {
	decoder := json.NewDecoder(bytes.NewReader(bodyBytes))
	decoder.UseNumber() // 保留数字精度

	var data map[string]interface{}
	if err := decoder.Decode(&data); err != nil {
		return bodyBytes, false
	}

	modified, removedCount := removeEmptySignaturesInMessages(data)
	if !modified {
		return bodyBytes, false
	}

	if enableLog && removedCount > 0 {
		log.Printf("[%s-Preprocess] 已移除 %d 个空 signature 字段", apiType, removedCount)
	}

	// 使用 Encoder 并禁用 HTML 转义，保持原始格式
	newBytes, err := utils.MarshalJSONNoEscape(data)
	if err != nil {
		return bodyBytes, false
	}
	return newBytes, true
}

// removeEmptySignaturesInMessages 仅处理 messages[*].content[*].signature 路径
// 返回 (是否有修改, 移除的字段数)
func removeEmptySignaturesInMessages(data map[string]interface{}) (bool, int) {
	modified := false
	removedCount := 0

	messages, ok := data["messages"].([]interface{})
	if !ok {
		return false, 0
	}

	for _, msg := range messages {
		msgMap, ok := msg.(map[string]interface{})
		if !ok {
			continue
		}

		content, ok := msgMap["content"].([]interface{})
		if !ok {
			continue
		}

		for _, block := range content {
			blockMap, ok := block.(map[string]interface{})
			if !ok {
				continue
			}

			if sig, exists := blockMap["signature"]; exists {
				if sig == nil {
					delete(blockMap, "signature")
					modified = true
					removedCount++
				} else if str, isStr := sig.(string); isStr && str == "" {
					delete(blockMap, "signature")
					modified = true
					removedCount++
				}
			}
		}
	}

	return modified, removedCount
}

// SanitizeMalformedThinkingBlocks 清理 messages[*].content[*] 中的 thinking 相关字段
// 策略：
// 1) 仅移除畸形的 type=thinking 内容块（避免上游严格校验导致 400）
// 2) 保留合法的 thinking 内容块（兼容 Claude extended thinking 回传）
// 3) 移除非 thinking 块里的残留 thinking 字段
// 返回 (新字节, 是否修改)
func SanitizeMalformedThinkingBlocks(bodyBytes []byte, enableLog bool, apiType string) ([]byte, bool) {
	decoder := json.NewDecoder(bytes.NewReader(bodyBytes))
	decoder.UseNumber() // 保留数字精度

	var data map[string]interface{}
	if err := decoder.Decode(&data); err != nil {
		return bodyBytes, false
	}

	modified, removedBlocks, removedMsgs := sanitizeMalformedThinkingBlocksInMessages(data)
	if !modified {
		return bodyBytes, false
	}

	if enableLog {
		if removedMsgs > 0 {
			log.Printf("[%s-Preprocess] 已移除 %d 个 thinking 内容块，并删除 %d 条清理后 content 为空的 assistant 消息", apiType, removedBlocks, removedMsgs)
		} else {
			log.Printf("[%s-Preprocess] 已移除 %d 个 thinking 内容块", apiType, removedBlocks)
		}
	}

	newBytes, err := utils.MarshalJSONNoEscape(data)
	if err != nil {
		return bodyBytes, false
	}
	return newBytes, true
}

func sanitizeMalformedThinkingBlocksInMessages(data map[string]interface{}) (bool, int, int) {
	messages, ok := data["messages"].([]interface{})
	if !ok {
		return false, 0, 0
	}

	modified := false
	removedBlocks := 0
	removedMsgs := 0
	sanitizedMessages := make([]interface{}, 0, len(messages))

	for _, msg := range messages {
		msgMap, ok := msg.(map[string]interface{})
		if !ok {
			sanitizedMessages = append(sanitizedMessages, msg)
			continue
		}

		role, _ := msgMap["role"].(string)

		switch content := msgMap["content"].(type) {
		case []interface{}:
			newContent := make([]interface{}, 0, len(content))
			removedInCurrentMessage := 0
			messageModified := false

			for _, block := range content {
				blockMap, ok := block.(map[string]interface{})
				if !ok {
					newContent = append(newContent, block)
					continue
				}

				blockModified, removeBlock := sanitizeThinkingInContentBlock(blockMap)
				if blockModified {
					modified = true
					messageModified = true
				}
				if removeBlock {
					removedBlocks++
					removedInCurrentMessage++
					continue
				}
				newContent = append(newContent, blockMap)
			}

			if messageModified {
				msgMap["content"] = newContent
			}

			if removedInCurrentMessage > 0 && len(newContent) == 0 && role == "assistant" {
				removedMsgs++
				msgMap["content"] = []interface{}{} // 保留消息骨架，清空 content，不删整条消息
			}

		case map[string]interface{}:
			blockModified, removeBlock := sanitizeThinkingInContentBlock(content)
			if blockModified {
				modified = true
			}
			if removeBlock {
				removedBlocks++
				if role == "assistant" {
					removedMsgs++
					continue
				}
				msgMap["content"] = []interface{}{}
			} else if blockModified {
				msgMap["content"] = content
			}
		}

		sanitizedMessages = append(sanitizedMessages, msgMap)
	}

	if modified {
		data["messages"] = sanitizedMessages
	}

	return modified, removedBlocks, removedMsgs
}

func sanitizeThinkingInContentBlock(block map[string]interface{}) (modified bool, removeBlock bool) {
	blockType, _ := block["type"].(string)
	if blockType == "thinking" {
		// 仅移除畸形 thinking block：
		// - 缺少 thinking 字段
		// - thinking 不是非空字符串
		thinking, hasThinking := block["thinking"]
		if !hasThinking {
			return true, true
		}
		thinkingText, ok := thinking.(string)
		if !ok || strings.TrimSpace(thinkingText) == "" {
			return true, true
		}

		// 保留合法 thinking block（含 signature 与否都透传）
		return false, false
	}

	if _, hasThinking := block["thinking"]; hasThinking {
		// 非 thinking block 里的 thinking 字段对上游无意义且可能触发校验错误，直接移除
		delete(block, "thinking")
		return true, false
	}

	return false, false
}

// NormalizeMetadataUserID 规范化 metadata.user_id 字段
// Claude Code v2.1.78 将 user_id 从扁平字符串改为 JSON 对象字符串:
//
//	v2.1.77: "user_{device_id}_account_{uuid}_session_{sid}"
//	v2.1.78: '{"device_id":"...","account_uuid":"...","session_id":"..."}'
//
// 部分上游（如 anyrouter）对 user_id 做严格校验，不接受 JSON 对象格式。
// 此函数检测并转换为扁平字符串格式，确保上游兼容性。
func NormalizeMetadataUserID(bodyBytes []byte) []byte {
	decoder := json.NewDecoder(bytes.NewReader(bodyBytes))
	decoder.UseNumber() // 保留数字精度

	var data map[string]interface{}
	if err := decoder.Decode(&data); err != nil {
		return bodyBytes
	}

	metadata, ok := data["metadata"].(map[string]interface{})
	if !ok {
		return bodyBytes
	}

	userID, ok := metadata["user_id"].(string)
	if !ok || userID == "" {
		return bodyBytes
	}

	// 检测是否为 JSON 对象格式
	if !strings.HasPrefix(userID, "{") {
		return bodyBytes
	}

	// 尝试解析为通用 JSON 对象
	var parsed map[string]interface{}
	if err := json.Unmarshal([]byte(userID), &parsed); err != nil {
		return bodyBytes
	}

	// 如果是空对象，不改写
	if len(parsed) == 0 {
		return bodyBytes
	}

	// 动态拼接为 key_value 格式
	var parts []string
	// 优先处理 Claude Code 标准字段顺序
	if deviceID, ok := parsed["device_id"].(string); ok && deviceID != "" {
		parts = append(parts, "user_"+deviceID)
		if accountUUID, ok := parsed["account_uuid"].(string); ok && accountUUID != "" {
			parts = append(parts, "account_"+accountUUID)
		}
		if sessionID, ok := parsed["session_id"].(string); ok && sessionID != "" {
			parts = append(parts, "session_"+sessionID)
		}
	} else {
		// 非 Claude Code 格式，按字母序拼接所有字段
		keys := make([]string, 0, len(parsed))
		for k := range parsed {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			if v, ok := parsed[k].(string); ok && v != "" {
				parts = append(parts, k+"_"+v)
			}
		}
	}

	// 如果没有有效字段，不改写
	if len(parts) == 0 {
		return bodyBytes
	}

	flatUserID := strings.Join(parts, "_")
	metadata["user_id"] = flatUserID

	newBytes, err := utils.MarshalJSONNoEscape(data)
	if err != nil {
		return bodyBytes
	}
	return newBytes
}

// Deprecated: 使用 utils.ExtractUnifiedSessionID 替代。
// ExtractUserID 从请求体中提取 user_id（用于 Messages API）
func ExtractUserID(bodyBytes []byte) string {
	var req struct {
		Metadata struct {
			UserID string `json:"user_id"`
		} `json:"metadata"`
	}
	if err := json.Unmarshal(bodyBytes, &req); err == nil {
		return req.Metadata.UserID
	}
	return ""
}

// Deprecated: 使用 utils.ExtractUnifiedSessionID 替代。
// ExtractConversationID 从请求中提取对话标识（用于 Responses API）
func ExtractConversationID(c *gin.Context, bodyBytes []byte) string {
	return utils.ExtractUnifiedSessionID(c, bodyBytes)
}

// cchPattern 匹配 cch=xxx; 部分（包括前后空格）
var cchPattern = regexp.MustCompile(`\s*cch=[^;]*;\s*`)

// RemoveBillingHeaders 移除请求体 system 数组中第一个包含 cch= 的文本块的 cch 参数
// 仅移除 cch=xxx; 部分，保留其他计费参数（如 cc_version、cc_entrypoint）
// enableLog: 是否输出日志（由 envCfg.EnableRequestLogs 控制）
// apiType: 接口类型（Messages/Responses/Gemini），用于日志标签前缀
func RemoveBillingHeaders(bodyBytes []byte, enableLog bool, apiType string) ([]byte, bool) {
	decoder := json.NewDecoder(bytes.NewReader(bodyBytes))
	decoder.UseNumber() // 保留数字精度

	var data map[string]interface{}
	if err := decoder.Decode(&data); err != nil {
		return bodyBytes, false
	}

	systemArr, ok := data["system"].([]interface{})
	if !ok || len(systemArr) == 0 {
		return bodyBytes, false
	}

	modified := false
	for _, item := range systemArr {
		itemMap, ok := item.(map[string]interface{})
		if !ok {
			continue
		}

		text, ok := itemMap["text"].(string)
		if !ok || !strings.Contains(text, "cch=") {
			continue
		}

		// 移除 cch=xxx; 部分
		newText := cchPattern.ReplaceAllString(text, "")
		// 清理末尾多余空格
		newText = strings.TrimRight(newText, " ")
		itemMap["text"] = newText
		modified = true

		if enableLog {
			log.Printf("[%s-Preprocess] 已移除 system 文本块中的 cch 计费参数", apiType)
		}
		break // 只处理第一个匹配的元素
	}

	if !modified {
		return bodyBytes, false
	}

	newBytes, err := utils.MarshalJSONNoEscape(data)
	if err != nil {
		return bodyBytes, false
	}
	return newBytes, true
}
