package main

import (
	"bytes"
	"context"

	"encoding/base64"
	"encoding/json"

	"fmt"
	"io"

	"mime/multipart"

	"net/http"

	"strconv"
	"strings"

	"time"

	"github.com/gin-gonic/gin"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"

	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

type imageRelayRequest struct {
	body           []byte
	stream         bool
	responseFormat string
	streamPrefix   string
	requestedModel string
}

type imageRelayResult struct {
	Result        string
	RevisedPrompt string
	OutputFormat  string
	Size          string
	Background    string
	Quality       string
}

type imageSSEAccumulator struct {
	pending []byte
}

func (a *imageSSEAccumulator) AddChunk(chunk []byte) [][]byte {
	if len(chunk) == 0 {
		return nil
	}
	if responsesSSENeedsLineBreak(a.pending, chunk) {
		a.pending = append(a.pending, '\n')
	}
	a.pending = append(a.pending, chunk...)

	var frames [][]byte
	for {
		frameLen := responsesSSEFrameLen(a.pending)
		if frameLen == 0 {
			break
		}
		frames = append(frames, a.pending[:frameLen])
		copy(a.pending, a.pending[frameLen:])
		a.pending = a.pending[:len(a.pending)-frameLen]
	}
	if len(bytes.TrimSpace(a.pending)) == 0 {
		a.pending = a.pending[:0]
		return frames
	}
	if responsesSSECanEmitWithoutDelimiter(a.pending) {
		frames = append(frames, a.pending)
		a.pending = a.pending[:0]
	}
	return frames
}

func (a *imageSSEAccumulator) Flush() [][]byte {
	if len(a.pending) == 0 {
		return nil
	}
	var frames [][]byte
	for {
		frameLen := responsesSSEFrameLen(a.pending)
		if frameLen == 0 {
			break
		}
		frames = append(frames, a.pending[:frameLen])
		copy(a.pending, a.pending[frameLen:])
		a.pending = a.pending[:len(a.pending)-frameLen]
	}
	if len(bytes.TrimSpace(a.pending)) > 0 && responsesSSECanEmitWithoutDelimiter(a.pending) {
		frames = append(frames, a.pending)
	}
	a.pending = nil
	return frames
}

func buildImageGenerationRelayRequest(rawJSON []byte) (imageRelayRequest, error) {
	if !json.Valid(rawJSON) {
		return imageRelayRequest{}, fmt.Errorf("body must be valid JSON")
	}
	var payload map[string]any
	if err := json.Unmarshal(rawJSON, &payload); err != nil {
		return imageRelayRequest{}, err
	}
	prompt := strings.TrimSpace(stringField(payload, "prompt"))
	if prompt == "" {
		return imageRelayRequest{}, fmt.Errorf("prompt is required")
	}
	tool, err := buildImageTool(payload, "generate")
	if err != nil {
		return imageRelayRequest{}, err
	}
	body, err := json.Marshal(buildImagesResponsesPayload(prompt, nil, tool))
	if err != nil {
		return imageRelayRequest{}, err
	}
	return imageRelayRequest{
		body:           body,
		stream:         boolField(payload, "stream"),
		responseFormat: normalizeImageResponseFormat(stringField(payload, "response_format")),
		streamPrefix:   "image_generation",
		requestedModel: imageModelOrDefault(payload),
	}, nil
}

func buildImageEditRelayRequest(c *gin.Context) (imageRelayRequest, error) {
	contentType := strings.ToLower(strings.TrimSpace(c.GetHeader("Content-Type")))
	if strings.HasPrefix(contentType, "multipart/form-data") || contentType == "" {
		return buildImageEditRelayRequestFromMultipart(c)
	}
	if !strings.HasPrefix(contentType, "application/json") {
		return imageRelayRequest{}, fmt.Errorf("unsupported Content-Type %q", contentType)
	}
	rawJSON, err := c.GetRawData()
	if err != nil {
		return imageRelayRequest{}, err
	}
	if !json.Valid(rawJSON) {
		return imageRelayRequest{}, fmt.Errorf("body must be valid JSON")
	}
	var payload map[string]any
	if err := json.Unmarshal(rawJSON, &payload); err != nil {
		return imageRelayRequest{}, err
	}
	prompt := strings.TrimSpace(stringField(payload, "prompt"))
	if prompt == "" {
		return imageRelayRequest{}, fmt.Errorf("prompt is required")
	}
	images := jsonImageURLs(payload)
	if len(images) == 0 {
		return imageRelayRequest{}, fmt.Errorf("images[].image_url is required")
	}
	tool, err := buildImageTool(payload, "edit")
	if err != nil {
		return imageRelayRequest{}, err
	}
	if mask, ok := payload["mask"].(map[string]any); ok {
		if url := strings.TrimSpace(stringField(mask, "image_url")); url != "" {
			tool["input_image_mask"] = map[string]any{"image_url": url}
		}
	}
	body, err := json.Marshal(buildImagesResponsesPayload(prompt, images, tool))
	if err != nil {
		return imageRelayRequest{}, err
	}
	return imageRelayRequest{
		body:           body,
		stream:         boolField(payload, "stream"),
		responseFormat: normalizeImageResponseFormat(stringField(payload, "response_format")),
		streamPrefix:   "image_edit",
		requestedModel: imageModelOrDefault(payload),
	}, nil
}

func buildImageEditRelayRequestFromMultipart(c *gin.Context) (imageRelayRequest, error) {
	form, err := c.MultipartForm()
	if err != nil {
		return imageRelayRequest{}, err
	}
	payload := map[string]any{
		"model":              strings.TrimSpace(c.PostForm("model")),
		"size":               strings.TrimSpace(c.PostForm("size")),
		"quality":            strings.TrimSpace(c.PostForm("quality")),
		"background":         strings.TrimSpace(c.PostForm("background")),
		"output_format":      strings.TrimSpace(c.PostForm("output_format")),
		"input_fidelity":     strings.TrimSpace(c.PostForm("input_fidelity")),
		"moderation":         strings.TrimSpace(c.PostForm("moderation")),
		"response_format":    strings.TrimSpace(c.PostForm("response_format")),
		"stream":             parseBoolString(c.PostForm("stream")),
		"output_compression": parseIntString(c.PostForm("output_compression")),
		"partial_images":     parseIntString(c.PostForm("partial_images")),
	}
	prompt := strings.TrimSpace(c.PostForm("prompt"))
	if prompt == "" {
		return imageRelayRequest{}, fmt.Errorf("prompt is required")
	}
	imageFiles := form.File["image[]"]
	if len(imageFiles) == 0 {
		imageFiles = form.File["image"]
	}
	if len(imageFiles) == 0 {
		return imageRelayRequest{}, fmt.Errorf("image is required")
	}
	images := make([]string, 0, len(imageFiles))
	for _, fh := range imageFiles {
		dataURL, err := multipartFileToDataURL(fh)
		if err != nil {
			return imageRelayRequest{}, err
		}
		images = append(images, dataURL)
	}
	tool, err := buildImageTool(payload, "edit")
	if err != nil {
		return imageRelayRequest{}, err
	}
	if masks := form.File["mask"]; len(masks) > 0 && masks[0] != nil {
		dataURL, err := multipartFileToDataURL(masks[0])
		if err != nil {
			return imageRelayRequest{}, err
		}
		tool["input_image_mask"] = map[string]any{"image_url": dataURL}
	}
	body, err := json.Marshal(buildImagesResponsesPayload(prompt, images, tool))
	if err != nil {
		return imageRelayRequest{}, err
	}
	return imageRelayRequest{
		body:           body,
		stream:         boolField(payload, "stream"),
		responseFormat: normalizeImageResponseFormat(stringField(payload, "response_format")),
		streamPrefix:   "image_edit",
		requestedModel: imageModelOrDefault(payload),
	}, nil
}

func (s *relayServer) handleImagesRelayRequest(c *gin.Context, imageReq imageRelayRequest) {
	spec, _ := c.Request.Context().Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestedModel := strings.TrimSpace(imageReq.requestedModel)
	if requestedModel == "" {
		requestedModel = defaultImagesToolModel
	}
	if !validateClientModelVisible(s.manifest, spec, requestedModel, defaultImagesToolModel) {
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("模型 %s 不在当前 API Key 的可用模型范围内", requestedModel), "model_not_available")
		return
	}
	model := defaultImagesMainModel
	req, opts := buildExecutorRequest(c, imageReq.body, model, sdktranslator.FormatOpenAIResponse, "", true)
	startedAt := time.Now()
	timeouts := s.streamTimeoutsForRequest(c.Request, imageReq.body, defaultImagesToolModel)
	immediateSSE := imageReq.stream && s.manifest != nil && s.manifest.ImmediateSSEResponse
	var immediateFlusher http.Flusher
	if immediateSSE {
		flusher, ok := c.Writer.(http.Flusher)
		if !ok {
			writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
			return
		}
		setEventStreamHeaders(c.Writer.Header())
		c.Status(http.StatusOK)
		_, _ = c.Writer.Write([]byte(": accepted\n\n"))
		flusher.Flush()
		immediateFlusher = flusher
	}
	streamCtx, cancelStream := context.WithCancel(relayContext(c))
	defer cancelStream()
	result, err := s.executeStreamWithOpenTimeout(c, streamCtx, []string{"codex"}, req, opts, model, startedAt, timeouts.open)
	if err != nil {
		if immediateSSE {
			writeImagesStreamError(c, immediateFlusher, err)
			return
		}
		s.writeExecutorError(c, err)
		return
	}
	if result == nil || result.Chunks == nil {
		if immediateSSE {
			writeImagesStreamError(c, immediateFlusher, relayStatusError{status: http.StatusBadGateway, message: "upstream stream is unavailable"})
			return
		}
		writeAPIError(c, http.StatusBadGateway, "upstream stream is unavailable", "bad_gateway")
		return
	}
	if imageReq.stream {
		s.forwardImagesStream(c, streamCtx, result, imageReq, timeouts.idle, immediateSSE)
		return
	}
	out, err := collectImagesResponse(streamCtx, result.Chunks, imageReq.responseFormat, timeouts.idle)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	writeUpstreamHeaders(c.Writer.Header(), result.Headers)
	c.Data(http.StatusOK, "application/json", out)
}

