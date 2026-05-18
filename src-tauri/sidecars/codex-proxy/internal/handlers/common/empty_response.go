// Package common 提供 handlers 模块的公共功能
package common

import (
	"errors"
	"strings"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// ErrEmptyNonStreamResponse 上游返回 HTTP 200 且 JSON 可解析，但语义内容为空。
// 语义空定义：无任何可交付文本、无 tool_use / function_call、无 thinking / reasoning。
// 下游拿到这类响应会立即中断，所以在 Header 尚未写入客户端之前返回此错误，
// 触发 TryUpstreamWithAllKeys 进行 Key/BaseURL/渠道级 failover。
//
// 仅在 Fuzzy 模式下启用，严格模式保留原行为（保证对上游"合法空回复"的语义透传）。
var ErrEmptyNonStreamResponse = errors.New("upstream returned empty non-stream response")

// IsClaudeResponseEmpty 判定 Claude Messages 非流式响应是否在语义上为空。
// 规则：
//   - content 为空或全部为无文本 text 块，且
//   - 不包含 tool_use / thinking 等语义块
func IsClaudeResponseEmpty(resp *types.ClaudeResponse) bool {
	if resp == nil {
		return true
	}
	if len(resp.Content) == 0 {
		return true
	}
	for _, block := range resp.Content {
		switch block.Type {
		case "text", "":
			if !IsEffectivelyEmptyStreamText(block.Text) {
				return false
			}
		case "tool_use", "server_tool_use", "thinking", "redacted_thinking":
			return false
		default:
			// 其他非文本块（image / document 等）视为有效内容
			return false
		}
	}
	return true
}

// IsChatResponseEmpty 判定 OpenAI Chat 非流式响应是否在语义上为空。
// 输入为上游 JSON 反序列化后的 map[string]interface{}。
// 规则（任一满足语义非空即返回 false）：
//   - choices[*].message.content 存在非空文本
//   - choices[*].message.tool_calls 非空
//   - choices[*].message.reasoning_content 非空
//   - choices[*].message.refusal 非空（保留错误语义）
func IsChatResponseEmpty(respMap map[string]interface{}) bool {
	if respMap == nil {
		return true
	}
	choices, _ := respMap["choices"].([]interface{})
	if len(choices) == 0 {
		return true
	}
	for _, ch := range choices {
		choice, ok := ch.(map[string]interface{})
		if !ok {
			continue
		}
		msg, _ := choice["message"].(map[string]interface{})
		if msg == nil {
			// 兼容流式 delta 形态（正常不会出现在非流式，这里保守放行）
			if _, ok := choice["delta"].(map[string]interface{}); ok {
				return false
			}
			continue
		}
		if !isChatMessageEmpty(msg) {
			return false
		}
	}
	return true
}

// isChatMessageEmpty 判定单条 OpenAI Chat assistant 消息是否语义为空。
func isChatMessageEmpty(msg map[string]interface{}) bool {
	// 1. content 字段：string 非空即视为有效；数组则检查每一项
	switch v := msg["content"].(type) {
	case string:
		if !IsEffectivelyEmptyStreamText(v) {
			return false
		}
	case []interface{}:
		for _, part := range v {
			partMap, ok := part.(map[string]interface{})
			if !ok {
				continue
			}
			if text, ok := partMap["text"].(string); ok && !IsEffectivelyEmptyStreamText(text) {
				return false
			}
			// image_url / input_audio 等非文本块视为有效
			if t, ok := partMap["type"].(string); ok && t != "" && t != "text" && t != "output_text" {
				return false
			}
		}
	}

	// 2. tool_calls 非空即视为有效
	if calls, ok := msg["tool_calls"].([]interface{}); ok && len(calls) > 0 {
		return false
	}

	// 3. reasoning_content / refusal 非空也视为有效
	if rc, ok := msg["reasoning_content"].(string); ok && !IsEffectivelyEmptyStreamText(rc) {
		return false
	}
	if refusal, ok := msg["refusal"].(string); ok && strings.TrimSpace(refusal) != "" {
		return false
	}

	return true
}

// IsResponsesResponseEmpty 判定 Codex Responses 非流式响应是否在语义上为空。
// 规则：output 为空，或全部 item 均无文本且非 function_call / reasoning / *_call 语义项。
func IsResponsesResponseEmpty(resp *types.ResponsesResponse) bool {
	if resp == nil {
		return true
	}
	if len(resp.Output) == 0 {
		return true
	}
	for _, item := range resp.Output {
		if !isResponsesItemEmpty(item) {
			return false
		}
	}
	return true
}

// isResponsesItemEmpty 判定单个 ResponsesItem 是否语义为空。
func isResponsesItemEmpty(item types.ResponsesItem) bool {
	itemType := item.Type
	switch itemType {
	case "function_call", "reasoning":
		return false
	}
	if strings.HasSuffix(itemType, "_call") {
		return false
	}
	// message / text 类：检查 content
	switch v := item.Content.(type) {
	case string:
		if !IsEffectivelyEmptyStreamText(v) {
			return false
		}
	case []interface{}:
		for _, part := range v {
			partMap, ok := part.(map[string]interface{})
			if !ok {
				continue
			}
			if text, ok := partMap["text"].(string); ok && !IsEffectivelyEmptyStreamText(text) {
				return false
			}
			// 非文本 part（image / input_image / input_file 等）视为有效内容
			if t, ok := partMap["type"].(string); ok && t != "" && t != "text" && t != "input_text" && t != "output_text" {
				return false
			}
		}
	case []types.ContentBlock:
		for _, part := range v {
			if !IsEffectivelyEmptyStreamText(part.Text) {
				return false
			}
		}
	}
	// Summary 存在非空文本视为有效（reasoning_summary）
	switch s := item.Summary.(type) {
	case string:
		if !IsEffectivelyEmptyStreamText(s) {
			return false
		}
	case []interface{}:
		for _, part := range s {
			partMap, ok := part.(map[string]interface{})
			if !ok {
				continue
			}
			if text, ok := partMap["text"].(string); ok && !IsEffectivelyEmptyStreamText(text) {
				return false
			}
			if t, ok := partMap["type"].(string); ok && t != "" && t != "text" && t != "input_text" && t != "output_text" && t != "summary_text" {
				return false
			}
		}
	}
	return true
}

// IsGeminiResponseEmpty 判定 Gemini 非流式响应是否在语义上为空。
// 规则：candidates 为空，或所有 candidate 均无文本且无 functionCall。
// 注意：prompt 被 block（promptFeedback.blockReason）时视为非空——内容审核错误应按上游原样透传。
func IsGeminiResponseEmpty(resp *types.GeminiResponse) bool {
	if resp == nil {
		return true
	}
	if resp.PromptFeedback != nil && strings.TrimSpace(resp.PromptFeedback.BlockReason) != "" {
		return false
	}
	if len(resp.Candidates) == 0 {
		return true
	}
	for _, cand := range resp.Candidates {
		// 安全拦截 / 版权拦截也应原样透传，不触发 failover
		switch cand.FinishReason {
		case "SAFETY", "RECITATION", "BLOCKLIST", "PROHIBITED_CONTENT", "SPII":
			return false
		}
		if cand.Content == nil {
			continue
		}
		for _, part := range cand.Content.Parts {
			if !IsEffectivelyEmptyStreamText(part.Text) {
				return false
			}
			if part.FunctionCall != nil {
				return false
			}
			if part.InlineData != nil || part.FileData != nil {
				return false
			}
		}
	}
	return true
}
