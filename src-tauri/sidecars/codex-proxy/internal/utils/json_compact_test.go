package utils

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestCompactContentArray(t *testing.T) {
	input := map[string]interface{}{
		"model": "claude-3",
		"tools": []interface{}{"Tool1", "Tool2", "Tool3"}, // 简化后的tools数组
		"messages": []interface{}{
			map[string]interface{}{
				"role": "user",
				"content": []interface{}{
					map[string]interface{}{
						"type": "text",
						"text": strings.Repeat("This is a very long text that should be truncated. ", 10),
					},
					map[string]interface{}{
						"type": "tool_use",
						"id":   "toolu_123",
						"name": "get_weather",
						"input": map[string]interface{}{
							"location": "San Francisco",
							"unit":     "celsius",
						},
					},
				},
			},
			map[string]interface{}{
				"role": "assistant",
				"content": []interface{}{
					map[string]interface{}{
						"type":        "tool_result",
						"tool_use_id": "toolu_123",
						"content":     "Temperature: 18°C, Clear sky",
						"is_error":    false,
					},
				},
			},
		},
	}

	result := FormatJSONForLog(input, 500)

	// 验证content数组被紧凑显示
	if !strings.Contains(result, `"type": "text"`) {
		t.Error("应该包含type字段")
	}

	// 验证文本被截断到200字符
	if strings.Contains(result, strings.Repeat("This is a very long text", 8)) {
		t.Error("长文本应该被截断到200字符")
	}

	// 验证tool_use的input显示JSON而不是{...}
	if !strings.Contains(result, `"location"`) || !strings.Contains(result, `"San Francisco"`) {
		t.Error("tool_use的input应该显示JSON内容")
	}

	// 验证tools数组被紧凑显示（单行或少量换行）
	if strings.Contains(result, `"tools": ["Tool1", "Tool2", "Tool3"]`) ||
		strings.Contains(result, `"tools": [
  "Tool1", "Tool2", "Tool3"
]`) {
		t.Log("[Test-OK] tools数组被紧凑显示")
	}

	// 验证输出没有被截断（不应该出现"需要�"这种乱码）
	if strings.Contains(result, "�") {
		t.Error("输出包含乱码，可能是截断导致的")
	}

	t.Logf("格式化后的输出:\n%s", result)
}

func TestContentArrayCompactFormat(t *testing.T) {
	// 测试各种content类型的紧凑显示
	tests := []struct {
		name    string
		content []interface{}
		checks  []string // 应该包含的内容
	}{
		{
			name: "文本类型 - 长文本截断",
			content: []interface{}{
				map[string]interface{}{
					"type": "text",
					"text": strings.Repeat("This is a very long text that exceeds 200 characters and should be truncated. ", 5),
				},
			},
			checks: []string{
				`"type": "text"`,
				// 文本应该被截断到200字符，包含省略号
				`...`,
			},
		},
		{
			name: "工具使用类型",
			content: []interface{}{
				map[string]interface{}{
					"type": "tool_use",
					"id":   "toolu_abc123",
					"name": "calculator",
					"input": map[string]interface{}{
						"expression": "2 + 2",
					},
				},
			},
			checks: []string{
				`"type": "tool_use"`,
				`"id": "toolu_abc123"`,
				`"name": "calculator"`,
				// input应该显示JSON内容而不是{...}
				`"expression"`,
			},
		},
		{
			name: "工具结果类型",
			content: []interface{}{
				map[string]interface{}{
					"type":        "tool_result",
					"tool_use_id": "toolu_abc123",
					"content":     "Result: 4",
					"is_error":    false,
				},
			},
			checks: []string{
				`"type": "tool_result"`,
				`"tool_use_id": "toolu_abc123"`,
				`"content": "Result: 4"`,
				`"is_error": false`,
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := map[string]interface{}{
				"messages": []interface{}{
					map[string]interface{}{
						"role":    "user",
						"content": tt.content,
					},
				},
			}

			result := FormatJSONForLog(input, 500)

			for _, check := range tt.checks {
				if !strings.Contains(result, check) {
					t.Errorf("输出应该包含: %s\n实际输出:\n%s", check, result)
				}
			}

			// 验证没有乱码
			if strings.Contains(result, "�") {
				t.Error("输出包含乱码")
			}
		})
	}
}

