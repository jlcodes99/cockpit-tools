// Package common 提供 handlers 模块的公共功能
package common

import (
	"encoding/json"
	"fmt"
	"log"
	"strings"

	"github.com/gin-gonic/gin"
)

// FailoverError 封装故障转移错误信息
type FailoverError struct {
	Status int
	Body   []byte
}

// ShouldRetryWithNextKey 判断是否应该使用下一个密钥重试
// 返回: (shouldFailover bool, isQuotaRelated bool)
//
// apiType: 接口类型（Messages/Responses/Gemini），用于日志标签前缀
// fuzzyMode: 启用时，所有非 2xx 错误都触发 failover（模糊处理错误类型）
//
// HTTP 状态码分类策略（非 fuzzy 模式）：
//   - 4xx 客户端错误：部分应触发 failover（密钥/配额问题）
//   - 5xx 服务端错误：应触发 failover（上游临时故障）
//   - 2xx/3xx：不应触发 failover（成功或重定向）
//
// isQuotaRelated 标记用于调度器优先级调整：
//   - true: 额度/配额相关，降低密钥优先级
//   - false: 临时错误，不影响优先级
func ShouldRetryWithNextKey(statusCode int, bodyBytes []byte, fuzzyMode bool, apiType string) (bool, bool) {
	log.Printf("[%s-Failover-Entry] ShouldRetryWithNextKey 入口: statusCode=%d, bodyLen=%d, fuzzyMode=%v",
		apiType, statusCode, len(bodyBytes), fuzzyMode)
	if fuzzyMode {
		return shouldRetryWithNextKeyFuzzy(statusCode, bodyBytes, apiType)
	}
	return shouldRetryWithNextKeyNormal(statusCode, bodyBytes, apiType)
}

// shouldRetryWithNextKeyFuzzy Fuzzy 模式：大多数非 2xx 错误都尝试 failover
// 同时检查消息体中的配额相关关键词，确保 403 + "预扣费额度" 等情况能正确识别
// 但对于内容审核错误，以及 4xx 下的 invalid_request、schema 校验失败等不可重试错误，即使在 Fuzzy 模式下也不应重试
func shouldRetryWithNextKeyFuzzy(statusCode int, bodyBytes []byte, apiType string) (bool, bool) {
	log.Printf("[%s-Failover-Fuzzy] 进入 Fuzzy 模式处理: statusCode=%d, bodyLen=%d", apiType, statusCode, len(bodyBytes))
	if statusCode >= 200 && statusCode < 300 {
		return false, false
	}

	// 内容审核类错误（sensitive_words_detected 等）任何状态码都不应 failover
	// 换渠道/换 Key 不会改变请求内容本身
	if len(bodyBytes) > 0 && isContentModerationError(bodyBytes) {
		log.Printf("[%s-Failover-Fuzzy] 检测到内容审核错误 (statusCode=%d)，不进行 failover", apiType, statusCode)
		return false, false
	}

	// 检查是否为参数校验类不可重试错误（invalid_request 等）
	// 仅对 4xx 客户端错误生效，5xx 服务端错误应始终允许 failover
	if statusCode >= 400 && statusCode < 500 && len(bodyBytes) > 0 {
		if isNonRetryableError(bodyBytes, apiType) {
			log.Printf("[%s-Failover-Fuzzy] 检测到不可重试错误 (statusCode=%d)，不进行 failover", apiType, statusCode)
			return false, false
		}
	}

	// 状态码直接标记为配额相关
	if statusCode == 402 || statusCode == 429 {
		log.Printf("[%s-Failover-Fuzzy] 状态码 %d 直接标记为配额相关", apiType, statusCode)
		return true, true
	}

	// 对于其他状态码，检查消息体是否包含配额相关关键词
	// 这样 403 + "预扣费额度" 消息 → isQuotaRelated=true
	if len(bodyBytes) > 0 {
		_, msgQuota := classifyByErrorMessage(bodyBytes, apiType)
		if msgQuota {
			log.Printf("[%s-Failover-Fuzzy] 消息体包含配额相关关键词，标记为配额相关", apiType)
			return true, true
		}
	}

	log.Printf("[%s-Failover-Fuzzy] Fuzzy 模式结果: shouldFailover=true, isQuotaRelated=false", apiType)
	return true, false
}

