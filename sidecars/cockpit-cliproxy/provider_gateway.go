package main

import (
	"bufio"
	"bytes"
	"context"

	"encoding/json"

	"fmt"
	"io"

	"net/http"
	"net/url"

	"sort"

	"strings"

	"time"

	"github.com/gin-gonic/gin"

	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"

	responsesconverter "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/openai/openai/responses"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"

	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func (s *relayServer) requireAPIKey(c *gin.Context) (*apiKeySpec, bool) {
	if c != nil && c.Request != nil {
		if spec, _ := c.Request.Context().Value(clientAPIKeyContextKey).(*apiKeySpec); spec != nil {
			return spec, true
		}
	}
	writeAPIError(c, http.StatusUnauthorized, "missing or invalid API key", "invalid_api_key")
	if c != nil {
		c.Abort()
	}
	return nil, false
}

func (s *relayServer) handleExecutorRequest(c *gin.Context, sourceFormat sdktranslator.Format, fixedAlt string) {
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
	s.handleExecutorBody(c, spec, body, sourceFormat, fixedAlt)
}

func (s *relayServer) handleExecutorBody(c *gin.Context, spec *apiKeySpec, body []byte, sourceFormat sdktranslator.Format, fixedAlt string) {
	if spec == nil {
		writeAPIError(c, http.StatusUnauthorized, "missing or invalid API key", "invalid_api_key")
		return
	}
	model := requestBodyModel(body)
	if model == "" {
		writeAPIError(c, http.StatusBadRequest, "model is required", "invalid_request")
		return
	}
	if gateway, upstreamModel, routeStatus := resolveModelRouting(spec, model); routeStatus != "none" {
		if routeStatus != "matched" {
			writeAPIError(c, http.StatusNotFound, fmt.Sprintf("model route %s is not available", model), "model_route_not_available")
			return
		}
		if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI) && isGPTImageGenerationModel(upstreamModel) {
			writeAPIError(c, http.StatusBadRequest, "This model is not supported on the Chat Completions endpoint", "invalid_request")
			return
		}
		s.handleProviderGatewayRequest(c, gateway, body, upstreamModel, sourceFormat, fixedAlt)
		return
	}

	canonicalModel := canonicalModelForClientModel(s.manifest, spec, model)
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI) && isGPTImageGenerationModel(canonicalModel) {
		writeAPIError(c, http.StatusBadRequest, "This model is not supported on the Chat Completions endpoint", "invalid_request")
		return
	}

	if spec.ProviderGateway != nil {
		s.handleProviderGatewayRequest(c, spec.ProviderGateway, body, model, sourceFormat, fixedAlt)
		return
	}

	alt := fixedAlt
	if alt == "" {
		alt = requestAlt(c)
	}
	stream := requestBodyStream(body) && fixedAlt != "responses/compact"
	if stream {
		s.handleStream(c, body, model, sourceFormat, alt)
		return
	}
	s.handleNonStream(c, body, model, sourceFormat, alt)
}

func resolveModelRouting(spec *apiKeySpec, clientModel string) (*providerGatewaySpec, string, string) {
	if spec == nil || spec.ModelRouting == nil {
		return nil, "", "none"
	}
	model := stripModelPrefix(clientModel, spec)
	separator := strings.Index(model, "/")
	if separator < 0 {
		return nil, model, "none"
	}
	namespace := strings.ToLower(strings.TrimSpace(model[:separator]))
	upstreamModel := strings.TrimSpace(model[separator+1:])
	if namespace == "" || upstreamModel == "" {
		return nil, "", "missing"
	}
	for i := range spec.ModelRouting.Routes {
		route := &spec.ModelRouting.Routes[i]
		if !strings.EqualFold(route.Namespace, namespace) {
			continue
		}
		if route.ProviderGateway == nil {
			return nil, "", "missing"
		}
		if len(route.ProviderGateway.UpstreamModels) == 0 {
			return nil, "", "missing"
		}
		for _, candidate := range route.ProviderGateway.UpstreamModels {
			if strings.EqualFold(candidate, upstreamModel) {
				return route.ProviderGateway, candidate, "matched"
			}
		}
		return nil, "", "missing"
	}
	return nil, "", "missing"
}

