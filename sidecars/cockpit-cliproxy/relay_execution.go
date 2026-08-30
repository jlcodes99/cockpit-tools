package main

import (
	"bytes"
	"context"

	"encoding/json"

	"fmt"

	"net/http"

	"strings"

	"time"

	"github.com/gin-gonic/gin"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"

	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func (s *relayServer) executeStreamWithOpenTimeout(
	c *gin.Context,
	ctx context.Context,
	providers []string,
	req cliproxyexecutor.Request,
	opts cliproxyexecutor.Options,
	model string,
	startedAt time.Time,
	openTimeout time.Duration,
) (*cliproxyexecutor.StreamResult, error) {
	attempts := s.streamOpenMaxAttempts()
	if attempts <= 0 {
		attempts = 1
	}
	if openTimeout <= 0 {
		openTimeout = streamOpenTimeout
	}
	for attempt := 1; attempt <= attempts; attempt++ {
		attemptCtx, cancelAttempt := context.WithCancel(ctx)
		done := make(chan executeStreamResult, 1)
		s.emitExecutorDiagnostic(
			c,
			"stream_open_attempt",
			model,
			"execute_stream",
			startedAt,
			fmt.Sprintf("attempt=%d/%d open_timeout=%s", attempt, attempts, openTimeout),
		)
		go func() {
			result, err := s.runtime.ExecuteStream(attemptCtx, providers, req, opts)
			done <- executeStreamResult{result: result, err: err}
		}()

		timer := time.NewTimer(openTimeout)
		select {
		case out := <-done:
			timer.Stop()
			if out.err != nil || out.result == nil {
				cancelAttempt()
			}
			return out.result, out.err
		case <-ctx.Done():
			timer.Stop()
			cancelAttempt()
			s.emitExecutorDiagnostic(
				c,
				"stream_open_canceled",
				model,
				"execute_stream",
				startedAt,
				fmt.Sprintf("cancel_source=downstream_context err=%v", ctx.Err()),
			)
			return nil, ctx.Err()
		case <-timer.C:
			cancelAttempt()
			err := relayTimeoutError{phase: fmt.Sprintf("stream_open attempt=%d/%d", attempt, attempts), timeout: openTimeout}
			detail := fmt.Sprintf("cancel_source=gateway_timeout_cancel %s", err.Error())
			if attempt < attempts {
				s.emitExecutorDiagnostic(c, "stream_open_retry", model, "execute_stream", startedAt, detail)
				continue
			}
			s.emitExecutorDiagnostic(c, "stream_open_retry_failed", model, "execute_stream", startedAt, detail)
			return nil, err
		}
	}
	return nil, relayTimeoutError{phase: "stream_open", timeout: openTimeout}
}

func (s *relayServer) startExecutorWaitLogger(c *gin.Context, model, phase string, startedAt time.Time) func() {
	if s == nil || s.emitter == nil || c == nil || c.Request == nil || !s.debugLogsEnabled() {
		return func() {}
	}
	payload := s.executorDiagnosticPayload(c, "executor_waiting", model, phase, startedAt, "")
	done := make(chan struct{})
	go func() {
		ticker := time.NewTicker(executorWaitLogInterval)
		defer ticker.Stop()
		for {
			select {
			case <-done:
				return
			case <-ticker.C:
				payload.LatencyMS = time.Since(startedAt).Milliseconds()
				payload.ErrorMessage = fmt.Sprintf("phase=%s", phase)
				s.emitter.emit(payload)
			}
		}
	}()
	return func() {
		close(done)
	}
}

func (s *relayServer) emitExecutorDiagnostic(c *gin.Context, typ, model, phase string, startedAt time.Time, message string) {
	if s == nil || s.emitter == nil || c == nil || c.Request == nil || !s.debugLogsEnabled() {
		return
	}
	s.emitter.emit(s.executorDiagnosticPayload(c, typ, model, phase, startedAt, message))
}

func (s *relayServer) debugLogsEnabled() bool {
	if s == nil || s.manifest == nil || s.manifest.DebugLogs == nil {
		return true
	}
	return *s.manifest.DebugLogs
}

func (s *relayServer) executorDiagnosticPayload(c *gin.Context, typ, model, phase string, startedAt time.Time, message string) requestDiagnosticPayload {
	spec, _ := c.Request.Context().Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestKind, _ := c.Request.Context().Value(requestKindContextKey).(string)
	if strings.TrimSpace(message) != "" && strings.TrimSpace(phase) != "" {
		message = fmt.Sprintf("phase=%s %s", phase, strings.TrimSpace(message))
	} else if strings.TrimSpace(phase) != "" {
		message = fmt.Sprintf("phase=%s", phase)
	}
	return requestDiagnosticPayload{
		Type:         typ,
		RequestID:    internallogging.GetRequestID(c.Request.Context()),
		Method:       c.Request.Method,
		Path:         requestPath(c.Request),
		RequestKind:  requestKind,
		Model:        model,
		APIKeyID:     stringFromAPIKey(spec, "id"),
		APIKeyLabel:  stringFromAPIKey(spec, "label"),
		Transport:    diagnosticTransport(c.Request),
		LatencyMS:    time.Since(startedAt).Milliseconds(),
		ErrorMessage: message,
	}
}

func (s *relayServer) emitStreamCompleted(c *gin.Context, model string, received int, reason string) {
	if s == nil || s.emitter == nil || c == nil || c.Request == nil {
		return
	}
	spec, _ := c.Request.Context().Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestKind, _ := c.Request.Context().Value(requestKindContextKey).(string)
	s.emitter.emit(requestDiagnosticPayload{
		Type:         "stream_completed",
		RequestID:    internallogging.GetRequestID(c.Request.Context()),
		Method:       c.Request.Method,
		Path:         requestPath(c.Request),
		RequestKind:  requestKind,
		Model:        model,
		APIKeyID:     stringFromAPIKey(spec, "id"),
		APIKeyLabel:  stringFromAPIKey(spec, "label"),
		Transport:    "sse",
		Status:       c.Writer.Status(),
		ErrorMessage: fmt.Sprintf("reason=%s received=%d", reason, received),
	})
}

func requestBodyModel(body []byte) string {
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return ""
	}
	model, _ := payload["model"].(string)
	return strings.TrimSpace(model)
}