func (s *relayServer) forwardImagesStream(c *gin.Context, ctx context.Context, result *cliproxyexecutor.StreamResult, imageReq imageRelayRequest, idleTimeout time.Duration, headersCommitted bool) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	if !headersCommitted {
		setEventStreamHeaders(c.Writer.Header())
		writeUpstreamHeaders(c.Writer.Header(), result.Headers)
		c.Status(http.StatusOK)
	}

	writeEvent := func(eventName string, payload []byte) {
		if strings.TrimSpace(eventName) != "" {
			_, _ = fmt.Fprintf(c.Writer, "event: %s\n", eventName)
		}
		_, _ = fmt.Fprintf(c.Writer, "data: %s\n\n", string(payload))
		flusher.Flush()
	}
	writeErr := func(err error) {
		writeImagesStreamError(c, flusher, err)
	}

	acc := &imageSSEAccumulator{}
	if idleTimeout <= 0 {
		idleTimeout = imageStreamIdleTimeout
	}
	idleTimer := time.NewTimer(idleTimeout)
	defer idleTimer.Stop()
	for {
		select {
		case <-idleTimer.C:
			writeErr(relayTimeoutError{phase: "stream_idle", timeout: idleTimeout})
			return
		case <-ctx.Done():
			writeErr(ctx.Err())
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
				for _, frame := range acc.Flush() {
					if done := forwardImageResponseFrame(frame, imageReq, writeEvent, writeErr); done {
						return
					}
				}
				return
			}
			if chunk.Err != nil {
				writeErr(chunk.Err)
				return
			}
			for _, frame := range acc.AddChunk(chunk.Payload) {
				if done := forwardImageResponseFrame(frame, imageReq, writeEvent, writeErr); done {
					return
				}
			}
		}
	}
}

