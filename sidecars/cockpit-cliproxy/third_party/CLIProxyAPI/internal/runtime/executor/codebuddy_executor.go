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
	"time"

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

// CodebuddyExecutor is a stateless executor for Tencent CodeBuddy's
// OpenAI-compatible chat completions backend. The backend only supports
// streaming responses, so non-streaming requests are forced to stream and
// aggregated locally.
type CodebuddyExecutor struct {
	cfg *config.Config
}

// NewCodebuddyExecutor creates a new Codebuddy executor.
func NewCodebuddyExecutor(cfg *config.Config) *CodebuddyExecutor {
	return &CodebuddyExecutor{cfg: cfg}
}

// Identifier returns the executor identifier.
func (e *CodebuddyExecutor) Identifier() string { return "codebuddy" }

// PrepareRequest injects CodeBuddy credentials and headers into the outgoing request.
func (e *CodebuddyExecutor) PrepareRequest(req *http.Request, auth *cliproxyauth.Auth) error {
	if req == nil {
		return nil
	}
	creds := codebuddy.CredsFromAuth(auth)
	applyCodebuddyHeaders(req, creds)
	return nil
}

// HttpRequest injects CodeBuddy credentials into the request and executes it.
func (e *CodebuddyExecutor) HttpRequest(ctx context.Context, auth *cliproxyauth.Auth, req *http.Request) (*http.Response, error) {
	if req == nil {
		return nil, fmt.Errorf("codebuddy executor: request is nil")
	}
	if ctx == nil {
		ctx = req.Context()
	}
	httpReq := req.WithContext(ctx)
	if err := e.PrepareRequest(httpReq, auth); err != nil {
		return nil, err
	}
	httpClient := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	return httpClient.Do(httpReq)
}

