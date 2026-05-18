package converters

import (
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// ConvertResponsesToOpenAIChatRequest 将 OpenAI Responses 请求格式转换为 OpenAI Chat Completions 格式
// 转换内容包括:
// 1. model 和 stream 配置
// 2. instructions → system message
// 3. input 数组 → messages 数组
// 4. tools 定义转换
// 5. function_call 和 function_call_output 处理
// 6. 生成参数映射 (max_tokens, reasoning 等)
//
// 参数:
//   - modelName: 要使用的模型名称
//   - inputRawJSON: Responses 格式的原始 JSON 请求
//   - stream: 是否为流式请求
//
// 返回:
//   - []byte: Chat Completions 格式的请求 JSON
func ConvertResponsesToOpenAIChatRequest(modelName string, inputRawJSON []byte, stream bool) []byte {
	// 基础 Chat Completions 模板
	out := `{"model":"","messages":[],"stream":false}`

	root := gjson.ParseBytes(inputRawJSON)

	// 设置 model
	out, _ = sjson.Set(out, "model", modelName)

	// 设置 stream
	out, _ = sjson.Set(out, "stream", stream)

	// 如果是流式请求，添加 stream_options 以获取 usage 信息
	if stream {
		out, _ = sjson.Set(out, "stream_options.include_usage", true)
	}

	// 映射生成参数
	if maxTokens := root.Get("max_output_tokens"); maxTokens.Exists() {
		out, _ = sjson.Set(out, "max_tokens", maxTokens.Int())
	}

	if parallelToolCalls := root.Get("parallel_tool_calls"); parallelToolCalls.Exists() {
		out, _ = sjson.Set(out, "parallel_tool_calls", parallelToolCalls.Bool())
	}

	if temperature := root.Get("temperature"); temperature.Exists() {
		out, _ = sjson.Set(out, "temperature", temperature.Float())
	}

	if topP := root.Get("top_p"); topP.Exists() {
		out, _ = sjson.Set(out, "top_p", topP.Float())
	}

	if user := root.Get("user"); user.Exists() {
		out, _ = sjson.Set(out, "user", user.String())
	}

	// 转换 instructions → system message
	if instructions := root.Get("instructions"); instructions.Exists() && instructions.String() != "" {
		systemMessage := `{"role":"system","content":""}`
		systemMessage, _ = sjson.Set(systemMessage, "content", instructions.String())
		out, _ = sjson.SetRaw(out, "messages.-1", systemMessage)
	}

	// 转换 input 数组 → messages
	if input := root.Get("input"); input.Exists() {
		if input.IsArray() {
			out = convertInputArrayToMessages(input, out)
		} else if input.Type == gjson.String {
			// 简单字符串输入
			msg := `{"role":"user","content":""}`
			msg, _ = sjson.Set(msg, "content", input.String())
			out, _ = sjson.SetRaw(out, "messages.-1", msg)
		}
	}

	// 转换 tools
	if tools := root.Get("tools"); tools.Exists() && tools.IsArray() {
		out = convertToolsToOpenAIFormat(tools, out)
	}

	// 转换 reasoning.effort → reasoning_effort
	if reasoningEffort := root.Get("reasoning.effort"); reasoningEffort.Exists() {
		effort := reasoningEffort.String()
		switch effort {
		case "none":
			out, _ = sjson.Set(out, "reasoning_effort", "none")
		case "auto":
			out, _ = sjson.Set(out, "reasoning_effort", "auto")
		case "minimal":
			out, _ = sjson.Set(out, "reasoning_effort", "low")
		case "low":
			out, _ = sjson.Set(out, "reasoning_effort", "low")
		case "medium":
			out, _ = sjson.Set(out, "reasoning_effort", "medium")
		case "high":
			out, _ = sjson.Set(out, "reasoning_effort", "high")
		case "xhigh":
			out, _ = sjson.Set(out, "reasoning_effort", "xhigh")
		default:
			out, _ = sjson.Set(out, "reasoning_effort", "auto")
		}
	}

	// 转换 tool_choice
	if toolChoice := root.Get("tool_choice"); toolChoice.Exists() {
		out, _ = sjson.Set(out, "tool_choice", toolChoice.Value())
	}

	return []byte(out)
}

// convertInputArrayToMessages 将 input 数组转换为 messages 数组
func convertInputArrayToMessages(input gjson.Result, out string) string {
	input.ForEach(func(_, item gjson.Result) bool {
		itemType := item.Get("type").String()

		// 如果没有 type 但有 role，则视为 message
		if itemType == "" && item.Get("role").String() != "" {
			itemType = "message"
		}

		switch itemType {
		case "message":
			out = convertMessageItem(item, out)

		case "function_call":
			out = convertFunctionCallItem(item, out)

		case "function_call_output":
			out = convertFunctionCallOutputItem(item, out)
		}

		return true
	})

	return out
}

// convertMessageItem 转换 message 类型的 item
func convertMessageItem(item gjson.Result, out string) string {
	role := item.Get("role").String()
	if role == "" {
		role = "user"
	}

	message := `{"role":"","content":""}`
	message, _ = sjson.Set(message, "role", role)

	content := item.Get("content")
	if content.Exists() {
		if content.IsArray() {
			// 文本-only 保持 string，包含图片时保留 OpenAI Chat 多模态数组
			var messageContent string
			chatContent := `[]`
			hasMedia := false
			var toolCalls []interface{}

			content.ForEach(func(_, contentItem gjson.Result) bool {
				contentType := contentItem.Get("type").String()
				if contentType == "" {
					contentType = "input_text"
				}

				switch contentType {
				case "input_text", "output_text", "text":
					text := contentItem.Get("text").String()
					if text != "" {
						textBlock := `{"type":"text","text":""}`
						textBlock, _ = sjson.Set(textBlock, "text", text)
						chatContent, _ = sjson.SetRaw(chatContent, "-1", textBlock)
					}
					if messageContent != "" {
						messageContent += "\n" + text
					} else {
						messageContent = text
					}
				case "input_image", "image_url":
					if imageBlock := responsesImageContentToChatBlock(contentItem); imageBlock != "" {
						chatContent, _ = sjson.SetRaw(chatContent, "-1", imageBlock)
						hasMedia = true
					}
				}
				return true
			})

			if hasMedia {
				message, _ = sjson.SetRaw(message, "content", chatContent)
			} else if messageContent != "" {
				message, _ = sjson.Set(message, "content", messageContent)
			}

			if len(toolCalls) > 0 {
				message, _ = sjson.Set(message, "tool_calls", toolCalls)
			}
		} else if content.Type == gjson.String {
			// content 是字符串
			message, _ = sjson.Set(message, "content", content.String())
		}
	}

	out, _ = sjson.SetRaw(out, "messages.-1", message)
	return out
}

func responsesImageContentToChatBlock(contentItem gjson.Result) string {
	imageURL := contentItem.Get("image_url")
	if !imageURL.Exists() {
		return ""
	}

	block := `{"type":"image_url","image_url":{}}`
	if imageURL.Type == gjson.String {
		if imageURL.String() == "" {
			return ""
		}
		block, _ = sjson.Set(block, "image_url.url", imageURL.String())
	} else if imageURL.IsObject() {
		block, _ = sjson.SetRaw(block, "image_url", imageURL.Raw)
	} else {
		return ""
	}

	if detail := contentItem.Get("detail"); detail.Exists() && detail.String() != "" {
		block, _ = sjson.Set(block, "image_url.detail", detail.String())
	}

	return block
}

// convertFunctionCallItem 转换 function_call 类型的 item
func convertFunctionCallItem(item gjson.Result, out string) string {
	// function_call → assistant message with tool_calls
	assistantMessage := `{"role":"assistant","tool_calls":[]}`

	toolCall := `{"id":"","type":"function","function":{"name":"","arguments":""}}`

	if callID := item.Get("call_id"); callID.Exists() {
		toolCall, _ = sjson.Set(toolCall, "id", callID.String())
	}

	if name := item.Get("name"); name.Exists() {
		toolCall, _ = sjson.Set(toolCall, "function.name", name.String())
	}

	if arguments := item.Get("arguments"); arguments.Exists() {
		toolCall, _ = sjson.Set(toolCall, "function.arguments", arguments.String())
	}

	assistantMessage, _ = sjson.SetRaw(assistantMessage, "tool_calls.0", toolCall)
	out, _ = sjson.SetRaw(out, "messages.-1", assistantMessage)

	return out
}

// convertFunctionCallOutputItem 转换 function_call_output 类型的 item
func convertFunctionCallOutputItem(item gjson.Result, out string) string {
	// function_call_output → tool message
	toolMessage := `{"role":"tool","tool_call_id":"","content":""}`

	if callID := item.Get("call_id"); callID.Exists() {
		toolMessage, _ = sjson.Set(toolMessage, "tool_call_id", callID.String())
	}

	if output := item.Get("output"); output.Exists() {
		toolMessage, _ = sjson.Set(toolMessage, "content", output.String())
	}

	out, _ = sjson.SetRaw(out, "messages.-1", toolMessage)
	return out
}

// convertToolsToOpenAIFormat 将 Responses tools 转换为 OpenAI Chat Completions tools 格式
//
// 兼容性说明：
//  1. Codex 新版 Responses 的 tool schema 可能省略 `required` 字段，部分严格
//     校验 JSONSchema 的上游镜像会报 "None is not of type 'array'"。
//     转换时统一补齐 `required: []`，并确保 `type: object` 与 `properties: {}`。
//  2. Responses 的 custom / web_search / namespace 等非 function 工具
//     在 Chat Completions 中没有对应概念，直接跳过，避免触发协议错误。
func convertToolsToOpenAIFormat(tools gjson.Result, out string) string {
	var chatCompletionsTools []interface{}

	tools.ForEach(func(_, tool gjson.Result) bool {
		toolType := tool.Get("type").String()
		// Chat Completions 只支持 function tool，其他类型直接跳过
		if toolType != "" && toolType != "function" {
			return true
		}

		chatTool := `{"type":"function","function":{}}`

		function := `{"name":"","description":"","parameters":{}}`

		// 支持两种写法：{name, parameters} 与 {function: {name, parameters}}
		name := tool.Get("name")
		if !name.Exists() {
			name = tool.Get("function.name")
		}
		description := tool.Get("description")
		if !description.Exists() {
			description = tool.Get("function.description")
		}
		parameters := tool.Get("parameters")
		if !parameters.Exists() {
			parameters = tool.Get("function.parameters")
		}

		if !name.Exists() || name.String() == "" {
			return true
		}
		function, _ = sjson.Set(function, "name", name.String())

		if description.Exists() && description.String() != "" {
			function, _ = sjson.Set(function, "description", description.String())
		}

		function, _ = sjson.SetRaw(function, "parameters", normalizeChatToolParameters(parameters))

		chatTool, _ = sjson.SetRaw(chatTool, "function", function)
		chatCompletionsTools = append(chatCompletionsTools, gjson.Parse(chatTool).Value())

		return true
	})

	if len(chatCompletionsTools) > 0 {
		out, _ = sjson.Set(out, "tools", chatCompletionsTools)
	}

	return out
}

// normalizeChatToolParameters 规范化 tool.parameters，补齐严格校验所需字段。
//
// 针对的上游异常：部分镜像在未提供 `required` 时直接按 None 走校验，
// 抛出 "Invalid schema for function ...: None is not of type 'array'"。
// 通过显式填充 required=[] / type=object / properties={} 规避。
func normalizeChatToolParameters(parameters gjson.Result) string {
	raw := "{}"
	if parameters.Exists() && parameters.IsObject() {
		raw = parameters.Raw
	}

	if !gjson.Get(raw, "type").Exists() {
		raw, _ = sjson.Set(raw, "type", "object")
	}
	if !gjson.Get(raw, "properties").Exists() {
		raw, _ = sjson.SetRaw(raw, "properties", "{}")
	}
	if !gjson.Get(raw, "required").Exists() {
		raw, _ = sjson.SetRaw(raw, "required", "[]")
	}

	return raw
}