func (s *relayServer) handleProviderGatewayRequest(c *gin.Context, gateway *providerGatewaySpec, body []byte, model string, sourceFormat sdktranslator.Format, fixedAlt string) {
	if gateway == nil {
		writeAPIError(c, http.StatusBadGateway, "provider gateway is not configured", "bad_gateway")
		return
	}
	if fixedAlt == "responses/compact" {
		writeAPIError(c, http.StatusNotFound, "provider gateway does not support responses/compact", "not_found")
		return
	}
	stream := requestBodyStream(body)
	wireAPI := normalizeProviderGatewayWireAPI(gateway.WireAPI)
	upstreamModel := providerGatewayCanonicalModel(gateway, model)
	if strings.TrimSpace(upstreamModel) == "" {
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("model %s is not available for this provider gateway", model), "model_not_available")
		return
	}
	supportsVision := providerGatewayModelSupportsVision(gateway, upstreamModel)
	if wireAPI == "chat_completions" {
		if modelSupportsVision, ok := providerGatewayModelCapabilityOverridesVision(gateway, upstreamModel); ok {
			supportsVision = modelSupportsVision
		}
	}
	if providerGatewayRequestHasVisionInput(body) && !supportsVision {
		visionRoutingModel := providerGatewayVisionRoutingModel(gateway)
		if strings.TrimSpace(visionRoutingModel) == "" {
			omittedBody, omittedCount, err := omitProviderGatewayVisionInput(body, sourceFormat)
			if err != nil || omittedCount == 0 {
				writeAPIError(c, http.StatusBadRequest, fmt.Sprintf("model %s does not support image input", upstreamModel), "unsupported_image_input")
				return
			}
			body = omittedBody
			if s.emitter != nil {
				s.emitter.emit(requestDiagnosticPayload{
					Type:         "provider_gateway_vision_omitted",
					RequestID:    internallogging.GetRequestID(c.Request.Context()),
					Method:       c.Request.Method,
					Path:         requestPath(c.Request),
					RequestKind:  requestKindFromPath(requestPath(c.Request)),
					Model:        upstreamModel,
					Transport:    diagnosticTransport(c.Request),
					ErrorMessage: fmt.Sprintf("omitted %d image input item(s) for text-only model", omittedCount),
				})
			}
		} else {
			originalModel := upstreamModel
			upstreamModel = visionRoutingModel
			if s.emitter != nil {
				s.emitter.emit(requestDiagnosticPayload{
					Type:         "provider_gateway_vision_routed",
					RequestID:    internallogging.GetRequestID(c.Request.Context()),
					Method:       c.Request.Method,
					Path:         requestPath(c.Request),
					RequestKind:  requestKindFromPath(requestPath(c.Request)),
					Model:        upstreamModel,
					Transport:    diagnosticTransport(c.Request),
					ErrorMessage: fmt.Sprintf("routed image input from %s to %s", originalModel, upstreamModel),
				})
			}
		}
	}
	upstreamPath := "/v1/responses"
	upstreamBody := rewriteProviderGatewayBodyModel(body, upstreamModel)
	if wireAPI == "chat_completions" {
		switch {
		case sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse):
			upstreamBody = responsesconverter.ConvertOpenAIResponsesRequestToOpenAIChatCompletions(upstreamModel, body, stream)
		case sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI):
			upstreamBody = rewriteProviderGatewayBodyModel(body, upstreamModel)
		case sourceFormatEqual(sourceFormat, sdktranslator.FormatClaude), sourceFormatEqual(sourceFormat, sdktranslator.FormatGemini):
			upstreamBody = sdktranslator.TranslateRequest(sourceFormat, sdktranslator.FormatOpenAI, upstreamModel, body, stream)
		default:
			writeAPIError(c, http.StatusBadRequest, "provider gateway does not support this request format", "invalid_request")
			return
		}
		upstreamPath = "/v1/chat/completions"
	} else if !sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse) {
		writeAPIError(c, http.StatusBadRequest, "provider gateway responses wire API only accepts responses requests", "invalid_request")
		return
	}

	upstreamURL, err := providerGatewayURL(gateway.BaseURL, upstreamPath)
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
	writeUpstreamHeaders(c.Writer.Header(), resp.Header)
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
		if wireAPI == "chat_completions" {
			switch {
			case sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse):
				s.writeProviderGatewayChatStream(c, resp.Body, upstreamModel, body, upstreamBody)
			case sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI):
				c.Status(http.StatusOK)
				c.Stream(func(w io.Writer) bool {
					_, _ = io.Copy(w, resp.Body)
					return false
				})
			default:
				alt := fixedAlt
				if alt == "" {
					alt = requestAlt(c)
				}
				s.writeProviderGatewayTranslatedChatStream(c, resp.Body, upstreamModel, body, upstreamBody, sourceFormat, alt)
			}
			return
		}
		c.Status(http.StatusOK)
		c.Stream(func(w io.Writer) bool {
			_, _ = io.Copy(w, resp.Body)
			return false
		})
		return
	}

	payload, err := io.ReadAll(resp.Body)
	if err != nil {
		writeAPIError(c, http.StatusBadGateway, err.Error(), "bad_gateway")
		return
	}
	if wireAPI == "chat_completions" {
		switch {
		case sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse):
			payload = responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponsesNonStream(relayContext(c), upstreamModel, body, upstreamBody, payload, nil)
		case sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI):
		default:
			payload = sdktranslator.TranslateNonStream(relayContext(c), sdktranslator.FormatOpenAI, sourceFormat, upstreamModel, body, upstreamBody, payload, nil)
		}
	}
	contentType := resp.Header.Get("Content-Type")
	if contentType == "" || (wireAPI == "chat_completions" && !sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI)) {
		contentType = "application/json"
	}
	c.Data(http.StatusOK, contentType, payload)
}

