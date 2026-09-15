package main

import (
	"bufio"
	"bytes"
	"context"
	"crypto/rand"

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

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	cliproxysession "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/session"

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
	if wireAPI == "responses" {
		body, _ = providerGatewayRestoreReasoningText(gateway, body)
		normalized, placeholders, synthesized, relocated, orderErr := providerGatewayNormalizeToolCallPairing(gateway, body)
		if orderErr != nil {
			if s.emitter != nil {
				s.emitter.emit(requestDiagnosticPayload{
					Type:         "provider_gateway_tool_call_pairing_error",
					RequestID:    internallogging.GetRequestID(c.Request.Context()),
					Method:       c.Request.Method,
					Path:         requestPath(c.Request),
					RequestKind:  requestKindFromPath(requestPath(c.Request)),
					Model:        upstreamModel,
					Transport:    diagnosticTransport(c.Request),
					ErrorMessage: orderErr.Error(),
				})
			}
		} else if placeholders > 0 || synthesized > 0 || relocated > 0 {
			body = normalized
			if s.emitter != nil {
				s.emitter.emit(requestDiagnosticPayload{
					Type:         "provider_gateway_tool_call_pairing_repaired",
					RequestID:    internallogging.GetRequestID(c.Request.Context()),
					Method:       c.Request.Method,
					Path:         requestPath(c.Request),
					RequestKind:  requestKindFromPath(requestPath(c.Request)),
					Model:        upstreamModel,
					Transport:    diagnosticTransport(c.Request),
					ErrorMessage: fmt.Sprintf("injected %d placeholder tool output(s), synthesized %d tool call(s), relocated %d displaced tool output(s)", placeholders, synthesized, relocated),
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
	if isOpenCodeGoGateway(gateway.BaseURL) {
		applyOpenCodeSessionHeader(req.Header, c.Request.Header, body)
	}

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
		s.writeProviderGatewayResponsesStream(c, resp.Body)
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
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse) {
		payload = normalizeResponsesReasoningContentBody(payload)
	}
	contentType := resp.Header.Get("Content-Type")
	if contentType == "" || (wireAPI == "chat_completions" && !sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI)) {
		contentType = "application/json"
	}
	c.Data(http.StatusOK, contentType, payload)
}

// providerGatewayRepairsToolCallPairing reports whether the upstream verifies tool call and
// output pairing per request and therefore needs the pairing repair below.
func providerGatewayRepairsToolCallPairing(gateway *providerGatewaySpec) bool {
	if gateway == nil {
		return false
	}
	parsed, err := url.Parse(strings.TrimSpace(gateway.BaseURL))
	if err != nil {
		return false
	}
	return strings.EqualFold(parsed.Hostname(), "api.deepseek.com")
}

// providerGatewayToolCallOutputPlaceholder is the explicit failure result used when the
// Codex app-server serializes a tool call before its output was committed. The upstream
// requires a matching output; the placeholder keeps the transcript valid without claiming
// the tool succeeded, so the model can decide to retry.
const providerGatewayToolCallOutputPlaceholder = "tool result unavailable: the local agent did not commit an output for this call"

// providerGatewayNormalizeToolCallPairing enforces the "every tool call has a tool output"
// invariant that strict Responses upstreams verify per request. The Codex app-server can
// serialize the next sampling request before the last function_call_output of a parallel
// batch reaches the conversation history, which makes the upstream reject the whole turn
// with `No tool output found ...` and leaves the in-memory thread unusable. Injecting the
// missing output (or the call that an injected standalone output never had) keeps the
// request valid and stops the thread from wedging.
func providerGatewayNormalizeToolCallPairing(gateway *providerGatewaySpec, body []byte) ([]byte, int, int, int, error) {
	if !providerGatewayRepairsToolCallPairing(gateway) {
		return body, 0, 0, 0, nil
	}
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() || len(input.Array()) == 0 {
		return body, 0, 0, 0, nil
	}
	needsOrdering := providerGatewayToolCallOrderNeedsRepair(input)

	type callSlot struct {
		index int
		item  gjson.Result
	}
	calls := make(map[string]callSlot)
	order := make([]string, 0)
	for index, item := range input.Array() {
		if !providerGatewayIsToolCallItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			continue
		}
		if _, exists := calls[callID]; !exists {
			order = append(order, callID)
		}
		calls[callID] = callSlot{index: index, item: item}
	}

	answers := make(map[string]struct{})
	for _, item := range input.Array() {
		if !providerGatewayIsToolCallOutputItem(item.Get("type").String()) {
			continue
		}
		if callID := strings.TrimSpace(item.Get("call_id").String()); callID != "" {
			answers[callID] = struct{}{}
		}
	}

	// Exhaustive pairing means every live call already has an output and, when the pairs are
	// also adjacent, there is nothing to repair: the request is forwarded byte-for-byte.
	// Requests without any call still fall through so standalone injected outputs get their
	// missing call.
	if len(answers) > 0 && len(calls) == len(answers) && !needsOrdering {
		return body, 0, 0, 0, nil
	}

	orphanAnswers := make(map[int]struct{})
	for index, item := range input.Array() {
		if !providerGatewayIsToolCallOutputItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			orphanAnswers[index] = struct{}{}
			continue
		}
		if _, exists := calls[callID]; !exists {
			orphanAnswers[index] = struct{}{}
		}
	}

	insertedAt := make(map[int][]string)
	placeholderCount := 0
	for _, callID := range order {
		if _, answered := answers[callID]; answered {
			continue
		}
		insertedAt[calls[callID].index] = append(insertedAt[calls[callID].index], callID)
		placeholderCount++
	}

	rebuilt := make([]byte, 0, len(body)+256*placeholderCount)
	rebuilt = append(rebuilt, '[')
	written := 0
	writeItem := func(raw string) {
		if written > 0 {
			rebuilt = append(rebuilt, ',')
		}
		rebuilt = append(rebuilt, raw...)
		written++
	}
	for index, item := range input.Array() {
		raw := item.Raw
		if _, orphan := orphanAnswers[index]; orphan {
			// A standalone output without a call cannot exist on a strict upstream. The
			// app server injects this shape for automations and cross-thread messages, so
			// keep the content and add the call the output belongs to.
			callID := providerGatewayStandaloneCallID(item)
			writeItem(providerGatewaySynthesizeCallForOutput(item, callID))
			raw = providerGatewayOutputWithCallID(item, callID)
		}
		writeItem(raw)
		for _, callID := range insertedAt[index] {
			writeItem(providerGatewayPlaceholderOutputJSON(item.Get("type").String(), callID))
		}
	}
	rebuilt = append(rebuilt, ']')

	updated, err := sjson.SetRawBytes(body, "input", rebuilt)
	if err != nil {
		return body, 0, 0, 0, nil
	}
	if !needsOrdering {
		return updated, placeholderCount, len(orphanAnswers), 0, nil
	}
	ordered, relocated, ok := providerGatewayOrderToolCallPairs(updated)
	if !ok {
		return body, 0, 0, 0, fmt.Errorf(
			"provider gateway cannot normalize the tool call order for this request; the transcript contains tool items without a call_id or a tool call without a matching output",
		)
	}
	return ordered, placeholderCount, len(orphanAnswers), relocated, nil
}

// providerGatewayOrderToolCallPairs restores the positional invariant DeepSeek verifies: every
// tool call must be immediately followed by its own output.
//
// Codex breaks that ordering on a normal path. A tool hook (PostToolUse) records its developer
// message as soon as the tool returns, which can be before the tool's output item reaches
// conversation history, so the transcript ends up as
//
//	function_call -> message -> function_call_output
//
// DeepSeek validates adjacency per request and answers `No tool output found for tool call ...`
// for the whole request, which wedges the thread until it is unloaded. The pairing repair above
// does not cover this: the items exist, they are merely ordered illegally.
//
// The rewrite is deterministic and lossless. Each live call is paired with its output, a run of
// consecutive calls keeps its relative order, and every other item keeps its position relative
// to those pairs. Only an output that is not already in place moves, and it moves forward to sit
// directly behind its call.
func providerGatewayOrderToolCallPairs(body []byte) ([]byte, int, bool) {
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() {
		return body, 0, true
	}
	items := input.Array()
	if len(items) < 2 {
		return body, 0, true
	}
	if !providerGatewayToolCallOrderNeedsRepair(input) {
		return body, 0, true
	}

	// The upstream pairs a call with the next item that carries the same call_id, so the rebuilt
	// transcript has to put that token directly behind its call. Two position facts decide the
	// whole rewrite, and both are computed up front so the rebuild stays a single pass:
	//
	//   firstCall[id] / firstOutput[id]  where the call and its answer first appear
	//   relocated                        answers that did not follow their call in the original
	//
	// An answer that is already in place stays where it is; an answer displaced by a later call
	// or an intervening message is emitted behind its call, and anything that sat in between
	// follows it. Items the model does not need to re-read in order keep their relative order.
	firstCall := make(map[string]int)
	firstOutput := make(map[string]int)
	relocated := 0
	for index, item := range items {
		itemType := item.Get("type").String()
		switch {
		case providerGatewayIsToolCallItem(itemType):
			callID := strings.TrimSpace(item.Get("call_id").String())
			if callID == "" {
				return body, 0, false
			}
			if _, seen := firstCall[callID]; !seen {
				firstCall[callID] = index
			}
		case providerGatewayIsToolCallOutputItem(itemType):
			callID := strings.TrimSpace(item.Get("call_id").String())
			if callID == "" {
				return body, 0, false
			}
			if _, seen := firstOutput[callID]; !seen {
				firstOutput[callID] = index
			}
		}
	}
	for callID, outputIndex := range firstOutput {
		if callIndex, ok := firstCall[callID]; !ok || callIndex != outputIndex-1 {
			relocated++
		}
	}

	ordered := make([]gjson.Result, 0, len(items))
	callIndex := make(map[string]int)     // position of each call's item inside ordered
	waiting := make(map[string]struct{})  // calls emitted so far whose answer has not been placed
	answered := make(map[string]struct{}) // calls whose answer is in place
	takeCall := func(callID string) {
		delete(answered, callID)
		waiting[callID] = struct{}{}
	}
	placeAnswer := func(callID string, item gjson.Result) {
		position, ok := callIndex[callID]
		if !ok {
			return
		}
		ordered = append(ordered, gjson.Result{})
		copy(ordered[position+2:], ordered[position+1:])
		ordered[position+1] = item
		delete(waiting, callID)
		answered[callID] = struct{}{}
		for id, slot := range callIndex {
			if slot > position {
				callIndex[id] = slot + 1
			}
		}
	}
	for index, item := range items {
		itemType := item.Get("type").String()
		if providerGatewayIsToolCallItem(itemType) {
			callID := strings.TrimSpace(item.Get("call_id").String())
			callIndex[callID] = len(ordered)
			ordered = append(ordered, item)
			if outputIndex, ok := firstOutput[callID]; !ok || outputIndex > index {
				takeCall(callID)
			} else {
				answered[callID] = struct{}{}
			}
			continue
		}
		if providerGatewayIsToolCallOutputItem(itemType) {
			callID := strings.TrimSpace(item.Get("call_id").String())
			if _, done := answered[callID]; done {
				// A duplicate answer cannot sit next to its call any more, so it is dropped: the
				// alternative is a transcript the upstream rejects outright.
				continue
			}
			if _, owed := waiting[callID]; owed {
				placeAnswer(callID, item)
				continue
			}
			if firstOutput[callID] != index {
				continue
			}
			if _, hasCall := firstCall[callID]; hasCall {
				continue
			}
			// An answer with no call in this request keeps its place; the pairing repair above
			// already gave it a matching call when the upstream requires one.
			ordered = append(ordered, item)
			continue
		}
		ordered = append(ordered, item)
	}

	rebuilt := make([]byte, 0, len(body))
	rebuilt = append(rebuilt, '[')
	for index, item := range ordered {
		if index > 0 {
			rebuilt = append(rebuilt, ',')
		}
		rebuilt = append(rebuilt, item.Raw...)
	}
	rebuilt = append(rebuilt, ']')

	updated, err := sjson.SetRawBytes(body, "input", rebuilt)
	if err != nil {
		return body, 0, false
	}
	if !providerGatewayToolCallPairsAdjacent(gjson.GetBytes(updated, "input")) {
		return body, 0, false
	}
	return updated, relocated, true
}

// providerGatewayToolCallOrderNeedsRepair reports whether any tool call is answered by
// something other than its own output, which is the positional rule DeepSeek enforces.
func providerGatewayToolCallOrderNeedsRepair(input gjson.Result) bool {
	if !input.IsArray() {
		return false
	}
	items := input.Array()
	for index, item := range items {
		if !providerGatewayIsToolCallItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			continue
		}
		if index+1 >= len(items) {
			// No output recorded in this request; the pairing repair handles that case.
			continue
		}
		next := items[index+1]
		if !providerGatewayIsToolCallOutputItem(next.Get("type").String()) ||
			strings.TrimSpace(next.Get("call_id").String()) != callID {
			return true
		}
	}
	return false
}

