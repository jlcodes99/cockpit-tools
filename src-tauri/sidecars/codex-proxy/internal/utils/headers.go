package utils

import (
	"encoding/json"
	"net/http"
	"sort"
	"strings"

	"github.com/gin-gonic/gin"
)

// PrepareUpstreamHeaders 准备上游请求头（统一头部处理逻辑）
// 保留原始请求头，移除代理相关头部，设置认证头
// 注意：此函数适用于Claude类型渠道，对于其他类型请使用 PrepareMinimalHeaders
// ExtractUnifiedSessionID 统一提取会话/缓存标识，供 Messages/Responses/Chat/Gemini 复用。
// 优先级: Conversation_id > Session_id > X-Claude-Code-Session-Id > X-Client-Request-Id > X-Gemini-Api-Privileged-User-Id > user > prompt_cache_key > metadata.user_id
func ExtractUnifiedSessionID(c *gin.Context, bodyBytes []byte) string {
	if c != nil {
		if convID := c.GetHeader("Conversation_id"); convID != "" {
			return convID
		}

		if sessID := c.GetHeader("Session_id"); sessID != "" {
			return sessID
		}

		if claudeCodeSessionID := c.GetHeader("X-Claude-Code-Session-Id"); claudeCodeSessionID != "" {
			return claudeCodeSessionID
		}

		if clientRequestID := c.GetHeader("X-Client-Request-Id"); clientRequestID != "" {
			return clientRequestID
		}

		if geminiUserID := c.GetHeader("X-Gemini-Api-Privileged-User-Id"); geminiUserID != "" {
			return geminiUserID
		}
	}

	var req map[string]interface{}
	if err := json.Unmarshal(bodyBytes, &req); err != nil {
		return ""
	}

	if user, ok := req["user"].(string); ok && user != "" {
		return user
	}
	if promptCacheKey, ok := req["prompt_cache_key"].(string); ok && promptCacheKey != "" {
		return promptCacheKey
	}
	if metadata, ok := req["metadata"].(map[string]interface{}); ok {
		if userID, ok := metadata["user_id"].(string); ok && userID != "" {
			return userID
		}
		if flattened := flattenMetadataUserID(metadata["user_id"]); flattened != "" {
			return flattened
		}
	}

	return ""
}

func flattenMetadataUserID(raw interface{}) string {
	if raw == nil {
		return ""
	}

	parsed, ok := raw.(map[string]interface{})
	if !ok || len(parsed) == 0 {
		return ""
	}

	var parts []string
	if deviceID, ok := parsed["device_id"].(string); ok && deviceID != "" {
		parts = append(parts, "user_"+deviceID)
		if accountUUID, ok := parsed["account_uuid"].(string); ok && accountUUID != "" {
			parts = append(parts, "account_"+accountUUID)
		}
		if sessionID, ok := parsed["session_id"].(string); ok && sessionID != "" {
			parts = append(parts, "session_"+sessionID)
		}
	} else {
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

	if len(parts) == 0 {
		return ""
	}
	return strings.Join(parts, "_")
}

func PrepareUpstreamHeaders(c *gin.Context, targetHost string) http.Header {
	headers := c.Request.Header.Clone()

	// 设置正确的Host头部
	headers.Set("Host", targetHost)

	// 移除代理相关头部，降低被识别为中转层的风险
	headers.Del("x-proxy-key")
	headers.Del("X-Forwarded-For")
	headers.Del("X-Forwarded-Host")
	headers.Del("X-Forwarded-Proto")
	headers.Del("X-Real-IP")
	headers.Del("Via")
	headers.Del("Forwarded")

	// 移除 Accept-Encoding，让 Go 的 http.Client 自动处理 gzip 压缩/解压缩
	// 这样可以避免在原始请求包含 Accept-Encoding 时 Go 不自动解压缩的问题
	headers.Del("Accept-Encoding")

	// 强制去重 Content-Type（部分客户端可能发送重复的 Content-Type 头）
	headers.Set("Content-Type", "application/json")

	return headers
}

// PrepareMinimalHeaders 准备最小化请求头（适用于非Claude渠道如OpenAI、Gemini等）
// 只保留必要的头部：Content-Type和Host，不包含任何Anthropic特定头部
// 注意：不设置Accept-Encoding，让Go的http.Client自动处理gzip压缩
func PrepareMinimalHeaders(targetHost string) http.Header {
	headers := http.Header{}

	// 只设置最基本的头部
	headers.Set("Host", targetHost)
	headers.Set("Content-Type", "application/json")
	// 不显式设置Accept-Encoding，让Go的http.Client自动添加并处理gzip解压

	return headers
}

// SetAuthenticationHeader 设置认证头部（根据密钥格式智能选择）
func SetAuthenticationHeader(headers http.Header, apiKey string) {
	// 移除旧的认证头
	headers.Del("authorization")
	headers.Del("x-api-key")
	headers.Del("x-goog-api-key")

	// Claude 官方密钥格式（sk-ant-api03-xxx）使用 x-api-key
	// 符合 Claude API 官方推荐的认证方式
	if strings.HasPrefix(apiKey, "sk-ant-") {
		headers.Set("x-api-key", apiKey)
	} else {
		// 其他格式密钥使用 Authorization: Bearer
		// 适用于 OpenAI、自定义密钥等
		headers.Set("Authorization", "Bearer "+apiKey)
	}
}

// SetGeminiAuthenticationHeader 设置Gemini认证头部
func SetGeminiAuthenticationHeader(headers http.Header, apiKey string) {
	headers.Del("authorization")
	headers.Del("x-api-key")
	headers.Set("x-goog-api-key", apiKey)
}

// ApplyCustomHeaders 应用自定义请求头（覆盖或添加）
// 使用 http.Header.Set 会自动规范化 key 为 CanonicalHeaderKey 格式
// 跳过空白 key 或 value
func ApplyCustomHeaders(headers http.Header, customHeaders map[string]string) {
	for key, value := range customHeaders {
		key = strings.TrimSpace(key)
		value = strings.TrimSpace(value)
		if key == "" || value == "" {
			continue
		}
		headers.Set(key, value)
	}
}

// EnsureCompatibleUserAgent 确保兼容的User-Agent（仅在必要时设置）
func EnsureCompatibleUserAgent(headers http.Header, serviceType string) {
	userAgent := headers.Get("User-Agent")

	// 仅在Claude服务类型且客户端未提供 User-Agent 时才设置默认值，有 UA 则透传
	if serviceType == "claude" {
		if userAgent == "" {
			headers.Set("User-Agent", "claude-cli/2.0.34 (external, cli)")
		}
	}
}

// ForwardResponseHeaders 转发上游响应头到客户端
// 作为透明代理，应该转发所有响应头，只过滤框架自动处理的头部
func ForwardResponseHeaders(upstreamHeaders http.Header, clientWriter http.ResponseWriter) {
	// 不应转发的头部列表（由框架或代理层自动处理）
	skipHeaders := map[string]bool{
		"transfer-encoding": true, // 由框架自动处理
		"content-length":    true, // 由框架自动处理
		"connection":        true, // 代理层控制
		"content-encoding":  true, // 如果已解压则不应转发
	}

	// 复制所有上游响应头到客户端
	for key, values := range upstreamHeaders {
		lowerKey := strings.ToLower(key)

		// 跳过不应转发的头部
		if skipHeaders[lowerKey] {
			continue
		}

		// 转发头部（可能有多个值）
		for _, value := range values {
			clientWriter.Header().Add(key, value)
		}
	}
}
