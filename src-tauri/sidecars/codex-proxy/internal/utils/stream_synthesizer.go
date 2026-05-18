package utils

import (
	"encoding/json"
	"regexp"
	"sort"
	"strconv"
	"strings"
)

// StreamSynthesizer 流式响应内容合成器
type StreamSynthesizer struct {
	serviceType         string
	synthesizedContent  strings.Builder
	toolCallAccumulator map[int]*ToolCall
	parseFailed         bool

	// responses专用累积器
	responsesText map[int]*strings.Builder
}

// ToolCall 工具调用累积器
type ToolCall struct {
	ID        string
	Name      string
	Arguments string
}

// NewStreamSynthesizer 创建新的流合成器
func NewStreamSynthesizer(serviceType string) *StreamSynthesizer {
	return &StreamSynthesizer{
		serviceType:         serviceType,
		toolCallAccumulator: make(map[int]*ToolCall),
		responsesText:       make(map[int]*strings.Builder),
	}
}

// ProcessLine 处理SSE流的一行
func (s *StreamSynthesizer) ProcessLine(line string) {
	trimmedLine := strings.TrimSpace(line)
	if trimmedLine == "" {
		return
	}

	// 使用正则匹配SSE data字段
	dataRegex := regexp.MustCompile(`^data:\s*(.*)$`)
	matches := dataRegex.FindStringSubmatch(trimmedLine)
	if len(matches) < 2 {
		return
	}

	jsonStr := strings.TrimSpace(matches[1])
	if jsonStr == "[DONE]" || jsonStr == "" {
		return
	}

	// 解析JSON - 不再因失败而停止处理
	var data map[string]interface{}
	if err := json.Unmarshal([]byte(jsonStr), &data); err != nil {
		// 记录解析失败但继续处理后续行，而不是完全停止
		if !s.parseFailed {
			s.parseFailed = true
			s.synthesizedContent.WriteString("\n[解析警告: 部分JSON解析失败，将显示原始文本内容]")
		}
		return
	}

	// 如果之前解析失败，但现在成功了，重置失败标记
	if s.parseFailed {
		s.parseFailed = false
	}

	// 根据服务类型解析
	switch s.serviceType {
	case "gemini":
		s.processGemini(data)
	case "openai":
		s.processOpenAI(data)
	case "claude":
		s.processClaude(data)
	case "responses":
		s.processResponses(data)
	}
}