// shouldRetryWithNextKeyNormal 原有的精确错误分类逻辑
func shouldRetryWithNextKeyNormal(statusCode int, bodyBytes []byte, apiType string) (bool, bool) {
	// 内容审核类错误（sensitive_words_detected 等）任何状态码都不应 failover
	// 换渠道/换 Key 不会改变请求内容本身
	if len(bodyBytes) > 0 && isContentModerationError(bodyBytes) {
		log.Printf("[%s-Failover-Debug] 检测到内容审核错误 (statusCode=%d)，不进行 failover", apiType, statusCode)
		return false, false
	}

	// 检查是否为参数校验类不可重试错误（invalid_request 等）
	// 仅对 4xx 客户端错误生效，5xx 服务端错误应始终允许 failover
	if statusCode >= 400 && statusCode < 500 && len(bodyBytes) > 0 && isNonRetryableError(bodyBytes, apiType) {
		log.Printf("[%s-Failover-Debug] 检测到不可重试错误 (statusCode=%d)，不进行 failover", apiType, statusCode)
		return false, false
	}

	shouldFailover, isQuotaRelated := classifyByStatusCode(statusCode)

	log.Printf("[%s-Failover-Debug] shouldRetryWithNextKeyNormal: statusCode=%d, bodyLen=%d, shouldFailover=%v, isQuotaRelated=%v",
		apiType, statusCode, len(bodyBytes), shouldFailover, isQuotaRelated)

	if shouldFailover {
		// 如果状态码已标记为 quota 相关，直接返回
		if isQuotaRelated {
			return true, true
		}
		// 否则，仍检查消息体是否包含 quota 相关关键词
		// 这样 403 + "预扣费额度" 消息 → isQuotaRelated=true
		log.Printf("[%s-Failover-Debug] 调用 classifyByErrorMessage, body=%s", apiType, string(bodyBytes))
		_, msgQuota := classifyByErrorMessage(bodyBytes, apiType)
		log.Printf("[%s-Failover-Debug] classifyByErrorMessage 返回: msgQuota=%v", apiType, msgQuota)
		if msgQuota {
			return true, true
		}
		return true, false
	}

	// statusCode 不触发 failover 时，完全依赖消息体判断
	return classifyByErrorMessage(bodyBytes, apiType)
}

// classifyByStatusCode 基于 HTTP 状态码分类
func classifyByStatusCode(statusCode int) (bool, bool) {
	switch {
	// 认证/授权错误 (应 failover，非配额相关)
	case statusCode == 401:
		return true, false
	case statusCode == 403:
		return true, false

	// 配额/计费错误 (应 failover，配额相关)
	case statusCode == 402:
		return true, true
	case statusCode == 429:
		return true, true

	// 超时错误 (应 failover，非配额相关)
	case statusCode == 408:
		return true, false

	// 需要检查消息体的状态码 (交给第二层判断)
	case statusCode == 400:
		return false, false

	// 请求错误 (不应 failover，客户端问题)
	case statusCode == 404, statusCode == 405, statusCode == 406,
		statusCode == 409, statusCode == 410, statusCode == 411,
		statusCode == 412, statusCode == 413, statusCode == 414,
		statusCode == 415, statusCode == 416, statusCode == 417,
		statusCode == 422, statusCode == 423, statusCode == 424,
		statusCode == 426, statusCode == 428, statusCode == 431,
		statusCode == 451:
		return false, false

	// 服务端错误 (应 failover，非配额相关)
	case statusCode >= 500:
		return true, false

	// 其他 4xx (保守处理，不 failover)
	case statusCode >= 400 && statusCode < 500:
		return false, false

	// 成功/重定向 (不应 failover)
	default:
		return false, false
	}
}

