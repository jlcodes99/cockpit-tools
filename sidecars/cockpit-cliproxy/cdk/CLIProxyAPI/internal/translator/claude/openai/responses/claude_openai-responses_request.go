package responses

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"strings"

	"github.com/google/uuid"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

var (
	user    = ""
	account = ""
	session = ""
)

// ConvertOpenAIResponsesRequestToClaude transforms an OpenAI Responses API request
// into a Claude Messages API request using only gjson/sjson for JSON handling.
// It supports:
// - instructions -> system message
// - input[].type==message with input_text/output_text -> user/assistant messages
// - function_call -> assistant tool_use
// - function_call_output -> user tool_result
// - tools[].parameters -> tools[].input_schema
// - max_output_tokens -> max_tokens
// - stream passthrough via parameter
func ConvertOpenAIResponsesRequestToClaude(modelName string, inputRawJSON []byte, stream bool) []byte {
	rawJSON := inputRawJSON

	if account == "" {
		u, _ := uuid.NewRandom()
		account = u.String()
	}
	if session == "" {
		u, _ := uuid.NewRandom()
		session = u.String()
	}
	if user == "" {
		sum := sha256.Sum256([]byte(account + session))
		user = hex.EncodeToString(sum[:])
	}
	userID := fmt.Sprintf("user_%s_account_%s_session_%s", user, account, session)

	// Base Claude message payload
	out := []byte(fmt.Sprintf(`{"model":"","max_tokens":32000,"messages":[],"metadata":{"user_id":"%s"}}`, userID))

	root := gjson.ParseBytes(rawJSON)

	// Convert OpenAI Responses reasoning.effort to Claude thinking config.
	if v := root.Get("reasoning.effort"); v.Exists() {
		effort := strings.ToLower(strings.TrimSpace(v.String()))
		if effort != "" {
			mi := registry.LookupModelInfo(modelName, "claude")
			supportsAdaptive := mi != nil && mi.Thinking != nil && len(mi.Thinking.Levels) > 0
			supportsMax := supportsAdaptive && thinking.HasLevel(mi.Thinking.Levels, string(thinking.LevelMax))

			// Claude 4.6 supports adaptive thinking with output_config.effort.
			// MapToClaudeEffort normalizes levels (e.g. minimal→low, xhigh→high) to avoid
			// validation errors since validate treats same-provider unsupported levels as errors.
			if supportsAdaptive {
				switch effort {
				case "none":
					out, _ = sjson.SetBytes(out, "thinking.type", "disabled")
					out, _ = sjson.DeleteBytes(out, "thinking.budget_tokens")
					out, _ = sjson.DeleteBytes(out, "output_config.effort")
				case "auto":
					out, _ = sjson.SetBytes(out, "thinking.type", "adaptive")
					out, _ = sjson.DeleteBytes(out, "thinking.budget_tokens")
					out, _ = sjson.DeleteBytes(out, "output_config.effort")
				default:
					if mapped, ok := thinking.MapToClaudeEffort(effort, supportsMax); ok {
						effort = mapped
					}
					out, _ = sjson.SetBytes(out, "thinking.type", "adaptive")
					out, _ = sjson.DeleteBytes(out, "thinking.budget_tokens")
					out, _ = sjson.SetBytes(out, "output_config.effort", effort)
				}
			} else {
				// Legacy/manual thinking (budget_tokens).
				budget, ok := thinking.ConvertLevelToBudget(effort)
				if ok {
					switch budget {
					case 0:
						out, _ = sjson.SetBytes(out, "thinking.type", "disabled")
					case -1:
						out, _ = sjson.SetBytes(out, "thinking.type", "enabled")
					default:
						if budget > 0 {
							out, _ = sjson.SetBytes(out, "thinking.type", "enabled")
							out, _ = sjson.SetBytes(out, "thinking.budget_tokens", budget)
						}
					}
				}
			}
		}
	}

	// Helper for generating tool call IDs when missing
	genToolCallID := func() string {
		const letters = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
		var b strings.Builder
		for i := 0; i < 24; i++ {
			n, _ := rand.Int(rand.Reader, big.NewInt(int64(len(letters))))
			b.WriteByte(letters[n.Int64()])
		}
		return "toolu_" + b.String()
	}

	// Model
	out, _ = sjson.SetBytes(out, "model", modelName)

	// Max tokens
	if mot := root.Get("max_output_tokens"); mot.Exists() {
		out, _ = sjson.SetBytes(out, "max_tokens", mot.Int())
	}

	// Stream
	out, _ = sjson.SetBytes(out, "stream", stream)

	// instructions -> as a leading message (use role user for Claude API compatibility)
	instructionsText := ""
	extractedFromSystem := false
	if instr := root.Get("instructions"); instr.Exists() && instr.Type == gjson.String {
		instructionsText = instr.String()
		if instructionsText != "" {
			sysMsg := []byte(`{"role":"user","content":""}`)
			sysMsg, _ = sjson.SetBytes(sysMsg, "content", instructionsText)
			out, _ = sjson.SetRawBytes(out, "messages.-1", sysMsg)
		}
	}

	if instructionsText == "" {
		if input := root.Get("input"); input.Exists() && input.IsArray() {
			input.ForEach(func(_, item gjson.Result) bool {
				if strings.EqualFold(item.Get("role").String(), "system") {
					var builder strings.Builder
					if parts := item.Get("content"); parts.Exists() && parts.IsArray() {
						parts.ForEach(func(_, part gjson.Result) bool {
							textResult := part.Get("text")
							text := textResult.String()
							if builder.Len() > 0 && text != "" {
								builder.WriteByte('\n')
							}
							builder.WriteString(text)
							return true
						})
					} else if parts.Type == gjson.String {
						builder.WriteString(parts.String())
					}
					instructionsText = builder.String()
					if instructionsText != "" {
						sysMsg := []byte(`{"role":"user","content":""}`)
						sysMsg, _ = sjson.SetBytes(sysMsg, "content", instructionsText)
						out, _ = sjson.SetRawBytes(out, "messages.-1", sysMsg)
						extractedFromSystem = true
					}
				}
				return instructionsText == ""
			})
		}
	}

	// input array processing
	if input := root.Get("input"); input.Exists() && input.IsArray() {
		input.ForEach(func(_, item gjson.Result) bool {
			if extractedFromSystem && strings.EqualFold(item.Get("role").String(), "system") {
				return true
			}
			typ := item.Get("type").String()
			if typ == "" && item.Get("role").String() != "" {
				typ = "message"
			}
			switch typ {
			case "message":
				// Determine role and construct Claude-compatible content parts.
				var role string
				var textAggregate strings.Builder
				var partsJSON []string
				hasImage := false
				hasFile := false
				if parts := item.Get("content"); parts.Exists() && parts.IsArray() {
					parts.ForEach(func(_, part gjson.Result) bool {
						ptype := part.Get("type").String()
						switch ptype {
						case "input_text", "output_text":
							if t := part.Get("text"); t.Exists() {
								txt := t.String()
								textAggregate.WriteString(txt)
								contentPart := []byte(`{"type":"text","text":""}`)
								contentPart, _ = sjson.SetBytes(contentPart, "text", txt)
								partsJSON = append(partsJSON, string(contentPart))
							}
							if ptype == "input_text" {
								role = "user"
							} else {
								role = "assistant"
							}
						case "input_image":
							url := part.Get("image_url").String()
							if url == "" {
								url = part.Get("url").String()
							}
							if url != "" {
								var contentPart []byte
								if strings.HasPrefix(url, "data:") {
									trimmed := strings.TrimPrefix(url, "data:")
									mediaAndData := strings.SplitN(trimmed, ";base64,", 2)
									mediaType := "application/octet-stream"
									data := ""
									if len(mediaAndData) == 2 {
										if mediaAndData[0] != "" {
											mediaType = mediaAndData[0]
										}
										data = mediaAndData[1]
									}
									if data != "" {
										contentPart = []byte(`{"type":"image","source":{"type":"base64","media_type":"","data":""}}`)
										contentPart, _ = sjson.SetBytes(contentPart, "source.media_type", mediaType)
										contentPart, _ = sjson.SetBytes(contentPart, "source.data", data)
									}
								} else {
									contentPart = []byte(`{"type":"image","source":{"type":"url","url":""}}`)
									contentPart, _ = sjson.SetBytes(contentPart, "source.url", url)
								}
								if len(contentPart) > 0 {
									partsJSON = append(partsJSON, string(contentPart))
									if role == "" {
										role = "user"
									}
									hasImage = true
								}
							}
						case "input_file":
							fileData := part.Get("file_data").String()
							if fileData != "" {
								mediaType := "application/octet-stream"
								data := fileData
								if strings.HasPrefix(fileData, "data:") {
									trimmed := strings.TrimPrefix(fileData, "data:")
									mediaAndData := strings.SplitN(trimmed, ";base64,", 2)
									if len(mediaAndData) == 2 {
										if mediaAndData[0] != "" {
											mediaType = mediaAndData[0]
										}
										data = mediaAndData[1]
									}
								}
								contentPart := []byte(`{"type":"document","source":{"type":"base64","media_type":"","data":""}}`)
								contentPart, _ = sjson.SetBytes(contentPart, "source.media_type", mediaType)
								contentPart, _ = sjson.SetBytes(contentPart, "source.data", data)
								partsJSON = append(partsJSON, string(contentPart))
								if role == "" {
									role = "user"
								}
								hasFile = true
							}
						}
						return true
					})
				} else if parts.Type == gjson.String {
					textAggregate.WriteString(parts.String())
				}

				// Fallback to given role if content types not decisive
				if role == "" {
					r := item.Get("role").String()
					switch r {
					case "user", "assistant", "system":
						role = r
					default:
						role = "user"
					}
				}

				if len(partsJSON) > 0 {
					msg := []byte(`{"role":"","content":[]}`)
					msg, _ = sjson.SetBytes(msg, "role", role)
					if len(partsJSON) == 1 && !hasImage && !hasFile {
						// Preserve legacy behavior for single text content
						msg, _ = sjson.DeleteBytes(msg, "content")
						textPart := gjson.Parse(partsJSON[0])
						msg, _ = sjson.SetBytes(msg, "content", textPart.Get("text").String())
					} else {
						for _, partJSON := range partsJSON {
							msg, _ = sjson.SetRawBytes(msg, "content.-1", []byte(partJSON))
						}
					}
					out, _ = sjson.SetRawBytes(out, "messages.-1", msg)
				} else if textAggregate.Len() > 0 || role == "system" {
					msg := []byte(`{"role":"","content":""}`)
					msg, _ = sjson.SetBytes(msg, "role", role)
					msg, _ = sjson.SetBytes(msg, "content", textAggregate.String())
					out, _ = sjson.SetRawBytes(out, "messages.-1", msg)
				}

			case "function_call", "custom_tool_call":
				// Map to assistant tool_use
				callID := item.Get("call_id").String()
				if callID == "" {
					callID = genToolCallID()
				}
				name := item.Get("name").String()
				if namespace := strings.TrimSpace(item.Get("namespace").String()); namespace != "" {
					name = qualifyResponsesNamespaceToolName(namespace, name)
				}
				argsStr := item.Get("arguments").String()
				if typ == "custom_tool_call" {
					input := item.Get("input")
					if input.IsObject() {
						argsStr = input.Raw
					} else {
						wrapped := []byte(`{"input":""}`)
						wrapped, _ = sjson.SetBytes(wrapped, "input", input.String())
						argsStr = string(wrapped)
					}
				}

				toolUse := []byte(`{"type":"tool_use","id":"","name":"","input":{}}`)
				toolUse, _ = sjson.SetBytes(toolUse, "id", callID)
				toolUse, _ = sjson.SetBytes(toolUse, "name", name)
				if argsStr != "" && gjson.Valid(argsStr) {
					argsJSON := gjson.Parse(argsStr)
					if argsJSON.IsObject() {
						toolUse, _ = sjson.SetRawBytes(toolUse, "input", []byte(argsJSON.Raw))
					}
				}

				asst := []byte(`{"role":"assistant","content":[]}`)
				asst, _ = sjson.SetRawBytes(asst, "content.-1", toolUse)
				out, _ = sjson.SetRawBytes(out, "messages.-1", asst)

			case "function_call_output", "custom_tool_call_output":
				// Map to user tool_result
				callID := item.Get("call_id").String()
				toolResult := []byte(`{"type":"tool_result","tool_use_id":"","content":""}`)
				toolResult, _ = sjson.SetBytes(toolResult, "tool_use_id", callID)
				toolResult, _ = sjson.SetRawBytes(toolResult, "content", responsesToolOutputToClaudeContent(item.Get("output")))

				usr := []byte(`{"role":"user","content":[]}`)
				usr, _ = sjson.SetRawBytes(usr, "content.-1", toolResult)
				out, _ = sjson.SetRawBytes(out, "messages.-1", usr)
			}
			return true
		})
	}

	out = normalizeClaudeToolPairing(out)

	includedToolNames := map[string]struct{}{}
	toolNameMap := map[string]string{}

	// tools mapping: parameters -> input_schema
	toolsJSON := []byte("[]")
	appendTools := func(tools gjson.Result) {
		if !tools.Exists() || !tools.IsArray() {
			return
		}
		tools.ForEach(func(_, tool gjson.Result) bool {
			convertedTools := convertResponsesToolToClaudeTools(tool, toolNameMap)
			for _, tJSON := range convertedTools {
				toolName := gjson.GetBytes(tJSON, "name").String()
				if toolName != "" {
					if _, exists := includedToolNames[toolName]; exists {
						continue
					}
					includedToolNames[toolName] = struct{}{}
				}
				toolsJSON, _ = sjson.SetRawBytes(toolsJSON, "-1", tJSON)
			}
			return true
		})
	}
	appendTools(root.Get("tools"))
	if input := root.Get("input"); input.Exists() && input.IsArray() {
		input.ForEach(func(_, item gjson.Result) bool {
			if item.Get("type").String() == "additional_tools" {
				appendTools(item.Get("tools"))
			}
			return true
		})
	}
	if parsedTools := gjson.ParseBytes(toolsJSON); parsedTools.IsArray() && len(parsedTools.Array()) > 0 {
		out, _ = sjson.SetRawBytes(out, "tools", toolsJSON)
	}

	// Map tool_choice similar to Chat Completions translator (optional in docs, safe to handle)
	if toolChoice := root.Get("tool_choice"); toolChoice.Exists() {
		switch toolChoice.Type {
		case gjson.String:
			switch toolChoice.String() {
			case "auto":
				out, _ = sjson.SetRawBytes(out, "tool_choice", []byte(`{"type":"auto"}`))
			case "none":
				// Leave unset; implies no tools
			case "required":
				if len(includedToolNames) > 0 {
					out, _ = sjson.SetRawBytes(out, "tool_choice", []byte(`{"type":"any"}`))
				}
			}
		case gjson.JSON:
			if toolChoice.Get("type").String() == "function" {
				fn := toolChoice.Get("function.name").String()
				if fn == "" {
					fn = toolChoice.Get("name").String()
				}
				if mappedName := toolNameMap[fn]; mappedName != "" {
					fn = mappedName
				}
				if _, ok := includedToolNames[fn]; ok {
					toolChoiceJSON := []byte(`{"name":"","type":"tool"}`)
					toolChoiceJSON, _ = sjson.SetBytes(toolChoiceJSON, "name", fn)
					out, _ = sjson.SetRawBytes(out, "tool_choice", toolChoiceJSON)
				}
			}
		default:

		}
	}

	return out
}

