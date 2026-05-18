package converters

import (
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// ============== Responses 透传转换器 ==============

// ResponsesPassthroughConverter 实现 Responses → Responses 透传
// 用于上游服务本身就是 Responses API 的场景
type ResponsesPassthroughConverter struct{}

// ToProviderRequest 透传 Responses 请求（不做转换）
func (c *ResponsesPassthroughConverter) ToProviderRequest(sess *session.Session, req *types.ResponsesRequest) (interface{}, error) {
	// 直接返回原始请求
	return map[string]interface{}{
		"model":                req.Model,
		"instructions":         req.Instructions,
		"input":                req.Input,
		"previous_response_id": req.PreviousResponseID,
		"store":                req.Store,
		"max_tokens":           req.MaxTokens,
		"temperature":          req.Temperature,
		"top_p":                req.TopP,
		"frequency_penalty":    req.FrequencyPenalty,
		"presence_penalty":     req.PresencePenalty,
		"stream":               req.Stream,
		"stop":                 req.Stop,
		"user":                 req.User,
		"stream_options":       req.StreamOptions,
	}, nil
}

// FromProviderResponse 透传 Responses 响应（不做转换）
func (c *ResponsesPassthroughConverter) FromProviderResponse(resp map[string]interface{}, sessionID string) (*types.ResponsesResponse, error) {
	// 直接解析为 ResponsesResponse
	// 注意：这里假设上游返回的就是标准 Responses 格式
	id, _ := resp["id"].(string)
	model, _ := resp["model"].(string)
	status, _ := resp["status"].(string)
	previousID, _ := resp["previous_id"].(string)

	// 解析 output
	output := []types.ResponsesItem{}
	if outputArr, ok := resp["output"].([]interface{}); ok {
		for _, item := range outputArr {
			if itemMap, ok := item.(map[string]interface{}); ok {
				itemType, _ := itemMap["type"].(string)
				role, _ := itemMap["role"].(string)
				content := itemMap["content"]

				output = append(output, types.ResponsesItem{
					Type:    itemType,
					Role:    role,
					Content: content,
				})
			}
		}
	}

	// 解析 usage（使用统一入口自动检测格式：Claude/Gemini/OpenAI）
	usage := ExtractUsageMetrics(resp["usage"])

	return &types.ResponsesResponse{
		ID:         id,
		Model:      model,
		Output:     output,
		Status:     status,
		PreviousID: previousID,
		Usage:      usage,
	}, nil
}

// GetProviderName 获取上游服务名称
func (c *ResponsesPassthroughConverter) GetProviderName() string {
	return "Responses API (Passthrough)"
}