// processResponses 处理OpenAI Responses流
func (s *StreamSynthesizer) processResponses(data map[string]interface{}) {
	typeStr, _ := data["type"].(string)

	// 辅助方法：获取对应 output_index 的 builder
	getBuilder := func(index int) *strings.Builder {
		if s.responsesText[index] == nil {
			s.responsesText[index] = &strings.Builder{}
		}
		return s.responsesText[index]
	}

	// 获取 output_index
	getIndex := func() int {
		if idx, ok := data["output_index"].(float64); ok {
			return int(idx)
		}
		return 0
	}

	switch typeStr {
	case "response.output_text.delta":
		if delta, ok := data["delta"].(string); ok {
			builder := getBuilder(getIndex())
			builder.WriteString(delta)
		}
	case "response.output_text.done":
		builder := getBuilder(getIndex())
		if text, ok := data["text"].(string); ok && text != "" {
			builder.Reset()
			builder.WriteString(text)
		}
	case "response.completed":
		// 兜底：从最终响应提取文本
		if respObj, ok := data["response"].(map[string]interface{}); ok {
			if outputArr, ok := respObj["output"].([]interface{}); ok {
				for i, item := range outputArr {
					itemMap, ok := item.(map[string]interface{})
					if !ok {
						continue
					}
					if itemMap["type"] != "message" {
						continue
					}
					contentArr, ok := itemMap["content"].([]interface{})
					if !ok {
						continue
					}
					for _, c := range contentArr {
						cm, ok := c.(map[string]interface{})
						if !ok {
							continue
						}
						if cm["type"] != "output_text" {
							continue
						}
						if text, ok := cm["text"].(string); ok && text != "" {
							builder := getBuilder(i)
							builder.Reset()
							builder.WriteString(text)
							break
						}
					}
				}
			}
		}
	case "response.output_item.added":
		// 记录函数调用元数据（用于后续拼接日志）
		if item, ok := data["item"].(map[string]interface{}); ok {
			if itemType, _ := item["type"].(string); itemType == "function_call" {
				index := getIndex()
				if s.toolCallAccumulator[index] == nil {
					s.toolCallAccumulator[index] = &ToolCall{}
				}
				acc := s.toolCallAccumulator[index]
				if id, ok := item["id"].(string); ok && id != "" {
					acc.ID = id
				}
				if name, ok := item["name"].(string); ok && name != "" {
					acc.Name = name
				}
			}
		}
	case "response.function_call_arguments.delta":
		index := getIndex()
		if s.toolCallAccumulator[index] == nil {
			s.toolCallAccumulator[index] = &ToolCall{}
		}
		acc := s.toolCallAccumulator[index]
		if id, ok := data["item_id"].(string); ok && id != "" {
			acc.ID = id
		}
		if delta, ok := data["delta"].(string); ok {
			acc.Arguments += delta
		}
	case "response.function_call_arguments.done":
		index := getIndex()
		if s.toolCallAccumulator[index] == nil {
			s.toolCallAccumulator[index] = &ToolCall{}
		}
		acc := s.toolCallAccumulator[index]
		if id, ok := data["item_id"].(string); ok && id != "" {
			acc.ID = id
		}
		if args, ok := data["arguments"].(string); ok && args != "" {
			acc.Arguments = args
		}
		if item, ok := data["item"].(map[string]interface{}); ok {
			if name, ok := item["name"].(string); ok && name != "" {
				acc.Name = name
			}
		}
	}
}

// processGemini 处理Gemini格式
func (s *StreamSynthesizer) processGemini(data map[string]interface{}) {
	candidates, ok := data["candidates"].([]interface{})
	if !ok || len(candidates) == 0 {
		return
	}

	candidate, ok := candidates[0].(map[string]interface{})
	if !ok {
		return
	}

	content, ok := candidate["content"].(map[string]interface{})
	if !ok {
		return
	}

	parts, ok := content["parts"].([]interface{})
	if !ok {
		return
	}

	for _, part := range parts {
		partMap, ok := part.(map[string]interface{})
		if !ok {
			continue
		}

		// 文本内容
		if text, ok := partMap["text"].(string); ok {
			s.synthesizedContent.WriteString(text)
		}

		// 函数调用
		if functionCall, ok := partMap["functionCall"].(map[string]interface{}); ok {
			name, _ := functionCall["name"].(string)
			args, _ := functionCall["args"]
			argsJSON, _ := json.Marshal(args)
			s.synthesizedContent.WriteString("\nTool Call: ")
			s.synthesizedContent.WriteString(name)
			s.synthesizedContent.WriteString("(")
			s.synthesizedContent.Write(argsJSON)
			s.synthesizedContent.WriteString(")")
		}
	}
}

// processOpenAI 处理OpenAI格式
func (s *StreamSynthesizer) processOpenAI(data map[string]interface{}) {
	choices, ok := data["choices"].([]interface{})
	if !ok || len(choices) == 0 {
		return
	}

	choice, ok := choices[0].(map[string]interface{})
	if !ok {
		return
	}

	delta, ok := choice["delta"].(map[string]interface{})
	if !ok {
		return
	}

	// 文本内容
	if content, ok := delta["content"].(string); ok {
		s.synthesizedContent.WriteString(content)
	}

	// 工具调用
	if toolCalls, ok := delta["tool_calls"].([]interface{}); ok {
		for _, tc := range toolCalls {
			toolCallMap, ok := tc.(map[string]interface{})
			if !ok {
				continue
			}

			index := 0
			if idx, ok := toolCallMap["index"].(float64); ok {
				index = int(idx)
			}

			if s.toolCallAccumulator[index] == nil {
				s.toolCallAccumulator[index] = &ToolCall{}
			}

			accumulated := s.toolCallAccumulator[index]

			if id, ok := toolCallMap["id"].(string); ok {
				accumulated.ID = id
			}

			if function, ok := toolCallMap["function"].(map[string]interface{}); ok {
				if name, ok := function["name"].(string); ok {
					accumulated.Name = name
				}
				if args, ok := function["arguments"].(string); ok {
					accumulated.Arguments += args
				}
			}
		}
	}
}