// Execute performs a chat completion request, aggregating the forced upstream
// stream into a single non-streaming response.
func (e *CodebuddyExecutor) Execute(ctx context.Context, auth *cliproxyauth.Auth, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (resp cliproxyexecutor.Response, err error) {
	if isCodebuddyOpenAIImageRequest(opts) {
		return e.executeOpenAIImage(ctx, auth, req, opts)
	}
	baseModel := req.Model
	creds := codebuddy.CredsFromAuth(auth)
	baseURL := creds.ResolveBaseURL()

	reporter := helps.NewUsageReporter(ctx, e.Identifier(), baseModel, auth)
	defer reporter.TrackFailure(ctx, &err)

	from := opts.SourceFormat
	to := sdktranslator.FromString("openai")

	originalPayloadSource := req.Payload
	if len(opts.OriginalRequest) > 0 {
		originalPayloadSource = opts.OriginalRequest
	}
	originalTranslated := helps.TranslateRequestWithCodexMultiAgentV2(ctx, opts.Headers, e.cfg, from, to, baseModel, originalPayloadSource, true)
	body := helps.TranslateRequestWithCodexMultiAgentV2(ctx, opts.Headers, e.cfg, from, to, baseModel, req.Payload, true)

	// Normalize image content parts to the backend's strict shape. Third-party
	// clients (Cursor, etc.) may send non-standard image structures.
	body = normalizeCodebuddyChatImageContent(body)

	// Vision proxy: transparently handle image input for non-vision models.
	body, _ = e.applyCodebuddyVisionProxy(ctx, auth, body, baseModel)

	// Agentic vision: server-side tool-calling loop for text-only models to
	// autonomously inspect images via the vision model.
	if e.codebuddyVisionAgenticEnabled() && codebuddyChatHasImageInput(body) {
		return e.executeCodebuddyVisionAgentic(ctx, auth, req, opts, body, baseModel, creds, baseURL)
	}

	// Prompt cache: inject a stable session-bound key so repeated turns in the
	// same conversation hit the backend prefix cache (lower credit).
	body = applyCodebuddyPromptCache(body, codebuddyExecutionSessionID(req, opts))

	// The CodeBuddy backend only supports streaming.
	body, err = sjson.SetBytes(body, "stream", true)
	if err != nil {
		return resp, fmt.Errorf("codebuddy executor: failed to force stream: %w", err)
	}
	body, err = sjson.SetBytes(body, "stream_options.include_usage", true)
	if err != nil {
		return resp, fmt.Errorf("codebuddy executor: failed to set stream_options: %w", err)
	}

	requestedModel := helps.PayloadRequestedModel(opts, req.Model)
	requestPath := helps.PayloadRequestPath(opts)
	body = helps.ApplyPayloadConfigWithRequest(e.cfg, baseModel, to.String(), from.String(), "", body, originalTranslated, requestedModel, requestPath, opts.Headers)

	// Normalize tool-related message fields so the strict backend does not
	// reject tool-calling rounds with 400 invalid_parameter_value.
	body, err = normalizeCodebuddyToolMessages(body)
	if err != nil {
		return resp, err
	}

	url := baseURL + codebuddy.ChatPath
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return resp, err
	}
	applyCodebuddyHeaders(httpReq, creds)
	httpReq.Header.Set("Accept", "text/event-stream")
	recordCodebuddyRequest(ctx, e.cfg, e.Identifier(), auth, url, httpReq, body)

	httpClient := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	httpResp, err := httpClient.Do(httpReq)
	if err != nil {
		helps.RecordAPIResponseError(ctx, e.cfg, err)
		return resp, err
	}
	defer func() {
		if errClose := httpResp.Body.Close(); errClose != nil {
			log.Errorf("codebuddy executor: close response body error: %v", errClose)
		}
	}()
	helps.RecordAPIResponseMetadata(ctx, e.cfg, httpResp.StatusCode, httpResp.Header.Clone())
	if httpResp.StatusCode < 200 || httpResp.StatusCode >= 300 {
		b, _ := io.ReadAll(httpResp.Body)
		helps.AppendAPIResponseChunk(ctx, e.cfg, b)
		// Diagnostic: dump the upstream error body (redacted) so the invalid
		// field / param reported by the backend can be inspected.
		helps.DumpCodebuddyDebugBody("error-response", b)
		err = statusErr{code: httpResp.StatusCode, msg: string(b)}
		return resp, err
	}

	// Consume the forced upstream stream and aggregate into a non-stream response.
	dataLines := make([][]byte, 0, 64)
	scanner := bufio.NewScanner(httpResp.Body)
	scanner.Buffer(nil, 52_428_800) // 50MB
	for scanner.Scan() {
		line := scanner.Bytes()
		helps.AppendAPIResponseChunk(ctx, e.cfg, line)
		if detail, ok := helps.ParseOpenAIStreamUsage(line); ok {
			// Diagnostic: dump the upstream usage chunk (redacted) so the exact
			// credit field path can be observed.
			helps.DumpCodebuddyDebugBody("usage", line)
			reporter.Publish(ctx, detail)
		}
		dataLines = append(dataLines, bytes.Clone(line))
	}
	if errScan := scanner.Err(); errScan != nil {
		helps.RecordAPIResponseError(ctx, e.cfg, errScan)
		return resp, errScan
	}

	aggregated := collectChatCompletion(dataLines)
	reporter.Publish(ctx, helps.ParseOpenAIUsage(aggregated))
	reporter.EnsurePublished(ctx)

	// 腾讯 CodeBuddy 后端对部分请求模型 ID（如 glm-5.2）会回退到不同引擎，
	// 并在响应 model 字段里回填真实服务的引擎名（如 GLM-4）。这里把响应 model
	// 字段对齐回客户端请求时的模型 ID，避免测试时看到模型名被静默改写。
	if reqModel := strings.TrimSpace(req.Model); reqModel != "" {
		aggregated, _ = sjson.SetBytes(aggregated, "model", reqModel)
	}

	var param any
	out := sdktranslator.TranslateNonStream(ctx, to, from, req.Model, opts.OriginalRequest, body, aggregated, &param)
	resp = cliproxyexecutor.Response{Payload: out, Headers: httpResp.Header.Clone()}
	return resp, nil
}