func writeImagesStreamError(c *gin.Context, flusher http.Flusher, err error) {
	if c == nil || flusher == nil {
		return
	}
	payload, _ := json.Marshal(map[string]any{
		"error": map[string]any{
			"message": errorMessage(err),
			"type":    "upstream_error",
			"code":    statusCodeFromError(err),
		},
	})
	_, _ = fmt.Fprintf(c.Writer, "event: error\ndata: %s\n\n", string(payload))
	flusher.Flush()
}

func forwardImageResponseFrame(frame []byte, imageReq imageRelayRequest, writeEvent func(string, []byte), writeErr func(error)) bool {
	for _, payload := range imageFramePayloads(frame) {
		var event map[string]any
		if err := json.Unmarshal(payload, &event); err != nil {
			continue
		}
		switch stringField(event, "type") {
		case "response.image_generation_call.partial_image":
			b64 := stringField(event, "partial_image_b64")
			if b64 == "" {
				continue
			}
			index, _ := numericField(event["partial_image_index"])
			eventName := imageReq.streamPrefix + ".partial_image"
			out := map[string]any{
				"type":                eventName,
				"partial_image_index": index,
			}
			if normalizeImageResponseFormat(imageReq.responseFormat) == "url" {
				out["url"] = "data:" + mimeTypeFromOutputFormat(stringField(event, "output_format")) + ";base64," + b64
			} else {
				out["b64_json"] = b64
			}
			data, _ := json.Marshal(out)
			writeEvent(eventName, data)
		case "response.completed":
			results, usage, _ := extractImageResults(event)
			if len(results) == 0 {
				writeErr(relayStatusError{status: http.StatusBadGateway, message: "upstream did not return image output"})
				return true
			}
			eventName := imageReq.streamPrefix + ".completed"
			for _, img := range results {
				out := map[string]any{"type": eventName}
				if normalizeImageResponseFormat(imageReq.responseFormat) == "url" {
					out["url"] = "data:" + mimeTypeFromOutputFormat(img.OutputFormat) + ";base64," + img.Result
				} else {
					out["b64_json"] = img.Result
				}
				if usage != nil {
					out["usage"] = usage
				}
				data, _ := json.Marshal(out)
				writeEvent(eventName, data)
			}
			return true
		}
	}
	return false
}

