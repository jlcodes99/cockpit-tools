package main

import (
	"bytes"
	"context"

	"encoding/json"
	"errors"

	"fmt"
	"io"

	"net/http"
	"net/url"

	"strings"

	"time"

	"github.com/gin-gonic/gin"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/util"

	sdkhandlers "github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"

	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func durationFromConfigMillis(value int, fallback time.Duration) time.Duration {
	if value <= 0 {
		return fallback
	}
	return time.Duration(value) * time.Millisecond
}

func (s *relayServer) streamOpenMaxAttempts() int {
	attempts := streamOpenMaxAttempts
	if s != nil && s.cfg != nil && s.cfg.Streaming.StreamOpenMaxAttempts > 0 {
		attempts = s.cfg.Streaming.StreamOpenMaxAttempts
	}
	if attempts < 1 {
		return 1
	}
	if attempts > 3 {
		return 3
	}
	return attempts
}

func (s *relayServer) streamTimeoutsForRequest(r *http.Request, body []byte, model string) streamTimeoutProfile {
	profile := streamTimeoutProfile{
		open: durationFromConfigMillis(0, streamOpenTimeout),
		idle: durationFromConfigMillis(0, streamIdleTimeout),
	}
	if s != nil && s.cfg != nil {
		profile.open = durationFromConfigMillis(s.cfg.Streaming.StreamOpenTimeoutMS, profile.open)
		profile.idle = durationFromConfigMillis(s.cfg.Streaming.StreamIdleTimeoutMS, profile.idle)
	}
	if !isImageGenerationRequest(r, body, model) {
		return profile
	}
	profile.open = imageStreamOpenTimeout
	profile.idle = imageStreamIdleTimeout
	if s != nil && s.cfg != nil {
		profile.open = durationFromConfigMillis(s.cfg.Streaming.ImageStreamOpenTimeoutMS, profile.open)
		profile.idle = durationFromConfigMillis(s.cfg.Streaming.ImageStreamIdleTimeoutMS, profile.idle)
	}
	return profile
}

func isImageGenerationRequest(r *http.Request, body []byte, model string) bool {
	if modelBase(model) == "gpt-image-2" {
		return true
	}
	if r != nil && r.URL != nil {
		path := strings.ToLower(strings.TrimSpace(r.URL.Path))
		if strings.Contains(path, "/images/generations") || strings.Contains(path, "/images/edits") {
			return true
		}
	}
	return jsonContainsImageGenerationTool(body)
}

func modelBase(model string) string {
	model = strings.ToLower(strings.TrimSpace(model))
	if idx := strings.LastIndex(model, "/"); idx >= 0 && idx < len(model)-1 {
		model = strings.TrimSpace(model[idx+1:])
	}
	return model
}

func isGPTImageGenerationModel(model string) bool {
	return strings.HasPrefix(modelBase(model), "gpt-image-")
}

func jsonContainsImageGenerationTool(body []byte) bool {
	if len(bytes.TrimSpace(body)) == 0 {
		return false
	}
	var payload any
	if err := json.Unmarshal(body, &payload); err != nil {
		return false
	}
	return valueContainsImageGenerationTool(payload)
}

func valueContainsImageGenerationTool(value any) bool {
	switch v := value.(type) {
	case map[string]any:
		if typ, ok := v["type"].(string); ok && strings.EqualFold(strings.TrimSpace(typ), "image_generation") {
			return true
		}
		for _, child := range v {
			if valueContainsImageGenerationTool(child) {
				return true
			}
		}
	case []any:
		for _, child := range v {
			if valueContainsImageGenerationTool(child) {
				return true
			}
		}
	}
	return false
}

func requestAlt(c *gin.Context) string {
	if c == nil {
		return ""
	}
	alt := strings.TrimSpace(c.Query("alt"))
	if alt == "" {
		alt = strings.TrimSpace(c.Query("$alt"))
	}
	if alt == "sse" {
		return ""
	}
	return alt
}

func relayContext(c *gin.Context) context.Context {
	if c == nil || c.Request == nil {
		return context.Background()
	}
	endpoint := c.Request.Method
	if c.Request.URL != nil {
		endpoint += " " + c.Request.URL.Path
	}
	ctx := internallogging.WithEndpoint(c.Request.Context(), endpoint)
	return context.WithValue(ctx, "gin", c)
}

func buildExecutorRequest(c *gin.Context, body []byte, model string, sourceFormat sdktranslator.Format, alt string, stream bool) (cliproxyexecutor.Request, cliproxyexecutor.Options) {
	metadata := map[string]any{
		cliproxyexecutor.RequestedModelMetadataKey: model,
	}
	if c != nil && c.Request != nil && c.Request.URL != nil {
		metadata[cliproxyexecutor.RequestPathMetadataKey] = c.Request.URL.Path
	}
	headers := http.Header{}
	query := url.Values{}
	if c != nil && c.Request != nil {
		headers = c.Request.Header.Clone()
		if c.Request.URL != nil && c.Request.URL.Query() != nil {
			for key, values := range c.Request.URL.Query() {
				query[key] = append([]string(nil), values...)
			}
		}
	}
	req := cliproxyexecutor.Request{
		Model:    model,
		Payload:  body,
		Format:   sourceFormat,
		Metadata: metadata,
	}
	opts := cliproxyexecutor.Options{
		Stream:          stream,
		Alt:             alt,
		Headers:         headers,
		Query:           query,
		OriginalRequest: body,
		SourceFormat:    sourceFormat,
		Metadata:        metadata,
	}
	return req, opts
}

func writeAPIError(c *gin.Context, status int, message, code string) {
	if status <= 0 {
		status = http.StatusInternalServerError
	}
	if message == "" {
		message = http.StatusText(status)
	}
	if code == "" {
		code = "error"
	}
	c.JSON(status, gin.H{
		"error": gin.H{
			"message": message,
			"type":    "invalid_request_error",
			"code":    code,
		},
	})
}

func (s *relayServer) writeExecutorError(c *gin.Context, err error) {
	status := statusCodeFromError(err)
	code := "upstream_error"
	if status == http.StatusUnauthorized || status == http.StatusForbidden {
		code = "auth_failed"
	} else if status == http.StatusTooManyRequests {
		code = "rate_limited"
	} else if status == http.StatusNotFound {
		code = "not_found"
	} else if status == http.StatusGatewayTimeout || status == http.StatusRequestTimeout {
		code = errorCategory(status, errorMessage(err), false)
	}
	if err != nil {
		_ = c.Error(err)
	}
	if shouldThrottleDownstreamExecutorError(status) {
		var ctx context.Context = context.Background()
		if c != nil && c.Request != nil {
			ctx = c.Request.Context()
		}
		if waitErr := util.SleepContext(ctx, s.downstreamExecutorErrorDelay()); waitErr != nil {
			return
		}
	}
	writeAPIError(c, status, errorMessage(err), code)
}

func shouldThrottleDownstreamExecutorError(status int) bool {
	if status == http.StatusUnauthorized || status == http.StatusPaymentRequired ||
		status == http.StatusForbidden || status == http.StatusRequestTimeout ||
		status == http.StatusTooManyRequests {
		return true
	}
	return status >= http.StatusInternalServerError
}

func (s *relayServer) downstreamExecutorErrorDelay() time.Duration {
	if s == nil || s.cfg == nil {
		return 0
	}
	base := time.Duration(s.cfg.Streaming.BootstrapRetryBaseDelayMS) * time.Millisecond
	max := time.Duration(s.cfg.Streaming.BootstrapRetryMaxDelayMS) * time.Millisecond
	return util.BackoffDelay(1, base, max)
}

func statusCodeFromError(err error) int {
	status := http.StatusBadGateway
	if err == nil {
		return status
	}
	var statusErr interface{ StatusCode() int }
	if errors.As(err, &statusErr) {
		if code := statusErr.StatusCode(); code > 0 {
			status = code
		}
	}
	return status
}

func errorMessage(err error) string {
	if err == nil {
		return ""
	}
	message := strings.TrimSpace(err.Error())
	if message == "" {
		return "upstream error"
	}
	return message
}

func setEventStreamHeaders(headers http.Header) {
	headers.Set("Content-Type", "text/event-stream")
	headers.Set("Cache-Control", "no-cache")
	headers.Set("Connection", "keep-alive")
	headers.Set("X-Accel-Buffering", "no")
}

func writeUpstreamHeaders(dst http.Header, src http.Header) {
	if src == nil {
		return
	}
	connectionScoped := connectionScopedResponseHeaders(src)
	for key, values := range src {
		canonicalKey := http.CanonicalHeaderKey(key)
		if shouldSkipResponseHeader(canonicalKey, connectionScoped) {
			continue
		}
		if dst.Get(canonicalKey) != "" {
			continue
		}
		for _, value := range values {
			dst.Add(canonicalKey, value)
		}
	}
}

func connectionScopedResponseHeaders(headers http.Header) map[string]struct{} {
	scoped := make(map[string]struct{})
	if headers == nil {
		return scoped
	}
	for _, rawValue := range headers.Values("Connection") {
		for _, token := range strings.Split(rawValue, ",") {
			name := strings.TrimSpace(token)
			if name == "" {
				continue
			}
			scoped[http.CanonicalHeaderKey(name)] = struct{}{}
		}
	}
	return scoped
}

func shouldSkipResponseHeader(key string, connectionScoped map[string]struct{}) bool {
	canonicalKey := http.CanonicalHeaderKey(strings.TrimSpace(key))
	if canonicalKey == "" {
		return true
	}
	if _, scoped := connectionScoped[canonicalKey]; scoped {
		return true
	}
	lowerKey := strings.ToLower(canonicalKey)
	for _, prefix := range []string{
		"x-litellm-",
		"helicone-",
		"x-portkey-",
		"cf-aig-",
		"x-kong-",
		"x-bt-",
	} {
		if strings.HasPrefix(lowerKey, prefix) {
			return true
		}
	}
	switch lowerKey {
	case "content-length", "content-encoding", "transfer-encoding", "connection",
		"keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer",
		"upgrade", "set-cookie":
		return true
	default:
		return false
	}
}

func streamKeepAliveInterval(cfg *config.Config) time.Duration {
	seconds := defaultStreamKeepAliveSeconds
	if cfg != nil && cfg.Streaming.KeepAliveSeconds > 0 {
		seconds = cfg.Streaming.KeepAliveSeconds
	}
	if seconds <= 0 {
		return 0
	}
	return time.Duration(seconds) * time.Second
}

func writeStreamTerminalError(c *gin.Context, err error) {
	status := statusCodeFromError(err)
	payload, marshalErr := json.Marshal(gin.H{
		"error": gin.H{
			"message": errorMessage(err),
			"type":    "upstream_error",
			"code":    status,
		},
	})
	if marshalErr != nil {
		return
	}
	_, _ = fmt.Fprintf(c.Writer, "data: %s\n\n", string(payload))
}

func writeStreamTerminalErrorForFormat(c *gin.Context, err error, sourceFormat sdktranslator.Format) {
	if c == nil {
		return
	}
	if !sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse) {
		writeStreamTerminalError(c, err)
		return
	}
	status := statusCodeFromError(err)
	eventName, payload := sdkhandlers.BuildOpenAIResponsesStreamTerminalEvent(status, err, 0)
	_, _ = fmt.Fprintf(c.Writer, "event: %s\ndata: %s\n\n", eventName, string(payload))
}

