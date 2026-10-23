package executor

import (
	"bufio"
	"bytes"
	"context"
	"io"
	"net/http"
	"strings"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/client/grokbuild"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/codex/collaboration"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/thinking"
	codexresponses "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/codex/openai/responses"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	log "github.com/sirupsen/logrus"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

func (e *CodexExecutor) ExecuteStream(ctx context.Context, auth *cliproxyauth.Auth, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (result *cliproxyexecutor.StreamResult, err error) {
	ctx = helps.EnsureSessionContext(ctx, opts, req.Payload)
	if continuityErr := cliproxyexecutor.ContinuityPreparationError(ctx); continuityErr != nil {
		return nil, continuityErr
	}
	opts.Headers = codexRequestHeadersWithGinResponsesLite(ctx, opts.Headers)
	liteHeaderValue := ""
	if opts.Headers != nil {
		liteHeaderValue = headerValueCaseInsensitive(opts.Headers, codexResponsesLiteHeaderName)
	}
	if opts.Alt == "responses/compact" {
		return nil, statusErr{code: http.StatusBadRequest, msg: "streaming not supported for /responses/compact"}
	}
	if isCodexOpenAIImageRequest(opts) {
		return e.executeOpenAIImageStream(ctx, auth, req, opts)
	}
	baseModel := thinking.ParseSuffix(req.Model).ModelName
	isCompat := e.resolveCodexModelIsCompat(auth, req, baseModel)

	apiKey, baseURL := codexCreds(auth)
	if baseURL == "" {
		baseURL = "https://chatgpt.com/backend-api/codex"
	}

	reporter := helps.NewExecutorUsageReporter(ctx, e, baseModel, auth)
	defer reporter.TrackFailure(ctx, &err)

	from := opts.SourceFormat
	responseFormat := cliproxyexecutor.ResponseFormatOrSource(opts)
	isGrokClient := grokbuild.IsGrokClientContext(ctx, opts.Headers)
	to := sdktranslator.FromString("codex")
	originalPayloadSource := req.Payload
	if len(opts.OriginalRequest) > 0 {
		originalPayloadSource = opts.OriginalRequest
	}
	originalPayload := originalPayloadSource
	originalTranslated, body := translateCodexRequestPair(from, to, baseModel, originalPayload, req.Payload, true, isCompat)

	body, err = helps.ApplyRequestThinking(body, req, opts, from.String(), to.String(), e.Identifier())
	if err != nil {
		return nil, err
	}

	requestedModel := helps.PayloadRequestedModel(opts, req.Model)
	requestPath := helps.PayloadRequestPath(opts)
	body = helps.ApplyPayloadConfigWithRequest(e.cfg, baseModel, to.String(), from.String(), "", body, originalTranslated, requestedModel, requestPath, opts.Headers)
	body = helps.ApplyServiceTierDefault(ctx, body, originalTranslated)
	if !cliproxyexecutor.ContinuityActive(ctx) {
		body, _ = sjson.DeleteBytes(body, "previous_response_id")
	}
	body, _ = sjson.DeleteBytes(body, "generate")
	body, _ = sjson.DeleteBytes(body, "prompt_cache_retention")
	body, _ = sjson.DeleteBytes(body, "safety_identifier")
	reasoningSummaryDelivery := gjson.GetBytes(body, "stream_options.reasoning_summary_delivery")
	body, _ = sjson.DeleteBytes(body, "stream_options")
	if reasoningSummaryDelivery.Exists() {
		body, _ = sjson.SetBytes(body, "stream_options.reasoning_summary_delivery", reasoningSummaryDelivery.Value())
	}
	body = helps.SetStringIfDifferent(body, "model", baseModel)
	body = normalizeCodexInstructions(body)
	if e.cfg == nil || e.cfg.DisableImageGeneration == config.DisableImageGenerationOff {
		body = ensureImageGenerationTool(body, baseModel, auth, opts.Headers)
	}
	if !isCompat && !cliproxyexecutor.ContinuityActive(ctx) {
		body = normalizeNonOfficialCodexReasoningItems(ctx, "codex executor", body)
	}
	if !cliproxyexecutor.ContinuityActive(ctx) {
		body = sanitizeOpenAIResponsesReasoningEncryptedContentWithCompat(ctx, "codex executor", body, isCompat)
	}
	body = normalizeCodexParallelToolCalls(body, opts.Headers)
	body = helps.NormalizeCodexToolSchemas(body)
	body, useFullResponses := normalizeCodexResponsesLiteRequest(body, opts.Headers, auth, true)
	body, optimizeMultiAgentV2 := helps.OptimizeCodexMultiAgentV2RequestForAuth(ctx, opts.Headers, body, e.cfg, auth, isCompat)
	body, replayScope, errReplay := applyCodexReasoningReplayCacheRequired(ctx, from, req, opts, body)
	if errReplay != nil {
		return nil, errReplay
	}
	if helps.IsCodexOAuthAuth(auth) && !isCompat {
		lite := !useFullResponses && (codexResponsesLiteEnabled(opts.Headers) || isCodexResponsesLiteRequest(body, opts.Headers))
		body = codexresponses.EnsureCodexWebSearchHistoryTool(body, lite)
	}
	reporter.SetTranslatedReasoningEffort(body, to.String())

	url := strings.TrimSuffix(baseURL, "/") + "/responses"
	var identityState codexIdentityConfuseState
	httpReq, upstreamBody, identityState, err := e.cacheHelper(ctx, from, url, auth, req, originalPayloadSource, body, opts.Headers)
	if err != nil {
		return nil, err
	}
	applyCodexHeaders(httpReq, auth, apiKey, true, e.cfg, opts.Headers)
	applyCodexRoutingHint(ctx, httpReq.Header, auth, baseModel, upstreamBody, opts.Headers)
	if !useFullResponses && liteHeaderValue != "" {
		httpReq.Header.Set(codexResponsesLiteHeaderName, liteHeaderValue)
	}
	applyModelHeaderOverrides(httpReq.Header, baseModel)
	applyCodexIdentityConfuseHeaders(httpReq.Header, &identityState)
	if useFullResponses {
		removeCodexResponsesLiteHeaderForFullResponse(httpReq.Header, true)
	} else if liteHeaderValue != "" {
		httpReq.Header.Set(codexResponsesLiteHeaderName, liteHeaderValue)
	}
	if !useFullResponses {
		if liteHeaderValue == "" {
			if ginCtx, ok := ctx.Value("gin").(*gin.Context); ok && ginCtx != nil && ginCtx.Request != nil && ginCtx.Request.Header.Get(codexResponsesLiteHeaderName) != "" {
				httpReq.Header.Set(codexResponsesLiteHeaderName, ginCtx.Request.Header.Get(codexResponsesLiteHeaderName))
			}
		}
	}
	if !useFullResponses && liteHeaderValue != "" {
		httpReq.Header[codexResponsesLiteHeaderName] = []string{liteHeaderValue}
	}
	// Desktop can carry the Lite hint only in client_metadata. Keep the
	// outgoing wire mode aligned with the final additional_tools placement.
	if helps.IsCodexOAuthAuth(auth) && !useFullResponses && isCodexResponsesLiteRequest(body, opts.Headers) && !codexResponsesLiteEnabled(httpReq.Header) {
		httpReq.Header.Set(codexResponsesLiteHeaderName, "true")
	}
	removeCodexResponsesLiteHeaderForAPIKey(httpReq.Header, auth)
	var authID, authLabel, authType, authValue string
	if auth != nil {
		authID = auth.ID
		authLabel = auth.Label
		authType, authValue = auth.AccountInfo()
	}
	continuityAttempt, continuityBody, errContinuity := helps.BindContinuityHTTPRequest(ctx, auth, httpReq, upstreamBody)
	if errContinuity != nil {
		return nil, errContinuity
	}
	upstreamBody = continuityBody
	recovery := helps.NewCodexHTTPEncryptedRecovery(auth, isCompat || cliproxyexecutor.ContinuityActive(ctx), httpReq, upstreamBody)
	upstreamBody, err = recovery.PrepareRequest(httpReq, upstreamBody)
	if err != nil {
		return nil, err
	}
	reporter.SetUpstreamServiceTier(upstreamBody)
	helps.RecordAPIRequest(ctx, e.cfg, helps.UpstreamRequestLog{
		Transport: "sse",
		URL:       url,
		Method:    http.MethodPost,
		Headers:   httpReq.Header.Clone(),
		Body:      upstreamBody,
		Provider:  e.Identifier(),
		AuthID:    authID,
		AuthLabel: authLabel,
		AuthType:  authType,
		AuthValue: authValue,
	})

	httpClient := helps.NewUtlsHTTPClient(ctx, e.cfg, auth, 0)
	httpClient = reporter.TrackHTTPClientRoundTripOnly(httpClient)
	httpResp, upstreamBody, err := recovery.Do(ctx, e.cfg, httpClient, httpReq, upstreamBody, func(status int, errorBody []byte) error {
		return clearCodexReasoningReplayOnInvalidSignature(ctx, replayScope, status, errorBody)
	})
	plaintextPolicy := collaboration.PlaintextPolicyForRequest(upstreamBody, optimizeMultiAgentV2)
	helps.SetCodexMultiAgentV2PlaintextObserver(ctx, opts.Headers, continuityAttempt, plaintextPolicy, e.cfg)
	if httpResp != nil && httpResp.Request == nil {
		httpResp.Request = httpReq
	}
	err = helps.ObserveContinuityHTTPResponse(ctx, continuityAttempt, httpResp, err)
	if err != nil {
		helps.RecordAPIResponseError(ctx, e.cfg, err)
		return nil, err
	}
	helps.RecordAPIResponseMetadata(ctx, e.cfg, httpResp.StatusCode, httpResp.Header.Clone())
	if httpResp.StatusCode < 200 || httpResp.StatusCode >= 300 {
		data, readErr := io.ReadAll(httpResp.Body)
		if errClose := httpResp.Body.Close(); errClose != nil {
			log.Errorf("codex executor: close response body error: %v", errClose)
		}
		if readErr != nil {
			helps.RecordAPIResponseError(ctx, e.cfg, readErr)
			return nil, readErr
		}
		data = applyCodexIdentityConfuseResponsePayload(data, identityState)
		if errClearReplay := clearCodexReasoningReplayOnInvalidSignature(ctx, replayScope, httpResp.StatusCode, data); errClearReplay != nil {
			return nil, errClearReplay
		}
		helps.AppendAPIResponseChunk(ctx, e.cfg, data)
		helps.LogWithRequestID(ctx).Debugf("request error, error status: %d, error message: %s", httpResp.StatusCode, helps.SummarizeErrorBody(httpResp.Header.Get("Content-Type"), data))
		err = cliproxyexecutor.UpstreamRequestRejected(newCodexStatusErrWithCooling(httpResp.StatusCode, data, e.modelLevelCooling()))
		return nil, err
	}

	buffering := e.cfg != nil && e.cfg.Codex.StreamBootstrapBuffering

	scanner := bufio.NewScanner(httpResp.Body)
	scanner.Buffer(nil, 52_428_800) // 50MB
	claudeInputTokens := helps.NewClaudeInputTokenState(from, to, responseFormat, originalPayload)
	var param any
	outputItemsByIndex := make(map[int64][]byte)
	var outputItemsFallback [][]byte

	var bufferedChunks [][]byte
	var bufferedEvents, bufferedBytes int
	var initialChunks [][]byte
	streamStarted := false
	immediateTerminal := false
	// bootstrapTerminalErr holds a non-overload terminal failure seen while buffering. It is
	// delivered as an in-stream chunk after the buffered handshake so downstream behaviour stays
	// identical to the unbuffered path instead of silently turning into a credential failover.
	var bootstrapTerminalErr error
	sawOutputDelta := false

	closeBootstrapBody := func() {
		if errClose := httpResp.Body.Close(); errClose != nil {
			log.Errorf("codex executor: close response body error: %v", errClose)
		}
	}

	if buffering {
		for scanner.Scan() {
			line := applyCodexIdentityConfuseResponsePayload(scanner.Bytes(), identityState)
			helps.AppendAPIResponseChunk(ctx, e.cfg, line)
			translatedLine := bytes.Clone(line)
			isHandshake := false
			terminalSuccess := false

			if transformed, ok := grokbuild.TransformKeepaliveSSELine(translatedLine, isGrokClient); ok {
				translatedLine = transformed
				isHandshake = true
			} else if bytes.HasPrefix(line, dataTag) {
				data := bytes.TrimSpace(line[5:])
				data = helps.RestoreCodexMultiAgentV2Response(data, optimizeMultiAgentV2)
				data = helps.MarkCodexMultiAgentV2PlaintextResponse(ctx, opts.Headers, data, plaintextPolicy, e.cfg)
				observeCodexTokenEvent(reporter, data)
				translatedLine = append([]byte("data: "), data...)
				eventType := gjson.GetBytes(data, "type").String()
				if streamErr, terminalBody, ok := codexTerminalFailureErrWithCooling(data, e.modelLevelCooling()); ok {
					closeBootstrapBody()
					if errClearReplay := clearCodexReasoningReplayOnInvalidSignature(ctx, replayScope, streamErr.StatusCode(), terminalBody); errClearReplay != nil {
						helps.RecordAPIResponseError(ctx, e.cfg, errClearReplay)
						reporter.PublishFailure(ctx, errClearReplay)
						return nil, errClearReplay
					}
					helps.RecordAPIResponseError(ctx, e.cfg, streamErr)
					reporter.PublishFailure(ctx, streamErr)
					if helps.CodexEventHasNoExecution(data) && isCodexRetryableBootstrapFailure(terminalBody) {
						// Transient capacity rejection smuggled into an HTTP 200 stream. Fail the
						// attempt before the downstream headers are committed so the conductor can
						// transparently retry on another credential, and report the status the
						// upstream refused to put on the wire.
						helps.LogWithRequestID(ctx).Debugf("codex executor: bootstrap overload rejection after %d buffered handshake events, failing over", len(bufferedChunks))
						return nil, newCodexBootstrapFailureErr(terminalBody, e.modelLevelCooling())
					}
					if !helps.CodexEventHasNoExecution(data) {
						return nil, helps.CodexStopAfterExecution(streamErr, false)
					}
					bootstrapTerminalErr = streamErr
					break
				}
				if helps.HasMeaningfulCodexOutputDelta(data) {
					sawOutputDelta = true
				}
				if helps.IsCodexTerminalEmptyIncomplete(data, len(outputItemsByIndex)+len(outputItemsFallback), sawOutputDelta) {
					closeBootstrapBody()
					streamErr := newCodexEmptyIncompleteStreamError()
					reporter.PublishFailure(ctx, streamErr)
					return nil, streamErr
				}
				if helps.CodexHandshakeMetadataEvent(data) {
					isHandshake = true
				}
				switch eventType {
				case "response.output_item.done":
					collectCodexOutputItemDone(data, outputItemsByIndex, &outputItemsFallback)
				case "response.completed", "response.incomplete", "response.done":
					terminalSuccess = true
					data = normalizeCodexWebsocketCompletion(data)
					if detail, ok := helps.ParseCodexUsage(data); ok {
						helps.ObserveCodexCompletionUsage(ctx, data)
						reporter.Publish(ctx, detail)
					} else {
						reporter.EnsurePublished(ctx)
					}
					publishCodexImageToolUsage(ctx, reporter, body, data)
					data = patchCodexCompletedOutput(data, outputItemsByIndex, outputItemsFallback)
					if eventType == "response.completed" || eventType == "response.done" {
						cacheCodexReasoningReplayFromCompleted(replayScope, data)
					}
					translatedLine = append([]byte("data: "), data...)
				}
			} else {
				isHandshake = true
			}

			translatedLine = applyCodexIdentityExposeResponsePayload(translatedLine, identityState)
			chunks := helps.TranslateStreamWithClaudeInputTokens(ctx, to, responseFormat, req.Model, originalPayload, body, translatedLine, &param, claudeInputTokens)
			if isHandshake && !terminalSuccess {
				if bytes.HasPrefix(line, dataTag) {
					bufferedEvents++
				}
				bufferedBytes += len(line)
				for _, chunk := range chunks {
					bufferedBytes += len(chunk)
				}
				if bufferedEvents <= codexBootstrapMaxBufferedEvents && bufferedBytes <= codexBootstrapMaxBufferedBytes {
					bufferedChunks = append(bufferedChunks, chunks...)
					continue
				}
				helps.LogWithRequestID(ctx).Debugf("codex executor: bootstrap buffer limit %d reached, releasing stream without overload probing", codexBootstrapMaxBufferedEvents)
			}

			initialChunks = chunks
			streamStarted = true
			if terminalSuccess {
				immediateTerminal = true
			}
			break
		}

		if !streamStarted && bootstrapTerminalErr == nil {
			closeBootstrapBody()
			if errScan := scanner.Err(); errScan != nil {
				// A cancelled downstream request must not be recorded as an upstream failure or
				// penalise the credential; mirror the unbuffered goroutine's guard.
				if ctx.Err() != nil {
					return nil, ctx.Err()
				}
				streamErr := newCodexIncompleteStreamError(errScan)
				helps.RecordAPIResponseError(ctx, e.cfg, streamErr)
				reporter.PublishFailure(ctx, streamErr)
				return nil, streamErr
			}
			if ctx.Err() != nil {
				return nil, ctx.Err()
			}
			streamErr := newCodexIncompleteStreamError(nil)
			helps.RecordAPIResponseError(ctx, e.cfg, streamErr)
			reporter.PublishFailure(ctx, streamErr)
			return nil, streamErr
		}
	}

	chanCapacity := len(bufferedChunks) + len(initialChunks)
	if bootstrapTerminalErr != nil {
		chanCapacity++
	}
	out := make(chan cliproxyexecutor.StreamChunk, chanCapacity)
	for _, chunk := range bufferedChunks {
		out <- cliproxyexecutor.StreamChunk{Payload: chunk}
	}
	for _, chunk := range initialChunks {
		out <- cliproxyexecutor.StreamChunk{Payload: chunk}
	}
	if bootstrapTerminalErr != nil {
		// Buffered handshake payloads are flushed first so the conductor observes a committed
		// stream and delivers this failure in-stream, exactly as the unbuffered path would.
		out <- cliproxyexecutor.StreamChunk{Err: bootstrapTerminalErr}
		close(out)
		return &cliproxyexecutor.StreamResult{Headers: httpResp.Header.Clone(), Chunks: out}, nil
	}
	if immediateTerminal {
		closeBootstrapBody()
		close(out)
		return &cliproxyexecutor.StreamResult{Headers: httpResp.Header.Clone(), Chunks: out}, nil
	}

	go func() {
		defer close(out)
		defer func() {
			if errClose := httpResp.Body.Close(); errClose != nil {
				log.Errorf("codex executor: close response body error: %v", errClose)
			}
		}()
		for scanner.Scan() {
			line := applyCodexIdentityConfuseResponsePayload(scanner.Bytes(), identityState)
			helps.AppendAPIResponseChunk(ctx, e.cfg, line)
			translatedLine := bytes.Clone(line)
			terminalSuccess := false

			if transformed, ok := grokbuild.TransformKeepaliveSSELine(translatedLine, isGrokClient); ok {
				translatedLine = transformed
			} else if bytes.HasPrefix(line, dataTag) {
				data := bytes.TrimSpace(line[5:])
				data = helps.RestoreCodexMultiAgentV2Response(data, optimizeMultiAgentV2)
				data = helps.MarkCodexMultiAgentV2PlaintextResponse(ctx, opts.Headers, data, plaintextPolicy, e.cfg)
				observeCodexTokenEvent(reporter, data)
				translatedLine = append([]byte("data: "), data...)
				eventType := gjson.GetBytes(data, "type").String()
				if streamErr, terminalBody, ok := codexTerminalFailureErrWithCooling(data, e.modelLevelCooling()); ok {
					if errClearReplay := clearCodexReasoningReplayOnInvalidSignature(ctx, replayScope, streamErr.StatusCode(), terminalBody); errClearReplay != nil {
						helps.RecordAPIResponseError(ctx, e.cfg, errClearReplay)
						reporter.PublishFailure(ctx, errClearReplay)
						select {
						case out <- cliproxyexecutor.StreamChunk{Err: errClearReplay}:
						case <-ctx.Done():
						}
						return
					}
					helps.RecordAPIResponseError(ctx, e.cfg, streamErr)
					reporter.PublishFailure(ctx, streamErr)
					select {
					case out <- cliproxyexecutor.StreamChunk{Err: streamErr}:
					case <-ctx.Done():
					}
					return
				}
				if helps.HasMeaningfulCodexOutputDelta(data) {
					sawOutputDelta = true
				}
				if helps.IsCodexTerminalEmptyIncomplete(data, len(outputItemsByIndex)+len(outputItemsFallback), sawOutputDelta) {
					streamErr := newCodexEmptyIncompleteStreamError()
					reporter.PublishFailure(ctx, streamErr)
					select {
					case out <- cliproxyexecutor.StreamChunk{Err: streamErr}:
					case <-ctx.Done():
					}
					return
				}
				switch eventType {
				case "response.output_item.done":
					collectCodexOutputItemDone(data, outputItemsByIndex, &outputItemsFallback)
				case "response.completed", "response.incomplete", "response.done":
					terminalSuccess = true
					data = normalizeCodexWebsocketCompletion(data)
					if detail, ok := helps.ParseCodexUsage(data); ok {
						helps.ObserveCodexCompletionUsage(ctx, data)
						reporter.Publish(ctx, detail)
					} else {
						reporter.EnsurePublished(ctx)
					}
					publishCodexImageToolUsage(ctx, reporter, body, data)
					data = patchCodexCompletedOutput(data, outputItemsByIndex, outputItemsFallback)
					if eventType == "response.completed" || eventType == "response.done" {
						cacheCodexReasoningReplayFromCompleted(replayScope, data)
					}
					translatedLine = append([]byte("data: "), data...)
				}
			}

			translatedLine = applyCodexIdentityExposeResponsePayload(translatedLine, identityState)
			chunks := helps.TranslateStreamWithClaudeInputTokens(ctx, to, responseFormat, req.Model, originalPayload, body, translatedLine, &param, claudeInputTokens)
			for i := range chunks {
				select {
				case out <- cliproxyexecutor.StreamChunk{Payload: chunks[i]}:
				case <-ctx.Done():
					return
				}
			}
			if terminalSuccess {
				return
			}
		}
		if ctx.Err() != nil {
			return
		}
		streamErr := newCodexIncompleteStreamError(scanner.Err())
		helps.RecordAPIResponseError(ctx, e.cfg, streamErr)
		reporter.PublishFailure(ctx, streamErr)
		select {
		case out <- cliproxyexecutor.StreamChunk{Err: streamErr}:
		case <-ctx.Done():
		}
	}()
	return &cliproxyexecutor.StreamResult{Headers: httpResp.Header.Clone(), Chunks: out}, nil
}