func stringField(payload map[string]any, key string) string {
	value, _ := payload[key].(string)
	return strings.TrimSpace(value)
}

func boolField(payload map[string]any, key string) bool {
	value, _ := payload[key].(bool)
	return value
}

func parseBoolString(raw string) bool {
	switch strings.ToLower(strings.TrimSpace(raw)) {
	case "1", "true", "yes", "on":
		return true
	default:
		return false
	}
}

func parseIntString(raw string) any {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil
	}
	value, err := strconv.ParseInt(raw, 10, 64)
	if err != nil {
		return nil
	}
	return value
}

func normalizeImageResponseFormat(value string) string {
	if strings.EqualFold(strings.TrimSpace(value), "url") {
		return "url"
	}
	return "b64_json"
}

func imageModelOrDefault(payload map[string]any) string {
	if model := strings.TrimSpace(stringField(payload, "model")); model != "" {
		return model
	}
	return defaultImagesToolModel
}

func buildImageTool(payload map[string]any, action string) (map[string]any, error) {
	model := imageModelOrDefault(payload)
	if modelBase(model) != defaultImagesToolModel {
		return nil, fmt.Errorf("model %s is not supported on %s or %s. Use %s.", model, imagesGenerationsPath, imagesEditsPath, defaultImagesToolModel)
	}
	tool := map[string]any{
		"type":   "image_generation",
		"action": action,
		"model":  defaultImagesToolModel,
	}
	for _, key := range []string{"size", "quality", "background", "output_format", "moderation"} {
		if value := stringField(payload, key); value != "" {
			tool[key] = value
		}
	}
	if action == "edit" {
		if value := stringField(payload, "input_fidelity"); value != "" {
			tool["input_fidelity"] = value
		}
	}
	for _, key := range []string{"output_compression", "partial_images"} {
		if value, ok := numericField(payload[key]); ok {
			tool[key] = value
		}
	}
	return tool, nil
}

