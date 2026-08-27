package executor

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/auth/codebuddy"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	log "github.com/sirupsen/logrus"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// inspectImageToolName is the tool injected into the text-only model so it can
// autonomously query the vision model for image details during reasoning.
const inspectImageToolName = "inspect_image"

// codebuddyAgenticImageRef holds an extracted image's original content part,
// kept in memory so inspect_image can re-send it to the vision model on demand.
type codebuddyAgenticImageRef struct {
	id       int
	partJSON []byte // {"type":"image_url","image_url":{...}} raw part
}

// codebuddyVisionAgenticEnabled reports whether the agentic vision-proxy mode
// is active.
func (e *CodebuddyExecutor) codebuddyVisionAgenticEnabled() bool {
	return e.cfg.CodebuddyVision.NormalizedVisionMode() == config.CodebuddyVisionModeAgentic
}

// inspectImageToolDef returns the OpenAI function-tool definition injected into
// the request so the text-only model can inspect attached images.
func inspectImageToolDef() map[string]any {
	return map[string]any{
		"type": "function",
		"function": map[string]any{
			"name":        inspectImageToolName,
			"description": "查看已附加图片的细节。当你需要确认图片中的具体内容（文字、数字、位置、颜色、图表数据等）时调用。可多次调用以查看不同细节。",
			"parameters": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"image_id": map[string]any{
						"type":        "integer",
						"description": "图片编号，从 1 开始。",
					},
					"question": map[string]any{
						"type":        "string",
						"description": "针对该图片的具体问题，例如“图片右上角的文字是什么”。",
					},
				},
				"required": []string{"image_id", "question"},
			},
		},
	}
}

// extractCodebuddyImagesForAgentic extracts every image content part from the
// messages, replaces each with a text hint, and returns the rewritten body plus
// the extracted image references (1-based ids).
func extractCodebuddyImagesForAgentic(body []byte) ([]byte, []codebuddyAgenticImageRef, error) {
	messages := gjson.GetBytes(body, "messages")
	if !messages.IsArray() {
		return body, nil, nil
	}

	out := body
	var images []codebuddyAgenticImageRef
	for mi, msg := range messages.Array() {
		content := msg.Get("content")
		if !content.IsArray() {
			continue
		}
		for ci, part := range content.Array() {
			if !isCodebuddyImagePartType(part.Get("type").String()) {
				continue
			}
			id := len(images) + 1
			images = append(images, codebuddyAgenticImageRef{
				id:       id,
				partJSON: append([]byte(nil), []byte(part.Raw)...),
			})

			text := fmt.Sprintf("[图片 #%d 已附加，可用 inspect_image 工具查看，image_id=%d]", id, id)
			replacement, err := json.Marshal(map[string]string{"type": "text", "text": text})
			if err != nil {
				return body, nil, err
			}
			path := fmt.Sprintf("messages.%d.content.%d", mi, ci)
			out, err = sjson.SetRawBytes(out, path, replacement)
			if err != nil {
				return body, nil, err
			}
		}
	}
	return out, images, nil
}

// injectCodebuddyInspectTool prepends a system guidance message and injects the
// inspect_image tool definition into the request body.
func injectCodebuddyInspectTool(body []byte, imageCount int) []byte {
	guide := fmt.Sprintf(
		"用户消息中附带了 %d 张图片，但图片内容已从消息中移除，你无法直接看到。你需要使用 inspect_image 工具查看图片细节。当回答涉及图片具体内容（文字、数字、位置、颜色、图表数据等）时，请先调用 inspect_image 工具获取细节，再作答。你可以多次调用该工具查看不同细节。",
		imageCount,
	)
	out, err := prependCodebuddySystemMessage(body, guide)
	if err != nil {
		out = body
	}

	toolsJSON, err := json.Marshal([]any{inspectImageToolDef()})
	if err != nil {
		return out
	}
	out, err = sjson.SetRawBytes(out, "tools", toolsJSON)
	if err != nil {
		return body
	}
	return out
}