// classifyByErrorMessage 基于错误消息内容分类
func classifyByErrorMessage(bodyBytes []byte, apiType string) (bool, bool) {
	var errResp map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &errResp); err != nil {
		log.Printf("[%s-Failover-Debug] JSON解析失败: %v, body长度=%d", apiType, err, len(bodyBytes))
		return false, false
	}

	if errValue, ok := errResp["error"].(string); ok {
		if failover, quota := classifyMessage(errValue); failover {
			log.Printf("[%s-Failover-Debug] 提取到字符串 error: %s", apiType, errValue)
			return true, quota
		}
	}

	if failover, quota, field := classifyMessageFromMap(errResp); failover {
		log.Printf("[%s-Failover-Debug] 提取到顶层消息 (字段: %s)", apiType, field)
		return true, quota
	}
	if errType, ok := errResp["type"].(string); ok {
		if failover, quota := classifyErrorType(errType); failover {
			log.Printf("[%s-Failover-Debug] 提取到顶层 type: %s", apiType, errType)
			return true, quota
		}
	}

	errObj, ok := errResp["error"].(map[string]interface{})
	if !ok {
		log.Printf("[%s-Failover-Debug] 未找到error对象, keys=%v", apiType, getMapKeys(errResp))
		return false, false
	}

	// 检查 error.code 字段，参数校验类错误码不应重试
	if errCode, ok := errObj["code"].(string); ok {
		if isNonRetryableErrorCode(errCode) {
			log.Printf("[%s-Failover-Debug] 检测到不可重试错误码: %s", apiType, errCode)
			return false, false
		}
	}

	if isSchemaValidationError(errObj) {
		log.Printf("[%s-Failover-Debug] 检测到 schema/invalid_request 错误，不进行 failover", apiType)
		return false, false
	}

	if failover, quota, field := classifyMessageFromMap(errObj); failover {
		log.Printf("[%s-Failover-Debug] 提取到消息 (字段: %s)", apiType, field)
		return true, quota
	}

	// 如果 upstream_error 是嵌套对象，尝试提取其中的消息
	if upstreamErr, ok := errObj["upstream_error"].(map[string]interface{}); ok {
		if failover, quota, field := classifyMessageFromMap(upstreamErr); failover {
			log.Printf("[%s-Failover-Debug] 提取到嵌套 upstream_error.%s", apiType, field)
			return true, quota
		}
	}

	// 检查 type 字段
	if errType, ok := errObj["type"].(string); ok {
		if failover, quota := classifyErrorType(errType); failover {
			return true, quota
		}
	}

	log.Printf("[%s-Failover-Debug] 未匹配任何关键词, errObj keys=%v", apiType, getMapKeys(errObj))
	return false, false
}

func classifyMessageFromMap(m map[string]interface{}) (bool, bool, string) {
	messageFields := []string{"message", "upstream_error", "detail", "error_description", "msg"}
	for _, field := range messageFields {
		if msg, ok := m[field].(string); ok {
			if failover, quota := classifyMessage(msg); failover {
				return true, quota, field
			}
		}
	}
	return false, false, ""
}

// classifyMessage 基于错误消息内容分类
func classifyMessage(msg string) (bool, bool) {
	msgLower := strings.ToLower(msg)

	if isSchemaValidationMessage(msgLower) {
		return false, false
	}

	// 配额/余额相关关键词 (failover + quota)
	quotaKeywords := []string{
		"insufficient", "quota", "credit", "balance",
		"rate limit", "limit exceeded", "exceeded",
		"billing", "payment", "subscription",
		"积分不足", "余额不足", "请求数限制", "额度", "预扣费",
	}
	for _, keyword := range quotaKeywords {
		if strings.Contains(msgLower, keyword) {
			return true, true
		}
	}

	// 认证/授权相关关键词 (failover + 非 quota)
	authKeywords := []string{
		"invalid", "unauthorized", "authentication",
		"api key", "apikey", "token", "expired",
		"permission", "forbidden", "denied",
		"密钥无效", "认证失败", "权限不足",
		"身份验证失败", "身份验证", "无效的令牌", "令牌无效", "令牌已过期",
		"令牌过期", "令牌已失效", "令牌失效", "未授权", "鉴权失败",
	}
	for _, keyword := range authKeywords {
		if strings.Contains(msgLower, keyword) {
			return true, false
		}
	}

	// 临时错误关键词 (failover + 非 quota)
	transientKeywords := []string{
		"timeout", "timed out", "temporarily",
		"overloaded", "unavailable", "retry",
		"server error", "internal error",
		"超时", "暂时", "重试",
	}
	for _, keyword := range transientKeywords {
		if strings.Contains(msgLower, keyword) {
			return true, false
		}
	}

	return false, false
}

