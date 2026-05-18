package converters

import (
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// ============== OpenAI Chat Completions 转换器 ==============

// OpenAIChatConverter 实现 Responses → OpenAI Chat Completions 转换
type OpenAIChatConverter struct{}

// ToProviderRequest 将 Responses 请求转换为 OpenAI Chat Completions 格式
func (c *OpenAIChatConverter) ToProviderRequest(sess *session.Session, req *types.ResponsesRequest) (interface{}, error) {
	// 转换 messages
	messages, err := ResponsesToOpenAIChatMessages(sess, req.Input, req.Instructions)
	if err != nil {
		return nil, err
	}

	// 构建 OpenAI 请求
	openaiReq := map[string]interface{}{
		"model":    req.Model,
		"messages": messages,
		"stream":   req.Stream,
	}

	// 复制其他参数
	if req.MaxTokens > 0 {
		openaiReq["max_tokens"] = req.MaxTokens
	}
	if req.Temperature > 0 {
		openaiReq["temperature"] = req.Temperature
	}
	if req.TopP > 0 {
		openaiReq["top_p"] = req.TopP
	}
	if req.FrequencyPenalty != 0 {
		openaiReq["frequency_penalty"] = req.FrequencyPenalty
	}
	if req.PresencePenalty != 0 {
		openaiReq["presence_penalty"] = req.PresencePenalty
	}
	if req.Stop != nil {
		openaiReq["stop"] = req.Stop
	}
	if req.User != "" {
		openaiReq["user"] = req.User
	}
	if req.StreamOptions != nil {
		openaiReq["stream_options"] = req.StreamOptions
	}

	// Tools conversion with Codex custom tool support.
	codexEnabled := false
	if req.TransformerMetadata != nil {
		if v, ok := req.TransformerMetadata["codex_tool_compat_enabled"].(bool); ok {
			codexEnabled = v
		}
	}
	var codexToolCtx CodexToolContext
	if codexEnabled {
		codexToolCtx = BuildCodexToolContextFromRaw(req.RawTools)
	}
	if len(req.RawTools) > 0 {
		if codexEnabled {
			if tools := responsesRawToolsToOpenAIWithContext(req.RawTools, codexToolCtx); len(tools) > 0 {
				openaiReq["tools"] = tools
			}
		} else if tools := responsesRawToolsToOpenAI(req.RawTools); len(tools) > 0 {
			openaiReq["tools"] = tools
		}
	} else if len(req.Tools) > 0 {
		if codexEnabled {
			if tools := responsesToolsToOpenAIWithContext(req.Tools, codexToolCtx); len(tools) > 0 {
				openaiReq["tools"] = tools
			}
		} else if tools := responsesToolsToOpenAI(req.Tools); len(tools) > 0 {
			openaiReq["tools"] = tools
		}
	}
	if req.ToolChoice != nil {
		if codexEnabled {
			if converted := ConvertToolChoiceForCodex(req.ToolChoice, codexToolCtx); converted != nil {
				openaiReq["tool_choice"] = converted
			} else {
				openaiReq["tool_choice"] = req.ToolChoice
			}
		} else {
			openaiReq["tool_choice"] = req.ToolChoice
		}
	}
	if req.ParallelToolCalls != nil {
		openaiReq["parallel_tool_calls"] = *req.ParallelToolCalls
	}

	// Store CodexToolContext in TransformerMetadata for response conversion.
	if codexToolCtx.HasCustomTools {
		if req.TransformerMetadata == nil {
			req.TransformerMetadata = make(map[string]interface{})
		}
		req.TransformerMetadata["codex_tool_context"] = codexToolCtx
	}

	return openaiReq, nil
}

// FromProviderResponse 将 OpenAI Chat 响应转换为 Responses 格式
func (c *OpenAIChatConverter) FromProviderResponse(resp map[string]interface{}, sessionID string) (*types.ResponsesResponse, error) {
	return OpenAIChatResponseToResponses(resp, sessionID)
}

// GetProviderName 获取上游服务名称
func (c *OpenAIChatConverter) GetProviderName() string {
	return "OpenAI Chat Completions"
}

// ============== OpenAI Completions 转换器 ==============

// OpenAICompletionsConverter 实现 Responses → OpenAI Completions 转换
type OpenAICompletionsConverter struct{}

// ToProviderRequest 将 Responses 请求转换为 OpenAI Completions 格式
func (c *OpenAICompletionsConverter) ToProviderRequest(sess *session.Session, req *types.ResponsesRequest) (interface{}, error) {
	// 提取纯文本（Completions API 不支持 messages）
	prompt, err := ExtractTextFromResponses(sess, req.Input)
	if err != nil {
		return nil, err
	}

	// 如果有 instructions，添加到 prompt 前面
	if req.Instructions != "" {
		prompt = req.Instructions + "\n\n" + prompt
	}

	// 构建 OpenAI Completions 请求
	completionsReq := map[string]interface{}{
		"model":  req.Model,
		"prompt": prompt,
		"stream": req.Stream,
	}

	// 复制其他参数
	if req.MaxTokens > 0 {
		completionsReq["max_tokens"] = req.MaxTokens
	}
	if req.Temperature > 0 {
		completionsReq["temperature"] = req.Temperature
	}
	if req.TopP > 0 {
		completionsReq["top_p"] = req.TopP
	}
	if req.FrequencyPenalty != 0 {
		completionsReq["frequency_penalty"] = req.FrequencyPenalty
	}
	if req.PresencePenalty != 0 {
		completionsReq["presence_penalty"] = req.PresencePenalty
	}
	if req.Stop != nil {
		completionsReq["stop"] = req.Stop
	}
	if req.User != "" {
		completionsReq["user"] = req.User
	}

	return completionsReq, nil
}

// FromProviderResponse 将 OpenAI Completions 响应转换为 Responses 格式
func (c *OpenAICompletionsConverter) FromProviderResponse(resp map[string]interface{}, sessionID string) (*types.ResponsesResponse, error) {
	return OpenAICompletionsResponseToResponses(resp, sessionID)
}

// GetProviderName 获取上游服务名称
func (c *OpenAICompletionsConverter) GetProviderName() string {
	return "OpenAI Completions"
}
