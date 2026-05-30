package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"mime/multipart"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/signal"
	"path/filepath"
	goruntime "runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/gin-gonic/gin"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/watcher/synthesizer"
	sdkauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/proxyutil"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	_ "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator/builtin"
)

type contextKey string

const (
	clientAPIKeyContextKey contextKey = "cockpitClientAPIKey"
	requestKindContextKey  contextKey = "cockpitRequestKind"
	requestModelContextKey contextKey = "cockpitRequestModel"
)

const ginUserAPIKeyKey = "userApiKey"

const defaultStreamKeepAliveSeconds = 15
const codexAutoReviewModel = "codex-auto-review"
const defaultImagesMainModel = "gpt-5.4-mini"
const defaultImagesToolModel = "gpt-image-2"
const imagesGenerationsPath = "/v1/images/generations"
const imagesEditsPath = "/v1/images/edits"
const maxImageUploadBytes int64 = 64 * 1024 * 1024

var (
	streamOpenTimeout      = 10 * time.Second
	streamOpenMaxAttempts  = 2
	streamIdleTimeout      = 60 * time.Second
	imageStreamOpenTimeout = 10 * time.Second
	imageStreamIdleTimeout = 60 * time.Second
)

type manifest struct {
	APIKeys            []apiKeySpec        `json:"apiKeys"`
	Accounts           []accountSpec       `json:"accounts"`
	ModelIDs           []string            `json:"modelIds"`
	ModelAliases       []modelAliasSpec    `json:"modelAliases"`
	ExcludedModels     []string            `json:"excludedModels"`
	RoutingStrategy    string              `json:"routingStrategy"`
	CustomRoutingRules []customRoutingRule `json:"customRoutingRules"`
	DebugLogs          *bool               `json:"debugLogs,omitempty"`

	apiKeyByValue     map[string]*apiKeySpec
	accountByID       map[string]*accountSpec
	accountByAuthID   map[string]*accountSpec
	accountByAPIKey   map[string]*accountSpec
	aliasToSource     map[string]string
	originalIndexByID map[string]int
}

type apiKeySpec struct {
	ID             string   `json:"id"`
	Label          string   `json:"label"`
	Key            string   `json:"key"`
	ModelPrefix    string   `json:"modelPrefix,omitempty"`
	AllowedModels  []string `json:"allowedModels"`
	ExcludedModels []string `json:"excludedModels"`
	Enabled        bool     `json:"enabled"`
}

type accountSpec struct {
	ID                   string `json:"id"`
	Email                string `json:"email"`
	AuthID               string `json:"authId,omitempty"`
	UpstreamAPIKey       string `json:"upstreamApiKey,omitempty"`
	PlanRank             *int   `json:"planRank,omitempty"`
	RemainingQuota       *int   `json:"remainingQuota,omitempty"`
	SubscriptionExpiryMS *int64 `json:"subscriptionExpiryMs,omitempty"`
}

type modelAliasSpec struct {
	SourceModel string `json:"sourceModel"`
	Alias       string `json:"alias"`
	Fork        bool   `json:"fork"`
}

type customRoutingRule struct {
	AccountID string `json:"accountId"`
	Priority  int    `json:"priority"`
	Weight    int    `json:"weight"`
}

type usagePayload struct {
	Type          string       `json:"type"`
	RequestID     string       `json:"requestId,omitempty"`
	Provider      string       `json:"provider,omitempty"`
	Model         string       `json:"model,omitempty"`
	Alias         string       `json:"alias,omitempty"`
	AccountID     string       `json:"accountId,omitempty"`
	AccountEmail  string       `json:"accountEmail,omitempty"`
	AuthID        string       `json:"authId,omitempty"`
	APIKeyID      string       `json:"apiKeyId,omitempty"`
	APIKeyLabel   string       `json:"apiKeyLabel,omitempty"`
	RequestKind   string       `json:"requestKind,omitempty"`
	Success       bool         `json:"success"`
	Status        int          `json:"status,omitempty"`
	ErrorCategory string       `json:"errorCategory,omitempty"`
	ErrorMessage  string       `json:"errorMessage,omitempty"`
	LatencyMS     int64        `json:"latencyMs,omitempty"`
	Usage         usageDetails `json:"usage"`
	RequestedAtMS int64        `json:"requestedAtMs,omitempty"`
}

type requestDiagnosticPayload struct {
	Type            string `json:"type"`
	RequestID       string `json:"requestId,omitempty"`
	Method          string `json:"method,omitempty"`
	Path            string `json:"path,omitempty"`
	RequestKind     string `json:"requestKind,omitempty"`
	Model           string `json:"model,omitempty"`
	APIKeyID        string `json:"apiKeyId,omitempty"`
	APIKeyLabel     string `json:"apiKeyLabel,omitempty"`
	Transport       string `json:"transport,omitempty"`
	Status          int    `json:"status,omitempty"`
	LatencyMS       int64  `json:"latencyMs,omitempty"`
	StartedAtMS     int64  `json:"startedAtMs,omitempty"`
	CompletedAtMS   int64  `json:"completedAtMs,omitempty"`
	Aborted         bool   `json:"aborted,omitempty"`
	ErrorMessage    string `json:"errorMessage,omitempty"`
	CandidateAuths  int    `json:"candidateAuths,omitempty"`
	AvailableAuths  int    `json:"availableAuths,omitempty"`
	RoutingStrategy string `json:"routingStrategy,omitempty"`
	Provider        string `json:"provider,omitempty"`
	AuthID          string `json:"authId,omitempty"`
	AccountID       string `json:"accountId,omitempty"`
	AccountEmail    string `json:"accountEmail,omitempty"`
	Success         *bool  `json:"success,omitempty"`
	ErrorCode       string `json:"errorCode,omitempty"`
	HTTPStatus      int    `json:"httpStatus,omitempty"`
	Retryable       *bool  `json:"retryable,omitempty"`
	RetryAfterMS    int64  `json:"retryAfterMs,omitempty"`
}

const executorWaitLogInterval = 30 * time.Second

type relayTimeoutError struct {
	phase   string
	timeout time.Duration
}

func (e relayTimeoutError) Error() string {
	if e.phase == "" {
		return fmt.Sprintf("upstream timed out after %s", e.timeout)
	}
	return fmt.Sprintf("upstream timed out in %s after %s", e.phase, e.timeout)
}

func (e relayTimeoutError) StatusCode() int {
	return http.StatusGatewayTimeout
}

type relayStatusError struct {
	status  int
	message string
}

func (e relayStatusError) Error() string {
	return e.message
}

func (e relayStatusError) StatusCode() int {
	if e.status > 0 {
		return e.status
	}
	return http.StatusBadGateway
}

type usageDetails struct {
	InputTokens     int64 `json:"inputTokens,omitempty"`
	OutputTokens    int64 `json:"outputTokens,omitempty"`
	ReasoningTokens int64 `json:"reasoningTokens,omitempty"`
	CachedTokens    int64 `json:"cachedTokens,omitempty"`
	TotalTokens     int64 `json:"totalTokens,omitempty"`
}

type usageFinalizeInput struct {
	spec          *apiKeySpec
	requestKind   string
	model         string
	status        int
	latencyMS     int64
	completedAtMS int64
	errorMessage  string
}

type requestUsageTracker struct {
	mu      sync.Mutex
	records map[string][]usagePayload
}

func newRequestUsageTracker() *requestUsageTracker {
	return &requestUsageTracker{records: make(map[string][]usagePayload)}
}

func (t *requestUsageTracker) record(payload usagePayload) {
	if t == nil {
		return
	}
	requestID := strings.TrimSpace(payload.RequestID)
	if requestID == "" {
		return
	}
	payload.Type = "usage"
	t.mu.Lock()
	t.records[requestID] = append(t.records[requestID], payload)
	t.mu.Unlock()
}

func (t *requestUsageTracker) finalize(requestID string, input usageFinalizeInput) (usagePayload, bool) {
	requestID = strings.TrimSpace(requestID)
	if requestID == "" {
		return usagePayload{}, false
	}

	var records []usagePayload
	if t != nil {
		t.mu.Lock()
		records = append(records, t.records[requestID]...)
		delete(t.records, requestID)
		t.mu.Unlock()
	}

	var payload usagePayload
	if len(records) > 0 {
		payload = records[len(records)-1]
		for i := len(records) - 1; i >= 0; i-- {
			if records[i].Success {
				payload = records[i]
				break
			}
		}
	} else {
		payload = usagePayload{
			Type:          "usage",
			RequestID:     requestID,
			Model:         strings.TrimSpace(input.model),
			APIKeyID:      stringFromAPIKey(input.spec, "id"),
			APIKeyLabel:   stringFromAPIKey(input.spec, "label"),
			RequestKind:   strings.TrimSpace(input.requestKind),
			RequestedAtMS: input.completedAtMS,
		}
	}

	payload.Type = "usage"
	payload.RequestID = requestID
	if strings.TrimSpace(payload.Model) == "" {
		payload.Model = strings.TrimSpace(input.model)
	}
	if strings.TrimSpace(payload.APIKeyID) == "" {
		payload.APIKeyID = stringFromAPIKey(input.spec, "id")
	}
	if strings.TrimSpace(payload.APIKeyLabel) == "" {
		payload.APIKeyLabel = stringFromAPIKey(input.spec, "label")
	}
	if strings.TrimSpace(payload.RequestKind) == "" {
		payload.RequestKind = strings.TrimSpace(input.requestKind)
	}
	if input.status > 0 {
		payload.Status = input.status
	}
	if input.latencyMS >= 0 {
		payload.LatencyMS = input.latencyMS
	}
	if payload.RequestedAtMS <= 0 {
		payload.RequestedAtMS = input.completedAtMS
	}

	finalHTTPFailed := input.status >= http.StatusBadRequest
	if finalHTTPFailed {
		payload.Success = false
		if strings.TrimSpace(payload.ErrorCategory) == "" {
			payload.ErrorCategory = errorCategory(input.status, input.errorMessage, false)
		}
		if strings.TrimSpace(payload.ErrorMessage) == "" {
			payload.ErrorMessage = strings.TrimSpace(input.errorMessage)
		}
		return payload, true
	}

	if len(records) == 0 {
		payload.Success = true
		payload.ErrorCategory = ""
		payload.ErrorMessage = ""
		return payload, true
	}
	if payload.Success {
		payload.ErrorCategory = ""
		payload.ErrorMessage = ""
	}
	return payload, true
}

type eventEmitter struct {
	mu sync.Mutex
}