func rewriteProviderGatewayBodyModel(body []byte, model string) []byte {
	model = strings.TrimSpace(model)
	if model == "" {
		return body
	}
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return body
	}
	payload["model"] = model
	next, err := json.Marshal(payload)
	if err != nil {
		return body
	}
	return next
}

func copyProviderGatewayDiagnosticHeaders(dst http.Header, src http.Header) {
	if dst == nil || src == nil {
		return
	}
	for key, values := range src {
		trimmedKey := strings.TrimSpace(key)
		if trimmedKey == "" {
			continue
		}
		lowerKey := strings.ToLower(trimmedKey)
		if lowerKey != "x-client-request-id" && !strings.HasPrefix(lowerKey, "x-agtools-") {
			continue
		}
		canonicalKey := http.CanonicalHeaderKey(trimmedKey)
		dst.Del(canonicalKey)
		for _, value := range values {
			value = strings.TrimSpace(value)
			if value == "" {
				continue
			}
			dst.Add(canonicalKey, value)
		}
	}
}

func (s *relayServer) writeProviderGatewayChatStream(c *gin.Context, body io.Reader, model string, originalBody []byte, chatBody []byte) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	c.Header("Content-Type", "text/event-stream")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	c.Status(http.StatusOK)
	var state any
	startedAt := time.Now()
	doneSeen := false
	completedSynthesized := false
	completedEventSeen := false
	convertedEventCount := 0
	rawLineCount := 0
	eventCounts := make(map[string]int)
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)
	for scanner.Scan() {
		line := bytes.TrimSpace(scanner.Bytes())
		if len(line) == 0 {
			continue
		}
		rawLineCount++
		if providerGatewayStreamLineIsDone(line) {
			doneSeen = true
		}
		events := responsesconverter.ConvertOpenAIChatCompletionsResponseToOpenAIResponses(relayContext(c), model, originalBody, chatBody, line, &state)
		for _, event := range events {
			if len(event) == 0 {
				continue
			}
			eventName := providerGatewayResponseSSEEventName(event)
			if eventName != "" {
				eventCounts[eventName]++
				if eventName == "response.completed" {
					completedEventSeen = true
				}
			}
			convertedEventCount++
			if _, err := c.Writer.Write(providerGatewaySSEFrame(event)); err != nil {
				return
			}
			flusher.Flush()
		}
	}
	if err := scanner.Err(); err != nil {
		s.emitExecutorDiagnostic(c, "provider_gateway_stream_scan_failed", model, "provider_gateway_chat_stream", startedAt, err.Error())
		writeStreamTerminalErrorForFormat(c, err, sdktranslator.FormatOpenAIResponse)
		flusher.Flush()
		return
	}
	if !doneSeen {
		events := responsesconverter.CompleteOpenAIChatCompletionsResponseToOpenAIResponses(relayContext(c), chatBody, &state)
		for _, event := range events {
			if len(event) == 0 {
				continue
			}
			completedSynthesized = true
			eventName := providerGatewayResponseSSEEventName(event)
			if eventName != "" {
				eventCounts[eventName]++
				if eventName == "response.completed" {
					completedEventSeen = true
				}
			}
			convertedEventCount++
			if _, err := c.Writer.Write(providerGatewaySSEFrame(event)); err != nil {
				s.emitExecutorDiagnostic(c, "provider_gateway_stream_write_failed", model, "provider_gateway_chat_stream", startedAt, err.Error())
				return
			}
			flusher.Flush()
		}
	}
	s.emitExecutorDiagnostic(
		c,
		"provider_gateway_stream_completed",
		model,
		"provider_gateway_chat_stream",
		startedAt,
		fmt.Sprintf(
			"done_seen=%t completed_event_seen=%t completed_synthesized=%t raw_line_count=%d converted_event_count=%d event_counts=%s",
			doneSeen,
			completedEventSeen,
			completedSynthesized,
			rawLineCount,
			convertedEventCount,
			providerGatewayFormatEventCounts(eventCounts),
		),
	)
}

