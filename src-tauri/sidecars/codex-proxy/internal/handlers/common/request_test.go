package common

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/http/httptrace"
	"sync/atomic"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

func TestNormalizeMetadataUserID(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		expected string // expected user_id value after normalization, empty means unchanged
	}{
		{
			name:     "v2.1.78 JSON - only device_id",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{\"device_id\":\"b854c106939c\",\"account_uuid\":\"\",\"session_id\":\"\"}"},"stream":true}`,
			expected: "user_b854c106939c",
		},
		{
			name:     "v2.1.78 JSON - device_id + session_id",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{\"device_id\":\"b854c106939c\",\"account_uuid\":\"\",\"session_id\":\"e692f803-4767\"}"}}`,
			expected: "user_b854c106939c_session_e692f803-4767",
		},
		{
			name:     "v2.1.78 JSON - device_id + account_uuid",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{\"device_id\":\"dev1\",\"account_uuid\":\"acc1\",\"session_id\":\"\"}"}}`,
			expected: "user_dev1_account_acc1",
		},
		{
			name:     "v2.1.78 JSON - all fields populated",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{\"device_id\":\"abc123\",\"account_uuid\":\"uuid-456\",\"session_id\":\"sess-789\"}"}}`,
			expected: "user_abc123_account_uuid-456_session_sess-789",
		},
		{
			name:     "v2.1.78 JSON - empty device_id, fallback to generic format",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{\"device_id\":\"\",\"account_uuid\":\"acc1\",\"session_id\":\"sess1\"}"}}`,
			expected: "account_uuid_acc1_session_id_sess1",
		},
		{
			name:     "v2.1.77 flat string user_id - no change",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"user_67bad5_account__session_7581b58b"},"stream":true}`,
			expected: "user_67bad5_account__session_7581b58b",
		},
		{
			name:     "no metadata - no change",
			input:    `{"model":"claude-opus-4-6","stream":true}`,
			expected: "",
		},
		{
			name:     "empty user_id - no change",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":""},"stream":true}`,
			expected: "",
		},
		{
			name:     "invalid JSON in user_id - no change",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{invalid json"}}`,
			expected: "{invalid json",
		},
		{
			name:     "non-claude JSON object - generic key_value format",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{\"uid\":\"abc123\"}"}}`,
			expected: "uid_abc123",
		},
		{
			name:     "non-claude JSON object - multiple fields sorted",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{\"session\":\"xyz\",\"uid\":\"abc123\"}"}}`,
			expected: "session_xyz_uid_abc123",
		},
		{
			name:     "preserves other fields",
			input:    `{"model":"claude-opus-4-6","metadata":{"user_id":"{\"device_id\":\"dev1\",\"account_uuid\":\"acc1\",\"session_id\":\"sess1\"}"},"stream":true,"max_tokens":1024}`,
			expected: "user_dev1_account_acc1_session_sess1",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := NormalizeMetadataUserID([]byte(tt.input))

			if tt.expected == "" {
				// Should be unchanged or no user_id
				var data map[string]interface{}
				if err := json.Unmarshal(result, &data); err != nil {
					t.Fatalf("failed to parse result: %v", err)
				}
				metadata, ok := data["metadata"].(map[string]interface{})
				if !ok {
					return // no metadata, as expected
				}
				userID, _ := metadata["user_id"].(string)
				if userID != "" {
					var origData map[string]interface{}
					json.Unmarshal([]byte(tt.input), &origData)
					origMeta, _ := origData["metadata"].(map[string]interface{})
					origUID, _ := origMeta["user_id"].(string)
					if userID != origUID {
						t.Errorf("user_id changed unexpectedly: got %q, want %q", userID, origUID)
					}
				}
				return
			}

			var data map[string]interface{}
			if err := json.Unmarshal(result, &data); err != nil {
				t.Fatalf("failed to parse result: %v", err)
			}
			metadata, ok := data["metadata"].(map[string]interface{})
			if !ok {
				t.Fatal("metadata not found in result")
			}
			userID, ok := metadata["user_id"].(string)
			if !ok {
				t.Fatal("user_id not found in metadata")
			}
			if userID != tt.expected {
				t.Errorf("user_id = %q, want %q", userID, tt.expected)
			}

			// Verify other fields are preserved
			var origData map[string]interface{}
			json.Unmarshal([]byte(tt.input), &origData)
			if origModel, ok := origData["model"].(string); ok {
				if resultModel, ok := data["model"].(string); ok {
					if origModel != resultModel {
						t.Errorf("model changed: got %q, want %q", resultModel, origModel)
					}
				}
			}
		})
	}
}

