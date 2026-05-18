package utils

import (
	"bytes"
	"encoding/json"
	"strings"
)

// MarshalJSONNoEscape 序列化 JSON 并禁用 HTML 字符转义
// 使用 json.Encoder + SetEscapeHTML(false) 避免将 <, >, & 等字符转义为 \u003c 等
// 返回去除末尾换行符的字节数组
func MarshalJSONNoEscape(v interface{}) ([]byte, error) {
	var buf bytes.Buffer
	encoder := json.NewEncoder(&buf)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(v); err != nil {
		return nil, err
	}
	// json.Encoder.Encode 会在末尾添加换行符，需要去掉
	return bytes.TrimSuffix(buf.Bytes(), []byte("\n")), nil
}

// TruncateJSONIntelligently 智能截断JSON中的长文本内容,保持结构完整
// 只截断字符串值,不影响JSON结构
func TruncateJSONIntelligently(data interface{}, maxTextLength int) interface{} {
	if data == nil {
		return nil
	}
	// maxTextLength <= 0 表示不截断
	if maxTextLength <= 0 {
		return data
	}

	switch v := data.(type) {
	case string:
		if len(v) > maxTextLength {
			return v[:maxTextLength] + "..."
		}
		return v

	case float64, int, int64, bool:
		return v

	case []interface{}:
		result := make([]interface{}, len(v))
		for i, item := range v {
			result[i] = TruncateJSONIntelligently(item, maxTextLength)
		}
		return result

	case map[string]interface{}:
		result := make(map[string]interface{}, len(v))
		for key, value := range v {
			result[key] = TruncateJSONIntelligently(value, maxTextLength)
		}
		return result

	default:
		return v
	}
}

// SimplifyToolsArray 简化tools数组为名称列表,减少日志输出
// 将完整的工具定义简化为只显示工具名称
func SimplifyToolsArray(data interface{}) interface{} {
	if data == nil {
		return nil
	}

	switch v := data.(type) {
	case []interface{}:
		result := make([]interface{}, len(v))
		for i, item := range v {
			result[i] = SimplifyToolsArray(item)
		}
		return result

	case map[string]interface{}:
		result := make(map[string]interface{}, len(v))
		for key, value := range v {
			// 如果是tools字段且是数组,提取工具名称
			if key == "tools" {
				if toolsArray, ok := value.([]interface{}); ok {
					result[key] = extractToolNames(toolsArray)
					continue
				}
			}
			// 如果是content字段且是数组,标记为需要紧凑显示
			if key == "content" {
				if contentArray, ok := value.([]interface{}); ok {
					result[key] = compactContentArray(contentArray)
					continue
				}
			}
			// 如果是contents字段（Gemini格式）且是数组,紧凑显示
			if key == "contents" {
				if contentsArray, ok := value.([]interface{}); ok {
					result[key] = compactGeminiContentsArray(contentsArray)
					continue
				}
			}
			result[key] = SimplifyToolsArray(value)
		}
		return result

	default:
		return v
	}
}