// ExecuteStream performs a streaming chat completion request to CodeBuddy.
func (e *CodebuddyExecutor) ExecuteStream(ctx context.Context, auth *cliproxyauth.Auth, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (_ *cliproxyexecutor.StreamResult, err error) {
	baseModel := req.Model
	creds := codebuddy.CredsFromAuth(auth)
	baseURL := creds.ResolveBaseURL()

	reporter := helps.NewUsageReporter(ctx, e.Identifier(), baseModel, auth)
	defer reporter.TrackFailure(ctx, &err)

	from := opts.SourceFormat
	to := sdktranslator.FromString("openai")

	originalPayloadSource := req.Payload
	if len(opts.OriginalRequest) > 0 {
		originalPayloadSource = opts.OriginalRequest
	}
	originalTranslated := helps.TranslateRequestWithCodexMultiAgentV2(ctx, opts.Headers, e.cfg, from, to, baseModel, originalPayloadSource, true)
	body := helps.TranslateRequestWithCodexMultiAgentV2(ctx, opts.Headers, e.cfg, from, to, baseModel, req.Payload, true)

	// Normalize image content parts to the backend's strict shape.
	body = normalizeCodebuddyChatImageContent(body)

	// Vision proxy: transparently handle image input for non-vision models.
	body, _ = e.applyCodebuddyVisionProxy(ctx, auth, body, baseModel)

	// Agentic vision: server-side tool-calling loop for text-only models to
	// autonomously inspect images via the vision model.
	if e.codebuddyVisionAgenticEnabled() && codebuddyChatHasImageInput(body) {
		return e.executeCodebuddyVisionAgenticStream(ctx, auth, req, opts, body, baseModel, creds, baseURL)
	}

	// Prompt cache: inject a stable session-bound key so repeated turns in the
	// same conversation hit the backend prefix cache (lower credit).
	body = applyCodebuddyPromptCache(body, codebuddyExecutionSessionID(req, opts))

	body, err = sjson.SetBytes(body, "stream", true)
	if err != nil {
		return nil, fmt.Errorf("codebuddy executor: failed to force stream: %w", err)
	}
	body, err = sjson.SetBytes(body, "stream_options.include_usage", true)
	if err != nil {
		return nil, fmt.Errorf("codebuddy executor: failed to set stream_options: %w", err)
	}

	requestedModel := helps.PayloadRequestedModel(opts, req.Model)
	requestPath := helps.PayloadRequestPath(opts)
	body = helps.ApplyPayloadConfigWithRequest(e.cfg, baseModel, to.String(), from.String(), "", body, originalTranslated, requestedModel, requestPath, opts.Headers)

	// Normalize tool-related message fields so the strict backend does not
	// reject tool-calling rounds with 400 invalid_parameter_value.
	body, err = normalizeCodebuddyToolMessages(body)
	if err != nil {
		return nil, err
	}

	url := baseURL + codebuddy.ChatPath
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	applyCodebuddyHeaders(httpReq, creds)
	httpReq.Header.Set("Accept", "text/event-stream")
	httpReq.Header.Set("Cache-Control", "no-cache")
	recordCodebuddyRequest(ctx, e.cfg, e.Identifier(), auth, url, httpReq, body)
	// Diagnostic: dump the upstream request body (redacted) so the exact tools
	// definition Cursor sent can be inspected.
	helps.DumpCodebuddyDebugBody("upstream-request", body)

	httpClient := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	httpResp, err := httpClient.Do(httpReq)
	if err != nil {
		helps.RecordAPIResponseError(ctx, e.cfg, err)
		return nil, err
	}
	helps.RecordAPIResponseMetadata(ctx, e.cfg, httpResp.StatusCode, httpResp.Header.Clone())
	if httpResp.StatusCode < 200 || httpResp.StatusCode >= 300 {
		b, _ := io.ReadAll(httpResp.Body)
		helps.AppendAPIResponseChunk(ctx, e.cfg, b)
		// Diagnostic: dump the upstream error body (redacted) so the invalid
		// field / param reported by the backend can be inspected.
		helps.DumpCodebuddyDebugBody("error-response", b)
		if errClose := httpResp.Body.Close(); errClose != nil {
			log.Errorf("codebuddy executor: close response body error: %v", errClose)
		}
		err = statusErr{code: httpResp.StatusCode, msg: string(b)}
		return nil, err
	}

	out := make(chan cliproxyexecutor.StreamChunk)
	go func() {
		defer close(out)
		defer func() {
			if errClose := httpResp.Body.Close(); errClose != nil {
				log.Errorf("codebuddy executor: close response body error: %v", errClose)
			}
		}()
		scanner := bufio.NewScanner(httpResp.Body)
		scanner.Buffer(nil, 52_428_800) // 50MB
		var param any
		tcBuf := newCodebuddyStreamToolCallBuffer()
		emittedToolCalls := false
		emit := func(line []byte) bool {
			chunks := sdktranslator.TranslateStream(ctx, to, from, req.Model, opts.OriginalRequest, body, bytes.Clone(line), &param)
			for i := range chunks {
				// Diagnostic: dump each downstream chunk (redacted) so the exact
				// stream Cursor receives can be inspected for duplicated tool_calls.
				helps.DumpCodebuddyDebugBody("downstream", chunks[i])
				select {
				case out <- cliproxyexecutor.StreamChunk{Payload: chunks[i]}:
				case <-ctx.Done():
					return false
				}
			}
			return true
		}
		emitToolCalls := func() bool {
			if emittedToolCalls || !tcBuf.HasToolCalls() {
				return true
			}
			emittedToolCalls = true
			if tc := tcBuf.BuildToolCallsChunk(); tc != nil {
				return emit(tc)
			}
			return true
		}
		for scanner.Scan() {
			line := scanner.Bytes()
			helps.AppendAPIResponseChunk(ctx, e.cfg, line)
			if detail, ok := helps.ParseOpenAIStreamUsage(line); ok {
				// Diagnostic: dump the upstream usage chunk (redacted) so the exact
				// credit field path can be observed.
				helps.DumpCodebuddyDebugBody("usage", line)
				reporter.Publish(ctx, detail)
			}
			trimmedLine := bytes.TrimSpace(line)
			if len(trimmedLine) == 0 {
				continue
			}
			if !bytes.HasPrefix(trimmedLine, []byte("data:")) {
				continue
			}
			// Buffer tool_calls and strip them (plus reasoning_content) from the
			// forwarded delta so Cursor receives one consolidated tool_calls chunk
			// instead of concatenating multiple complete argument snapshots.
			stripped, finishReason := tcBuf.Consume(trimmedLine)
			if finishReason != "" {
				if !emitToolCalls() {
					return
				}
			}
			if !emit(stripped) {
				return
			}
		}
		if errScan := scanner.Err(); errScan != nil {
			helps.RecordAPIResponseError(ctx, e.cfg, errScan)
			reporter.PublishFailure(ctx, errScan)
			select {
			case out <- cliproxyexecutor.StreamChunk{Err: errScan}:
			case <-ctx.Done():
			}
			return
		}
		if !emitToolCalls() {
			return
		}
		emit([]byte("data: [DONE]"))
	}()
	return &cliproxyexecutor.StreamResult{Headers: httpResp.Header.Clone(), Chunks: out}, nil
}

// CountTokens is not supported for CodeBuddy.
func (e *CodebuddyExecutor) CountTokens(ctx context.Context, auth *cliproxyauth.Auth, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	return cliproxyexecutor.Response{}, fmt.Errorf("codebuddy executor: count tokens not implemented")
}

// Refresh refreshes the CodeBuddy access token using the refresh token.
func (e *CodebuddyExecutor) Refresh(ctx context.Context, auth *cliproxyauth.Auth) (*cliproxyauth.Auth, error) {
	if auth == nil {
		return nil, fmt.Errorf("codebuddy executor: auth is nil")
	}
	creds := codebuddy.CredsFromAuth(auth)
	if strings.TrimSpace(creds.RefreshToken) == "" {
		// Nothing to refresh.
		return auth, nil
	}

	url := creds.ResolveRefreshURL()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, url, strings.NewReader("{}"))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json")
	req.Header.Set("Authorization", "Bearer "+creds.AccessToken)
	req.Header.Set("X-Refresh-Token", creds.RefreshToken)
	if creds.Domain != "" {
		req.Header.Set("X-Domain", creds.Domain)
	}

	client := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer func() { _ = resp.Body.Close() }()
	body, _ := io.ReadAll(resp.Body)
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, fmt.Errorf("codebuddy refresh failed: status %d: %s", resp.StatusCode, summarize(body))
	}

	var envelope struct {
		Code int             `json:"code"`
		Data json.RawMessage `json:"data"`
	}
	if err := json.Unmarshal(body, &envelope); err != nil {
		return nil, fmt.Errorf("codebuddy refresh: decode envelope: %w", err)
	}
	if envelope.Code != 0 && envelope.Code != 200 {
		return nil, fmt.Errorf("codebuddy refresh failed (code=%d)", envelope.Code)
	}

	var data map[string]any
	if err := json.Unmarshal(envelope.Data, &data); err != nil {
		return nil, fmt.Errorf("codebuddy refresh: decode data: %w", err)
	}
	if auth.Metadata == nil {
		auth.Metadata = make(map[string]any)
	}
	if at, ok := data["accessToken"].(string); ok && strings.TrimSpace(at) != "" {
		auth.Metadata["access_token"] = at
	}
	if rt, ok := data["refreshToken"].(string); ok && strings.TrimSpace(rt) != "" {
		auth.Metadata["refresh_token"] = rt
	}
	if exp, ok := data["expiresAt"].(float64); ok && exp > 0 {
		auth.Metadata["expired"] = time.Unix(int64(exp)/1000, 0).UTC().Format(time.RFC3339)
	}
	auth.Metadata["last_refresh"] = time.Now().UTC().Format(time.RFC3339)
	auth.Metadata["type"] = "codebuddy"
	return auth, nil
}