func TestExtractUnifiedSessionID(t *testing.T) {
	gin.SetMode(gin.TestMode)

	tests := []struct {
		name     string
		headers  map[string]string
		body     string
		expected string
	}{
		{
			name:     "conversation header has highest priority",
			headers:  map[string]string{"Conversation_id": "conv_1", "Session_id": "sess_1"},
			body:     `{"user":"body_user","prompt_cache_key":"cache_1","metadata":{"user_id":"meta_1"}}`,
			expected: "conv_1",
		},
		{
			name:     "session header outranks claude code session header",
			headers:  map[string]string{"Session_id": "sess_2", "X-Claude-Code-Session-Id": "claude_2"},
			body:     `{"user":"body_user"}`,
			expected: "sess_2",
		},
		{
			name:     "claude code session header outranks client request id",
			headers:  map[string]string{"X-Claude-Code-Session-Id": "claude_3", "X-Client-Request-Id": "req_3"},
			body:     `{"prompt_cache_key":"cache_3"}`,
			expected: "claude_3",
		},
		{
			name:     "client request id outranks gemini privileged user",
			headers:  map[string]string{"X-Client-Request-Id": "req_4", "X-Gemini-Api-Privileged-User-Id": "gemini_4"},
			body:     `{}`,
			expected: "req_4",
		},
		{
			name:     "body user outranks prompt cache key and metadata user id",
			headers:  map[string]string{},
			body:     `{"user":"body_user","prompt_cache_key":"cache_5","metadata":{"user_id":"meta_5"}}`,
			expected: "body_user",
		},
		{
			name:     "prompt cache key outranks metadata user id",
			headers:  map[string]string{},
			body:     `{"prompt_cache_key":"cache_6","metadata":{"user_id":"meta_6"}}`,
			expected: "cache_6",
		},
		{
			name:     "metadata user id is final fallback",
			headers:  map[string]string{},
			body:     `{"metadata":{"user_id":"meta_7"}}`,
			expected: "meta_7",
		},
		{
			name:     "metadata user id object falls back to flattened value after user and prompt cache key",
			headers:  map[string]string{},
			body:     `{"metadata":{"user_id":{"device_id":"dev1","account_uuid":"acc1","session_id":"sess1"}}}`,
			expected: "user_dev1_account_acc1_session_sess1",
		},
		{
			name:     "invalid metadata user id type does not discard valid user",
			headers:  map[string]string{},
			body:     `{"user":"body_user","metadata":{"user_id":{"device_id":"dev1"}}}`,
			expected: "body_user",
		},
		{
			name:     "invalid metadata user id type does not discard valid prompt cache key",
			headers:  map[string]string{},
			body:     `{"prompt_cache_key":"cache_8","metadata":{"user_id":{"device_id":"dev1"}}}`,
			expected: "cache_8",
		},
		{
			name:     "empty request returns empty string",
			headers:  map[string]string{},
			body:     `{}`,
			expected: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, "/v1/messages", bytes.NewBufferString(tt.body))
			for k, v := range tt.headers {
				c.Request.Header.Set(k, v)
			}

			if got := utils.ExtractUnifiedSessionID(c, []byte(tt.body)); got != tt.expected {
				t.Fatalf("ExtractUnifiedSessionID() = %q, want %q", got, tt.expected)
			}
		})
	}
}

func TestPassthroughResponse(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)

	resp := &http.Response{
		StatusCode: http.StatusAccepted,
		Header:     http.Header{"X-Test": []string{"ok"}},
		Body:       io.NopCloser(bytes.NewBufferString(`{"ok":true}`)),
	}

	if err := PassthroughResponse(c, resp); err != nil {
		t.Fatalf("PassthroughResponse() err = %v", err)
	}

	if w.Code != http.StatusAccepted {
		t.Fatalf("status = %d, want %d", w.Code, http.StatusAccepted)
	}
	if got := w.Header().Get("X-Test"); got != "ok" {
		t.Fatalf("header X-Test = %q, want ok", got)
	}
	if got := w.Body.String(); got != `{"ok":true}` {
		t.Fatalf("body = %q", got)
	}
}

