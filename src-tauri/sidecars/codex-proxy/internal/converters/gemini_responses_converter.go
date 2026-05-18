package converters

import (
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// ============== Gemini API 转换器 ==============

// GeminiResponsesConverter 实现 Responses → Gemini API 转换
// 用于 Responses handler 将请求转发到 Gemini 上游
type GeminiResponsesConverter struct{}

// ToProviderRequest 将 Responses 请求转换为 Gemini 格式
func (g *GeminiResponsesConverter) ToProviderRequest(sess *session.Session, req *types.ResponsesRequest) (interface{}, error) {
	geminiReq, err := ResponsesToGeminiRequest(sess, req, req.Model)
	if err != nil {
		return nil, err
	}

	// 确保 thought_signature 字段存在
	for i := range geminiReq.Contents {
		for j := range geminiReq.Contents[i].Parts {
			part := &geminiReq.Contents[i].Parts[j]
			if part.FunctionCall != nil && part.FunctionCall.ThoughtSignature == "" {
				part.FunctionCall.ThoughtSignature = types.DummyThoughtSignature
			}
		}
	}

	return geminiReq, nil
}

// FromProviderResponse 将 Gemini 响应转换为 Responses 格式
func (g *GeminiResponsesConverter) FromProviderResponse(resp map[string]interface{}, sessionID string) (*types.ResponsesResponse, error) {
	return GeminiResponseToResponses(resp, sessionID)
}

// GetProviderName 获取上游服务名称
func (g *GeminiResponsesConverter) GetProviderName() string {
	return "Gemini API"
}