func (e *eventEmitter) emit(v any) {
	data, err := json.Marshal(v)
	if err != nil {
		return
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	fmt.Println(string(data))
}

func loadManifest(path string) (*manifest, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var m manifest
	if err := json.Unmarshal(data, &m); err != nil {
		return nil, err
	}
	m.apiKeyByValue = make(map[string]*apiKeySpec)
	for i := range m.APIKeys {
		key := strings.TrimSpace(m.APIKeys[i].Key)
		if key == "" || !m.APIKeys[i].Enabled {
			continue
		}
		m.APIKeys[i].Key = key
		m.apiKeyByValue[key] = &m.APIKeys[i]
	}
	m.accountByID = make(map[string]*accountSpec)
	m.accountByAuthID = make(map[string]*accountSpec)
	m.accountByAPIKey = make(map[string]*accountSpec)
	m.originalIndexByID = make(map[string]int)
	for i := range m.Accounts {
		account := &m.Accounts[i]
		account.ID = strings.TrimSpace(account.ID)
		if account.ID == "" {
			continue
		}
		m.accountByID[account.ID] = account
		m.originalIndexByID[account.ID] = i
		if authID := strings.TrimSpace(account.AuthID); authID != "" {
			account.AuthID = authID
			m.accountByAuthID[strings.ToLower(authID)] = account
		}
		if key := strings.TrimSpace(account.UpstreamAPIKey); key != "" {
			account.UpstreamAPIKey = key
			m.accountByAPIKey[key] = account
		}
	}
	m.aliasToSource = make(map[string]string)
	for _, alias := range m.ModelAliases {
		source := strings.TrimSpace(alias.SourceModel)
		name := strings.TrimSpace(alias.Alias)
		if source == "" || name == "" {
			continue
		}
		m.aliasToSource[strings.ToLower(name)] = source
	}
	m.ModelIDs = normalizeStringList(m.ModelIDs)
	m.ExcludedModels = normalizeStringList(m.ExcludedModels)
	return &m, nil
}

func normalizeStringList(values []string) []string {
	seen := make(map[string]struct{}, len(values))
	out := make([]string, 0, len(values))
	for _, value := range values {
		trimmed := strings.TrimSpace(value)
		if trimmed == "" {
			continue
		}
		key := strings.ToLower(trimmed)
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = struct{}{}
		out = append(out, trimmed)
	}
	return out
}

func extractClientAPIKey(r *http.Request) string {
	if r == nil {
		return ""
	}
	authHeader := strings.TrimSpace(r.Header.Get("Authorization"))
	apiKey := extractBearerToken(authHeader)
	candidates := []string{
		apiKey,
		strings.TrimSpace(r.Header.Get("X-Goog-Api-Key")),
		strings.TrimSpace(r.Header.Get("X-Api-Key")),
	}
	if r.URL != nil {
		candidates = append(candidates, strings.TrimSpace(r.URL.Query().Get("key")))
		candidates = append(candidates, strings.TrimSpace(r.URL.Query().Get("auth_token")))
	}
	for _, candidate := range candidates {
		if strings.TrimSpace(candidate) != "" {
			return strings.TrimSpace(candidate)
		}
	}
	return ""
}

func extractBearerToken(header string) string {
	if header == "" {
		return ""
	}
	parts := strings.SplitN(header, " ", 2)
	if len(parts) != 2 {
		return strings.TrimSpace(header)
	}
	if !strings.EqualFold(parts[0], "bearer") {
		return strings.TrimSpace(header)
	}
	return strings.TrimSpace(parts[1])
}

type requestPolicy struct {
	manifest *manifest
	emitter  *eventEmitter
	tracker  *requestUsageTracker
}

func (p *requestPolicy) middleware() gin.HandlerFunc {
	return func(c *gin.Context) {
		if c.Request == nil || c.Request.Method == http.MethodOptions {
			c.Next()
			return
		}

		startedAt := time.Now()
		requestID := ensureRequestID(c)
		spec := p.lookupAPIKey(c.Request)
		requestKind := requestKindFromPath(c.Request.URL.Path)
		model := ""
		startLogged := false
		emitStart := func() {
			if startLogged || !shouldEmitRequestDiagnostic(c.Request) {
				return
			}
			startLogged = true
			p.emitRequestStarted(c, requestID, spec, requestKind, model, startedAt)
		}
		defer func() {
			if startLogged {
				p.emitRequestCompleted(c, requestID, spec, requestKind, model, startedAt)
			}
		}()

		if spec != nil {
			c.Set(ginUserAPIKeyKey, spec.Key)
			ctx := context.WithValue(c.Request.Context(), clientAPIKeyContextKey, spec)
			ctx = context.WithValue(ctx, requestKindContextKey, requestKind)
			c.Request = c.Request.WithContext(ctx)
		}

		if spec != nil && isModelsRequest(c.Request) {
			models := clientCatalogModelsForAPIKey(p.manifest, spec)
			if isCodexClientModelsRequest(c.Request) {
				c.JSON(http.StatusOK, buildCodexClientModelsResponse(models))
			} else {
				c.JSON(http.StatusOK, buildModelsResponse(models))
			}
			c.Abort()
			return
		}

		if spec == nil || !shouldInspectJSONBody(c.Request) {
			emitStart()
			c.Next()
			return
		}

		body, err := readAndRestoreBody(c.Request)
		if err != nil || len(body) == 0 {
			emitStart()
			c.Next()
			return
		}

		nextBody, model, err := rewriteBodyModel(p.manifest, spec, body)
		if model != "" {
			ctx := context.WithValue(c.Request.Context(), requestModelContextKey, model)
			c.Request = c.Request.WithContext(ctx)
		}
		emitStart()
		if err != nil {
			p.emitBlockedRequest(requestID, spec, model, requestKind, startedAt, err.Error())
			c.AbortWithStatusJSON(http.StatusNotFound, gin.H{
				"error": gin.H{
					"message": err.Error(),
					"type":    "invalid_request_error",
					"code":    "model_not_available",
				},
			})
			return
		}
		if nextBody != nil {
			c.Request.Body = io.NopCloser(bytes.NewReader(nextBody))
			c.Request.ContentLength = int64(len(nextBody))
			c.Request.Header.Set("Content-Length", strconv.Itoa(len(nextBody)))
		}

		c.Next()
	}
}

func (p *requestPolicy) lookupAPIKey(r *http.Request) *apiKeySpec {
	if p == nil || p.manifest == nil {
		return nil
	}
	key := extractClientAPIKey(r)
	if key == "" {
		return nil
	}
	return p.manifest.apiKeyByValue[key]
}

func ensureRequestID(c *gin.Context) string {
	if c == nil || c.Request == nil {
		return internallogging.GenerateRequestID()
	}
	requestID := strings.TrimSpace(internallogging.GetRequestID(c.Request.Context()))
	if requestID == "" {
		requestID = strings.TrimSpace(internallogging.GetGinRequestID(c))
	}
	if requestID == "" {
		requestID = internallogging.GenerateRequestID()
	}
	internallogging.SetGinRequestID(c, requestID)
	c.Request = c.Request.WithContext(internallogging.WithRequestID(c.Request.Context(), requestID))
	return requestID
}

func shouldEmitRequestDiagnostic(r *http.Request) bool {
	if r == nil || r.URL == nil {
		return false
	}
	if isModelsRequest(r) {
		return false
	}
	return requestKindFromPath(r.URL.Path) != "other"
}

func diagnosticTransport(r *http.Request) string {
	if r == nil {
		return ""
	}
	if strings.EqualFold(strings.TrimSpace(r.Header.Get("Upgrade")), "websocket") {
		return "websocket"
	}
	if strings.Contains(strings.ToLower(r.Header.Get("Accept")), "text/event-stream") {
		return "sse"
	}
	return "http"
}

func requestPath(r *http.Request) string {
	if r == nil || r.URL == nil {
		return ""
	}
	return r.URL.Path
}

func (p *requestPolicy) emitRequestStarted(c *gin.Context, requestID string, spec *apiKeySpec, requestKind, model string, startedAt time.Time) {
	if p == nil || p.emitter == nil || c == nil || c.Request == nil {
		return
	}
	p.emitter.emit(requestDiagnosticPayload{
		Type:        "request_started",
		RequestID:   requestID,
		Method:      c.Request.Method,
		Path:        requestPath(c.Request),
		RequestKind: requestKind,
		Model:       model,
		APIKeyID:    stringFromAPIKey(spec, "id"),
		APIKeyLabel: stringFromAPIKey(spec, "label"),
		Transport:   diagnosticTransport(c.Request),
		StartedAtMS: startedAt.UnixMilli(),
	})
}

func (p *requestPolicy) emitRequestCompleted(c *gin.Context, requestID string, spec *apiKeySpec, requestKind, model string, startedAt time.Time) {
	if p == nil || p.emitter == nil || c == nil || c.Request == nil {
		return
	}
	status := c.Writer.Status()
	latencyMS := time.Since(startedAt).Milliseconds()
	completedAtMS := time.Now().UnixMilli()
	p.emitter.emit(requestDiagnosticPayload{
		Type:          "request_completed",
		RequestID:     requestID,
		Method:        c.Request.Method,
		Path:          requestPath(c.Request),
		RequestKind:   requestKind,
		Model:         model,
		APIKeyID:      stringFromAPIKey(spec, "id"),
		APIKeyLabel:   stringFromAPIKey(spec, "label"),
		Transport:     diagnosticTransport(c.Request),
		Status:        status,
		LatencyMS:     latencyMS,
		CompletedAtMS: completedAtMS,
		Aborted:       c.IsAborted(),
		ErrorMessage:  strings.TrimSpace(c.Errors.String()),
	})
	if p.tracker == nil || !shouldEmitRequestDiagnostic(c.Request) {
		return
	}
	if payload, ok := p.tracker.finalize(requestID, usageFinalizeInput{
		spec:          spec,
		requestKind:   requestKind,
		model:         model,
		status:        status,
		latencyMS:     latencyMS,
		completedAtMS: completedAtMS,
		errorMessage:  strings.TrimSpace(c.Errors.String()),
	}); ok {
		p.emitter.emit(payload)
	}
}

func (p *requestPolicy) emitBlockedRequest(requestID string, spec *apiKeySpec, model, requestKind string, startedAt time.Time, message string) {
	if p == nil || spec == nil {
		return
	}
	payload := usagePayload{
		Type:          "usage",
		RequestID:     requestID,
		Model:         model,
		APIKeyID:      spec.ID,
		APIKeyLabel:   spec.Label,
		RequestKind:   requestKind,
		Success:       false,
		Status:        http.StatusNotFound,
		ErrorCategory: "model_not_available",
		ErrorMessage:  message,
		LatencyMS:     time.Since(startedAt).Milliseconds(),
		RequestedAtMS: time.Now().UnixMilli(),
	}
	if p.tracker != nil {
		p.tracker.record(payload)
		return
	}
	if p.emitter != nil {
		p.emitter.emit(payload)
	}
}

func isModelsRequest(r *http.Request) bool {
	return r != nil && r.Method == http.MethodGet && r.URL != nil && r.URL.Path == "/v1/models"
}

func isCodexClientModelsRequest(r *http.Request) bool {
	if r == nil || r.URL == nil {
		return false
	}
	_, ok := r.URL.Query()["client_version"]
	return ok
}

func buildModelsResponse(models []string) gin.H {
	data := make([]gin.H, 0, len(models))
	for _, model := range models {
		data = append(data, gin.H{
			"id":       model,
			"object":   "model",
			"created":  0,
			"owned_by": "openai",
		})
	}
	return gin.H{"object": "list", "data": data}
}

func buildCodexClientModelsResponse(models []string) gin.H {
	data := make([]gin.H, 0, len(models))
	for _, model := range models {
		displayName := displayNameForModel(model)
		visibility := "show"
		switch model {
		case codexAutoReviewModel, "gpt-image-2", "grok-imagine-image", "grok-imagine-video", "grok-imagine-image-quality":
			visibility = "hide"
		}
		data = append(data, gin.H{
			"slug":                       model,
			"display_name":               displayName,
			"description":                displayName,
			"context_window":             272000,
			"max_context_window":         1000000,
			"default_reasoning_level":    "medium",
			"supported_reasoning_levels": reasoningLevels(),
			"prefer_websockets":          true,
			"visibility":                 visibility,
		})
	}
	return gin.H{"models": data}
}

func displayNameForModel(model string) string {
	switch model {
	case "gpt-5-codex":
		return "GPT-5 Codex"
	case "gpt-5-codex-mini":
		return "GPT-5 Codex Mini"
	case "gpt-5.4":
		return "GPT-5.4"
	case "gpt-5.4-mini":
		return "GPT-5.4 Mini"
	case "gpt-5.3-codex":
		return "GPT-5.3 Codex"
	case "gpt-5.3-codex-spark":
		return "GPT-5.3 Codex Spark"
	case "gpt-5.2":
		return "GPT-5.2"
	case "gpt-5.2-codex":
		return "GPT-5.2 Codex"
	case "gpt-5.1-codex-max":
		return "GPT-5.1 Codex Max"
	case "gpt-5.1-codex-mini":
		return "GPT-5.1 Codex Mini"
	case "gpt-image-2":
		return "GPT Image 2"
	case codexAutoReviewModel:
		return "Codex Auto Review"
	default:
		return model
	}
}

func reasoningLevels() []gin.H {
	return []gin.H{
		{"effort": "minimal", "description": "Fastest responses with minimal reasoning"},
		{"effort": "low", "description": "Fast responses with lighter reasoning"},
		{"effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks"},
		{"effort": "high", "description": "Greater reasoning depth for complex problems"},
		{"effort": "xhigh", "description": "Extra high reasoning depth for complex problems"},
	}
}

func shouldInspectJSONBody(r *http.Request) bool {
	if r == nil {
		return false
	}
	if r.Method != http.MethodPost && r.Method != http.MethodPut && r.Method != http.MethodPatch {
		return false
	}
	contentType := strings.ToLower(r.Header.Get("Content-Type"))
	return strings.Contains(contentType, "application/json") || contentType == ""
}

func readAndRestoreBody(r *http.Request) ([]byte, error) {
	if r == nil || r.Body == nil {
		return nil, nil
	}
	body, err := io.ReadAll(r.Body)
	_ = r.Body.Close()
	r.Body = io.NopCloser(bytes.NewReader(body))
	return body, err
}

func rewriteBodyModel(m *manifest, spec *apiKeySpec, body []byte) ([]byte, string, error) {
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, "", nil
	}
	rawModel, _ := payload["model"].(string)
	model := strings.TrimSpace(rawModel)
	if model == "" {
		return nil, "", nil
	}
	canonical := canonicalModelForClientModel(m, spec, model)
	if !validateClientModelVisible(m, spec, model, canonical) {
		return nil, model, fmt.Errorf("模型 %s 不在当前 API Key 的可用模型范围内", model)
	}
	if canonical == model {
		return nil, model, nil
	}
	payload["model"] = canonical
	next, err := json.Marshal(payload)
	if err != nil {
		return nil, model, err
	}
	return next, model, nil
}