type relayStreamFrameMode int

const (
	relayStreamFrameRaw relayStreamFrameMode = iota
	relayStreamFrameOpenAI
	relayStreamFrameResponses
)

type relayStreamFramer struct {
	mode      relayStreamFrameMode
	responses responsesSSEFramer
}

func newRelayStreamFramer(sourceFormat sdktranslator.Format, path string) *relayStreamFramer {
	mode := relayStreamFrameRaw
	switch sourceFormat {
	case sdktranslator.FormatOpenAIResponse:
		mode = relayStreamFrameResponses
	case sdktranslator.FormatOpenAI, sdktranslator.FormatGemini:
		mode = relayStreamFrameOpenAI
	}
	if strings.HasPrefix(strings.Split(path, "?")[0], "/v1/responses") {
		mode = relayStreamFrameResponses
	}
	return &relayStreamFramer{mode: mode}
}

func (f *relayStreamFramer) Write(w io.Writer, chunk []byte) error {
	if len(chunk) == 0 {
		return nil
	}
	switch f.mode {
	case relayStreamFrameResponses:
		return f.responses.WriteChunk(w, normalizeResponsesInputChunk(f.responses.HasPending(), chunk))
	case relayStreamFrameOpenAI:
		_, err := w.Write(frameOpenAIStreamChunk(chunk))
		return err
	default:
		_, err := w.Write(chunk)
		return err
	}
}