// providerGatewayToolCallPairsAdjacent is the post-condition of providerGatewayOrderToolCallPairs.
func providerGatewayToolCallPairsAdjacent(input gjson.Result) bool {
	if !input.IsArray() {
		return false
	}
	items := input.Array()
	answered := make(map[string]struct{})
	for _, item := range items {
		if !providerGatewayIsToolCallOutputItem(item.Get("type").String()) {
			continue
		}
		if callID := strings.TrimSpace(item.Get("call_id").String()); callID != "" {
			answered[callID] = struct{}{}
		}
	}
	for index, item := range items {
		if !providerGatewayIsToolCallItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			return false
		}
		// A call whose result is genuinely absent from the request cannot be paired here; the
		// pairing repair decides what to do with it, so adjacency does not apply.
		if _, hasAnswer := answered[callID]; !hasAnswer {
			continue
		}
		if index+1 >= len(items) {
			return false
		}
		next := items[index+1]
		if !providerGatewayIsToolCallOutputItem(next.Get("type").String()) ||
			strings.TrimSpace(next.Get("call_id").String()) != callID {
			return false
		}
	}
	return true
}

func providerGatewayIsToolCallItem(itemType string) bool {
	switch strings.ToLower(strings.TrimSpace(itemType)) {
	case "function_call", "custom_tool_call":
		return true
	default:
		return false
	}
}

