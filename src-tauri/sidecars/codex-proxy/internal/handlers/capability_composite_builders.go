package handlers

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/providers"
	"github.com/gin-gonic/gin"
)

// ============== 复合协议请求构造（复用现有 provider 转换链路） ==============

// entryPathForProtocol 返回各基础协议的入口路径，用于构造 gin test context 的请求路径。
func entryPathForProtocol(protocol CapabilityBaseProtocol) string {
	switch protocol {
	case CapabilityProtocolMessages:
		return "/v1/messages"
	case CapabilityProtocolChat:
		return "/v1/chat/completions"
	case CapabilityProtocolResponses:
		return "/v1/responses"
	case CapabilityProtocolGemini:
		return "/v1beta/models/probe:streamGenerateContent?alt=sse"
	default:
		return "/v1/messages"
	}
}

// providerServiceTypeForProtocol 返回各基础协议对应的 provider serviceType，用于 GetProvider。
func providerServiceTypeForProtocol(protocol CapabilityBaseProtocol) string {
	switch protocol {
	case CapabilityProtocolMessages:
		return "claude"
	case CapabilityProtocolChat:
		return "openai"
	case CapabilityProtocolResponses:
		return "responses"
	case CapabilityProtocolGemini:
		return "gemini"
	default:
		return "claude"
	}
}

// buildMessagesProbeBody 构造 messages 协议最小探测请求体。
func buildMessagesProbeBody(probeModel string) []byte {
	body, _ := json.Marshal(map[string]interface{}{
		"model": probeModel,
		"system": []map[string]interface{}{
			{
				"type": "text",
				"text": "x-anthropic-billing-header: cc_version=2.1.71.2f9; cc_entrypoint=cli;",
			},
			{
				"type": "text",
				"text": "You are a Claude agent, built on Anthropic's Claude Agent SDK.",
				"cache_control": map[string]string{
					"type": "ephemeral",
				},
			},
		},
		"messages":   []map[string]string{{"role": "user", "content": "What are you best at: code generation, creative writing, or math problem solving?"}},
		"max_tokens": 100,
		"stream":     true,
		"thinking": map[string]interface{}{
			"type": "disabled",
		},
	})
	return body
}

// buildChatProbeBody 构造 chat 协议最小探测请求体。
func buildChatProbeBody(probeModel string) []byte {
	body, _ := json.Marshal(map[string]interface{}{
		"model": probeModel,
		"messages": []map[string]string{
			{"role": "system", "content": "You are a helpful assistant."},
			{"role": "user", "content": "What are you best at: code generation, creative writing, or math problem solving?"},
		},
		"max_tokens":       100,
		"stream":           true,
		"reasoning_effort": "low",
	})
	return body
}

// buildResponsesProbeBody 构造 responses 协议最小探测请求体。
func buildResponsesProbeBody(probeModel string) []byte {
	body, _ := json.Marshal(map[string]interface{}{
		"model": probeModel,
		"input": []map[string]interface{}{
			{
				"role": "user",
				"content": []map[string]string{
					{"type": "input_text", "text": "What are you best at: code generation, creative writing, or math problem solving?"},
				},
			},
		},
		"max_output_tokens": 100,
		"stream":            true,
	})
	return body
}

// buildGeminiProbeBody 构造 gemini 协议最小探测请求体。
func buildGeminiProbeBody(probeModel string) []byte {
	body, _ := json.Marshal(map[string]interface{}{
		"contents": []map[string]interface{}{
			{
				"role": "user",
				"parts": []map[string]string{
					{"text": "What are you best at: code generation, creative writing, or math problem solving?"},
				},
			},
		},
		"generationConfig": map[string]interface{}{
			"maxOutputTokens": 100,
		},
	})
	return body
}

// buildCompositeRequestViaProvider 复用现有 provider 转换链路，将 fromProtocol 入口请求体
// 经过 provider.ConvertToProviderRequest 转换为 toProtocol 上游请求。
//
// 实现原理：构造 gin.CreateTestContext + httptest.NewRequest 模拟 fromProtocol 入口请求，
// 调用 toProtocol 对应 provider 的 ConvertToProviderRequest 完成协议方向转换 + ModelMapping。
//
// 返回值：
//   - reqURL: 完整上游请求 URL
//   - reqBody: 上游请求体（已序列化）
//   - targetProtocol: 实际目标协议（即 toProtocol）
//   - err: 错误
func buildCompositeRequestViaProvider(
	toProtocol CapabilityBaseProtocol,
	channel *config.UpstreamConfig,
	apiKey string,
	fromBody []byte,
	fromPath string,
) (string, []byte, string, error) {
	serviceType := providerServiceTypeForProtocol(toProtocol)
	provider := providers.GetProvider(serviceType)
	if provider == nil {
		return "", nil, "", fmt.Errorf("no provider for serviceType=%s", serviceType)
	}

	// 构造模拟入口请求：使用 from 协议的 body 和 path
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, fromPath, bytes.NewReader(fromBody))
	c.Request.Header.Set("Content-Type", "application/json")

	// 通过 provider 转换：fromBody → toProtocol 上游请求
	// 此过程会自动应用 ModelMapping 和协议方向转换
	req, _, err := provider.ConvertToProviderRequest(c, channel, apiKey)
	if err != nil {
		return "", nil, "", fmt.Errorf("ConvertToProviderRequest failed: %w", err)
	}
	if req == nil {
		return "", nil, "", fmt.Errorf("ConvertToProviderRequest returned nil request")
	}

	// 读取上游请求体（req.Body 是一次性 reader）
	var reqBody []byte
	if req.Body != nil {
		reqBody, err = io.ReadAll(req.Body)
		req.Body.Close()
		if err != nil {
			return "", nil, "", fmt.Errorf("read converted request body: %w", err)
		}
	}

	return req.URL.String(), reqBody, string(toProtocol), nil
}