func (f *relayStreamFramer) Close(w io.Writer) error {
	if f.mode == relayStreamFrameResponses {
		return f.responses.Flush(w)
	}
	return nil
}

func frameOpenAIStreamChunk(chunk []byte) []byte {
	trimmed := bytes.TrimSpace(chunk)
	if len(trimmed) == 0 {
		return nil
	}
	if bytes.HasPrefix(trimmed, []byte("data:")) {
		return ensureSSETrailingBlankLine(chunk)
	}
	if bytes.HasPrefix(trimmed, []byte("[DONE]")) {
		return []byte("data: [DONE]\n\n")
	}
	out := make([]byte, 0, len(trimmed)+8)
	out = append(out, []byte("data: ")...)
	out = append(out, trimmed...)
	out = append(out, '\n', '\n')
	return out
}

func normalizeResponsesInputChunk(hasPending bool, chunk []byte) []byte {
	if hasPending {
		return chunk
	}
	trimmed := bytes.TrimSpace(chunk)
	if len(trimmed) == 0 {
		return nil
	}
	if isSSEFieldChunk(trimmed) || chunk[0] == '\n' || chunk[0] == '\r' {
		return chunk
	}
	if bytes.HasPrefix(trimmed, []byte("[DONE]")) {
		return []byte("data: [DONE]\n\n")
	}
	if bytes.HasPrefix(trimmed, []byte("{")) || bytes.HasPrefix(trimmed, []byte("[")) {
		out := make([]byte, 0, len(trimmed)+6)
		out = append(out, []byte("data: ")...)
		out = append(out, trimmed...)
		return out
	}
	return chunk
}