func providerGatewayIsToolCallOutputItem(itemType string) bool {
	switch strings.ToLower(strings.TrimSpace(itemType)) {
	case "function_call_output", "custom_tool_call_output":
		return true
	default:
		return false
	}
}

// providerGatewayPlaceholderOutputJSON builds the stand-in output for a call whose result
// never reached the transcript. It mirrors the call flavor so a custom tool call does not
// receive a function-shaped output.
func providerGatewayPlaceholderOutputJSON(callItemType string, callID string) string {
	outputType := "function_call_output"
	if strings.EqualFold(strings.TrimSpace(callItemType), "custom_tool_call") {
		outputType = "custom_tool_call_output"
	}
	return fmt.Sprintf(
		`{"type":%q,"call_id":%q,"output":%q}`,
		outputType,
		callID,
		providerGatewayToolCallOutputPlaceholder,
	)
}

// providerGatewaySynthesizeCallForOutput converts an injected standalone output into the
// call/output pair a strict upstream accepts. Adding only a call_id is not enough: without a
// matching function_call the upstream answers `No tool call found for tool output ...`.
func providerGatewaySynthesizeCallForOutput(item gjson.Result, callID string) string {
	itemType := strings.ToLower(strings.TrimSpace(item.Get("type").String()))
	callType := "function_call"
	argumentsKey := "arguments"
	if itemType == "custom_tool_call_output" {
		callType = "custom_tool_call"
		argumentsKey = "input"
	}
	name := strings.TrimSpace(item.Get("name").String())
	if name == "" {
		name = "external_tool"
	}
	call := map[string]any{
		"type":    callType,
		"call_id": callID,
		"name":    name,
	}
	if argumentsKey == "arguments" {
		call["arguments"] = "{}"
	} else {
		call["input"] = ""
	}
	if namespace := strings.TrimSpace(item.Get("namespace").String()); namespace != "" {
		call["namespace"] = namespace
	}
	encoded, err := json.Marshal(call)
	if err != nil {
		return item.Raw
	}
	return string(encoded)
}