// processClaude 处理Claude格式
func (s *StreamSynthesizer) processClaude(data map[string]interface{}) {
	eventType, _ := data["type"].(string)

	switch eventType {
	case "message_start":
		// 从 message_start 中提取初始内容（如果有）
		if msg, ok := data["message"].(map[string]interface{}); ok {
			if content, ok := msg["content"].([]interface{}); ok {
				for _, c := range content {
					if cm, ok := c.(map[string]interface{}); ok {
						if text, ok := cm["text"].(string); ok {
							s.synthesizedContent.WriteString(text)
						}
					}
				}
			}
		}

	case "content_block_start":
		contentBlock, ok := data["content_block"].(map[string]interface{})
		if !ok {
			return
		}

		blockIndex := 0
		if idx, ok := data["index"].(float64); ok {
			blockIndex = int(idx)
		}

		blockType, _ := contentBlock["type"].(string)

		switch blockType {
		case "tool_use":
			if s.toolCallAccumulator[blockIndex] == nil {
				s.toolCallAccumulator[blockIndex] = &ToolCall{}
			}
			accumulated := s.toolCallAccumulator[blockIndex]
			if id, ok := contentBlock["id"].(string); ok {
				accumulated.ID = id
			}
			if name, ok := contentBlock["name"].(string); ok {
				accumulated.Name = name
			}
		case "text":
			// text 类型的 content_block_start 可能包含初始文本
			if text, ok := contentBlock["text"].(string); ok && text != "" {
				s.synthesizedContent.WriteString(text)
			}
		}

	case "content_block_delta":
		delta, ok := data["delta"].(map[string]interface{})
		if !ok {
			return
		}

		deltaType, _ := delta["type"].(string)

		switch deltaType {
		case "text_delta":
			if text, ok := delta["text"].(string); ok {
				s.synthesizedContent.WriteString(text)
			}
		case "input_json_delta":
			if partialJSON, ok := delta["partial_json"].(string); ok {
				blockIndex := 0
				if idx, ok := data["index"].(float64); ok {
					blockIndex = int(idx)
				}

				if s.toolCallAccumulator[blockIndex] == nil {
					s.toolCallAccumulator[blockIndex] = &ToolCall{}
				}

				accumulated := s.toolCallAccumulator[blockIndex]
				accumulated.Arguments += partialJSON
			}
		case "thinking_delta":
			// thinking 内容不记录到合成内容中（可选：如需记录可取消注释）
			// if thinking, ok := delta["thinking"].(string); ok {
			// 	s.synthesizedContent.WriteString(thinking)
			// }
		}

	case "message_delta":
		// message_delta 通常包含 stop_reason 和 usage，不包含文本内容
		// 但某些情况下可能有额外数据，这里做兜底处理
		if delta, ok := data["delta"].(map[string]interface{}); ok {
			if text, ok := delta["text"].(string); ok {
				s.synthesizedContent.WriteString(text)
			}
		}
	}
}

