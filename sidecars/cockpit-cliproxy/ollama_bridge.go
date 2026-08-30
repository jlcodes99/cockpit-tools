package main

import (
	"bufio"
	"bytes"
	"context"

	"encoding/json"

	"fmt"
	"io"

	"net/http"

	"sort"

	"strings"

	"time"

	"github.com/gin-gonic/gin"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"

	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func stringSliceContainsFold(values []string, target string) bool {
	for _, value := range values {
		if strings.EqualFold(strings.TrimSpace(value), strings.TrimSpace(target)) {
			return true
		}
	}
	return false
}

func (s *relayServer) handleOllamaVersion(c *gin.Context) {
	if _, ok := s.requireAPIKey(c); !ok {
		return
	}
	c.JSON(http.StatusOK, gin.H{"version": ollamaBridgeVersion})
}

func (s *relayServer) handleOllamaTags(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	c.JSON(http.StatusOK, buildOllamaTagsResponse(clientCatalogModelsForAPIKey(s.manifest, spec), time.Now()))
}

func (s *relayServer) handleOllamaShow(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	body, err := readAndRestoreBody(c.Request)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, "failed to read request body", "invalid_request")
		return
	}
	model := requestBodyModel(body)
	if model == "" {
		writeAPIError(c, http.StatusBadRequest, "model is required", "invalid_request")
		return
	}
	_, canonical, ok := s.bodyWithValidatedModel(c, spec, body, model, nil)
	if !ok {
		return
	}
	c.JSON(http.StatusOK, buildOllamaShowResponse(canonical, time.Now()))
}

func (s *relayServer) handleOllamaChat(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	body, err := readAndRestoreBody(c.Request)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, "failed to read request body", "invalid_request")
		return
	}
	if len(bytes.TrimSpace(body)) == 0 {
		writeAPIError(c, http.StatusBadRequest, "request body is required", "invalid_request")
		return
	}
	model := requestBodyModel(body)
	if model == "" {
		writeAPIError(c, http.StatusBadRequest, "model is required", "invalid_request")
		return
	}
	body, canonical, ok := s.bodyWithValidatedModel(c, spec, body, model, nil)
	if !ok {
		return
	}
	openAIBody, stream, err := buildOpenAIChatRequestFromOllama(body)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, err.Error(), "invalid_request")
		return
	}
	if spec.ProviderGateway != nil {
		s.handleOllamaProviderGatewayChat(c, spec.ProviderGateway, openAIBody, canonical, stream)
		return
	}
	if stream {
		s.handleOllamaRuntimeStream(c, openAIBody, canonical)
		return
	}
	s.handleOllamaRuntimeNonStream(c, openAIBody, canonical)
}

func buildOpenAIChatRequestFromOllama(body []byte) ([]byte, bool, error) {
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, false, fmt.Errorf("request body must be a JSON object")
	}
	model, _ := payload["model"].(string)
	if strings.TrimSpace(model) == "" {
		return nil, false, fmt.Errorf("model is required")
	}
	messages, ok := payload["messages"].([]any)
	if !ok {
		return nil, false, fmt.Errorf("messages is required")
	}
	stream := true
	if value, ok := payload["stream"].(bool); ok {
		stream = value
	}
	out := map[string]any{
		"model":    strings.TrimSpace(model),
		"messages": ollamaMessagesToOpenAI(messages),
		"stream":   stream,
	}
	if tools, ok := payload["tools"].([]any); ok && len(tools) > 0 {
		out["tools"] = tools
	}
	if options, ok := payload["options"].(map[string]any); ok {
		if value, ok := options["temperature"].(float64); ok {
			out["temperature"] = value
		}
		if value, ok := options["top_p"].(float64); ok {
			out["top_p"] = value
		}
		if value, ok := options["num_predict"].(float64); ok {
			out["max_tokens"] = int64(value)
		}
	}
	if effort := ollamaThinkingEffort(payload["think"]); effort != "" {
		out["reasoning_effort"] = effort
	}
	if responseFormat := ollamaResponseFormat(payload["format"]); responseFormat != nil {
		out["response_format"] = responseFormat
	}
	raw, err := json.Marshal(out)
	return raw, stream, err
}