func isSSEFieldChunk(chunk []byte) bool {
	for _, prefix := range [][]byte{
		[]byte("data:"),
		[]byte("event:"),
		[]byte("id:"),
		[]byte("retry:"),
		[]byte(":"),
	} {
		if bytes.HasPrefix(chunk, prefix) {
			return true
		}
	}
	return false
}

func ensureSSETrailingBlankLine(chunk []byte) []byte {
	if bytes.HasSuffix(chunk, []byte("\n\n")) || bytes.HasSuffix(chunk, []byte("\r\n\r\n")) {
		return chunk
	}
	out := make([]byte, 0, len(chunk)+2)
	out = append(out, chunk...)
	if bytes.HasSuffix(out, []byte("\r\n")) || bytes.HasSuffix(out, []byte("\n")) {
		out = append(out, '\n')
	} else {
		out = append(out, '\n', '\n')
	}
	return out
}

type responsesSSEFramer struct {
	pending []byte
}

func (f *responsesSSEFramer) HasPending() bool {
	return len(f.pending) > 0
}

func (f *responsesSSEFramer) WriteChunk(w io.Writer, chunk []byte) error {
	if len(chunk) == 0 {
		return nil
	}
	if responsesSSENeedsLineBreak(f.pending, chunk) {
		f.pending = append(f.pending, '\n')
	}
	f.pending = append(f.pending, chunk...)
	for {
		frameLen := responsesSSEFrameLen(f.pending)
		if frameLen == 0 {
			break
		}
		if err := writeResponsesSSEFrame(w, f.pending[:frameLen]); err != nil {
			return err
		}
		copy(f.pending, f.pending[frameLen:])
		f.pending = f.pending[:len(f.pending)-frameLen]
	}
	if len(bytes.TrimSpace(f.pending)) == 0 {
		f.pending = f.pending[:0]
		return nil
	}
	if !responsesSSECanEmitWithoutDelimiter(f.pending) {
		return nil
	}
	if err := writeResponsesSSEFrame(w, f.pending); err != nil {
		return err
	}
	f.pending = f.pending[:0]
	return nil
}

