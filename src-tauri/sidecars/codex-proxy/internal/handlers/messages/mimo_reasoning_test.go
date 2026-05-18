package messages

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

// TestMessagesHandler_MimoReasoningContentPassback 测试 mimo 场景的 reasoning_content 回传
func TestMessagesHandler_MimoReasoningContentPassback(t *testing.T) {
	tests := []struct {
		name             string
		passbackEnabled  bool
		requestBody      string
		upstreamResponse string
		wantUpstream     func(t *testing.T, body []byte)
		wantDownstream   func(t *testing.T, body []byte)
	}{
		{
			name:            "passbackReasoningContent=true 时 thinking 块转为 reasoning_content",
			passbackEnabled: true,
			requestBody: `{
				"model": "mimo-v2.5-pro",
				"messages": [
					{"role": "user", "content": [{"type": "text", "text": "hello"}]},
					{"role": "assistant", "content": [
						{"type": "thinking", "thinking": "previous reasoning", "signature": "sig_prev"},
						{"type": "text", "text": "previous answer"}
					]}
				]
			}`,
			upstreamResponse: `{
				"id": "msg_mimo",
				"type": "message",
				"role": "assistant",
				"reasoning_content": "mimo reasoning",
				"content": [{"type": "text", "text": "mimo answer"}],
				"stop_reason": "end_turn",
				"usage": {"input_tokens": 10, "output_tokens": 20}
			}`,
			wantUpstream: func(t *testing.T, body []byte) {
				var req map[string]interface{}
				if err := json.Unmarshal(body, &req); err != nil {
					t.Fatalf("unmarshal upstream request: %v", err)
				}

				messages, ok := req["messages"].([]interface{})
				if !ok || len(messages) < 2 {
					t.Fatalf("messages shape invalid: %s", string(body))
				}

				assistant, ok := messages[1].(map[string]interface{})
				if !ok {
					t.Fatalf("assistant message shape invalid: %s", string(body))
				}

				// 验证 reasoning_content 字段存在
				if reasoningContent, ok := assistant["reasoning_content"].(string); !ok || reasoningContent != "previous reasoning" {
					t.Fatalf("reasoning_content = %v, want 'previous reasoning'; body=%s", assistant["reasoning_content"], string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				var resp map[string]interface{}
				if err := json.Unmarshal(body, &resp); err != nil {
					t.Fatalf("unmarshal downstream response: %v", err)
				}

				content, ok := resp["content"].([]interface{})
				if !ok || len(content) < 2 {
					t.Fatalf("content shape invalid: %s", string(body))
				}

				// 验证第一个块是 thinking 块
				firstBlock, ok := content[0].(map[string]interface{})
				if !ok {
					t.Fatalf("first content block shape invalid: %s", string(body))
				}

				if blockType, _ := firstBlock["type"].(string); blockType != "thinking" {
					t.Fatalf("first block type = %q, want thinking; body=%s", blockType, string(body))
				}

				if thinking, _ := firstBlock["thinking"].(string); thinking != "mimo reasoning" {
					t.Fatalf("thinking = %q, want 'mimo reasoning'; body=%s", thinking, string(body))
				}

				// 验证 reasoning_content 已被移除
				if _, exists := resp["reasoning_content"]; exists {
					t.Fatalf("reasoning_content should be removed from response, got: %s", string(body))
				}
			},
		},
		{
			name:            "passbackReasoningContent=false 时保持透传",
			passbackEnabled: false,
			requestBody: `{
				"model": "claude-opus-4-7",
				"messages": [
					{"role": "user", "content": [{"type": "text", "text": "hello"}]},
					{"role": "assistant", "content": [
						{"type": "thinking", "thinking": "previous reasoning", "signature": "sig_prev"},
						{"type": "text", "text": "previous answer"}
					]}
				]
			}`,
			upstreamResponse: `{
				"id": "msg_claude",
				"type": "message",
				"role": "assistant",
				"content": [
					{"type": "thinking", "thinking": "claude reasoning", "signature": "sig_claude"},
					{"type": "text", "text": "claude answer"}
				],
				"stop_reason": "end_turn",
				"usage": {"input_tokens": 10, "output_tokens": 20}
			}`,
			wantUpstream: func(t *testing.T, body []byte) {
				var req map[string]interface{}
				if err := json.Unmarshal(body, &req); err != nil {
					t.Fatalf("unmarshal upstream request: %v", err)
				}

				messages, ok := req["messages"].([]interface{})
				if !ok || len(messages) < 2 {
					t.Fatalf("messages shape invalid: %s", string(body))
				}

				assistant, ok := messages[1].(map[string]interface{})
				if !ok {
					t.Fatalf("assistant message shape invalid: %s", string(body))
				}

				// 验证 reasoning_content 字段不存在（保持 Claude 原生格式）
				if _, exists := assistant["reasoning_content"]; exists {
					t.Fatalf("reasoning_content should not exist when passbackReasoningContent=false; body=%s", string(body))
				}

				// 验证 thinking 块保留
				content, ok := assistant["content"].([]interface{})
				if !ok || len(content) < 1 {
					t.Fatalf("assistant content invalid: %s", string(body))
				}

				firstBlock, ok := content[0].(map[string]interface{})
				if !ok {
					t.Fatalf("first content block shape invalid: %s", string(body))
				}

				if blockType, _ := firstBlock["type"].(string); blockType != "thinking" {
					t.Fatalf("first block type = %q, want thinking; body=%s", blockType, string(body))
				}
			},
			wantDownstream: func(t *testing.T, body []byte) {
				var resp map[string]interface{}
				if err := json.Unmarshal(body, &resp); err != nil {
					t.Fatalf("unmarshal downstream response: %v", err)
				}

				// 验证响应保持 Claude 原生格式（thinking 块）
				content, ok := resp["content"].([]interface{})
				if !ok || len(content) < 2 {
					t.Fatalf("content shape invalid: %s", string(body))
				}

				firstBlock, ok := content[0].(map[string]interface{})
				if !ok {
					t.Fatalf("first content block shape invalid: %s", string(body))
				}

				if blockType, _ := firstBlock["type"].(string); blockType != "thinking" {
					t.Fatalf("first block type = %q, want thinking; body=%s", blockType, string(body))
				}
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var capturedUpstreamBody []byte
			upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				body, err := io.ReadAll(r.Body)
				if err != nil {
					t.Fatalf("read upstream request: %v", err)
				}
				capturedUpstreamBody = body

				w.Header().Set("Content-Type", "application/json")
				w.WriteHeader(http.StatusOK)
				_, _ = w.Write([]byte(tt.upstreamResponse))
			}))
			defer upstream.Close()

			router := newMessagesTestRouter(t, config.UpstreamConfig{
				Name:                     tt.name,
				BaseURL:                  upstream.URL,
				APIKeys:                  []string{"sk-test"},
				ServiceType:              "claude",
				Status:                   "active",
				PassbackReasoningContent: tt.passbackEnabled,
			})

			w := performMessagesHandlerRequest(t, router, tt.requestBody)
			if w.Code != http.StatusOK {
				t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
			}

			tt.wantUpstream(t, capturedUpstreamBody)
			tt.wantDownstream(t, w.Body.Bytes())
		})
	}
}