// compactContentArray 紧凑显示content数组
// 只保留type和text/id/name等关键字段的简短摘要
func compactContentArray(contents []interface{}) []interface{} {
	result := make([]interface{}, len(contents))
	for i, item := range contents {
		if contentMap, ok := item.(map[string]interface{}); ok {
			compact := make(map[string]interface{})

			// 保留type字段
			if contentType, ok := contentMap["type"].(string); ok {
				compact["type"] = contentType

				// 根据类型保留关键信息
				switch contentType {
				case "text":
					if text, ok := contentMap["text"].(string); ok {
						// 文本内容截断到前200个字符
						if len(text) > 200 {
							compact["text"] = text[:200] + "..."
						} else {
							compact["text"] = text
						}
					}
				case "input_text", "output_text":
					// Responses API 的 input/output 类型
					if text, ok := contentMap["text"].(string); ok {
						if len(text) > 200 {
							compact["text"] = text[:200] + "..."
						} else {
							compact["text"] = text
						}
					}
				case "tool_use":
					if id, ok := contentMap["id"].(string); ok {
						compact["id"] = id
					}
					if name, ok := contentMap["name"].(string); ok {
						compact["name"] = name
					}
					// input字段紧凑显示 - 保留结构但截断长字符串值
					if input, ok := contentMap["input"]; ok {
						compactInput := truncateInputValues(input, 200)
						compact["input"] = compactInput
					}
				case "tool_result":
					if toolUseID, ok := contentMap["tool_use_id"].(string); ok {
						compact["tool_use_id"] = toolUseID
					}
					// content字段显示前200字符
					if content, ok := contentMap["content"].(string); ok {
						if len(content) > 200 {
							compact["content"] = content[:200] + "..."
						} else {
							compact["content"] = content
						}
					}
					if isError, ok := contentMap["is_error"].(bool); ok {
						compact["is_error"] = isError
					}
				case "image":
					if source, ok := contentMap["source"].(map[string]interface{}); ok {
						compact["source"] = map[string]interface{}{
							"type": source["type"],
						}
					}
				case "reasoning":
					// Codex Responses API 的 reasoning 类型
					// 保留 summary，截断 encrypted_content
					if summary, ok := contentMap["summary"]; ok {
						compact["summary"] = summary
					}
					if encryptedContent, ok := contentMap["encrypted_content"].(string); ok {
						if len(encryptedContent) > 100 {
							compact["encrypted_content"] = encryptedContent[:100] + "..."
						} else {
							compact["encrypted_content"] = encryptedContent
						}
					}
					// 保留其他可能的字段（如 content）
					if content, ok := contentMap["content"]; ok {
						compact["content"] = content
					}
				case "function_call":
					// Codex Responses API 的 function_call 类型
					if callID, ok := contentMap["call_id"].(string); ok {
						compact["call_id"] = callID
					}
					if name, ok := contentMap["name"].(string); ok {
						compact["name"] = name
					}
					// arguments 字段截断显示
					if args, ok := contentMap["arguments"].(string); ok {
						if len(args) > 200 {
							compact["arguments"] = args[:200] + "..."
						} else {
							compact["arguments"] = args
						}
					}
				case "function_call_output":
					// Codex Responses API 的 function_call_output 类型
					if callID, ok := contentMap["call_id"].(string); ok {
						compact["call_id"] = callID
					}
					// output 字段截断显示
					if output, ok := contentMap["output"].(string); ok {
						if len(output) > 200 {
							compact["output"] = output[:200] + "..."
						} else {
							compact["output"] = output
						}
					}
				}
			}
			result[i] = compact
		} else {
			result[i] = item
		}
	}
	return result
}

// compactGeminiContentsArray 紧凑显示Gemini contents数组
// Gemini格式: contents[].{role, parts[].{text, functionCall, functionResponse}}
func compactGeminiContentsArray(contents []interface{}) []interface{} {
	result := make([]interface{}, len(contents))
	for i, item := range contents {
		if contentMap, ok := item.(map[string]interface{}); ok {
			compact := make(map[string]interface{})

			// 保留role字段
			if role, ok := contentMap["role"].(string); ok {
				compact["role"] = role
			}

			// 处理parts数组
			if parts, ok := contentMap["parts"].([]interface{}); ok {
				compactParts := make([]interface{}, len(parts))
				for j, part := range parts {
					if partMap, ok := part.(map[string]interface{}); ok {
						compactPart := compactGeminiPart(partMap)
						compactParts[j] = compactPart
					} else {
						compactParts[j] = part
					}
				}
				compact["parts"] = compactParts
			}

			result[i] = compact
		} else {
			result[i] = item
		}
	}
	return result
}