func visibleModelsForAPIKey(m *manifest, spec *apiKeySpec) []string {
	if m == nil {
		return nil
	}
	models := applyModelFilters(m.ModelIDs, nil, m.ExcludedModels)
	if spec != nil {
		models = applyModelFilters(models, spec.AllowedModels, spec.ExcludedModels)
		if strings.TrimSpace(spec.ModelPrefix) != "" {
			prefix := strings.Trim(strings.TrimSpace(spec.ModelPrefix), "/")
			for i := range models {
				models[i] = prefix + "/" + models[i]
			}
		}
	}
	return models
}

func clientCatalogModelsForAPIKey(m *manifest, spec *apiKeySpec) []string {
	return appendCodexInternalModels(visibleModelsForAPIKey(m, spec))
}

func appendCodexInternalModels(models []string) []string {
	for _, model := range models {
		if isCodexInternalModel(model) {
			return models
		}
	}
	return append(models, codexAutoReviewModel)
}

func isCodexInternalModel(model string) bool {
	return strings.EqualFold(strings.TrimSpace(model), codexAutoReviewModel)
}

func canonicalModelForClientModel(m *manifest, spec *apiKeySpec, model string) string {
	withoutPrefix := stripModelPrefix(model, spec)
	if isCodexInternalModel(withoutPrefix) {
		return codexAutoReviewModel
	}
	if m != nil {
		if source := m.aliasToSource[strings.ToLower(withoutPrefix)]; source != "" {
			return source
		}
	}
	return resolveSupportedModelAlias(m, withoutPrefix)
}

func stripModelPrefix(model string, spec *apiKeySpec) string {
	trimmed := strings.TrimSpace(model)
	if spec == nil || strings.TrimSpace(spec.ModelPrefix) == "" {
		return trimmed
	}
	prefix := strings.Trim(strings.TrimSpace(spec.ModelPrefix), "/") + "/"
	if strings.HasPrefix(trimmed, prefix) {
		return strings.TrimSpace(strings.TrimPrefix(trimmed, prefix))
	}
	return trimmed
}

func resolveSupportedModelAlias(m *manifest, model string) string {
	trimmed := strings.TrimSpace(model)
	normalized := strings.ToLower(trimmed)
	if m == nil {
		return trimmed
	}
	for _, supported := range m.ModelIDs {
		base := strings.ToLower(strings.TrimSpace(supported))
		if base == "" {
			continue
		}
		if normalized == base {
			return supported
		}
		if strings.HasPrefix(normalized, base+"-") && hasDateSnapshotSuffix(normalized[len(base):]) {
			return supported
		}
	}
	return trimmed
}

func hasDateSnapshotSuffix(suffix string) bool {
	if len(suffix) != len("-2006-01-02") || !strings.HasPrefix(suffix, "-") {
		return false
	}
	for i, ch := range suffix {
		switch i {
		case 0, 5, 8:
			if ch != '-' {
				return false
			}
		default:
			if ch < '0' || ch > '9' {
				return false
			}
		}
	}
	return true
}

func validateClientModelVisible(m *manifest, spec *apiKeySpec, model, canonical string) bool {
	withoutPrefix := stripModelPrefix(model, spec)
	if isCodexInternalModel(withoutPrefix) || isCodexInternalModel(canonical) {
		return true
	}
	visible := visibleModelsForAPIKey(m, nil)
	visibleMatch := false
	for _, item := range visible {
		if strings.EqualFold(item, withoutPrefix) || strings.EqualFold(item, canonical) || strings.EqualFold(resolveSupportedModelAlias(m, item), canonical) {
			visibleMatch = true
			break
		}
	}
	if !visibleMatch {
		return false
	}
	if spec != nil {
		if len(spec.AllowedModels) > 0 && !modelMatchesAnyRule(withoutPrefix, spec.AllowedModels) && !modelMatchesAnyRule(canonical, spec.AllowedModels) {
			return false
		}
		if modelMatchesAnyRule(withoutPrefix, spec.ExcludedModels) || modelMatchesAnyRule(canonical, spec.ExcludedModels) {
			return false
		}
	}
	return true
}

func applyModelFilters(models, allowed, excluded []string) []string {
	out := make([]string, 0, len(models))
	for _, model := range models {
		if len(allowed) > 0 && !modelMatchesAnyRule(model, allowed) {
			continue
		}
		if modelMatchesAnyRule(model, excluded) {
			continue
		}
		out = append(out, model)
	}
	return out
}

func modelMatchesAnyRule(model string, rules []string) bool {
	for _, rule := range rules {
		if wildcardModelMatches(rule, model) {
			return true
		}
	}
	return false
}

func wildcardModelMatches(pattern, model string) bool {
	pattern = strings.ToLower(strings.TrimSpace(pattern))
	model = strings.ToLower(strings.TrimSpace(model))
	if pattern == "" || model == "" {
		return false
	}
	if pattern == "*" {
		return true
	}
	if !strings.Contains(pattern, "*") {
		return pattern == model
	}
	anchoredStart := !strings.HasPrefix(pattern, "*")
	anchoredEnd := !strings.HasSuffix(pattern, "*")
	parts := strings.Split(pattern, "*")
	remaining := model
	for idx, part := range parts {
		if part == "" {
			continue
		}
		found := strings.Index(remaining, part)
		if found < 0 {
			return false
		}
		if idx == 0 && anchoredStart && found != 0 {
			return false
		}
		remaining = remaining[found+len(part):]
	}
	if anchoredEnd {
		for i := len(parts) - 1; i >= 0; i-- {
			if parts[i] != "" {
				return strings.HasSuffix(model, parts[i])
			}
		}
	}
	return true
}

func requestKindFromPath(path string) string {
	path = strings.ToLower(strings.TrimSpace(path))
	switch {
	case strings.Contains(path, "/images/generations"):
		return "image_generation"
	case strings.Contains(path, "/images/edits"):
		return "image_edit"
	case strings.Contains(path, "/chat/completions"), strings.Contains(path, "/responses"):
		return "text"
	default:
		return "other"
	}
}

type cockpitSelector struct {
	manifest *manifest
	emitter  *eventEmitter
	mu       sync.Mutex
	cursor   int
}

func (s *cockpitSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	_ = ctx
	_ = provider
	_ = opts
	now := time.Now()
	available := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		if authAvailable(auth, model, now) {
			available = append(available, auth)
		}
	}
	if len(available) == 0 {
		return nil, fmt.Errorf("no auth available")
	}

	s.mu.Lock()
	start := s.cursor
	s.cursor++
	s.mu.Unlock()

	ordered := s.orderAuths(available, start)
	if len(ordered) == 0 {
		return nil, fmt.Errorf("no auth available")
	}
	selected := ordered[0]
	s.emitAuthSelected(ctx, selected, provider, model, len(auths), len(available))
	return selected, nil
}

func authAvailable(auth *coreauth.Auth, model string, now time.Time) bool {
	if auth == nil || auth.Disabled || auth.Status == coreauth.StatusDisabled {
		return false
	}
	if model != "" && len(auth.ModelStates) > 0 {
		state := auth.ModelStates[model]
		if state == nil {
			state = auth.ModelStates[resolveBaseModelKey(model)]
		}
		if state != nil {
			if state.Status == coreauth.StatusDisabled {
				return false
			}
			if state.Unavailable && !state.NextRetryAfter.IsZero() && state.NextRetryAfter.After(now) {
				return false
			}
		}
	}
	if auth.Unavailable && !auth.NextRetryAfter.IsZero() && auth.NextRetryAfter.After(now) {
		return false
	}
	return true
}

func resolveBaseModelKey(model string) string {
	model = strings.TrimSpace(model)
	for i := len(model) - 1; i >= 0; i-- {
		if model[i] == '-' && i+len("-2006-01-02") == len(model) && hasDateSnapshotSuffix(model[i:]) {
			return model[:i]
		}
	}
	return model
}

func (s *cockpitSelector) orderAuths(auths []*coreauth.Auth, start int) []*coreauth.Auth {
	if len(auths) <= 1 || s == nil || s.manifest == nil {
		return auths
	}
	strategy := strings.TrimSpace(strings.ToLower(s.manifest.RoutingStrategy))
	if strategy == "custom" {
		return s.orderCustom(auths, start)
	}
	out := append([]*coreauth.Auth(nil), auths...)
	sort.SliceStable(out, func(i, j int) bool {
		left := s.accountForAuth(out[i])
		right := s.accountForAuth(out[j])
		if compareAccountSpecs(left, right, strategy) != 0 {
			return compareAccountSpecs(left, right, strategy) < 0
		}
		return s.rotatedIndex(left, start) < s.rotatedIndex(right, start)
	})
	return out
}