func responsesToolOutputToClaudeContent(output gjson.Result) []byte {
	if !output.Exists() || output.Type == gjson.Null {
		return []byte(`"(empty)"`)
	}
	if output.Type == gjson.String {
		encoded, _ := json.Marshal(output.String())
		return encoded
	}
	if !output.IsArray() {
		encoded, _ := json.Marshal(output.Raw)
		return encoded
	}

	content := []byte("[]")
	count := 0
	output.ForEach(func(_, part gjson.Result) bool {
		if part.Type == gjson.String {
			block := []byte(`{"type":"text","text":""}`)
			block, _ = sjson.SetBytes(block, "text", part.String())
			content, _ = sjson.SetRawBytes(content, "-1", block)
			count++
			return true
		}
		switch part.Get("type").String() {
		case "input_text", "output_text", "text":
			if text := part.Get("text"); text.Exists() {
				block := []byte(`{"type":"text","text":""}`)
				block, _ = sjson.SetBytes(block, "text", text.String())
				content, _ = sjson.SetRawBytes(content, "-1", block)
				count++
			}
		case "input_image", "image_url":
			if block := responsesImagePartToClaude(part); len(block) > 0 {
				content, _ = sjson.SetRawBytes(content, "-1", block)
				count++
			}
		}
		return true
	})
	if count == 0 {
		if len(output.Array()) == 0 {
			return []byte(`"(empty)"`)
		}
		encoded, _ := json.Marshal(output.Raw)
		return encoded
	}
	return content
}

