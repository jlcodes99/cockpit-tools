package providers

import (
	"bytes"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/gin-gonic/gin"
)

const requestBodyBytesContextKey = "requestBodyBytes"

// Provider 提供商接口
type Provider interface {
	// ConvertToProviderRequest 将 gin context 中的请求转换为目标上游的 http.Request，并返回用于日志的原始请求体
	ConvertToProviderRequest(c *gin.Context, upstream *config.UpstreamConfig, apiKey string) (*http.Request, []byte, error)

	// ConvertToClaudeResponse 将提供商响应转换为 Claude 响应
	ConvertToClaudeResponse(providerResp *types.ProviderResponse) (*types.ClaudeResponse, error)

	// HandleStreamResponse 处理流式响应
	HandleStreamResponse(body io.ReadCloser) (<-chan string, <-chan error, error)
}

// normalizeSSEFieldLine 标准化 SSE 字段行的格式
// SSE 规范允许 "data:value" 和 "data: value" 两种格式，
// 但下游解析统一使用 "data: " 前缀，因此需要标准化。
// 例如: "data:{...}" → "data: {...}", "event:message_start" → "event: message_start"
func normalizeSSEFieldLine(line string) string {
	for _, prefix := range []string{"data:", "event:", "id:", "retry:"} {
		if strings.HasPrefix(line, prefix) && !strings.HasPrefix(line, prefix+" ") {
			return prefix + " " + line[len(prefix):]
		}
	}
	return line
}

func newDefaultSessionManager() *session.SessionManager {
	return session.NewSessionManager(24*time.Hour, 100, 100000)
}

func getRequestBodyBytes(c *gin.Context) ([]byte, error) {
	if cached, ok := c.Get(requestBodyBytesContextKey); ok {
		if bodyBytes, ok := cached.([]byte); ok {
			c.Request.Body = io.NopCloser(bytes.NewReader(bodyBytes))
			return bodyBytes, nil
		}
	}

	bodyBytes, err := io.ReadAll(c.Request.Body)
	if err != nil {
		return nil, err
	}
	c.Request.Body = io.NopCloser(bytes.NewReader(bodyBytes))
	c.Set(requestBodyBytesContextKey, bodyBytes)
	return bodyBytes, nil
}

// GetProvider 根据服务类型获取提供商
func GetProvider(serviceType string) Provider {
	switch serviceType {
	case "openai":
		return &OpenAIProvider{}
	case "gemini":
		return &GeminiProvider{}
	case "claude":
		return &ClaudeProvider{}
	case "responses":
		return &ResponsesProvider{SessionManager: newDefaultSessionManager()}
	default:
		return nil
	}
}