func (s *relayServer) writeProviderGatewayTranslatedChatStream(c *gin.Context, body io.Reader, model string, originalBody []byte, chatBody []byte, targetFormat sdktranslator.Format, alt string) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	c.Header("Content-Type", "text/event-stream")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	c.Status(http.StatusOK)

	var state any
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)
	for scanner.Scan() {
		line := bytes.TrimSpace(scanner.Bytes())
		if len(line) == 0 {
			continue
		}
		outputs := sdktranslator.TranslateStream(relayContext(c), sdktranslator.FormatOpenAI, targetFormat, model, originalBody, chatBody, line, &state)
		for _, output := range outputs {
			if len(bytes.TrimSpace(output)) == 0 {
				continue
			}
			if sourceFormatEqual(targetFormat, sdktranslator.FormatGemini) && alt == "" {
				output = frameOpenAIStreamChunk(output)
			}
			if _, err := c.Writer.Write(output); err != nil {
				return
			}
			flusher.Flush()
		}
	}
	if err := scanner.Err(); err != nil {
		writeStreamTerminalErrorForFormat(c, err, targetFormat)
		flusher.Flush()
	}
}

func providerGatewaySSEFrame(event []byte) []byte {
	if len(event) == 0 || bytes.HasSuffix(event, []byte("\n\n")) || bytes.HasSuffix(event, []byte("\r\n\r\n")) {
		return event
	}
	out := make([]byte, 0, len(event)+2)
	out = append(out, event...)
	if bytes.HasSuffix(event, []byte("\n")) {
		out = append(out, '\n')
	} else {
		out = append(out, '\n', '\n')
	}
	return out
}

func providerGatewayResponseSSEEventName(event []byte) string {
	for _, line := range bytes.Split(event, []byte("\n")) {
		line = bytes.TrimSpace(line)
		if !bytes.HasPrefix(line, []byte("event:")) {
			continue
		}
		return strings.TrimSpace(string(bytes.TrimSpace(line[len("event:"):])))
	}
	return ""
}

func providerGatewayFormatEventCounts(counts map[string]int) string {
	if len(counts) == 0 {
		return "none"
	}
	names := make([]string, 0, len(counts))
	for name := range counts {
		names = append(names, name)
	}
	sort.Strings(names)
	parts := make([]string, 0, len(names))
	for _, name := range names {
		parts = append(parts, fmt.Sprintf("%s:%d", name, counts[name]))
	}
	return strings.Join(parts, ",")
}

func providerGatewayStreamLineIsDone(line []byte) bool {
	line = bytes.TrimSpace(line)
	if bytes.HasPrefix(line, []byte("data:")) {
		line = bytes.TrimSpace(line[len("data:"):])
	}
	return bytes.Equal(line, []byte("[DONE]"))
}