func responsesImagePartToClaude(part gjson.Result) []byte {
	imageURL := part.Get("image_url")
	url := ""
	if imageURL.Type == gjson.String {
		url = strings.TrimSpace(imageURL.String())
	} else if imageURL.IsObject() {
		url = strings.TrimSpace(imageURL.Get("url").String())
	}
	if url == "" {
		url = strings.TrimSpace(part.Get("url").String())
	}
	if url == "" {
		return nil
	}
	if strings.HasPrefix(url, "data:") {
		mediaAndData := strings.SplitN(strings.TrimPrefix(url, "data:"), ";base64,", 2)
		if len(mediaAndData) != 2 || mediaAndData[1] == "" {
			return nil
		}
		mediaType := mediaAndData[0]
		if mediaType == "" {
			mediaType = "application/octet-stream"
		}
		block := []byte(`{"type":"image","source":{"type":"base64","media_type":"","data":""}}`)
		block, _ = sjson.SetBytes(block, "source.media_type", mediaType)
		block, _ = sjson.SetBytes(block, "source.data", mediaAndData[1])
		return block
	}
	block := []byte(`{"type":"image","source":{"type":"url","url":""}}`)
	block, _ = sjson.SetBytes(block, "source.url", url)
	return block
}

type claudeRequestMessage struct {
	role   string
	blocks [][]byte
}