// CodeBuddy image generation uses a dedicated OpenAI Images API compatible
// endpoint (/v2/images/generations, /v2/images/edits) rather than the Chat
// Completions image_generation tool (verified 2026-08-19 against a real CN
// account: the image_generation tool is silently ignored and answered with an
// SVG snippet, while /v2/images/generations parses model/prompt/n/size and
// returns "Image model [...] route config not found" when the account lacks an
// image route). The client's image request body is passed through largely
// unchanged; the upstream response is already OpenAI Images API shaped.
const codebuddyOpenAIImageSourceFormat = "openai-image"

// isCodebuddyOpenAIImageRequest reports whether the request targets the OpenAI
// Images API via the codebuddy provider.
func isCodebuddyOpenAIImageRequest(opts cliproxyexecutor.Options) bool {
	if !strings.EqualFold(strings.TrimSpace(opts.SourceFormat.String()), codebuddyOpenAIImageSourceFormat) {
		return false
	}
	path := helps.PayloadRequestPath(opts)
	return path == "/v1/images/generations" || path == "/v1/images/edits"
}

// executeOpenAIImage relays an OpenAI Images API request to the backend's
// dedicated image endpoint. The endpoint returns a single JSON document (not an
// SSE stream), so this path is non-streaming.
func (e *CodebuddyExecutor) executeOpenAIImage(ctx context.Context, auth *cliproxyauth.Auth, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (resp cliproxyexecutor.Response, err error) {
	creds := codebuddy.CredsFromAuth(auth)
	baseURL := creds.ResolveBaseURL()
	endpoint := codebuddy.ImageGenerationsPath
	if helps.PayloadRequestPath(opts) == "/v1/images/edits" {
		endpoint = codebuddy.ImageEditsPath
	}

	reporter := helps.NewUsageReporter(ctx, e.Identifier(), req.Model, auth)
	defer reporter.TrackFailure(ctx, &err)

	body := req.Payload
	url := baseURL + endpoint
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(body))
	if err != nil {
		return resp, err
	}
	applyCodebuddyHeaders(httpReq, creds)
	httpReq.Header.Set("Accept", "application/json")
	recordCodebuddyRequest(ctx, e.cfg, e.Identifier(), auth, url, httpReq, body)

	httpClient := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	httpResp, err := httpClient.Do(httpReq)
	if err != nil {
		helps.RecordAPIResponseError(ctx, e.cfg, err)
		return resp, err
	}
	defer func() {
		if errClose := httpResp.Body.Close(); errClose != nil {
			log.Errorf("codebuddy executor: close image response body error: %v", errClose)
		}
	}()
	helps.RecordAPIResponseMetadata(ctx, e.cfg, httpResp.StatusCode, httpResp.Header.Clone())
	respBody, readErr := io.ReadAll(io.LimitReader(httpResp.Body, 52_428_800))
	if readErr != nil {
		helps.RecordAPIResponseError(ctx, e.cfg, readErr)
		return resp, readErr
	}
	helps.AppendAPIResponseChunk(ctx, e.cfg, respBody)
	if httpResp.StatusCode < 200 || httpResp.StatusCode >= 300 {
		return resp, statusErr{code: httpResp.StatusCode, msg: string(respBody)}
	}
	return cliproxyexecutor.Response{Payload: respBody, Headers: httpResp.Header.Clone()}, nil
}