// TestMessagesHandler_MimoStreamReasoningContentPassback 测试 mimo 流式场景的 reasoning_content 回传
func TestMessagesHandler_MimoStreamReasoningContentPassback(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)

		flusher, ok := w.(http.Flusher)
		if !ok {
			t.Fatal("ResponseWriter does not support flushing")
		}

		// 模拟 mimo 返回带 reasoning_content 的流式响应
		events := []string{
			"event: message_start\n",
			`data: {"type":"message_start","message":{"id":"msg_stream","type":"message","role":"assistant","content":[]}}` + "\n\n",
			"event: content_block_delta\n",
			`data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","reasoning_content":"stream reasoning"}}` + "\n\n",
			"event: content_block_delta\n",
			`data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"stream answer"}}` + "\n\n",
			"event: message_stop\n",
			`data: {"type":"message_stop"}` + "\n\n",
		}

		for _, event := range events {
			_, _ = w.Write([]byte(event))
			flusher.Flush()
		}
	}))
	defer upstream.Close()

	router := newMessagesTestRouter(t, config.UpstreamConfig{
		Name:                     "mimo-stream",
		BaseURL:                  upstream.URL,
		APIKeys:                  []string{"sk-test"},
		ServiceType:              "claude",
		Status:                   "active",
		PassbackReasoningContent: true,
	})

	reqBody := `{"model":"mimo-v2.5-pro","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],"stream":true}`
	w := performMessagesHandlerRequest(t, router, reqBody)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body=%s", w.Code, w.Body.String())
	}

	body := w.Body.String()

	// 验证 reasoning_content 被转换为 thinking_delta
	if !bytes.Contains([]byte(body), []byte(`"type":"thinking_delta"`)) {
		t.Errorf("expected thinking_delta in stream response, got: %s", body)
	}

	if !bytes.Contains([]byte(body), []byte(`"thinking":"stream reasoning"`)) {
		t.Errorf("expected thinking field in stream response, got: %s", body)
	}

	// 验证 reasoning_content 已被移除
	if bytes.Contains([]byte(body), []byte(`"reasoning_content"`)) {
		t.Errorf("reasoning_content should be removed from stream response, got: %s", body)
	}
}
