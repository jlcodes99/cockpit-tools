package providers

import (
	"encoding/json"
	"io"
	"strings"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// TestConvertThinkingToReasoningContent 测试 thinking 块 → reasoning_content 转换
func TestConvertThinkingToReasoningContent(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		wantJSON map[string]interface{}
	}{
		{
			name: "单个 assistant 消息带 thinking 块",
			input: `{
				"model": "mimo-v2.5-pro",
				"messages": [
					{"role": "user", "content": [{"type": "text", "text": "hello"}]},
					{"role": "assistant", "content": [
						{"type": "thinking", "thinking": "let me think", "signature": "sig_123"},
						{"type": "text", "text": "response"}
					]}
				]
			}`,
			wantJSON: map[string]interface{}{
				"model": "mimo-v2.5-pro",
				"messages": []interface{}{
					map[string]interface{}{"role": "user", "content": []interface{}{map[string]interface{}{"type": "text", "text": "hello"}}},
					map[string]interface{}{
						"role": "assistant",
						"content": []interface{}{
							map[string]interface{}{"type": "thinking", "thinking": "let me think", "signature": "sig_123"},
							map[string]interface{}{"type": "text", "text": "response"},
						},
						"reasoning_content": "let me think",
					},
				},
			},
		},
		{
			name: "多个 thinking 块合并",
			input: `{
				"model": "mimo-v2.5-pro",
				"messages": [
					{"role": "assistant", "content": [
						{"type": "thinking", "thinking": "first thought"},
						{"type": "thinking", "thinking": "second thought"},
						{"type": "text", "text": "answer"}
					]}
				]
			}`,
			wantJSON: map[string]interface{}{
				"model": "mimo-v2.5-pro",
				"messages": []interface{}{
					map[string]interface{}{
						"role": "assistant",
						"content": []interface{}{
							map[string]interface{}{"type": "thinking", "thinking": "first thought"},
							map[string]interface{}{"type": "thinking", "thinking": "second thought"},
							map[string]interface{}{"type": "text", "text": "answer"},
						},
						"reasoning_content": "first thought\nsecond thought",
					},
				},
			},
		},
		{
			name: "无 thinking 块不修改",
			input: `{
				"model": "mimo-v2.5-pro",
				"messages": [
					{"role": "assistant", "content": [{"type": "text", "text": "answer"}]}
				]
			}`,
			wantJSON: map[string]interface{}{
				"model": "mimo-v2.5-pro",
				"messages": []interface{}{
					map[string]interface{}{
						"role":    "assistant",
						"content": []interface{}{map[string]interface{}{"type": "text", "text": "answer"}},
					},
				},
			},
		},
		{
			name: "空 thinking 块不转换",
			input: `{
				"model": "mimo-v2.5-pro",
				"messages": [
					{"role": "assistant", "content": [
						{"type": "thinking", "thinking": "", "signature": "sig_empty"},
						{"type": "text", "text": "answer"}
					]}
				]
			}`,
			wantJSON: map[string]interface{}{
				"model": "mimo-v2.5-pro",
				"messages": []interface{}{
					map[string]interface{}{
						"role": "assistant",
						"content": []interface{}{
							map[string]interface{}{"type": "thinking", "thinking": "", "signature": "sig_empty"},
							map[string]interface{}{"type": "text", "text": "answer"},
						},
					},
				},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := convertThinkingToReasoningContent([]byte(tt.input))

			var got map[string]interface{}
			if err := json.Unmarshal(result, &got); err != nil {
				t.Fatalf("unmarshal result: %v", err)
			}

			gotJSON, _ := json.Marshal(got)
			wantJSON, _ := json.Marshal(tt.wantJSON)

			if string(gotJSON) != string(wantJSON) {
				t.Errorf("convertThinkingToReasoningContent() mismatch:\ngot:  %s\nwant: %s", string(gotJSON), string(wantJSON))
			}
		})
	}
}

// TestConvertReasoningContentToThinking 测试 reasoning_content → thinking 块转换
func TestConvertReasoningContentToThinking(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		wantJSON map[string]interface{}
	}{
		{
			name: "顶层 reasoning_content 转为 thinking 块",
			input: `{
				"id": "msg_123",
				"type": "message",
				"role": "assistant",
				"reasoning_content": "internal reasoning",
				"content": [{"type": "text", "text": "answer"}],
				"stop_reason": "end_turn"
			}`,
			wantJSON: map[string]interface{}{
				"id":          "msg_123",
				"type":        "message",
				"role":        "assistant",
				"content":     []interface{}{map[string]interface{}{"type": "thinking", "thinking": "internal reasoning"}, map[string]interface{}{"type": "text", "text": "answer"}},
				"stop_reason": "end_turn",
			},
		},
		{
			name: "content 块中的 reasoning_content 转为 thinking",
			input: `{
				"id": "msg_456",
				"type": "message",
				"role": "assistant",
				"content": [
					{"type": "text", "reasoning_content": "block reasoning", "text": "answer"}
				],
				"stop_reason": "end_turn"
			}`,
			wantJSON: map[string]interface{}{
				"id":   "msg_456",
				"type": "message",
				"role": "assistant",
				"content": []interface{}{
					map[string]interface{}{"type": "thinking", "thinking": "block reasoning", "text": "answer"},
				},
				"stop_reason": "end_turn",
			},
		},
		{
			name: "无 reasoning_content 不修改",
			input: `{
				"id": "msg_789",
				"type": "message",
				"role": "assistant",
				"content": [{"type": "text", "text": "answer"}],
				"stop_reason": "end_turn"
			}`,
			wantJSON: map[string]interface{}{
				"id":          "msg_789",
				"type":        "message",
				"role":        "assistant",
				"content":     []interface{}{map[string]interface{}{"type": "text", "text": "answer"}},
				"stop_reason": "end_turn",
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := convertReasoningContentToThinking([]byte(tt.input))

			var got map[string]interface{}
			if err := json.Unmarshal(result, &got); err != nil {
				t.Fatalf("unmarshal result: %v", err)
			}

			gotJSON, _ := json.Marshal(got)
			wantJSON, _ := json.Marshal(tt.wantJSON)

			if string(gotJSON) != string(wantJSON) {
				t.Errorf("convertReasoningContentToThinking() mismatch:\ngot:  %s\nwant: %s", string(gotJSON), string(wantJSON))
			}
		})
	}
}