func ollamaMessagesToOpenAI(messages []any) []any {
	out := make([]any, 0, len(messages))
	toolCallIDByName := map[string]string{}
	for index, raw := range messages {
		message, _ := raw.(map[string]any)
		role, _ := message["role"].(string)
		switch role {
		case "assistant":
			item := map[string]any{
				"role":    "assistant",
				"content": ollamaMessageContentToOpenAI(message),
			}
			if toolCalls, ok := message["tool_calls"].([]any); ok && len(toolCalls) > 0 {
				item["tool_calls"] = ollamaToolCallsToOpenAI(toolCalls, index, toolCallIDByName)
			}
			out = append(out, item)
		case "tool":
			toolName, _ := message["tool_name"].(string)
			if toolName == "" {
				toolName, _ = message["name"].(string)
			}
			toolCallID, _ := message["tool_call_id"].(string)
			if toolCallID == "" {
				toolCallID = toolCallIDByName[toolName]
			}
			if toolCallID == "" {
				toolCallID = fmt.Sprintf("tool_%d", index)
			}
			out = append(out, map[string]any{
				"role":         "tool",
				"tool_call_id": toolCallID,
				"content":      ollamaContentString(message["content"]),
			})
		default:
			if role != "system" {
				role = "user"
			}
			out = append(out, map[string]any{
				"role":    role,
				"content": ollamaMessageContentToOpenAI(message),
			})
		}
	}
	return out
}

func ollamaToolCallsToOpenAI(toolCalls []any, messageIndex int, toolCallIDByName map[string]string) []any {
	out := make([]any, 0, len(toolCalls))
	for index, raw := range toolCalls {
		toolCall, _ := raw.(map[string]any)
		fn, _ := toolCall["function"].(map[string]any)
		id, _ := toolCall["id"].(string)
		if id == "" {
			id = fmt.Sprintf("tool_%d_%d", messageIndex, index)
		}
		name, _ := fn["name"].(string)
		if name == "" {
			name = "tool"
		}
		toolCallIDByName[name] = id
		out = append(out, map[string]any{
			"id":   id,
			"type": "function",
			"function": map[string]any{
				"name":      name,
				"arguments": ollamaArgumentsString(fn["arguments"]),
			},
		})
	}
	return out
}

func ollamaMessageContentToOpenAI(message map[string]any) any {
	text := ollamaContentString(message["content"])
	images, _ := message["images"].([]any)
	if len(images) == 0 {
		return text
	}
	parts := make([]any, 0, len(images)+1)
	if text != "" {
		parts = append(parts, map[string]any{"type": "text", "text": text})
	}
	for _, image := range images {
		url, _ := image.(string)
		url = strings.TrimSpace(url)
		if url == "" {
			continue
		}
		if !strings.HasPrefix(url, "data:") && !strings.HasPrefix(url, "http://") && !strings.HasPrefix(url, "https://") {
			url = "data:image/png;base64," + url
		}
		parts = append(parts, map[string]any{
			"type":      "image_url",
			"image_url": map[string]any{"url": url},
		})
	}
	return parts
}

func ollamaContentString(value any) string {
	switch v := value.(type) {
	case string:
		return v
	default:
		if value == nil {
			return ""
		}
		raw, err := json.Marshal(value)
		if err != nil {
			return ""
		}
		return string(raw)
	}
}

func ollamaArgumentsString(value any) string {
	if s, ok := value.(string); ok {
		return s
	}
	if value == nil {
		return "{}"
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return "{}"
	}
	return string(raw)
}

func ollamaThinkingEffort(value any) string {
	switch v := value.(type) {
	case string:
		switch strings.ToLower(strings.TrimSpace(v)) {
		case "low", "medium", "high", "xhigh", "max", "ultra":
			return strings.ToLower(strings.TrimSpace(v))
		case "true":
			return "medium"
		default:
			return ""
		}
	case bool:
		if v {
			return "medium"
		}
	}
	return ""
}

func ollamaResponseFormat(value any) map[string]any {
	switch v := value.(type) {
	case string:
		if strings.EqualFold(strings.TrimSpace(v), "json") {
			return map[string]any{"type": "json_object"}
		}
	case map[string]any:
		return map[string]any{
			"type": "json_schema",
			"json_schema": map[string]any{
				"name":   "ollama_schema",
				"schema": v,
				"strict": true,
			},
		}
	}
	return nil
}

