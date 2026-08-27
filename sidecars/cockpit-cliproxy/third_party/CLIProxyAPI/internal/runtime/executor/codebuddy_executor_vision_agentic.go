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
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	log "github.com/sirupsen/logrus"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// inspectImageToolName is the tool injected into the text-only model so it can
// autonomously query the vision model for image details during reasoning.
const inspectImageToolName = "inspect_image"

// codebuddyVisionSubagentSuffix is appended to the request-log model label when
// a text-only model request is handled by the pure-text vision sub-agent loop
// (主模型 + 混元视觉子代理), so the log reads e.g. "deepseek-v4-pro视".
const codebuddyVisionSubagentSuffix = "视"

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

// extractCodebuddyImagesForAgentic rewrites the body so no image part reaches a
// text-only model: images in the current turn (the last user message and any
// subsequent assistant/tool messages) are extracted as inspect_image targets,
// while images in earlier messages (stale history re-sent by the client) are
// replaced with a neutral placeholder.
func extractCodebuddyImagesForAgentic(body []byte) ([]byte, []codebuddyAgenticImageRef, error) {
	messages := gjson.GetBytes(body, "messages")
	if !messages.IsArray() {
		return body, nil, nil
	}
	arr := messages.Array()
	lastUserIdx := lastCodebuddyUserMessageIndex(arr)
	if lastUserIdx < 0 {
		return body, nil, nil
	}

	out := body
	var images []codebuddyAgenticImageRef
	for mi, msg := range arr {
		content := msg.Get("content")
		if !content.IsArray() {
			continue
		}
		isCurrent := mi >= lastUserIdx
		for ci, part := range content.Array() {
			if !isCodebuddyImagePartType(part.Get("type").String()) {
				continue
			}
			path := fmt.Sprintf("messages.%d.content.%d", mi, ci)

			var replacement []byte
			if isCurrent {
				id := len(images) + 1
				images = append(images, codebuddyAgenticImageRef{
					id:       id,
					partJSON: append([]byte(nil), []byte(part.Raw)...),
				})
				text := fmt.Sprintf("[图片 #%d 已附加，可用 inspect_image 工具查看，image_id=%d]", id, id)
				var err error
				replacement, err = json.Marshal(map[string]string{"type": "text", "text": text})
				if err != nil {
					return body, nil, err
				}
			} else {
				replacement, _ = json.Marshal(map[string]string{"type": "text", "text": codebuddyHistoricalImageText})
			}

			var err error
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
	// The client may have sent a `tool_choice` naming one of its own tools (or
	// "required"). Since `tools` was just replaced with the single inspect_image
	// tool, a stale tool_choice is now inconsistent and can be rejected by the
	// strict backend with 400 — reset it to "auto".
	out, err = sjson.SetBytes(out, "tool_choice", "auto")
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
	// Clamp oversized max_tokens (Cursor sends 65536) to the model's declared
	// ceiling, matching the normal request path.
	body = clampCodebuddyMaxTokens(body, gjson.GetBytes(body, "model").String())

	url := baseURL + codebuddy.ChatPath
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return nil, nil, err
	}
	applyCodebuddyHeaders(httpReq, creds)
	httpReq.Header.Set("Accept", "text/event-stream")

	// Diagnostic: dump the agentic sub-request body (redacted) so the exact
	// tools/tool_choice/max_tokens the loop sends can be inspected on failure.
	helps.DumpCodebuddyDebugBody("agentic-request", body)

	httpClient := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	httpResp, err := httpClient.Do(httpReq)
	if err != nil {
		return nil, nil, err
	}
	defer func() { _ = httpResp.Body.Close() }()
	if httpResp.StatusCode < 200 || httpResp.StatusCode >= 300 {
		b, _ := io.ReadAll(httpResp.Body)
		helps.DumpCodebuddyDebugBody("agentic-error", b)
		log.Warnf("codebuddy vision agentic: round request failed status=%d body=%s",
			httpResp.StatusCode, summarize(b))
		return nil, httpResp.Header.Clone(), statusErr{code: codebuddyEffectiveStatus(httpResp.StatusCode, b), msg: string(b)}
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
) (string, usage.Detail, error) {
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
		return "", usage.Detail{}, err
	}

	aggregated, _, err := e.doCodebuddyChatRequest(ctx, auth, creds, baseURL, bodyJSON)
	if err != nil {
		return "", usage.Detail{}, err
	}
	content := strings.TrimSpace(gjson.GetBytes(aggregated, "choices.0.message.content").String())
	if content == "" {
		return "", helps.ParseOpenAIUsage(aggregated), fmt.Errorf("vision model returned empty answer")
	}
	return content, helps.ParseOpenAIUsage(aggregated), nil
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
	reporter *helps.UsageReporter,
) ([]byte, http.Header, error) {
	var lastAggregated []byte
	var lastHeaders http.Header
	var totalUsage usage.Detail

	// Publish the aggregated usage of the whole agentic loop exactly once,
	// regardless of which return path terminates the loop. The reporter is
	// constructed with the "<model>视" label upstream so the request log shows
	// that this text-only model request used the vision sub-agent.
	defer func() {
		if reporter != nil {
			helps.DumpCodebuddyDebugBody("vision-reporter-publish",
				[]byte(fmt.Sprintf("model=%s visionSubagent=true inputTokens=%d outputTokens=%d credit=%v",
					reporter.Model(), totalUsage.InputTokens, totalUsage.OutputTokens, totalUsage.Credit)))
			reporter.Publish(ctx, totalUsage)
		}
	}()

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

		// Accumulate the main-loop usage so the request log's token/credit
		// columns reflect the entire vision sub-agent conversation.
		addCodebuddyAgenticUsage(&totalUsage, helps.ParseOpenAIUsage(aggregated))

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
				a, inspectUsage, inspectErr := e.inspectCodebuddyImage(
					ctx, auth, creds, baseURL, images[imageID-1].partJSON, question, visionModel,
				)
				if inspectErr != nil {
					answer = fmt.Sprintf("[图片查看失败: %v]", inspectErr)
					log.Warnf("codebuddy vision agentic: inspect_image(%d) failed: %v", imageID, inspectErr)
				} else {
					answer = a
					// Accumulate the vision sub-model usage so the request log's
					// token/credit columns include the whole sub-agent loop.
					addCodebuddyAgenticUsage(&totalUsage, inspectUsage)
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

// addCodebuddyAgenticUsage accumulates a single round's usage into the running
// total for the vision sub-agent loop.
func addCodebuddyAgenticUsage(total *usage.Detail, add usage.Detail) {
	if total == nil {
		return
	}
	total.InputTokens += add.InputTokens
	total.OutputTokens += add.OutputTokens
	total.ReasoningTokens += add.ReasoningTokens
	total.CachedTokens += add.CachedTokens
	total.CacheReadTokens += add.CacheReadTokens
	total.CacheCreationTokens += add.CacheCreationTokens
	total.TotalTokens += add.TotalTokens
	total.Credit += add.Credit
	total.TokenBreakdown.TotalTokens += add.TokenBreakdown.TotalTokens
	total.TokenBreakdown.Input.TotalTokens += add.TokenBreakdown.Input.TotalTokens
	total.TokenBreakdown.Input.UncachedTokens += add.TokenBreakdown.Input.UncachedTokens
	total.TokenBreakdown.Input.CacheReadTokens += add.TokenBreakdown.Input.CacheReadTokens
	total.TokenBreakdown.Input.CacheWriteTokens += add.TokenBreakdown.Input.CacheWriteTokens
	total.TokenBreakdown.Output.TotalTokens += add.TokenBreakdown.Output.TotalTokens
	total.TokenBreakdown.Output.NonReasoningTokens += add.TokenBreakdown.Output.NonReasoningTokens
	total.TokenBreakdown.Output.ReasoningTokens += add.TokenBreakdown.Output.ReasoningTokens
	total.TokenBreakdown.UnclassifiedTokens += add.TokenBreakdown.UnclassifiedTokens
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
	reporter *helps.UsageReporter,
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

	aggregated, headers, err := e.runAgenticLoop(ctx, auth, creds, baseURL, body, images, visionModel, maxRounds, reporter)
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
	reporter *helps.UsageReporter,
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
		aggregated, _, loopErr := e.runAgenticLoop(ctx, auth, creds, baseURL, body, images, visionModel, maxRounds, reporter)
		if loopErr != nil || aggregated == nil {
			// Surface the failure as visible assistant content instead of a
			// silent empty response, and log the full error for diagnosis.
			errText := "codebuddy 视觉代理失败，未能获取图片描述"
			if loopErr != nil {
				errText = fmt.Sprintf("codebuddy 视觉代理失败: %s", summarize([]byte(loopErr.Error())))
				log.Errorf("codebuddy vision agentic (stream): loop failed: %v", loopErr)
			} else {
				log.Errorf("codebuddy vision agentic (stream): loop returned no result")
			}
			errChunk, errJSON := json.Marshal(map[string]any{
				"id": "", "object": "chat.completion.chunk", "created": 0, "model": baseModel,
				"choices": []any{
					map[string]any{"index": 0, "delta": map[string]any{"content": errText}, "finish_reason": nil},
				},
			})
			if errJSON == nil {
				if !emit(append([]byte("data: "), errChunk...)) {
					return
				}
			}
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