// classifyErrorType 基于错误类型分类
func classifyErrorType(errType string) (bool, bool) {
	typeLower := strings.ToLower(errType)

	// 只拦截明确的 schema/validation 错误
	nonRetryableTypes := []string{
		"schema_validation_error",
		"validation_error",
	}
	for _, t := range nonRetryableTypes {
		if strings.Contains(typeLower, t) {
			return false, false
		}
	}

	// 配额相关的错误类型 (failover + quota)
	quotaTypes := []string{
		"over_quota", "quota_exceeded", "rate_limit",
		"billing", "insufficient", "payment",
	}
	for _, t := range quotaTypes {
		if strings.Contains(typeLower, t) {
			return true, true
		}
	}

	// 认证相关的错误类型 (failover + 非 quota)
	authTypes := []string{
		"authentication", "authorization", "permission",
		"invalid_api_key", "invalid_token", "expired",
	}
	for _, t := range authTypes {
		if strings.Contains(typeLower, t) {
			return true, false
		}
	}

	// 服务端错误类型 (failover + 非 quota)
	serverTypes := []string{
		"server_error", "internal_error", "service_unavailable",
		"timeout", "overloaded",
	}
	for _, t := range serverTypes {
		if strings.Contains(typeLower, t) {
			return true, false
		}
	}

	return false, false
}

func isSchemaValidationError(errObj map[string]interface{}) bool {
	// 先检查 error.code，排除认证相关错误（需要 failover）
	if errCode, ok := errObj["code"].(string); ok {
		codeLower := strings.ToLower(errCode)
		// 认证错误应该触发 failover，不拦截
		authCodes := []string{
			"invalid_api_key",
			"authentication_error",
			"permission_denied",
			"unauthorized",
		}
		for _, authCode := range authCodes {
			if strings.Contains(codeLower, authCode) {
				return false
			}
		}
	}

	// 检查消息内容
	for _, field := range []string{"message", "upstream_error", "detail"} {
		if msg, ok := errObj[field].(string); ok && isSchemaValidationMessage(strings.ToLower(msg)) {
			return true
		}
	}

	if upstreamErr, ok := errObj["upstream_error"].(map[string]interface{}); ok {
		if msg, ok := upstreamErr["message"].(string); ok && isSchemaValidationMessage(strings.ToLower(msg)) {
			return true
		}
	}

	// 检查 error.type，但排除单纯的 invalid_request_error（可能是认证问题）
	if errType, ok := errObj["type"].(string); ok {
		typeLower := strings.ToLower(errType)
		// schema_validation_error 明确是参数错误，拦截
		if strings.Contains(typeLower, "schema_validation") || strings.Contains(typeLower, "validation_error") {
			return true
		}
		// invalid_request_error 需要结合其他信息判断，单独出现不拦截
	}

	return false
}

func isSchemaValidationMessage(msgLower string) bool {
	nonRetryableKeywords := []string{
		"invalid value",
		"supported values are",
		"schema validation",
		"validation failed",
		"invalid_request",
		"invalid request",
		"unsupported content type",
		// 结构字段校验（Anthropic 常见）
		"field required",
		"required field",
		"missing required parameter",
		"is required",
		"messages.",
		".content.",
		".thinking.",
	}
	for _, keyword := range nonRetryableKeywords {
		if strings.Contains(msgLower, keyword) {
			return true
		}
	}
	return false
}

// handleFuzzyModelRoutingError 在 fuzzy 模式下处理可归一化的模型路由错误
// 如果最后失败的错误可以被归一化为非 503 状态码（如 model_not_found → 404），
// 则透传该错误体和归一化后的状态码；否则返回 false，由调用方继续返回通用 503
func handleFuzzyModelRoutingError(c *gin.Context, lastFailoverError *FailoverError) bool {
	if lastFailoverError == nil {
		return false
	}
	normalizedStatus := normalizeUpstreamErrorStatus(lastFailoverError.Status, lastFailoverError.Body)
	if normalizedStatus == lastFailoverError.Status {
		return false
	}
	var errBody map[string]interface{}
	if err := json.Unmarshal(lastFailoverError.Body, &errBody); err == nil {
		c.JSON(normalizedStatus, errBody)
	} else {
		c.JSON(normalizedStatus, gin.H{"error": string(lastFailoverError.Body)})
	}
	return true
}