// normalizeCodebuddyChatImageContent rewrites `messages[].content[]` image parts
// into the exact shape the Tencent backend requires:
//
//	{"type":"image_url","image_url":{"url":"<string>"}}
//
// The backend is strict (verified 2026-08-19 against a real CN account): a
// string `image_url`, a nested `url` object, or a double-encoded JSON string all
// fail with HTTP 400. Third-party clients (Cursor, etc.) can emit these shapes,
// so this defensive normalization runs after protocol translation and before the
// request is sent upstream. Non-image parts and already-standard image parts are
// left untouched so the blast radius is limited to CodeBuddy only.
func normalizeCodebuddyChatImageContent(body []byte) []byte {
	if len(body) == 0 || !gjson.ValidBytes(body) {
		return body
	}
	messages := gjson.GetBytes(body, "messages")
	if !messages.IsArray() {
		return body
	}

	out := body
	changed := false
	msgArr := messages.Array()
	for mi := range msgArr {
		content := msgArr[mi].Get("content")
		if !content.IsArray() {
			continue
		}
		partArr := content.Array()
		for ci := range partArr {
			normalized := normalizeCodebuddyImagePart(partArr[ci])
			if normalized == nil {
				continue
			}
			path := fmt.Sprintf("messages.%d.content.%d", mi, ci)
			var err error
			out, err = sjson.SetRawBytes(out, path, normalized)
			if err != nil {
				// Never corrupt the request on a path error; keep the original.
				return body
			}
			changed = true
		}
	}
	if !changed {
		return body
	}
	return out
}

