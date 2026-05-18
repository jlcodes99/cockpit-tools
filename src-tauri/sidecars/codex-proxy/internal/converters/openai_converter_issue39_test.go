package converters

import (
	"encoding/json"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// TestOpenAIChatConverter_Issue39_ContentTypeText 测试 GitHub Issue #39
// 当 content parts 的 type 为 "text" 时，通过 OpenAIChatConverter 转换应该正常工作
// 这是实际生产代码路径：ResponsesProvider.ConvertToProviderRequest -> OpenAIChatConverter.ToProviderRequest
func TestOpenAIChatConverter_Issue39_ContentTypeText(t *testing.T) {
	tests := []struct {
		name       string
		rawRequest string
	}{
		{
			name: "content parts with type:text (issue #39 format)",
			rawRequest: `{
				"model": "MiniMax-M2.7",
				"max_output_tokens": 20,
				"input": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
			}`,
		},
		{
			name: "content parts with type:input_text (official format)",
			rawRequest: `{
				"model": "MiniMax-M2.7",
				"max_output_tokens": 20,
				"input": [{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}]
			}`,
		},
		{
			name: "content as string",
			rawRequest: `{
				"model": "MiniMax-M2.7",
				"max_output_tokens": 20,
				"input": [{"role": "user", "content": "hi"}]
			}`,
		},
		{
			name: "input as string",
			rawRequest: `{
				"model": "MiniMax-M2.7",
				"max_output_tokens": 20,
				"input": "hi"
			}`,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var req types.ResponsesRequest
			if err := json.Unmarshal([]byte(tt.rawRequest), &req); err != nil {
				t.Fatalf("failed to unmarshal request: %v", err)
			}

			converter := &OpenAIChatConverter{}
			sess := &session.Session{}
			result, err := converter.ToProviderRequest(sess, &req)
			if err != nil {
				t.Fatalf("ToProviderRequest failed: %v", err)
			}

			// 序列化看看结果
			resultJSON, _ := json.MarshalIndent(result, "", "  ")
			t.Logf("Converted result:\n%s", string(resultJSON))

			// 验证 messages 字段
			resultMap, ok := result.(map[string]interface{})
			if !ok {
				t.Fatalf("result is not a map")
			}

			messagesRaw, ok := resultMap["messages"]
			if !ok {
				t.Fatalf("messages field is missing")
			}

			messages, ok := messagesRaw.([]map[string]interface{})
			if !ok {
				t.Fatalf("messages is not []map[string]interface{}, got %T", messagesRaw)
			}

			if len(messages) == 0 {
				t.Fatalf("messages array is empty")
			}

			// 验证第一条消息的 content 不为 nil
			firstMsg := messages[0]
			content, exists := firstMsg["content"]
			if !exists {
				t.Fatalf("first message has no content field")
			}

			if content == nil {
				t.Errorf("first message content is nil (this is the Issue #39 bug!)")
			}

			// content 可以是 string 或 []parts
			switch c := content.(type) {
			case string:
				if c == "" {
					t.Errorf("first message content is empty string")
				}
			case []map[string]interface{}:
				if len(c) == 0 {
					t.Errorf("first message content parts array is empty")
				}
			default:
				t.Errorf("first message content is unexpected type: %T", content)
			}
		})
	}
}