func normalizeClaudeToolPairing(request []byte) []byte {
	messages := gjson.GetBytes(request, "messages")
	if !messages.IsArray() {
		return request
	}
	parsed := make([]claudeRequestMessage, 0, len(messages.Array()))
	hasToolBlocks := false
	messages.ForEach(func(_, message gjson.Result) bool {
		blocks := claudeRequestContentBlocks(message.Get("content"))
		if len(blocks) > 0 {
			for _, block := range blocks {
				switch gjson.GetBytes(block, "type").String() {
				case "tool_use", "tool_result":
					hasToolBlocks = true
				}
			}
			parsed = append(parsed, claudeRequestMessage{role: message.Get("role").String(), blocks: blocks})
		}
		return true
	})
	if !hasToolBlocks {
		return request
	}
	parsed = mergeClaudeRequestMessages(parsed)

	results := make(map[string][]byte)
	for _, message := range parsed {
		if message.role != "user" {
			continue
		}
		for _, block := range message.blocks {
			if gjson.GetBytes(block, "type").String() == "tool_result" {
				if id := gjson.GetBytes(block, "tool_use_id").String(); id != "" {
					results[id] = block
				}
			}
		}
	}

	normalized := make([]claudeRequestMessage, 0, len(parsed))
	for _, message := range parsed {
		switch message.role {
		case "assistant":
			other := make([][]byte, 0, len(message.blocks))
			answered := make([][]byte, 0, len(message.blocks))
			for _, block := range message.blocks {
				if gjson.GetBytes(block, "type").String() != "tool_use" {
					other = append(other, block)
					continue
				}
				if _, ok := results[gjson.GetBytes(block, "id").String()]; ok {
					answered = append(answered, block)
				}
			}
			if len(other)+len(answered) > 0 {
				normalized = append(normalized, claudeRequestMessage{role: "assistant", blocks: append(other, answered...)})
			}
			if len(answered) > 0 {
				resultBlocks := make([][]byte, 0, len(answered))
				for _, block := range answered {
					resultBlocks = append(resultBlocks, results[gjson.GetBytes(block, "id").String()])
				}
				normalized = append(normalized, claudeRequestMessage{role: "user", blocks: resultBlocks})
			}
		case "user":
			nonResults := make([][]byte, 0, len(message.blocks))
			for _, block := range message.blocks {
				if gjson.GetBytes(block, "type").String() != "tool_result" {
					nonResults = append(nonResults, block)
				}
			}
			if len(nonResults) > 0 {
				normalized = append(normalized, claudeRequestMessage{role: "user", blocks: nonResults})
			}
		default:
			normalized = append(normalized, message)
		}
	}
	normalized = mergeClaudeRequestMessages(normalized)

	messagesJSON := []byte("[]")
	for _, message := range normalized {
		messageJSON := []byte(`{"role":"","content":[]}`)
		messageJSON, _ = sjson.SetBytes(messageJSON, "role", message.role)
		for _, block := range message.blocks {
			messageJSON, _ = sjson.SetRawBytes(messageJSON, "content.-1", block)
		}
		messagesJSON, _ = sjson.SetRawBytes(messagesJSON, "-1", messageJSON)
	}
	updated, err := sjson.SetRawBytes(request, "messages", messagesJSON)
	if err != nil {
		return request
	}
	return updated
}