// appendAgenticMessage appends a raw JSON message to the messages array.
func appendAgenticMessage(body []byte, msgRaw []byte) ([]byte, error) {
	return sjson.SetRawBytes(body, "messages.-1", msgRaw)
}

// doCodebuddyChatRequest performs a single forced-stream chat completion and
// aggregates it into a non-streaming ChatCompletion JSON. It returns the
// aggregated payload and the upstream response headers.
func (e *CodebuddyExecutor) doCodebuddyChatRequest(
	ctx context.Context,
	auth *cliproxyauth.Auth,
	creds codebuddy.Creds,
	baseURL string,
	body []byte,
) ([]byte, http.Header, error) {
	var err error
	body, err = sjson.SetBytes(body, "stream", true)
	if err != nil {
		return nil, nil, err
	}
	body, err = sjson.SetBytes(body, "stream_options.include_usage", true)
	if err != nil {
		return nil, nil, err
	}

	url := baseURL + codebuddy.ChatPath
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return nil, nil, err
	}
	applyCodebuddyHeaders(httpReq, creds)
	httpReq.Header.Set("Accept", "text/event-stream")

	httpClient := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	httpResp, err := httpClient.Do(httpReq)
	if err != nil {
		return nil, nil, err
	}
	defer func() { _ = httpResp.Body.Close() }()
	if httpResp.StatusCode < 200 || httpResp.StatusCode >= 300 {
		b, _ := io.ReadAll(httpResp.Body)
		return nil, httpResp.Header.Clone(), statusErr{code: httpResp.StatusCode, msg: string(b)}
	}

	lines := make([][]byte, 0, 64)
	scanner := bufio.NewScanner(httpResp.Body)
	scanner.Buffer(nil, 52_428_800)
	for scanner.Scan() {
		lines = append(lines, bytes.Clone(scanner.Bytes()))
	}
	if errScan := scanner.Err(); errScan != nil {
		return nil, httpResp.Header.Clone(), errScan
	}
	return collectChatCompletion(lines), httpResp.Header.Clone(), nil
}

// inspectCodebuddyImage asks the vision model a specific question about a single
// image, returning the model's answer text.
func (e *CodebuddyExecutor) inspectCodebuddyImage(
	ctx context.Context,
	auth *cliproxyauth.Auth,
	creds codebuddy.Creds,
	baseURL string,
	imagePart []byte,
	question string,
	visionModel string,
) (string, error) {
	userContent := []any{
		json.RawMessage(imagePart),
		map[string]any{"type": "text", "text": question},
	}
	body := map[string]any{
		"model": visionModel,
		"messages": []any{
			map[string]any{"role": "user", "content": userContent},
		},
	}
	bodyJSON, err := json.Marshal(body)
	if err != nil {
		return "", err
	}

	aggregated, _, err := e.doCodebuddyChatRequest(ctx, auth, creds, baseURL, bodyJSON)
	if err != nil {
		return "", err
	}
	content := strings.TrimSpace(gjson.GetBytes(aggregated, "choices.0.message.content").String())
	if content == "" {
		return "", fmt.Errorf("vision model returned empty answer")
	}
	return content, nil
}