func (s *relayServer) handleOllamaRuntimeNonStream(c *gin.Context, body []byte, model string) {
	req, opts := buildExecutorRequest(c, body, model, sdktranslator.FormatOpenAI, "", false)
	resp, err := s.runtime.Execute(relayContext(c), []string{"codex"}, req, opts)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	payload := convertOpenAIChatResponseToOllama(resp.Payload, model)
	writeUpstreamHeaders(c.Writer.Header(), resp.Headers)
	c.Data(http.StatusOK, "application/json", payload)
}

func (s *relayServer) handleOllamaRuntimeStream(c *gin.Context, body []byte, model string) {
	req, opts := buildExecutorRequest(c, body, model, sdktranslator.FormatOpenAI, "", true)
	startedAt := time.Now()
	timeouts := s.streamTimeoutsForRequest(c.Request, body, model)
	streamCtx, cancelStream := context.WithCancel(relayContext(c))
	defer cancelStream()
	result, err := s.executeStreamWithOpenTimeout(c, streamCtx, []string{"codex"}, req, opts, model, startedAt, timeouts.open)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	if result == nil || result.Chunks == nil {
		writeAPIError(c, http.StatusBadGateway, "upstream stream is unavailable", "bad_gateway")
		return
	}
	s.forwardOllamaRuntimeStream(c, streamCtx, result, model, timeouts.idle)
}

func (s *relayServer) forwardOllamaRuntimeStream(c *gin.Context, ctx context.Context, result *cliproxyexecutor.StreamResult, model string, idleTimeout time.Duration) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	c.Header("Content-Type", "application/x-ndjson; charset=utf-8")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	writeUpstreamHeaders(c.Writer.Header(), result.Headers)
	c.Status(http.StatusOK)

	state := newOllamaStreamState(model)
	if idleTimeout <= 0 {
		idleTimeout = streamIdleTimeout
	}
	idleTimer := time.NewTimer(idleTimeout)
	defer idleTimer.Stop()
	for {
		select {
		case <-idleTimer.C:
			writeOllamaErrorLine(c.Writer, relayTimeoutError{phase: "stream_idle", timeout: idleTimeout})
			flusher.Flush()
			return
		case <-ctx.Done():
			writeOllamaErrorLine(c.Writer, ctx.Err())
			flusher.Flush()
			return
		case <-c.Request.Context().Done():
			return
		case chunk, ok := <-result.Chunks:
			if !idleTimer.Stop() {
				select {
				case <-idleTimer.C:
				default:
				}
			}
			idleTimer.Reset(idleTimeout)
			if !ok {
				writeOllamaJSONLine(c.Writer, state.finalChunk())
				flusher.Flush()
				return
			}
			if chunk.Err != nil {
				writeOllamaErrorLine(c.Writer, chunk.Err)
				flusher.Flush()
				return
			}
			for _, payload := range openAIStreamPayloadsFromChunk(chunk.Payload) {
				for _, event := range state.applyOpenAIChunk(payload) {
					writeOllamaJSONLine(c.Writer, event)
				}
			}
			flusher.Flush()
		}
	}
}