// GetSynthesizedContent 获取合成的内容
func (s *StreamSynthesizer) GetSynthesizedContent() string {
	// 不再完全失败，即使有解析错误也返回部分结果
	var result string

	if s.serviceType == "responses" && len(s.responsesText) > 0 {
		var builder strings.Builder
		keys := make([]int, 0, len(s.responsesText))
		for k := range s.responsesText {
			keys = append(keys, k)
		}
		sort.Ints(keys)

		for i, k := range keys {
			text := s.responsesText[k].String()
			if text == "" {
				continue
			}
			if i > 0 && builder.Len() > 0 {
				builder.WriteString("\n")
			}
			builder.WriteString(text)
		}
		result = builder.String()
	} else {
		result = s.synthesizedContent.String()
	}

	// 添加工具调用信息
	if len(s.toolCallAccumulator) > 0 {
		// 修复分裂的工具调用：检测并合并元数据和参数分离的情况
		s.mergeSplitToolCalls()

		// 按 index 排序输出，避免 map 遍历顺序不稳定
		indices := make([]int, 0, len(s.toolCallAccumulator))
		for idx := range s.toolCallAccumulator {
			indices = append(indices, idx)
		}
		sort.Ints(indices)

		var toolCallsBuilder strings.Builder
		for _, index := range indices {
			tool := s.toolCallAccumulator[index]
			args := tool.Arguments
			if args == "" {
				args = "{}"
			}

			name := tool.Name
			if name == "" {
				name = "unknown_function"
			}

			id := tool.ID
			if id == "" {
				id = "tool_" + strconv.Itoa(index)
			}

			toolCallsBuilder.WriteString("\nTool Call: ")
			toolCallsBuilder.WriteString(name)
			toolCallsBuilder.WriteString("(")

			// 尝试格式化JSON
			var parsedArgs interface{}
			if err := json.Unmarshal([]byte(args), &parsedArgs); err == nil {
				prettyArgs, _ := json.Marshal(parsedArgs)
				toolCallsBuilder.Write(prettyArgs)
			} else {
				toolCallsBuilder.WriteString(args)
			}

			toolCallsBuilder.WriteString(") [ID: ")
			toolCallsBuilder.WriteString(id)
			toolCallsBuilder.WriteString("]")
		}

		result += toolCallsBuilder.String()
	}

	return result
}

// mergeSplitToolCalls 修复分裂的工具调用
// 问题场景：上游返回的工具调用被意外分成两个 content_block：
// - 第一个 block 有 name 和 id，但参数为空 "{}"
// - 第二个 block 没有 name（显示为 unknown_function），但有完整参数
// 此方法检测并合并这种情况
func (s *StreamSynthesizer) mergeSplitToolCalls() {
	if len(s.toolCallAccumulator) < 2 {
		return
	}

	// 收集所有索引并排序
	indices := make([]int, 0, len(s.toolCallAccumulator))
	for idx := range s.toolCallAccumulator {
		indices = append(indices, idx)
	}
	sort.Ints(indices)

	// 检测分裂模式：有 name 但参数为空/"{}" 的 block，后面紧跟无 name 但有参数的 block
	toDelete := make(map[int]bool)

	for i := 0; i < len(indices)-1; i++ {
		currIdx := indices[i]
		nextIdx := indices[i+1]

		// 约束：只合并连续的 index（防止误合并不相关的调用）
		if nextIdx != currIdx+1 {
			continue
		}

		curr := s.toolCallAccumulator[currIdx]
		next := s.toolCallAccumulator[nextIdx]

		// 检测分裂条件：
		// 1. 当前 block 有 name 和 id，但参数为空或只有 "{}"
		// 2. 下一个 block 没有 name，但有实际参数
		// 3. 如果 next 有 ID，必须与 curr 相同（或 curr 无 ID）
		currArgsEmpty := curr.Arguments == "" || curr.Arguments == "{}"
		nextHasNoName := next.Name == ""
		nextHasArgs := next.Arguments != "" && next.Arguments != "{}"
		idMatch := next.ID == "" || curr.ID == "" || next.ID == curr.ID

		if curr.Name != "" && currArgsEmpty && nextHasNoName && nextHasArgs && idMatch {
			// 合并：将 next 的参数移到 curr，补全缺失字段
			curr.Arguments = next.Arguments
			if curr.ID == "" && next.ID != "" {
				curr.ID = next.ID
			}
			toDelete[nextIdx] = true
			// 跳过下一个，因为已经处理了
			i++
		}
	}

	// 删除已合并的 block
	for idx := range toDelete {
		delete(s.toolCallAccumulator, idx)
	}
}

// IsParseFailed 检查解析是否失败
func (s *StreamSynthesizer) IsParseFailed() bool {
	return s.parseFailed
}

// HasToolCalls 检查是否有工具调用被处理
func (s *StreamSynthesizer) HasToolCalls() bool {
	return len(s.toolCallAccumulator) > 0
}