func compareAccountSpecs(left, right *accountSpec, strategy string) int {
	switch strategy {
	case "quota_high_first":
		if cmp := compareIntPtrDesc(valueInt(left, "quota"), valueInt(right, "quota")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "plan"), valueInt(right, "plan"))
	case "quota_low_first":
		if cmp := compareIntPtrAsc(valueInt(left, "quota"), valueInt(right, "quota")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "plan"), valueInt(right, "plan"))
	case "plan_low_first":
		if cmp := compareIntPtrAsc(valueInt(left, "plan"), valueInt(right, "plan")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "quota"), valueInt(right, "quota"))
	case "expiry_soon_first":
		if cmp := compareInt64PtrAsc(valueInt64(left), valueInt64(right)); cmp != 0 {
			return cmp
		}
		if cmp := compareIntPtrDesc(valueInt(left, "plan"), valueInt(right, "plan")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "quota"), valueInt(right, "quota"))
	case "plan_high_first":
		fallthrough
	case "auto":
		fallthrough
	default:
		if cmp := compareIntPtrDesc(valueInt(left, "plan"), valueInt(right, "plan")); cmp != 0 {
			return cmp
		}
		return compareIntPtrDesc(valueInt(left, "quota"), valueInt(right, "quota"))
	}
}

func valueInt(account *accountSpec, kind string) *int {
	if account == nil {
		return nil
	}
	if kind == "quota" {
		return account.RemainingQuota
	}
	return account.PlanRank
}

func valueInt64(account *accountSpec) *int64 {
	if account == nil {
		return nil
	}
	return account.SubscriptionExpiryMS
}

func compareIntPtrDesc(left, right *int) int {
	switch {
	case left != nil && right != nil:
		return *right - *left
	case left != nil:
		return -1
	case right != nil:
		return 1
	default:
		return 0
	}
}

func compareIntPtrAsc(left, right *int) int {
	switch {
	case left != nil && right != nil:
		return *left - *right
	case left != nil:
		return -1
	case right != nil:
		return 1
	default:
		return 0
	}
}

func compareInt64PtrAsc(left, right *int64) int {
	switch {
	case left != nil && right != nil:
		if *left < *right {
			return -1
		}
		if *left > *right {
			return 1
		}
		return 0
	case left != nil:
		return -1
	case right != nil:
		return 1
	default:
		return 0
	}
}

func (s *cockpitSelector) orderCustom(auths []*coreauth.Auth, start int) []*coreauth.Auth {
	rules := make(map[string]customRoutingRule)
	for _, rule := range s.manifest.CustomRoutingRules {
		if strings.TrimSpace(rule.AccountID) == "" {
			continue
		}
		if rule.Weight <= 0 {
			rule.Weight = 1
		}
		rules[rule.AccountID] = rule
	}
	groups := make(map[int][]*coreauth.Auth)
	priorities := make([]int, 0)
	seenPriority := make(map[int]struct{})
	for _, auth := range auths {
		account := s.accountForAuth(auth)
		priority := 0
		if account != nil {
			priority = rules[account.ID].Priority
		}
		groups[priority] = append(groups[priority], auth)
		if _, ok := seenPriority[priority]; !ok {
			seenPriority[priority] = struct{}{}
			priorities = append(priorities, priority)
		}
	}
	sort.Sort(sort.Reverse(sort.IntSlice(priorities)))
	out := make([]*coreauth.Auth, 0, len(auths))
	for _, priority := range priorities {
		group := groups[priority]
		out = append(out, weightedOrder(group, rules, s, start)...)
	}
	return out
}

func weightedOrder(group []*coreauth.Auth, rules map[string]customRoutingRule, selector *cockpitSelector, start int) []*coreauth.Auth {
	if len(group) <= 1 {
		return group
	}
	total := 0
	weights := make([]int, len(group))
	for i, auth := range group {
		weight := 1
		if account := selector.accountForAuth(auth); account != nil {
			if rule, ok := rules[account.ID]; ok && rule.Weight > 0 {
				weight = rule.Weight
			}
		}
		weights[i] = weight
		total += weight
	}
	slot := start % total
	first := 0
	for i, weight := range weights {
		if slot < weight {
			first = i
			break
		}
		slot -= weight
	}
	out := make([]*coreauth.Auth, 0, len(group))
	for offset := 0; offset < len(group); offset++ {
		out = append(out, group[(first+offset)%len(group)])
	}
	return out
}

func (s *cockpitSelector) accountForAuth(auth *coreauth.Auth) *accountSpec {
	if s == nil || s.manifest == nil || auth == nil {
		return nil
	}
	if auth.ID != "" {
		if account := s.manifest.accountByAuthID[strings.ToLower(auth.ID)]; account != nil {
			return account
		}
		base := strings.TrimSuffix(filepath.Base(auth.ID), filepath.Ext(auth.ID))
		if account := s.manifest.accountByID[base]; account != nil {
			return account
		}
	}
	if auth.Attributes != nil {
		if key := strings.TrimSpace(auth.Attributes["api_key"]); key != "" {
			return s.manifest.accountByAPIKey[key]
		}
	}
	return nil
}