// providerGatewayStandaloneCallID derives the shared call_id for a synthesized pair. The
// source item id is used when the app server provided one so the mapping stays explainable
// in logs and rollouts; the result is sanitized because it travels as an opaque token.
func providerGatewayStandaloneCallID(item gjson.Result) string {
	if callID := strings.TrimSpace(item.Get("call_id").String()); callID != "" {
		return callID
	}
	return "call_standalone_" + providerGatewayShortToken(item.Get("id").String())
}

// providerGatewayOutputWithCallID rewrites the output item so it references the caller's
// call_id. The original item is returned unchanged if the rewrite fails.
func providerGatewayOutputWithCallID(item gjson.Result, callID string) string {
	updated, err := sjson.SetBytes([]byte(item.Raw), "call_id", callID)
	if err != nil {
		return item.Raw
	}
	return string(updated)
}

func providerGatewayShortToken(raw string) string {
	token := strings.TrimSpace(raw)
	if token == "" {
		return "orphan"
	}
	if len(token) > 24 {
		token = token[len(token)-24:]
	}
	token = strings.NewReplacer("\"", "", "\\", "", " ", "").Replace(token)
	return token
}

func isOpenCodeGoGateway(rawURL string) bool {
	u, err := url.Parse(strings.TrimSpace(rawURL))
	if err != nil {
		return false
	}
	host := strings.ToLower(strings.TrimSpace(u.Hostname()))
	return host == "opencode.ai" || strings.HasSuffix(host, ".opencode.ai")
}

