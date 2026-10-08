package main

import (
	"bytes"
	"fmt"
	"net/http"
	"strings"

	"github.com/gin-gonic/gin"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func automaticNativeModel(spec *apiKeySpec, model string) bool {
	if spec == nil || spec.ModelRouting == nil {
		return false
	}
	model = stripModelPrefix(model, spec)
	if automaticListedModel(spec, model) {
		return true
	}
	// RoutableModels 不展示在选择器里，但仍按原生模型路由。
	for _, candidate := range spec.ModelRouting.RoutableModels {
		if strings.EqualFold(candidate, model) {
			return true
		}
	}
	return false
}

// automaticListedModel 判断模型是否属于客户端展示清单（nativeModels）。
// routableModels 只参与路由与校验，不参与展示。
func automaticListedModel(spec *apiKeySpec, model string) bool {
	if spec == nil || spec.ModelRouting == nil {
		return false
	}
	model = stripModelPrefix(model, spec)
	for _, candidate := range spec.ModelRouting.NativeModels {
		if strings.EqualFold(candidate, model) {
			return true
		}
	}
	return false
}

func routedClientModelAllowed(m *manifest, spec *apiKeySpec, model string) bool {
	model = stripModelPrefix(model, spec)
	if m != nil && modelMatchesAnyRule(model, m.ExcludedModels) {
		return false
	}
	if spec == nil {
		return true
	}
	return (len(spec.AllowedModels) == 0 || modelMatchesAnyRule(model, spec.AllowedModels)) && !modelMatchesAnyRule(model, spec.ExcludedModels)
}

func automaticClientModelVisible(m *manifest, spec *apiKeySpec, model string) bool {
	if !routedClientModelAllowed(m, spec, model) {
		return false
	}
	// 展示清单之外的 routable 模型（唤醒预设、历史兼容模型）仍然允许请求。
	if automaticNativeModel(spec, model) {
		return true
	}
	for _, visible := range visibleModelsForAPIKey(m, spec) {
		if strings.EqualFold(strings.TrimSpace(model), visible) || strings.EqualFold(stripModelPrefix(model, spec), stripModelPrefix(visible, spec)) {
			return true
		}
	}
	return false
}

type automaticModelCandidate struct {
	route    *modelRouteSpec
	upstream string
	auth     *coreauth.Auth
}

func (s *relayServer) automaticCandidates(spec *apiKeySpec, model string) []automaticModelCandidate {
	var candidates []automaticModelCandidate
	if spec == nil || spec.ModelRouting == nil || s.manifest == nil {
		return candidates
	}
	model = stripModelPrefix(model, spec)
	seen := make(map[string]bool)
	for i := range spec.ModelRouting.Routes {
		route := &spec.ModelRouting.Routes[i]
		accountID := strings.TrimSpace(route.ProviderAccountID)
		account := s.manifest.accountByID[accountID]
		if route.ProviderGateway == nil || account == nil || seen[accountID] {
			continue
		}
		for _, mapping := range route.Models {
			if !strings.EqualFold(mapping.ClientModel, model) {
				continue
			}
			// Use the business ID as the synthetic auth identity; account policy is
			// resolved from account_id, including IDs containing dots or slashes.
			auth := &coreauth.Auth{ID: "cockpit-provider:" + accountID, Provider: "codex", Status: coreauth.StatusActive, Attributes: map[string]string{"account_id": accountID}}
			if authModelExcluded(s.manifest, auth, model) || authModelExcluded(s.manifest, auth, mapping.UpstreamModel) {
				continue
			}
			seen[accountID] = true
			candidates = append(candidates, automaticModelCandidate{route: route, upstream: mapping.UpstreamModel, auth: auth})
			break
		}
	}
	return candidates
}

func (s *relayServer) providerRouteSelector() coreauth.Selector {
	s.automaticSelectorOnce.Do(func() {
		if s.automaticSelector != nil {
			return
		}
		var tracker *requestUsageTracker
		if s.policy != nil {
			tracker = s.policy.tracker
		}
		base := &cockpitSelector{manifest: s.manifest, emitter: s.emitter, tracker: tracker}
		if s.manifest != nil {
			base.locale = normalizeCockpitLocale(s.manifest.Locale)
		}
		s.automaticSelector = buildCoreAuthSelectorWithConcurrency(nil, base, s.manifest, nil, tracker)
	})
	return s.automaticSelector
}

func retryableAutomaticStatus(status int) bool {
	return status == http.StatusUnauthorized || status == http.StatusPaymentRequired || status == http.StatusForbidden || status == http.StatusRequestTimeout || status == http.StatusTooManyRequests || status >= http.StatusInternalServerError
}

// A failed upstream attempt is held until a same-model candidate succeeds.
// Successful output passes straight through, including the first SSE byte;
// once that happens it is never replayed against another provider.
func (s *relayServer) handleAutomaticModelRequest(c *gin.Context, spec *apiKeySpec, body []byte, model string, sourceFormat sdktranslator.Format, fixedAlt string) {
	if !automaticClientModelVisible(s.manifest, spec, model) {
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("model %s is not available for this API key", model), "model_not_available")
		return
	}
	candidates := s.automaticCandidates(spec, model)
	originalWriter := c.Writer
	defer func() { c.Writer = originalWriter }()
	var last *automaticAttemptWriter
	if automaticNativeModel(spec, model) {
		last = newAutomaticAttemptWriter(originalWriter)
		c.Writer = last
		canonical := canonicalModelForClientModel(s.manifest, spec, model)
		if strings.TrimSpace(canonical) == "" {
			// 原生（订阅）路由不经过 provider gateway 的模型清单，保留客户端模型名。
			canonical = stripModelPrefix(model, spec)
		}
		nativeBody := rewriteProviderGatewayBodyModel(body, canonical)
		alt := fixedAlt
		if alt == "" {
			alt = requestAlt(c)
		}
		if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI) && isGPTImageGenerationModel(canonical) {
			writeAPIError(c, http.StatusBadRequest, "This model is not supported on the Chat Completions endpoint", "invalid_request")
		} else if requestBodyStream(body) && fixedAlt != "responses/compact" {
			s.handleStream(c, nativeBody, canonical, sourceFormat, alt, executionProviders())
		} else {
			s.handleNonStream(c, nativeBody, canonical, sourceFormat, alt, executionProviders())
		}
		if originalWriter.Written() || !retryableAutomaticStatus(last.Status()) || c.Request.Context().Err() != nil || fixedAlt == "responses/compact" {
			last.commit()
			return
		}
		s.releaseAutomaticAccountSlots(c)
	}
	selector := s.providerRouteSelector()
	for len(candidates) > 0 && c.Request.Context().Err() == nil {
		auths := make([]*coreauth.Auth, 0, len(candidates))
		for _, candidate := range candidates {
			auths = append(auths, candidate.auth)
		}
		selected, err := selector.Pick(c.Request.Context(), "codex", stripModelPrefix(model, spec), cliproxyexecutor.Options{}, auths)
		if err != nil || selected == nil {
			if last == nil {
				c.Writer = originalWriter
				if err == nil {
					err = fmt.Errorf("no account is available for model %s", model)
				}
				s.writeExecutorError(c, err)
			}
			break
		}
		index := -1
		for i := range candidates {
			if candidates[i].auth.ID == selected.ID {
				index = i
				break
			}
		}
		if index < 0 {
			break
		}
		candidate := candidates[index]
		candidates = append(candidates[:index], candidates[index+1:]...)
		last = newAutomaticAttemptWriter(originalWriter)
		c.Writer = last
		// Clear errors from a rejected native/candidate attempt before the next one.
		c.Errors = nil
		if s.policy != nil && s.policy.tracker != nil {
			s.policy.tracker.recordSelectedAccount(internallogging.GetRequestID(c.Request.Context()), s.manifest.accountByID[candidate.route.ProviderAccountID], selected.ID)
		}
		s.handleProviderGatewayRequest(c, candidate.route.ProviderGateway, body, candidate.upstream, sourceFormat, fixedAlt)
		if originalWriter.Written() || !retryableAutomaticStatus(last.Status()) {
			last.commit()
			return
		}
		s.releaseAutomaticAccountSlots(c)
	}
	if last != nil {
		last.commit()
	} else if !originalWriter.Written() {
		c.Writer = originalWriter
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("no route is available for model %s", model), "model_route_not_available")
	}
}