// TestClaudeProvider_ConvertToClaudeResponse_WithReasoningContent 测试响应转换
func TestClaudeProvider_ConvertToClaudeResponse_WithReasoningContent(t *testing.T) {
	provider := &ClaudeProvider{}

	t.Run("reasoning_content 转为 thinking 块", func(t *testing.T) {
		providerResp := &types.ProviderResponse{
			StatusCode: 200,
			Body: []byte(`{
				"id": "msg_mimo",
				"type": "message",
				"role": "assistant",
				"reasoning_content": "mimo reasoning",
				"content": [{"type": "text", "text": "mimo answer"}],
				"stop_reason": "end_turn",
				"usage": {"input_tokens": 10, "output_tokens": 20}
			}`),
		}

		claudeResp, err := provider.ConvertToClaudeResponse(providerResp)
		if err != nil {
			t.Fatalf("ConvertToClaudeResponse() error = %v", err)
		}

		if len(claudeResp.Content) != 2 {
			t.Fatalf("content length = %d, want 2", len(claudeResp.Content))
		}

		if claudeResp.Content[0].Type != "thinking" {
			t.Errorf("content[0].type = %q, want thinking", claudeResp.Content[0].Type)
		}
		if claudeResp.Content[0].Thinking != "mimo reasoning" {
			t.Errorf("content[0].thinking = %q, want mimo reasoning", claudeResp.Content[0].Thinking)
		}
		if claudeResp.Content[1].Type != "text" {
			t.Errorf("content[1].type = %q, want text", claudeResp.Content[1].Type)
		}
	})

	t.Run("无 reasoning_content 正常解析", func(t *testing.T) {
		providerResp := &types.ProviderResponse{
			StatusCode: 200,
			Body: []byte(`{
				"id": "msg_normal",
				"type": "message",
				"role": "assistant",
				"content": [{"type": "text", "text": "normal answer"}],
				"stop_reason": "end_turn",
				"usage": {"input_tokens": 5, "output_tokens": 10}
			}`),
		}

		claudeResp, err := provider.ConvertToClaudeResponse(providerResp)
		if err != nil {
			t.Fatalf("ConvertToClaudeResponse() error = %v", err)
		}

		if len(claudeResp.Content) != 1 {
			t.Fatalf("content length = %d, want 1", len(claudeResp.Content))
		}
		if claudeResp.Content[0].Type != "text" {
			t.Errorf("content[0].type = %q, want text", claudeResp.Content[0].Type)
		}
	})
}

// TestClaudeProvider_HandleStreamResponse_WithReasoningContent 测试流式响应转换
func TestClaudeProvider_HandleStreamResponse_WithReasoningContent(t *testing.T) {
	provider := &ClaudeProvider{}

	streamBody := `event: message_start
data: {"type":"message_start","message":{"id":"msg_stream","type":"message","role":"assistant","content":[]}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","reasoning_content":"stream reasoning"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"stream answer"}}

event: message_stop
data: {"type":"message_stop"}

`

	eventChan, errChan, err := provider.HandleStreamResponse(io.NopCloser(strings.NewReader(streamBody)))
	if err != nil {
		t.Fatalf("HandleStreamResponse() error = %v", err)
	}

	var events []string
	for {
		select {
		case event, ok := <-eventChan:
			if !ok {
				goto done
			}
			events = append(events, event)
		case err := <-errChan:
			if err != nil {
				t.Fatalf("stream error: %v", err)
			}
		}
	}
done:

	joined := strings.Join(events, "")

	// 验证 reasoning_content 被转换为 thinking_delta
	if !strings.Contains(joined, `"type":"thinking_delta"`) {
		t.Errorf("expected thinking_delta in stream, got: %s", joined)
	}
	if !strings.Contains(joined, `"thinking":"stream reasoning"`) {
		t.Errorf("expected thinking field in stream, got: %s", joined)
	}
	if strings.Contains(joined, `"reasoning_content"`) {
		t.Errorf("reasoning_content should be removed from stream, got: %s", joined)
	}
}