func fuzzyUnavailableMessage(lastFailoverError *FailoverError, lastError error) string {
	base := "All upstream channels are currently unavailable"
	if lastError != nil {
		if detail := strings.TrimSpace(lastError.Error()); detail != "" {
			return base + ": " + truncateMessage(detail)
		}
	}
	if lastFailoverError == nil {
		return base
	}

	errType, errMessage := extractErrorInfo(lastFailoverError.Body)
	parts := make([]string, 0, 3)
	if lastFailoverError.Status > 0 {
		parts = append(parts, fmt.Sprintf("upstream status %d", lastFailoverError.Status))
	}
	if errType != "" {
		parts = append(parts, "code/type "+errType)
	}
	if errMessage != "" {
		parts = append(parts, truncateMessage(errMessage))
	}
	if len(parts) == 0 && len(lastFailoverError.Body) > 0 {
		parts = append(parts, truncateMessage(string(lastFailoverError.Body)))
	}
	if len(parts) == 0 {
		return base
	}
	return base + ": " + strings.Join(parts, "; ")
}

// HandleAllChannelsFailed 处理所有渠道都失败的情况
// fuzzyMode: 是否启用模糊模式（返回通用错误）
// lastFailoverError: 最后一个故障转移错误
// lastError: 最后一个错误
// apiType: API 类型（用于错误消息）
func HandleAllChannelsFailed(c *gin.Context, fuzzyMode bool, lastFailoverError *FailoverError, lastError error, apiType string) {
	// Fuzzy 模式下默认返回通用错误，但保留明确的模型路由错误语义
	if fuzzyMode {
		if handleFuzzyModelRoutingError(c, lastFailoverError) {
			return
		}
		message := fuzzyUnavailableMessage(lastFailoverError, lastError)
		c.JSON(503, gin.H{
			"type": "error",
			"error": gin.H{
				"type":    "service_unavailable",
				"message": message,
			},
		})
		return
	}

	// 非 Fuzzy 模式：透传最后一个错误的详情
	if lastFailoverError != nil {
		status := normalizeUpstreamErrorStatus(lastFailoverError.Status, lastFailoverError.Body)
		if status == 0 {
			status = 503
		}
		var errBody map[string]interface{}
		if err := json.Unmarshal(lastFailoverError.Body, &errBody); err == nil {
			c.JSON(status, errBody)
		} else {
			c.JSON(status, gin.H{"error": string(lastFailoverError.Body)})
		}
	} else {
		errMsg := "所有渠道都不可用"
		if lastError != nil {
			errMsg = lastError.Error()
		}
		c.JSON(503, gin.H{
			"error":   "所有" + apiType + "渠道都不可用",
			"details": errMsg,
		})
	}
}

// HandleAllKeysFailed 处理所有密钥都失败的情况（单渠道模式）
func HandleAllKeysFailed(c *gin.Context, fuzzyMode bool, lastFailoverError *FailoverError, lastError error, apiType string) {
	// Fuzzy 模式下默认返回通用错误，但保留明确的模型路由错误语义
	if fuzzyMode {
		if handleFuzzyModelRoutingError(c, lastFailoverError) {
			return
		}
		message := fuzzyUnavailableMessage(lastFailoverError, lastError)
		c.JSON(503, gin.H{
			"type": "error",
			"error": gin.H{
				"type":    "service_unavailable",
				"message": message,
			},
		})
		return
	}

	// 非 Fuzzy 模式：透传最后一个错误的详情
	if lastFailoverError != nil {
		status := normalizeUpstreamErrorStatus(lastFailoverError.Status, lastFailoverError.Body)
		if status == 0 {
			status = 500
		}
		var errBody map[string]interface{}
		if err := json.Unmarshal(lastFailoverError.Body, &errBody); err == nil {
			c.JSON(status, errBody)
		} else {
			c.JSON(status, gin.H{"error": string(lastFailoverError.Body)})
		}
	} else {
		errMsg := "未知错误"
		if lastError != nil {
			errMsg = lastError.Error()
		}
		c.JSON(500, gin.H{
			"error":   "所有上游" + apiType + "API密钥都不可用",
			"details": errMsg,
		})
	}
}