func (s *relayServer) releaseAutomaticAccountSlots(c *gin.Context) {
	if s.policy != nil && s.policy.tracker != nil {
		s.policy.tracker.releaseAccountSlots(internallogging.GetRequestID(c.Request.Context()))
	}
}

// Only error responses are buffered, with a strict size bound. Successful
// streaming remains incremental and preserves back pressure and cancellation.
type automaticAttemptWriter struct {
	gin.ResponseWriter
	headers    http.Header
	status     int
	size       int
	failedBody bytes.Buffer
	committed  bool
}

func newAutomaticAttemptWriter(writer gin.ResponseWriter) *automaticAttemptWriter {
	return &automaticAttemptWriter{ResponseWriter: writer, headers: writer.Header().Clone(), status: http.StatusOK, size: -1}
}
func (w *automaticAttemptWriter) Header() http.Header { return w.headers }
func (w *automaticAttemptWriter) Status() int         { return w.status }
func (w *automaticAttemptWriter) Size() int           { return w.size }
func (w *automaticAttemptWriter) Written() bool       { return w.size >= 0 || w.ResponseWriter.Written() }
func (w *automaticAttemptWriter) WriteHeader(status int) {
	if !w.Written() {
		w.status = status
	}
}
func (w *automaticAttemptWriter) WriteHeaderNow() {
	if w.status >= 400 && !w.committed {
		if w.size < 0 {
			w.size = 0
		}
		return
	}
	w.commitHeaders()
	w.ResponseWriter.WriteHeaderNow()
	if w.size < 0 {
		w.size = 0
	}
}
func (w *automaticAttemptWriter) Write(body []byte) (int, error) {
	if w.status >= 400 && !w.committed {
		if w.size < 0 {
			w.size = 0
		}
		w.size += len(body)
		if w.failedBody.Len()+len(body) <= 1024*1024 {
			return w.failedBody.Write(body)
		}
		// Oversized errors are terminal: flush the bounded prefix and continue
		// directly instead of retaining unbounded error bodies or retrying.
		w.commit()
		return w.ResponseWriter.Write(body)
	}
	w.commitHeaders()
	n, err := w.ResponseWriter.Write(body)
	if w.size < 0 {
		w.size = 0
	}
	w.size += n
	return n, err
}
func (w *automaticAttemptWriter) WriteString(body string) (int, error) { return w.Write([]byte(body)) }
func (w *automaticAttemptWriter) Flush() {
	if w.status < 400 || w.committed {
		w.WriteHeaderNow()
		w.ResponseWriter.Flush()
	}
}
func (w *automaticAttemptWriter) commitHeaders() {
	if w.committed {
		return
	}
	dst := w.ResponseWriter.Header()
	for key := range dst {
		delete(dst, key)
	}
	for key, values := range w.headers {
		dst[key] = append([]string(nil), values...)
	}
	w.ResponseWriter.WriteHeader(w.status)
	w.committed = true
}
func (w *automaticAttemptWriter) commit() {
	if w.committed {
		return
	}
	w.commitHeaders()
	if w.failedBody.Len() > 0 {
		_, _ = w.ResponseWriter.Write(w.failedBody.Bytes())
	} else {
		w.ResponseWriter.WriteHeaderNow()
	}
}
