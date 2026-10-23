package executor

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"strings"

	"github.com/gorilla/websocket"
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

func (e *CodexWebsocketsExecutor) Execute(ctx context.Context, auth *cliproxyauth.Auth, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (resp cliproxyexecutor.Response, err error) {
	if continuityErr := cliproxyexecutor.ContinuityPreparationError(ctx); continuityErr != nil {
		return resp, continuityErr
	}
	opts.Headers = codexRequestHeadersWithGinResponsesLite(ctx, opts.Headers)
	if ctx == nil {
		ctx = context.Background()
	}
	if opts.Alt == "responses/compact" {
		return e.CodexExecutor.executeCompact(ctx, auth, req, opts)
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
	to := sdktranslator.FromString("codex")
	originalPayloadSource := req.Payload
	if len(opts.OriginalRequest) > 0 {
		originalPayloadSource = opts.OriginalRequest
	}
	originalPayload := originalPayloadSource
	originalTranslated, body := translateCodexRequestPair(from, to, baseModel, originalPayload, req.Payload, false, isCompat)

	body, err = helps.ApplyRequestThinking(body, req, opts, from.String(), to.String(), e.Identifier())
	if err != nil {
		return resp, err
	}

	requestedModel := helps.PayloadRequestedModel(opts, req.Model)
	requestPath := helps.PayloadRequestPath(opts)
	body = helps.ApplyPayloadConfigWithRequest(e.cfg, baseModel, to.String(), from.String(), "", body, originalTranslated, requestedModel, requestPath, opts.Headers)
	body = helps.ApplyServiceTierDefault(ctx, body, originalTranslated)
	body = helps.SetStringIfDifferent(body, "model", baseModel)
	body = helps.SetBoolIfDifferent(body, "stream", true)
	body, _ = sjson.DeleteBytes(body, "prompt_cache_retention")
	body, _ = sjson.DeleteBytes(body, "safety_identifier")
	body = normalizeCodexInstructions(body)
	if e.cfg == nil || e.cfg.DisableImageGeneration == config.DisableImageGenerationOff {
		body = ensureImageGenerationTool(body, baseModel, auth, opts.Headers)
	}
	body, useFullResponses := normalizeCodexResponsesLiteRequest(body, opts.Headers, auth, true)
	if !isCompat && !cliproxyexecutor.ContinuityActive(ctx) {
		body = normalizeNonOfficialCodexReasoningItems(ctx, "codex websockets executor", body)
	}
	if !cliproxyexecutor.ContinuityActive(ctx) {
		body = sanitizeOpenAIResponsesReasoningEncryptedContentWithCompat(ctx, "codex websockets executor", body, isCompat)
	}
	body = normalizeCodexWebsocketParallelToolCalls(body, opts.Headers)
	body = helps.NormalizeCodexToolSchemas(body)
	multiAgentV2Conflict := helps.HasCodexMultiAgentV2NamespaceConflict(body)
	body, optimizeMultiAgentV2 := helps.OptimizeCodexMultiAgentV2RequestForAuth(ctx, opts.Headers, body, e.cfg, auth, isCompat)
	body, replayScope, errReplay := applyCodexReasoningReplayCacheRequired(ctx, from, req, opts, body)
	if errReplay != nil {
		return resp, errReplay
	}

	if helps.IsCodexOAuthAuth(auth) && !isCompat {
		lite := !useFullResponses && (codexResponsesLiteEnabled(opts.Headers) || isCodexResponsesLiteRequest(body, opts.Headers))
		body = codexresponses.EnsureCodexWebSearchHistoryTool(body, lite)
	}

	httpURL := strings.TrimSuffix(baseURL, "/") + "/responses"
	wsURL, err := buildCodexResponsesWebsocketURL(httpURL)
	if err != nil {
		return resp, err
	}

	body, wsHeaders, errPromptCache := applyCodexPromptCacheHeadersWithContext(ctx, from, req, body, opts.Headers)
	if errPromptCache != nil {
		return resp, errPromptCache
	}
	clientBody := body
	var identityState codexIdentityConfuseState
	upstreamBody, identityState := applyCodexIdentityConfuseBody(e.cfg, auth, originalPayloadSource, body)
	if cliproxyexecutor.ContinuityActive(ctx) {
		upstreamBody = body
		identityState = codexIdentityConfuseState{}
	} else {
		upstreamBody, err = helps.ApplyManagedIdentity(auth, nil, upstreamBody)
		if err != nil {
			return resp, err
		}
		identityState.managedAuth = auth
	}
	reporter.SetTranslatedReasoningEffort(clientBody, to.String())
	wsHeaders = applyCodexWebsocketHeaders(ctx, wsHeaders, auth, apiKey, e.cfg, opts.Headers)
	// The Lite declaration shape must match the mode actually sent on the socket.
	if !useFullResponses && helps.IsCodexOAuthAuth(auth) {
		if codexResponsesLiteEnabled(opts.Headers) {
			wsHeaders.Set(codexResponsesLiteHeaderName, headerValueCaseInsensitive(opts.Headers, codexResponsesLiteHeaderName))
		} else if isCodexResponsesLiteRequest(body, opts.Headers) {
			wsHeaders.Set(codexResponsesLiteHeaderName, "true")
		}
	}

	applyCodexRoutingHint(ctx, wsHeaders, auth, baseModel, upstreamBody, opts.Headers)
	applyModelHeaderOverrides(wsHeaders, baseModel)
	removeCodexResponsesLiteHeaderForFullResponse(wsHeaders, useFullResponses)
	removeCodexResponsesLiteHeaderForAPIKey(wsHeaders, auth)
	applyCodexIdentityConfuseHeaders(wsHeaders, &identityState)
	continuityInput := bytes.Clone(upstreamBody)
	continuityHeaders := wsHeaders.Clone()
	continuityAttempt, boundHeaders, boundBody, errContinuity := helps.BindCodexContinuity(ctx, auth, wsHeaders, upstreamBody)
	if errContinuity != nil {
		return resp, errContinuity
	}
	wsHeaders, upstreamBody = boundHeaders, boundBody
	defer func() {
		if errFinish := continuityAttempt.Finish(ctx, err); errFinish != nil && err == nil {
			err = errFinish
		}
	}()

	var authID, authLabel, authType, authValue string
	if auth != nil {
		authID = auth.ID
		authLabel = auth.Label
		authType, authValue = auth.AccountInfo()
	}

	executionSessionID := executionSessionIDFromOptions(opts)
	var sess *codexWebsocketSession
	sessionLocked := false
	unlockSession := func() {
		if sess != nil && sessionLocked {
			sess.reqMu.Unlock()
			sessionLocked = false
		}
	}
	if executionSessionID != "" {
		sess = e.getOrCreateSession(executionSessionID)
		sess.reqMu.Lock()
		sessionLocked = true
		defer unlockSession()
	}

	wsReqBody := buildCodexWebsocketRequestBody(upstreamBody, ctx)
	wsReqLog := helps.UpstreamRequestLog{
		URL:       wsURL,
		Method:    "WEBSOCKET",
		Headers:   wsHeaders.Clone(),
		Body:      wsReqBody,
		Provider:  e.Identifier(),
		AuthID:    authID,
		AuthLabel: authLabel,
		AuthType:  authType,
		AuthValue: authValue,
	}
	helps.RecordAPIWebsocketRequest(ctx, e.cfg, wsReqLog)

	var conn *websocket.Conn
	var closer *websocketConnectionCloser
	var respHS *http.Response
	var errDial error
	dialCtx := ctx
	if cliproxyexecutor.RequiredUpstreamWebsocket(ctx) {
		identityKey, _ := helps.ManagedWebsocketKey(auth, codexWebsocketProxyURL(e.cfg, auth), wsURL, wsHeaders)
		conn, closer = existingWebsocketSessionConn(sess, authID, wsURL, codexWebsocketProxyURL(e.cfg, auth), identityKey)
		if conn == nil {
			return resp, cliproxyexecutor.NewUpstreamWebsocketReplayRequiredError()
		}
	} else {
		dialCtx = cliproxyexecutor.WithUpstreamAttemptTracker(ctx)
		conn, closer, respHS, errDial = e.ensureUpstreamConn(dialCtx, auth, sess, authID, wsURL, wsHeaders)
	}
	if errDial != nil {
		bodyErr := websocketHandshakeBody(respHS)
		if respHS != nil {
			helps.RecordAPIWebsocketUpgradeRejection(ctx, e.cfg, websocketUpgradeRequestLog(wsReqLog), respHS.StatusCode, respHS.Header.Clone(), bodyErr)
		}
		if respHS != nil && respHS.StatusCode == http.StatusUpgradeRequired {
			if opts.ExecutionLifecycle == nil && !cliproxyexecutor.DownstreamWebsocket(ctx) {
				if errFinish := continuityAttempt.Finish(ctx, errDial); errFinish != nil {
					return resp, errFinish
				}
				return e.CodexExecutor.Execute(ctx, auth, req, opts)
			}
			if cliproxyexecutor.UpstreamAttempted(dialCtx) {
				cliproxyexecutor.MarkUpstreamAttempt(ctx)
			}
			return resp, statusErr{code: respHS.StatusCode, msg: string(bodyErr)}
		}
		if cliproxyexecutor.UpstreamAttempted(dialCtx) {
			cliproxyexecutor.MarkUpstreamAttempt(ctx)
		}
		if respHS != nil && respHS.StatusCode > 0 {
			return resp, newCodexStatusErrWithCooling(respHS.StatusCode, bodyErr, e.modelLevelCooling())
		}
		helps.RecordAPIWebsocketError(ctx, e.cfg, "dial", errDial)
		return resp, errDial
	}
	if closer != nil {
		reporter.SetProxyRouteGetter(closer.proxyRouteGetter)
	}
	if errBind := sess.bindExecutionLifecycle(opts, conn, closer, req.Model); errBind != nil {
		unlockSession()
		closeWebsocketAfterBindFailure(sess, conn, closer)
		return resp, errBind
	}
	recordAPIWebsocketHandshake(ctx, e.cfg, respHS)
	if respHS != nil {
		if errObserve := continuityAttempt.Observe(ctx, respHS.Header, nil); errObserve != nil {
			unlockSession()
			closeWebsocketAfterBindFailure(sess, conn, closer)
			return resp, errObserve
		}
	}
	reporter.StartResponseTTFT()
	if sess == nil {
		logCodexWebsocketConnected(executionSessionID, authID, wsURL)
		defer func() {
			reason := "completed"
			if err != nil {
				reason = "error"
			}
			logCodexWebsocketDisconnected(executionSessionID, authID, wsURL, reason, err)
			if errClose := closer.Close(); errClose != nil {
				log.Errorf("codex websockets executor: close websocket error: %v", errClose)
			}
		}()
	}

	var readCh chan codexWebsocketRead
	if sess != nil {
		readCh = sess.activate(conn)
		defer func() {
			sess.clearActive(conn, readCh)
		}()
	}
	restoreMultiAgentV2 := !multiAgentV2Conflict && (optimizeMultiAgentV2 || sess.isMultiAgentV2Optimized(conn))

	reporter.SetUpstreamServiceTier(wsReqBody)
	if errSend := writeCodexWebsocketMessage(ctx, authID, sess, conn, wsReqBody); errSend != nil {
		errSend = mapCodexWebsocketWriteError(sess, conn, errSend)
		if sess != nil {
			if cliproxyexecutor.RequiredUpstreamWebsocket(ctx) {
				e.invalidateUpstreamConnWithoutDisconnectNotify(sess, conn, "send_error", errSend)
				if !shouldRetryCodexWebsocketSend(errSend) {
					helps.RecordAPIWebsocketError(ctx, e.cfg, "send", errSend)
					return resp, errSend
				}
				return resp, cliproxyexecutor.NewUpstreamWebsocketReplayRequiredError()
			}
			e.invalidateUpstreamConn(sess, conn, "send_error", errSend)
			if !shouldRetryCodexWebsocketSend(errSend) {
				helps.RecordAPIWebsocketError(ctx, e.cfg, "send", errSend)
				return resp, errSend
			}

			// Retry once with a fresh websocket connection. This is mainly to handle
			// upstream closing the socket between sequential requests within the same
			// execution session.
			reporter.SetProxyRouteGetter(nil)
			connRetry, closerRetry, respHSRetry, errDialRetry := e.ensureUpstreamConn(ctx, auth, sess, authID, wsURL, wsHeaders)
			if errDialRetry == nil && connRetry != nil {
				previousConn, previousReadCh := conn, readCh
				conn = connRetry
				closer = closerRetry
				if closer != nil {
					reporter.SetProxyRouteGetter(closer.proxyRouteGetter)
				}
				if errBind := sess.bindExecutionLifecycle(opts, conn, closer, req.Model); errBind != nil {
					clearRetryActiveState(sess, previousConn, previousReadCh)
					unlockSession()
					closeWebsocketAfterBindFailure(sess, conn, closer)
					return resp, errBind
				}
				readCh = sess.activate(conn)
				restoreMultiAgentV2 = !multiAgentV2Conflict && (optimizeMultiAgentV2 || sess.isMultiAgentV2Optimized(conn))
				if errFinish := continuityAttempt.Finish(ctx, errSend); errFinish != nil {
					clearRetryActiveState(sess, previousConn, previousReadCh)
					unlockSession()
					return resp, errFinish
				}
				var retryBindErr error
				continuityAttempt, wsHeaders, upstreamBody, retryBindErr = helps.BindCodexContinuity(ctx, auth, continuityHeaders, continuityInput)
				if retryBindErr != nil {
					clearRetryActiveState(sess, previousConn, previousReadCh)
					unlockSession()
					return resp, retryBindErr
				}
				wsReqBodyRetry := buildCodexWebsocketRequestBody(upstreamBody, ctx)
				helps.RecordAPIWebsocketRequest(ctx, e.cfg, helps.UpstreamRequestLog{
					URL:       wsURL,
					Method:    "WEBSOCKET",
					Headers:   wsHeaders.Clone(),
					Body:      wsReqBodyRetry,
					Provider:  e.Identifier(),
					AuthID:    authID,
					AuthLabel: authLabel,
					AuthType:  authType,
					AuthValue: authValue,
				})
				recordAPIWebsocketHandshake(ctx, e.cfg, respHSRetry)
				reporter.StartResponseTTFT()
				reporter.SetUpstreamServiceTier(wsReqBodyRetry)
				if errSendRetry := writeCodexWebsocketMessage(ctx, authID, sess, conn, wsReqBodyRetry); errSendRetry == nil {
					wsReqBody = wsReqBodyRetry
				} else {
					errSendRetry = mapCodexWebsocketWriteError(sess, connRetry, errSendRetry)
					e.invalidateUpstreamConn(sess, connRetry, "send_error", errSendRetry)
					helps.RecordAPIWebsocketError(ctx, e.cfg, "send_retry", errSendRetry)
					return resp, errSendRetry
				}
			} else {
				closeHTTPResponseBody(respHSRetry, "codex websockets executor: close handshake response body error")
				helps.RecordAPIWebsocketError(ctx, e.cfg, "dial_retry", errDialRetry)
				return resp, errDialRetry
			}
		} else {
			helps.RecordAPIWebsocketError(ctx, e.cfg, "send", errSend)
			return resp, errSend
		}
	}

	if optimizeMultiAgentV2 || multiAgentV2Conflict {
		sess.setMultiAgentV2Optimized(conn, optimizeMultiAgentV2 && !multiAgentV2Conflict)
	}
	plaintextPolicy := sess.plaintextPolicyForSentRequest(conn, wsReqBody, optimizeMultiAgentV2)
	helps.SetCodexMultiAgentV2PlaintextObserver(ctx, opts.Headers, continuityAttempt, plaintextPolicy, e.cfg)

	outputItemsByIndex := make(map[int64][]byte)
	var outputItemsFallback [][]byte
	sawOutputDelta := false
	replaySafe := true
	for {
		msgType, payload, errRead := readCodexWebsocketMessage(ctx, sess, conn, readCh)
		if errRead != nil {
			mappedErr := codexResponsesReadError(mapCodexWebsocketReadError(errRead))
			if sess != nil {
				defer sess.notifyUpstreamDisconnect(mappedErr)
			}
			helps.RecordAPIWebsocketError(ctx, e.cfg, "read", mappedErr)
			return resp, mappedErr
		}
		if msgType != websocket.TextMessage {
			if msgType == websocket.BinaryMessage {
				err = fmt.Errorf("codex websockets executor: unexpected binary message")
				if sess != nil {
					e.invalidateUpstreamConn(sess, conn, "unexpected_binary", err)
				}
				helps.RecordAPIWebsocketError(ctx, e.cfg, "unexpected_binary", err)
				return resp, err
			}
			continue
		}

		payload = bytes.TrimSpace(payload)
		if len(payload) == 0 {
			continue
		}
		if ctx.Err() != nil && !helps.CodexWebsocketFailureCandidate(payload) {
			continue
		}
		if ctx.Err() == nil {
			if errObserve := continuityAttempt.Observe(ctx, nil, payload); errObserve != nil {
				return resp, errObserve
			}
		}
		observeCodexTokenEvent(reporter, payload)
		payload = applyCodexIdentityConfuseResponsePayload(payload, identityState)
		helps.AppendCodexAPIWebsocketResponse(ctx, e.cfg, payload)
		helps.EmitWebSocketResponseEvent(ctx, opts, auth, e.Identifier(), req.Model, payload)
		payload = helps.RestoreCodexMultiAgentV2Response(payload, restoreMultiAgentV2)
		payload = helps.MarkCodexMultiAgentV2PlaintextResponse(ctx, opts.Headers, payload, plaintextPolicy, e.cfg)

		if wsErr, ok := parseCodexWebsocketErrorWithCooling(payload, e.modelLevelCooling()); ok {
			if sess != nil {
				e.invalidateUpstreamConn(sess, conn, "upstream_error", wsErr)
			}
			if errClearReplay := clearCodexReasoningReplayOnWebsocketError(ctx, replayScope, payload); errClearReplay != nil {
				return resp, errClearReplay
			}
			helps.RecordAPIWebsocketError(ctx, e.cfg, "upstream_error", wsErr)
			return resp, helps.CodexStopAfterExecution(wsErr, replaySafe && helps.CodexEventHasNoExecution(payload))
		}
		if streamErr, terminalBody, ok := codexTerminalFailureErrWithCooling(payload, e.modelLevelCooling()); ok {
			if sess != nil {
				unlockSession()
				e.invalidateUpstreamConn(sess, conn, "terminal_failure", streamErr)
			}
			if errClearReplay := clearCodexReasoningReplayOnInvalidSignature(ctx, replayScope, streamErr.StatusCode(), terminalBody); errClearReplay != nil {
				return resp, errClearReplay
			}
			return resp, helps.CodexStopAfterExecution(streamErr, replaySafe && helps.CodexEventHasNoExecution(payload))
		}
		replaySafe = replaySafe && helps.CodexHandshakeMetadataEvent(payload)

		if ctx.Err() != nil {
			continue
		}
		payload = normalizeCodexWebsocketCompletion(payload)
		eventType := gjson.GetBytes(payload, "type").String()
		if helps.HasMeaningfulCodexOutputDelta(payload) {
			sawOutputDelta = true
		}
		switch eventType {
		case "response.output_item.done":
			collectCodexOutputItemDone(payload, outputItemsByIndex, &outputItemsFallback)
		case "response.completed", "response.done", "response.incomplete":
			if helps.IsCodexTerminalEmptyIncomplete(payload, len(outputItemsByIndex)+len(outputItemsFallback), sawOutputDelta) {
				streamErr := newCodexEmptyIncompleteStreamError()
				reporter.PublishFailure(ctx, streamErr)
				return resp, streamErr
			}
			payload = patchCodexCompletedOutput(payload, outputItemsByIndex, outputItemsFallback)
			if eventType != "response.incomplete" {
				cacheCodexReasoningReplayFromCompleted(replayScope, payload)
			}
			if detail, ok := helps.ParseCodexUsage(payload); ok {
				reporter.Publish(ctx, detail)
			} else {
				reporter.EnsurePublished(ctx)
			}
			var param any
			clientPayload := applyCodexIdentityExposeResponsePayload(payload, identityState)
			out := sdktranslator.TranslateNonStream(ctx, to, responseFormat, req.Model, originalPayload, clientBody, clientPayload, &param)
			if responseFormat == sdktranslator.FormatOpenAIResponse {
				out = helps.EnsureResponsesUsageDetails(out)
			}
			resp = cliproxyexecutor.Response{Payload: out}
			return resp, nil
		}
	}
}