// compactGeminiPart 紧凑显示单个Gemini part
func compactGeminiPart(partMap map[string]interface{}) map[string]interface{} {
	compact := make(map[string]interface{})

	// 处理text字段
	if text, ok := partMap["text"].(string); ok {
		if len(text) > 200 {
			compact["text"] = text[:200] + "..."
		} else {
			compact["text"] = text
		}
	}

	// 处理functionCall字段
	if fc, ok := partMap["functionCall"].(map[string]interface{}); ok {
		compactFC := make(map[string]interface{})
		if name, ok := fc["name"].(string); ok {
			compactFC["name"] = name
		}
		// args字段紧凑显示
		if args, ok := fc["args"]; ok {
			compactFC["args"] = truncateInputValues(args, 200)
		}
		compact["functionCall"] = compactFC
	}

	// 处理functionResponse字段
	if fr, ok := partMap["functionResponse"].(map[string]interface{}); ok {
		compactFR := make(map[string]interface{})
		if name, ok := fr["name"].(string); ok {
			compactFR["name"] = name
		}
		// response字段紧凑显示
		if response, ok := fr["response"]; ok {
			compactFR["response"] = truncateInputValues(response, 200)
		}
		compact["functionResponse"] = compactFR
	}

	// 处理inlineData字段（图片等）
	if inlineData, ok := partMap["inlineData"].(map[string]interface{}); ok {
		compactInline := make(map[string]interface{})
		if mimeType, ok := inlineData["mimeType"].(string); ok {
			compactInline["mimeType"] = mimeType
		}
		// data字段只显示前50个字符
		if data, ok := inlineData["data"].(string); ok {
			if len(data) > 50 {
				compactInline["data"] = data[:50] + "...[base64]"
			} else {
				compactInline["data"] = data
			}
		}
		compact["inlineData"] = compactInline
	}

	// 处理fileData字段
	if fileData, ok := partMap["fileData"].(map[string]interface{}); ok {
		compact["fileData"] = fileData
	}

	// 处理thought字段
	if thought, ok := partMap["thought"].(bool); ok && thought {
		compact["thought"] = thought
	}

	return compact
}

// truncateInputValues 递归截断input对象中的长字符串值
// 保留JSON结构,只截断字符串值到指定长度
func truncateInputValues(data interface{}, maxLength int) interface{} {
	switch v := data.(type) {
	case string:
		if len(v) > maxLength {
			return v[:maxLength] + "..."
		}
		return v

	case map[string]interface{}:
		result := make(map[string]interface{}, len(v))
		for key, value := range v {
			result[key] = truncateInputValues(value, maxLength)
		}
		return result

	case []interface{}:
		result := make([]interface{}, len(v))
		for i, item := range v {
			result[i] = truncateInputValues(item, maxLength)
		}
		return result

	default:
		return v
	}
}

// extractToolNames 从tools数组中提取所有工具名称
// 支持Claude格式、OpenAI格式和Gemini格式
func extractToolNames(toolsArray []interface{}) []interface{} {
	var names []interface{}

	for _, tool := range toolsArray {
		toolMap, ok := tool.(map[string]interface{})
		if !ok {
			// 如果不是 map，可能已经是简化后的名称字符串
			names = append(names, tool)
			continue
		}

		// Gemini格式: tool.functionDeclarations[].name
		if funcDecls, ok := toolMap["functionDeclarations"].([]interface{}); ok {
			for _, funcDecl := range funcDecls {
				if declMap, ok := funcDecl.(map[string]interface{}); ok {
					if name, ok := declMap["name"].(string); ok {
						names = append(names, name)
					}
				}
			}
			continue
		}

		// Claude格式: tool.name
		if name, ok := toolMap["name"].(string); ok {
			names = append(names, name)
			continue
		}

		// OpenAI格式: tool.function.name
		if function, ok := toolMap["function"].(map[string]interface{}); ok {
			if name, ok := function["name"].(string); ok {
				names = append(names, name)
				continue
			}
		}

		// 未知格式，保留原始对象
		names = append(names, tool)
	}

	return names
}

// extractToolName 从工具定义中提取名称（保留用于兼容）
// 支持Claude格式(tool.name)和OpenAI格式(tool.function.name)
func extractToolName(tool interface{}) interface{} {
	toolMap, ok := tool.(map[string]interface{})
	if !ok {
		return tool
	}

	// 检查Claude格式: tool.name
	if name, ok := toolMap["name"].(string); ok {
		return name
	}

	// 检查OpenAI格式: tool.function.name
	if function, ok := toolMap["function"].(map[string]interface{}); ok {
		if name, ok := function["name"].(string); ok {
			return name
		}
	}

	return tool
}

// SimplifyToolsInJSON 简化JSON字节数组中的tools字段
// 这是一个便利函数,直接处理JSON字节
func SimplifyToolsInJSON(jsonData []byte) []byte {
	var data interface{}
	if err := json.Unmarshal(jsonData, &data); err != nil {
		return jsonData // 如果不是有效JSON,返回原始数据
	}

	simplifiedData := SimplifyToolsArray(data)

	simplifiedBytes, err := json.Marshal(simplifiedData)
	if err != nil {
		return jsonData // 如果序列化失败,返回原始数据
	}

	return simplifiedBytes
}