// normalizeCodebuddyImagePart normalizes a single message content part.
// It returns the canonical JSON part, or nil when the part is not an image or
// already conforms to the backend's expected shape.
func normalizeCodebuddyImagePart(part gjson.Result) []byte {
	typ := part.Get("type").String()
	img := part.Get("image_url")
	switch typ {
	case "image_url":
		// Already canonical: object with a string url.
		if img.IsObject() && img.Get("url").Type == gjson.String && img.Get("url").String() != "" {
			return nil
		}
	case "input_image":
		// Responses-style part that slipped through to the Chat path.
	default:
		return nil
	}

	url := extractCodebuddyImageURL(img)
	if url == "" {
		return nil
	}

	imageObj := map[string]any{"url": url}
	if detail := img.Get("detail").String(); detail != "" {
		imageObj["detail"] = detail
	} else if detail := part.Get("detail").String(); detail != "" {
		imageObj["detail"] = detail
	}

	out, err := json.Marshal(map[string]any{"type": "image_url", "image_url": imageObj})
	if err != nil {
		return nil
	}
	return out
}

// extractCodebuddyImageURL extracts a string URL from an image payload that may
// be a plain string, a nested object, or a double-encoded JSON string.
func extractCodebuddyImageURL(img gjson.Result) string {
	switch {
	case img.Type == gjson.String:
		s := img.String()
		trimmed := strings.TrimSpace(s)
		// Double-encoded JSON string (e.g. `"{\"url\":\"...\"}"`).
		if strings.HasPrefix(trimmed, "{") {
			if inner := gjson.Parse(s); inner.IsObject() {
				if u := extractCodebuddyImageURL(inner); u != "" {
					return u
				}
			}
		}
		return s
	case img.IsObject():
		if u := img.Get("url"); u.Exists() {
			if u.Type == gjson.String && u.String() != "" {
				return u.String()
			}
			// url is itself an object (the "extra braces" case): recurse.
			if u.IsObject() {
				if inner := extractCodebuddyImageURL(u); inner != "" {
					return inner
				}
			}
		}
		// Nested image_url field.
		if inner := img.Get("image_url"); inner.Exists() {
			if u := extractCodebuddyImageURL(inner); u != "" {
				return u
			}
		}
	}
	return ""
}