func claudeRequestContentBlocks(content gjson.Result) [][]byte {
	if content.IsArray() {
		blocks := make([][]byte, 0, len(content.Array()))
		content.ForEach(func(_, block gjson.Result) bool {
			blocks = append(blocks, []byte(block.Raw))
			return true
		})
		return blocks
	}
	if content.Type == gjson.String {
		block := []byte(`{"type":"text","text":""}`)
		block, _ = sjson.SetBytes(block, "text", content.String())
		return [][]byte{block}
	}
	return nil
}

func mergeClaudeRequestMessages(messages []claudeRequestMessage) []claudeRequestMessage {
	merged := make([]claudeRequestMessage, 0, len(messages))
	for _, message := range messages {
		if len(merged) > 0 && merged[len(merged)-1].role == message.role {
			merged[len(merged)-1].blocks = append(merged[len(merged)-1].blocks, message.blocks...)
			continue
		}
		merged = append(merged, message)
	}
	return merged
}

func convertResponsesToolToClaudeTools(tool gjson.Result, toolNameMap map[string]string) [][]byte {
	toolType := strings.TrimSpace(tool.Get("type").String())
	switch toolType {
	case "", "function":
		if tJSON, ok := convertResponsesFunctionToolToClaude(tool, ""); ok {
			return [][]byte{tJSON}
		}
	case "custom":
		if tJSON, ok := convertResponsesCustomToolToClaude(tool); ok {
			return [][]byte{tJSON}
		}
	case "namespace":
		return convertResponsesNamespaceToolToClaude(tool, toolNameMap)
	case "web_search":
		if tJSON, ok := convertResponsesWebSearchToolToClaude(tool); ok {
			if name := gjson.GetBytes(tJSON, "name").String(); name != "" {
				toolNameMap[name] = name
			}
			return [][]byte{tJSON}
		}
	default:
		if isUnsupportedOpenAIBuiltinToolType(toolType) {
			return nil
		}
		if tool.Get("name").String() != "" {
			return [][]byte{[]byte(tool.Raw)}
		}
	}
	return nil
}