// FormatJSONForLog 格式化JSON用于日志输出
// 先简化tools,再截断长文本,最后美化格式
func FormatJSONForLog(data interface{}, maxTextLength int) string {
	// 先简化tools和content数组
	simplified := SimplifyToolsArray(data)
	// 再截断长文本
	truncated := TruncateJSONIntelligently(simplified, maxTextLength)

	// 使用自定义格式化来实现content数组的紧凑显示
	result := formatJSONWithCompactArrays(truncated, "", 0)

	return result
}

// formatMapAsOneLine 将map格式化为单行JSON
func formatMapAsOneLine(m map[string]interface{}) string {
	if len(m) == 0 {
		return "{}"
	}

	var pairs []string
	// 按照特定顺序输出字段（type优先，然后其他字段）
	if typeVal, ok := m["type"]; ok {
		typeJSON, _ := json.Marshal(typeVal)
		pairs = append(pairs, `"type": `+string(typeJSON))
	}

	// 其他字段按字母顺序
	for k, v := range m {
		if k == "type" {
			continue // 已经处理过
		}
		keyJSON, _ := json.Marshal(k)

		// 对于input字段，使用紧凑的单行显示
		if k == "input" {
			if inputMap, ok := v.(map[string]interface{}); ok {
				valueStr := formatInputMapCompact(inputMap)
				pairs = append(pairs, string(keyJSON)+": "+valueStr)
				continue
			}
		}

		// 对于长字符串字段（如 encrypted_content, arguments, output），进行截断
		if k == "encrypted_content" || k == "arguments" || k == "output" || k == "text" {
			if strVal, ok := v.(string); ok {
				maxLen := 100
				if k == "arguments" || k == "output" || k == "text" {
					maxLen = 200
				}
				if len(strVal) > maxLen {
					truncated := strVal[:maxLen] + "..."
					valueJSON, _ := json.Marshal(truncated)
					pairs = append(pairs, string(keyJSON)+": "+string(valueJSON))
					continue
				}
			}
		}

		valueJSON, _ := json.Marshal(v)
		pairs = append(pairs, string(keyJSON)+": "+string(valueJSON))
	}

	return "{" + strings.Join(pairs, ", ") + "}"
}

// formatInputMapCompact 将input map紧凑格式化为单行
func formatInputMapCompact(m map[string]interface{}) string {
	if len(m) == 0 {
		return "{}"
	}

	var pairs []string
	for k, v := range m {
		keyJSON, _ := json.Marshal(k)
		valueJSON, _ := json.Marshal(v)
		pairs = append(pairs, string(keyJSON)+": "+string(valueJSON))
	}

	return "{" + strings.Join(pairs, ", ") + "}"
}

// formatMessageAsOneLine 将message对象（包含role和content/parts）格式化为紧凑的一行
// 支持Claude格式：{role: "user", content: [...]}
// 支持Gemini格式：{role: "user", parts: [...]}
func formatMessageAsOneLine(m map[string]interface{}) string {
	var parts []string

	// 先输出role
	if role, ok := m["role"]; ok {
		roleJSON, _ := json.Marshal(role)
		parts = append(parts, `"role": `+string(roleJSON))
	}

	// 处理content字段（Claude格式）
	if content, ok := m["content"]; ok {
		// 如果content是字符串，直接输出
		if contentStr, isString := content.(string); isString {
			contentJSON, _ := json.Marshal(contentStr)
			parts = append(parts, `"content": `+string(contentJSON))
		} else if contentArray, isArray := content.([]interface{}); isArray {
			// content数组已经是紧凑格式，直接格式化
			contentItems := make([]string, len(contentArray))
			for i, item := range contentArray {
				if itemMap, ok := item.(map[string]interface{}); ok {
					contentItems[i] = formatMapAsOneLine(itemMap)
				} else {
					itemJSON, _ := json.Marshal(item)
					contentItems[i] = string(itemJSON)
				}
			}
			parts = append(parts, `"content": [`+strings.Join(contentItems, ", ")+`]`)
		}
	}

	// 处理parts字段（Gemini格式）
	if partsField, ok := m["parts"]; ok {
		if partsArray, isArray := partsField.([]interface{}); isArray {
			partsItems := make([]string, len(partsArray))
			for i, item := range partsArray {
				if itemMap, ok := item.(map[string]interface{}); ok {
					partsItems[i] = formatMapAsOneLine(itemMap)
				} else {
					itemJSON, _ := json.Marshal(item)
					partsItems[i] = string(itemJSON)
				}
			}
			parts = append(parts, `"parts": [`+strings.Join(partsItems, ", ")+`]`)
		}
	}

	return "{" + strings.Join(parts, ", ") + "}"
}

