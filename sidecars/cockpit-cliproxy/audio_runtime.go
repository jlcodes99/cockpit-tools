package main

import (
	"bytes"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

type audioRelayError struct {
	status       int
	code, detail string
}

func (e *audioRelayError) Error() string { return e.code + ": " + e.detail }

func invalidAudio(detail string) error {
	return &audioRelayError{http.StatusBadRequest, "audio_invalid_request", detail}
}

type audioResponseBody struct {
	io.ReadCloser
	once   sync.Once
	finish func()
}

func (b *audioResponseBody) Close() error {
	err := b.ReadCloser.Close()
	b.once.Do(b.finish)
	return err
}

func (s *relayServer) forwardAudio(c *gin.Context, spec *apiKeySpec, a *audioRequest) (*http.Response, bool, error) {
	gateway := spec.ProviderGateway
	model := stripModelPrefix(a.model, spec)
	if gateway == nil && model != "" && spec.ModelRouting != nil {
		route, upstreamModel, status := resolveModelRoutingRoute(spec, model)
		if status == "matched" {
			gateway, model = route.ProviderGateway, upstreamModel
			c.Request = c.Request.WithContext(bindProviderGatewayAccount(c.Request.Context(), route.ProviderAccountID))
		} else if strings.Contains(model, "/") {
			// An explicit namespace must never fall back to a different account.
			return nil, false, invalidAudio("audio_route_not_available")
		}
	}
	if gateway != nil {
		return s.forwardProviderAudio(c, gateway, a, model)
	}
	if s.authManager == nil {
		return nil, false, &audioRelayError{http.StatusServiceUnavailable, "audio_unavailable", ""}
	}
	ctx := relayContext(c)
	kind := coreauth.AuthKindOAuth
	// Keep OAuth-only bindings intact. Speech and translation require API-key
	// credentials; ordinary transcription retains its OAuth preference.
	if !spec.BoundOAuth {
		candidates := (&cockpitSelector{manifest: s.manifest}).filterAuthsForAPIKeyScope(ctx, s.authManager.List())
		hasOAuth, hasAPIKey := false, false
		for _, auth := range candidates {
			if auth == nil || auth.Provider != "codex" {
				continue
			}
			hasOAuth = hasOAuth || auth.AuthKind() == coreauth.AuthKindOAuth
			hasAPIKey = hasAPIKey || auth.AuthKind() == coreauth.AuthKindAPIKey
		}
		if hasAPIKey && (!hasOAuth || a.operation != "transcriptions") {
			kind = coreauth.AuthKindAPIKey
		}
	}
	opts := cliproxyexecutor.Options{Headers: c.Request.Header.Clone(), Metadata: map[string]any{
		cliproxyexecutor.RequestPathMetadataKey: c.Request.URL.Path,
	}}
	// Audio capabilities are independent of the registered coding-model catalog.
	// The selector still enforces API-key account scope, quota and concurrency.
	selected, err := s.authManager.SelectAuthByKind(ctx, "codex", "", kind, opts)
	if err != nil {
		return nil, false, err
	}
	if selected == nil {
		return nil, false, &audioRelayError{http.StatusServiceUnavailable, "audio_unavailable", ""}
	}
	// A late Retry-After belongs to the credential that sent the request.
	backoffKey := providerGatewayBackoffKey{selected.ID,
		fmt.Sprintf("audio:%d:%d", selected.CredentialVersion, selected.RegistrationEpoch), a.operation}
	if s.rejectProviderBackoff(c, backoffKey) {
		return nil, false, nil
	}
	oauth := kind == coreauth.AuthKindOAuth
	upstreamURL := ""
	if oauth {
		if a.operation != "transcriptions" {
			return nil, true, &audioRelayError{http.StatusBadRequest, "audio_not_supported", ""}
		}
		if !a.native {
			if a.format != "json" && a.format != "text" {
				return nil, true, invalidAudio("response_format_requires_api_key")
			}
			if detail := a.rewriteMultipart("", true); detail != "" {
				return nil, true, invalidAudio(detail)
			}
		}
		upstreamURL = codexTranscribeURL(selected)
	} else {
		if err := prepareProviderAudio(a, model); err != nil {
			return nil, false, err
		}
		base := strings.TrimSpace(selected.Attributes["base_url"])
		if base == "" {
			base = "https://api.openai.com/v1"
		}
		upstreamURL, err = providerGatewayURL(base, "/v1/audio/"+a.operation)
		if err != nil {
			return nil, false, invalidAudio("invalid_base_url")
		}
	}
	headers := buildCodexAlphaSearchHeaders(c.Request.Header, selected)
	headers.Set("Content-Type", a.contentType)
	if accept := c.GetHeader("Accept"); accept != "" {
		headers.Set("Accept", accept)
	}
	if !oauth {
		headers.Del("Chatgpt-Account-Id")
	}
	req, err := s.authManager.NewHttpRequest(ctx, selected, http.MethodPost, upstreamURL, a.body, headers)
	if err != nil {
		return nil, oauth, err
	}
	if oauth {
		if id := codexAuthChatGPTAccountID(selected); id != "" {
			req.Header.Set("Chatgpt-Account-Id", id)
		}
	}
	// Record transport metadata, never save uploaded audio in request snapshots.
	observeDirectGatewayRequest(ctx, selected.ID, a.model, req.Header, nil)
	resp, err := s.authManager.HttpRequest(ctx, selected, req)
	if err != nil {
		if observer := requestDiagnosticsFromContext(ctx); observer != nil {
			observer.UpstreamError(ctx, "dial", err)
		}
		return nil, oauth, &audioRelayError{http.StatusBadGateway, "audio_upstream_failed", ""}
	}
	s.providerBackoff.observe(backoffKey, resp.StatusCode, resp.Header.Get("Retry-After"), time.Now())
	if resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden || resp.StatusCode == http.StatusTooManyRequests {
		result := coreauth.Result{AuthID: selected.ID, CredentialVersion: selected.CredentialVersion,
			RegistrationEpoch: selected.RegistrationEpoch, Provider: "codex", Model: "codex-transcribe", Options: opts,
			Error: &coreauth.Error{Code: "audio_upstream_error", Message: http.StatusText(resp.StatusCode), HTTPStatus: resp.StatusCode}, SkipQuotaObservation: true}
		if retry := parseProviderRetryAfter(resp.Header.Get("Retry-After"), time.Now()); retry > 0 {
			result.RetryAfter = &retry
		}
		s.authManager.MarkResult(ctx, result)
	}
	return resp, oauth, nil
}

func prepareProviderAudio(a *audioRequest, model string) error {
	if model == "" {
		if !a.native {
			return invalidAudio("model_required")
		}
		model = defaultTranscriptionModel
	}
	if a.operation != "speech" {
		if detail := a.rewriteMultipart(model, false); detail != "" {
			return invalidAudio(detail)
		}
	} else {
		a.body = rewriteProviderGatewayBodyModel(a.body, model)
	}
	return nil
}

func codexTranscribeURL(auth *coreauth.Auth) string {
	base := strings.TrimRight(strings.TrimSpace(auth.Attributes["base_url"]), "/")
	if base == "" {
		return "https://chatgpt.com/backend-api/transcribe"
	}
	if strings.HasSuffix(base, "/transcribe") {
		return base
	}
	return strings.TrimSuffix(base, "/codex") + "/transcribe"
}

func (s *relayServer) forwardProviderAudio(c *gin.Context, gateway *providerGatewaySpec, a *audioRequest, model string) (*http.Response, bool, error) {
	if err := prepareProviderAudio(a, model); err != nil {
		return nil, false, err
	}
	target, err := providerGatewayURL(gateway.BaseURL, "/v1/audio/"+a.operation)
	if err != nil {
		return nil, false, invalidAudio("invalid_base_url")
	}
	key := providerGatewayBackoffKey{providerGatewayBoundAccount(c), strings.TrimRight(gateway.BaseURL, "/"), model}
	release, admitted := s.admitProviderGateway(c, key)
	if !admitted {
		return nil, false, nil
	}
	req, err := http.NewRequestWithContext(relayContext(c), http.MethodPost, target, bytes.NewReader(a.body))
	if err != nil {
		release()
		return nil, false, invalidAudio("invalid_base_url")
	}
	req.Header.Set("Authorization", "Bearer "+gateway.APIKey)
	req.Header.Set("Content-Type", a.contentType)
	if accept := c.GetHeader("Accept"); accept != "" {
		req.Header.Set("Accept", accept)
	}
	copyProviderGatewayDiagnosticHeaders(req.Header, c.Request.Header)
	observeDirectGatewayRequest(req.Context(), key.account, model, req.Header, nil)
	client := &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	resp, err := client.Do(req)
	if err != nil {
		release()
		if observer := requestDiagnosticsFromContext(req.Context()); observer != nil {
			observer.UpstreamError(req.Context(), "dial", err)
		}
		return nil, false, &audioRelayError{http.StatusBadGateway, "audio_upstream_failed", ""}
	}
	s.providerBackoff.observe(key, resp.StatusCode, resp.Header.Get("Retry-After"), time.Now())
	resp.Body = &audioResponseBody{ReadCloser: resp.Body, finish: release}
	return resp, false, nil
}

// Keep the interface assertion near the lifecycle wrapper.
var _ io.ReadCloser = (*audioResponseBody)(nil)