// runAgenticLoop runs the server-side tool-calling loop: repeatedly send the
// accumulated messages to the text-only model, intercept inspect_image tool
// calls, answer them via the vision model, and continue until the model stops
// calling tools or the round limit is reached. It returns the final aggregated
// ChatCompletion JSON and the last upstream response headers.
func (e *CodebuddyExecutor) runAgenticLoop(
	ctx context.Context,
	auth *cliproxyauth.Auth,
	creds codebuddy.Creds,
	baseURL string,
	body []byte,
	images []codebuddyAgenticImageRef,
	visionModel string,
	maxRounds int,
) ([]byte, http.Header, error) {
	var lastAggregated []byte
	var lastHeaders http.Header

	for round := 0; round < maxRounds; round++ {
		aggregated, headers, err := e.doCodebuddyChatRequest(ctx, auth, creds, baseURL, body)
		if err != nil {
			if lastAggregated != nil {
				return lastAggregated, lastHeaders, nil
			}
			return nil, nil, err
		}
		lastAggregated = aggregated
		lastHeaders = headers

		toolCalls := gjson.GetBytes(aggregated, "choices.0.message.tool_calls")
		if !toolCalls.IsArray() || len(toolCalls.Array()) == 0 {
			// No tool calls: this is the final answer.
			return aggregated, headers, nil
		}

		// Append the assistant message (carrying tool_calls) first.
		assistantRaw := gjson.GetBytes(aggregated, "choices.0.message").Raw
		body, err = appendAgenticMessage(body, []byte(assistantRaw))
		if err != nil {
			return aggregated, headers, nil
		}

		handled := false
		for _, tc := range toolCalls.Array() {
			fn := tc.Get("function")
			if fn.Get("name").String() != inspectImageToolName {
				continue
			}
			handled = true
			toolCallID := tc.Get("id").String()
			argsStr := fn.Get("arguments").String()
			argsParsed := gjson.Parse(argsStr)
			imageID := int(argsParsed.Get("image_id").Int())
			question := argsParsed.Get("question").String()

			var answer string
			if imageID >= 1 && imageID <= len(images) {
				a, inspectErr := e.inspectCodebuddyImage(
					ctx, auth, creds, baseURL, images[imageID-1].partJSON, question, visionModel,
				)
				if inspectErr != nil {
					answer = fmt.Sprintf("[图片查看失败: %v]", inspectErr)
					log.Warnf("codebuddy vision agentic: inspect_image(%d) failed: %v", imageID, inspectErr)
				} else {
					answer = a
				}
			} else {
				answer = fmt.Sprintf("[无效的图片编号 %d，可用范围 1-%d]", imageID, len(images))
			}

			toolMsgJSON, err := json.Marshal(map[string]any{
				"role":         "tool",
				"tool_call_id": toolCallID,
				"content":      answer,
			})
			if err != nil {
				continue
			}
			body, err = appendAgenticMessage(body, toolMsgJSON)
			if err != nil {
				return aggregated, headers, nil
			}
		}

		if !handled {
			// Tool calls present but none is inspect_image: return as-is.
			return aggregated, headers, nil
		}
	}

	// Round limit reached: return the last accumulated result.
	if lastAggregated != nil {
		return lastAggregated, lastHeaders, nil
	}
	return nil, nil, fmt.Errorf("codebuddy vision agentic: no result after %d rounds", maxRounds)
}

// executeCodebuddyVisionAgentic runs the agentic loop for non-streaming Execute
// and translates the final aggregated payload back to the client format.
func (e *CodebuddyExecutor) executeCodebuddyVisionAgentic(
	ctx context.Context,
	auth *cliproxyauth.Auth,
	req cliproxyexecutor.Request,
	opts cliproxyexecutor.Options,
	body []byte,
	baseModel string,
	creds codebuddy.Creds,
	baseURL string,
) (cliproxyexecutor.Response, error) {
	visionCfg := e.cfg.CodebuddyVision
	visionModel := visionCfg.VisionModel()
	maxRounds := visionCfg.MaxVisionToolRounds()

	body, images, err := extractCodebuddyImagesForAgentic(body)
	if err != nil {
		return cliproxyexecutor.Response{}, err
	}
	if len(images) == 0 {
		return cliproxyexecutor.Response{}, fmt.Errorf("codebuddy vision agentic: no images to inspect")
	}
	body = injectCodebuddyInspectTool(body, len(images))

	log.Infof("codebuddy vision agentic: %d images, model=%s, vision=%s, maxRounds=%d",
		len(images), baseModel, visionModel, maxRounds)

	aggregated, headers, err := e.runAgenticLoop(ctx, auth, creds, baseURL, body, images, visionModel, maxRounds)
	if err != nil {
		return cliproxyexecutor.Response{}, err
	}

	respFrom := sdktranslator.FromString("openai")
	respTo := opts.SourceFormat
	var param any
	out := sdktranslator.TranslateNonStream(ctx, respFrom, respTo, req.Model, opts.OriginalRequest, body, aggregated, &param)
	return cliproxyexecutor.Response{Payload: out, Headers: headers}, nil
}

