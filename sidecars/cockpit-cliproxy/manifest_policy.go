package main

import (
	"bytes"
	"context"
	"crypto/sha256"

	"encoding/json"

	"fmt"
	"io"

	"net/http"

	"os"

	"path/filepath"

	"strconv"
	"strings"
	"sync"
	"sync/atomic"

	"time"

	"github.com/gin-gonic/gin"
	"github.com/klauspost/compress/zstd"

	codexmodels "github.com/router-for-me/CLIProxyAPI/v7/internal/client/codex/models"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"

	coreusage "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"

	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

type contextKey string

const (
	clientAPIKeyContextKey     contextKey = "cockpitClientAPIKey"
	requestKindContextKey      contextKey = "cockpitRequestKind"
	requestModelContextKey     contextKey = "cockpitRequestModel"
	clientInstanceIDContextKey contextKey = "cockpitClientInstanceId"
	clientInstanceIDHeaderName            = "X-Cockpit-Instance-Id"
)

const ginUserAPIKeyKey = "userApiKey"

const defaultStreamKeepAliveSeconds = 15
const quotaReserveMaxSnapshotAge = 3 * time.Minute
const codexAutoReviewModel = "codex-auto-review"
const codexSparkModel = "gpt-5.3-codex-spark"
const codexSparkCatalogTemplateModel = "gpt-5.3-codex"
const defaultImagesMainModel = "gpt-5.4-mini"
const defaultImagesToolModel = "gpt-image-2"
const imagesGenerationsPath = "/v1/images/generations"
const imagesEditsPath = "/v1/images/edits"
const anthropicMessagesPath = "/v1/messages"
const anthropicCountTokensPath = "/v1/messages/count_tokens"
const geminiModelsPath = "/v1beta/models"
const ollamaVersionPath = "/api/version"
const ollamaTagsPath = "/api/tags"
const ollamaShowPath = "/api/show"
const ollamaChatPath = "/api/chat"
const ollamaBridgeVersion = "0.18.3"
const maxImageUploadBytes int64 = 64 * 1024 * 1024
const codexAlphaSearchPath = "/v1/alpha/search"
const codexDirectAlphaSearchPath = "/backend-api/codex/alpha/search"
const cockpitQuotaPath = "/v1/cockpit/quota"
const defaultCodexAlphaSearchURL = "https://chatgpt.com/backend-api/codex/alpha/search"
const maxCodexAlphaSearchRequestBytes = 16 << 20
const maxCodexAlphaSearchResponseBytes = 32 << 20

var (
	streamOpenTimeout      = 10 * time.Second
	streamOpenMaxAttempts  = 2
	streamIdleTimeout      = 60 * time.Second
	imageStreamOpenTimeout = 10 * time.Second
	imageStreamIdleTimeout = 60 * time.Second
)

type accountModelRule struct {
	AccountID      string   `json:"accountId"`
	ExcludedModels []string `json:"excludedModels"`
}

type manifest struct {
	Locale                     string              `json:"locale"`
	APIKeys                    []apiKeySpec        `json:"apiKeys"`
	Accounts                   []accountSpec       `json:"accounts"`
	ModelIDs                   []string            `json:"modelIds"`
	ModelAliases               []modelAliasSpec    `json:"modelAliases"`
	ExcludedModels             []string            `json:"excludedModels"`
	AccountModelRules          []accountModelRule  `json:"accountModelRules"`
	RoutingStrategy            string              `json:"routingStrategy"`
	CustomRoutingRules         []customRoutingRule `json:"customRoutingRules"`
	ImmediateSSEResponse       bool                `json:"immediateSseResponse"`
	MaxConcurrentImageRequests int                 `json:"maxConcurrentImageRequests"`
	DebugLogs                  *bool               `json:"debugLogs,omitempty"`

	apiKeyByValue     map[string]*apiKeySpec
	accountByID       map[string]*accountSpec
	accountByAuthID   map[string]*accountSpec
	accountByAPIKey   map[string]*accountSpec
	accountByChatGPT  map[string]*accountSpec
	accountByEmail    map[string]*accountSpec
	aliasToSource     map[string]string
	originalIndexByID map[string]int
}

type apiKeySpec struct {
	ID                  string               `json:"id"`
	Label               string               `json:"label"`
	Key                 string               `json:"key"`
	ProviderGateway     *providerGatewaySpec `json:"providerGateway,omitempty"`
	ModelRouting        *modelRoutingSpec    `json:"modelRouting,omitempty"`
	BoundOAuth          bool                 `json:"boundOAuth,omitempty"`
	AccountIDs          []string             `json:"accountIds"`
	ModelPrefix         string               `json:"modelPrefix,omitempty"`
	ResponsesWebsockets bool                 `json:"responsesWebsockets,omitempty"`
	AllowedModels       []string             `json:"allowedModels"`
	ExcludedModels      []string             `json:"excludedModels"`
	TokenLimit          uint64               `json:"tokenLimit,omitempty"`
	TokenUsed           uint64               `json:"tokenUsed,omitempty"`
	Enabled             bool                 `json:"enabled"`
}

type modelRoutingSpec struct {
	DefaultRoute  string           `json:"defaultRoute"`
	FailurePolicy string           `json:"failurePolicy"`
	Routes        []modelRouteSpec `json:"routes"`
}

type modelRouteSpec struct {
	ID                string               `json:"id"`
	Namespace         string               `json:"namespace"`
	ProviderAccountID string               `json:"providerAccountId"`
	ProviderGateway   *providerGatewaySpec `json:"providerGateway"`
}

type apiKeyTokenState struct {
	limit uint64
	used  uint64
}

type apiKeyTokenLimiter struct {
	mu    sync.Mutex
	byKey map[string]*apiKeyTokenState
}

func apiKeyTokenIdentity(spec *apiKeySpec) string {
	if spec == nil {
		return ""
	}
	if id := strings.TrimSpace(spec.ID); id != "" {
		return id
	}
	return strings.TrimSpace(spec.Key)
}

func newAPIKeyTokenLimiter(m *manifest) *apiKeyTokenLimiter {
	limiter := &apiKeyTokenLimiter{byKey: make(map[string]*apiKeyTokenState)}
	if m == nil {
		return limiter
	}
	for i := range m.APIKeys {
		spec := &m.APIKeys[i]
		identity := apiKeyTokenIdentity(spec)
		if identity == "" {
			continue
		}
		limiter.byKey[identity] = &apiKeyTokenState{
			limit: spec.TokenLimit,
			used:  spec.TokenUsed,
		}
	}
	return limiter
}

func (l *apiKeyTokenLimiter) exceeded(spec *apiKeySpec) (used, limit uint64, exceeded bool) {
	if l == nil || spec == nil {
		return 0, 0, false
	}
	identity := apiKeyTokenIdentity(spec)
	if identity == "" {
		return 0, 0, false
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	state := l.byKey[identity]
	if state == nil {
		state = &apiKeyTokenState{limit: spec.TokenLimit, used: spec.TokenUsed}
		l.byKey[identity] = state
	}
	return state.used, state.limit, state.limit > 0 && state.used >= state.limit
}

func (l *apiKeyTokenLimiter) addUsage(spec *apiKeySpec, totalTokens int64) {
	if l == nil || spec == nil || totalTokens <= 0 {
		return
	}
	identity := apiKeyTokenIdentity(spec)
	if identity == "" {
		return
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	state := l.byKey[identity]
	if state == nil {
		state = &apiKeyTokenState{limit: spec.TokenLimit, used: spec.TokenUsed}
		l.byKey[identity] = state
	}
	addition := uint64(totalTokens)
	if ^uint64(0)-state.used < addition {
		state.used = ^uint64(0)
	} else {
		state.used += addition
	}
}

type apiKeyPriorityState struct {
	PriorityAccountIDs  map[string][]string `json:"priorityAccountIds"`
	PreferredAccountIDs map[string]string   `json:"preferredAccountIds"`
}

type apiKeyPriorityStateStore struct {
	path            string
	mu              sync.RWMutex
	lastModUnixNano int64
	priorities      map[string][]string
}

func newAPIKeyPriorityStateStore(manifestPath string) *apiKeyPriorityStateStore {
	store := &apiKeyPriorityStateStore{
		path:       filepath.Join(filepath.Dir(manifestPath), "api-key-priorities.json"),
		priorities: make(map[string][]string),
	}
	store.reloadIfChanged()
	return store
}

func (s *apiKeyPriorityStateStore) priorityAccountIDs(apiKeyID string) []string {
	if s == nil {
		return nil
	}
	s.reloadIfChanged()
	s.mu.RLock()
	defer s.mu.RUnlock()
	return append([]string(nil), s.priorities[strings.TrimSpace(apiKeyID)]...)
}

func (s *apiKeyPriorityStateStore) reloadIfChanged() {
	if s == nil || strings.TrimSpace(s.path) == "" {
		return
	}
	info, err := os.Stat(s.path)
	if err != nil {
		return
	}
	modifiedAt := info.ModTime().UnixNano()
	s.mu.RLock()
	unchanged := modifiedAt == s.lastModUnixNano
	s.mu.RUnlock()
	if unchanged {
		return
	}

	data, err := os.ReadFile(s.path)
	if err != nil {
		return
	}
	var state apiKeyPriorityState
	if err := json.Unmarshal(data, &state); err != nil {
		return
	}
	next := make(map[string][]string, len(state.PriorityAccountIDs))
	for apiKeyID, accountIDs := range state.PriorityAccountIDs {
		apiKeyID = strings.TrimSpace(apiKeyID)
		if apiKeyID == "" {
			continue
		}
		seen := make(map[string]struct{}, len(accountIDs))
		priorities := make([]string, 0, len(accountIDs))
		for _, accountID := range accountIDs {
			accountID = strings.TrimSpace(accountID)
			if accountID == "" {
				continue
			}
			if _, exists := seen[accountID]; exists {
				continue
			}
			seen[accountID] = struct{}{}
			priorities = append(priorities, accountID)
		}
		if len(priorities) > 0 {
			next[apiKeyID] = priorities
		}
	}
	for apiKeyID, accountID := range state.PreferredAccountIDs {
		apiKeyID = strings.TrimSpace(apiKeyID)
		accountID = strings.TrimSpace(accountID)
		if apiKeyID != "" && accountID != "" && len(next[apiKeyID]) == 0 {
			next[apiKeyID] = []string{accountID}
		}
	}
	s.mu.Lock()
	s.lastModUnixNano = modifiedAt
	s.priorities = next
	s.mu.Unlock()
}

type providerGatewaySpec struct {
	BaseURL            string                                    `json:"baseUrl"`
	APIKey             string                                    `json:"apiKey"`
	UpstreamModel      string                                    `json:"upstreamModel"`
	UpstreamModels     []string                                  `json:"upstreamModels,omitempty"`
	WireAPI            string                                    `json:"wireApi,omitempty"`
	SupportsVision     bool                                      `json:"supportsVision,omitempty"`
	ModelCapabilities  map[string]providerGatewayModelCapability `json:"modelCapabilities,omitempty"`
	VisionRoutingModel string                                    `json:"visionRoutingModel,omitempty"`
}

type providerGatewayModelCapability struct {
	SupportsVision bool `json:"supportsVision,omitempty"`
}

type accountSpec struct {
	ID                    string            `json:"id"`
	Email                 string            `json:"email"`
	AuthID                string            `json:"authId,omitempty"`
	AuthKind              string            `json:"authKind,omitempty"`
	PlanType              string            `json:"planType,omitempty"`
	AccessTokenOnly       bool              `json:"accessTokenOnly,omitempty"`
	ChatGPTAccountID      string            `json:"chatgptAccountId,omitempty"`
	UpstreamAPIKey        string            `json:"upstreamApiKey,omitempty"`
	PlanRank              *int              `json:"planRank,omitempty"`
	RemainingQuota        *int              `json:"remainingQuota,omitempty"`
	SubscriptionExpiryMS  *int64            `json:"subscriptionExpiryMs,omitempty"`
	ImageGenerationPolicy string            `json:"imageGenerationPolicy,omitempty"`
	QuotaReserve          *quotaReserveSpec `json:"quotaReserve,omitempty"`
	ModelContextWindows   map[string]int64  `json:"modelContextWindows,omitempty"`
}

type quotaReserveSpec struct {
	HourlyThresholdPercent       *int   `json:"hourlyThresholdPercent,omitempty"`
	WeeklyThresholdPercent       *int   `json:"weeklyThresholdPercent,omitempty"`
	SnapshotUpdatedAtUnixSeconds *int64 `json:"snapshotUpdatedAtUnixSeconds,omitempty"`
	HourlyRemainingPercent       *int   `json:"hourlyRemainingPercent,omitempty"`
	WeeklyRemainingPercent       *int   `json:"weeklyRemainingPercent,omitempty"`
	HourlyWindowPresent          *bool  `json:"hourlyWindowPresent,omitempty"`
	WeeklyWindowPresent          *bool  `json:"weeklyWindowPresent,omitempty"`
}

type quotaReserveSnapshot struct {
	SnapshotUpdatedAtUnixSeconds *int64 `json:"snapshotUpdatedAtUnixSeconds,omitempty"`
	HourlyRemainingPercent       *int   `json:"hourlyRemainingPercent,omitempty"`
	WeeklyRemainingPercent       *int   `json:"weeklyRemainingPercent,omitempty"`
	HourlyWindowPresent          *bool  `json:"hourlyWindowPresent,omitempty"`
	WeeklyWindowPresent          *bool  `json:"weeklyWindowPresent,omitempty"`
}

type quotaReserveStateFile struct {
	Accounts map[string]quotaReserveSnapshot `json:"accounts"`
}

type quotaReserveStateStore struct {
	path     string
	snapshot atomic.Value
	mu       sync.Mutex
	lastHash [sha256.Size]byte
	hasHash  bool
}

type modelAliasSpec struct {
	SourceModel string `json:"sourceModel"`
	Alias       string `json:"alias"`
	Fork        bool   `json:"fork"`
}

type customRoutingRule struct {
	AccountID   string `json:"accountId"`
	Priority    int    `json:"priority"`
	Weight      int    `json:"weight"`
	IsBackup    bool   `json:"isBackup"`
	IsPreferred bool   `json:"isPreferred"`
}

type usagePayload struct {
	Type             string       `json:"type"`
	RequestID        string       `json:"requestId,omitempty"`
	Provider         string       `json:"provider,omitempty"`
	Model            string       `json:"model,omitempty"`
	Alias            string       `json:"alias,omitempty"`
	AccountID        string       `json:"accountId,omitempty"`
	AccountEmail     string       `json:"accountEmail,omitempty"`
	AuthID           string       `json:"authId,omitempty"`
	APIKeyID         string       `json:"apiKeyId,omitempty"`
	APIKeyLabel      string       `json:"apiKeyLabel,omitempty"`
	ClientInstanceID string       `json:"clientInstanceId,omitempty"`
	RequestKind      string       `json:"requestKind,omitempty"`
	ServiceTier      string       `json:"serviceTier,omitempty"`
	ReasoningEffort  string       `json:"reasoningEffort,omitempty"`
	Success          bool         `json:"success"`
	Status           int          `json:"status,omitempty"`
	ErrorCategory    string       `json:"errorCategory,omitempty"`
	ErrorMessage     string       `json:"errorMessage,omitempty"`
	LatencyMS        int64        `json:"latencyMs,omitempty"`
	Usage            usageDetails `json:"usage"`
	RequestedAtMS    int64        `json:"requestedAtMs,omitempty"`
}

type requestDiagnosticPayload struct {
	Type                    string                     `json:"type"`
	RequestID               string                     `json:"requestId,omitempty"`
	Method                  string                     `json:"method,omitempty"`
	Path                    string                     `json:"path,omitempty"`
	RequestKind             string                     `json:"requestKind,omitempty"`
	Model                   string                     `json:"model,omitempty"`
	APIKeyID                string                     `json:"apiKeyId,omitempty"`
	APIKeyLabel             string                     `json:"apiKeyLabel,omitempty"`
	Transport               string                     `json:"transport,omitempty"`
	Status                  int                        `json:"status,omitempty"`
	LatencyMS               int64                      `json:"latencyMs,omitempty"`
	StartedAtMS             int64                      `json:"startedAtMs,omitempty"`
	CompletedAtMS           int64                      `json:"completedAtMs,omitempty"`
	Aborted                 bool                       `json:"aborted,omitempty"`
	ErrorMessage            string                     `json:"errorMessage,omitempty"`
	CandidateAuths          int                        `json:"candidateAuths,omitempty"`
	ScopedAuths             int                        `json:"scopedAuths,omitempty"`
	AvailableAuths          int                        `json:"availableAuths,omitempty"`
	UnavailableAuths        int                        `json:"unavailableAuths,omitempty"`
	ModelExcludedAuths      int                        `json:"modelExcludedAuths,omitempty"`
	QuotaReservedAuths      int                        `json:"quotaReservedAuths,omitempty"`
	ImagePolicyBlockedAuths int                        `json:"imagePolicyBlockedAuths,omitempty"`
	RoutingStrategy         string                     `json:"routingStrategy,omitempty"`
	Provider                string                     `json:"provider,omitempty"`
	AuthID                  string                     `json:"authId,omitempty"`
	AccountID               string                     `json:"accountId,omitempty"`
	AccountEmail            string                     `json:"accountEmail,omitempty"`
	Success                 *bool                      `json:"success,omitempty"`
	ErrorCode               string                     `json:"errorCode,omitempty"`
	HTTPStatus              int                        `json:"httpStatus,omitempty"`
	Retryable               *bool                      `json:"retryable,omitempty"`
	RetryAfterMS            int64                      `json:"retryAfterMs,omitempty"`
	AuthAvailable           *bool                      `json:"authAvailable,omitempty"`
	NextRetryAtMS           int64                      `json:"nextRetryAtMs,omitempty"`
	AuthStateReason         string                     `json:"authStateReason,omitempty"`
	AccountStatuses         []authPoolMemberDiagnostic `json:"accountStatuses,omitempty"`
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
	InputTokens     int64                    `json:"inputTokens,omitempty"`
	OutputTokens    int64                    `json:"outputTokens,omitempty"`
	ReasoningTokens int64                    `json:"reasoningTokens,omitempty"`
	CachedTokens    int64                    `json:"cachedTokens,omitempty"`
	TotalTokens     int64                    `json:"totalTokens,omitempty"`
	TokenBreakdown  coreusage.TokenBreakdown `json:"tokenBreakdown,omitempty"`
}

func effectiveUsageTotalTokens(usage usageDetails) int64 {
	if usage.TotalTokens > 0 {
		return usage.TotalTokens
	}
	input := max(usage.InputTokens, 0)
	output := max(usage.OutputTokens, 0)
	if input > int64(^uint64(0)>>1)-output {
		return int64(^uint64(0) >> 1)
	}
	return input + output
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

type selectedAccountRecord struct {
	AccountID    string
	AccountEmail string
	AuthID       string
}

type requestUsageTracker struct {
	mu               sync.Mutex
	records          map[string][]usagePayload
	selectedAccounts map[string]selectedAccountRecord
	imageJobs        map[string]map[string]struct{}
	imageInFlight    map[string]int
	imageJobsChanged chan struct{}
}

func newRequestUsageTracker() *requestUsageTracker {
	return &requestUsageTracker{
		records:          make(map[string][]usagePayload),
		selectedAccounts: make(map[string]selectedAccountRecord),
		imageJobs:        make(map[string]map[string]struct{}),
		imageInFlight:    make(map[string]int),
		imageJobsChanged: make(chan struct{}),
	}
}

func (t *requestUsageTracker) imageInFlightCount(authID string) int {
	if t == nil {
		return 0
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.imageInFlight[strings.TrimSpace(authID)]
}

func (t *requestUsageTracker) tryReserveImageJob(requestID, authID string, maxConcurrent int) bool {
	if t == nil {
		return true
	}
	requestID = strings.TrimSpace(requestID)
	authID = strings.TrimSpace(authID)
	if requestID == "" || authID == "" {
		return false
	}
	if maxConcurrent < 1 {
		maxConcurrent = 1
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	jobs := t.imageJobs[requestID]
	if jobs == nil {
		jobs = make(map[string]struct{})
		t.imageJobs[requestID] = jobs
	}
	if _, alreadyReserved := jobs[authID]; alreadyReserved {
		return true
	}
	if t.imageInFlight[authID] >= maxConcurrent {
		return false
	}
	jobs[authID] = struct{}{}
	t.imageInFlight[authID]++
	return true
}

func (t *requestUsageTracker) imageJobChangeSignal() <-chan struct{} {
	if t == nil {
		return nil
	}
	t.mu.Lock()
	changed := t.imageJobsChanged
	t.mu.Unlock()
	return changed
}

func (t *requestUsageTracker) notifyImageJobChangeLocked() {
	if t == nil || t.imageJobsChanged == nil {
		return
	}
	close(t.imageJobsChanged)
	t.imageJobsChanged = make(chan struct{})
}

func (t *requestUsageTracker) releaseImageJobs(requestID string) {
	if t == nil {
		return
	}
	requestID = strings.TrimSpace(requestID)
	if requestID == "" {
		return
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	for authID := range t.imageJobs[requestID] {
		if t.imageInFlight[authID] <= 1 {
			delete(t.imageInFlight, authID)
		} else {
			t.imageInFlight[authID]--
		}
	}
	delete(t.imageJobs, requestID)
	t.notifyImageJobChangeLocked()
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

func (t *requestUsageTracker) recordSelectedAccount(requestID string, account *accountSpec, authID string) {
	if t == nil {
		return
	}
	requestID = strings.TrimSpace(requestID)
	if requestID == "" || account == nil {
		return
	}
	t.mu.Lock()
	t.selectedAccounts[requestID] = selectedAccountRecord{
		AccountID:    strings.TrimSpace(account.ID),
		AccountEmail: strings.TrimSpace(account.Email),
		AuthID:       strings.TrimSpace(authID),
	}
	t.mu.Unlock()
}

func normalizedUsageServiceTier(value string) string {
	switch strings.ToLower(strings.TrimSpace(value)) {
	case "priority":
		return "priority"
	case "", "default", "standard":
		return ""
	default:
		return ""
	}
}

func (t *requestUsageTracker) finalize(requestID string, input usageFinalizeInput) (usagePayload, bool) {
	requestID = strings.TrimSpace(requestID)
	if requestID == "" {
		return usagePayload{}, false
	}

	var records []usagePayload
	var selected selectedAccountRecord
	var selectedOK bool
	if t != nil {
		t.mu.Lock()
		records = append(records, t.records[requestID]...)
		delete(t.records, requestID)
		selected, selectedOK = t.selectedAccounts[requestID]
		delete(t.selectedAccounts, requestID)
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
	if selectedOK {
		payload.AccountID = selected.AccountID
		payload.AccountEmail = selected.AccountEmail
		payload.AuthID = selected.AuthID
	} else {
		payload.AccountID = ""
		payload.AccountEmail = ""
		payload.AuthID = ""
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

func (e *eventEmitter) emitStartupStage(stage string) {
	e.emit(map[string]any{"type": "startup", "stage": stage})
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
		if gateway := m.APIKeys[i].ProviderGateway; gateway != nil {
			if !normalizeProviderGatewaySpec(gateway) {
				m.APIKeys[i].ProviderGateway = nil
			}
		}
		if routing := m.APIKeys[i].ModelRouting; routing != nil {
			routing.DefaultRoute = strings.ToLower(strings.TrimSpace(routing.DefaultRoute))
			routing.FailurePolicy = strings.ToLower(strings.TrimSpace(routing.FailurePolicy))
			seenNamespaces := make(map[string]struct{}, len(routing.Routes))
			routes := make([]modelRouteSpec, 0, len(routing.Routes))
			for _, route := range routing.Routes {
				route.ID = strings.TrimSpace(route.ID)
				route.Namespace = strings.ToLower(strings.Trim(strings.TrimSpace(route.Namespace), "/"))
				route.ProviderAccountID = strings.TrimSpace(route.ProviderAccountID)
				if route.ID == "" || route.Namespace == "" || strings.Contains(route.Namespace, "/") {
					continue
				}
				if _, exists := seenNamespaces[route.Namespace]; exists {
					continue
				}
				if route.ProviderGateway == nil || !normalizeProviderGatewaySpec(route.ProviderGateway) {
					continue
				}
				seenNamespaces[route.Namespace] = struct{}{}
				routes = append(routes, route)
			}
			routing.Routes = routes
			if routing.DefaultRoute != "oauth" || routing.FailurePolicy != "strict" || len(routes) == 0 {
				m.APIKeys[i].ModelRouting = nil
			}
		}
		m.apiKeyByValue[key] = &m.APIKeys[i]
	}
	m.accountByID = make(map[string]*accountSpec)
	m.accountByAuthID = make(map[string]*accountSpec)
	m.accountByAPIKey = make(map[string]*accountSpec)
	m.accountByChatGPT = make(map[string]*accountSpec)
	m.accountByEmail = make(map[string]*accountSpec)
	m.originalIndexByID = make(map[string]int)
	for i := range m.Accounts {
		account := &m.Accounts[i]
		account.ID = strings.TrimSpace(account.ID)
		if account.ID == "" {
			continue
		}
		account.Email = strings.TrimSpace(account.Email)
		account.AuthKind = strings.ToLower(strings.TrimSpace(account.AuthKind))
		account.ChatGPTAccountID = strings.TrimSpace(account.ChatGPTAccountID)
		m.accountByID[account.ID] = account
		m.originalIndexByID[account.ID] = i
		if authID := strings.TrimSpace(account.AuthID); authID != "" {
			account.AuthID = authID
			m.accountByAuthID[strings.ToLower(authID)] = account
			if base := filepath.Base(authID); base != authID {
				m.accountByAuthID[strings.ToLower(base)] = account
			}
		}
		if key := strings.TrimSpace(account.UpstreamAPIKey); key != "" {
			account.UpstreamAPIKey = key
			m.accountByAPIKey[key] = account
		}
		if account.ChatGPTAccountID != "" {
			m.accountByChatGPT[strings.ToLower(account.ChatGPTAccountID)] = account
		}
		if account.Email != "" {
			key := strings.ToLower(account.Email)
			if existing, exists := m.accountByEmail[key]; exists && existing != account {
				m.accountByEmail[key] = nil
			} else {
				m.accountByEmail[key] = account
			}
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
	for index := range m.AccountModelRules {
		m.AccountModelRules[index].AccountID = strings.TrimSpace(m.AccountModelRules[index].AccountID)
		m.AccountModelRules[index].ExcludedModels = normalizeStringList(m.AccountModelRules[index].ExcludedModels)
	}
	return &m, nil
}

func normalizeProviderGatewaySpec(gateway *providerGatewaySpec) bool {
	if gateway == nil {
		return false
	}
	gateway.BaseURL = strings.TrimSpace(gateway.BaseURL)
	gateway.APIKey = strings.TrimSpace(gateway.APIKey)
	gateway.UpstreamModel = strings.TrimSpace(gateway.UpstreamModel)
	gateway.UpstreamModels = normalizeStringList(gateway.UpstreamModels)
	gateway.VisionRoutingModel = strings.TrimSpace(gateway.VisionRoutingModel)
	if len(gateway.UpstreamModels) == 0 && gateway.UpstreamModel != "" {
		gateway.UpstreamModels = []string{gateway.UpstreamModel}
	}
	gateway.WireAPI = normalizeProviderGatewayWireAPI(gateway.WireAPI)
	gateway.ModelCapabilities = normalizeProviderGatewayModelCapabilities(gateway.ModelCapabilities)
	return gateway.BaseURL != "" && gateway.APIKey != "" && len(gateway.UpstreamModels) > 0
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

func normalizeProviderGatewayWireAPI(value string) string {
	switch strings.ToLower(strings.TrimSpace(value)) {
	case "chat_completions", "chat-completions", "openai_chat", "openai-chat", "chat":
		return "chat_completions"
	default:
		return "responses"
	}
}

func normalizeProviderGatewayModelCapabilities(value map[string]providerGatewayModelCapability) map[string]providerGatewayModelCapability {
	if len(value) == 0 {
		return nil
	}
	out := make(map[string]providerGatewayModelCapability, len(value))
	for model, capability := range value {
		key := strings.ToLower(strings.TrimSpace(model))
		if key == "" {
			continue
		}
		out[key] = capability
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func sourceFormatEqual(from, want sdktranslator.Format) bool {
	return strings.EqualFold(strings.TrimSpace(from.String()), strings.TrimSpace(want.String()))
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

func extractClientInstanceID(r *http.Request) string {
	if r == nil {
		return ""
	}
	if value := strings.TrimSpace(r.Header.Get(clientInstanceIDHeaderName)); value != "" {
		return value
	}
	// Header map is case-insensitive via Get; keep an explicit fallback for odd clients.
	for key, values := range r.Header {
		if strings.EqualFold(strings.TrimSpace(key), clientInstanceIDHeaderName) {
			for _, value := range values {
				if trimmed := strings.TrimSpace(value); trimmed != "" {
					return trimmed
				}
			}
		}
	}
	return ""
}

func clientInstanceIDFromContext(ctx context.Context) string {
	if ctx == nil {
		return ""
	}
	value, _ := ctx.Value(clientInstanceIDContextKey).(string)
	return strings.TrimSpace(value)
}

func withClientInstanceID(ctx context.Context, instanceID string) context.Context {
	instanceID = strings.TrimSpace(instanceID)
	if instanceID == "" || ctx == nil {
		return ctx
	}
	return context.WithValue(ctx, clientInstanceIDContextKey, instanceID)
}

type requestPolicy struct {
	manifest     *manifest
	emitter      *eventEmitter
	tracker      *requestUsageTracker
	tokenLimiter *apiKeyTokenLimiter
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
		clientInstanceID := extractClientInstanceID(c.Request)
		if clientInstanceID != "" {
			c.Request = c.Request.WithContext(withClientInstanceID(c.Request.Context(), clientInstanceID))
		}
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
			if clientInstanceID != "" {
				ctx = withClientInstanceID(ctx, clientInstanceID)
			}
			c.Request = c.Request.WithContext(ctx)
		}

		if spec != nil && isModelsRequest(c.Request) {
			models := clientCatalogModelsForAPIKey(p.manifest, spec)
			if isCodexClientModelsRequest(c.Request) {
				c.JSON(http.StatusOK, buildCodexClientModelsResponse(models, spec, contextWindowsForAPIKey(p.manifest, spec)))
			} else {
				c.JSON(http.StatusOK, buildModelsResponse(models))
			}
			c.Abort()
			return
		}

		if used, limit, exceeded := p.tokenLimiter.exceeded(spec); shouldEmitRequestDiagnostic(c.Request) && exceeded {
			emitStart()
			message := fmt.Sprintf(
				"API key token limit exceeded (%d of %d tokens used)",
				used,
				limit,
			)
			p.emitTokenLimitBlockedRequest(
				c,
				requestID,
				spec,
				model,
				requestKind,
				startedAt,
				message,
			)
			c.AbortWithStatusJSON(http.StatusTooManyRequests, gin.H{
				"error": gin.H{
					"message": message,
					"type":    "invalid_request_error",
					"code":    "token_limit_exceeded",
				},
			})
			return
		}

		if spec == nil || isCodexLiveRequest(c.Request) || !shouldInspectJSONBody(c.Request) {
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
			p.emitBlockedRequest(c, requestID, spec, model, requestKind, startedAt, err.Error())
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
	p.tracker.releaseImageJobs(requestID)
	if payload, ok := p.tracker.finalize(requestID, usageFinalizeInput{
		spec:          spec,
		requestKind:   requestKind,
		model:         model,
		status:        status,
		latencyMS:     latencyMS,
		completedAtMS: completedAtMS,
		errorMessage:  strings.TrimSpace(c.Errors.String()),
	}); ok {
		p.tokenLimiter.addUsage(spec, effectiveUsageTotalTokens(payload.Usage))
		p.emitter.emit(payload)
	}
}

func (p *requestPolicy) emitTokenLimitBlockedRequest(c *gin.Context, requestID string, spec *apiKeySpec, model, requestKind string, startedAt time.Time, message string) {
	if p == nil || spec == nil {
		return
	}
	clientInstanceID := ""
	if c != nil && c.Request != nil {
		clientInstanceID = clientInstanceIDFromContext(c.Request.Context())
	}
	payload := usagePayload{
		Type:             "usage",
		RequestID:        requestID,
		Model:            model,
		APIKeyID:         spec.ID,
		APIKeyLabel:      spec.Label,
		ClientInstanceID: clientInstanceID,
		RequestKind:      requestKind,
		Success:          false,
		Status:           http.StatusTooManyRequests,
		ErrorCategory:    "token_limit_exceeded",
		ErrorMessage:     message,
		LatencyMS:        time.Since(startedAt).Milliseconds(),
		RequestedAtMS:    time.Now().UnixMilli(),
	}
	if p.tracker != nil {
		p.tracker.record(payload)
		return
	}
	if p.emitter != nil {
		p.emitter.emit(payload)
	}
}

func (p *requestPolicy) emitBlockedRequest(c *gin.Context, requestID string, spec *apiKeySpec, model, requestKind string, startedAt time.Time, message string) {
	if p == nil || spec == nil {
		return
	}
	clientInstanceID := ""
	if c != nil && c.Request != nil {
		clientInstanceID = clientInstanceIDFromContext(c.Request.Context())
	}
	payload := usagePayload{
		Type:             "usage",
		RequestID:        requestID,
		Model:            model,
		APIKeyID:         spec.ID,
		APIKeyLabel:      spec.Label,
		ClientInstanceID: clientInstanceID,
		RequestKind:      requestKind,
		Success:          false,
		Status:           http.StatusNotFound,
		ErrorCategory:    "model_not_available",
		ErrorMessage:     message,
		LatencyMS:        time.Since(startedAt).Milliseconds(),
		RequestedAtMS:    time.Now().UnixMilli(),
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

func buildGeminiModelsResponse(models []string) gin.H {
	data := make([]gin.H, 0, len(models))
	for _, model := range models {
		data = append(data, buildGeminiModelEntry(model))
	}
	return gin.H{"models": data}
}

func buildGeminiModelEntry(model string) gin.H {
	displayName := displayNameForModel(model)
	return gin.H{
		"name":                       "models/" + model,
		"baseModelId":                model,
		"version":                    "001",
		"displayName":                displayName,
		"description":                displayName,
		"inputTokenLimit":            1000000,
		"outputTokenLimit":           128000,
		"supportedGenerationMethods": []string{"generateContent", "streamGenerateContent", "countTokens"},
	}
}

func buildOllamaTagsResponse(models []string, modifiedAt time.Time) gin.H {
	data := make([]gin.H, 0, len(models))
	for _, model := range models {
		data = append(data, buildOllamaTag(model, modifiedAt))
	}
	return gin.H{"models": data}
}

func buildOllamaTag(model string, modifiedAt time.Time) gin.H {
	family := ollamaModelFamily(model)
	return gin.H{
		"name":        model,
		"model":       model,
		"modified_at": modifiedAt.Format(time.RFC3339Nano),
		"size":        0,
		"digest":      fmt.Sprintf("%x", sha256.Sum256([]byte(model))),
		"details": gin.H{
			"parent_model":       "",
			"format":             "cockpit-codex-api-service",
			"family":             family,
			"families":           []string{family},
			"parameter_size":     "unknown",
			"quantization_level": "unknown",
		},
	}
}

func buildOllamaShowResponse(model string, modifiedAt time.Time) gin.H {
	family := ollamaModelFamily(model)
	contextLength := ollamaContextLength(model)
	return gin.H{
		"model":        model,
		"remote_model": model,
		"license":      "Proxied via Cockpit Codex API Service",
		"modelfile":    "FROM " + model,
		"parameters":   fmt.Sprintf("num_ctx %d", contextLength),
		"template":     "{{ .Prompt }}",
		"capabilities": []string{
			"completion",
			"tools",
			"thinking",
		},
		"modified_at": modifiedAt.Format(time.RFC3339Nano),
		"details":     buildOllamaTag(model, modifiedAt)["details"],
		"model_info": gin.H{
			"general.architecture":        family,
			family + ".context_length":    contextLength,
			"general.basename":            model,
			"upstream_id":                 model,
			"display_name":                displayNameForModel(model),
			"input_modalities":            []string{"text", "image"},
			"context_length":              contextLength,
			"supported_reasoning_efforts": ollamaReasoningEfforts(model),
			"default_reasoning_effort":    ollamaDefaultReasoningEffort(model),
		},
	}
}

func ollamaModelFamily(model string) string {
	normalized := strings.ToLower(strings.TrimSpace(model))
	for _, prefix := range []string{"gpt-5.6", "gpt-5.5", "gpt-5.4", "gpt-5.3", "gpt-5.2", "gpt-5.1", "gpt-oss", "codex"} {
		if strings.HasPrefix(normalized, prefix) {
			return prefix
		}
	}
	for _, sep := range []string{":", "/", "-"} {
		if index := strings.Index(normalized, sep); index > 0 {
			return normalized[:index]
		}
	}
	if normalized == "" {
		return "codex"
	}
	return normalized
}

func ollamaContextLength(model string) int {
	switch {
	case strings.HasPrefix(model, "gpt-5.6"):
		return 372000
	case strings.HasPrefix(model, "gpt-5.5"), strings.HasPrefix(model, "gpt-5.4"):
		return 400000
	case strings.HasPrefix(model, "gpt-5.3"), strings.HasPrefix(model, "gpt-5.2"), strings.HasPrefix(model, "gpt-5.1"):
		return 272000
	default:
		return 131072
	}
}

func ollamaReasoningEfforts(model string) []string {
	switch {
	case strings.HasPrefix(model, "gpt-5.6-sol"), strings.HasPrefix(model, "gpt-5.6-terra"):
		return []string{"low", "medium", "high", "xhigh", "max", "ultra"}
	case strings.HasPrefix(model, "gpt-5.6-luna"), strings.HasPrefix(model, "gpt-5.6"):
		return []string{"low", "medium", "high", "xhigh", "max"}
	default:
		return []string{"low", "medium", "high", "xhigh"}
	}
}

func ollamaDefaultReasoningEffort(model string) string {
	if strings.HasPrefix(model, "gpt-5.6-sol") {
		return "low"
	}
	return "medium"
}

func lookupExplicitContextWindow(windows map[string]int64, slug string) int64 {
	slug = strings.TrimSpace(slug)
	if slug == "" || len(windows) == 0 {
		return 0
	}
	candidates := []string{slug}
	if index := strings.LastIndex(slug, "/"); index >= 0 {
		candidates = append(candidates, strings.TrimSpace(slug[index+1:]))
	}
	for _, candidate := range candidates {
		if candidate == "" {
			continue
		}
		if window, ok := windows[candidate]; ok && window > 0 {
			return window
		}
		for name, window := range windows {
			if window > 0 && strings.EqualFold(strings.TrimSpace(name), candidate) {
				return window
			}
		}
	}
	return 0
}

func contextWindowsForAPIKey(m *manifest, spec *apiKeySpec) map[string]int64 {
	if m == nil {
		return nil
	}
	accountIDs := make([]string, 0)
	if spec != nil {
		accountIDs = append(accountIDs, spec.AccountIDs...)
	}
	if len(accountIDs) == 0 {
		for i := range m.Accounts {
			if id := strings.TrimSpace(m.Accounts[i].ID); id != "" {
				accountIDs = append(accountIDs, id)
			}
		}
	}
	merged := make(map[string]int64)
	for _, accountID := range accountIDs {
		account := m.accountByID[strings.TrimSpace(accountID)]
		if account == nil {
			continue
		}
		for name, window := range account.ModelContextWindows {
			key := strings.TrimSpace(name)
			if key == "" || window <= 0 {
				continue
			}
			merged[key] = window
		}
	}
	if len(merged) == 0 {
		return nil
	}
	return merged
}

func applyExplicitContextWindows(models []map[string]any, windows map[string]int64) {
	if len(windows) == 0 {
		return
	}
	for _, model := range models {
		slug, _ := model["slug"].(string)
		if window := lookupExplicitContextWindow(windows, slug); window > 0 {
			model["context_window"] = window
			model["max_context_window"] = window
		}
	}
}

func buildCodexClientModelsResponse(models []string, spec *apiKeySpec, windows map[string]int64) gin.H {
	sourceModels := make([]map[string]any, 0, len(models))
	for _, model := range models {
		displayName := displayNameForModel(model)
		entry := map[string]any{
			"id":           model,
			"display_name": displayName,
			"description":  displayName,
		}
		// Only seed context_length for non-template models. Template models keep
		// official context/service-tier values from codex_client_models.json.
		if cw := ollamaContextLength(model); cw > 0 {
			entry["context_length"] = cw
		}
		sourceModels = append(sourceModels, entry)
	}
	response := gin.H(codexmodels.BuildResponse(sourceModels, func(string) []string {
		if spec != nil && spec.ProviderGateway != nil {
			return []string{"provider-gateway"}
		}
		return []string{"codex"}
	}, false))
	if data, ok := response["models"].([]map[string]any); ok {
		hydrateCodexCompatibilityModels(data)
		preferWebsockets := spec != nil && spec.ProviderGateway == nil && spec.ResponsesWebsockets
		for _, model := range data {
			model["prefer_websockets"] = preferWebsockets
			if spec != nil && spec.ProviderGateway != nil {
				applyProviderGatewayCodexInputModalities(model, spec.ProviderGateway)
			}
			slug, _ := model["slug"].(string)
			if isHiddenCodexClientModel(slug) {
				model["visibility"] = "hide"
			}
			// Preserve template priority/context/service_tiers. Only fill gaps
			// for synthesized models that lack official catalog fields.
			if _, ok := model["max_context_window"]; !ok {
				if cw := intModelValueAny(model["context_window"]); cw > 0 {
					model["max_context_window"] = cw
				}
			}
			if _, ok := model["additional_speed_tiers"]; !ok {
				model["additional_speed_tiers"] = []any{}
			}
			if _, ok := model["service_tiers"]; !ok {
				model["service_tiers"] = []any{}
			}
			if _, ok := model["availability_nux"]; !ok {
				model["availability_nux"] = nil
			}
			if _, ok := model["upgrade"]; !ok {
				model["upgrade"] = nil
			}
		}
		applyExplicitContextWindows(data, windows)
	}
	return response
}

func applyProviderGatewayCodexInputModalities(model map[string]any, gateway *providerGatewaySpec) {
	slug, _ := model["slug"].(string)
	slug = strings.TrimSpace(slug)
	supportsImage := providerGatewayModelSupportsVision(gateway, slug) ||
		strings.TrimSpace(providerGatewayVisionRoutingModel(gateway)) != ""
	if supportsImage {
		model["input_modalities"] = []any{"text", "image"}
		model["supports_image_detail_original"] = true
		return
	}
	model["input_modalities"] = []any{"text"}
	delete(model, "supports_image_detail_original")
}

func intModelValueAny(value any) int {
	switch v := value.(type) {
	case int:
		return v
	case int64:
		return int(v)
	case float64:
		return int(v)
	default:
		return 0
	}
}

func hydrateCodexCompatibilityModels(models []map[string]any) {
	var template map[string]any
	for _, model := range models {
		if model["slug"] == codexSparkCatalogTemplateModel {
			template = model
			break
		}
	}
	if template == nil {
		return
	}

	for index, model := range models {
		if model["slug"] != codexSparkModel {
			continue
		}
		compatibilityModel := make(map[string]any, len(template))
		for key, value := range template {
			compatibilityModel[key] = value
		}
		compatibilityModel["slug"] = codexSparkModel
		compatibilityModel["display_name"] = "GPT-5.3 Codex Spark"
		compatibilityModel["description"] = "GPT-5.3 Codex Spark"
		models[index] = compatibilityModel
	}
}

func displayNameForModel(model string) string {
	switch model {
	case "gpt-5-codex":
		return "GPT-5 Codex"
	case "gpt-5-codex-mini":
		return "GPT-5 Codex Mini"
	case "gpt-5.6-sol":
		return "GPT-5.6-Sol"
	case "gpt-5.6-terra":
		return "GPT-5.6-Terra"
	case "gpt-5.6-luna":
		return "GPT-5.6-Luna"
	case "gpt-5.5":
		return "GPT-5.5"
	case "gpt-5.4":
		return "GPT-5.4"
	case "gpt-5.4-mini":
		return "GPT-5.4 Mini"
	case "gpt-5.3-codex":
		return "GPT-5.3 Codex"
	case codexSparkModel:
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

func isHiddenCodexClientModel(model string) bool {
	switch model {
	case codexAutoReviewModel, "gpt-image-2", "grok-imagine-image", "grok-imagine-video", "grok-imagine-image-quality":
		return true
	default:
		return false
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

func isCodexLiveRequest(r *http.Request) bool {
	if r == nil || r.URL == nil {
		return false
	}
	path := strings.TrimRight(strings.TrimSpace(r.URL.Path), "/")
	return path == "/v1/live" ||
		strings.HasPrefix(path, "/v1/live/") ||
		path == "/v1/realtime" ||
		path == "/v1/realtime/calls" ||
		strings.HasPrefix(path, "/v1/realtime/calls/") ||
		strings.HasPrefix(path, "/v1/realtime/")
}

func readAndRestoreBody(r *http.Request) ([]byte, error) {
	if r == nil || r.Body == nil {
		return nil, nil
	}

	raw, err := io.ReadAll(r.Body)
	_ = r.Body.Close()
	if err != nil {
		r.Body = io.NopCloser(bytes.NewReader(raw))
		return raw, err
	}

	contentEncoding := strings.TrimSpace(r.Header.Get("Content-Encoding"))
	if contentEncoding == "" || strings.EqualFold(contentEncoding, "identity") {
		r.Body = io.NopCloser(bytes.NewReader(raw))
		return raw, nil
	}

	body, err := decodeRelayRequestBody(raw, contentEncoding)
	if err != nil {
		r.Body = io.NopCloser(bytes.NewReader(raw))
		return nil, err
	}

	// 请求体现在是普通 JSON，禁止后续处理器再次按 zstd 解压。
	r.Header.Del("Content-Encoding")
	r.Header.Del("Transfer-Encoding")
	r.TransferEncoding = nil

	r.Body = io.NopCloser(bytes.NewReader(body))
	r.ContentLength = int64(len(body))
	r.Header.Set(
		"Content-Length",
		strconv.FormatInt(r.ContentLength, 10),
	)

	return body, nil
}

func decodeRelayRequestBody(
	raw []byte,
	contentEncoding string,
) ([]byte, error) {
	encodings := strings.Split(contentEncoding, ",")
	body := raw

	// Content-Encoding 必须按编码应用顺序的反方向解码。
	for i := len(encodings) - 1; i >= 0; i-- {
		encoding := strings.ToLower(
			strings.TrimSpace(encodings[i]),
		)

		switch encoding {
		case "", "identity":
			continue

		case "zstd":
			decoder, err := zstd.NewReader(
				bytes.NewReader(body),
			)
			if err != nil {
				return nil, fmt.Errorf(
					"failed to create zstd request decoder: %w",
					err,
				)
			}

			decoded, readErr := io.ReadAll(decoder)
			decoder.Close()

			if readErr != nil {
				return nil, fmt.Errorf(
					"failed to decode zstd request body: %w",
					readErr,
				)
			}

			body = decoded

		default:
			return nil, fmt.Errorf(
				"unsupported request content encoding: %s",
				encoding,
			)
		}
	}

	return body, nil
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
	if _, _, status := resolveModelRouting(spec, model); status != "none" {
		// Keep the namespaced client model intact so the executor can route
		// before OAuth catalog canonicalization strips the namespace.
		return nil, model, nil
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
	if spec != nil && spec.ProviderGateway != nil {
		models := make([]string, 0, len(spec.ProviderGateway.UpstreamModels))
		for _, upstreamModel := range spec.ProviderGateway.UpstreamModels {
			clientModel := upstreamModel
			for _, alias := range m.ModelAliases {
				if strings.EqualFold(alias.SourceModel, upstreamModel) {
					clientModel = alias.Alias
					break
				}
			}
			models = append(models, clientModel)
		}
		return normalizeStringList(models)
	}
	models := applyModelFilters(m.ModelIDs, nil, m.ExcludedModels)
	if spec != nil && spec.ModelRouting != nil {
		for _, route := range spec.ModelRouting.Routes {
			if route.ProviderGateway == nil {
				continue
			}
			for _, upstreamModel := range route.ProviderGateway.UpstreamModels {
				models = append(models, route.Namespace+"/"+upstreamModel)
			}
		}
		models = normalizeStringList(models)
	}
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
	if spec != nil && spec.ProviderGateway != nil {
		if m != nil {
			if source := m.aliasToSource[strings.ToLower(withoutPrefix)]; source != "" {
				withoutPrefix = source
			}
		}
		return providerGatewayCanonicalModel(spec.ProviderGateway, withoutPrefix)
	}
	if m != nil {
		if source := m.aliasToSource[strings.ToLower(withoutPrefix)]; source != "" {
			return source
		}
	}
	return resolveSupportedModelAlias(m, withoutPrefix)
}

func providerGatewayCanonicalModel(gateway *providerGatewaySpec, model string) string {
	if gateway == nil {
		return strings.TrimSpace(model)
	}
	model = strings.TrimSpace(model)
	if len(gateway.UpstreamModels) == 0 && strings.TrimSpace(gateway.UpstreamModel) == "" {
		return model
	}
	for _, upstreamModel := range gateway.UpstreamModels {
		if strings.EqualFold(model, upstreamModel) {
			return upstreamModel
		}
	}
	return strings.TrimSpace(gateway.UpstreamModel)
}

func providerGatewayModelSupportsVision(gateway *providerGatewaySpec, model string) bool {
	if gateway == nil {
		return false
	}
	key := strings.ToLower(strings.TrimSpace(model))
	if key != "" && gateway.ModelCapabilities != nil {
		if capability, ok := gateway.ModelCapabilities[key]; ok {
			return capability.SupportsVision
		}
	}
	return gateway.SupportsVision
}

func providerGatewayModelCapabilityOverridesVision(gateway *providerGatewaySpec, model string) (bool, bool) {
	if gateway == nil {
		return false, false
	}
	key := strings.ToLower(strings.TrimSpace(model))
	if key == "" || gateway.ModelCapabilities == nil {
		return false, false
	}
	capability, ok := gateway.ModelCapabilities[key]
	if !ok {
		return false, false
	}
	return capability.SupportsVision, true
}

func providerGatewayVisionRoutingModel(gateway *providerGatewaySpec) string {
	if gateway == nil {
		return ""
	}
	model := strings.TrimSpace(gateway.VisionRoutingModel)
	if model != "" && len(gateway.UpstreamModels) > 0 {
		matched := ""
		for _, upstreamModel := range gateway.UpstreamModels {
			if strings.EqualFold(model, upstreamModel) {
				matched = upstreamModel
				break
			}
		}
		if matched == "" {
			return ""
		}
		model = matched
	}
	if model != "" && providerGatewayModelSupportsVision(gateway, model) {
		return model
	}
	if model != "" {
		return ""
	}
	visionModel := ""
	for rawModel, capability := range gateway.ModelCapabilities {
		if !capability.SupportsVision {
			continue
		}
		model = strings.TrimSpace(rawModel)
		if model == "" {
			continue
		}
		if len(gateway.UpstreamModels) > 0 {
			matched := ""
			for _, upstreamModel := range gateway.UpstreamModels {
				if strings.EqualFold(model, upstreamModel) {
					matched = upstreamModel
					break
				}
			}
			if matched == "" {
				continue
			}
			model = matched
		}
		if visionModel != "" && !strings.EqualFold(visionModel, model) {
			return ""
		}
		visionModel = model
	}
	if visionModel != "" && providerGatewayModelSupportsVision(gateway, visionModel) {
		return visionModel
	}
	return ""
}

func providerGatewayRequestHasVisionInput(body []byte) bool {
	if len(body) == 0 || !json.Valid(body) {
		return false
	}
	var payload any
	if err := json.Unmarshal(body, &payload); err != nil {
		return false
	}
	return providerGatewayValueHasVisionInput(payload)
}

func providerGatewayValueHasVisionInput(value any) bool {
	switch typed := value.(type) {
	case map[string]any:
		if typ, _ := typed["type"].(string); strings.EqualFold(strings.TrimSpace(typ), "input_image") || strings.EqualFold(strings.TrimSpace(typ), "image_url") {
			return true
		}
		for _, child := range typed {
			if providerGatewayValueHasVisionInput(child) {
				return true
			}
		}
	case []any:
		for _, child := range typed {
			if providerGatewayValueHasVisionInput(child) {
				return true
			}
		}
	}
	return false
}

const providerGatewayOmittedImageText = "[Image omitted because the current model does not support image input.]"

func omitProviderGatewayVisionInput(body []byte, sourceFormat sdktranslator.Format) ([]byte, int, error) {
	var payload any
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, 0, err
	}
	textType := "text"
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAIResponse) {
		textType = "input_text"
	}
	omitted, count := omitProviderGatewayVisionValue(payload, textType)
	if count == 0 {
		return body, 0, nil
	}
	normalized, err := json.Marshal(omitted)
	if err != nil {
		return nil, 0, err
	}
	return normalized, count, nil
}

func omitProviderGatewayVisionValue(value any, textType string) (any, int) {
	switch typed := value.(type) {
	case map[string]any:
		typ, _ := typed["type"].(string)
		if strings.EqualFold(strings.TrimSpace(typ), "input_image") || strings.EqualFold(strings.TrimSpace(typ), "image_url") {
			return map[string]any{
				"type": textType,
				"text": providerGatewayOmittedImageText,
			}, 1
		}
		count := 0
		for key, child := range typed {
			next, childCount := omitProviderGatewayVisionValue(child, textType)
			typed[key] = next
			count += childCount
		}
		return typed, count
	case []any:
		count := 0
		for index, child := range typed {
			next, childCount := omitProviderGatewayVisionValue(child, textType)
			typed[index] = next
			count += childCount
		}
		return typed, count
	default:
		return value, 0
	}
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
	if spec != nil && spec.ProviderGateway != nil {
		if len(spec.ProviderGateway.UpstreamModels) == 0 {
			return true
		}
		for _, upstreamModel := range spec.ProviderGateway.UpstreamModels {
			if strings.EqualFold(canonical, upstreamModel) {
				return true
			}
		}
		return false
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
	case strings.Contains(path, "/alpha/search"):
		// Responses Lite web.run uses a standalone search endpoint.
		return "text"
	case strings.Contains(path, "/chat/completions"),
		strings.Contains(path, "/responses"),
		strings.Contains(path, "/v1/messages"),
		strings.Contains(path, "/v1beta/models"),
		strings.Contains(path, "/api/chat"):
		return "text"
	default:
		return "other"
	}
}