func TestNoTruncationInMiddleOfJSON(t *testing.T) {
	// 创建一个超大的JSON对象来测试截断逻辑
	largeMessages := make([]interface{}, 100)
	for i := 0; i < 100; i++ {
		largeMessages[i] = map[string]interface{}{
			"role": "user",
			"content": []interface{}{
				map[string]interface{}{
					"type": "text",
					"text": "Message " + strings.Repeat("x", 100),
				},
			},
		}
	}

	input := map[string]interface{}{
		"model":    "claude-3",
		"messages": largeMessages,
	}

	result := FormatJSONForLog(input, 500)

	// 如果被截断，应该在换行符处截断
	if strings.Contains(result, "... (输出已截断)") {
		// 检查截断位置是否在合适的地方
		truncateIndex := strings.Index(result, "... (输出已截断)")
		beforeTruncate := result[:truncateIndex]

		// 应该在换行符后截断
		if !strings.HasSuffix(strings.TrimSpace(beforeTruncate), "\n") &&
			!strings.HasSuffix(beforeTruncate, "}") &&
			!strings.HasSuffix(beforeTruncate, "]") {
			// 允许截断点不完美，但至少不应该在字符串中间
			if !strings.Contains(beforeTruncate[len(beforeTruncate)-20:], "\n") {
				t.Error("截断位置不在合适的边界")
			}
		}

		t.Logf("[Test-OK] 超长输出被正确截断，截断位置: %d", truncateIndex)
	}
}

func TestFormatJSONBytesForLog(t *testing.T) {
	input := map[string]interface{}{
		"messages": []interface{}{
			map[string]interface{}{
				"role": "user",
				"content": []interface{}{
					map[string]interface{}{
						"type": "text",
						"text": "Hello, world!",
					},
				},
			},
		},
	}

	jsonBytes, _ := json.Marshal(input)
	result := FormatJSONBytesForLog(jsonBytes, 500)

	// 验证基本功能
	if !strings.Contains(result, `"type": "text"`) {
		t.Error("应该包含type字段")
	}

	if !strings.Contains(result, `"text": "Hello, world!"`) {
		t.Error("应该包含完整的短文本")
	}

	// 验证没有乱码
	if strings.Contains(result, "�") {
		t.Error("输出包含乱码")
	}

	t.Logf("格式化结果:\n%s", result)
}