// executeCodebuddyVisionAgenticStream runs the agentic loop in a background
// goroutine and returns immediately, emitting an initial chunk first so the
// relay's stream-open watchdog (default 10s) is satisfied even though the loop
// itself may take 10-30s (multiple deepseek + hy3 round-trips).
func (e *CodebuddyExecutor) executeCodebuddyVisionAgenticStream(
	ctx context.Context,
	auth *cliproxyauth.Auth,
	req cliproxyexecutor.Request,
	opts cliproxyexecutor.Options,
	body []byte,
	baseModel string,
	creds codebuddy.Creds,
	baseURL string,
) (*cliproxyexecutor.StreamResult, error) {
	visionCfg := e.cfg.CodebuddyVision
	visionModel := visionCfg.VisionModel()
	maxRounds := visionCfg.MaxVisionToolRounds()

	// Extract images + inject tool synchronously (fast, <1s).
	body, images, err := extractCodebuddyImagesForAgentic(body)
	if err != nil {
		return nil, err
	}
	if len(images) == 0 {
		return nil, fmt.Errorf("codebuddy vision agentic: no images to inspect")
	}
	body = injectCodebuddyInspectTool(body, len(images))

	log.Infof("codebuddy vision agentic (stream): %d images, model=%s, vision=%s, maxRounds=%d",
		len(images), baseModel, visionModel, maxRounds)

	respFrom := sdktranslator.FromString("openai")
	respTo := opts.SourceFormat

	out := make(chan cliproxyexecutor.StreamChunk)
	go func() {
		defer close(out)
		var param any
		emit := func(line []byte) bool {
			chunks := sdktranslator.TranslateStream(ctx, respFrom, respTo, req.Model, opts.OriginalRequest, body, line, &param)
			for _, c := range chunks {
				select {
				case out <- cliproxyexecutor.StreamChunk{Payload: c}:
				case <-ctx.Done():
					return false
				}
			}
			return true
		}

		// 1. Open the stream immediately (role delta) so the relay watchdog
		//    does not time out while the loop runs.
		initChunk := []byte("data: " + `{"id":"","object":"chat.completion.chunk","created":0,"model":"","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}`)
		if !emit(initChunk) {
			return
		}

		// 2. Run the loop (blocking; may take 10-30s).
		aggregated, _, loopErr := e.runAgenticLoop(ctx, auth, creds, baseURL, body, images, visionModel, maxRounds)
		if loopErr != nil || aggregated == nil {
			emit([]byte("data: [DONE]"))
			return
		}

		// 3. Replay the final content pseudo-streamed.
		id := gjson.GetBytes(aggregated, "id").String()
		model := gjson.GetBytes(aggregated, "model").String()
		created := gjson.GetBytes(aggregated, "created").Int()
		content := gjson.GetBytes(aggregated, "choices.0.message.content").String()

		runes := []rune(content)
		const chunkSize = 8
		for i := 0; i < len(runes); i += chunkSize {
			end := i + chunkSize
			if end > len(runes) {
				end = len(runes)
			}
			delta := string(runes[i:end])
			chunkJSON, err := json.Marshal(map[string]any{
				"id": id, "object": "chat.completion.chunk", "created": created, "model": model,
				"choices": []any{
					map[string]any{"index": 0, "delta": map[string]any{"content": delta}, "finish_reason": nil},
				},
			})
			if err != nil {
				continue
			}
			if !emit(append([]byte("data: "), chunkJSON...)) {
				return
			}
		}

		// Terminal finish chunk.
		finishJSON, _ := json.Marshal(map[string]any{
			"id": id, "object": "chat.completion.chunk", "created": created, "model": model,
			"choices": []any{
				map[string]any{"index": 0, "delta": map[string]any{}, "finish_reason": "stop"},
			},
		})
		emit(append([]byte("data: "), finishJSON...))
		emit([]byte("data: [DONE]"))
	}()
	return &cliproxyexecutor.StreamResult{Chunks: out}, nil
}