func (s *cockpitSelector) emitAuthSelected(ctx context.Context, auth *coreauth.Auth, provider, model string, candidateAuths, availableAuths int) {
	if s == nil || s.emitter == nil || auth == nil {
		return
	}
	if ctx == nil {
		ctx = context.Background()
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if requestKind == "" {
		requestKind = requestKindFromPath(internallogging.GetEndpoint(ctx))
	}
	requestModel, _ := ctx.Value(requestModelContextKey).(string)
	if strings.TrimSpace(requestModel) != "" {
		model = requestModel
	}
	account := s.accountForAuth(auth)
	routingStrategy := ""
	if s.manifest != nil {
		routingStrategy = strings.TrimSpace(s.manifest.RoutingStrategy)
	}
	s.emitter.emit(requestDiagnosticPayload{
		Type:            "auth_selected",
		RequestID:       internallogging.GetRequestID(ctx),
		RequestKind:     requestKind,
		Model:           model,
		APIKeyID:        stringFromAPIKey(spec, "id"),
		APIKeyLabel:     stringFromAPIKey(spec, "label"),
		CandidateAuths:  candidateAuths,
		AvailableAuths:  availableAuths,
		RoutingStrategy: routingStrategy,
		Provider:        provider,
		AuthID:          auth.ID,
		AccountID:       stringFromAccount(account, "id"),
		AccountEmail:    stringFromAccount(account, "email"),
	})
}

func (s *cockpitSelector) rotatedIndex(account *accountSpec, start int) int {
	if s == nil || s.manifest == nil || account == nil {
		return 1 << 30
	}
	index, ok := s.manifest.originalIndexByID[account.ID]
	if !ok || len(s.manifest.Accounts) == 0 {
		return 1 << 30
	}
	total := len(s.manifest.Accounts)
	return (index - (start % total) + total) % total
}

type usagePlugin struct {
	manifest *manifest
	tracker  *requestUsageTracker
}

func (p *usagePlugin) HandleUsage(ctx context.Context, record coreusage.Record) {
	if p == nil || p.tracker == nil {
		return
	}
	if ctx == nil {
		ctx = context.Background()
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec == nil && p.manifest != nil && strings.TrimSpace(record.APIKey) != "" {
		spec = p.manifest.apiKeyByValue[strings.TrimSpace(record.APIKey)]
	}
	account := p.accountForRecord(record)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if strings.TrimSpace(requestKind) == "" {
		requestKind = requestKindFromPath(internallogging.GetEndpoint(ctx))
	}
	if strings.TrimSpace(requestKind) == "" {
		requestKind = "other"
	}
	requestModel, _ := ctx.Value(requestModelContextKey).(string)
	model := strings.TrimSpace(record.Model)
	if requestModel != "" {
		model = requestModel
	}
	status := record.Fail.StatusCode
	success := !record.Failed
	p.tracker.record(usagePayload{
		Type:          "usage",
		RequestID:     internallogging.GetRequestID(ctx),
		Provider:      record.Provider,
		Model:         model,
		Alias:         record.Alias,
		AccountID:     stringFromAccount(account, "id"),
		AccountEmail:  stringFromAccount(account, "email"),
		AuthID:        record.AuthID,
		APIKeyID:      stringFromAPIKey(spec, "id"),
		APIKeyLabel:   stringFromAPIKey(spec, "label"),
		RequestKind:   requestKind,
		Success:       success,
		Status:        status,
		ErrorCategory: errorCategory(status, record.Fail.Body, success),
		ErrorMessage:  strings.TrimSpace(record.Fail.Body),
		LatencyMS:     record.Latency.Milliseconds(),
		Usage: usageDetails{
			InputTokens:     record.Detail.InputTokens,
			OutputTokens:    record.Detail.OutputTokens,
			ReasoningTokens: record.Detail.ReasoningTokens,
			CachedTokens:    record.Detail.CachedTokens,
			TotalTokens:     record.Detail.TotalTokens,
		},
		RequestedAtMS: record.RequestedAt.UnixMilli(),
	})
}

func (p *usagePlugin) accountForRecord(record coreusage.Record) *accountSpec {
	if p == nil || p.manifest == nil {
		return nil
	}
	if record.AuthID != "" {
		if account := p.manifest.accountByAuthID[strings.ToLower(record.AuthID)]; account != nil {
			return account
		}
		base := strings.TrimSuffix(filepath.Base(record.AuthID), filepath.Ext(record.AuthID))
		if account := p.manifest.accountByID[base]; account != nil {
			return account
		}
	}
	if record.APIKey != "" {
		return p.manifest.accountByAPIKey[record.APIKey]
	}
	return nil
}

func stringFromAccount(account *accountSpec, field string) string {
	if account == nil {
		return ""
	}
	if field == "email" {
		return account.Email
	}
	return account.ID
}

func stringFromAPIKey(spec *apiKeySpec, field string) string {
	if spec == nil {
		return ""
	}
	if field == "label" {
		return spec.Label
	}
	return spec.ID
}

func errorCategory(status int, body string, success bool) string {
	if success {
		return ""
	}
	lower := strings.ToLower(body)
	switch {
	case strings.Contains(lower, "upstream timed out in stream_open") ||
		strings.Contains(lower, "phase=execute_stream upstream timed out in stream_open") ||
		strings.Contains(lower, "stream_open"):
		return "upstream_first_byte_timeout"
	case strings.Contains(lower, "upstream timed out in stream_idle") ||
		strings.Contains(lower, "stream_idle"):
		return "upstream_stream_timeout"
	case strings.Contains(lower, "upstream timed out") ||
		strings.Contains(lower, "request_timeout") ||
		strings.Contains(lower, "deadline exceeded"):
		return "upstream_stream_timeout"
	case strings.Contains(lower, "downstream_client_closed") ||
		strings.Contains(lower, "stream_client_gone") ||
		strings.Contains(lower, "client_gone") ||
		strings.Contains(lower, "client canceled") ||
		strings.Contains(lower, "client disconnected") ||
		strings.Contains(lower, "client closed") ||
		strings.Contains(lower, "broken pipe") ||
		strings.Contains(lower, "connection reset") ||
		strings.Contains(lower, "connection aborted") ||
		strings.Contains(lower, "unexpected eof"):
		return "client_canceled"
	case strings.Contains(lower, "context canceled"):
		if status >= http.StatusInternalServerError || status == http.StatusRequestTimeout {
			return "gateway_context_canceled"
		}
		return "client_canceled"
	case strings.Contains(lower, "upstream_response_failed") ||
		strings.Contains(lower, "codex upstream response.failed") ||
		strings.Contains(lower, "last_event=response.failed"):
		return "upstream_response_failed"
	case status == http.StatusUnauthorized || status == http.StatusForbidden:
		return "auth_failed"
	case status == http.StatusNotFound:
		return "model_not_available"
	case status == http.StatusTooManyRequests || strings.Contains(lower, "quota") || strings.Contains(lower, "rate limit"):
		return "quota_or_rate_limit"
	case status >= 500:
		return "upstream_error"
	default:
		return "request_failed"
	}
}

type authHook struct {
	manifest *manifest
	emitter  *eventEmitter
}

func (h *authHook) OnAuthRegistered(_ context.Context, auth *coreauth.Auth) {
	h.emit("auth_registered", auth)
}

func (h *authHook) OnAuthUpdated(_ context.Context, auth *coreauth.Auth) {
	h.emit("auth_updated", auth)
}

func (h *authHook) OnResult(ctx context.Context, result coreauth.Result) {
	if h == nil || h.emitter == nil {
		return
	}
	if ctx == nil {
		ctx = context.Background()
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if requestKind == "" {
		requestKind = requestKindFromPath(internallogging.GetEndpoint(ctx))
	}
	model := result.Model
	if requestModel, _ := ctx.Value(requestModelContextKey).(string); strings.TrimSpace(requestModel) != "" {
		model = requestModel
	}
	account := h.accountForAuthID(result.AuthID)
	status := 0
	errorCode := ""
	errorMessage := ""
	retryable := false
	var retryablePtr *bool
	if result.Error != nil {
		status = result.Error.HTTPStatus
		errorCode = result.Error.Code
		errorMessage = result.Error.Message
		retryable = result.Error.Retryable
		retryablePtr = &retryable
	}
	retryAfterMS := int64(0)
	if result.RetryAfter != nil {
		retryAfterMS = result.RetryAfter.Milliseconds()
	}
	success := result.Success
	h.emitter.emit(requestDiagnosticPayload{
		Type:         "auth_result",
		RequestID:    internallogging.GetRequestID(ctx),
		Provider:     result.Provider,
		Model:        model,
		AuthID:       result.AuthID,
		AccountID:    stringFromAccount(account, "id"),
		AccountEmail: stringFromAccount(account, "email"),
		APIKeyID:     stringFromAPIKey(spec, "id"),
		APIKeyLabel:  stringFromAPIKey(spec, "label"),
		RequestKind:  requestKind,
		Success:      &success,
		HTTPStatus:   status,
		ErrorCode:    errorCode,
		ErrorMessage: errorMessage,
		Retryable:    retryablePtr,
		RetryAfterMS: retryAfterMS,
	})
}

func (h *authHook) accountForAuthID(authID string) *accountSpec {
	if h == nil || h.manifest == nil {
		return nil
	}
	authID = strings.TrimSpace(authID)
	if authID == "" {
		return nil
	}
	if account := h.manifest.accountByAuthID[strings.ToLower(authID)]; account != nil {
		return account
	}
	base := strings.TrimSuffix(filepath.Base(authID), filepath.Ext(authID))
	return h.manifest.accountByID[base]
}

func (h *authHook) emit(eventType string, auth *coreauth.Auth) {
	if h == nil || h.emitter == nil || auth == nil {
		return
	}
	h.emitter.emit(map[string]any{
		"type":     eventType,
		"authId":   auth.ID,
		"provider": auth.Provider,
		"label":    auth.Label,
		"status":   string(auth.Status),
		"disabled": auth.Disabled,
	})
}

func buildCoreAuthManager(cfg *config.Config, selector coreauth.Selector, hook coreauth.Hook) *coreauth.Manager {
	tokenStore := sdkauth.GetTokenStore()
	if dirSetter, ok := tokenStore.(interface{ SetBaseDir(string) }); ok && cfg != nil {
		dirSetter.SetBaseDir(cfg.AuthDir)
	}
	if cfg != nil && cfg.Routing.SessionAffinity {
		ttl := time.Hour
		if parsed, err := time.ParseDuration(strings.TrimSpace(cfg.Routing.SessionAffinityTTL)); err == nil && parsed > 0 {
			ttl = parsed
		}
		selector = coreauth.NewSessionAffinitySelectorWithConfig(coreauth.SessionAffinityConfig{
			Fallback: selector,
			TTL:      ttl,
		})
	}
	return coreauth.NewManager(tokenStore, selector, hook)
}

type sidecarRuntime struct {
	manager *coreauth.Manager
	service *cliproxy.Service
	cancel  context.CancelFunc
	done    chan error
}

func newSidecarRuntime(ctx context.Context, configPath string, cfg *config.Config, m *manifest, manager *coreauth.Manager) (*sidecarRuntime, error) {
	if cfg == nil {
		return nil, fmt.Errorf("config is nil")
	}
	if manager == nil {
		return nil, fmt.Errorf("auth manager is nil")
	}
	if err := ensureSidecarAuthDir(cfg); err != nil {
		return nil, err
	}

	authManager := sdkauth.NewManager(
		sdkauth.GetTokenStore(),
		sdkauth.NewGeminiAuthenticator(),
		sdkauth.NewCodexAuthenticator(),
		sdkauth.NewClaudeAuthenticator(),
		sdkauth.NewAntigravityAuthenticator(),
		sdkauth.NewKimiAuthenticator(),
	)
	readyCh := make(chan struct{})
	var readyOnce sync.Once
	service, err := cliproxy.NewBuilder().
		WithConfig(cfg).
		WithConfigPath(configPath).
		WithAuthManager(authManager).
		WithCoreAuthManager(manager).
		WithHooks(cliproxy.Hooks{
			OnAfterStart: func(*cliproxy.Service) {
				readyOnce.Do(func() { close(readyCh) })
			},
		}).
		Build()
	if err != nil {
		return nil, err
	}

	manager.SetRoundTripperProvider(newSidecarRoundTripperProvider())

	runtimeCtx, cancel := context.WithCancel(ctx)
	done := make(chan error, 1)
	go func() {
		runErr := service.StartRuntime(runtimeCtx)
		if runErr != nil && !errors.Is(runErr, context.Canceled) {
			done <- runErr
			return
		}
		done <- nil
	}()

	select {
	case <-readyCh:
	case runErr := <-done:
		cancel()
		if runErr == nil {
			return nil, fmt.Errorf("runtime stopped before becoming ready")
		}
		return nil, runErr
	case <-time.After(10 * time.Second):
		cancel()
		return nil, fmt.Errorf("runtime startup timeout")
	}

	if err := registerConfigCodexAPIKeyAuths(runtimeCtx, service, cfg, m); err != nil {
		cancel()
		return nil, err
	}
	for _, auth := range manager.List() {
		if auth == nil || !strings.EqualFold(strings.TrimSpace(auth.Provider), "codex") {
			continue
		}
		linkManifestAccountForAuth(m, auth)
		registerManifestModelsForAuth(manager, m, auth)
	}
	service.RebindRuntimeExecutors()

	return &sidecarRuntime{manager: manager, service: service, cancel: cancel, done: done}, nil
}

func registerConfigCodexAPIKeyAuths(ctx context.Context, service *cliproxy.Service, cfg *config.Config, m *manifest) error {
	if service == nil || cfg == nil {
		return nil
	}
	auths, err := synthesizer.NewConfigSynthesizer().Synthesize(&synthesizer.SynthesisContext{
		Config:      cfg,
		AuthDir:     cfg.AuthDir,
		Now:         time.Now(),
		IDGenerator: synthesizer.NewStableIDGenerator(),
	})
	if err != nil {
		return fmt.Errorf("synthesize config auths: %w", err)
	}
	for _, auth := range auths {
		if auth == nil || !strings.EqualFold(strings.TrimSpace(auth.Provider), "codex") {
			continue
		}
		if auth.Attributes == nil || strings.TrimSpace(auth.Attributes["api_key"]) == "" {
			continue
		}
		registered, err := service.UpsertRuntimeAuth(coreauth.WithSkipPersist(ctx), auth)
		if err != nil {
			return fmt.Errorf("register codex api key auth %s: %w", auth.ID, err)
		}
		linkManifestAccountForAuth(m, registered)
	}
	return nil
}

func (r *sidecarRuntime) Execute(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error) {
	if r == nil || r.service == nil {
		return cliproxyexecutor.Response{}, fmt.Errorf("runtime is not initialized")
	}
	return r.service.Execute(ctx, providers, req, opts)
}

func (r *sidecarRuntime) ExecuteStream(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error) {
	if r == nil || r.service == nil {
		return nil, fmt.Errorf("runtime is not initialized")
	}
	return r.service.ExecuteStream(ctx, providers, req, opts)
}

func (r *sidecarRuntime) Stop() {
	if r == nil || r.cancel == nil {
		return
	}
	r.cancel()
	if r.done == nil {
		return
	}
	select {
	case <-r.done:
	case <-time.After(10 * time.Second):
	}
}

func ensureSidecarAuthDir(cfg *config.Config) error {
	if cfg == nil || strings.TrimSpace(cfg.AuthDir) == "" {
		return nil
	}
	info, err := os.Stat(cfg.AuthDir)
	if err == nil {
		if !info.IsDir() {
			return fmt.Errorf("auth path exists but is not a directory: %s", cfg.AuthDir)
		}
		return nil
	}
	if !os.IsNotExist(err) {
		return fmt.Errorf("check auth directory %s: %w", cfg.AuthDir, err)
	}
	if err := os.MkdirAll(cfg.AuthDir, 0o755); err != nil {
		return fmt.Errorf("create auth directory %s: %w", cfg.AuthDir, err)
	}
	return nil
}

func linkManifestAccountForAuth(m *manifest, auth *coreauth.Auth) {
	if m == nil || auth == nil || strings.TrimSpace(auth.ID) == "" {
		return
	}
	if m.accountByAuthID == nil {
		m.accountByAuthID = make(map[string]*accountSpec)
	}
	if _, exists := m.accountByAuthID[strings.ToLower(auth.ID)]; exists {
		return
	}
	if auth.Attributes != nil {
		if key := strings.TrimSpace(auth.Attributes["api_key"]); key != "" {
			if account := m.accountByAPIKey[key]; account != nil {
				m.accountByAuthID[strings.ToLower(auth.ID)] = account
			}
		}
	}
}

func registerManifestModelsForAuth(manager *coreauth.Manager, m *manifest, auth *coreauth.Auth) {
	if manager == nil || m == nil || auth == nil || strings.TrimSpace(auth.ID) == "" {
		return
	}
	models := manifestRegistryModels(m)
	if len(models) == 0 {
		cliproxy.GlobalModelRegistry().UnregisterClient(auth.ID)
		manager.RefreshSchedulerEntry(auth.ID)
		return
	}
	cliproxy.GlobalModelRegistry().RegisterClient(auth.ID, "codex", models)
	manager.ReconcileRegistryModelStates(context.Background(), auth.ID)
	manager.RefreshSchedulerEntry(auth.ID)
}

func manifestRegistryModels(m *manifest) []*cliproxy.ModelInfo {
	if m == nil {
		return nil
	}
	ids := make([]string, 0, len(m.ModelIDs)+len(m.ModelAliases)*2)
	ids = append(ids, m.ModelIDs...)
	for _, alias := range m.ModelAliases {
		ids = append(ids, alias.SourceModel, alias.Alias)
	}
	ids = appendCodexInternalModels(ids)
	ids = normalizeStringList(ids)
	models := make([]*cliproxy.ModelInfo, 0, len(ids))
	now := time.Now().Unix()
	for _, id := range ids {
		models = append(models, &cliproxy.ModelInfo{
			ID:          id,
			Object:      "model",
			Created:     now,
			OwnedBy:     "openai",
			Type:        "openai",
			DisplayName: displayNameForModel(id),
		})
	}
	return models
}

type sidecarRoundTripperProvider struct {
	mu    sync.RWMutex
	cache map[string]http.RoundTripper
}

func newSidecarRoundTripperProvider() *sidecarRoundTripperProvider {
	return &sidecarRoundTripperProvider{cache: make(map[string]http.RoundTripper)}
}

func (p *sidecarRoundTripperProvider) RoundTripperFor(auth *coreauth.Auth) http.RoundTripper {
	if p == nil || auth == nil {
		return nil
	}
	proxyURL := strings.TrimSpace(auth.ProxyURL)
	if proxyURL == "" {
		return nil
	}
	p.mu.RLock()
	rt := p.cache[proxyURL]
	p.mu.RUnlock()
	if rt != nil {
		return rt
	}
	transport, _, err := proxyutil.BuildHTTPTransport(proxyURL)
	if err != nil || transport == nil {
		return nil
	}
	p.mu.Lock()
	p.cache[proxyURL] = transport
	p.mu.Unlock()
	return transport
}

type executorRuntime interface {
	Execute(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (cliproxyexecutor.Response, error)
	ExecuteStream(ctx context.Context, providers []string, req cliproxyexecutor.Request, opts cliproxyexecutor.Options) (*cliproxyexecutor.StreamResult, error)
}

type relayServer struct {
	runtime  executorRuntime
	cfg      *config.Config
	manifest *manifest
	emitter  *eventEmitter
	policy   *requestPolicy
}

func (s *relayServer) router() *gin.Engine {
	router := gin.New()
	router.Use(gin.Recovery())
	router.Use(corsMiddleware())
	router.Use(s.policy.middleware())
	router.GET("/v1/models", s.handleModels)
	router.POST("/v1/responses", s.handleResponses)
	router.POST("/v1/responses/compact", s.handleResponsesCompact)
	router.POST("/v1/chat/completions", s.handleChatCompletions)
	router.POST(imagesGenerationsPath, s.handleImagesGenerations)
	router.POST(imagesEditsPath, s.handleImagesEdits)
	router.NoRoute(func(c *gin.Context) {
		writeAPIError(c, http.StatusNotFound, "endpoint not supported", "not_found")
	})
	return router
}

func corsMiddleware() gin.HandlerFunc {
	return func(c *gin.Context) {
		c.Header("Access-Control-Allow-Origin", "*")
		c.Header("Access-Control-Allow-Methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS")
		c.Header("Access-Control-Allow-Headers", "*")
		if c.Request != nil && c.Request.Method == http.MethodOptions {
			c.AbortWithStatus(http.StatusNoContent)
			return
		}
		c.Next()
	}
}

func (s *relayServer) handleModels(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	models := clientCatalogModelsForAPIKey(s.manifest, spec)
	if isCodexClientModelsRequest(c.Request) {
		c.JSON(http.StatusOK, buildCodexClientModelsResponse(models))
		return
	}
	c.JSON(http.StatusOK, buildModelsResponse(models))
}

func (s *relayServer) handleResponses(c *gin.Context) {
	s.handleExecutorRequest(c, sdktranslator.FormatOpenAIResponse, "")
}

func (s *relayServer) handleResponsesCompact(c *gin.Context) {
	s.handleExecutorRequest(c, sdktranslator.FormatOpenAIResponse, "responses/compact")
}

func (s *relayServer) handleChatCompletions(c *gin.Context) {
	s.handleExecutorRequest(c, sdktranslator.FormatOpenAI, "")
}

func (s *relayServer) handleImagesGenerations(c *gin.Context) {
	if _, ok := s.requireAPIKey(c); !ok {
		return
	}
	rawJSON, err := c.GetRawData()
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, "failed to read request body", "invalid_request")
		return
	}
	imageReq, err := buildImageGenerationRelayRequest(rawJSON)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, err.Error(), "invalid_request")
		return
	}
	s.handleImagesRelayRequest(c, imageReq)
}