// TestCodexResponsesFormat 测试 Codex Responses API 格式的压缩显示
func TestCodexResponsesFormat(t *testing.T) {
	// 模拟 Codex Responses API 的请求体
	input := map[string]interface{}{
		"input": []interface{}{
			map[string]interface{}{
				"role": "assistant",
				"content": []interface{}{
					map[string]interface{}{
						"type": "output_text",
						"text": "- This repo is a desktop 文档智能评分系统 built with Wails v3: Go backend + Vue 3 frontend; it parallel-scores DOCX/PDF/PPTX/TXT docs via multiple AI models (Kimi, MiniMax, DeepSeek), su...",
					},
				},
			},
			map[string]interface{}{
				"role": "user",
				"content": []interface{}{
					map[string]interface{}{
						"type": "input_text",
						"text": "[截屏2025-11-18 12.09.22.png 3022x2022] 失败的时候正在评分这个消息一直消不掉",
					},
					map[string]interface{}{
						"type": "input_image",
					},
				},
			},
			map[string]interface{}{
				"type":              "reasoning",
				"content":           nil,
				"encrypted_content": "gAAAAABpG_GLKdJFoKhQfJKcN5k9efb8cQRy3md40ZemIZlJMlmuGgxhTjUtFPwmTAToAwIDtPsPMoOxV8SwDDLohrOqLqUMNEBgFV3ZBNgbNamdzu_jRW7JiFFpB8supDB4lIWyIhvh6HwuHP-8it62DBcdKp9U_V1GuSsP96C8GacKBEEyUmmcHbAcgXj341PxsVpiLx3y5xS18kXTXafmVK_EATeun9vLZ-A9m2BbbEfXoC4zb1AfUGQ_46sZyYXZNWr-v3gbbRkPug4Hq8j4d8vHMmDqNHGDuuScL5r63obEnrrhdTl9dbpOeSgIm7ag-fzmdofyP4I4XKx_SUxaEbTbWbHxunTYpA4lZy04Qw0b85TTvY62G6hcik5i-l5b6LgU0LTycR9lp_LE8OnAswvjLT3HQz6tFzZM288H1vWykftDb-eCyOX4pXn7WP4HFFNp_GvoVy1RPGJh_QbVxKAZCYiv0_7AaSjpv1_RS8EYbssy...",
				"summary": []interface{}{
					map[string]interface{}{
						"type": "summary_text",
						"text": "**Investigating status toast bug**",
					},
				},
			},
			map[string]interface{}{
				"call_id":   "call_FIgewLLjtlkutO7mK5scikpN",
				"name":      "shell",
				"type":      "function_call",
				"arguments": `{"command":["bash","-lc","ls frontend/src"],"workdir":"/Users/petaflops/projects/doc-scorer-wails"}`,
			},
		},
	}

	result := FormatJSONForLog(input, 500)

	// 验证 input 数组被压缩成单行
	lines := strings.Split(result, "\n")
	var inputArrayLines []string
	inInputArray := false
	for _, line := range lines {
		if strings.Contains(line, `"input":`) {
			inInputArray = true
		}
		if inInputArray {
			inputArrayLines = append(inputArrayLines, line)
			if strings.Contains(line, "]") && !strings.Contains(line, "[") {
				break
			}
		}
	}

	// 验证每个 message 对象都在单行
	inputArrayStr := strings.Join(inputArrayLines, "\n")
	if !strings.Contains(inputArrayStr, `{"role": "assistant"`) {
		t.Error("input 数组中的 message 对象应该被压缩成单行")
	}

	// 验证 reasoning 类型被压缩
	if !strings.Contains(result, `"type": "reasoning"`) {
		t.Error("应该包含 reasoning 类型")
	}

	// 验证 function_call 类型被压缩
	if !strings.Contains(result, `"type": "function_call"`) {
		t.Error("应该包含 function_call 类型")
	}

	// 验证 encrypted_content 被截断
	if strings.Contains(result, "gAAAAABpG_GLKdJFoKhQfJKcN5k9efb8cQRy3md40ZemIZlJMlmuGgxhTjUtFPwmTAToAwIDtPsPMoOxV8SwDDLohrOqLqUMNEBgFV3ZBNgbNamdzu_jRW7JiFFpB8supDB4lIWyIhvh6HwuHP-8it62DBcdKp9U_V1GuSsP96C8GacKBEEyUmmcHbAcgXj341PxsVpiLx3y5xS18kXTXafmVK_EATeun9vLZ-A9m2BbbEfXoC4zb1AfUGQ_46sZyYXZNWr-v3gbbRkPug4Hq8j4d8vHMmDqNHGDuuScL5r63obEnrrhdTl9dbpOeSgIm7ag-fzmdofyP4I4XKx_SUxaEbTbWbHxunTYpA4lZy04Qw0b85TTvY62G6hcik5i-l5b6LgU0LTycR9lp_LE8OnAswvjLT3HQz6tFzZM288H1vWykftDb-eCyOX4pXn7WP4HFFNp_GvoVy1RPGJh_QbVxKAZCYiv0_7AaSjpv1_RS8EYbssy...") {
		t.Error("encrypted_content 应该被截断")
	}

	t.Logf("Codex Responses 格式化结果:\n%s", result)
}

