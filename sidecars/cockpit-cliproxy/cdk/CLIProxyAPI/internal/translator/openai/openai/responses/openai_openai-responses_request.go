package responses

import (
	"crypto/sha256"
	"encoding/json"
	"regexp"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const customToolChatParametersJSON = `{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}`
const toolSearchChatName = "tool_search"
const chatToolNameMaxLen = 64

var chatToolNameInvalidChars = regexp.MustCompile(`[^A-Za-z0-9_-]`)

func isResponsesToolCallItem(itemType string) bool {
	return itemType == "function_call" || itemType == "custom_tool_call" || itemType == "tool_search_call"
}

func isResponsesToolOutputItem(itemType string) bool {
	return itemType == "function_call_output" || itemType == "custom_tool_call_output" || itemType == "tool_search_output"
}

func customToolInputArguments(input string) string {
	out := []byte(`{"input":""}`)
	out, _ = sjson.SetBytes(out, "input", input)
	return string(out)
}

func canonicalizeToolArguments(arguments string) string {
	if strings.TrimSpace(arguments) == "" {
		return "{}"
	}
	return arguments
}

func flattenNamespaceToolName(namespace, name string) string {
	fullName := namespace + "__" + name
	fullName = chatToolNameInvalidChars.ReplaceAllString(fullName, "_")
	if len(fullName) <= chatToolNameMaxLen {
		return fullName
	}
	sum := sha256.Sum256([]byte(fullName))
	suffix := hexPrefix(sum[:], 10)
	prefixLen := chatToolNameMaxLen - len(suffix) - 1
	if prefixLen < 1 {
		return suffix
	}
	return strings.TrimRight(fullName[:prefixLen], "_-") + "_" + suffix
}

func hexPrefix(bytes []byte, chars int) string {
	const table = "0123456789abcdef"
	out := make([]byte, len(bytes)*2)
	for i, b := range bytes {
		out[i*2] = table[b>>4]
		out[i*2+1] = table[b&0x0f]
	}
	if chars > len(out) {
		chars = len(out)
	}
	return string(out[:chars])
}

func toolSearchArguments(item gjson.Result) string {
	arguments := item.Get("arguments")
	if arguments.Exists() {
		if arguments.Type == gjson.String {
			return canonicalizeToolArguments(arguments.String())
		}
		return arguments.Raw
	}
	return "{}"
}

func appendChatTool(chatTools *[]interface{}, name, description, parametersRaw string) {
	if strings.TrimSpace(name) == "" {
		return
	}
	chatTool := []byte(`{"type":"function","function":{"name":"","description":"","parameters":{}}}`)
	chatTool, _ = sjson.SetBytes(chatTool, "function.name", name)
	if description != "" {
		chatTool, _ = sjson.SetBytes(chatTool, "function.description", description)
	}
	if parametersRaw != "" && gjson.Valid(parametersRaw) {
		chatTool, _ = sjson.SetRawBytes(chatTool, "function.parameters", []byte(parametersRaw))
	}
	*chatTools = append(*chatTools, gjson.ParseBytes(chatTool).Value())
}

func responsesInputImageToChatContentPart(contentItem gjson.Result) []byte {
	imageURL := contentItem.Get("image_url")
	contentPart := []byte(`{"type":"image_url","image_url":{"url":""}}`)
	if imageURL.Exists() && imageURL.IsObject() {
		contentPart, _ = sjson.SetRawBytes(contentPart, "image_url", []byte(imageURL.Raw))
		return contentPart
	}
	contentPart, _ = sjson.SetBytes(contentPart, "image_url.url", imageURL.String())
	return contentPart
}

func appendResponsesToolToChatTools(chatTools *[]interface{}, tool gjson.Result, namespace string) {
	toolType := tool.Get("type").String()
	switch toolType {
	case "function":
		name := tool.Get("name").String()
		chatName := name
		if namespace != "" {
			chatName = flattenNamespaceToolName(namespace, name)
		}
		appendChatTool(chatTools, chatName, tool.Get("description").String(), tool.Get("parameters").Raw)
	case "custom":
		name := tool.Get("name").String()
		chatName := name
		if namespace != "" {
			chatName = flattenNamespaceToolName(namespace, name)
		}
		appendChatTool(chatTools, chatName, tool.Get("description").String(), customToolChatParametersJSON)
	case "tool_search":
		appendChatTool(chatTools, toolSearchChatName, "Search and load Codex tools, plugins, connectors, and MCP namespaces for the current task.", `{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}}}`)
	case "namespace":
		childNamespace := tool.Get("name").String()
		if childNamespace == "" {
			childNamespace = namespace
		}
		children := tool.Get("tools")
		if !children.IsArray() {
			children = tool.Get("children")
		}
		if children.IsArray() {
			children.ForEach(func(_, child gjson.Result) bool {
				appendResponsesToolToChatTools(chatTools, child, childNamespace)
				return true
			})
		}
	}
}

func appendToolSearchOutputTools(chatTools *[]interface{}, value gjson.Result) {
	if !value.Exists() {
		return
	}
	switch {
	case value.IsArray():
		value.ForEach(func(_, item gjson.Result) bool {
			appendToolSearchOutputTools(chatTools, item)
			return true
		})
	case value.IsObject():
		if value.Get("type").String() == "tool_search_output" {
			tools := value.Get("tools")
			if tools.IsArray() {
				tools.ForEach(func(_, tool gjson.Result) bool {
					appendResponsesToolToChatTools(chatTools, tool, "")
					return true
				})
			}
		}
		value.ForEach(func(_, child gjson.Result) bool {
			appendToolSearchOutputTools(chatTools, child)
			return true
		})
	}
}

func collapseSystemMessagesToHead(rawJSON []byte) []byte {
	var root map[string]any
	if err := json.Unmarshal(rawJSON, &root); err != nil {
		return rawJSON
	}
	rawMessages, ok := root["messages"].([]any)
	if !ok || len(rawMessages) == 0 {
		return rawJSON
	}

	systemParts := make([]string, 0)
	nextMessages := make([]any, 0, len(rawMessages))
	for _, rawMessage := range rawMessages {
		message, ok := rawMessage.(map[string]any)
		if !ok || message["role"] != "system" {
			nextMessages = append(nextMessages, rawMessage)
			continue
		}
		if text := chatMessageContentText(message["content"]); text != "" {
			systemParts = append(systemParts, text)
		}
	}
	if len(systemParts) == 0 {
		return rawJSON
	}

	systemContent := strings.Join(systemParts, "\n\n")
	systemMessage := map[string]any{
		"role":    "system",
		"content": systemContent,
	}
	root["messages"] = append([]any{systemMessage}, nextMessages...)

	next, err := json.Marshal(root)
	if err != nil {
		return rawJSON
	}
	return next
}

func chatMessageContentText(content any) string {
	switch value := content.(type) {
	case string:
		return value
	case []any:
		parts := make([]string, 0, len(value))
		for _, rawPart := range value {
			part, ok := rawPart.(map[string]any)
			if !ok {
				continue
			}
			text, ok := part["text"].(string)
			if ok && text != "" {
				parts = append(parts, text)
			}
		}
		return strings.Join(parts, "\n\n")
	default:
		return ""
	}
}

// ConvertOpenAIResponsesRequestToOpenAIChatCompletions converts OpenAI responses format to OpenAI chat completions format.
// It transforms the OpenAI responses API format (with instructions and input array) into the standard
// OpenAI chat completions format (with messages array and system content).
//
// The conversion handles:
// 1. Model name and streaming configuration
// 2. Instructions to system message conversion
// 3. Input array to messages array transformation
// 4. Tool definitions and tool choice conversion
// 5. Function calls and function results handling
// 6. Generation parameters mapping (max_tokens, reasoning, etc.)
//
// Parameters:
//   - modelName: The name of the model to use for the request
//   - rawJSON: The raw JSON request data in OpenAI responses format
//   - stream: A boolean indicating if the request is for a streaming response
//
// Returns:
//   - []byte: The transformed request data in OpenAI chat completions format
func ConvertOpenAIResponsesRequestToOpenAIChatCompletions(modelName string, inputRawJSON []byte, stream bool) []byte {
	rawJSON := inputRawJSON
	// Base OpenAI chat completions template with default values
	out := []byte(`{"model":"","messages":[],"stream":false}`)

	root := gjson.ParseBytes(rawJSON)

	// Set model name
	out, _ = sjson.SetBytes(out, "model", modelName)

	// Set stream configuration
	out, _ = sjson.SetBytes(out, "stream", stream)
	if stream {
		out, _ = sjson.SetBytes(out, "stream_options.include_usage", true)
	}

	// Map generation parameters from responses format to chat completions format
	if maxTokens := root.Get("max_output_tokens"); maxTokens.Exists() {
		out, _ = sjson.SetBytes(out, "max_tokens", maxTokens.Int())
	}

	if parallelToolCalls := root.Get("parallel_tool_calls"); parallelToolCalls.Exists() {
		out, _ = sjson.SetBytes(out, "parallel_tool_calls", parallelToolCalls.Bool())
	}

	// Convert instructions to system message
	if instructions := root.Get("instructions"); instructions.Exists() {
		systemMessage := []byte(`{"role":"system","content":""}`)
		systemMessage, _ = sjson.SetBytes(systemMessage, "content", instructions.String())
		out, _ = sjson.SetRawBytes(out, "messages.-1", systemMessage)
	}

	// Convert input array to messages
	if input := root.Get("input"); input.Exists() && input.IsArray() {
		inputItems := input.Array()
		outputCallIDs := make(map[string]struct{})
		for _, item := range inputItems {
			if !isResponsesToolOutputItem(item.Get("type").String()) {
				continue
			}
			callID := strings.TrimSpace(item.Get("call_id").String())
			if callID == "" {
				continue
			}
			outputCallIDs[callID] = struct{}{}
		}

		pendingToolCalls := make([]interface{}, 0)
		pendingToolCallIDs := make([]string, 0)
		awaitingToolOutputs := make(map[string]struct{})
		deferredMessages := make([][]byte, 0)

		flushPendingToolCalls := func() {
			if len(pendingToolCalls) == 0 {
				return
			}
			assistantMessage := []byte(`{"role":"assistant","tool_calls":[]}`)
			assistantMessage, _ = sjson.SetBytes(assistantMessage, "tool_calls", pendingToolCalls)
			out, _ = sjson.SetRawBytes(out, "messages.-1", assistantMessage)
			for _, id := range pendingToolCallIDs {
				if strings.TrimSpace(id) == "" {
					continue
				}
				awaitingToolOutputs[id] = struct{}{}
			}
			pendingToolCalls = pendingToolCalls[:0]
			pendingToolCallIDs = pendingToolCallIDs[:0]
		}
		flushDeferredMessages := func() {
			for _, message := range deferredMessages {
				out, _ = sjson.SetRawBytes(out, "messages.-1", message)
			}
			deferredMessages = deferredMessages[:0]
		}
		hasAwaitingToolOutput := func() bool {
			for id := range awaitingToolOutputs {
				if _, ok := outputCallIDs[id]; ok {
					return true
				}
			}
			return false
		}
		appendRegularMessage := func(message []byte) {
			// Keep tool-call adjacency strict for providers that require
			// assistant(tool_calls) -> tool(tool_call_id) with no message in between.
			if hasAwaitingToolOutput() {
				deferredMessages = append(deferredMessages, message)
				return
			}
			out, _ = sjson.SetRawBytes(out, "messages.-1", message)
		}

		for _, item := range inputItems {
			itemType := item.Get("type").String()
			if itemType == "" && item.Get("role").String() != "" {
				itemType = "message"
			}
			if !isResponsesToolCallItem(itemType) {
				flushPendingToolCalls()
			}

			switch itemType {
			case "message", "":
				// Handle regular message conversion
				role := item.Get("role").String()
				if role == "developer" {
					role = "system"
				}
				message := []byte(`{"role":"","content":[]}`)
				message, _ = sjson.SetBytes(message, "role", role)

				if content := item.Get("content"); content.Exists() && content.IsArray() {
					var messageContent string
					var toolCalls []interface{}

					content.ForEach(func(_, contentItem gjson.Result) bool {
						contentType := contentItem.Get("type").String()
						if contentType == "" {
							contentType = "input_text"
						}

						switch contentType {
						case "input_text", "output_text":
							text := contentItem.Get("text").String()
							contentPart := []byte(`{"type":"text","text":""}`)
							contentPart, _ = sjson.SetBytes(contentPart, "text", text)
							message, _ = sjson.SetRawBytes(message, "content.-1", contentPart)
						case "input_image":
							contentPart := responsesInputImageToChatContentPart(contentItem)
							message, _ = sjson.SetRawBytes(message, "content.-1", contentPart)
						}
						return true
					})

					if messageContent != "" {
						message, _ = sjson.SetBytes(message, "content", messageContent)
					}

					if len(toolCalls) > 0 {
						message, _ = sjson.SetBytes(message, "tool_calls", toolCalls)
					}
				} else if content.Type == gjson.String {
					message, _ = sjson.SetBytes(message, "content", content.String())
				}

				appendRegularMessage(message)

			case "function_call", "custom_tool_call", "tool_search_call":
				// Buffer consecutive function calls and emit them as one assistant message.
				toolCall := []byte(`{"id":"","type":"function","function":{"name":"","arguments":""}}`)

				if callId := item.Get("call_id"); callId.Exists() {
					toolCall, _ = sjson.SetBytes(toolCall, "id", callId.String())
				} else if itemType == "tool_search_call" {
					toolCall, _ = sjson.SetBytes(toolCall, "id", item.Get("id").String())
				}

				if itemType == "tool_search_call" {
					toolCall, _ = sjson.SetBytes(toolCall, "function.name", toolSearchChatName)
				} else if name := item.Get("name"); name.Exists() {
					chatName := name.String()
					if namespace := item.Get("namespace").String(); namespace != "" {
						chatName = flattenNamespaceToolName(namespace, chatName)
					}
					toolCall, _ = sjson.SetBytes(toolCall, "function.name", chatName)
				}

				if itemType == "custom_tool_call" {
					toolCall, _ = sjson.SetBytes(toolCall, "function.arguments", customToolInputArguments(item.Get("input").String()))
				} else if itemType == "tool_search_call" {
					toolCall, _ = sjson.SetBytes(toolCall, "function.arguments", toolSearchArguments(item))
				} else if arguments := item.Get("arguments"); arguments.Exists() {
					toolCall, _ = sjson.SetBytes(toolCall, "function.arguments", canonicalizeToolArguments(arguments.String()))
				} else {
					toolCall, _ = sjson.SetBytes(toolCall, "function.arguments", "{}")
				}
				pendingToolCalls = append(pendingToolCalls, gjson.ParseBytes(toolCall).Value())
				callID := strings.TrimSpace(item.Get("call_id").String())
				if callID == "" && itemType == "tool_search_call" {
					callID = strings.TrimSpace(item.Get("id").String())
				}
				if callID != "" {
					pendingToolCallIDs = append(pendingToolCallIDs, callID)
				}

			case "function_call_output", "custom_tool_call_output", "tool_search_output":
				// Handle function call output conversion to tool message
				toolMessage := []byte(`{"role":"tool","tool_call_id":"","content":""}`)
				callID := ""

				if callId := item.Get("call_id"); callId.Exists() {
					callID = strings.TrimSpace(callId.String())
					toolMessage, _ = sjson.SetBytes(toolMessage, "tool_call_id", callID)
				}

				if output := item.Get("output"); output.Exists() {
					toolMessage, _ = sjson.SetBytes(toolMessage, "content", output.String())
				}

				out, _ = sjson.SetRawBytes(out, "messages.-1", toolMessage)
				if callID != "" {
					delete(awaitingToolOutputs, callID)
				}
				if len(awaitingToolOutputs) == 0 && len(deferredMessages) > 0 {
					flushDeferredMessages()
				}
			}

		}
		flushPendingToolCalls()
		flushDeferredMessages()
	} else if input.Type == gjson.String {
		msg := []byte(`{}`)
		msg, _ = sjson.SetBytes(msg, "role", "user")
		msg, _ = sjson.SetBytes(msg, "content", input.String())
		out, _ = sjson.SetRawBytes(out, "messages.-1", msg)
	}

	// Convert tools from responses format to chat completions format
	var chatCompletionsTools []interface{}
	if tools := root.Get("tools"); tools.Exists() && tools.IsArray() {
		tools.ForEach(func(_, tool gjson.Result) bool {
			appendResponsesToolToChatTools(&chatCompletionsTools, tool, "")
			return true
		})
	}
	appendToolSearchOutputTools(&chatCompletionsTools, root.Get("input"))
	if len(chatCompletionsTools) > 0 {
		out, _ = sjson.SetBytes(out, "tools", chatCompletionsTools)
	}

	if reasoningEffort := root.Get("reasoning.effort"); reasoningEffort.Exists() {
		effort := strings.ToLower(strings.TrimSpace(reasoningEffort.String()))
		if effort != "" {
			out, _ = sjson.SetBytes(out, "reasoning_effort", effort)
		}
	}

	// Convert tool_choice if present
	if toolChoice := root.Get("tool_choice"); toolChoice.Exists() {
		out, _ = sjson.SetBytes(out, "tool_choice", toolChoice.String())
	}

	return collapseSystemMessagesToHead(out)
}