func (s *relayServer) handleImagesEdits(c *gin.Context) {
	if _, ok := s.requireAPIKey(c); !ok {
		return
	}
	imageReq, err := buildImageEditRelayRequest(c)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, err.Error(), "invalid_request")
		return
	}
	s.handleImagesRelayRequest(c, imageReq)
}

type imageRelayRequest struct {
	body           []byte
	stream         bool
	responseFormat string
	streamPrefix   string
	requestedModel string
}

type imageRelayResult struct {
	Result        string
	RevisedPrompt string
	OutputFormat  string
	Size          string
	Background    string
	Quality       string
}

type imageSSEAccumulator struct {
	pending []byte
}

func (a *imageSSEAccumulator) AddChunk(chunk []byte) [][]byte {
	if len(chunk) == 0 {
		return nil
	}
	if responsesSSENeedsLineBreak(a.pending, chunk) {
		a.pending = append(a.pending, '\n')
	}
	a.pending = append(a.pending, chunk...)

	var frames [][]byte
	for {
		frameLen := responsesSSEFrameLen(a.pending)
		if frameLen == 0 {
			break
		}
		frames = append(frames, a.pending[:frameLen])
		copy(a.pending, a.pending[frameLen:])
		a.pending = a.pending[:len(a.pending)-frameLen]
	}
	if len(bytes.TrimSpace(a.pending)) == 0 {
		a.pending = a.pending[:0]
		return frames
	}
	if responsesSSECanEmitWithoutDelimiter(a.pending) {
		frames = append(frames, a.pending)
		a.pending = a.pending[:0]
	}
	return frames
}

func (a *imageSSEAccumulator) Flush() [][]byte {
	if len(a.pending) == 0 {
		return nil
	}
	var frames [][]byte
	for {
		frameLen := responsesSSEFrameLen(a.pending)
		if frameLen == 0 {
			break
		}
		frames = append(frames, a.pending[:frameLen])
		copy(a.pending, a.pending[frameLen:])
		a.pending = a.pending[:len(a.pending)-frameLen]
	}
	if len(bytes.TrimSpace(a.pending)) > 0 && responsesSSECanEmitWithoutDelimiter(a.pending) {
		frames = append(frames, a.pending)
	}
	a.pending = nil
	return frames
}

func buildImageGenerationRelayRequest(rawJSON []byte) (imageRelayRequest, error) {
	if !json.Valid(rawJSON) {
		return imageRelayRequest{}, fmt.Errorf("body must be valid JSON")
	}
	var payload map[string]any
	if err := json.Unmarshal(rawJSON, &payload); err != nil {
		return imageRelayRequest{}, err
	}
	prompt := strings.TrimSpace(stringField(payload, "prompt"))
	if prompt == "" {
		return imageRelayRequest{}, fmt.Errorf("prompt is required")
	}
	tool, err := buildImageTool(payload, "generate")
	if err != nil {
		return imageRelayRequest{}, err
	}
	body, err := json.Marshal(buildImagesResponsesPayload(prompt, nil, tool))
	if err != nil {
		return imageRelayRequest{}, err
	}
	return imageRelayRequest{
		body:           body,
		stream:         boolField(payload, "stream"),
		responseFormat: normalizeImageResponseFormat(stringField(payload, "response_format")),
		streamPrefix:   "image_generation",
		requestedModel: imageModelOrDefault(payload),
	}, nil
}

func buildImageEditRelayRequest(c *gin.Context) (imageRelayRequest, error) {
	contentType := strings.ToLower(strings.TrimSpace(c.GetHeader("Content-Type")))
	if strings.HasPrefix(contentType, "multipart/form-data") || contentType == "" {
		return buildImageEditRelayRequestFromMultipart(c)
	}
	if !strings.HasPrefix(contentType, "application/json") {
		return imageRelayRequest{}, fmt.Errorf("unsupported Content-Type %q", contentType)
	}
	rawJSON, err := c.GetRawData()
	if err != nil {
		return imageRelayRequest{}, err
	}
	if !json.Valid(rawJSON) {
		return imageRelayRequest{}, fmt.Errorf("body must be valid JSON")
	}
	var payload map[string]any
	if err := json.Unmarshal(rawJSON, &payload); err != nil {
		return imageRelayRequest{}, err
	}
	prompt := strings.TrimSpace(stringField(payload, "prompt"))
	if prompt == "" {
		return imageRelayRequest{}, fmt.Errorf("prompt is required")
	}
	images := jsonImageURLs(payload)
	if len(images) == 0 {
		return imageRelayRequest{}, fmt.Errorf("images[].image_url is required")
	}
	tool, err := buildImageTool(payload, "edit")
	if err != nil {
		return imageRelayRequest{}, err
	}
	if mask, ok := payload["mask"].(map[string]any); ok {
		if url := strings.TrimSpace(stringField(mask, "image_url")); url != "" {
			tool["input_image_mask"] = map[string]any{"image_url": url}
		}
	}
	body, err := json.Marshal(buildImagesResponsesPayload(prompt, images, tool))
	if err != nil {
		return imageRelayRequest{}, err
	}
	return imageRelayRequest{
		body:           body,
		stream:         boolField(payload, "stream"),
		responseFormat: normalizeImageResponseFormat(stringField(payload, "response_format")),
		streamPrefix:   "image_edit",
		requestedModel: imageModelOrDefault(payload),
	}, nil
}

func buildImageEditRelayRequestFromMultipart(c *gin.Context) (imageRelayRequest, error) {
	form, err := c.MultipartForm()
	if err != nil {
		return imageRelayRequest{}, err
	}
	payload := map[string]any{
		"model":              strings.TrimSpace(c.PostForm("model")),
		"size":               strings.TrimSpace(c.PostForm("size")),
		"quality":            strings.TrimSpace(c.PostForm("quality")),
		"background":         strings.TrimSpace(c.PostForm("background")),
		"output_format":      strings.TrimSpace(c.PostForm("output_format")),
		"input_fidelity":     strings.TrimSpace(c.PostForm("input_fidelity")),
		"moderation":         strings.TrimSpace(c.PostForm("moderation")),
		"response_format":    strings.TrimSpace(c.PostForm("response_format")),
		"stream":             parseBoolString(c.PostForm("stream")),
		"output_compression": parseIntString(c.PostForm("output_compression")),
		"partial_images":     parseIntString(c.PostForm("partial_images")),
	}
	prompt := strings.TrimSpace(c.PostForm("prompt"))
	if prompt == "" {
		return imageRelayRequest{}, fmt.Errorf("prompt is required")
	}
	imageFiles := form.File["image[]"]
	if len(imageFiles) == 0 {
		imageFiles = form.File["image"]
	}
	if len(imageFiles) == 0 {
		return imageRelayRequest{}, fmt.Errorf("image is required")
	}
	images := make([]string, 0, len(imageFiles))
	for _, fh := range imageFiles {
		dataURL, err := multipartFileToDataURL(fh)
		if err != nil {
			return imageRelayRequest{}, err
		}
		images = append(images, dataURL)
	}
	tool, err := buildImageTool(payload, "edit")
	if err != nil {
		return imageRelayRequest{}, err
	}
	if masks := form.File["mask"]; len(masks) > 0 && masks[0] != nil {
		dataURL, err := multipartFileToDataURL(masks[0])
		if err != nil {
			return imageRelayRequest{}, err
		}
		tool["input_image_mask"] = map[string]any{"image_url": dataURL}
	}
	body, err := json.Marshal(buildImagesResponsesPayload(prompt, images, tool))
	if err != nil {
		return imageRelayRequest{}, err
	}
	return imageRelayRequest{
		body:           body,
		stream:         boolField(payload, "stream"),
		responseFormat: normalizeImageResponseFormat(stringField(payload, "response_format")),
		streamPrefix:   "image_edit",
		requestedModel: imageModelOrDefault(payload),
	}, nil
}

