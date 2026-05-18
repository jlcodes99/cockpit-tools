package common

import (
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

func TestIsClaudeResponseEmpty(t *testing.T) {
	tests := []struct {
		name string
		resp *types.ClaudeResponse
		want bool
	}{
		{"nil", nil, true},
		{"empty content", &types.ClaudeResponse{Content: nil}, true},
		{"single empty text", &types.ClaudeResponse{Content: []types.ClaudeContent{{Type: "text", Text: ""}}}, true},
		{"single brace text", &types.ClaudeResponse{Content: []types.ClaudeContent{{Type: "text", Text: "{"}}}, true},
		{"single text with content", &types.ClaudeResponse{Content: []types.ClaudeContent{{Type: "text", Text: "hello"}}}, false},
		{"tool_use only", &types.ClaudeResponse{Content: []types.ClaudeContent{{Type: "tool_use", Name: "x"}}}, false},
		{"thinking only", &types.ClaudeResponse{Content: []types.ClaudeContent{{Type: "thinking", Thinking: "..."}}}, false},
		{"image block", &types.ClaudeResponse{Content: []types.ClaudeContent{{Type: "image"}}}, false},
		{"mixed empty text + tool_use", &types.ClaudeResponse{Content: []types.ClaudeContent{
			{Type: "text", Text: ""},
			{Type: "tool_use", Name: "x"},
		}}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := IsClaudeResponseEmpty(tt.resp); got != tt.want {
				t.Errorf("IsClaudeResponseEmpty() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestIsChatResponseEmpty(t *testing.T) {
	tests := []struct {
		name string
		body map[string]interface{}
		want bool
	}{
		{"nil", nil, true},
		{"no choices", map[string]interface{}{"id": "x"}, true},
		{"empty choices", map[string]interface{}{"choices": []interface{}{}}, true},
		{"empty message content", map[string]interface{}{
			"choices": []interface{}{
				map[string]interface{}{"message": map[string]interface{}{"role": "assistant", "content": ""}},
			},
		}, true},
		{"only brace content", map[string]interface{}{
			"choices": []interface{}{
				map[string]interface{}{"message": map[string]interface{}{"role": "assistant", "content": "{"}},
			},
		}, true},
		{"valid text content", map[string]interface{}{
			"choices": []interface{}{
				map[string]interface{}{"message": map[string]interface{}{"role": "assistant", "content": "hi"}},
			},
		}, false},
		{"tool_calls", map[string]interface{}{
			"choices": []interface{}{
				map[string]interface{}{"message": map[string]interface{}{
					"role":       "assistant",
					"content":    nil,
					"tool_calls": []interface{}{map[string]interface{}{"id": "1"}},
				}},
			},
		}, false},
		{"reasoning_content", map[string]interface{}{
			"choices": []interface{}{
				map[string]interface{}{"message": map[string]interface{}{
					"role":              "assistant",
					"content":           "",
					"reasoning_content": "thinking...",
				}},
			},
		}, false},
		{"refusal", map[string]interface{}{
			"choices": []interface{}{
				map[string]interface{}{"message": map[string]interface{}{
					"role":    "assistant",
					"content": "",
					"refusal": "I cannot answer that",
				}},
			},
		}, false},
		{"content array with text", map[string]interface{}{
			"choices": []interface{}{
				map[string]interface{}{"message": map[string]interface{}{
					"role": "assistant",
					"content": []interface{}{
						map[string]interface{}{"type": "text", "text": "hello"},
					},
				}},
			},
		}, false},
		{"content array all empty text", map[string]interface{}{
			"choices": []interface{}{
				map[string]interface{}{"message": map[string]interface{}{
					"role": "assistant",
					"content": []interface{}{
						map[string]interface{}{"type": "text", "text": ""},
					},
				}},
			},
		}, true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := IsChatResponseEmpty(tt.body); got != tt.want {
				t.Errorf("IsChatResponseEmpty() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestIsResponsesResponseEmpty(t *testing.T) {
	tests := []struct {
		name string
		resp *types.ResponsesResponse
		want bool
	}{
		{"nil", nil, true},
		{"empty output", &types.ResponsesResponse{Output: nil}, true},
		{"message with empty text", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "message", Content: ""},
		}}, true},
		{"message with text", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "message", Content: "hi"},
		}}, false},
		{"function_call", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "function_call", Name: "tool"},
		}}, false},
		{"reasoning", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "reasoning", Summary: "thought"},
		}}, false},
		{"custom_tool_call (suffix _call)", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "custom_tool_call", Name: "x"},
		}}, false},
		{"message with content blocks text", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "message", Content: []types.ContentBlock{{Type: "output_text", Text: "ok"}}},
		}}, false},
		{"message with content blocks empty", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "message", Content: []types.ContentBlock{{Type: "output_text", Text: ""}}},
		}}, true},
		{"message with non-text part (image)", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "message", Content: []interface{}{
				map[string]interface{}{"type": "input_image", "image_url": "https://x/y.png"},
			}},
		}}, false},
		{"reasoning summary with summary_text only", &types.ResponsesResponse{Output: []types.ResponsesItem{
			{Type: "reasoning", Summary: []interface{}{
				map[string]interface{}{"type": "summary_text", "text": ""},
			}},
		}}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := IsResponsesResponseEmpty(tt.resp); got != tt.want {
				t.Errorf("IsResponsesResponseEmpty() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestIsGeminiResponseEmpty(t *testing.T) {
	tests := []struct {
		name string
		resp *types.GeminiResponse
		want bool
	}{
		{"nil", nil, true},
		{"no candidates", &types.GeminiResponse{}, true},
		{"candidate without content", &types.GeminiResponse{Candidates: []types.GeminiCandidate{{}}}, true},
		{"candidate with empty parts", &types.GeminiResponse{Candidates: []types.GeminiCandidate{
			{Content: &types.GeminiContent{Parts: []types.GeminiPart{{Text: ""}}}},
		}}, true},
		{"candidate with text", &types.GeminiResponse{Candidates: []types.GeminiCandidate{
			{Content: &types.GeminiContent{Parts: []types.GeminiPart{{Text: "hello"}}}},
		}}, false},
		{"candidate with functionCall", &types.GeminiResponse{Candidates: []types.GeminiCandidate{
			{Content: &types.GeminiContent{Parts: []types.GeminiPart{{FunctionCall: &types.GeminiFunctionCall{Name: "f"}}}}},
		}}, false},
		{"safety blocked candidate is preserved (not failover)", &types.GeminiResponse{Candidates: []types.GeminiCandidate{
			{FinishReason: "SAFETY"},
		}}, false},
		{"prompt block reason is preserved", &types.GeminiResponse{
			PromptFeedback: &types.GeminiPromptFeedback{BlockReason: "SAFETY"},
		}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := IsGeminiResponseEmpty(tt.resp); got != tt.want {
				t.Errorf("IsGeminiResponseEmpty() = %v, want %v", got, tt.want)
			}
		})
	}
}