// TestGeminiContentsFormat 测试 Gemini contents 数组的紧凑格式化
func TestGeminiContentsFormat(t *testing.T) {
	// 模拟 Gemini 请求格式
	input := map[string]interface{}{
		"contents": []interface{}{
			map[string]interface{}{
				"role": "user",
				"parts": []interface{}{
					map[string]interface{}{
						"text": "Hello, this is a test message that might be quite long and should be truncated if it exceeds the limit. " + strings.Repeat("More text here. ", 20),
					},
				},
			},
			map[string]interface{}{
				"role": "model",
				"parts": []interface{}{
					map[string]interface{}{
						"text": "This is the model response.",
					},
					map[string]interface{}{
						"functionCall": map[string]interface{}{
							"name": "get_weather",
							"args": map[string]interface{}{
								"location": "San Francisco",
								"unit":     "celsius",
							},
						},
					},
				},
			},
			map[string]interface{}{
				"role": "user",
				"parts": []interface{}{
					map[string]interface{}{
						"functionResponse": map[string]interface{}{
							"name": "get_weather",
							"response": map[string]interface{}{
								"temperature": 18,
								"condition":   "Clear sky",
							},
						},
					},
				},
			},
		},
		"generationConfig": map[string]interface{}{
			"maxOutputTokens": 1024,
		},
	}

	result := FormatJSONForLog(input, 500)

	// 验证 contents 数组被正确处理
	if !strings.Contains(result, `"contents"`) {
		t.Error("应该包含 contents 字段")
	}

	// 验证 parts 字段被保留
	if !strings.Contains(result, `"parts"`) {
		t.Error("应该包含 parts 字段")
	}

	// 验证 role 字段被保留
	if !strings.Contains(result, `"role": "user"`) {
		t.Error("应该包含 role 字段")
	}

	// 验证 functionCall 被保留
	if !strings.Contains(result, `"functionCall"`) {
		t.Error("应该包含 functionCall 字段")
	}

	// 验证 functionResponse 被保留
	if !strings.Contains(result, `"functionResponse"`) {
		t.Error("应该包含 functionResponse 字段")
	}

	// 验证长文本被截断
	if strings.Contains(result, "More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here. More text here.") {
		t.Error("长文本应该被截断")
	}

	t.Logf("Gemini contents 格式化结果:\n%s", result)
}

// TestGeminiToolsFormat 测试 Gemini tools 数组的简化显示
func TestGeminiToolsFormat(t *testing.T) {
	// 模拟 Gemini 请求中的 tools 格式
	input := map[string]interface{}{
		"contents": []interface{}{
			map[string]interface{}{
				"role": "user",
				"parts": []interface{}{
					map[string]interface{}{
						"text": "List the files in the current directory",
					},
				},
			},
		},
		"tools": []interface{}{
			map[string]interface{}{
				"functionDeclarations": []interface{}{
					map[string]interface{}{
						"name":        "list_directory",
						"description": "Lists the names of files and subdirectories directly within a specified directory path.",
						"parametersJsonSchema": map[string]interface{}{
							"type": "object",
							"properties": map[string]interface{}{
								"dir_path": map[string]interface{}{
									"type":        "string",
									"description": "The path to the directory to list",
								},
							},
							"required": []interface{}{"dir_path"},
						},
					},
					map[string]interface{}{
						"name":        "read_file",
						"description": "Reads and returns the content of a specified file.",
						"parametersJsonSchema": map[string]interface{}{
							"type": "object",
							"properties": map[string]interface{}{
								"file_path": map[string]interface{}{
									"type":        "string",
									"description": "The path to the file to read.",
								},
							},
							"required": []interface{}{"file_path"},
						},
					},
					map[string]interface{}{
						"name":        "write_file",
						"description": "Writes content to a specified file.",
						"parametersJsonSchema": map[string]interface{}{
							"type": "object",
							"properties": map[string]interface{}{
								"file_path": map[string]interface{}{
									"type":        "string",
									"description": "The path to the file to write.",
								},
								"content": map[string]interface{}{
									"type":        "string",
									"description": "The content to write.",
								},
							},
							"required": []interface{}{"file_path", "content"},
						},
					},
				},
			},
		},
		"generationConfig": map[string]interface{}{
			"maxOutputTokens": 1024,
		},
	}

	result := FormatJSONForLog(input, 500)

	// 验证 tools 数组被简化为工具名称列表
	if !strings.Contains(result, `"tools": ["list_directory", "read_file", "write_file"]`) {
		t.Errorf("Gemini tools 应该被简化为工具名称列表，实际输出:\n%s", result)
	}

	// 验证完整的 parametersJsonSchema 没有出现在输出中
	if strings.Contains(result, "parametersJsonSchema") {
		t.Error("tools 中不应该包含 parametersJsonSchema")
	}

	// 验证完整的 description 没有出现在输出中
	if strings.Contains(result, "Lists the names of files") {
		t.Error("tools 中不应该包含完整的 description")
	}

	t.Logf("Gemini tools 格式化结果:\n%s", result)
}
