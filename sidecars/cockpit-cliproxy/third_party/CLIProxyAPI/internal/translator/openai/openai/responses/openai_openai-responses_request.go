package responses

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"regexp"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const customToolChatParametersJSON = `{"type":"object","properties":{"input":{"type":"string"}},"required":["input"]}`
const toolSearchChatName = "tool_search"
const chatToolNameMaxLen = 64

const (
	toolOutputMediaMarker      = "[Tool output media moved to the following user message]"
	toolOutputMediaAttribution = "[Tool output media for call %s]"
)

type chatToolOutputMediaPart struct {
	Type     string                       `json:"type"`
	Text     string                       `json:"text,omitempty"`
	ImageURL *chatToolOutputMediaImageURL `json:"image_url,omitempty"`
}

type chatToolOutputMediaImageURL struct {
	URL string `json:"url"`
}

var chatToolNameInvalidChars = regexp.MustCompile(`[^A-Za-z0-9_-]`)

func responsesServiceTier(value string) string {
	if strings.ToLower(strings.TrimSpace(value)) == "priority" {
		return "priority"
	}
	return ""
}

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

// appendAdditionalToolsFromInput collects tools declared in Codex Desktop
// Responses Lite "additional_tools" input items.
func appendAdditionalToolsFromInput(chatTools *[]interface{}, input gjson.Result) {
	if !input.Exists() || !input.IsArray() {
		return
	}
	input.ForEach(func(_, item gjson.Result) bool {
		if item.Get("type").String() != "additional_tools" {
			return true
		}
		tools := item.Get("tools")
		if !tools.IsArray() {
			return true
		}
		tools.ForEach(func(_, tool gjson.Result) bool {
			appendResponsesToolToChatTools(chatTools, tool, "")
			return true
		})
		return true
	})
}

// responsesToolOutputText flattens a tool output value that may be a plain
// string or an array of content parts into a single text payload.
func responsesToolOutputText(output gjson.Result) string {
	if output.Type == gjson.String {
		return output.String()
	}
	if output.IsArray() {
		var b strings.Builder
		output.ForEach(func(_, part gjson.Result) bool {
			if part.Type == gjson.String {
				b.WriteString(part.String())
				return true
			}
			if text := part.Get("text"); text.Exists() {
				b.WriteString(text.String())
			}
			return true
		})
		return b.String()
	}
	if output.Exists() {
		return output.Raw
	}
	return ""
}

// extractResponsesToolOutputMedia moves image parts out of tool messages. Chat
// Completions only permits images in user messages, while Responses tool output
// may contain image content parts directly.
func extractResponsesToolOutputMedia(outputRaw []byte) (string, []chatToolOutputMediaPart, bool) {
	outputRaw = bytes.TrimSpace(outputRaw)
	if len(outputRaw) == 0 || bytes.Equal(outputRaw, []byte("null")) {
		return "", nil, false
	}

	var outputString string
	if err := json.Unmarshal(outputRaw, &outputString); err == nil {
		if isResponsesToolOutputImageDataURL(outputString) {
			return toolOutputMediaMarker, []chatToolOutputMediaPart{responsesToolOutputImagePart(outputString)}, true
		}

		nested, ok := decodeResponsesToolOutputJSON([]byte(outputString))
		if !ok {
			return "", nil, false
		}
		rewritten, media, changed := rewriteResponsesToolOutputMediaValue(nested)
		if !changed {
			return "", nil, false
		}
		encoded, err := json.Marshal(rewritten)
		if err != nil {
			return "", nil, false
		}
		return string(encoded), media, true
	}

	value, ok := decodeResponsesToolOutputJSON(outputRaw)
	if !ok {
		return "", nil, false
	}
	rewritten, media, changed := rewriteResponsesToolOutputMediaValue(value)
	if !changed {
		return "", nil, false
	}
	encoded, err := json.Marshal(rewritten)
	if err != nil {
		return "", nil, false
	}
	return string(encoded), media, true
}