func convertResponsesCustomToolToClaude(tool gjson.Result) ([]byte, bool) {
	name := responsesToolName(tool)
	if name == "" {
		return nil, false
	}
	tJSON := []byte(`{"name":"","description":"","input_schema":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}}`)
	tJSON, _ = sjson.SetBytes(tJSON, "name", name)
	if description := responsesToolDescription(tool); description != "" {
		tJSON, _ = sjson.SetBytes(tJSON, "description", description)
	}
	return tJSON, true
}

func convertResponsesNamespaceToolToClaude(tool gjson.Result, toolNameMap map[string]string) [][]byte {
	namespaceName := strings.TrimSpace(tool.Get("name").String())
	children := tool.Get("tools")
	if !children.Exists() || !children.IsArray() {
		return nil
	}

	var out [][]byte
	children.ForEach(func(_, child gjson.Result) bool {
		childName := responsesToolName(child)
		qualifiedName := qualifyResponsesNamespaceToolName(namespaceName, childName)
		if tJSON, ok := convertResponsesFunctionToolToClaude(child, qualifiedName); ok {
			out = append(out, tJSON)
			toolNameMap[qualifiedName] = qualifiedName
			if childName != "" {
				toolNameMap[childName] = qualifiedName
			}
		}
		return true
	})
	return out
}