func (s *relayServer) handleOllamaProviderGatewayChat(c *gin.Context, gateway *providerGatewaySpec, body []byte, model string, stream bool) {
	if gateway == nil {
		writeAPIError(c, http.StatusBadGateway, "provider gateway is not configured", "bad_gateway")
		return
	}
	if normalizeProviderGatewayWireAPI(gateway.WireAPI) != "chat_completions" {
		writeAPIError(c, http.StatusBadRequest, "Ollama bridge requires provider gateway wire API chat_completions", "invalid_request")
		return
	}
	upstreamModel := providerGatewayCanonicalModel(gateway, model)
	if strings.TrimSpace(upstreamModel) == "" {
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("model %s is not available for this provider gateway", model), "model_not_available")
		return
	}
	if providerGatewayRequestHasVisionInput(body) && !providerGatewayModelSupportsVision(gateway, upstreamModel) {
		writeAPIError(c, http.StatusBadRequest, fmt.Sprintf("model %s does not support image input", upstreamModel), "unsupported_image_input")
		return
	}
	upstreamBody := rewriteProviderGatewayBodyModel(body, upstreamModel)
	upstreamURL, err := providerGatewayURL(gateway.BaseURL, "/v1/chat/completions")
	if err != nil {
		writeAPIError(c, http.StatusBadGateway, err.Error(), "bad_gateway")
		return
	}
	req, err := http.NewRequestWithContext(relayContext(c), http.MethodPost, upstreamURL, bytes.NewReader(upstreamBody))
	if err != nil {
		writeAPIError(c, http.StatusBadGateway, err.Error(), "bad_gateway")
		return
	}
	req.Header.Set("Authorization", "Bearer "+gateway.APIKey)
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	if stream {
		req.Header.Set("Accept", "text/event-stream")
	}
	copyProviderGatewayDiagnosticHeaders(req.Header, c.Request.Header)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		writeAPIError(c, http.StatusBadGateway, err.Error(), "bad_gateway")
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		payload, _ := io.ReadAll(resp.Body)
		contentType := resp.Header.Get("Content-Type")
		if contentType == "" {
			contentType = "application/json"
		}
		c.Data(resp.StatusCode, contentType, payload)
		return
	}
	if stream {
		s.forwardOllamaProviderGatewayStream(c, resp.Body, upstreamModel, resp.Header)
		return
	}
	payload, err := io.ReadAll(resp.Body)
	if err != nil {
		writeAPIError(c, http.StatusBadGateway, err.Error(), "bad_gateway")
		return
	}
	writeUpstreamHeaders(c.Writer.Header(), resp.Header)
	c.Data(http.StatusOK, "application/json", convertOpenAIChatResponseToOllama(payload, upstreamModel))
}

func (s *relayServer) forwardOllamaProviderGatewayStream(c *gin.Context, body io.Reader, model string, headers http.Header) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	c.Header("Content-Type", "application/x-ndjson; charset=utf-8")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	writeUpstreamHeaders(c.Writer.Header(), headers)
	c.Status(http.StatusOK)

	state := newOllamaStreamState(model)
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)
	for scanner.Scan() {
		for _, payload := range openAIStreamPayloadsFromChunk(scanner.Bytes()) {
			for _, event := range state.applyOpenAIChunk(payload) {
				writeOllamaJSONLine(c.Writer, event)
			}
		}
		flusher.Flush()
	}
	if err := scanner.Err(); err != nil {
		writeOllamaErrorLine(c.Writer, err)
		flusher.Flush()
		return
	}
	writeOllamaJSONLine(c.Writer, state.finalChunk())
	flusher.Flush()
}

type ollamaToolCallAccumulator struct {
	ID        string
	Name      string
	Arguments string
}

type ollamaStreamState struct {
	model            string
	content          string
	thinking         string
	promptTokens     int64
	completionTokens int64
	doneReason       string
	toolCalls        map[int]*ollamaToolCallAccumulator
}

func newOllamaStreamState(model string) *ollamaStreamState {
	return &ollamaStreamState{
		model:      model,
		doneReason: "stop",
		toolCalls:  map[int]*ollamaToolCallAccumulator{},
	}
}