// applyCodebuddyHeaders sets the headers required by the CodeBuddy backend.
func applyCodebuddyHeaders(r *http.Request, creds codebuddy.Creds) {
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("Accept", "application/json")
	if creds.AccessToken != "" {
		r.Header.Set("Authorization", "Bearer "+creds.AccessToken)
	}
	if creds.UID != "" {
		r.Header.Set("X-User-Id", creds.UID)
	}
	if creds.EnterpriseID != "" {
		r.Header.Set("X-Enterprise-Id", creds.EnterpriseID)
		r.Header.Set("X-Tenant-Id", creds.EnterpriseID)
	}
	if creds.Domain != "" {
		r.Header.Set("X-Domain", creds.Domain)
	}
	r.Header.Set("X-Product", "SaaS")
	r.Header.Set("X-IDE-Name", "CodeBuddyIDE")
	r.Header.Set("X-Requested-With", "XMLHttpRequest")
	r.Header.Set("User-Agent", "CodeBuddyIDE")
}

func recordCodebuddyRequest(ctx context.Context, cfg *config.Config, provider string, auth *cliproxyauth.Auth, url string, httpReq *http.Request, body []byte) {
	// Diagnostic: dump the final upstream request body (redacted) to stdout when
	// CODEBUDDY_DEBUG_BODY=1, so 200 vs 400 field differences can be inspected.
	helps.DumpCodebuddyDebugBody("request", body)
	var authID, authLabel, authType, authValue string
	if auth != nil {
		authID = auth.ID
		authLabel = auth.Label
		authType, authValue = auth.AccountInfo()
	}
	helps.RecordAPIRequest(ctx, cfg, helps.UpstreamRequestLog{
		URL:       url,
		Method:    http.MethodPost,
		Headers:   httpReq.Header.Clone(),
		Body:      body,
		Provider:  provider,
		AuthID:    authID,
		AuthLabel: authLabel,
		AuthType:  authType,
		AuthValue: authValue,
	})
}

func summarize(b []byte) string {
	s := strings.TrimSpace(string(b))
	if len(s) > 512 {
		return s[:512]
	}
	return s
}

// collectChatCompletion aggregates an OpenAI SSE stream into a single
// non-streaming chat.completion object.
func collectChatCompletion(lines [][]byte) []byte {
	var (
		id                string
		model             string
		created           int64
		finish            string
		role              string
		content           string
		reasoningContent  string
		usage             gjson.Result
		toolCallFragments []gjson.Result
	)

	for _, line := range lines {
		trimmed := bytes.TrimSpace(line)
		if !bytes.HasPrefix(trimmed, []byte("data:")) {
			continue
		}
		payload := bytes.TrimSpace(trimmed[len("data:"):])
		if len(payload) == 0 || bytes.Equal(payload, []byte("[DONE]")) {
			continue
		}
		if !gjson.ValidBytes(payload) {
			continue
		}
		res := gjson.ParseBytes(payload)
		if id == "" {
			id = res.Get("id").String()
		}
		if model == "" {
			model = res.Get("model").String()
		}
		if created == 0 {
			created = res.Get("created").Int()
		}
		if u := res.Get("usage"); u.Exists() {
			usage = u
		}
		for _, ch := range res.Get("choices").Array() {
			delta := ch.Get("delta")
			if role == "" {
				if r := delta.Get("role"); r.Exists() && r.String() != "" {
					role = r.String()
				}
			}
			if c := delta.Get("content"); c.Exists() && c.Type == gjson.String {
				content += c.String()
			}
			if rc := delta.Get("reasoning_content"); rc.Exists() && rc.Type == gjson.String {
				reasoningContent += rc.String()
			}
			for _, tc := range delta.Get("tool_calls").Array() {
				toolCallFragments = append(toolCallFragments, tc)
			}
			if fr := ch.Get("finish_reason"); fr.Exists() && fr.String() != "" {
				finish = fr.String()
			}
		}
	}

	if role == "" {
		role = "assistant"
	}
	if finish == "" {
		finish = "stop"
	}

	toolCalls := mergeToolCallFragments(toolCallFragments)
	message := map[string]any{
		"role":    role,
		"content": content,
	}
	if len(toolCalls) > 0 {
		// Assistant messages carrying tool_calls must not expose non-null text
		// content; clients (Cursor) expect null here.
		if strings.TrimSpace(content) == "" {
			message["content"] = nil
		}
		message["tool_calls"] = toolCalls
	}
	if reasoningContent != "" {
		message["reasoning_content"] = reasoningContent
	}

	obj := map[string]any{
		"id":      id,
		"object":  "chat.completion",
		"created": created,
		"model":   model,
		"choices": []map[string]any{
			{
				"index":         0,
				"message":       message,
				"finish_reason": finish,
			},
		},
	}
	if usage.Exists() {
		obj["usage"] = usage.Value()
	}

	out, err := json.Marshal(obj)
	if err != nil {
		return []byte(`{"object":"chat.completion","choices":[]}`)
	}
	return out
}