func decodeResponsesToolOutputJSON(raw []byte) (any, bool) {
	if !json.Valid(raw) {
		return nil, false
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	var value any
	if err := decoder.Decode(&value); err != nil {
		return nil, false
	}
	return value, true
}

func rewriteResponsesToolOutputMediaValue(value any) (any, []chatToolOutputMediaPart, bool) {
	switch typed := value.(type) {
	case []any:
		var media []chatToolOutputMediaPart
		changed := false
		for index, item := range typed {
			rewritten, itemMedia, itemChanged := rewriteResponsesToolOutputMediaValue(item)
			if !itemChanged {
				continue
			}
			typed[index] = rewritten
			media = append(media, itemMedia...)
			changed = true
		}
		return typed, media, changed
	case map[string]any:
		if imageURL, ok := recognizedResponsesToolOutputImageURL(typed); ok {
			return map[string]any{
				"type": "input_text",
				"text": toolOutputMediaMarker,
			}, []chatToolOutputMediaPart{responsesToolOutputImagePart(imageURL)}, true
		}

		content, ok := typed["content"]
		if !ok {
			return typed, nil, false
		}
		rewritten, media, changed := rewriteResponsesToolOutputMediaValue(content)
		if !changed {
			return typed, nil, false
		}
		typed["content"] = rewritten
		return typed, media, true
	default:
		return value, nil, false
	}
}

func recognizedResponsesToolOutputImageURL(value map[string]any) (string, bool) {
	partType, _ := value["type"].(string)
	if partType != "input_image" && partType != "image_url" {
		return "", false
	}

	switch imageURL := value["image_url"].(type) {
	case string:
		return imageURL, strings.TrimSpace(imageURL) != ""
	case map[string]any:
		url, _ := imageURL["url"].(string)
		return url, strings.TrimSpace(url) != ""
	default:
		return "", false
	}
}

func isResponsesToolOutputImageDataURL(value string) bool {
	const prefix = "data:image/"
	const separator = ";base64,"
	if !strings.HasPrefix(value, prefix) {
		return false
	}
	separatorIndex := strings.Index(value[len(prefix):], separator)
	if separatorIndex <= 0 {
		return false
	}
	payloadIndex := len(prefix) + separatorIndex + len(separator)
	return payloadIndex < len(value)
}

func responsesToolOutputImagePart(imageURL string) chatToolOutputMediaPart {
	return chatToolOutputMediaPart{
		Type:     "image_url",
		ImageURL: &chatToolOutputMediaImageURL{URL: imageURL},
	}
}

// normalizeChatMessagesWithResponsesToolOutputMedia restores strict Chat
// Completions tool adjacency, then places extracted media after the complete
// reply batch in the assistant's tool-call order.
func normalizeChatMessagesWithResponsesToolOutputMedia(rawJSON []byte, mediaByCallID map[string][]chatToolOutputMediaPart) []byte {
	var root map[string]any
	decoder := json.NewDecoder(bytes.NewReader(rawJSON))
	decoder.UseNumber()
	if err := decoder.Decode(&root); err != nil {
		return rawJSON
	}
	messages, ok := root["messages"].([]any)
	if !ok {
		return rawJSON
	}

	replies := make(map[string]map[string]any)
	for _, rawMessage := range messages {
		message, ok := rawMessage.(map[string]any)
		if !ok || message["role"] != "tool" {
			continue
		}
		callID, _ := message["tool_call_id"].(string)
		if callID != "" {
			replies[callID] = message
		}
	}

	normalized := make([]any, 0, len(messages)+1)
	for _, rawMessage := range messages {
		message, ok := rawMessage.(map[string]any)
		if !ok {
			normalized = append(normalized, rawMessage)
			continue
		}
		role, _ := message["role"].(string)
		if role == "tool" {
			callID, _ := message["tool_call_id"].(string)
			if callID == "" {
				normalized = append(normalized, message)
			}
			continue
		}

		toolCalls, _ := message["tool_calls"].([]any)
		if len(toolCalls) == 0 {
			normalized = append(normalized, message)
			continue
		}

		keptCalls := make([]any, 0, len(toolCalls))
		keptIDs := make([]string, 0, len(toolCalls))
		for _, rawCall := range toolCalls {
			call, ok := rawCall.(map[string]any)
			if !ok {
				continue
			}
			callID, _ := call["id"].(string)
			if _, ok := replies[callID]; !ok || callID == "" {
				continue
			}
			keptCalls = append(keptCalls, call)
			keptIDs = append(keptIDs, callID)
		}
		if len(keptCalls) == 0 {
			delete(message, "tool_calls")
			if content, exists := message["content"]; exists && content != nil && content != "" {
				normalized = append(normalized, message)
			}
			continue
		}

		message["tool_calls"] = keptCalls
		normalized = append(normalized, message)
		for _, callID := range keptIDs {
			normalized = append(normalized, replies[callID])
		}

		mediaParts := make([]chatToolOutputMediaPart, 0)
		for _, callID := range keptIDs {
			media := mediaByCallID[callID]
			if len(media) == 0 {
				continue
			}
			mediaParts = append(mediaParts, chatToolOutputMediaPart{
				Type: "text",
				Text: fmt.Sprintf(toolOutputMediaAttribution, callID),
			})
			mediaParts = append(mediaParts, media...)
		}
		if len(mediaParts) > 0 {
			normalized = append(normalized, map[string]any{
				"role":    "user",
				"content": mediaParts,
			})
		}
	}

	root["messages"] = normalized
	next, err := json.Marshal(root)
	if err != nil {
		return rawJSON
	}
	return next
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

func appendResponsesReasoningText(existing, incoming string, unique bool) string {
	incoming = strings.TrimSpace(incoming)
	if incoming == "" {
		return existing
	}
	existing = strings.TrimSpace(existing)
	if existing == "" {
		return incoming
	}
	if unique && strings.Contains(existing, incoming) {
		return existing
	}
	return existing + "\n\n" + incoming
}

func responsesReasoningNodeText(value gjson.Result) string {
	if !value.Exists() {
		return ""
	}
	if value.Type == gjson.String {
		return strings.TrimSpace(value.String())
	}
	if value.IsArray() {
		parts := make([]string, 0)
		value.ForEach(func(_, part gjson.Result) bool {
			if text := responsesReasoningNodeText(part); text != "" {
				parts = append(parts, text)
			}
			return true
		})
		return strings.Join(parts, "\n\n")
	}
	if !value.IsObject() {
		return ""
	}
	for _, key := range []string{"text", "content", "summary", "parts"} {
		if text := responsesReasoningNodeText(value.Get(key)); text != "" {
			return text
		}
	}
	return ""
}

func responsesItemReasoningText(item gjson.Result) string {
	for _, key := range []string{"reasoning_content", "reasoning", "reasoning_details"} {
		if text := responsesReasoningNodeText(item.Get(key)); text != "" {
			return text
		}
	}
	return ""
}

func responsesReasoningItemText(item gjson.Result) string {
	for _, key := range []string{"reasoning_content", "content", "text", "summary"} {
		if text := responsesReasoningNodeText(item.Get(key)); text != "" {
			return text
		}
	}
	return ""
}

func backfillResponsesToolCallReasoningPlaceholders(rawJSON []byte) []byte {
	out := rawJSON
	for index, message := range gjson.GetBytes(rawJSON, "messages").Array() {
		if message.Get("role").String() != "assistant" || len(message.Get("tool_calls").Array()) == 0 {
			continue
		}
		if strings.TrimSpace(message.Get("reasoning_content").String()) != "" {
			continue
		}
		out, _ = sjson.SetBytes(out, fmt.Sprintf("messages.%d.reasoning_content", index), "tool call")
	}
	return out
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
	mediaByCallID := make(map[string][]chatToolOutputMediaPart)
	sawToolOutputMedia := false
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

	if serviceTier := responsesServiceTier(root.Get("service_tier").String()); serviceTier != "" {
		out, _ = sjson.SetBytes(out, "service_tier", serviceTier)
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
		pendingReasoning := ""
		lastAssistantIndex := -1
		awaitingToolOutputs := make(map[string]struct{})
		deferredMessages := make([][]byte, 0)
		appendOutputMessage := func(message []byte) {
			messageIndex := len(gjson.GetBytes(out, "messages").Array())
			out, _ = sjson.SetRawBytes(out, "messages.-1", message)
			switch gjson.GetBytes(message, "role").String() {
			case "assistant":
				lastAssistantIndex = messageIndex
			case "tool":
			default:
				lastAssistantIndex = -1
			}
		}
		attachPendingReasoningToPreviousAssistant := func() {
			reasoning := strings.TrimSpace(pendingReasoning)
			pendingReasoning = ""
			if reasoning == "" || lastAssistantIndex < 0 {
				return
			}
			path := fmt.Sprintf("messages.%d", lastAssistantIndex)
			if gjson.GetBytes(out, path+".role").String() != "assistant" {
				return
			}
			existing := gjson.GetBytes(out, path+".reasoning_content").String()
			combined := appendResponsesReasoningText(existing, reasoning, false)
			out, _ = sjson.SetBytes(out, path+".reasoning_content", combined)
		}

		flushPendingToolCalls := func() {
			if len(pendingToolCalls) == 0 {
				return
			}
			assistantMessage := []byte(`{"role":"assistant","tool_calls":[]}`)
			assistantMessage, _ = sjson.SetBytes(assistantMessage, "tool_calls", pendingToolCalls)
			if reasoning := strings.TrimSpace(pendingReasoning); reasoning != "" {
				assistantMessage, _ = sjson.SetBytes(assistantMessage, "reasoning_content", reasoning)
				pendingReasoning = ""
			}
			appendOutputMessage(assistantMessage)
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
				appendOutputMessage(message)
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
			appendOutputMessage(message)
		}

		for _, item := range inputItems {
			itemType := item.Get("type").String()
			if itemType == "" && item.Get("role").String() != "" {
				itemType = "message"
			}
			if !isResponsesToolCallItem(itemType) && itemType != "reasoning" {
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

				if role == "assistant" {
					pendingReasoning = appendResponsesReasoningText(
						pendingReasoning,
						responsesItemReasoningText(item),
						false,
					)
					if reasoning := strings.TrimSpace(pendingReasoning); reasoning != "" {
						message, _ = sjson.SetBytes(message, "reasoning_content", reasoning)
						pendingReasoning = ""
					}
				} else {
					attachPendingReasoningToPreviousAssistant()
					// A deferred user/system message is still a semantic turn boundary.
					lastAssistantIndex = -1
				}

				appendRegularMessage(message)

			case "function_call", "custom_tool_call", "tool_search_call":
				pendingReasoning = appendResponsesReasoningText(
					pendingReasoning,
					responsesItemReasoningText(item),
					true,
				)
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
					delete(mediaByCallID, callID)
					outputText, media, rewritten := extractResponsesToolOutputMedia([]byte(output.Raw))
					if rewritten {
						sawToolOutputMedia = true
						if callID != "" {
							mediaByCallID[callID] = media
						}
					} else if output.Type == gjson.String {
						outputText = output.String()
					} else {
						// Preserve object/array bytes when there is no recognized media.
						outputText = output.Raw
					}
					toolMessage, _ = sjson.SetBytes(toolMessage, "content", outputText)
				}

				out, _ = sjson.SetRawBytes(out, "messages.-1", toolMessage)
				if callID != "" {
					delete(awaitingToolOutputs, callID)
				}
				if len(awaitingToolOutputs) == 0 && len(deferredMessages) > 0 {
					flushDeferredMessages()
				}

			case "additional_tools":
				// Codex Desktop Responses Lite may carry tool definitions in
				// input items rather than top-level "tools". Handled below when
				// collecting chat tools.

			case "reasoning":
				// Responses reasoning belongs to the assistant message or tool call
				// that follows it. Keep it pending instead of attaching it backward.
				pendingReasoning = appendResponsesReasoningText(
					pendingReasoning,
					responsesReasoningItemText(item),
					false,
				)
			}

		}
		flushPendingToolCalls()
		flushDeferredMessages()
		// Only genuine trailing reasoning is attached backward. Always consume it
		// here so it cannot leak across a later user turn.
		attachPendingReasoningToPreviousAssistant()
	} else if input.Type == gjson.String {
		msg := []byte(`{}`)
		msg, _ = sjson.SetBytes(msg, "role", "user")
		msg, _ = sjson.SetBytes(msg, "content", input.String())
		out, _ = sjson.SetRawBytes(out, "messages.-1", msg)
	}

	// Convert tools from responses format to chat completions format.
	// Responses Lite often sets top-level tools to null and ships definitions
	// via input items of type "additional_tools".
	var chatCompletionsTools []interface{}
	if tools := root.Get("tools"); tools.Exists() && tools.IsArray() {
		tools.ForEach(func(_, tool gjson.Result) bool {
			appendResponsesToolToChatTools(&chatCompletionsTools, tool, "")
			return true
		})
	}
	appendAdditionalToolsFromInput(&chatCompletionsTools, root.Get("input"))
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

	// Chat Completions providers reject tool_choice when no tools are present.
	if toolChoice := root.Get("tool_choice"); toolChoice.Exists() && len(chatCompletionsTools) > 0 {
		out, _ = sjson.SetBytes(out, "tool_choice", toolChoice.String())
	}

	out = collapseSystemMessagesToHead(out)
	if sawToolOutputMedia {
		out = normalizeChatMessagesWithResponsesToolOutputMedia(out, mediaByCallID)
	}
	// Some thinking providers reject historical assistant tool calls without a
	// non-empty reasoning_content. Run this last so genuine reasoning always wins.
	out = backfillResponsesToolCallReasoningPlaceholders(out)
	return out
}