func applyOpenCodeSessionHeader(dst, src http.Header, payload []byte) {
	if dst.Get("x-opencode-session") != "" {
		return
	}
	if value := strings.TrimSpace(src.Get("x-opencode-session")); value != "" {
		dst.Set("x-opencode-session", value)
		return
	}
	if info, ok := cliproxysession.ExtractSessionInfo(src, payload, nil); ok && info.SessionID != "" {
		dst.Set("x-opencode-session", info.SessionID)
		return
	}
	// A request-scoped opaque fallback is preferable to dropping the required
	// header. Clients that expose a conversation identity are handled above.
	var randomID [16]byte
	if _, err := rand.Read(randomID[:]); err == nil {
		dst.Set("x-opencode-session", fmt.Sprintf("cockpit-%x", randomID[:]))
	}
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

// writeProviderGatewayResponsesStream 透传 provider gateway 的 Responses SSE，
// 只在出口清洗第三方推理项，其余字节与原有 io.Copy 透传保持一致。
func (s *relayServer) writeProviderGatewayResponsesStream(c *gin.Context, body io.Reader) {
	if body == nil {
		return
	}
	reader := bufio.NewReaderSize(body, 64*1024)
	for {
		line, err := reader.ReadBytes('\n')
		if len(line) > 0 {
			if _, writeErr := c.Writer.Write(normalizeResponsesReasoningContentSSELine(line)); writeErr != nil {
				return
			}
		}
		if err != nil {
			return
		}
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
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse) {
		// 出口统一清洗第三方推理项，避免客户端把不兼容的 reasoning content 落盘。
		resp.Payload = normalizeResponsesReasoningContentBody(resp.Payload)
	}
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

// providerGatewayRestoreReasoningText 把“只有 summary、没有 content”的推理项补回
// reasoning_text 再发往严格上游。
//
// 背景：两个上游对同一个字段的要求相反。
//   - DeepSeek 的思考模式要求每次回放都带上 reasoning_text，否则整段请求被拒：
//     `The reasoning_text in the thinking mode must be passed back to the API.`
//   - 官方 Codex 后端只接受 content 为空数组的推理项（见 responses_reasoning_sanitize.go）。
//
// 网关在响应出口为了让历史能切回官方账号，会把 DeepSeek 的 content 迁移进 summary 并清空
// content。于是同一份历史再发回 DeepSeek 时必然被拒，线程从此无法继续。本函数在请求出口做
// 反向补全：content 为空而 summary 有文本时，把 summary 文本写回 reasoning_text。两个方向各自
// 得到自己需要的形状，且不新增任何信息——只是把原本来自该模型自己的思考文本放回原字段。
func providerGatewayRestoreReasoningText(gateway *providerGatewaySpec, body []byte) ([]byte, int) {
	if !providerGatewayRepairsToolCallPairing(gateway) {
		return body, 0
	}
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() || len(input.Array()) == 0 {
		return body, 0
	}
	restored := 0
	for index, item := range input.Array() {
		if !strings.EqualFold(strings.TrimSpace(item.Get("type").String()), "reasoning") {
			continue
		}
		content := item.Get("content")
		if content.IsArray() && len(content.Array()) > 0 {
			continue
		}
		summary := item.Get("summary")
		if !summary.IsArray() || len(summary.Array()) == 0 {
			continue
		}
		texts := make([]string, 0, len(summary.Array()))
		for _, part := range summary.Array() {
			if text := part.Get("text").String(); text != "" {
				texts = append(texts, text)
			}
		}
		if len(texts) == 0 {
			continue
		}
		payload, err := json.Marshal([]map[string]string{{"type": "reasoning_text", "text": strings.Join(texts, "\n")}})
		if err != nil {
			continue
		}
		updated, err := sjson.SetRawBytes(body, fmt.Sprintf("input.%d.content", index), payload)
		if err != nil {
			continue
		}
		body = updated
		restored++
	}
	return body, restored
}