func (s *relayServer) handleImagesRelayRequest(c *gin.Context, imageReq imageRelayRequest) {
	spec, _ := c.Request.Context().Value(clientAPIKeyContextKey).(*apiKeySpec)
	requestedModel := strings.TrimSpace(imageReq.requestedModel)
	if requestedModel == "" {
		requestedModel = defaultImagesToolModel
	}
	if !validateClientModelVisible(s.manifest, spec, requestedModel, defaultImagesToolModel) {
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("模型 %s 不在当前 API Key 的可用模型范围内", requestedModel), "model_not_available")
		return
	}
	model := defaultImagesMainModel
	req, opts := buildExecutorRequest(c, imageReq.body, model, sdktranslator.FormatOpenAIResponse, "", true)
	startedAt := time.Now()
	timeouts := s.streamTimeoutsForRequest(c.Request, imageReq.body, defaultImagesToolModel)
	streamCtx, cancelStream := context.WithCancel(relayContext(c))
	defer cancelStream()
	result, err := s.executeStreamWithOpenTimeout(c, streamCtx, []string{"codex"}, req, opts, model, startedAt, timeouts.open)
	if err != nil {
		writeExecutorError(c, err)
		return
	}
	if result == nil || result.Chunks == nil {
		writeAPIError(c, http.StatusBadGateway, "upstream stream is unavailable", "bad_gateway")
		return
	}
	if imageReq.stream {
		s.forwardImagesStream(c, streamCtx, result, imageReq, timeouts.idle)
		return
	}
	out, err := collectImagesResponse(streamCtx, result.Chunks, imageReq.responseFormat, timeouts.idle)
	if err != nil {
		writeExecutorError(c, err)
		return
	}
	writeUpstreamHeaders(c.Writer.Header(), result.Headers)
	c.Data(http.StatusOK, "application/json", out)
}

func (s *relayServer) forwardImagesStream(c *gin.Context, ctx context.Context, result *cliproxyexecutor.StreamResult, imageReq imageRelayRequest, idleTimeout time.Duration) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	setEventStreamHeaders(c.Writer.Header())
	writeUpstreamHeaders(c.Writer.Header(), result.Headers)
	c.Status(http.StatusOK)

	writeEvent := func(eventName string, payload []byte) {
		if strings.TrimSpace(eventName) != "" {
			_, _ = fmt.Fprintf(c.Writer, "event: %s\n", eventName)
		}
		_, _ = fmt.Fprintf(c.Writer, "data: %s\n\n", string(payload))
		flusher.Flush()
	}
	writeErr := func(err error) {
		status := statusCodeFromError(err)
		payload, _ := json.Marshal(map[string]any{
			"error": map[string]any{
				"message": errorMessage(err),
				"type":    "upstream_error",
				"code":    status,
			},
		})
		writeEvent("error", payload)
	}

	acc := &imageSSEAccumulator{}
	if idleTimeout <= 0 {
		idleTimeout = imageStreamIdleTimeout
	}
	idleTimer := time.NewTimer(idleTimeout)
	defer idleTimer.Stop()
	for {
		select {
		case <-idleTimer.C:
			writeErr(relayTimeoutError{phase: "stream_idle", timeout: idleTimeout})
			return
		case <-ctx.Done():
			writeErr(ctx.Err())
			return
		case <-c.Request.Context().Done():
			return
		case chunk, ok := <-result.Chunks:
			if !idleTimer.Stop() {
				select {
				case <-idleTimer.C:
				default:
				}
			}
			idleTimer.Reset(idleTimeout)
			if !ok {
				for _, frame := range acc.Flush() {
					if done := forwardImageResponseFrame(frame, imageReq, writeEvent, writeErr); done {
						return
					}
				}
				return
			}
			if chunk.Err != nil {
				writeErr(chunk.Err)
				return
			}
			for _, frame := range acc.AddChunk(chunk.Payload) {
				if done := forwardImageResponseFrame(frame, imageReq, writeEvent, writeErr); done {
					return
				}
			}
		}
	}
}

func forwardImageResponseFrame(frame []byte, imageReq imageRelayRequest, writeEvent func(string, []byte), writeErr func(error)) bool {
	for _, payload := range imageFramePayloads(frame) {
		var event map[string]any
		if err := json.Unmarshal(payload, &event); err != nil {
			continue
		}
		switch stringField(event, "type") {
		case "response.image_generation_call.partial_image":
			b64 := stringField(event, "partial_image_b64")
			if b64 == "" {
				continue
			}
			index, _ := numericField(event["partial_image_index"])
			eventName := imageReq.streamPrefix + ".partial_image"
			out := map[string]any{
				"type":                eventName,
				"partial_image_index": index,
			}
			if normalizeImageResponseFormat(imageReq.responseFormat) == "url" {
				out["url"] = "data:" + mimeTypeFromOutputFormat(stringField(event, "output_format")) + ";base64," + b64
			} else {
				out["b64_json"] = b64
			}
			data, _ := json.Marshal(out)
			writeEvent(eventName, data)
		case "response.completed":
			results, usage, _ := extractImageResults(event)
			if len(results) == 0 {
				writeErr(relayStatusError{status: http.StatusBadGateway, message: "upstream did not return image output"})
				return true
			}
			eventName := imageReq.streamPrefix + ".completed"
			for _, img := range results {
				out := map[string]any{"type": eventName}
				if normalizeImageResponseFormat(imageReq.responseFormat) == "url" {
					out["url"] = "data:" + mimeTypeFromOutputFormat(img.OutputFormat) + ";base64," + img.Result
				} else {
					out["b64_json"] = img.Result
				}
				if usage != nil {
					out["usage"] = usage
				}
				data, _ := json.Marshal(out)
				writeEvent(eventName, data)
			}
			return true
		}
	}
	return false
}

func stringField(payload map[string]any, key string) string {
	value, _ := payload[key].(string)
	return strings.TrimSpace(value)
}

func boolField(payload map[string]any, key string) bool {
	value, _ := payload[key].(bool)
	return value
}

func parseBoolString(raw string) bool {
	switch strings.ToLower(strings.TrimSpace(raw)) {
	case "1", "true", "yes", "on":
		return true
	default:
		return false
	}
}

func parseIntString(raw string) any {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil
	}
	value, err := strconv.ParseInt(raw, 10, 64)
	if err != nil {
		return nil
	}
	return value
}

func normalizeImageResponseFormat(value string) string {
	if strings.EqualFold(strings.TrimSpace(value), "url") {
		return "url"
	}
	return "b64_json"
}

func imageModelOrDefault(payload map[string]any) string {
	if model := strings.TrimSpace(stringField(payload, "model")); model != "" {
		return model
	}
	return defaultImagesToolModel
}

func buildImageTool(payload map[string]any, action string) (map[string]any, error) {
	model := imageModelOrDefault(payload)
	if modelBase(model) != defaultImagesToolModel {
		return nil, fmt.Errorf("model %s is not supported on %s or %s. Use %s.", model, imagesGenerationsPath, imagesEditsPath, defaultImagesToolModel)
	}
	tool := map[string]any{
		"type":   "image_generation",
		"action": action,
		"model":  defaultImagesToolModel,
	}
	for _, key := range []string{"size", "quality", "background", "output_format", "moderation"} {
		if value := stringField(payload, key); value != "" {
			tool[key] = value
		}
	}
	if action == "edit" {
		if value := stringField(payload, "input_fidelity"); value != "" {
			tool["input_fidelity"] = value
		}
	}
	for _, key := range []string{"output_compression", "partial_images"} {
		if value, ok := numericField(payload[key]); ok {
			tool[key] = value
		}
	}
	return tool, nil
}

func numericField(value any) (int64, bool) {
	switch v := value.(type) {
	case int:
		return int64(v), true
	case int64:
		return v, true
	case float64:
		return int64(v), true
	case json.Number:
		n, err := v.Int64()
		return n, err == nil
	default:
		return 0, false
	}
}

func jsonImageURLs(payload map[string]any) []string {
	var out []string
	if image := stringField(payload, "image"); image != "" {
		out = append(out, image)
	}
	if items, ok := payload["images"].([]any); ok {
		for _, item := range items {
			switch v := item.(type) {
			case string:
				if trimmed := strings.TrimSpace(v); trimmed != "" {
					out = append(out, trimmed)
				}
			case map[string]any:
				if url := stringField(v, "image_url"); url != "" {
					out = append(out, url)
				}
			}
		}
	}
	return out
}

func buildImagesResponsesPayload(prompt string, images []string, tool map[string]any) map[string]any {
	content := []any{map[string]any{
		"type": "input_text",
		"text": prompt,
	}}
	for _, image := range images {
		if image = strings.TrimSpace(image); image != "" {
			content = append(content, map[string]any{
				"type":      "input_image",
				"image_url": image,
			})
		}
	}
	return map[string]any{
		"instructions":        "",
		"stream":              true,
		"reasoning":           map[string]any{"effort": "medium", "summary": "auto"},
		"parallel_tool_calls": true,
		"include":             []string{"reasoning.encrypted_content"},
		"model":               defaultImagesMainModel,
		"store":               false,
		"tool_choice":         map[string]any{"type": "image_generation"},
		"input": []any{map[string]any{
			"type":    "message",
			"role":    "user",
			"content": content,
		}},
		"tools": []any{tool},
	}
}

func multipartFileToDataURL(fileHeader *multipart.FileHeader) (string, error) {
	if fileHeader == nil {
		return "", fmt.Errorf("upload file is nil")
	}
	if fileHeader.Size > maxImageUploadBytes {
		return "", fmt.Errorf("upload file exceeds %d bytes", maxImageUploadBytes)
	}
	file, err := fileHeader.Open()
	if err != nil {
		return "", err
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, maxImageUploadBytes+1))
	if err != nil {
		return "", err
	}
	if int64(len(data)) > maxImageUploadBytes {
		return "", fmt.Errorf("upload file exceeds %d bytes", maxImageUploadBytes)
	}
	mediaType := strings.TrimSpace(fileHeader.Header.Get("Content-Type"))
	if mediaType == "" {
		mediaType = http.DetectContentType(data)
	}
	return "data:" + mediaType + ";base64," + base64.StdEncoding.EncodeToString(data), nil
}

func collectImagesResponse(ctx context.Context, chunks <-chan cliproxyexecutor.StreamChunk, responseFormat string, idleTimeout time.Duration) ([]byte, error) {
	acc := &imageSSEAccumulator{}
	if idleTimeout <= 0 {
		idleTimeout = imageStreamIdleTimeout
	}
	idleTimer := time.NewTimer(idleTimeout)
	defer idleTimer.Stop()
	for {
		select {
		case <-idleTimer.C:
			return nil, relayTimeoutError{phase: "stream_idle", timeout: idleTimeout}
		case <-ctx.Done():
			return nil, ctx.Err()
		case chunk, ok := <-chunks:
			if !idleTimer.Stop() {
				select {
				case <-idleTimer.C:
				default:
				}
			}
			idleTimer.Reset(idleTimeout)
			if !ok {
				for _, frame := range acc.Flush() {
					if out, done, err := processImageResponseFrame(frame, responseFormat); err != nil {
						return nil, err
					} else if done {
						return out, nil
					}
				}
				return nil, relayStatusError{status: http.StatusBadGateway, message: "stream disconnected before completion"}
			}
			if chunk.Err != nil {
				return nil, chunk.Err
			}
			for _, frame := range acc.AddChunk(chunk.Payload) {
				if out, done, err := processImageResponseFrame(frame, responseFormat); err != nil {
					return nil, err
				} else if done {
					return out, nil
				}
			}
		}
	}
}