func (f *responsesSSEFramer) Flush(w io.Writer) error {
	if len(f.pending) == 0 {
		return nil
	}
	if len(bytes.TrimSpace(f.pending)) == 0 {
		f.pending = f.pending[:0]
		return nil
	}
	if !responsesSSECanEmitWithoutDelimiter(f.pending) {
		f.pending = f.pending[:0]
		return nil
	}
	if err := writeResponsesSSEFrame(w, f.pending); err != nil {
		return err
	}
	f.pending = f.pending[:0]
	return nil
}

const (
	maxResponsesConcatenatedJSONDocuments = 16
	maxResponsesConcatenatedJSONBytes     = 16 * 1024 * 1024
)

func splitResponsesConcatenatedJSONDocuments(payload []byte) ([][]byte, bool) {
	payload = bytes.TrimSpace(payload)
	if len(payload) == 0 || len(payload) > maxResponsesConcatenatedJSONBytes || json.Valid(payload) {
		return nil, false
	}

	decoder := json.NewDecoder(bytes.NewReader(payload))
	documents := make([][]byte, 0, 2)
	for {
		var raw json.RawMessage
		err := decoder.Decode(&raw)
		if err != nil {
			if errors.Is(err, io.EOF) && len(documents) > 1 {
				return documents, true
			}
			return nil, false
		}

		raw = bytes.TrimSpace(raw)
		var envelope struct {
			Type string `json:"type"`
		}
		if err := json.Unmarshal(raw, &envelope); err != nil {
			return nil, false
		}
		eventType := strings.TrimSpace(envelope.Type)
		if eventType == "" || strings.ContainsAny(eventType, "\r\n") {
			return nil, false
		}
		if len(documents) == maxResponsesConcatenatedJSONDocuments {
			return nil, false
		}
		documents = append(documents, bytes.Clone(raw))
	}
}

func writeResponsesSSEFrame(w io.Writer, chunk []byte) error {
	payload, ok := responsesSSEDataPayload(chunk)
	if !ok {
		return writeResponsesSSEChunk(w, chunk)
	}
	documents, repaired := splitResponsesConcatenatedJSONDocuments(payload)
	if !repaired {
		return writeResponsesSSEChunk(w, chunk)
	}

	for _, document := range documents {
		var envelope struct {
			Type string `json:"type"`
		}
		if err := json.Unmarshal(document, &envelope); err != nil {
			return err
		}
		frame := make([]byte, 0, len(document)+len(envelope.Type)+17)
		frame = append(frame, "event: "...)
		frame = append(frame, strings.TrimSpace(envelope.Type)...)
		frame = append(frame, '\n')
		frame = append(frame, "data: "...)
		frame = append(frame, document...)
		frame = append(frame, '\n', '\n')
		if err := writeResponsesSSEChunk(w, frame); err != nil {
			return err
		}
	}
	return nil
}