func (s *ollamaStreamState) applyOpenAIChunk(payload []byte) []gin.H {
	payload = bytes.TrimSpace(payload)
	if len(payload) == 0 || bytes.Equal(payload, []byte("[DONE]")) {
		return nil
	}
	var root map[string]any
	if err := json.Unmarshal(payload, &root); err != nil {
		return nil
	}
	if usage, ok := root["usage"].(map[string]any); ok {
		if value, ok := numericInt64(usage["prompt_tokens"]); ok {
			s.promptTokens = value
		}
		if value, ok := numericInt64(usage["completion_tokens"]); ok {
			s.completionTokens = value
		}
	}
	choices, _ := root["choices"].([]any)
	if len(choices) == 0 {
		return nil
	}
	choice, _ := choices[0].(map[string]any)
	if reason, _ := choice["finish_reason"].(string); reason != "" {
		s.doneReason = mapOpenAIFinishReasonToOllama(reason)
	}
	delta, _ := choice["delta"].(map[string]any)
	events := []gin.H{}
	if thinking, _ := delta["reasoning_content"].(string); thinking != "" {
		s.thinking += thinking
		events = append(events, gin.H{
			"model":      s.model,
			"created_at": time.Now().Format(time.RFC3339Nano),
			"message":    gin.H{"role": "assistant", "content": "", "thinking": thinking},
			"done":       false,
		})
	}
	if content, _ := delta["content"].(string); content != "" {
		s.content += content
		events = append(events, gin.H{
			"model":      s.model,
			"created_at": time.Now().Format(time.RFC3339Nano),
			"message":    gin.H{"role": "assistant", "content": content},
			"done":       false,
		})
	}
	if toolCalls, ok := delta["tool_calls"].([]any); ok {
		s.applyToolCallDeltas(toolCalls)
	}
	return events
}

func (s *ollamaStreamState) applyToolCallDeltas(toolCalls []any) {
	for _, raw := range toolCalls {
		item, _ := raw.(map[string]any)
		index := 0
		if value, ok := numericInt64(item["index"]); ok {
			index = int(value)
		}
		acc := s.toolCalls[index]
		if acc == nil {
			acc = &ollamaToolCallAccumulator{ID: fmt.Sprintf("tool_%d", index), Name: "tool"}
			s.toolCalls[index] = acc
		}
		if id, _ := item["id"].(string); id != "" {
			acc.ID = id
		}
		fn, _ := item["function"].(map[string]any)
		if name, _ := fn["name"].(string); name != "" {
			acc.Name = name
		}
		if arguments, _ := fn["arguments"].(string); arguments != "" {
			acc.Arguments += arguments
		}
	}
}

func (s *ollamaStreamState) finalChunk() gin.H {
	message := gin.H{
		"role":    "assistant",
		"content": s.content,
	}
	if s.thinking != "" {
		message["thinking"] = s.thinking
	}
	if toolCalls := s.ollamaToolCalls(); len(toolCalls) > 0 {
		message["tool_calls"] = toolCalls
	}
	return gin.H{
		"model":                s.model,
		"created_at":           time.Now().Format(time.RFC3339Nano),
		"message":              message,
		"done":                 true,
		"done_reason":          s.doneReason,
		"total_duration":       0,
		"load_duration":        0,
		"prompt_eval_count":    s.promptTokens,
		"prompt_eval_duration": 0,
		"eval_count":           s.completionTokens,
		"eval_duration":        0,
	}
}

func (s *ollamaStreamState) ollamaToolCalls() []gin.H {
	if len(s.toolCalls) == 0 {
		return nil
	}
	indexes := make([]int, 0, len(s.toolCalls))
	for index := range s.toolCalls {
		indexes = append(indexes, index)
	}
	sort.Ints(indexes)
	out := make([]gin.H, 0, len(indexes))
	for _, index := range indexes {
		acc := s.toolCalls[index]
		if acc == nil {
			continue
		}
		out = append(out, gin.H{
			"id":   acc.ID,
			"type": "function",
			"function": gin.H{
				"name":      acc.Name,
				"arguments": parseOllamaToolArguments(acc.Arguments),
			},
		})
	}
	return out
}