func numericField(value any) (int64, bool) {
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

func jsonImageURLs(payload map[string]any) []string {
	var out []string
	if image := stringField(payload, "image"); image != "" {
		out = append(out, image)
	}
	if items, ok := payload["images"].([]any); ok {
		for _, item := range items {
			switch v := item.(type) {
			case string:
				if trimmed := strings.TrimSpace(v); trimmed != "" {
					out = append(out, trimmed)
				}
			case map[string]any:
				if url := stringField(v, "image_url"); url != "" {
					out = append(out, url)
				}
			}
		}
	}
	return out
}

func buildImagesResponsesPayload(prompt string, images []string, tool map[string]any) map[string]any {
	content := []any{map[string]any{
		"type": "input_text",
		"text": prompt,
	}}
	for _, image := range images {
		if image = strings.TrimSpace(image); image != "" {
			content = append(content, map[string]any{
				"type":      "input_image",
				"image_url": image,
			})
		}
	}
	return map[string]any{
		"instructions":        "",
		"stream":              true,
		"reasoning":           map[string]any{"effort": "medium", "summary": "auto"},
		"parallel_tool_calls": true,
		"include":             []string{"reasoning.encrypted_content"},
		"model":               defaultImagesMainModel,
		"store":               false,
		"tool_choice":         map[string]any{"type": "image_generation"},
		"input": []any{map[string]any{
			"type":    "message",
			"role":    "user",
			"content": content,
		}},
		"tools": []any{tool},
	}
}

func multipartFileToDataURL(fileHeader *multipart.FileHeader) (string, error) {
	if fileHeader == nil {
		return "", fmt.Errorf("upload file is nil")
	}
	if fileHeader.Size > maxImageUploadBytes {
		return "", fmt.Errorf("upload file exceeds %d bytes", maxImageUploadBytes)
	}
	file, err := fileHeader.Open()
	if err != nil {
		return "", err
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, maxImageUploadBytes+1))
	if err != nil {
		return "", err
	}
	if int64(len(data)) > maxImageUploadBytes {
		return "", fmt.Errorf("upload file exceeds %d bytes", maxImageUploadBytes)
	}
	mediaType := strings.TrimSpace(fileHeader.Header.Get("Content-Type"))
	if mediaType == "" {
		mediaType = http.DetectContentType(data)
	}
	return "data:" + mediaType + ";base64," + base64.StdEncoding.EncodeToString(data), nil
}

func collectImagesResponse(ctx context.Context, chunks <-chan cliproxyexecutor.StreamChunk, responseFormat string, idleTimeout time.Duration) ([]byte, error) {
	acc := &imageSSEAccumulator{}
	if idleTimeout <= 0 {
		idleTimeout = imageStreamIdleTimeout
	}
	idleTimer := time.NewTimer(idleTimeout)
	defer idleTimer.Stop()
	for {
		select {
		case <-idleTimer.C:
			return nil, relayTimeoutError{phase: "stream_idle", timeout: idleTimeout}
		case <-ctx.Done():
			return nil, ctx.Err()
		case chunk, ok := <-chunks:
			if !idleTimer.Stop() {
				select {
				case <-idleTimer.C:
				default:
				}
			}
			idleTimer.Reset(idleTimeout)
			if !ok {
				for _, frame := range acc.Flush() {
					if out, done, err := processImageResponseFrame(frame, responseFormat); err != nil {
						return nil, err
					} else if done {
						return out, nil
					}
				}
				return nil, relayStatusError{status: http.StatusBadGateway, message: "stream disconnected before completion"}
			}
			if chunk.Err != nil {
				return nil, chunk.Err
			}
			for _, frame := range acc.AddChunk(chunk.Payload) {
				if out, done, err := processImageResponseFrame(frame, responseFormat); err != nil {
					return nil, err
				} else if done {
					return out, nil
				}
			}
		}
	}
}