func convertResponsesFunctionToolToClaude(tool gjson.Result, overrideName string) ([]byte, bool) {
	name := strings.TrimSpace(overrideName)
	if name == "" {
		name = responsesToolName(tool)
	}
	if name == "" {
		return nil, false
	}

	tJSON := []byte(`{"name":"","description":"","input_schema":{}}`)
	tJSON, _ = sjson.SetBytes(tJSON, "name", name)
	if d := responsesToolDescription(tool); d != "" {
		tJSON, _ = sjson.SetBytes(tJSON, "description", d)
	}
	tJSON, _ = sjson.SetRawBytes(tJSON, "input_schema", normalizeClaudeToolInputSchema(responsesToolParameters(tool)))
	return tJSON, true
}

func convertResponsesWebSearchToolToClaude(tool gjson.Result) ([]byte, bool) {
	if externalWebAccess := tool.Get("external_web_access"); externalWebAccess.Exists() && !externalWebAccess.Bool() {
		return nil, false
	}

	name := strings.TrimSpace(tool.Get("name").String())
	if name == "" {
		name = "web_search"
	}
	tJSON := []byte(`{"type":"web_search_20250305","name":""}`)
	tJSON, _ = sjson.SetBytes(tJSON, "name", name)
	if maxUses := tool.Get("max_uses"); maxUses.Exists() {
		tJSON, _ = sjson.SetBytes(tJSON, "max_uses", maxUses.Int())
	}
	if allowedDomains := tool.Get("filters.allowed_domains"); allowedDomains.Exists() && allowedDomains.IsArray() {
		tJSON, _ = sjson.SetRawBytes(tJSON, "allowed_domains", []byte(allowedDomains.Raw))
	}
	if userLocation := tool.Get("user_location"); userLocation.Exists() && userLocation.IsObject() {
		tJSON, _ = sjson.SetRawBytes(tJSON, "user_location", []byte(userLocation.Raw))
	}
	return tJSON, true
}

func responsesToolName(tool gjson.Result) string {
	if name := strings.TrimSpace(tool.Get("name").String()); name != "" {
		return name
	}
	return strings.TrimSpace(tool.Get("function.name").String())
}

func responsesToolDescription(tool gjson.Result) string {
	if description := tool.Get("description").String(); description != "" {
		return description
	}
	return tool.Get("function.description").String()
}

func responsesToolParameters(tool gjson.Result) gjson.Result {
	for _, path := range []string{
		"parameters",
		"parametersJsonSchema",
		"input_schema",
		"function.parameters",
		"function.parametersJsonSchema",
	} {
		if parameters := tool.Get(path); parameters.Exists() {
			return parameters
		}
	}
	return gjson.Result{}
}

func normalizeClaudeToolInputSchema(parameters gjson.Result) []byte {
	raw := strings.TrimSpace(parameters.Raw)
	if raw == "" || raw == "null" || !gjson.Valid(raw) {
		return []byte(`{"type":"object","properties":{}}`)
	}
	result := gjson.Parse(raw)
	if !result.IsObject() {
		return []byte(`{"type":"object","properties":{}}`)
	}
	schema := []byte(raw)
	schemaType := result.Get("type").String()
	if schemaType == "" {
		schema, _ = sjson.SetBytes(schema, "type", "object")
		schemaType = "object"
	}
	if schemaType == "object" && !result.Get("properties").Exists() {
		schema, _ = sjson.SetRawBytes(schema, "properties", []byte(`{}`))
	}
	return schema
}

func qualifyResponsesNamespaceToolName(namespaceName, childName string) string {
	childName = strings.TrimSpace(childName)
	if childName == "" || namespaceName == "" || strings.HasPrefix(childName, "mcp__") {
		return childName
	}
	if strings.HasPrefix(childName, namespaceName) {
		return childName
	}
	if strings.HasSuffix(namespaceName, "__") {
		return namespaceName + childName
	}
	return namespaceName + "__" + childName
}

func isUnsupportedOpenAIBuiltinToolType(toolType string) bool {
	switch toolType {
	case "image_generation", "file_search", "code_interpreter", "computer_use_preview":
		return true
	default:
		return false
	}
}