// getMapKeys 获取 map 的所有 key（用于调试日志）
func getMapKeys(m map[string]interface{}) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	return keys
}

// isContentModerationErrorCode 判断错误码是否为内容审核类错误
// 内容审核错误与请求内容本身相关，换渠道/换 Key 重试不会改变结果，任何状态码都不应 failover
func isContentModerationErrorCode(code string) bool {
	codes := []string{
		"sensitive_words_detected",
		"content_policy_violation",
		"content_filter",
		"content_blocked",
		"moderation_blocked",
	}
	codeLower := strings.ToLower(code)
	for _, c := range codes {
		if codeLower == c {
			return true
		}
	}
	return false
}

// isNonRetryableErrorCode 判断错误码是否为参数校验类不可重试错误
// 这类错误在 4xx 时不应 failover，但 5xx 时可能是上游误报，应允许 failover
func isNonRetryableErrorCode(code string) bool {
	codes := []string{
		"invalid_request",
		"invalid_request_error",
		"bad_request",
	}
	codeLower := strings.ToLower(code)
	for _, c := range codes {
		if codeLower == c {
			return true
		}
	}
	return false
}

// isContentModerationError 检查响应体是否包含内容审核类错误
// 任何状态码下都应阻止 failover
func isContentModerationError(bodyBytes []byte) bool {
	var errResp map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &errResp); err != nil {
		return false
	}
	errObj, ok := errResp["error"].(map[string]interface{})
	if !ok {
		return false
	}
	if errCode, ok := errObj["code"].(string); ok {
		return isContentModerationErrorCode(errCode)
	}
	return false
}

// isNonRetryableError 检查响应体是否包含不可重试的错误码
func isNonRetryableError(bodyBytes []byte, apiType string) bool {
	var errResp map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &errResp); err != nil {
		return false
	}
	errObj, ok := errResp["error"].(map[string]interface{})
	if !ok {
		return false
	}
	// 在判定 schema/invalid_request 之前，先放行上游协议兼容类 400：
	// 这些错误源于上游镜像不认识新版 Responses tools schema，换兼容上游后可恢复。
	if strings.EqualFold(apiType, "Responses") && isResponsesToolsProtocolError(errObj) {
		return false
	}
	if isSchemaValidationError(errObj) {
		return true
	}
	if errCode, ok := errObj["code"].(string); ok {
		return isNonRetryableErrorCode(errCode)
	}
	return false
}

// isResponsesToolsProtocolError 识别上游对 /v1/responses 中 tools 结构的兼容性拒绝。
// 例如第三方 Responses 镜像不识别 Codex CLI 0.130+ 的 namespace/custom/web_search 等条目，
// 返回形如 "Missing required parameter: 'tools[15].tools'" 的 400。
func isResponsesToolsProtocolError(errObj map[string]interface{}) bool {
	message := strings.ToLower(toStringField(errObj, "message"))
	param := strings.ToLower(toStringField(errObj, "param"))
	code := strings.ToLower(toStringField(errObj, "code"))
	upstream := strings.ToLower(toStringField(errObj, "upstream_error"))
	if nested, ok := errObj["upstream_error"].(map[string]interface{}); ok {
		upstream += " " + strings.ToLower(toStringField(nested, "message"))
		upstream += " " + strings.ToLower(toStringField(nested, "param"))
		upstream += " " + strings.ToLower(toStringField(nested, "code"))
	}

	combined := strings.Join([]string{message, param, code, upstream}, " ")
	if !mentionsResponsesTools(combined) {
		return false
	}
	if code == "invalid_function_parameters" || code == "missing_required_parameter" {
		return true
	}
	markers := []string{
		"missing required parameter",
		"unknown parameter",
		"invalid schema",
		"invalid function parameters",
		"unsupported tool",
		"unknown tool",
		"expected", // 兼容 "expected object/array/string" 类 schema 文案
	}
	for _, marker := range markers {
		if strings.Contains(combined, marker) {
			return true
		}
	}
	return false
}

