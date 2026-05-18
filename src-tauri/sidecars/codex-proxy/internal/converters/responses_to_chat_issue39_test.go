package converters

import (
	"encoding/json"
	"testing"

	"github.com/tidwall/gjson"
)

// TestConvertResponsesToChat_Issue39_ContentTypeText 测试 GitHub Issue #39
// 当 content parts 的 type 为 "text" 时，转换应该正常工作
func TestConvertResponsesToChat_Issue39_ContentTypeText(t *testing.T) {
	tests := []struct {
		name        string
		input       string
		wantNilMsgs bool
	}{
		{
			name: "content parts with type:text (issue format)",
			input: `{
				"model": "gpt",
				"max_output_tokens": 20,
				"input": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
			}`,
			wantNilMsgs: false,
		},
		{
			name: "content parts with type:input_text (official format)",
			input: `{
				"model": "gpt",
				"max_output_tokens": 20,
				"input": [{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}]
			}`,
			wantNilMsgs: false,
		},
		{
			name: "content as string",
			input: `{
				"model": "gpt",
				"max_output_tokens": 20,
				"input": [{"role": "user", "content": "hi"}]
			}`,
			wantNilMsgs: false,
		},
		{
			name: "input as string",
			input: `{
				"model": "gpt",
				"max_output_tokens": 20,
				"input": "hi"
			}`,
			wantNilMsgs: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := ConvertResponsesToOpenAIChatRequest("test-model", []byte(tt.input), false)

			// 解析结果
			parsed := gjson.ParseBytes(result)
			messages := parsed.Get("messages")

			// 检查 messages 是否为 null 或空
			if !messages.Exists() || messages.Type == gjson.Null {
				if !tt.wantNilMsgs {
					t.Errorf("messages is null or missing, got: %s", string(result))
				}
				return
			}

			if !messages.IsArray() {
				t.Errorf("messages is not an array, got: %s", string(result))
				return
			}

			if len(messages.Array()) == 0 {
				if !tt.wantNilMsgs {
					t.Errorf("messages array is empty, got: %s", string(result))
				}
				return
			}

			// 检查第一条消息的 content
			firstMsg := messages.Array()[0]
			content := firstMsg.Get("content")

			if !content.Exists() {
				t.Errorf("first message has no content, got: %s", string(result))
				return
			}

			// content 可以是字符串或数组
			if content.Type == gjson.String {
				if content.String() == "" {
					t.Errorf("first message content is empty string, got: %s", string(result))
				}
			} else if content.IsArray() {
				if len(content.Array()) == 0 {
					t.Errorf("first message content array is empty, got: %s", string(result))
				}
			} else {
				t.Errorf("first message content is neither string nor array, got: %s", string(result))
			}

			// 打印结果以便调试
			var prettyResult map[string]interface{}
			json.Unmarshal(result, &prettyResult)
			prettyJSON, _ := json.MarshalIndent(prettyResult, "", "  ")
			t.Logf("Converted result:\n%s", string(prettyJSON))
		})
	}
}