func providerGatewayURL(baseURL string, path string) (string, error) {
	trimmedBase := strings.TrimRight(strings.TrimSpace(baseURL), "/")
	if trimmedBase == "" {
		return "", fmt.Errorf("provider gateway base URL is empty")
	}
	parsed, err := url.Parse(trimmedBase)
	if err != nil || parsed.Scheme == "" || parsed.Host == "" {
		return "", fmt.Errorf("provider gateway base URL is invalid")
	}
	cleanPath := "/" + strings.TrimLeft(path, "/")
	basePath := strings.TrimRight(parsed.Path, "/")
	endpointPath := providerGatewayEndpointPath(cleanPath)
	if strings.HasSuffix(basePath, strings.TrimSuffix(cleanPath, "/")) {
		parsed.Path = basePath
	} else if endpointPath != "" && strings.HasSuffix(basePath, strings.TrimSuffix(endpointPath, "/")) {
		parsed.Path = basePath
	} else if endpointPath != "" && providerGatewayBasePathHasVersionSegment(basePath) {
		parsed.Path = basePath + endpointPath
	} else {
		parsed.Path = basePath + cleanPath
	}
	parsed.RawQuery = ""
	parsed.Fragment = ""
	return parsed.String(), nil
}

func providerGatewayEndpointPath(path string) string {
	cleanPath := "/" + strings.TrimLeft(strings.TrimSpace(path), "/")
	if strings.HasPrefix(cleanPath, "/v1/") {
		return strings.TrimPrefix(cleanPath, "/v1")
	}
	return ""
}

func providerGatewayBasePathHasVersionSegment(basePath string) bool {
	for _, segment := range strings.Split(strings.Trim(basePath, "/"), "/") {
		if providerGatewayPathSegmentIsVersion(segment) {
			return true
		}
	}
	return false
}

func providerGatewayPathSegmentIsVersion(segment string) bool {
	segment = strings.TrimSpace(segment)
	if len(segment) < 2 || (segment[0] != 'v' && segment[0] != 'V') {
		return false
	}
	hasDigit := false
	for i := 1; i < len(segment); i++ {
		ch := segment[i]
		if ch >= '0' && ch <= '9' {
			hasDigit = true
			continue
		}
		if !hasDigit {
			return false
		}
		if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || ch == '-' || ch == '_' || ch == '.' {
			continue
		}
		return false
	}
	return hasDigit
}

func (s *relayServer) handleNonStream(c *gin.Context, body []byte, model string, sourceFormat sdktranslator.Format, alt string) {
	req, opts := buildExecutorRequest(c, body, model, sourceFormat, alt, false)
	startedAt := time.Now()
	s.emitExecutorDiagnostic(c, "executor_started", model, "execute", startedAt, "")
	stopWaitLogger := s.startExecutorWaitLogger(c, model, "execute", startedAt)
	resp, err := s.runtime.Execute(relayContext(c), []string{"codex"}, req, opts)
	stopWaitLogger()
	if err != nil {
		s.emitExecutorDiagnostic(c, "executor_failed", model, "execute", startedAt, err.Error())
		s.writeExecutorError(c, err)
		return
	}
	s.emitExecutorDiagnostic(c, "executor_completed", model, "execute", startedAt, "")
	writeUpstreamHeaders(c.Writer.Header(), resp.Headers)
	contentType := resp.Headers.Get("Content-Type")
	if contentType == "" {
		contentType = "application/json"
	}
	c.Data(http.StatusOK, contentType, resp.Payload)
}