func TestPassthroughJSONResponse(t *testing.T) {
	gin.SetMode(gin.TestMode)

	t.Run("decode success", func(t *testing.T) {
		w := httptest.NewRecorder()
		c, _ := gin.CreateTestContext(w)
		resp := &http.Response{
			StatusCode: http.StatusOK,
			Header:     http.Header{"X-Test": []string{"ok"}},
			Body:       io.NopCloser(bytes.NewBufferString(`{"usage":{"prompt_tokens":12,"completion_tokens":34}}`)),
		}

		var got map[string]interface{}
		if err := PassthroughJSONResponse(c, resp, &got); err != nil {
			t.Fatalf("PassthroughJSONResponse() err = %v", err)
		}

		if w.Body.String() != `{"usage":{"prompt_tokens":12,"completion_tokens":34}}` {
			t.Fatalf("unexpected body: %q", w.Body.String())
		}
		usage, ok := got["usage"].(map[string]interface{})
		if !ok || usage["prompt_tokens"].(float64) != 12 {
			t.Fatalf("decoded usage = %#v", got["usage"])
		}
	})

	t.Run("decode failure still writes full body", func(t *testing.T) {
		w := httptest.NewRecorder()
		c, _ := gin.CreateTestContext(w)
		resp := &http.Response{
			StatusCode: http.StatusOK,
			Body:       io.NopCloser(bytes.NewBufferString(`{"usage": invalid-json}`)),
		}

		var got map[string]interface{}
		err := PassthroughJSONResponse(c, resp, &got)
		if err == nil {
			t.Fatal("expected decode error")
		}
		var syntaxErr *json.SyntaxError
		if !errors.As(err, &syntaxErr) {
			t.Fatalf("err = %T, want *json.SyntaxError", err)
		}
		if w.Body.String() != `{"usage": invalid-json}` {
			t.Fatalf("unexpected body: %q", w.Body.String())
		}
	})
}

func TestWithLifecycleTrace_AttachesClientTraceCallbacks(t *testing.T) {
	req, err := http.NewRequest(http.MethodGet, "https://example.com", nil)
	if err != nil {
		t.Fatalf("NewRequest() err = %v", err)
	}

	var connected atomic.Int32
	var firstByte atomic.Int32
	tracedReq := withLifecycleTrace(
		req,
		&RequestLifecycleTrace{
			OnConnected: func() {
				connected.Add(1)
			},
			OnFirstResponseByte: func() {
				firstByte.Add(1)
			},
		},
	)

	trace := httptrace.ContextClientTrace(tracedReq.Context())
	if trace == nil {
		t.Fatal("client trace was not attached")
	}

	trace.GotConn(httptrace.GotConnInfo{})
	trace.GotFirstResponseByte()

	if connected.Load() != 1 {
		t.Fatalf("OnConnected calls = %d, want 1", connected.Load())
	}
	if firstByte.Load() != 1 {
		t.Fatalf("OnFirstResponseByte calls = %d, want 1", firstByte.Load())
	}
}

func TestSanitizeMalformedThinkingBlocks(t *testing.T) {
	input := `{
		"messages": [
			{
				"role": "assistant",
				"content": [
					{"type": "thinking"},
					{"type": "text", "text": "ok"}
				]
			},
			{
				"role": "assistant",
				"content": [
					{"type": "thinking", "thinking": {"foo": "bar"}}
				]
			},
			{
				"role": "assistant",
				"content": [
					{"type": "thinking", "thinking": "keep me"}
				]
			},
			{
				"role": "assistant",
				"content": [
					{"type": "thinking", "thinking": "signed", "signature": "sig_123"}
				]
			},
			{
				"role": "user",
				"content": [
					{"type": "text", "text": "hello"}
				]
			}
		]
	}`

	gotBytes, modified := SanitizeMalformedThinkingBlocks([]byte(input), false, "Messages")
	if !modified {
		t.Fatal("expected modified=true")
	}

	var got map[string]interface{}
	if err := json.Unmarshal(gotBytes, &got); err != nil {
		t.Fatalf("unmarshal result failed: %v", err)
	}

	messages, ok := got["messages"].([]interface{})
	if !ok {
		t.Fatalf("messages type = %T, want []interface{}", got["messages"])
	}

	// 仅畸形 thinking 的 assistant 消息保留骨架（content 清空），不删除整条消息，共 5 条
	if len(messages) != 5 {
		t.Fatalf("messages len = %d, want 5", len(messages))
	}

	firstMsg, _ := messages[0].(map[string]interface{})
	firstContent, _ := firstMsg["content"].([]interface{})
	if len(firstContent) != 1 {
		t.Fatalf("first message content len = %d, want 1", len(firstContent))
	}
	firstBlock, _ := firstContent[0].(map[string]interface{})
	if firstBlock["type"] != "text" {
		t.Fatalf("first message content[0].type = %v, want text", firstBlock["type"])
	}

	// 第二条：仅含 thinking，保留骨架 content=[]
	secondMsg, _ := messages[1].(map[string]interface{})
	secondContent, _ := secondMsg["content"].([]interface{})
	if len(secondContent) != 0 {
		t.Fatalf("second message content len = %d, want 0 (thinking-only, kept as empty)", len(secondContent))
	}

	// 第三条：合法 thinking 保留
	thirdMsg, _ := messages[2].(map[string]interface{})
	thirdContent, _ := thirdMsg["content"].([]interface{})
	if len(thirdContent) != 1 {
		t.Fatalf("third message content len = %d, want 1", len(thirdContent))
	}
	thirdBlock, _ := thirdContent[0].(map[string]interface{})
	if thirdBlock["type"] != "thinking" {
		t.Fatalf("third message content[0].type = %v, want thinking", thirdBlock["type"])
	}
	if thirdBlock["thinking"] != "keep me" {
		t.Fatalf("third message content[0].thinking = %v, want keep me", thirdBlock["thinking"])
	}

	// 第四条：带 signature 的合法 thinking 也保留
	fourthMsg, _ := messages[3].(map[string]interface{})
	fourthContent, _ := fourthMsg["content"].([]interface{})
	if len(fourthContent) != 1 {
		t.Fatalf("fourth message content len = %d, want 1", len(fourthContent))
	}
	fourthBlock, _ := fourthContent[0].(map[string]interface{})
	if fourthBlock["type"] != "thinking" {
		t.Fatalf("fourth message content[0].type = %v, want thinking", fourthBlock["type"])
	}
	if fourthBlock["thinking"] != "signed" {
		t.Fatalf("fourth message content[0].thinking = %v, want signed", fourthBlock["thinking"])
	}
	if fourthBlock["signature"] != "sig_123" {
		t.Fatalf("fourth message content[0].signature = %v, want sig_123", fourthBlock["signature"])
	}

	// 最后一条：user 文本消息
	lastMsg, _ := messages[4].(map[string]interface{})
	if lastMsg["role"] != "user" {
		t.Fatalf("last message role = %v, want user", lastMsg["role"])
	}
}