func processImageResponseFrame(frame []byte, responseFormat string) ([]byte, bool, error) {
	for _, payload := range imageFramePayloads(frame) {
		var event map[string]any
		if err := json.Unmarshal(payload, &event); err != nil {
			return nil, false, relayStatusError{status: http.StatusBadGateway, message: "invalid SSE data JSON"}
		}
		if stringField(event, "type") != "response.completed" {
			continue
		}
		results, usage, createdAt := extractImageResults(event)
		if len(results) == 0 {
			return nil, false, relayStatusError{status: http.StatusBadGateway, message: "upstream did not return image output"}
		}
		out, err := buildImagesAPIResponse(results, usage, createdAt, responseFormat)
		return out, true, err
	}
	return nil, false, nil
}

func imageFramePayloads(frame []byte) [][]byte {
	var payloads [][]byte
	for _, line := range bytes.Split(frame, []byte("\n")) {
		trimmed := bytes.TrimSpace(bytes.TrimRight(line, "\r"))
		if !bytes.HasPrefix(trimmed, []byte("data:")) {
			continue
		}
		payload := bytes.TrimSpace(trimmed[len("data:"):])
		if len(payload) == 0 || bytes.Equal(payload, []byte("[DONE]")) {
			continue
		}
		payloads = append(payloads, payload)
	}
	return payloads
}

func extractImageResults(event map[string]any) ([]imageRelayResult, any, int64) {
	createdAt := time.Now().Unix()
	if response, ok := event["response"].(map[string]any); ok {
		if created, ok := numericField(response["created_at"]); ok && created > 0 {
			createdAt = created
		}
		var usage any
		if toolUsage, ok := response["tool_usage"].(map[string]any); ok {
			usage = toolUsage["image_gen"]
		}
		var results []imageRelayResult
		if output, ok := response["output"].([]any); ok {
			for _, item := range output {
				obj, ok := item.(map[string]any)
				if !ok || stringField(obj, "type") != "image_generation_call" {
					continue
				}
				result := stringField(obj, "result")
				if result == "" {
					continue
				}
				results = append(results, imageRelayResult{
					Result:        result,
					RevisedPrompt: stringField(obj, "revised_prompt"),
					OutputFormat:  stringField(obj, "output_format"),
					Size:          stringField(obj, "size"),
					Background:    stringField(obj, "background"),
					Quality:       stringField(obj, "quality"),
				})
			}
		}
		return results, usage, createdAt
	}
	return nil, nil, createdAt
}

func buildImagesAPIResponse(results []imageRelayResult, usage any, createdAt int64, responseFormat string) ([]byte, error) {
	responseFormat = normalizeImageResponseFormat(responseFormat)
	data := make([]any, 0, len(results))
	for _, img := range results {
		item := map[string]any{}
		if responseFormat == "url" {
			item["url"] = "data:" + mimeTypeFromOutputFormat(img.OutputFormat) + ";base64," + img.Result
		} else {
			item["b64_json"] = img.Result
		}
		if img.RevisedPrompt != "" {
			item["revised_prompt"] = img.RevisedPrompt
		}
		data = append(data, item)
	}
	out := map[string]any{
		"created": createdAt,
		"data":    data,
	}
	if len(results) > 0 {
		first := results[0]
		if first.Background != "" {
			out["background"] = first.Background
		}
		if first.OutputFormat != "" {
			out["output_format"] = first.OutputFormat
		}
		if first.Quality != "" {
			out["quality"] = first.Quality
		}
		if first.Size != "" {
			out["size"] = first.Size
		}
	}
	if usage != nil {
		out["usage"] = usage
	}
	return json.Marshal(out)
}

func mimeTypeFromOutputFormat(outputFormat string) string {
	switch strings.ToLower(strings.TrimSpace(outputFormat)) {
	case "":
		return "image/png"
	case "png":
		return "image/png"
	case "jpg", "jpeg":
		return "image/jpeg"
	case "webp":
		return "image/webp"
	default:
		if strings.Contains(outputFormat, "/") {
			return outputFormat
		}
		return "image/png"
	}
}