func mentionsResponsesTools(text string) bool {
	toolMarkers := []string{
		"tools",
		"tool_choice",
		"function '",
		"function \"",
		"web_search",
		"namespace",
		"custom tool",
	}
	for _, marker := range toolMarkers {
		if strings.Contains(text, marker) {
			return true
		}
	}
	return false
}

func toStringField(m map[string]interface{}, key string) string {
	if v, ok := m[key].(string); ok {
		return v
	}
	return ""
}

// isModelRoutingError 识别上游将模型路由失败误报为 5xx 的错误
// 仅用于状态码归一化（5xx → 404），不阻断 failover
func isModelRoutingError(bodyBytes []byte) bool {
	var errResp map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &errResp); err != nil {
		return false
	}
	errObj, ok := errResp["error"].(map[string]interface{})
	if !ok {
		return false
	}
	if errCode, ok := errObj["code"].(string); ok {
		if strings.ToLower(errCode) == "model_not_found" {
			return true
		}
	}
	if errMsg, ok := errObj["message"].(string); ok {
		msgLower := strings.ToLower(errMsg)
		if strings.Contains(msgLower, "no available channel for model") {
			return true
		}
	}
	return false
}

// normalizeUpstreamErrorStatus 修正上游误报的客户端配置错误状态码
func normalizeUpstreamErrorStatus(status int, bodyBytes []byte) int {
	if status >= 500 && len(bodyBytes) > 0 && isModelRoutingError(bodyBytes) {
		return 404
	}
	return status
}

// BlacklistResult 拉黑判定结果
type BlacklistResult struct {
	ShouldBlacklist bool
	Reason          string // "authentication_error" / "permission_error" / "insufficient_balance"
	Message         string // 原始错误信息摘要
}

// ShouldBlacklistKey 判断 HTTP 错误响应是否应该永久拉黑该 Key
// 对余额不足只识别明确语义，避免将普通 403/429 误判为永久失效
func ShouldBlacklistKey(statusCode int, bodyBytes []byte) BlacklistResult {
	// HTTP 402: 明确的付费/余额不足
	if statusCode == 402 {
		return BlacklistResult{
			ShouldBlacklist: true,
			Reason:          "insufficient_balance",
			Message:         truncateMessage(string(bodyBytes)),
		}
	}

	// 解析响应体
	errType, errMessage := extractErrorInfo(bodyBytes)
	if errType == "" && errMessage == "" {
		return BlacklistResult{}
	}

	typeLower := strings.ToLower(errType)

	// 认证错误: authentication_error / invalid_api_key
	if typeLower == "authentication_error" || typeLower == "invalid_api_key" {
		return BlacklistResult{
			ShouldBlacklist: true,
			Reason:          "authentication_error",
			Message:         truncateMessage(errMessage),
		}
	}

	// 某些上游只返回 401/403 + 明确的认证失败消息，没有 type/code
	if (statusCode == 401 || statusCode == 403) && isAuthenticationMessage(errMessage) {
		return BlacklistResult{
			ShouldBlacklist: true,
			Reason:          "authentication_error",
			Message:         truncateMessage(errMessage),
		}
	}

	// 权限错误: permission_error / permission_denied
	if typeLower == "permission_error" || typeLower == "permission_denied" {
		return BlacklistResult{
			ShouldBlacklist: true,
			Reason:          "permission_error",
			Message:         truncateMessage(errMessage),
		}
	}

	// 余额不足的明确错误类型/错误码
	if isInsufficientBalanceCode(errType) || typeLower == "billing_error" {
		return BlacklistResult{
			ShouldBlacklist: true,
			Reason:          "insufficient_balance",
			Message:         truncateMessage(errMessage),
		}
	}

	// 某些上游会返回 HTTP 401/403/429，但在 message 中携带明确的余额不足语义
	if (statusCode == 401 || statusCode == 403 || statusCode == 429) && isInsufficientBalanceMessage(errMessage) {
		return BlacklistResult{
			ShouldBlacklist: true,
			Reason:          "insufficient_balance",
			Message:         truncateMessage(errMessage),
		}
	}

	return BlacklistResult{}
}