// formatJSONWithCompactArrays 自定义JSON格式化,对content数组使用紧凑单行显示
func formatJSONWithCompactArrays(data interface{}, indent string, depth int) string {
	switch v := data.(type) {
	case nil:
		return "null"

	case bool:
		if v {
			return "true"
		}
		return "false"

	case float64:
		bytes, _ := json.Marshal(v)
		return string(bytes)

	case string:
		bytes, _ := json.Marshal(v)
		return string(bytes)

	case []interface{}:
		if len(v) == 0 {
			return "[]"
		}

		// 检查是否是已经紧凑化的content数组
		isCompactContent := false
		isInputArray := false
		isToolsArray := false

		if len(v) > 0 {
			// 检查第一个元素判断数组类型
			if firstItem, ok := v[0].(map[string]interface{}); ok {
				if typeVal, ok := firstItem["type"].(string); ok {
					// 如果第一个元素有type字段,且看起来是content项,使用紧凑格式
					if typeVal == "text" || typeVal == "tool_use" || typeVal == "tool_result" || typeVal == "image" ||
						typeVal == "input_text" || typeVal == "output_text" {
						isCompactContent = true
					}
					// 检查是否是 Codex input 数组中的特殊类型对象
					// 这些对象应该被单独压缩成一行
					if typeVal == "reasoning" || typeVal == "function_call" || typeVal == "function_call_output" {
						isCompactContent = true
					}
				}
				// 检查是否是 input 数组（包含 message 对象，有 role 字段）
				// 或者包含 Codex 特殊类型对象
				if _, hasRole := firstItem["role"]; hasRole {
					isInputArray = true
				} else if typeVal, ok := firstItem["type"].(string); ok {
					// 如果数组包含 reasoning/function_call 等类型，也当作 input 数组处理
					if typeVal == "reasoning" || typeVal == "function_call" || typeVal == "function_call_output" {
						isInputArray = true
					}
				}
			} else if _, ok := v[0].(string); ok {
				// 如果数组元素都是字符串,可能是tools数组（已简化为工具名）
				isToolsArray = true
				// 验证是否所有元素都是字符串
				for _, item := range v {
					if _, ok := item.(string); !ok {
						isToolsArray = false
						break
					}
				}
			}
		}

		if isCompactContent {
			// 紧凑单行显示 - 每个content项压缩为单行
			items := make([]string, len(v))
			for i, item := range v {
				// 将单个content项格式化为单行JSON
				if itemMap, ok := item.(map[string]interface{}); ok {
					compactItem := formatMapAsOneLine(itemMap)
					items[i] = compactItem
				} else {
					items[i] = formatJSONWithCompactArrays(item, "", depth+1)
				}
			}
			return "[\n" + indent + "  " + strings.Join(items, ",\n"+indent+"  ") + "\n" + indent + "]"
		}

		if isInputArray {
			// input 数组（包含 message 对象和特殊类型对象）使用紧凑单行显示
			items := make([]string, len(v))
			for i, item := range v {
				if itemMap, ok := item.(map[string]interface{}); ok {
					// 检查是否是 message 对象（有 role 字段）
					if _, hasRole := itemMap["role"]; hasRole {
						items[i] = formatMessageAsOneLine(itemMap)
					} else if typeVal, hasType := itemMap["type"].(string); hasType {
						// 检查是否是特殊类型对象（reasoning, function_call 等）
						if typeVal == "reasoning" || typeVal == "function_call" || typeVal == "function_call_output" {
							items[i] = formatMapAsOneLine(itemMap)
						} else {
							items[i] = formatJSONWithCompactArrays(item, "", depth+1)
						}
					} else {
						items[i] = formatJSONWithCompactArrays(item, "", depth+1)
					}
				} else {
					items[i] = formatJSONWithCompactArrays(item, "", depth+1)
				}
			}
			return "[\n" + indent + "  " + strings.Join(items, ",\n"+indent+"  ") + "\n" + indent + "]"
		}

		if isToolsArray {
			// tools数组使用紧凑的单行显示
			items := make([]string, len(v))
			for i, item := range v {
				itemJSON, _ := json.Marshal(item)
				items[i] = string(itemJSON)
			}
			// 始终使用单行显示所有工具
			return "[" + strings.Join(items, ", ") + "]"
		}

		// 普通数组的多行显示
		items := make([]string, len(v))
		for i, item := range v {
			items[i] = indent + "  " + formatJSONWithCompactArrays(item, indent+"  ", depth+1)
		}
		return "[\n" + strings.Join(items, ",\n") + "\n" + indent + "]"

	case map[string]interface{}:
		if len(v) == 0 {
			return "{}"
		}

		// 检查是否是message对象（包含role和content字段）
		if _, hasRole := v["role"]; hasRole {
			if _, hasContent := v["content"]; hasContent {
				// 这是一个message对象，使用紧凑的单行显示
				return formatMessageAsOneLine(v)
			}
		}

		// 检查是否是包含 type 字段的特殊对象（reasoning, function_call, function_call_output 等）
		if typeVal, hasType := v["type"].(string); hasType {
			// 这些类型的对象使用紧凑的单行显示
			if typeVal == "reasoning" || typeVal == "function_call" || typeVal == "function_call_output" ||
				typeVal == "text" || typeVal == "tool_use" || typeVal == "tool_result" || typeVal == "image" ||
				typeVal == "input_text" || typeVal == "output_text" {
				return formatMapAsOneLine(v)
			}
		}

		// 对于普通map,使用多行显示
		var keys []string
		for k := range v {
			keys = append(keys, k)
		}

		items := make([]string, len(keys))
		for i, k := range keys {
			value := formatJSONWithCompactArrays(v[k], indent+"  ", depth+1)
			keyJSON, _ := json.Marshal(k)
			items[i] = indent + "  " + string(keyJSON) + ": " + value
		}
		return "{\n" + strings.Join(items, ",\n") + "\n" + indent + "}"

	default:
		bytes, _ := json.Marshal(v)
		return string(bytes)
	}
}