func requestBodyStream(body []byte) bool {
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return false
	}
	stream, _ := payload["stream"].(bool)
	return stream
}

func (s *relayServer) bodyWithValidatedModel(c *gin.Context, spec *apiKeySpec, body []byte, model string, stream *bool) ([]byte, string, bool) {
	body, err := injectRequestBodyModelAndStream(body, model, stream)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, err.Error(), "invalid_request")
		return nil, "", false
	}
	nextBody, requestedModel, err := rewriteBodyModel(s.manifest, spec, body)
	if requestedModel != "" && c != nil && c.Request != nil {
		ctx := context.WithValue(c.Request.Context(), requestModelContextKey, requestedModel)
		c.Request = c.Request.WithContext(ctx)
	}
	if err != nil {
		writeAPIError(c, http.StatusNotFound, err.Error(), "model_not_available")
		return nil, "", false
	}
	if nextBody != nil {
		body = nextBody
	}
	canonical := requestBodyModel(body)
	if canonical == "" {
		canonical = strings.TrimSpace(model)
	}
	return body, canonical, true
}

func injectRequestBodyModelAndStream(body []byte, model string, stream *bool) ([]byte, error) {
	var payload map[string]any
	if len(bytes.TrimSpace(body)) == 0 {
		payload = map[string]any{}
	} else if err := json.Unmarshal(body, &payload); err != nil {
		return nil, fmt.Errorf("request body must be a JSON object")
	}
	if payload == nil {
		payload = map[string]any{}
	}
	if trimmed := strings.TrimSpace(model); trimmed != "" {
		payload["model"] = trimmed
	}
	if stream != nil {
		payload["stream"] = *stream
	}
	out, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	return out, nil
}

func (s *relayServer) handleTokenCount(c *gin.Context, targetFormat sdktranslator.Format, model string) {
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
	if strings.TrimSpace(model) == "" {
		model = requestBodyModel(body)
	}
	if strings.TrimSpace(model) == "" {
		writeAPIError(c, http.StatusBadRequest, "model is required", "invalid_request")
		return
	}
	body, _, ok = s.bodyWithValidatedModel(c, spec, body, model, nil)
	if !ok {
		return
	}
	s.handleTokenCountBody(c, body, targetFormat)
}

func (s *relayServer) handleTokenCountBody(c *gin.Context, body []byte, targetFormat sdktranslator.Format) {
	count := estimateRequestTokens(body)
	payload := sdktranslator.TranslateTokenCount(relayContext(c), sdktranslator.FormatCodex, targetFormat, count, body)
	c.Data(http.StatusOK, "application/json", payload)
}

func estimateRequestTokens(body []byte) int64 {
	var payload any
	if err := json.Unmarshal(body, &payload); err != nil {
		return 1
	}
	chars := estimateTextChars(payload)
	if chars <= 0 {
		chars = len(body)
	}
	count := int64(chars / 4)
	if count < 1 {
		count = 1
	}
	return count
}

func estimateTextChars(value any) int {
	switch v := value.(type) {
	case string:
		return len([]rune(v))
	case []any:
		total := 0
		for _, child := range v {
			total += estimateTextChars(child)
		}
		return total
	case map[string]any:
		total := 0
		for key, child := range v {
			switch strings.ToLower(strings.TrimSpace(key)) {
			case "text", "content", "system", "prompt":
				total += estimateTextChars(child)
			default:
				if _, ok := child.(map[string]any); ok {
					total += estimateTextChars(child)
				} else if _, ok := child.([]any); ok {
					total += estimateTextChars(child)
				}
			}
		}
		return total
	default:
		return 0
	}
}

func parseGeminiModelAction(action string) (string, string, bool) {
	raw := strings.Trim(strings.TrimPrefix(strings.TrimSpace(action), "/"), "/")
	if raw == "" {
		return "", "", false
	}
	index := strings.LastIndex(raw, ":")
	if index < 0 {
		return normalizeGeminiModelPath(raw), "", true
	}
	model := normalizeGeminiModelPath(raw[:index])
	method := strings.TrimSpace(raw[index+1:])
	return model, method, model != "" && method != ""
}

func normalizeGeminiModelPath(model string) string {
	model = strings.Trim(strings.TrimSpace(model), "/")
	model = strings.TrimPrefix(model, "models/")
	if index := strings.LastIndex(model, "/models/"); index >= 0 {
		model = model[index+len("/models/"):]
	}
	return strings.TrimSpace(model)
}