func TestSanitizeMalformedThinkingBlocks_InvalidJSON_NoChange(t *testing.T) {
	input := []byte(`{"messages":[`)
	got, modified := SanitizeMalformedThinkingBlocks(input, false, "Messages")
	if modified {
		t.Fatal("expected modified=false")
	}
	if string(got) != string(input) {
		t.Fatalf("unexpected output change: got %q, want %q", string(got), string(input))
	}
}

func TestSanitizeMalformedThinkingBlocks_ContentObject(t *testing.T) {
	input := `{
		"messages": [
			{
				"role": "assistant",
				"content": {"type": "thinking", "thinking": {}}
			},
			{
				"role": "assistant",
				"content": {"type": "text", "text": "ok", "thinking": {"thinking": "noise"}}
			}
		]
	}`

	gotBytes, modified := SanitizeMalformedThinkingBlocks([]byte(input), false, "Messages")
	if !modified {
		t.Fatal("expected modified=true")
	}

	var got map[string]interface{}
	if err := json.Unmarshal(gotBytes, &got); err != nil {
		t.Fatalf("unmarshal result failed: %v", err)
	}

	messages, _ := got["messages"].([]interface{})
	if len(messages) != 1 {
		t.Fatalf("messages len = %d, want 1", len(messages))
	}

	msg, _ := messages[0].(map[string]interface{})
	content, _ := msg["content"].(map[string]interface{})
	if _, exists := content["thinking"]; exists {
		t.Fatalf("unexpected thinking field in non-thinking content: %v", content["thinking"])
	}
	if content["type"] != "text" {
		t.Fatalf("content.type = %v, want text", content["type"])
	}
}

func TestRestoreRequestBodyAndContextCacheUseAttemptBody(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	original := []byte(`{"model":"claude-3","metadata":{"user_id":"{\"device_id\":\"abc\"}"}}`)
	attemptBody := NormalizeMetadataUserID(original)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/messages", bytes.NewReader(original))

	RestoreRequestBody(c, attemptBody)
	c.Set("requestBodyBytes", attemptBody)

	body, err := io.ReadAll(c.Request.Body)
	if err != nil {
		t.Fatalf("ReadAll() err = %v", err)
	}
	if string(body) != string(attemptBody) {
		t.Fatalf("request body = %s, want %s", string(body), string(attemptBody))
	}

	cached, ok := c.Get("requestBodyBytes")
	if !ok {
		t.Fatal("requestBodyBytes not found in context")
	}
	if string(cached.([]byte)) != string(attemptBody) {
		t.Fatalf("cached body = %s, want %s", string(cached.([]byte)), string(attemptBody))
	}
}