// FormatJSONBytesForLog 格式化JSON字节数组用于日志输出
// maxTextLength <= 0 时不截断，保留完整内容
func FormatJSONBytesForLog(jsonData []byte, maxTextLength int) string {
	var data interface{}
	if err := json.Unmarshal(jsonData, &data); err != nil {
		// 如果不是有效JSON,按字符串处理
		return string(jsonData)
	}

	return FormatJSONForLog(data, maxTextLength)
}

// MaskSensitiveHeaders 脱敏敏感请求头
func MaskSensitiveHeaders(headers map[string]string) map[string]string {
	sensitiveKeys := map[string]bool{
		"authorization":  true,
		"x-api-key":      true,
		"x-goog-api-key": true,
	}

	masked := make(map[string]string, len(headers))
	for key, value := range headers {
		lowerKey := strings.ToLower(key)
		if sensitiveKeys[lowerKey] {
			if lowerKey == "authorization" && strings.HasPrefix(value, "Bearer ") {
				token := value[7:]
				masked[key] = "Bearer " + MaskAPIKey(token)
			} else {
				masked[key] = MaskAPIKey(value)
			}
		} else {
			masked[key] = value
		}
	}
	return masked
}

// MaskAPIKey 掩码API密钥
func MaskAPIKey(key string) string {
	if key == "" {
		return ""
	}

	length := len(key)
	if length <= 10 {
		if length <= 5 {
			return "***"
		}
		return key[:3] + "***" + key[length-2:]
	}

	return key[:8] + "***" + key[length-5:]
}

// FormatJSONBytesRaw 原始输出JSON字节数组（不缩进、不截断、不重排序）
func FormatJSONBytesRaw(jsonData []byte) string {
	return string(jsonData)
}