// normalizeToolCallID rewrites the backend's `tooluse_` prefix to the standard
// `call_` prefix expected by OpenAI-compatible clients.
func normalizeToolCallID(id string) string {
	id = strings.TrimSpace(id)
	if strings.HasPrefix(id, "tooluse_") {
		return "call_" + strings.TrimPrefix(id, "tooluse_")
	}
	return id
}

// mergeToolCallFragments merges streaming tool_calls deltas by index.
func mergeToolCallFragments(fragments []gjson.Result) []map[string]any {
	type acc struct {
		index     int64
		id        string
		typ       string
		name      string
		arguments string
	}
	byIndex := make(map[int64]*acc)
	order := make([]int64, 0)
	for _, tc := range fragments {
		idx := tc.Get("index").Int()
		a, ok := byIndex[idx]
		if !ok {
			a = &acc{index: idx}
			byIndex[idx] = a
			order = append(order, idx)
		}
		if a.id == "" {
			if v := tc.Get("id"); v.Exists() && strings.TrimSpace(v.String()) != "" {
				a.id = normalizeToolCallID(v.String())
			}
		}
		if v := tc.Get("type"); v.Exists() && v.String() != "" {
			a.typ = v.String()
		}
		if fn := tc.Get("function"); fn.Exists() {
			if v := fn.Get("name"); v.Exists() && v.String() != "" {
				a.name = v.String()
			}
			if v := fn.Get("arguments"); v.Exists() && v.String() != "" {
				arg := v.String()
				switch {
				case a.arguments == "":
					if isCompleteJSONObject(arg) {
						if isEmptyToolArguments(arg) {
							// First snapshot is an empty "{}": skip it and wait
							// for the real arguments instead of seeding empty.
							break
						}
						a.arguments = strings.TrimSpace(arg)
					} else {
						a.arguments = arg
					}
				case isCompleteJSONObject(arg):
					trimmed := strings.TrimSpace(arg)
					if isEmptyToolArguments(trimmed) {
						// Empty "{}" snapshot: never clobber real arguments.
						break
					}
					// Full snapshot: replace, not append; identical values dedupe.
					if trimmed != a.arguments {
						a.arguments = trimmed
					}
				default:
					// Incremental fragment: append.
					a.arguments += arg
				}
			}
		}
	}

	out := make([]map[string]any, 0, len(order))
	for _, idx := range order {
		a := byIndex[idx]
		arguments := a.arguments
		if strings.TrimSpace(arguments) == "" {
			arguments = "{}"
		}
		item := map[string]any{
			"index": a.index,
			"id":    a.id,
			"type":  "function",
			"function": map[string]any{
				"name":      a.name,
				"arguments": arguments,
			},
		}
		if a.typ != "" {
			item["type"] = a.typ
		}
		out = append(out, item)
	}
	return out
}