func convertOpenAIChatResponseToOllama(payload []byte, fallbackModel string) []byte {
	var root map[string]any
	if err := json.Unmarshal(payload, &root); err != nil {
		return payload
	}
	model, _ := root["model"].(string)
	if strings.TrimSpace(model) == "" {
		model = fallbackModel
	}
	createdSeconds := time.Now().Unix()
	if value, ok := numericInt64(root["created"]); ok && value > 0 {
		createdSeconds = value
	}
	choice := firstOpenAIChoice(root)
	message, _ := choice["message"].(map[string]any)
	usage, _ := root["usage"].(map[string]any)
	promptTokens, _ := numericInt64(usage["prompt_tokens"])
	completionTokens, _ := numericInt64(usage["completion_tokens"])
	outMessage := gin.H{
		"role":    "assistant",
		"content": stringFieldFromAny(message["content"]),
	}
	if thinking := stringFieldFromAny(message["reasoning_content"]); thinking != "" {
		outMessage["thinking"] = thinking
	}
	if toolCalls, ok := message["tool_calls"].([]any); ok && len(toolCalls) > 0 {
		outMessage["tool_calls"] = openAIToolCallsToOllama(toolCalls)
	}
	out := gin.H{
		"model":                model,
		"created_at":           time.Unix(createdSeconds, 0).Format(time.RFC3339Nano),
		"message":              outMessage,
		"done":                 true,
		"done_reason":          mapOpenAIFinishReasonToOllama(stringFieldFromAny(choice["finish_reason"])),
		"total_duration":       0,
		"load_duration":        0,
		"prompt_eval_count":    promptTokens,
		"prompt_eval_duration": 0,
		"eval_count":           completionTokens,
		"eval_duration":        0,
	}
	raw, err := json.Marshal(out)
	if err != nil {
		return payload
	}
	return raw
}

func firstOpenAIChoice(root map[string]any) map[string]any {
	choices, _ := root["choices"].([]any)
	if len(choices) == 0 {
		return map[string]any{}
	}
	choice, _ := choices[0].(map[string]any)
	if choice == nil {
		return map[string]any{}
	}
	return choice
}

func openAIToolCallsToOllama(toolCalls []any) []gin.H {
	out := make([]gin.H, 0, len(toolCalls))
	for index, raw := range toolCalls {
		item, _ := raw.(map[string]any)
		fn, _ := item["function"].(map[string]any)
		id := stringFieldFromAny(item["id"])
		if id == "" {
			id = fmt.Sprintf("tool_%d", index)
		}
		name := stringFieldFromAny(fn["name"])
		if name == "" {
			name = "tool"
		}
		out = append(out, gin.H{
			"id":   id,
			"type": "function",
			"function": gin.H{
				"name":      name,
				"arguments": parseOllamaToolArguments(stringFieldFromAny(fn["arguments"])),
			},
		})
	}
	return out
}

func parseOllamaToolArguments(raw string) any {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return gin.H{}
	}
	var parsed any
	if err := json.Unmarshal([]byte(raw), &parsed); err == nil {
		return parsed
	}
	return raw
}

func openAIStreamPayloadsFromChunk(chunk []byte) [][]byte {
	trimmed := bytes.TrimSpace(chunk)
	if len(trimmed) == 0 {
		return nil
	}
	var payloads [][]byte
	for _, line := range bytes.Split(trimmed, []byte("\n")) {
		line = bytes.TrimSpace(line)
		if len(line) == 0 {
			continue
		}
		if bytes.HasPrefix(line, []byte("data:")) {
			payload := bytes.TrimSpace(line[len("data:"):])
			if len(payload) > 0 {
				payloads = append(payloads, append([]byte(nil), payload...))
			}
			continue
		}
		if bytes.HasPrefix(line, []byte("event:")) || bytes.HasPrefix(line, []byte(":")) {
			continue
		}
		payloads = append(payloads, append([]byte(nil), line...))
	}
	return payloads
}

func writeOllamaJSONLine(w io.Writer, value any) {
	if w == nil {
		return
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return
	}
	_, _ = w.Write(raw)
	_, _ = w.Write([]byte("\n"))
}

func writeOllamaErrorLine(w io.Writer, err error) {
	writeOllamaJSONLine(w, gin.H{"error": errorMessage(err)})
}

func mapOpenAIFinishReasonToOllama(reason string) string {
	switch strings.TrimSpace(reason) {
	case "length":
		return "length"
	case "tool_calls", "function_call":
		return "tool_calls"
	default:
		return "stop"
	}
}

func numericInt64(value any) (int64, bool) {
	switch v := value.(type) {
	case int:
		return int64(v), true
	case int64:
		return v, true
	case float64:
		return int64(v), true
	case json.Number:
		n, err := v.Int64()
		return n, err == nil
	default:
		return 0, false
	}
}

func stringFieldFromAny(value any) string {
	if value == nil {
		return ""
	}
	if s, ok := value.(string); ok {
		return s
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return ""
	}
	return string(raw)
}

type streamTimeoutProfile struct {
	open time.Duration
	idle time.Duration
}