func (s *relayServer) handleStream(c *gin.Context, body []byte, model string, sourceFormat sdktranslator.Format, alt string) {
	req, opts := buildExecutorRequest(c, body, model, sourceFormat, alt, true)
	startedAt := time.Now()
	timeouts := s.streamTimeoutsForRequest(c.Request, body, model)
	immediateSSE := s.manifest != nil && s.manifest.ImmediateSSEResponse
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
	s.emitExecutorDiagnostic(c, "executor_started", model, "execute_stream", startedAt, "")
	stopWaitLogger := s.startExecutorWaitLogger(c, model, "execute_stream", startedAt)
	streamCtx, cancelStream := context.WithCancel(relayContext(c))
	defer cancelStream()
	result, err := s.executeStreamWithOpenTimeout(c, streamCtx, []string{"codex"}, req, opts, model, startedAt, timeouts.open)
	stopWaitLogger()
	if err != nil {
		s.emitExecutorDiagnostic(c, "executor_failed", model, "execute_stream", startedAt, err.Error())
		if immediateSSE {
			writeStreamTerminalErrorForFormat(c, err, sourceFormat)
			immediateFlusher.Flush()
			return
		}
		s.writeExecutorError(c, err)
		return
	}
	if result == nil || result.Chunks == nil {
		s.emitExecutorDiagnostic(c, "executor_failed", model, "execute_stream", startedAt, "upstream stream is unavailable")
		if immediateSSE {
			writeStreamTerminalErrorForFormat(c, relayStatusError{status: http.StatusBadGateway, message: "upstream stream is unavailable"}, sourceFormat)
			immediateFlusher.Flush()
		} else {
			writeAPIError(c, http.StatusBadGateway, "upstream stream is unavailable", "bad_gateway")
		}
		return
	}
	s.emitExecutorDiagnostic(c, "stream_opened", model, "execute_stream", startedAt, "")
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}

	if !immediateSSE {
		setEventStreamHeaders(c.Writer.Header())
		writeUpstreamHeaders(c.Writer.Header(), result.Headers)
		c.Status(http.StatusOK)
	}

	framer := newRelayStreamFramer(sourceFormat, requestPath(c.Request))
	keepAlive := streamKeepAliveInterval(s.cfg)
	var ticker *time.Ticker
	var tickerC <-chan time.Time
	if keepAlive > 0 {
		ticker = time.NewTicker(keepAlive)
		tickerC = ticker.C
		defer ticker.Stop()
	}

	received := 0
	endReason := "done"
	firstChunkLogged := false
	idleTimer := time.NewTimer(timeouts.idle)
	defer idleTimer.Stop()
	defer func() {
		s.emitStreamCompleted(c, model, received, endReason)
	}()

	for {
		select {
		case <-idleTimer.C:
			cancelStream()
			endReason = "stream_idle_timeout"
			err := relayTimeoutError{phase: "stream_idle", timeout: timeouts.idle}
			s.emitExecutorDiagnostic(c, "stream_idle_timeout", model, "stream_loop", startedAt, err.Error())
			writeStreamTerminalErrorForFormat(c, err, sourceFormat)
			flusher.Flush()
			return
		case <-c.Request.Context().Done():
			cancelStream()
			endReason = "client_gone"
			s.emitExecutorDiagnostic(c, "stream_client_gone", model, "stream_loop", startedAt, c.Request.Context().Err().Error())
			return
		case <-tickerC:
			if _, err := c.Writer.Write([]byte(": keep-alive\n\n")); err != nil {
				endReason = "write_failed"
				s.emitExecutorDiagnostic(c, "stream_write_failed", model, "stream_loop", startedAt, err.Error())
				return
			}
			if received == 0 {
				s.emitExecutorDiagnostic(c, "stream_keepalive", model, "stream_loop", startedAt, "received=0")
			}
			flusher.Flush()
		case chunk, ok := <-result.Chunks:
			if !idleTimer.Stop() {
				select {
				case <-idleTimer.C:
				default:
				}
			}
			idleTimer.Reset(timeouts.idle)
			if !ok {
				if err := framer.Close(c.Writer); err != nil {
					endReason = "write_failed"
					s.emitExecutorDiagnostic(c, "stream_write_failed", model, "stream_loop", startedAt, err.Error())
					return
				}
				flusher.Flush()
				return
			}
			if chunk.Err != nil {
				endReason = "stream_error"
				s.emitExecutorDiagnostic(c, "stream_error", model, "stream_loop", startedAt, chunk.Err.Error())
				writeStreamTerminalErrorForFormat(c, chunk.Err, sourceFormat)
				flusher.Flush()
				return
			}
			if len(chunk.Payload) == 0 {
				continue
			}
			if !firstChunkLogged {
				firstChunkLogged = true
				s.emitExecutorDiagnostic(c, "stream_first_chunk", model, "stream_loop", startedAt, fmt.Sprintf("bytes=%d", len(chunk.Payload)))
			}
			if err := framer.Write(c.Writer, chunk.Payload); err != nil {
				endReason = "write_failed"
				s.emitExecutorDiagnostic(c, "stream_write_failed", model, "stream_loop", startedAt, err.Error())
				return
			}
			received++
			flusher.Flush()
		}
	}
}

type executeStreamResult struct {
	result *cliproxyexecutor.StreamResult
	err    error
}