func writeResponsesSSEChunk(w io.Writer, chunk []byte) error {
	if w == nil || len(chunk) == 0 {
		return nil
	}
	if _, err := w.Write(chunk); err != nil {
		return err
	}
	if bytes.HasSuffix(chunk, []byte("\n\n")) || bytes.HasSuffix(chunk, []byte("\r\n\r\n")) {
		return nil
	}
	suffix := []byte("\n\n")
	if bytes.HasSuffix(chunk, []byte("\r\n")) {
		suffix = []byte("\r\n")
	} else if bytes.HasSuffix(chunk, []byte("\n")) {
		suffix = []byte("\n")
	}
	_, err := w.Write(suffix)
	return err
}

func responsesSSEFrameLen(chunk []byte) int {
	if len(chunk) == 0 {
		return 0
	}
	lf := bytes.Index(chunk, []byte("\n\n"))
	crlf := bytes.Index(chunk, []byte("\r\n\r\n"))
	switch {
	case lf < 0:
		if crlf < 0 {
			return 0
		}
		return crlf + 4
	case crlf < 0:
		return lf + 2
	case lf < crlf:
		return lf + 2
	default:
		return crlf + 4
	}
}

func responsesSSENeedsLineBreak(pending []byte, chunk []byte) bool {
	if len(pending) == 0 || len(chunk) == 0 {
		return false
	}
	if bytes.HasSuffix(pending, []byte("\n")) || bytes.HasSuffix(pending, []byte("\r")) {
		return false
	}
	trimmed := bytes.TrimSpace(chunk)
	if len(trimmed) == 0 {
		return false
	}
	return isSSEFieldChunk(trimmed)
}

func responsesSSECanEmitWithoutDelimiter(chunk []byte) bool {
	trimmed := bytes.TrimSpace(chunk)
	if len(trimmed) == 0 {
		return false
	}
	if responsesSSENeedsMoreData(trimmed) {
		return false
	}
	if payload, ok := responsesSSEDataPayload(trimmed); ok {
		if bytes.Equal(bytes.TrimSpace(payload), []byte("[DONE]")) {
			return true
		}
		if json.Valid(payload) {
			return true
		}
		_, repaired := splitResponsesConcatenatedJSONDocuments(payload)
		return repaired
	}
	return isSSEFieldChunk(trimmed)
}

func responsesSSEDataPayload(chunk []byte) ([]byte, bool) {
	trimmed := bytes.TrimSpace(chunk)
	if len(trimmed) == 0 {
		return nil, false
	}
	if bytes.HasPrefix(trimmed, []byte("{")) || bytes.HasPrefix(trimmed, []byte("[")) {
		return trimmed, true
	}

	lines := bytes.Split(bytes.ReplaceAll(trimmed, []byte("\r\n"), []byte("\n")), []byte("\n"))
	dataLines := make([][]byte, 0, 1)
	for _, line := range lines {
		line = bytes.TrimSpace(line)
		if !bytes.HasPrefix(line, []byte("data:")) {
			continue
		}
		value := line[len("data:"):]
		if len(value) > 0 && value[0] == ' ' {
			value = value[1:]
		}
		dataLines = append(dataLines, value)
	}
	if len(dataLines) == 0 {
		return nil, false
	}
	return bytes.Join(dataLines, []byte("\n")), true
}

func responsesSSENeedsMoreData(chunk []byte) bool {
	trimmed := bytes.TrimSpace(chunk)
	if len(trimmed) == 0 {
		return false
	}
	return responsesSSEHasField(trimmed, []byte("event:")) && !responsesSSEHasField(trimmed, []byte("data:"))
}

func responsesSSEHasField(chunk []byte, prefix []byte) bool {
	s := chunk
	for len(s) > 0 {
		line := s
		if i := bytes.IndexByte(s, '\n'); i >= 0 {
			line = s[:i]
			s = s[i+1:]
		} else {
			s = nil
		}
		line = bytes.TrimSpace(line)
		if bytes.HasPrefix(line, prefix) {
			return true
		}
	}
	return false
}