func processImageResponseFrame(frame []byte, responseFormat string) ([]byte, bool, error) {
	for _, payload := range imageFramePayloads(frame) {
		var event map[string]any
		if err := json.Unmarshal(payload, &event); err != nil {
			return nil, false, relayStatusError{status: http.StatusBadGateway, message: "invalid SSE data JSON"}
		}
		if stringField(event, "type") != "response.completed" {
			continue
		}
		results, usage, createdAt := extractImageResults(event)
		if len(results) == 0 {
			return nil, false, relayStatusError{status: http.StatusBadGateway, message: "upstream did not return image output"}
		}
		out, err := buildImagesAPIResponse(results, usage, createdAt, responseFormat)
		return out, true, err
	}
	return nil, false, nil
}

func imageFramePayloads(frame []byte) [][]byte {
	var payloads [][]byte
	for _, line := range bytes.Split(frame, []byte("\n")) {
		trimmed := bytes.TrimSpace(bytes.TrimRight(line, "\r"))
		if !bytes.HasPrefix(trimmed, []byte("data:")) {
			continue
		}
		payload := bytes.TrimSpace(trimmed[len("data:"):])
		if len(payload) == 0 || bytes.Equal(payload, []byte("[DONE]")) {
			continue
		}
		payloads = append(payloads, payload)
	}
	return payloads
}

func extractImageResults(event map[string]any) ([]imageRelayResult, any, int64) {
	createdAt := time.Now().Unix()
	if response, ok := event["response"].(map[string]any); ok {
		if created, ok := numericField(response["created_at"]); ok && created > 0 {
			createdAt = created
		}
		var usage any
		if toolUsage, ok := response["tool_usage"].(map[string]any); ok {
			usage = toolUsage["image_gen"]
		}
		var results []imageRelayResult
		if output, ok := response["output"].([]any); ok {
			for _, item := range output {
				obj, ok := item.(map[string]any)
				if !ok || stringField(obj, "type") != "image_generation_call" {
					continue
				}
				result := stringField(obj, "result")
				if result == "" {
					continue
				}
				results = append(results, imageRelayResult{
					Result:        result,
					RevisedPrompt: stringField(obj, "revised_prompt"),
					OutputFormat:  stringField(obj, "output_format"),
					Size:          stringField(obj, "size"),
					Background:    stringField(obj, "background"),
					Quality:       stringField(obj, "quality"),
				})
			}
		}
		return results, usage, createdAt
	}
	return nil, nil, createdAt
}

func buildImagesAPIResponse(results []imageRelayResult, usage any, createdAt int64, responseFormat string) ([]byte, error) {
	responseFormat = normalizeImageResponseFormat(responseFormat)
	data := make([]any, 0, len(results))
	for _, img := range results {
		item := map[string]any{}
		if responseFormat == "url" {
			item["url"] = "data:" + mimeTypeFromOutputFormat(img.OutputFormat) + ";base64," + img.Result
		} else {
			item["b64_json"] = img.Result
		}
		if img.RevisedPrompt != "" {
			item["revised_prompt"] = img.RevisedPrompt
		}
		data = append(data, item)
	}
	out := map[string]any{
		"created": createdAt,
		"data":    data,
	}
	if len(results) > 0 {
		first := results[0]
		if first.Background != "" {
			out["background"] = first.Background
		}
		if first.OutputFormat != "" {
			out["output_format"] = first.OutputFormat
		}
		if first.Quality != "" {
			out["quality"] = first.Quality
		}
		if first.Size != "" {
			out["size"] = first.Size
		}
	}
	if usage != nil {
		out["usage"] = usage
	}
	return json.Marshal(out)
}

func mimeTypeFromOutputFormat(outputFormat string) string {
	switch strings.ToLower(strings.TrimSpace(outputFormat)) {
	case "":
		return "image/png"
	case "png":
		return "image/png"
	case "jpg", "jpeg":
		return "image/jpeg"
	case "webp":
		return "image/webp"
	default:
		if strings.Contains(outputFormat, "/") {
			return outputFormat
		}
		return "image/png"
	}
}

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
	if _, ok := s.requireAPIKey(c); !ok {
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
	model := requestBodyModel(body)
	if model == "" {
		writeAPIError(c, http.StatusBadRequest, "model is required", "invalid_request")
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

func (s *relayServer) handleNonStream(c *gin.Context, body []byte, model string, sourceFormat sdktranslator.Format, alt string) {
	req, opts := buildExecutorRequest(c, body, model, sourceFormat, alt, false)
	startedAt := time.Now()
	s.emitExecutorDiagnostic(c, "executor_started", model, "execute", startedAt, "")
	stopWaitLogger := s.startExecutorWaitLogger(c, model, "execute", startedAt)
	resp, err := s.runtime.Execute(relayContext(c), []string{"codex"}, req, opts)
	stopWaitLogger()
	if err != nil {
		s.emitExecutorDiagnostic(c, "executor_failed", model, "execute", startedAt, err.Error())
		writeExecutorError(c, err)
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
	s.emitExecutorDiagnostic(c, "executor_started", model, "execute_stream", startedAt, "")
	stopWaitLogger := s.startExecutorWaitLogger(c, model, "execute_stream", startedAt)
	streamCtx, cancelStream := context.WithCancel(relayContext(c))
	defer cancelStream()
	result, err := s.executeStreamWithOpenTimeout(c, streamCtx, []string{"codex"}, req, opts, model, startedAt, timeouts.open)
	stopWaitLogger()
	if err != nil {
		s.emitExecutorDiagnostic(c, "executor_failed", model, "execute_stream", startedAt, err.Error())
		writeExecutorError(c, err)
		return
	}
	if result == nil || result.Chunks == nil {
		s.emitExecutorDiagnostic(c, "executor_failed", model, "execute_stream", startedAt, "upstream stream is unavailable")
		writeAPIError(c, http.StatusBadGateway, "upstream stream is unavailable", "bad_gateway")
		return
	}
	s.emitExecutorDiagnostic(c, "stream_opened", model, "execute_stream", startedAt, "")
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}

	setEventStreamHeaders(c.Writer.Header())
	writeUpstreamHeaders(c.Writer.Header(), result.Headers)
	c.Status(http.StatusOK)

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
			writeStreamTerminalError(c, err)
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
				writeStreamTerminalError(c, chunk.Err)
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

type streamTimeoutProfile struct {
	open time.Duration
	idle time.Duration
}

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

func writeExecutorError(c *gin.Context, err error) {
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
	writeAPIError(c, status, errorMessage(err), code)
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
	case sdktranslator.FormatOpenAI:
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
		if err := writeResponsesSSEChunk(w, f.pending[:frameLen]); err != nil {
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
	if err := writeResponsesSSEChunk(w, f.pending); err != nil {
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
	if err := writeResponsesSSEChunk(w, f.pending); err != nil {
		return err
	}
	f.pending = f.pending[:0]
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
	return isSSEFieldChunk(trimmed) || bytes.HasPrefix(trimmed, []byte("{")) || bytes.HasPrefix(trimmed, []byte("["))
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

func runRelayHTTPServer(ctx context.Context, cfg *config.Config, handler http.Handler, emitter *eventEmitter) error {
	host := "127.0.0.1"
	port := 0
	if cfg != nil {
		if strings.TrimSpace(cfg.Host) != "" {
			host = strings.TrimSpace(cfg.Host)
		}
		port = cfg.Port
	}
	listener, err := net.Listen("tcp", net.JoinHostPort(host, strconv.Itoa(port)))
	if err != nil {
		return err
	}
	server := &http.Server{
		Handler:           handler,
		ReadHeaderTimeout: 30 * time.Second,
	}
	errCh := make(chan error, 1)
	go func() {
		if serveErr := server.Serve(listener); serveErr != nil && !errors.Is(serveErr, http.ErrServerClosed) {
			errCh <- serveErr
			return
		}
		errCh <- nil
	}()
	if emitter != nil {
		readyPort := port
		if tcpAddr, ok := listener.Addr().(*net.TCPAddr); ok {
			readyPort = tcpAddr.Port
		}
		emitter.emit(map[string]any{"type": "ready", "port": readyPort, "host": host})
	}
	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		_ = server.Shutdown(shutdownCtx)
		return ctx.Err()
	case serveErr := <-errCh:
		return serveErr
	}
}

func monitorParentProcess(ctx context.Context, parentPID int, cancel context.CancelFunc, emitter *eventEmitter) {
	if parentPID <= 0 || parentPID == os.Getpid() {
		return
	}
	if goruntime.GOOS == "windows" {
		if emitter != nil {
			emitter.emit(map[string]any{
				"type":      "parent_monitor_disabled",
				"reason":    "windows_getppid_unreliable",
				"parentPid": parentPID,
			})
		}
		return
	}
	go func() {
		ticker := time.NewTicker(2 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				if os.Getppid() == parentPID {
					continue
				}
				if emitter != nil {
					emitter.emit(map[string]any{
						"type":      "parent_exit",
						"parentPid": parentPID,
					})
				}
				cancel()
				return
			}
		}
	}()
}

func main() {
	configPath := flag.String("config", "", "CLIProxyAPI config file")
	manifestPath := flag.String("manifest", "", "Cockpit sidecar manifest file")
	parentPID := flag.Int("parent-pid", 0, "Cockpit Tools parent process id")
	flag.Parse()

	emitter := &eventEmitter{}
	if strings.TrimSpace(*configPath) == "" || strings.TrimSpace(*manifestPath) == "" {
		emitter.emit(map[string]any{"type": "error", "message": "missing --config or --manifest"})
		os.Exit(2)
	}

	absConfigPath, err := filepath.Abs(*configPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}
	cfg, err := config.LoadConfig(absConfigPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}
	m, err := loadManifest(*manifestPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}

	usageTracker := newRequestUsageTracker()
	policy := &requestPolicy{manifest: m, emitter: emitter, tracker: usageTracker}
	hook := &authHook{manifest: m, emitter: emitter}
	selector := &cockpitSelector{manifest: m, emitter: emitter}
	coreManager := buildCoreAuthManager(cfg, selector, hook)

	signalCtx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	ctx, cancel := context.WithCancel(signalCtx)
	defer cancel()
	monitorParentProcess(ctx, *parentPID, cancel, emitter)

	coreusage.RegisterPlugin(&usagePlugin{manifest: m, tracker: usageTracker})

	runtime, err := newSidecarRuntime(ctx, absConfigPath, cfg, m, coreManager)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(1)
	}
	defer runtime.Stop()

	relay := &relayServer{
		runtime:  runtime,
		cfg:      cfg,
		manifest: m,
		emitter:  emitter,
		policy:   policy,
	}
	if err := runRelayHTTPServer(ctx, cfg, relay.router(), emitter); err != nil && !errors.Is(err, context.Canceled) {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(1)
	}
}
