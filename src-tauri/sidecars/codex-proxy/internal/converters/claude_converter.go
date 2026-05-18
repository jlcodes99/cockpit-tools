package converters

import (
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// ============== Claude Messages API 转换器 ==============

// ClaudeConverter 实现 Responses → Claude Messages API 转换
type ClaudeConverter struct{}

// ToProviderRequest 将 Responses 请求转换为 Claude Messages 格式
func (c *ClaudeConverter) ToProviderRequest(sess *session.Session, req *types.ResponsesRequest) (interface{}, error) {
	// 转换 messages 和 system
	messages, system, err := ResponsesToClaudeMessages(sess, req.Input, req.Instructions)
	if err != nil {
		return nil, err
	}

	// 构建 Claude 请求
	claudeReq := map[string]interface{}{
		"model":    req.Model,
		"messages": messages,
		"stream":   req.Stream,
	}

	// Claude 使用独立的 system 参数（不在 messages 中）
	if system != "" {
		claudeReq["system"] = system
	}

	// 复制其他参数
	if req.MaxTokens > 0 {
		claudeReq["max_tokens"] = req.MaxTokens
	}
	if req.Temperature > 0 {
		claudeReq["temperature"] = req.Temperature
	}
	if req.TopP > 0 {
		claudeReq["top_p"] = req.TopP
	}
	if req.Stop != nil {
		claudeReq["stop_sequences"] = req.Stop // Claude 使用 stop_sequences
	}
	if len(req.Tools) > 0 {
		if tools := responsesToolsToClaude(req.Tools); len(tools) > 0 {
			claudeReq["tools"] = tools
		}
	}

	return claudeReq, nil
}

// FromProviderResponse 将 Claude 响应转换为 Responses 格式
func (c *ClaudeConverter) FromProviderResponse(resp map[string]interface{}, sessionID string) (*types.ResponsesResponse, error) {
	return ClaudeResponseToResponses(resp, sessionID)
}

// GetProviderName 获取上游服务名称
func (c *ClaudeConverter) GetProviderName() string {
	return "Claude Messages API"
}