func isInsufficientBalanceMessage(msg string) bool {
	if msg == "" {
		return false
	}

	msgLower := strings.ToLower(msg)
	keywords := []string{
		"insufficient balance",
		"insufficient account balance",
		"insufficient quota",
		"insufficient credits",
		"insufficient funds",
		"balance too low",
		"no balance",
		"out of credits",
		"quota exhausted",
		"quota used up",
		"quota is not enough",
		"not enough quota",
		"remain quota",
		"need quota",
		"daily limit exceeded",
		"daily usage limit exceeded",
		"tokenstatusexhausted",
		"余额不足",
		"余额已用尽",
		"额度不足",
		"额度已用尽",
		"额度已用完",
		"额度耗尽",
		"当日额度已用尽",
		"每日额度已用尽",
		"日额度已用尽",
		"令牌额度已用尽",
		"预扣费额度失败",
		"需要预扣费额度",
	}
	for _, keyword := range keywords {
		if strings.Contains(msgLower, keyword) {
			return true
		}
	}
	return false
}

func isAuthenticationMessage(msg string) bool {
	if msg == "" {
		return false
	}

	msgLower := strings.ToLower(msg)
	keywords := []string{
		"invalid api key",
		"invalid_api_key",
		"invalid key",
		"invalid token",
		"token expired",
		"token has expired",
		"expired token",
		"authentication failed",
		"authentication error",
		"unauthorized",
		"api key is invalid",
		"api key provided is invalid",
		"无效的api key",
		"api key无效",
		"无效 api key",
		"认证失败",
		"身份验证失败",
		"无效的令牌",
		"令牌无效",
		"令牌已过期",
		"令牌过期",
		"令牌已失效",
		"令牌失效",
		"鉴权失败",
	}
	for _, keyword := range keywords {
		if strings.Contains(msgLower, keyword) {
			return true
		}
	}
	return false
}

func isPermissionMessage(msg string) bool {
	if msg == "" {
		return false
	}

	msgLower := strings.ToLower(msg)
	keywords := []string{
		"permission denied",
		"permission error",
		"forbidden",
		"access denied",
		"权限不足",
		"没有权限",
		"禁止访问",
	}
	for _, keyword := range keywords {
		if strings.Contains(msgLower, keyword) {
			return true
		}
	}
	return false
}

// extractErrorInfo 从 JSON 响应体中提取错误类型和错误消息
// 支持嵌套格式 {"error":{"type":"...","code":"...","message":"..."}}
// 和扁平格式 {"type":"...","code":"...","message":"..."}
func extractErrorInfo(bodyBytes []byte) (errType string, errMessage string) {
	var resp map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &resp); err != nil {
		return "", ""
	}

	// 优先尝试嵌套格式: {"error": {"type": "...", "code": "...", "message": "..."}}
	if errObj, ok := resp["error"].(map[string]interface{}); ok {
		if t, ok := errObj["type"].(string); ok {
			errType = t
		} else if c, ok := errObj["code"].(string); ok {
			errType = c
		}
		if m, ok := errObj["message"].(string); ok {
			errMessage = m
		}
		return
	}

	// 兼容字符串格式: {"error": "..."}
	if errStr, ok := resp["error"].(string); ok {
		errMessage = errStr
	}

	// fallback: 扁平格式 {"type": "...", "code": "...", "message": "..."}
	if t, ok := resp["type"].(string); ok {
		errType = t
	} else if c, ok := resp["code"].(string); ok {
		errType = c
	}
	if m, ok := resp["message"].(string); ok {
		errMessage = m
	}
	return
}

// truncateMessage 截断错误信息（最多800字符），用于指标/原因字段等短摘要场景
// 使用 rune 截断，避免切断 UTF-8 多字节字符
func truncateMessage(msg string) string {
	runes := []rune(msg)
	if len(runes) > 800 {
		return string(runes[:800])
	}
	return msg
}

// truncateErrorSummary 用于日志打印上游错误详情，保留足够上下文以便定位协议/schema 问题
// 上限设置为 4KB，避免极端情况下把巨型响应体全量入日志
// 使用 rune 截断，避免切断 UTF-8 多字节字符
func truncateErrorSummary(msg string) string {
	const maxLen = 4096
	runes := []rune(msg)
	if len(runes) > maxLen {
		return string(runes[:maxLen]) + "...(truncated)"
	}
	return msg
}
