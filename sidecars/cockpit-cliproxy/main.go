package main

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"math/rand"
	"mime/multipart"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/signal"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/gin-gonic/gin"
	internallogging "github.com/router-for-me/CLIProxyAPI/v7/internal/logging"
	internalregistry "github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	responsesconverter "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/openai/openai/responses"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/util"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/watcher/synthesizer"
	sdkhandlers "github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	sdkopenai "github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers/openai"
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
	BoundOAuth          bool                 `json:"boundOAuth,omitempty"`
	AccountIDs          []string             `json:"accountIds"`
	ModelPrefix         string               `json:"modelPrefix,omitempty"`
	ResponsesWebsockets bool                 `json:"responsesWebsockets,omitempty"`
	AllowedModels       []string             `json:"allowedModels"`
	ExcludedModels      []string             `json:"excludedModels"`
	Enabled             bool                 `json:"enabled"`
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
	ID                   string            `json:"id"`
	Email                string            `json:"email"`
	AuthID               string            `json:"authId,omitempty"`
	AuthKind             string            `json:"authKind,omitempty"`
	PlanType             string            `json:"planType,omitempty"`
	AccessTokenOnly      bool              `json:"accessTokenOnly,omitempty"`
	ChatGPTAccountID     string            `json:"chatgptAccountId,omitempty"`
	UpstreamAPIKey       string            `json:"upstreamApiKey,omitempty"`
	PlanRank             *int              `json:"planRank,omitempty"`
	RemainingQuota       *int              `json:"remainingQuota,omitempty"`
	SubscriptionExpiryMS *int64            `json:"subscriptionExpiryMs,omitempty"`
	QuotaReserve         *quotaReserveSpec `json:"quotaReserve,omitempty"`
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
	AccountID string `json:"accountId"`
	Priority  int    `json:"priority"`
	Weight    int    `json:"weight"`
	IsBackup  bool   `json:"isBackup"`
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
	Success          bool         `json:"success"`
	Status           int          `json:"status,omitempty"`
	ErrorCategory    string       `json:"errorCategory,omitempty"`
	ErrorMessage     string       `json:"errorMessage,omitempty"`
	LatencyMS        int64        `json:"latencyMs,omitempty"`
	Usage            usageDetails `json:"usage"`
	RequestedAtMS    int64        `json:"requestedAtMs,omitempty"`
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
	AuthAvailable   *bool  `json:"authAvailable,omitempty"`
	NextRetryAtMS   int64  `json:"nextRetryAtMs,omitempty"`
	AuthStateReason string `json:"authStateReason,omitempty"`
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
			if gateway.BaseURL == "" || gateway.APIKey == "" {
				m.APIKeys[i].ProviderGateway = nil
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
				c.JSON(http.StatusOK, buildCodexClientModelsResponse(models, spec))
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

func buildCodexClientModelsResponse(models []string, spec *apiKeySpec) gin.H {
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
	response := gin.H(sdkopenai.CodexClientModelsResponseWithProviders(sourceModels, func(string) []string {
		if spec != nil && spec.ProviderGateway != nil {
			return []string{"provider-gateway"}
		}
		return []string{"codex"}
	}))
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

type cockpitSelector struct {
	manifest   *manifest
	emitter    *eventEmitter
	quota      *quotaReserveStateStore
	priorities *apiKeyPriorityStateStore
	tracker    *requestUsageTracker
	mu         sync.Mutex
	cursor     int
}

type recordingSelector struct {
	inner    coreauth.Selector
	manifest *manifest
	tracker  *requestUsageTracker
}

type imageRequestSelector struct {
	imageFallback coreauth.Selector
	fallback      coreauth.Selector
}

// modelExclusionSelector filters account-level model exclusions before wrappers
// such as session affinity can reuse a cached account binding.
type modelExclusionSelector struct {
	manifest *manifest
	fallback coreauth.Selector
}

func (s *imageRequestSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	if isImageRequestKind(requestKind) && s.imageFallback != nil {
		return s.imageFallback.Pick(ctx, provider, model, opts, auths)
	}
	if s.fallback == nil {
		return nil, fmt.Errorf("image request selector fallback is not initialized")
	}
	return s.fallback.Pick(ctx, provider, model, opts, auths)
}

func (s *imageRequestSelector) Stop() {
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *modelExclusionSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil {
		return nil, fmt.Errorf("model exclusion selector is not initialized")
	}
	if strings.TrimSpace(model) == "" {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}
	filtered := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		if authModelExcluded(s.manifest, auth, model) {
			continue
		}
		filtered = append(filtered, auth)
	}
	if len(filtered) == 0 {
		return nil, noAuthAvailableError(nil)
	}
	return s.fallback.Pick(ctx, provider, model, opts, filtered)
}

func (s *modelExclusionSelector) Stop() {
	if s == nil || s.fallback == nil {
		return
	}
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *recordingSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	auth, err := s.inner.Pick(ctx, provider, model, opts, auths)
	if err != nil || auth == nil || s.tracker == nil {
		return auth, err
	}
	s.tracker.recordSelectedAccount(internallogging.GetRequestID(ctx), accountForAuthInManifest(s.manifest, auth), auth.ID)
	return auth, nil
}

func (s *recordingSelector) Stop() {
	if stoppable, ok := s.inner.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

type quotaReserveSelector struct {
	manifest *manifest
	fallback coreauth.Selector
	quota    *quotaReserveStateStore
}

type backupAccountSelector struct {
	manifest *manifest
	fallback coreauth.Selector
}

func quotaReserveSnapshotsFromManifest(m *manifest) map[string]quotaReserveSnapshot {
	snapshots := make(map[string]quotaReserveSnapshot)
	if m == nil {
		return snapshots
	}
	for index := range m.Accounts {
		account := &m.Accounts[index]
		if account.QuotaReserve == nil || strings.TrimSpace(account.ID) == "" {
			continue
		}
		reserve := account.QuotaReserve
		snapshots[account.ID] = quotaReserveSnapshot{
			SnapshotUpdatedAtUnixSeconds: reserve.SnapshotUpdatedAtUnixSeconds,
			HourlyRemainingPercent:       reserve.HourlyRemainingPercent,
			WeeklyRemainingPercent:       reserve.WeeklyRemainingPercent,
			HourlyWindowPresent:          reserve.HourlyWindowPresent,
			WeeklyWindowPresent:          reserve.WeeklyWindowPresent,
		}
	}
	return snapshots
}

func newQuotaReserveStateStore(path string, m *manifest) *quotaReserveStateStore {
	store := &quotaReserveStateStore{path: strings.TrimSpace(path)}
	store.snapshot.Store(quotaReserveSnapshotsFromManifest(m))
	return store
}

func (s *quotaReserveStateStore) load() error {
	if s == nil || s.path == "" {
		return nil
	}
	content, err := os.ReadFile(s.path)
	if err != nil {
		return err
	}
	hash := sha256.Sum256(content)
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.hasHash && hash == s.lastHash {
		return nil
	}
	var state quotaReserveStateFile
	if err := json.Unmarshal(content, &state); err != nil {
		return err
	}
	if state.Accounts == nil {
		state.Accounts = make(map[string]quotaReserveSnapshot)
	}
	normalized := make(map[string]quotaReserveSnapshot, len(state.Accounts))
	for accountID, snapshot := range state.Accounts {
		accountID = strings.TrimSpace(accountID)
		if accountID != "" {
			normalized[accountID] = snapshot
		}
	}
	s.snapshot.Store(normalized)
	s.lastHash = hash
	s.hasHash = true
	return nil
}

func (s *quotaReserveStateStore) start(ctx context.Context, emitter *eventEmitter) {
	if s == nil || s.path == "" {
		return
	}
	go func() {
		ticker := time.NewTicker(time.Second)
		defer ticker.Stop()
		lastError := ""
		for {
			if err := s.load(); err != nil {
				message := err.Error()
				if message != lastError && emitter != nil {
					emitter.emit(map[string]any{
						"type":    "quota_reserve_state_error",
						"message": message,
					})
				}
				lastError = message
			} else {
				lastError = ""
			}
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
			}
		}
	}()
}

func (s *quotaReserveStateStore) forAccount(accountID string) *quotaReserveSnapshot {
	if s == nil {
		return nil
	}
	loaded := s.snapshot.Load()
	snapshots, ok := loaded.(map[string]quotaReserveSnapshot)
	if !ok {
		return nil
	}
	snapshot, ok := snapshots[strings.TrimSpace(accountID)]
	if !ok {
		return nil
	}
	return &snapshot
}

func (s *quotaReserveSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil {
		return nil, fmt.Errorf("quota reserve selector is not initialized")
	}
	if s.manifest == nil {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}

	now := time.Now()
	var filtered []*coreauth.Auth
	quotaReserveReasons := make([]string, 0)
	availableAfterReserve := 0
	for index, auth := range auths {
		if !authAvailable(auth, model, now) {
			if filtered != nil {
				filtered = append(filtered, auth)
			}
			continue
		}
		reason := quotaReserveBlockReasonWithState(
			accountForAuthInManifest(s.manifest, auth),
			s.quota,
			now,
		)
		if reason == "" {
			availableAfterReserve++
			if filtered != nil {
				filtered = append(filtered, auth)
			}
			continue
		}
		if filtered == nil {
			filtered = append(make([]*coreauth.Auth, 0, len(auths)-1), auths[:index]...)
		}
		quotaReserveReasons = append(quotaReserveReasons, reason)
	}
	if filtered == nil {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}
	if availableAfterReserve == 0 {
		return nil, noAuthAvailableError(quotaReserveReasons)
	}
	return s.fallback.Pick(ctx, provider, model, opts, filtered)
}

func (s *quotaReserveSelector) Stop() {
	if s == nil || s.fallback == nil {
		return
	}
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *backupAccountSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.fallback == nil {
		return nil, fmt.Errorf("backup account selector is not initialized")
	}
	if s.manifest == nil || !strings.EqualFold(strings.TrimSpace(s.manifest.RoutingStrategy), "custom") {
		return s.fallback.Pick(ctx, provider, model, opts, auths)
	}

	now := time.Now()
	regular := make([]*coreauth.Auth, 0, len(auths))
	backup := make([]*coreauth.Auth, 0)
	regularAvailable := false
	for _, auth := range auths {
		if s.isBackupAuth(auth) {
			backup = append(backup, auth)
			continue
		}
		regular = append(regular, auth)
		if authAvailable(auth, model, now) {
			regularAvailable = true
		}
	}

	if regularAvailable || len(backup) == 0 {
		return s.fallback.Pick(ctx, provider, model, opts, regular)
	}
	return s.fallback.Pick(ctx, provider, model, opts, backup)
}

func (s *backupAccountSelector) isBackupAuth(auth *coreauth.Auth) bool {
	account := accountForAuthInManifest(s.manifest, auth)
	if account == nil {
		return false
	}
	for _, rule := range s.manifest.CustomRoutingRules {
		if rule.AccountID == account.ID {
			return rule.IsBackup
		}
	}
	return false
}

func (s *backupAccountSelector) Stop() {
	if s == nil || s.fallback == nil {
		return
	}
	if stoppable, ok := s.fallback.(coreauth.StoppableSelector); ok {
		stoppable.Stop()
	}
}

func (s *cockpitSelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	_ = provider
	_ = opts
	auths = s.filterAuthsForAPIKeyScope(ctx, auths)
	now := time.Now()
	available := make([]*coreauth.Auth, 0, len(auths))
	quotaReserveReasons := make([]string, 0)
	for _, auth := range auths {
		if !authAvailable(auth, model, now) {
			continue
		}
		if reason := quotaReserveBlockReasonWithState(s.accountForAuth(auth), s.quota, now); reason != "" {
			quotaReserveReasons = append(quotaReserveReasons, reason)
			continue
		}
		available = append(available, auth)
	}
	if len(available) == 0 {
		return nil, noAuthAvailableError(quotaReserveReasons)
	}

	s.mu.Lock()
	start := s.cursor
	s.cursor++
	s.mu.Unlock()

	requestKind, _ := ctx.Value(requestKindContextKey).(string)
	ordered := s.orderAuths(available, start)
	if isImageRequestKind(requestKind) {
		ordered = s.orderImageAuths(available, start)
	} else {
		ordered = s.prioritizeAuthsForAPIKey(ctx, ordered)
	}
	if len(ordered) == 0 {
		return nil, noAuthAvailableError(quotaReserveReasons)
	}
	if isImageRequestKind(requestKind) && s.tracker != nil {
		requestID := internallogging.GetRequestID(ctx)
		for {
			changed := s.tracker.imageJobChangeSignal()
			for _, candidate := range s.orderImageAuths(available, start) {
				if s.tracker.tryReserveImageJob(requestID, candidate.ID, s.maxConcurrentImageRequests()) {
					s.emitAuthSelected(ctx, candidate, provider, model, len(auths), len(available))
					return candidate, nil
				}
			}
			select {
			case <-ctx.Done():
				return nil, ctx.Err()
			case <-changed:
			}
		}
	}
	selected := ordered[0]
	s.emitAuthSelected(ctx, selected, provider, model, len(auths), len(available))
	return selected, nil
}

func (s *cockpitSelector) prioritizeAuthsForAPIKey(ctx context.Context, auths []*coreauth.Auth) []*coreauth.Auth {
	if s == nil || ctx == nil || len(auths) <= 1 || s.priorities == nil {
		return auths
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec == nil {
		return auths
	}
	priorityAccountIDs := s.priorities.priorityAccountIDs(spec.ID)
	if len(priorityAccountIDs) == 0 {
		return auths
	}

	ordered := make([]*coreauth.Auth, 0, len(auths))
	selected := make(map[*coreauth.Auth]struct{}, len(priorityAccountIDs))
	for _, priorityAccountID := range priorityAccountIDs {
		for _, auth := range auths {
			account := s.accountForAuth(auth)
			if account == nil || account.ID != priorityAccountID {
				continue
			}
			if _, alreadySelected := selected[auth]; alreadySelected {
				break
			}
			ordered = append(ordered, auth)
			selected[auth] = struct{}{}
			break
		}
	}
	if len(ordered) == 0 {
		return auths
	}
	for _, auth := range auths {
		if _, alreadySelected := selected[auth]; !alreadySelected {
			ordered = append(ordered, auth)
		}
	}
	return ordered
}

func (s *cockpitSelector) filterAuthsForAPIKeyScope(ctx context.Context, auths []*coreauth.Auth) []*coreauth.Auth {
	if s == nil || s.manifest == nil || ctx == nil {
		return auths
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec == nil || len(spec.AccountIDs) == 0 {
		return auths
	}

	allowedAccountIDs := make(map[string]struct{}, len(spec.AccountIDs))
	for _, accountID := range spec.AccountIDs {
		if accountID = strings.TrimSpace(accountID); accountID != "" {
			allowedAccountIDs[accountID] = struct{}{}
		}
	}
	if len(allowedAccountIDs) == 0 {
		return nil
	}

	scoped := make([]*coreauth.Auth, 0, len(auths))
	for _, auth := range auths {
		account := s.accountForAuth(auth)
		if account == nil {
			continue
		}
		if _, allowed := allowedAccountIDs[account.ID]; allowed {
			scoped = append(scoped, auth)
		}
	}
	return scoped
}

func (s *cockpitSelector) maxConcurrentImageRequests() int {
	if s == nil || s.manifest == nil || s.manifest.MaxConcurrentImageRequests < 1 {
		return 1
	}
	return s.manifest.MaxConcurrentImageRequests
}

func isImageRequestKind(requestKind string) bool {
	switch strings.TrimSpace(requestKind) {
	case "image_generation", "image_edit":
		return true
	default:
		return false
	}
}

func (s *cockpitSelector) orderImageAuths(auths []*coreauth.Auth, start int) []*coreauth.Auth {
	if len(auths) <= 1 || s == nil || s.tracker == nil {
		return s.orderAuths(auths, start)
	}
	out := append([]*coreauth.Auth(nil), auths...)
	sort.SliceStable(out, func(i, j int) bool {
		leftJobs := s.tracker.imageInFlightCount(out[i].ID)
		rightJobs := s.tracker.imageInFlightCount(out[j].ID)
		if leftJobs != rightJobs {
			return leftJobs < rightJobs
		}
		return s.rotatedIndex(s.accountForAuth(out[i]), start) < s.rotatedIndex(s.accountForAuth(out[j]), start)
	})
	return out
}

func quotaReserveBlockReason(account *accountSpec, now time.Time) string {
	return quotaReserveBlockReasonWithSnapshot(account, quotaReserveSnapshotFromSpec(account), now)
}

func quotaReserveBlockReasonWithState(account *accountSpec, state *quotaReserveStateStore, now time.Time) string {
	var snapshot *quotaReserveSnapshot
	if account != nil && state != nil {
		snapshot = state.forAccount(account.ID)
	}
	if snapshot == nil {
		snapshot = quotaReserveSnapshotFromSpec(account)
	}
	return quotaReserveBlockReasonWithSnapshot(account, snapshot, now)
}

func quotaReserveSnapshotFromSpec(account *accountSpec) *quotaReserveSnapshot {
	if account == nil || account.QuotaReserve == nil {
		return nil
	}
	reserve := account.QuotaReserve
	return &quotaReserveSnapshot{
		SnapshotUpdatedAtUnixSeconds: reserve.SnapshotUpdatedAtUnixSeconds,
		HourlyRemainingPercent:       reserve.HourlyRemainingPercent,
		WeeklyRemainingPercent:       reserve.WeeklyRemainingPercent,
		HourlyWindowPresent:          reserve.HourlyWindowPresent,
		WeeklyWindowPresent:          reserve.WeeklyWindowPresent,
	}
}

func quotaReserveBlockReasonWithSnapshot(account *accountSpec, snapshot *quotaReserveSnapshot, now time.Time) string {
	if account == nil || account.QuotaReserve == nil {
		return ""
	}

	reserve := account.QuotaReserve
	if snapshot == nil {
		return quotaReserveAccountReason(account, []string{"quota snapshot unknown"})
	}
	if reason := quotaReserveSnapshotBlockReason(snapshot.SnapshotUpdatedAtUnixSeconds, now); reason != "" {
		return quotaReserveAccountReason(account, []string{reason})
	}

	reasons := make([]string, 0, 2)
	if reason := quotaReserveWindowBlockReason(
		"5h",
		reserve.HourlyThresholdPercent,
		snapshot.HourlyRemainingPercent,
		snapshot.HourlyWindowPresent,
	); reason != "" {
		reasons = append(reasons, reason)
	}
	if reason := quotaReserveWindowBlockReason(
		"weekly",
		reserve.WeeklyThresholdPercent,
		snapshot.WeeklyRemainingPercent,
		snapshot.WeeklyWindowPresent,
	); reason != "" {
		reasons = append(reasons, reason)
	}
	if len(reasons) == 0 {
		return ""
	}
	return quotaReserveAccountReason(account, reasons)
}

func quotaReserveSnapshotBlockReason(updatedAt *int64, now time.Time) string {
	if updatedAt == nil {
		return "quota snapshot timestamp unknown"
	}

	nowUnix := now.Unix()
	if *updatedAt <= 0 || *updatedAt > nowUnix {
		return "quota snapshot timestamp invalid"
	}
	if nowUnix-*updatedAt > int64(quotaReserveMaxSnapshotAge/time.Second) {
		return "quota snapshot stale"
	}
	return ""
}

func quotaReserveAccountReason(account *accountSpec, reasons []string) string {
	accountLabel := strings.TrimSpace(account.Email)
	if accountLabel == "" {
		accountLabel = strings.TrimSpace(account.ID)
	}
	if accountLabel == "" {
		accountLabel = "unknown account"
	}
	return fmt.Sprintf("%s (%s)", accountLabel, strings.Join(reasons, ", "))
}

func quotaReserveWindowBlockReason(window string, threshold, remaining *int, present *bool) string {
	if present != nil && !*present {
		return ""
	}
	if threshold == nil || *threshold < 1 || *threshold > 100 {
		return fmt.Sprintf("%s reserve threshold unknown", window)
	}
	if remaining == nil || *remaining < 0 || *remaining > 100 {
		return fmt.Sprintf("%s remaining quota unknown; reserve %d%%", window, *threshold)
	}
	if *remaining <= *threshold {
		return fmt.Sprintf("%s remaining %d%% <= reserve %d%%", window, *remaining, *threshold)
	}
	return ""
}

func noAuthAvailableError(quotaReserveReasons []string) error {
	if len(quotaReserveReasons) == 0 {
		return fmt.Errorf("no auth available")
	}

	const maxReasons = 3
	reasons := quotaReserveReasons
	if len(reasons) > maxReasons {
		reasons = reasons[:maxReasons]
	}
	detail := strings.Join(reasons, "; ")
	if omitted := len(quotaReserveReasons) - len(reasons); omitted > 0 {
		detail = fmt.Sprintf("%s; and %d more", detail, omitted)
	}
	return fmt.Errorf(
		"no auth available: bound OAuth quota reserve blocked %d auth(s): %s",
		len(quotaReserveReasons),
		detail,
	)
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
	if strategy == "random" {
		out := append([]*coreauth.Auth(nil), auths...)
		rand.Shuffle(len(out), func(i, j int) {
			out[i], out[j] = out[j], out[i]
		})
		return out
	}
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
	if s == nil {
		return nil
	}
	return accountForAuthInManifest(s.manifest, auth)
}

func accountForAuthInManifest(m *manifest, auth *coreauth.Auth) *accountSpec {
	if m == nil || auth == nil {
		return nil
	}
	if auth.ID != "" {
		if account := m.accountByAuthID[strings.ToLower(auth.ID)]; account != nil {
			return account
		}
		base := strings.TrimSuffix(filepath.Base(auth.ID), filepath.Ext(auth.ID))
		if account := m.accountByID[base]; account != nil {
			return account
		}
	}
	if auth.Attributes != nil {
		if key := strings.TrimSpace(auth.Attributes["api_key"]); key != "" {
			return m.accountByAPIKey[key]
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
		Type:             "usage",
		RequestID:        internallogging.GetRequestID(ctx),
		Provider:         record.Provider,
		Model:            model,
		Alias:            record.Alias,
		AccountID:        stringFromAccount(account, "id"),
		AccountEmail:     stringFromAccount(account, "email"),
		AuthID:           record.AuthID,
		APIKeyID:         stringFromAPIKey(spec, "id"),
		APIKeyLabel:      stringFromAPIKey(spec, "label"),
		ClientInstanceID: clientInstanceIDFromContext(ctx),
		RequestKind:      requestKind,
		ServiceTier:      normalizedUsageServiceTier(record.ServiceTier),
		Success:          success,
		Status:           status,
		ErrorCategory:    errorCategory(status, record.Fail.Body, success),
		ErrorMessage:     strings.TrimSpace(record.Fail.Body),
		LatencyMS:        record.Latency.Milliseconds(),
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
	var authAvailable *bool
	if result.AuthStateKnown {
		value := result.AuthAvailable
		authAvailable = &value
	}
	nextRetryAtMS := int64(0)
	if !result.NextRetryAt.IsZero() {
		nextRetryAtMS = result.NextRetryAt.UnixMilli()
	}
	h.emitter.emit(requestDiagnosticPayload{
		Type:            "auth_result",
		RequestID:       internallogging.GetRequestID(ctx),
		Provider:        result.Provider,
		Model:           model,
		AuthID:          result.AuthID,
		AccountID:       stringFromAccount(account, "id"),
		AccountEmail:    stringFromAccount(account, "email"),
		APIKeyID:        stringFromAPIKey(spec, "id"),
		APIKeyLabel:     stringFromAPIKey(spec, "label"),
		RequestKind:     requestKind,
		Success:         &success,
		HTTPStatus:      status,
		ErrorCode:       errorCode,
		ErrorMessage:    errorMessage,
		Retryable:       retryablePtr,
		RetryAfterMS:    retryAfterMS,
		AuthAvailable:   authAvailable,
		NextRetryAtMS:   nextRetryAtMS,
		AuthStateReason: result.AuthStateReason,
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

func buildCoreAuthSelector(cfg *config.Config, selector coreauth.Selector, m *manifest, quota *quotaReserveStateStore) coreauth.Selector {
	if selector == nil {
		selector = &coreauth.RoundRobinSelector{}
	}
	if cfg != nil && cfg.Routing.SessionAffinity {
		imageFallback := selector
		ttl := time.Hour
		if parsed, err := time.ParseDuration(strings.TrimSpace(cfg.Routing.SessionAffinityTTL)); err == nil && parsed > 0 {
			ttl = parsed
		}
		// Session affinity + per-client-key namespace, with image requests bypassing affinity.
		selector = coreauth.NewSessionAffinitySelectorWithConfig(coreauth.SessionAffinityConfig{
			Fallback: selector,
			TTL:      ttl,
		})
		selector = &cockpitSessionAffinitySelector{inner: selector}
		selector = &imageRequestSelector{
			imageFallback: imageFallback,
			fallback:      selector,
		}
	}
	if m != nil {
		selector = &backupAccountSelector{manifest: m, fallback: selector}
		selector = &quotaReserveSelector{manifest: m, fallback: selector, quota: quota}
		selector = &modelExclusionSelector{manifest: m, fallback: selector}
	}
	return selector
}

func buildCoreAuthManager(cfg *config.Config, selector coreauth.Selector, hook coreauth.Hook, m *manifest, quota *quotaReserveStateStore, tracker *requestUsageTracker) *coreauth.Manager {
	tokenStore := sdkauth.GetTokenStore()
	if dirSetter, ok := tokenStore.(interface{ SetBaseDir(string) }); ok && cfg != nil {
		dirSetter.SetBaseDir(cfg.AuthDir)
	}
	selector = buildCoreAuthSelector(cfg, selector, m, quota)
	if tracker != nil {
		selector = &recordingSelector{inner: selector, manifest: m, tracker: tracker}
	}
	return coreauth.NewManager(tokenStore, selector, hook)
}

type cockpitSessionAffinitySelector struct {
	inner coreauth.Selector
}

func (s *cockpitSessionAffinitySelector) Pick(ctx context.Context, provider, model string, opts cliproxyexecutor.Options, auths []*coreauth.Auth) (*coreauth.Auth, error) {
	if s == nil || s.inner == nil {
		return nil, errors.New("session affinity selector is unavailable")
	}
	if spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec); spec != nil && strings.TrimSpace(spec.ID) != "" {
		metadata := make(map[string]any, len(opts.Metadata)+1)
		for key, value := range opts.Metadata {
			metadata[key] = value
		}
		metadata[cliproxyexecutor.SessionAffinityNamespaceMetadataKey] = spec.ID
		opts.Metadata = metadata
	}
	return s.inner.Pick(ctx, provider, model, opts, auths)
}

type sidecarRuntime struct {
	manager *coreauth.Manager
	service *cliproxy.Service
	cancel  context.CancelFunc
	done    chan error
}

// CodexAlphaSearch selects a Codex OAuth credential and forwards the standalone
// search payload to the ChatGPT Codex alpha search backend.
func (r *sidecarRuntime) CodexAlphaSearch(ctx context.Context, model string, body []byte, headers http.Header) (int, http.Header, []byte, error) {
	if r == nil || r.manager == nil {
		return 0, nil, nil, errors.New("Codex auth manager is unavailable")
	}
	upstreamBody := sanitizeCodexAlphaSearchBody(body)
	selectionHeaders := http.Header{}
	if headers != nil {
		selectionHeaders = headers.Clone()
	}
	opts := cliproxyexecutor.Options{
		Headers:         selectionHeaders,
		OriginalRequest: body,
		Metadata: map[string]any{
			cliproxyexecutor.RequestedModelMetadataKey: model,
			cliproxyexecutor.RequestPathMetadataKey:    codexAlphaSearchPath,
		},
	}
	selected, err := r.manager.SelectAuthByKind(ctx, "codex", model, coreauth.AuthKindOAuth, opts)
	if err != nil {
		return 0, nil, nil, err
	}
	if selected == nil {
		return 0, nil, nil, errors.New("no Codex OAuth account available for alpha search")
	}

	upstreamHeaders := buildCodexAlphaSearchHeaders(selectionHeaders, selected)
	upstreamURL := resolveCodexAlphaSearchURL(selected)
	req, err := r.manager.NewHttpRequest(ctx, selected, http.MethodPost, upstreamURL, upstreamBody, upstreamHeaders)
	if err != nil {
		return 0, nil, nil, err
	}
	// Ensure ChatGPT account binding for OAuth (PrepareRequest only injects Bearer).
	if accountID := codexAuthChatGPTAccountID(selected); accountID != "" {
		req.Header.Set("Chatgpt-Account-Id", accountID)
	}

	resp, err := r.manager.HttpRequest(ctx, selected, req)
	if err != nil {
		return 0, nil, nil, err
	}
	defer func() {
		_ = resp.Body.Close()
	}()
	payload, err := io.ReadAll(io.LimitReader(resp.Body, maxCodexAlphaSearchResponseBytes))
	if err != nil {
		return 0, nil, nil, fmt.Errorf("failed to read Codex search response: %w", err)
	}
	return resp.StatusCode, resp.Header.Clone(), payload, nil
}

func sanitizeCodexAlphaSearchBody(body []byte) []byte {
	var payload map[string]json.RawMessage
	if err := json.Unmarshal(body, &payload); err != nil || payload == nil {
		return body
	}
	removed := false
	for _, field := range []string{"prompt_cache_key", "prompt_cache_retention"} {
		if _, exists := payload[field]; exists {
			delete(payload, field)
			removed = true
		}
	}
	if !removed {
		return body
	}
	sanitized, err := json.Marshal(payload)
	if err != nil {
		return body
	}
	return sanitized
}

func resolveCodexAlphaSearchURL(auth *coreauth.Auth) string {
	baseURL := ""
	if auth != nil && auth.Attributes != nil {
		baseURL = strings.TrimSpace(auth.Attributes["base_url"])
	}
	if baseURL == "" {
		return defaultCodexAlphaSearchURL
	}
	baseURL = strings.TrimRight(baseURL, "/")
	switch {
	case strings.HasSuffix(strings.ToLower(baseURL), "/alpha/search"):
		return baseURL
	case strings.HasSuffix(strings.ToLower(baseURL), "/codex"):
		return baseURL + "/alpha/search"
	case strings.HasSuffix(strings.ToLower(baseURL), "/backend-api"):
		return baseURL + "/codex/alpha/search"
	default:
		return baseURL + "/alpha/search"
	}
}

func codexAuthChatGPTAccountID(auth *coreauth.Auth) string {
	if auth == nil {
		return ""
	}
	if auth.Metadata != nil {
		if accountID, ok := auth.Metadata["account_id"].(string); ok {
			if accountID = strings.TrimSpace(accountID); accountID != "" {
				return accountID
			}
		}
		if accountID, ok := auth.Metadata["chatgpt_account_id"].(string); ok {
			if accountID = strings.TrimSpace(accountID); accountID != "" {
				return accountID
			}
		}
	}
	if auth.Attributes != nil {
		if accountID := strings.TrimSpace(auth.Attributes["chatgpt_account_id"]); accountID != "" {
			return accountID
		}
	}
	return ""
}

func buildCodexAlphaSearchHeaders(src http.Header, auth *coreauth.Auth) http.Header {
	headers := make(http.Header)
	headers.Set("Content-Type", "application/json")
	headers.Set("Accept", "application/json")
	headers.Set("Originator", "codex_cli_rs")
	for _, name := range []string{
		"Version",
		"User-Agent",
		"Session_id",
		"X-Session-ID",
		"X-Client-Request-Id",
		"X-Openai-Actor-Authorization",
		"x-openai-actor-authorization",
	} {
		if src == nil {
			continue
		}
		if value := strings.TrimSpace(src.Get(name)); value != "" {
			headers.Set(name, value)
		}
	}
	// Preserve agtools diagnostic headers used by Cockpit.
	if src != nil {
		for key, values := range src {
			trimmed := strings.TrimSpace(key)
			if trimmed == "" || !strings.HasPrefix(strings.ToLower(trimmed), "x-agtools-") {
				continue
			}
			canonical := http.CanonicalHeaderKey(trimmed)
			headers.Del(canonical)
			for _, value := range values {
				value = strings.TrimSpace(value)
				if value != "" {
					headers.Add(canonical, value)
				}
			}
		}
	}
	if accountID := codexAuthChatGPTAccountID(auth); accountID != "" {
		headers.Set("Chatgpt-Account-Id", accountID)
	}
	return headers
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
	if err := registerManifestCodexTokenAuths(runtimeCtx, service, cfg, m, manager); err != nil {
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

func registerManifestCodexTokenAuths(
	ctx context.Context,
	service *cliproxy.Service,
	cfg *config.Config,
	m *manifest,
	manager *coreauth.Manager,
) error {
	if service == nil || cfg == nil || m == nil {
		return nil
	}
	for i := range m.Accounts {
		account := &m.Accounts[i]
		authID := strings.TrimSpace(account.AuthID)
		if authID == "" || manifestAccountAuthKind(account) == "api_key" {
			continue
		}
		path := authID
		if !filepath.IsAbs(path) {
			path = filepath.Join(cfg.AuthDir, path)
		}
		auth, err := readManifestCodexTokenAuth(account, cfg.AuthDir, path)
		if err != nil {
			return err
		}
		registered, err := service.UpsertRuntimeAuth(coreauth.WithSkipPersist(ctx), auth)
		if err != nil {
			return fmt.Errorf("register codex token auth %s: %w", auth.ID, err)
		}
		linkManifestAccountForAuth(m, registered)
		registerManifestModelsForAuth(manager, m, registered)
	}
	return nil
}

func readManifestCodexTokenAuth(account *accountSpec, authDir, path string) (*coreauth.Auth, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read codex token auth file %s: %w", path, err)
	}
	metadata := make(map[string]any)
	if err = json.Unmarshal(data, &metadata); err != nil {
		return nil, fmt.Errorf("parse codex token auth file %s: %w", path, err)
	}
	provider := strings.TrimSpace(metadataString(metadata, "type"))
	if provider == "" {
		provider = "codex"
	}
	if !strings.EqualFold(provider, "codex") {
		return nil, fmt.Errorf("codex token auth file %s has unsupported provider %q", path, provider)
	}
	accessToken := firstMetadataString(
		metadata,
		"personal_access_token",
		"at_token",
		"access_token",
	)
	authMode := firstMetadataString(metadata, "auth_mode", "openai_auth_mode")
	isAgentIdentity := strings.EqualFold(authMode, "agentIdentity") || manifestAccountAuthKind(account) == "agent_identity"
	if accessToken == "" && !isAgentIdentity {
		return nil, fmt.Errorf("codex token auth file %s is missing access_token", path)
	}
	if isAgentIdentity {
		if firstMetadataString(metadata, "agent_runtime_id", "agentRuntimeId") == "" ||
			firstMetadataString(metadata, "agent_private_key", "agentPrivateKey") == "" {
			return nil, fmt.Errorf("codex Agent Identity auth file %s is missing runtime or private key", path)
		}
		metadata["auth_mode"] = "agentIdentity"
		metadata["openai_auth_mode"] = "agentIdentity"
	} else {
		metadata["access_token"] = accessToken
		if strings.TrimSpace(metadataString(metadata, "token_type")) == "" {
			metadata["token_type"] = "Bearer"
		}
	}
	if account != nil &&
		(account.AccessTokenOnly || manifestAccountAuthKind(account) == "access_token") {
		if strings.TrimSpace(metadataString(metadata, "auth_mode")) == "" {
			metadata["auth_mode"] = "personal_access_token"
		}
		if strings.TrimSpace(metadataString(metadata, "openai_auth_mode")) == "" {
			metadata["openai_auth_mode"] = "personal_access_token"
		}
	}

	info, err := os.Stat(path)
	if err != nil {
		return nil, fmt.Errorf("stat codex token auth file %s: %w", path, err)
	}
	id := manifestAuthFileID(authDir, path)
	label := ""
	if account != nil {
		label = strings.TrimSpace(account.Email)
	}
	if label == "" {
		label = firstMetadataString(metadata, "email", "label")
	}
	disabled, _ := metadata["disabled"].(bool)
	status := coreauth.StatusActive
	if disabled {
		status = coreauth.StatusDisabled
	}
	runtimeAuthKind := manifestAccountAuthKind(account)
	if isAgentIdentity {
		runtimeAuthKind = coreauth.AuthKindOAuth
	}
	auth := &coreauth.Auth{
		ID:       id,
		Provider: "codex",
		FileName: id,
		Label:    label,
		Status:   status,
		Disabled: disabled,
		Attributes: map[string]string{
			"path":       path,
			"auth_kind":  runtimeAuthKind,
			"websockets": "true",
		},
		Metadata:        metadata,
		CreatedAt:       info.ModTime(),
		UpdatedAt:       info.ModTime(),
		LastRefreshedAt: time.Time{},
	}
	if account != nil {
		auth.Attributes["account_id"] = strings.TrimSpace(account.ID)
		if strings.TrimSpace(account.Email) != "" {
			auth.Attributes["email"] = strings.TrimSpace(account.Email)
		}
		if strings.TrimSpace(account.ChatGPTAccountID) != "" {
			auth.Attributes["chatgpt_account_id"] = strings.TrimSpace(account.ChatGPTAccountID)
		}
	}
	if email := firstMetadataString(metadata, "email"); email != "" {
		auth.Attributes["email"] = email
	}
	if proxyURL := firstMetadataString(metadata, "proxy_url", "proxy-url"); proxyURL != "" {
		auth.ProxyURL = proxyURL
	}
	coreauth.ApplyCustomHeadersFromMetadata(auth)
	return auth, nil
}

func manifestAccountAuthKind(account *accountSpec) string {
	if account == nil {
		return ""
	}
	if kind := strings.ToLower(strings.TrimSpace(account.AuthKind)); kind != "" {
		switch kind {
		case "api-key", "apikey", "api key":
			return "api_key"
		case "access-token", "accesstoken", "access token",
			"personal_access_token", "pat", "at":
			return "access_token"
		default:
			return kind
		}
	}
	if strings.TrimSpace(account.UpstreamAPIKey) != "" {
		return "api_key"
	}
	if account.AccessTokenOnly {
		return "access_token"
	}
	return "oauth"
}

func manifestAuthFileID(authDir, path string) string {
	id := path
	if strings.TrimSpace(authDir) != "" {
		if rel, err := filepath.Rel(authDir, path); err == nil && strings.TrimSpace(rel) != "" {
			id = rel
		}
	}
	if runtime.GOOS == "windows" {
		id = strings.ToLower(id)
	}
	return id
}

func metadataString(metadata map[string]any, key string) string {
	if metadata == nil {
		return ""
	}
	if raw, ok := metadata[key].(string); ok {
		return strings.TrimSpace(raw)
	}
	return ""
}

func firstMetadataString(metadata map[string]any, keys ...string) string {
	for _, key := range keys {
		if value := metadataString(metadata, key); value != "" {
			return value
		}
	}
	return ""
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
	authID := strings.ToLower(strings.TrimSpace(auth.ID))
	if _, exists := m.accountByAuthID[authID]; exists {
		return
	}
	if account := findManifestAccountForAuth(m, auth); account != nil {
		m.accountByAuthID[authID] = account
		if base := strings.ToLower(filepath.Base(strings.TrimSpace(auth.ID))); base != "" && base != authID {
			m.accountByAuthID[base] = account
		}
		return
	}
}

func findManifestAccountForAuth(m *manifest, auth *coreauth.Auth) *accountSpec {
	if m == nil || auth == nil {
		return nil
	}
	for _, candidate := range []string{
		strings.TrimSpace(auth.ID),
		filepath.Base(strings.TrimSpace(auth.ID)),
		strings.TrimSpace(auth.FileName),
		filepath.Base(strings.TrimSpace(auth.FileName)),
	} {
		if candidate == "." || candidate == "" {
			continue
		}
		if account := m.accountByAuthID[strings.ToLower(candidate)]; account != nil {
			return account
		}
	}
	if auth.Attributes != nil {
		if path := strings.TrimSpace(auth.Attributes["path"]); path != "" {
			if account := m.accountByAuthID[strings.ToLower(path)]; account != nil {
				return account
			}
			if account := m.accountByAuthID[strings.ToLower(filepath.Base(path))]; account != nil {
				return account
			}
		}
		if key := strings.TrimSpace(auth.Attributes["api_key"]); key != "" {
			if account := m.accountByAPIKey[key]; account != nil {
				return account
			}
		}
		if accountID := strings.TrimSpace(auth.Attributes["account_id"]); accountID != "" {
			if account := m.accountByID[accountID]; account != nil {
				return account
			}
		}
		if chatGPTID := strings.TrimSpace(auth.Attributes["chatgpt_account_id"]); chatGPTID != "" {
			if account := m.accountByChatGPT[strings.ToLower(chatGPTID)]; account != nil {
				return account
			}
		}
		if email := strings.TrimSpace(auth.Attributes["email"]); email != "" {
			if account := m.accountByEmail[strings.ToLower(email)]; account != nil {
				return account
			}
		}
	}
	if auth.Metadata != nil {
		for _, key := range []string{"account_id", "chatgpt_account_id"} {
			if value := metadataString(auth.Metadata, key); value != "" {
				if account := m.accountByChatGPT[strings.ToLower(value)]; account != nil {
					return account
				}
				if account := m.accountByID[value]; account != nil {
					return account
				}
			}
		}
		if email := metadataString(auth.Metadata, "email"); email != "" {
			if account := m.accountByEmail[strings.ToLower(email)]; account != nil {
				return account
			}
		}
	}
	return nil
}

func registerManifestModelsForAuth(manager *coreauth.Manager, m *manifest, auth *coreauth.Auth) {
	if manager == nil || m == nil || auth == nil || strings.TrimSpace(auth.ID) == "" {
		return
	}
	models := filterRegistryModelsByExcluded(manifestRegistryModels(m), excludedModelsForAuth(m, auth))
	if len(models) == 0 {
		cliproxy.GlobalModelRegistry().UnregisterClient(auth.ID)
		manager.RefreshSchedulerEntry(auth.ID)
		return
	}
	cliproxy.GlobalModelRegistry().RegisterClient(auth.ID, "codex", models)
	manager.ReconcileRegistryModelStates(context.Background(), auth.ID)
	manager.RefreshSchedulerEntry(auth.ID)
}

func excludedModelsForAuth(m *manifest, auth *coreauth.Auth) []string {
	seen := make(map[string]struct{})
	add := func(items []string) {
		for _, item := range items {
			trimmed := strings.TrimSpace(item)
			if trimmed == "" {
				continue
			}
			key := strings.ToLower(trimmed)
			if _, exists := seen[key]; exists {
				continue
			}
			seen[key] = struct{}{}
		}
	}
	if m != nil {
		add(m.ExcludedModels)
		if account := accountForAuthInManifest(m, auth); account != nil {
			add(accountExcludedModelsFromManifest(m, account.ID))
		}
	}
	if auth != nil {
		add(extractExcludedModelsFromMetadataMap(auth.Metadata))
		if auth.Attributes != nil {
			add(strings.Split(auth.Attributes["excluded_models"], ","))
		}
	}
	if len(seen) == 0 {
		return nil
	}
	out := make([]string, 0, len(seen))
	for item := range seen {
		out = append(out, item)
	}
	sort.Strings(out)
	return out
}

func accountExcludedModelsFromManifest(m *manifest, accountID string) []string {
	if m == nil || strings.TrimSpace(accountID) == "" {
		return nil
	}
	for _, rule := range m.AccountModelRules {
		if strings.TrimSpace(rule.AccountID) == accountID {
			return append([]string(nil), rule.ExcludedModels...)
		}
	}
	return nil
}

func extractExcludedModelsFromMetadataMap(metadata map[string]any) []string {
	if metadata == nil {
		return nil
	}
	raw, ok := metadata["excluded_models"]
	if !ok {
		raw, ok = metadata["excluded-models"]
	}
	if !ok || raw == nil {
		return nil
	}
	switch values := raw.(type) {
	case string:
		return strings.Split(values, ",")
	case []string:
		return append([]string(nil), values...)
	case []any:
		out := make([]string, 0, len(values))
		for _, item := range values {
			if value, ok := item.(string); ok {
				out = append(out, value)
			}
		}
		return out
	default:
		return nil
	}
}

func filterRegistryModelsByExcluded(models []*cliproxy.ModelInfo, excluded []string) []*cliproxy.ModelInfo {
	if len(models) == 0 || len(excluded) == 0 {
		return models
	}
	filtered := make([]*cliproxy.ModelInfo, 0, len(models))
	for _, model := range models {
		if model == nil || strings.TrimSpace(model.ID) == "" || modelMatchesAnyRule(model.ID, excluded) {
			continue
		}
		filtered = append(filtered, model)
	}
	return filtered
}

func authModelExcluded(m *manifest, auth *coreauth.Auth, model string) bool {
	model = strings.TrimSpace(model)
	if model == "" || auth == nil {
		return false
	}
	excluded := excludedModelsForAuth(m, auth)
	if len(excluded) == 0 {
		return false
	}
	if modelMatchesAnyRule(model, excluded) {
		return true
	}
	canonical := resolveSupportedModelAlias(m, model)
	return canonical != "" && modelMatchesAnyRule(canonical, excluded)
}

func manifestRegistryModels(m *manifest) []*cliproxy.ModelInfo {
	if m == nil {
		return nil
	}
	entries := make([]manifestRegistryModelEntry, 0, len(m.ModelIDs)+len(m.ModelAliases)*2)
	seen := make(map[string]struct{}, cap(entries))
	for _, id := range m.ModelIDs {
		entries = appendManifestRegistryModelEntry(entries, seen, id, "")
	}
	for _, alias := range m.ModelAliases {
		entries = appendManifestRegistryModelEntry(entries, seen, alias.SourceModel, "")
		entries = appendManifestRegistryModelEntry(entries, seen, alias.Alias, alias.SourceModel)
	}
	for _, id := range appendCodexInternalModels(nil) {
		entries = appendManifestRegistryModelEntry(entries, seen, id, "")
	}
	models := make([]*cliproxy.ModelInfo, 0, len(entries))
	now := time.Now().Unix()
	for _, entry := range entries {
		models = append(models, manifestRegistryModelInfo(entry.id, entry.source, now))
	}
	return models
}

type manifestRegistryModelEntry struct {
	id     string
	source string
}

func appendManifestRegistryModelEntry(entries []manifestRegistryModelEntry, seen map[string]struct{}, id string, source string) []manifestRegistryModelEntry {
	id = strings.TrimSpace(id)
	if id == "" {
		return entries
	}
	key := strings.ToLower(id)
	if _, exists := seen[key]; exists {
		return entries
	}
	seen[key] = struct{}{}
	return append(entries, manifestRegistryModelEntry{
		id:     id,
		source: strings.TrimSpace(source),
	})
}

func manifestRegistryModelInfo(id string, source string, created int64) *cliproxy.ModelInfo {
	info := &cliproxy.ModelInfo{
		ID:          id,
		Object:      "model",
		Created:     created,
		OwnedBy:     "openai",
		Type:        "openai",
		DisplayName: displayNameForModel(id),
	}
	lookupID := id
	if source != "" {
		lookupID = source
	}
	if staticInfo := internalregistry.LookupStaticModelInfo(lookupID); staticInfo != nil {
		if staticInfo.Thinking != nil {
			info.Thinking = staticInfo.Thinking
		}
		return info
	}
	info.UserDefined = true
	return info
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

// codexAlphaSearcher forwards Responses Lite web.run search requests using the
// same OAuth account pool as /v1/responses.
type codexAlphaSearcher interface {
	CodexAlphaSearch(ctx context.Context, model string, body []byte, headers http.Header) (status int, respHeaders http.Header, payload []byte, err error)
}

type relayServer struct {
	runtime            executorRuntime
	cfg                *config.Config
	manifest           *manifest
	authManager        *coreauth.Manager
	emitter            *eventEmitter
	policy             *requestPolicy
	responsesWebsocket gin.HandlerFunc
	quotaPoolStatePath string
}

func (s *relayServer) router() *gin.Engine {
	router := gin.New()
	router.Use(gin.Recovery())
	router.Use(corsMiddleware())
	router.Use(s.policy.middleware())
	router.GET("/v1/models", s.handleModels)
	router.GET(cockpitQuotaPath, s.handleCockpitQuota)
	router.POST("/v1/cockpit/auth/reset", s.handleResetAuthState)
	// Codex Responses WebSocket upgrade uses GET /v1/responses (not POST/SSE).
	router.GET("/v1/responses", s.handleResponsesWebsocket)
	router.POST("/v1/responses", s.handleResponses)
	router.POST("/v1/responses/compact", s.handleResponsesCompact)
	// Compatibility: some clients set chat-completions base and still append /v1/responses.
	router.POST("/v1/chat/completions/v1/responses", s.handleResponses)
	router.POST("/v1/chat/completions/v1/responses/compact", s.handleResponsesCompact)
	router.POST("/v1/chat/completions", s.handleChatCompletions)
	// Responses Lite web.run independent search endpoint.
	router.POST(codexAlphaSearchPath, s.handleCodexAlphaSearch)
	router.POST(codexDirectAlphaSearchPath, s.handleCodexAlphaSearch)
	router.POST(anthropicMessagesPath, s.handleAnthropicMessages)
	router.POST(anthropicCountTokensPath, s.handleAnthropicCountTokens)
	router.GET(geminiModelsPath, s.handleGeminiModels)
	router.GET(geminiModelsPath+"/*action", s.handleGeminiModel)
	router.POST(geminiModelsPath+"/*action", s.handleGeminiAction)
	router.POST(imagesGenerationsPath, s.handleImagesGenerations)
	router.POST(imagesEditsPath, s.handleImagesEdits)
	router.GET(ollamaVersionPath, s.handleOllamaVersion)
	router.GET(ollamaTagsPath, s.handleOllamaTags)
	router.POST(ollamaShowPath, s.handleOllamaShow)
	router.POST(ollamaChatPath, s.handleOllamaChat)
	router.NoRoute(func(c *gin.Context) {
		writeAPIError(c, http.StatusNotFound, "endpoint not supported", "not_found")
	})
	return router
}

type quotaPoolWindowState struct {
	Present          *bool  `json:"present,omitempty"`
	RemainingPercent *int   `json:"remainingPercent,omitempty"`
	WindowMinutes    *int64 `json:"windowMinutes,omitempty"`
	ResetAt          *int64 `json:"resetAt,omitempty"`
}

type quotaPoolAccountState struct {
	Primary   *quotaPoolWindowState `json:"primary,omitempty"`
	Secondary *quotaPoolWindowState `json:"secondary,omitempty"`
	UpdatedAt *int64                `json:"updatedAt,omitempty"`
}

type quotaPoolStateFile struct {
	Accounts map[string]quotaPoolAccountState `json:"accounts"`
}

type cockpitQuotaResponse struct {
	Version                  int                       `json:"version"`
	Scope                    string                    `json:"scope"`
	RemainingPercent         *int                      `json:"remainingPercent,omitempty"`
	WeeklyRemainingPercent   *int                      `json:"weeklyRemainingPercent,omitempty"`
	FiveHourRemainingPercent *int                      `json:"fiveHourRemainingPercent,omitempty"`
	AccountCount             int                       `json:"accountCount"`
	IncludedAccountCount     int                       `json:"includedAccountCount"`
	MissingAccountCount      int                       `json:"missingAccountCount"`
	AvailableAccountCount    int                       `json:"availableAccountCount"`
	AbnormalAccountCount     int                       `json:"abnormalAccountCount"`
	CooldownAccountCount     int                       `json:"cooldownAccountCount"`
	Plans                    []cockpitQuotaPlanSummary `json:"plans,omitempty"`
	UpdatedAt                int64                     `json:"updatedAt,omitempty"`
	Stale                    bool                      `json:"stale"`
}

type cockpitQuotaPlanSummary struct {
	Plan                     string `json:"plan"`
	Count                    int    `json:"count"`
	WeeklyRemainingPercent   *int   `json:"weeklyRemainingPercent,omitempty"`
	FiveHourRemainingPercent *int   `json:"fiveHourRemainingPercent,omitempty"`
}

func readQuotaPoolState(path string) (quotaPoolStateFile, error) {
	content, err := os.ReadFile(strings.TrimSpace(path))
	if err != nil {
		return quotaPoolStateFile{}, err
	}
	var state quotaPoolStateFile
	if err := json.Unmarshal(content, &state); err != nil {
		return quotaPoolStateFile{}, err
	}
	if state.Accounts == nil {
		state.Accounts = make(map[string]quotaPoolAccountState)
	}
	return state, nil
}

func quotaWindowPresent(window *quotaPoolWindowState) bool {
	return window != nil && (window.Present == nil || *window.Present)
}

func quotaWindowValue(window *quotaPoolWindowState) (int, int64, bool) {
	if !quotaWindowPresent(window) || window.RemainingPercent == nil {
		return 0, 0, false
	}
	minutes := int64(10080)
	if window.WindowMinutes != nil && *window.WindowMinutes > 0 {
		minutes = *window.WindowMinutes
	}
	return *window.RemainingPercent, minutes, true
}

func quotaPlanLabel(account *accountSpec) string {
	if account == nil {
		return "UNKNOWN"
	}
	if strings.EqualFold(strings.TrimSpace(account.AuthKind), "api_key") {
		return "API_KEY"
	}
	plan := strings.TrimSpace(account.PlanType)
	if plan == "" {
		return "UNKNOWN"
	}
	return strings.ToUpper(strings.ReplaceAll(strings.ReplaceAll(plan, "-", "_"), " ", "_"))
}

func addQuotaPercent(current *int, value int) *int {
	result := value
	if current != nil {
		result += *current
	}
	return &result
}

func buildCockpitQuotaResponse(spec *apiKeySpec, state quotaPoolStateFile, now time.Time) cockpitQuotaResponse {
	return buildCockpitQuotaResponseWithAccounts(spec, state, now, nil)
}

func buildCockpitQuotaResponseWithAccounts(spec *apiKeySpec, state quotaPoolStateFile, now time.Time, accounts map[string]*accountSpec) cockpitQuotaResponse {
	accountIDs := make([]string, 0)
	if spec != nil {
		accountIDs = normalizeStringList(spec.AccountIDs)
	}
	quotaAccountIDs := accountIDs
	if accounts != nil {
		quotaAccountIDs = make([]string, 0, len(accountIDs))
		for _, accountID := range accountIDs {
			if account := accounts[accountID]; account != nil && strings.EqualFold(strings.TrimSpace(account.AuthKind), "api_key") {
				continue
			}
			quotaAccountIDs = append(quotaAccountIDs, accountID)
		}
	}
	result := cockpitQuotaResponse{
		Version:      1,
		Scope:        "api_key_account_pool",
		AccountCount: len(quotaAccountIDs),
	}
	planIndex := make(map[string]int)
	for _, accountID := range accountIDs {
		var account *accountSpec
		if accounts != nil {
			account = accounts[accountID]
		}
		plan := quotaPlanLabel(account)
		index, exists := planIndex[plan]
		if !exists {
			index = len(result.Plans)
			planIndex[plan] = index
			result.Plans = append(result.Plans, cockpitQuotaPlanSummary{Plan: plan})
		}
		result.Plans[index].Count++
	}
	total := 0
	hasValue := false
	weeklyTotal := 0
	fiveHourTotal := 0
	hasWeekly := false
	hasFiveHour := false
	for _, accountID := range quotaAccountIDs {
		item, ok := state.Accounts[accountID]
		if !ok {
			result.MissingAccountCount++
			result.AbnormalAccountCount++
			continue
		}
		primaryValue, primaryMinutes, primaryOK := quotaWindowValue(item.Primary)
		secondaryValue, secondaryMinutes, secondaryOK := quotaWindowValue(item.Secondary)
		value, ok := 0, false
		switch {
		case primaryOK && secondaryOK && primaryMinutes <= secondaryMinutes:
			value, ok = primaryValue, true
		case primaryOK && secondaryOK:
			value, ok = secondaryValue, true
		case primaryOK:
			value, ok = primaryValue, true
		case secondaryOK:
			value, ok = secondaryValue, true
		}
		if !ok {
			result.MissingAccountCount++
			result.AbnormalAccountCount++
			continue
		}
		result.AvailableAccountCount++
		total += value
		hasValue = true
		if primaryOK && primaryMinutes >= 5*24*60 {
			weeklyTotal += primaryValue
			hasWeekly = true
		}
		if secondaryOK && secondaryMinutes >= 5*24*60 {
			weeklyTotal += secondaryValue
			hasWeekly = true
		}
		if primaryOK && primaryMinutes > 0 && primaryMinutes <= 6*60 {
			fiveHourTotal += primaryValue
			hasFiveHour = true
		}
		if secondaryOK && secondaryMinutes > 0 && secondaryMinutes <= 6*60 {
			fiveHourTotal += secondaryValue
			hasFiveHour = true
		}
		var account *accountSpec
		if accounts != nil {
			account = accounts[accountID]
		}
		if index, exists := planIndex[quotaPlanLabel(account)]; exists {
			planSummary := &result.Plans[index]
			if primaryOK && primaryMinutes >= 5*24*60 {
				planSummary.WeeklyRemainingPercent = addQuotaPercent(planSummary.WeeklyRemainingPercent, primaryValue)
			}
			if secondaryOK && secondaryMinutes >= 5*24*60 {
				planSummary.WeeklyRemainingPercent = addQuotaPercent(planSummary.WeeklyRemainingPercent, secondaryValue)
			}
			if primaryOK && primaryMinutes > 0 && primaryMinutes <= 6*60 {
				planSummary.FiveHourRemainingPercent = addQuotaPercent(planSummary.FiveHourRemainingPercent, primaryValue)
			}
			if secondaryOK && secondaryMinutes > 0 && secondaryMinutes <= 6*60 {
				planSummary.FiveHourRemainingPercent = addQuotaPercent(planSummary.FiveHourRemainingPercent, secondaryValue)
			}
		}
		result.IncludedAccountCount++
		if item.UpdatedAt != nil {
			if *item.UpdatedAt > result.UpdatedAt {
				result.UpdatedAt = *item.UpdatedAt
			}
			if now.Unix()-*item.UpdatedAt > 15*60 {
				result.Stale = true
			}
		}
	}
	if hasValue {
		result.RemainingPercent = &total
	}
	if hasWeekly {
		result.WeeklyRemainingPercent = &weeklyTotal
	}
	if hasFiveHour {
		result.FiveHourRemainingPercent = &fiveHourTotal
	}
	return result
}

func applyCockpitQuotaAuthHealth(result *cockpitQuotaResponse, spec *apiKeySpec, state quotaPoolStateFile, auths []*coreauth.Auth, now time.Time) {
	if result == nil || spec == nil || len(auths) == 0 {
		return
	}
	targets := make(map[string]struct{}, len(spec.AccountIDs))
	for _, accountID := range normalizeStringList(spec.AccountIDs) {
		targets[accountID] = struct{}{}
	}
	seen := make(map[string]struct{})
	for _, auth := range auths {
		if auth == nil || auth.Attributes == nil {
			continue
		}
		accountID := strings.TrimSpace(auth.Attributes["account_id"])
		if _, ok := targets[accountID]; !ok {
			continue
		}
		if _, ok := seen[accountID]; ok {
			continue
		}
		seen[accountID] = struct{}{}
		isCooldown := auth.Unavailable && auth.NextRetryAfter.After(now)
		isAbnormal := auth.Disabled || auth.Status == coreauth.StatusDisabled || (auth.Unavailable && !isCooldown)
		if !isCooldown && !isAbnormal {
			continue
		}
		item, hasState := state.Accounts[accountID]
		_, _, primaryOK := quotaWindowValue(item.Primary)
		_, _, secondaryOK := quotaWindowValue(item.Secondary)
		wasAvailable := hasState && (primaryOK || secondaryOK)
		if wasAvailable && result.AvailableAccountCount > 0 {
			result.AvailableAccountCount--
		}
		if isCooldown {
			result.CooldownAccountCount++
			if !wasAvailable && result.AbnormalAccountCount > 0 {
				result.AbnormalAccountCount--
			}
		} else if wasAvailable {
			result.AbnormalAccountCount++
		}
	}
}

func (s *relayServer) handleCockpitQuota(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	if strings.TrimSpace(s.quotaPoolStatePath) == "" {
		c.JSON(http.StatusOK, cockpitQuotaResponse{Version: 1, Scope: "api_key_account_pool"})
		return
	}
	state, err := readQuotaPoolState(s.quotaPoolStatePath)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			c.JSON(http.StatusOK, cockpitQuotaResponse{Version: 1, Scope: "api_key_account_pool"})
			return
		}
		writeAPIError(c, http.StatusServiceUnavailable, "quota state unavailable", "quota_state_unavailable")
		return
	}
	response := buildCockpitQuotaResponseWithAccounts(spec, state, time.Now(), s.manifest.accountByID)
	if s.authManager != nil {
		applyCockpitQuotaAuthHealth(&response, spec, state, s.authManager.List(), time.Now())
	}
	if response.RemainingPercent == nil && spec != nil && spec.ProviderGateway != nil {
		upstreamURL, urlErr := providerGatewayURL(spec.ProviderGateway.BaseURL, cockpitQuotaPath)
		if urlErr == nil {
			request, requestErr := http.NewRequestWithContext(c.Request.Context(), http.MethodGet, upstreamURL, nil)
			if requestErr == nil {
				request.Header.Set("Authorization", "Bearer "+spec.ProviderGateway.APIKey)
				client := &http.Client{Timeout: 2 * time.Second}
				if upstream, doErr := client.Do(request); doErr == nil {
					defer upstream.Body.Close()
					if upstream.StatusCode == http.StatusOK {
						var upstreamResponse cockpitQuotaResponse
						if json.NewDecoder(upstream.Body).Decode(&upstreamResponse) == nil {
							c.JSON(http.StatusOK, upstreamResponse)
							return
						}
					}
				}
			}
		}
	}
	c.JSON(http.StatusOK, response)
}

type resetAuthStateRequest struct {
	AccountIDs []string `json:"accountIds"`
}

func (s *relayServer) handleResetAuthState(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	if s.authManager == nil || s.manifest == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "auth manager unavailable", "service_unavailable")
		return
	}

	var req resetAuthStateRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		writeAPIError(c, http.StatusBadRequest, "invalid request body", "invalid_request")
		return
	}

	accountIDs := normalizeStringList(req.AccountIDs)
	if len(accountIDs) == 0 {
		writeAPIError(c, http.StatusBadRequest, "accountIds is required", "invalid_request")
		return
	}

	allowed := make(map[string]struct{}, len(spec.AccountIDs))
	for _, accountID := range spec.AccountIDs {
		allowed[strings.TrimSpace(accountID)] = struct{}{}
	}
	type resetTarget struct {
		accountID string
		authID    string
	}
	targets := make([]resetTarget, 0, len(accountIDs))
	for _, accountID := range accountIDs {
		account := s.manifest.accountByID[accountID]
		if account == nil || strings.TrimSpace(account.AuthID) == "" {
			continue
		}
		if len(allowed) > 0 {
			if _, ok := allowed[accountID]; !ok {
				continue
			}
		}
		targets = append(targets, resetTarget{accountID: accountID, authID: account.AuthID})
	}
	if len(targets) == 0 {
		writeAPIError(c, http.StatusNotFound, "no resettable accounts found", "account_not_found")
		return
	}

	resetAccountIDs := make([]string, 0, len(targets))
	for _, target := range targets {
		if auth, _ := s.authManager.ResetAuthState(c.Request.Context(), target.authID); auth != nil {
			resetAccountIDs = append(resetAccountIDs, target.accountID)
		}
	}
	c.JSON(http.StatusOK, gin.H{
		"status":     "ok",
		"reset":      len(resetAccountIDs),
		"accountIds": resetAccountIDs,
	})
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
		c.JSON(http.StatusOK, buildCodexClientModelsResponse(models, spec))
		return
	}
	c.JSON(http.StatusOK, buildModelsResponse(models))
}

func (s *relayServer) handleResponses(c *gin.Context) {
	s.handleExecutorRequest(c, sdktranslator.FormatOpenAIResponse, "")
}

func (s *relayServer) handleResponsesWebsocket(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	if spec.ProviderGateway != nil {
		writeAPIError(
			c,
			http.StatusBadRequest,
			"provider gateway does not support responses websocket",
			"websocket_not_supported",
		)
		return
	}
	if !spec.ResponsesWebsockets {
		writeAPIError(c, http.StatusBadRequest, "responses websocket is disabled", "websocket_disabled")
		return
	}
	if s.responsesWebsocket == nil {
		writeAPIError(
			c,
			http.StatusServiceUnavailable,
			"responses websocket unavailable",
			"service_unavailable",
		)
		return
	}
	s.responsesWebsocket(c)
}

func (s *relayServer) handleResponsesCompact(c *gin.Context) {
	s.handleExecutorRequest(c, sdktranslator.FormatOpenAIResponse, "responses/compact")
}

// handleCodexAlphaSearch proxies Codex web.run search requests to the ChatGPT
// Codex alpha search backend. Unlike /v1/responses, the body is already in
// Codex search format and must not pass through protocol translation.
func (s *relayServer) handleCodexAlphaSearch(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	if spec.ProviderGateway != nil {
		writeAPIError(c, http.StatusNotFound, "provider gateway does not support /v1/alpha/search", "not_found")
		return
	}
	searcher, ok := s.runtime.(codexAlphaSearcher)
	if !ok || searcher == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "Codex alpha search runtime is unavailable", "service_unavailable")
		return
	}

	body, err := io.ReadAll(io.LimitReader(c.Request.Body, maxCodexAlphaSearchRequestBytes))
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, "failed to read search request", "invalid_request")
		return
	}
	if len(bytes.TrimSpace(body)) == 0 {
		writeAPIError(c, http.StatusBadRequest, "request body is required", "invalid_request")
		return
	}

	var routing struct {
		ID    string `json:"id"`
		Model string `json:"model"`
	}
	_ = json.Unmarshal(body, &routing)
	model := strings.TrimSpace(routing.Model)
	if model == "" {
		if ctxModel, _ := c.Request.Context().Value(requestModelContextKey).(string); strings.TrimSpace(ctxModel) != "" {
			model = strings.TrimSpace(ctxModel)
		}
	}
	if model != "" {
		canonical := canonicalModelForClientModel(s.manifest, spec, model)
		if !validateClientModelVisible(s.manifest, spec, model, canonical) {
			writeAPIError(c, http.StatusNotFound, fmt.Sprintf("模型 %s 不在当前 API Key 的可用模型范围内", model), "model_not_available")
			return
		}
		model = canonical
	}

	headers := c.Request.Header.Clone()
	if sessionID := strings.TrimSpace(routing.ID); sessionID != "" {
		headers.Set("X-Session-ID", sessionID)
		if headers.Get("Session_id") == "" {
			headers.Set("Session_id", sessionID)
		}
	}

	startedAt := time.Now()
	s.emitExecutorDiagnostic(c, "executor_started", model, "alpha_search", startedAt, "")
	status, respHeaders, payload, err := searcher.CodexAlphaSearch(relayContext(c), model, body, headers)
	if err != nil {
		s.emitExecutorDiagnostic(c, "executor_failed", model, "alpha_search", startedAt, err.Error())
		s.writeExecutorError(c, err)
		return
	}
	s.emitExecutorDiagnostic(c, "executor_completed", model, "alpha_search", startedAt, "")
	writeUpstreamHeaders(c.Writer.Header(), respHeaders)
	contentType := ""
	if respHeaders != nil {
		contentType = respHeaders.Get("Content-Type")
	}
	if contentType == "" {
		contentType = "application/json"
	}
	if status <= 0 {
		status = http.StatusOK
	}
	c.Data(status, contentType, payload)
}

func (s *relayServer) handleChatCompletions(c *gin.Context) {
	s.handleExecutorRequest(c, sdktranslator.FormatOpenAI, "")
}

func (s *relayServer) handleAnthropicMessages(c *gin.Context) {
	s.handleExecutorRequest(c, sdktranslator.FormatClaude, "")
}

func (s *relayServer) handleAnthropicCountTokens(c *gin.Context) {
	s.handleTokenCount(c, sdktranslator.FormatClaude, "")
}

func (s *relayServer) handleGeminiModels(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	c.JSON(http.StatusOK, buildGeminiModelsResponse(clientCatalogModelsForAPIKey(s.manifest, spec)))
}

func (s *relayServer) handleGeminiModel(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	model, method, parseOK := parseGeminiModelAction(c.Param("action"))
	if !parseOK || method != "" {
		writeAPIError(c, http.StatusNotFound, "model not found", "not_found")
		return
	}
	body, canonical, ok := s.bodyWithValidatedModel(c, spec, []byte(`{}`), model, nil)
	if !ok {
		return
	}
	_ = body
	if !stringSliceContainsFold(clientCatalogModelsForAPIKey(s.manifest, spec), model) && !stringSliceContainsFold(clientCatalogModelsForAPIKey(s.manifest, spec), canonical) {
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("model %s not found", model), "not_found")
		return
	}
	c.JSON(http.StatusOK, buildGeminiModelEntry(canonical))
}

func (s *relayServer) handleGeminiAction(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	model, method, parseOK := parseGeminiModelAction(c.Param("action"))
	if !parseOK || method == "" {
		writeAPIError(c, http.StatusNotFound, "endpoint not supported", "not_found")
		return
	}
	body, err := readAndRestoreBody(c.Request)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, "failed to read request body", "invalid_request")
		return
	}
	forceStream := method == "streamGenerateContent"
	var streamPtr *bool
	if forceStream {
		streamPtr = &forceStream
	}
	body, _, ok = s.bodyWithValidatedModel(c, spec, body, model, streamPtr)
	if !ok {
		return
	}
	switch method {
	case "generateContent", "streamGenerateContent":
		s.handleExecutorBody(c, spec, body, sdktranslator.FormatGemini, "")
	case "countTokens":
		s.handleTokenCountBody(c, body, sdktranslator.FormatGemini)
	default:
		writeAPIError(c, http.StatusNotFound, "endpoint not supported", "not_found")
	}
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
	immediateSSE := imageReq.stream && s.manifest != nil && s.manifest.ImmediateSSEResponse
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
	streamCtx, cancelStream := context.WithCancel(relayContext(c))
	defer cancelStream()
	result, err := s.executeStreamWithOpenTimeout(c, streamCtx, []string{"codex"}, req, opts, model, startedAt, timeouts.open)
	if err != nil {
		if immediateSSE {
			writeImagesStreamError(c, immediateFlusher, err)
			return
		}
		s.writeExecutorError(c, err)
		return
	}
	if result == nil || result.Chunks == nil {
		if immediateSSE {
			writeImagesStreamError(c, immediateFlusher, relayStatusError{status: http.StatusBadGateway, message: "upstream stream is unavailable"})
			return
		}
		writeAPIError(c, http.StatusBadGateway, "upstream stream is unavailable", "bad_gateway")
		return
	}
	if imageReq.stream {
		s.forwardImagesStream(c, streamCtx, result, imageReq, timeouts.idle, immediateSSE)
		return
	}
	out, err := collectImagesResponse(streamCtx, result.Chunks, imageReq.responseFormat, timeouts.idle)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	writeUpstreamHeaders(c.Writer.Header(), result.Headers)
	c.Data(http.StatusOK, "application/json", out)
}

func (s *relayServer) forwardImagesStream(c *gin.Context, ctx context.Context, result *cliproxyexecutor.StreamResult, imageReq imageRelayRequest, idleTimeout time.Duration, headersCommitted bool) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	if !headersCommitted {
		setEventStreamHeaders(c.Writer.Header())
		writeUpstreamHeaders(c.Writer.Header(), result.Headers)
		c.Status(http.StatusOK)
	}

	writeEvent := func(eventName string, payload []byte) {
		if strings.TrimSpace(eventName) != "" {
			_, _ = fmt.Fprintf(c.Writer, "event: %s\n", eventName)
		}
		_, _ = fmt.Fprintf(c.Writer, "data: %s\n\n", string(payload))
		flusher.Flush()
	}
	writeErr := func(err error) {
		writeImagesStreamError(c, flusher, err)
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

func writeImagesStreamError(c *gin.Context, flusher http.Flusher, err error) {
	if c == nil || flusher == nil {
		return
	}
	payload, _ := json.Marshal(map[string]any{
		"error": map[string]any{
			"message": errorMessage(err),
			"type":    "upstream_error",
			"code":    statusCodeFromError(err),
		},
	})
	_, _ = fmt.Fprintf(c.Writer, "event: error\ndata: %s\n\n", string(payload))
	flusher.Flush()
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

func stringSliceContainsFold(values []string, target string) bool {
	for _, value := range values {
		if strings.EqualFold(strings.TrimSpace(value), strings.TrimSpace(target)) {
			return true
		}
	}
	return false
}

func (s *relayServer) handleOllamaVersion(c *gin.Context) {
	if _, ok := s.requireAPIKey(c); !ok {
		return
	}
	c.JSON(http.StatusOK, gin.H{"version": ollamaBridgeVersion})
}

func (s *relayServer) handleOllamaTags(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	c.JSON(http.StatusOK, buildOllamaTagsResponse(clientCatalogModelsForAPIKey(s.manifest, spec), time.Now()))
}

func (s *relayServer) handleOllamaShow(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	body, err := readAndRestoreBody(c.Request)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, "failed to read request body", "invalid_request")
		return
	}
	model := requestBodyModel(body)
	if model == "" {
		writeAPIError(c, http.StatusBadRequest, "model is required", "invalid_request")
		return
	}
	_, canonical, ok := s.bodyWithValidatedModel(c, spec, body, model, nil)
	if !ok {
		return
	}
	c.JSON(http.StatusOK, buildOllamaShowResponse(canonical, time.Now()))
}

func (s *relayServer) handleOllamaChat(c *gin.Context) {
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
	model := requestBodyModel(body)
	if model == "" {
		writeAPIError(c, http.StatusBadRequest, "model is required", "invalid_request")
		return
	}
	body, canonical, ok := s.bodyWithValidatedModel(c, spec, body, model, nil)
	if !ok {
		return
	}
	openAIBody, stream, err := buildOpenAIChatRequestFromOllama(body)
	if err != nil {
		writeAPIError(c, http.StatusBadRequest, err.Error(), "invalid_request")
		return
	}
	if spec.ProviderGateway != nil {
		s.handleOllamaProviderGatewayChat(c, spec.ProviderGateway, openAIBody, canonical, stream)
		return
	}
	if stream {
		s.handleOllamaRuntimeStream(c, openAIBody, canonical)
		return
	}
	s.handleOllamaRuntimeNonStream(c, openAIBody, canonical)
}

func buildOpenAIChatRequestFromOllama(body []byte) ([]byte, bool, error) {
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		return nil, false, fmt.Errorf("request body must be a JSON object")
	}
	model, _ := payload["model"].(string)
	if strings.TrimSpace(model) == "" {
		return nil, false, fmt.Errorf("model is required")
	}
	messages, ok := payload["messages"].([]any)
	if !ok {
		return nil, false, fmt.Errorf("messages is required")
	}
	stream := true
	if value, ok := payload["stream"].(bool); ok {
		stream = value
	}
	out := map[string]any{
		"model":    strings.TrimSpace(model),
		"messages": ollamaMessagesToOpenAI(messages),
		"stream":   stream,
	}
	if tools, ok := payload["tools"].([]any); ok && len(tools) > 0 {
		out["tools"] = tools
	}
	if options, ok := payload["options"].(map[string]any); ok {
		if value, ok := options["temperature"].(float64); ok {
			out["temperature"] = value
		}
		if value, ok := options["top_p"].(float64); ok {
			out["top_p"] = value
		}
		if value, ok := options["num_predict"].(float64); ok {
			out["max_tokens"] = int64(value)
		}
	}
	if effort := ollamaThinkingEffort(payload["think"]); effort != "" {
		out["reasoning_effort"] = effort
	}
	if responseFormat := ollamaResponseFormat(payload["format"]); responseFormat != nil {
		out["response_format"] = responseFormat
	}
	raw, err := json.Marshal(out)
	return raw, stream, err
}

func ollamaMessagesToOpenAI(messages []any) []any {
	out := make([]any, 0, len(messages))
	toolCallIDByName := map[string]string{}
	for index, raw := range messages {
		message, _ := raw.(map[string]any)
		role, _ := message["role"].(string)
		switch role {
		case "assistant":
			item := map[string]any{
				"role":    "assistant",
				"content": ollamaMessageContentToOpenAI(message),
			}
			if toolCalls, ok := message["tool_calls"].([]any); ok && len(toolCalls) > 0 {
				item["tool_calls"] = ollamaToolCallsToOpenAI(toolCalls, index, toolCallIDByName)
			}
			out = append(out, item)
		case "tool":
			toolName, _ := message["tool_name"].(string)
			if toolName == "" {
				toolName, _ = message["name"].(string)
			}
			toolCallID, _ := message["tool_call_id"].(string)
			if toolCallID == "" {
				toolCallID = toolCallIDByName[toolName]
			}
			if toolCallID == "" {
				toolCallID = fmt.Sprintf("tool_%d", index)
			}
			out = append(out, map[string]any{
				"role":         "tool",
				"tool_call_id": toolCallID,
				"content":      ollamaContentString(message["content"]),
			})
		default:
			if role != "system" {
				role = "user"
			}
			out = append(out, map[string]any{
				"role":    role,
				"content": ollamaMessageContentToOpenAI(message),
			})
		}
	}
	return out
}

func ollamaToolCallsToOpenAI(toolCalls []any, messageIndex int, toolCallIDByName map[string]string) []any {
	out := make([]any, 0, len(toolCalls))
	for index, raw := range toolCalls {
		toolCall, _ := raw.(map[string]any)
		fn, _ := toolCall["function"].(map[string]any)
		id, _ := toolCall["id"].(string)
		if id == "" {
			id = fmt.Sprintf("tool_%d_%d", messageIndex, index)
		}
		name, _ := fn["name"].(string)
		if name == "" {
			name = "tool"
		}
		toolCallIDByName[name] = id
		out = append(out, map[string]any{
			"id":   id,
			"type": "function",
			"function": map[string]any{
				"name":      name,
				"arguments": ollamaArgumentsString(fn["arguments"]),
			},
		})
	}
	return out
}

func ollamaMessageContentToOpenAI(message map[string]any) any {
	text := ollamaContentString(message["content"])
	images, _ := message["images"].([]any)
	if len(images) == 0 {
		return text
	}
	parts := make([]any, 0, len(images)+1)
	if text != "" {
		parts = append(parts, map[string]any{"type": "text", "text": text})
	}
	for _, image := range images {
		url, _ := image.(string)
		url = strings.TrimSpace(url)
		if url == "" {
			continue
		}
		if !strings.HasPrefix(url, "data:") && !strings.HasPrefix(url, "http://") && !strings.HasPrefix(url, "https://") {
			url = "data:image/png;base64," + url
		}
		parts = append(parts, map[string]any{
			"type":      "image_url",
			"image_url": map[string]any{"url": url},
		})
	}
	return parts
}

func ollamaContentString(value any) string {
	switch v := value.(type) {
	case string:
		return v
	default:
		if value == nil {
			return ""
		}
		raw, err := json.Marshal(value)
		if err != nil {
			return ""
		}
		return string(raw)
	}
}

func ollamaArgumentsString(value any) string {
	if s, ok := value.(string); ok {
		return s
	}
	if value == nil {
		return "{}"
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return "{}"
	}
	return string(raw)
}

func ollamaThinkingEffort(value any) string {
	switch v := value.(type) {
	case string:
		switch strings.ToLower(strings.TrimSpace(v)) {
		case "low", "medium", "high", "xhigh", "max", "ultra":
			return strings.ToLower(strings.TrimSpace(v))
		case "true":
			return "medium"
		default:
			return ""
		}
	case bool:
		if v {
			return "medium"
		}
	}
	return ""
}

func ollamaResponseFormat(value any) map[string]any {
	switch v := value.(type) {
	case string:
		if strings.EqualFold(strings.TrimSpace(v), "json") {
			return map[string]any{"type": "json_object"}
		}
	case map[string]any:
		return map[string]any{
			"type": "json_schema",
			"json_schema": map[string]any{
				"name":   "ollama_schema",
				"schema": v,
				"strict": true,
			},
		}
	}
	return nil
}

func (s *relayServer) handleOllamaRuntimeNonStream(c *gin.Context, body []byte, model string) {
	req, opts := buildExecutorRequest(c, body, model, sdktranslator.FormatOpenAI, "", false)
	resp, err := s.runtime.Execute(relayContext(c), []string{"codex"}, req, opts)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	payload := convertOpenAIChatResponseToOllama(resp.Payload, model)
	writeUpstreamHeaders(c.Writer.Header(), resp.Headers)
	c.Data(http.StatusOK, "application/json", payload)
}

func (s *relayServer) handleOllamaRuntimeStream(c *gin.Context, body []byte, model string) {
	req, opts := buildExecutorRequest(c, body, model, sdktranslator.FormatOpenAI, "", true)
	startedAt := time.Now()
	timeouts := s.streamTimeoutsForRequest(c.Request, body, model)
	streamCtx, cancelStream := context.WithCancel(relayContext(c))
	defer cancelStream()
	result, err := s.executeStreamWithOpenTimeout(c, streamCtx, []string{"codex"}, req, opts, model, startedAt, timeouts.open)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	if result == nil || result.Chunks == nil {
		writeAPIError(c, http.StatusBadGateway, "upstream stream is unavailable", "bad_gateway")
		return
	}
	s.forwardOllamaRuntimeStream(c, streamCtx, result, model, timeouts.idle)
}

func (s *relayServer) forwardOllamaRuntimeStream(c *gin.Context, ctx context.Context, result *cliproxyexecutor.StreamResult, model string, idleTimeout time.Duration) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	c.Header("Content-Type", "application/x-ndjson; charset=utf-8")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	writeUpstreamHeaders(c.Writer.Header(), result.Headers)
	c.Status(http.StatusOK)

	state := newOllamaStreamState(model)
	if idleTimeout <= 0 {
		idleTimeout = streamIdleTimeout
	}
	idleTimer := time.NewTimer(idleTimeout)
	defer idleTimer.Stop()
	for {
		select {
		case <-idleTimer.C:
			writeOllamaErrorLine(c.Writer, relayTimeoutError{phase: "stream_idle", timeout: idleTimeout})
			flusher.Flush()
			return
		case <-ctx.Done():
			writeOllamaErrorLine(c.Writer, ctx.Err())
			flusher.Flush()
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
				writeOllamaJSONLine(c.Writer, state.finalChunk())
				flusher.Flush()
				return
			}
			if chunk.Err != nil {
				writeOllamaErrorLine(c.Writer, chunk.Err)
				flusher.Flush()
				return
			}
			for _, payload := range openAIStreamPayloadsFromChunk(chunk.Payload) {
				for _, event := range state.applyOpenAIChunk(payload) {
					writeOllamaJSONLine(c.Writer, event)
				}
			}
			flusher.Flush()
		}
	}
}

func (s *relayServer) handleOllamaProviderGatewayChat(c *gin.Context, gateway *providerGatewaySpec, body []byte, model string, stream bool) {
	if gateway == nil {
		writeAPIError(c, http.StatusBadGateway, "provider gateway is not configured", "bad_gateway")
		return
	}
	if normalizeProviderGatewayWireAPI(gateway.WireAPI) != "chat_completions" {
		writeAPIError(c, http.StatusBadRequest, "Ollama bridge requires provider gateway wire API chat_completions", "invalid_request")
		return
	}
	upstreamModel := providerGatewayCanonicalModel(gateway, model)
	if strings.TrimSpace(upstreamModel) == "" {
		writeAPIError(c, http.StatusNotFound, fmt.Sprintf("model %s is not available for this provider gateway", model), "model_not_available")
		return
	}
	if providerGatewayRequestHasVisionInput(body) && !providerGatewayModelSupportsVision(gateway, upstreamModel) {
		writeAPIError(c, http.StatusBadRequest, fmt.Sprintf("model %s does not support image input", upstreamModel), "unsupported_image_input")
		return
	}
	upstreamBody := rewriteProviderGatewayBodyModel(body, upstreamModel)
	upstreamURL, err := providerGatewayURL(gateway.BaseURL, "/v1/chat/completions")
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
		s.forwardOllamaProviderGatewayStream(c, resp.Body, upstreamModel, resp.Header)
		return
	}
	payload, err := io.ReadAll(resp.Body)
	if err != nil {
		writeAPIError(c, http.StatusBadGateway, err.Error(), "bad_gateway")
		return
	}
	writeUpstreamHeaders(c.Writer.Header(), resp.Header)
	c.Data(http.StatusOK, "application/json", convertOpenAIChatResponseToOllama(payload, upstreamModel))
}

func (s *relayServer) forwardOllamaProviderGatewayStream(c *gin.Context, body io.Reader, model string, headers http.Header) {
	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		writeAPIError(c, http.StatusInternalServerError, "streaming not supported", "streaming_not_supported")
		return
	}
	c.Header("Content-Type", "application/x-ndjson; charset=utf-8")
	c.Header("Cache-Control", "no-cache")
	c.Header("Connection", "keep-alive")
	writeUpstreamHeaders(c.Writer.Header(), headers)
	c.Status(http.StatusOK)

	state := newOllamaStreamState(model)
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)
	for scanner.Scan() {
		for _, payload := range openAIStreamPayloadsFromChunk(scanner.Bytes()) {
			for _, event := range state.applyOpenAIChunk(payload) {
				writeOllamaJSONLine(c.Writer, event)
			}
		}
		flusher.Flush()
	}
	if err := scanner.Err(); err != nil {
		writeOllamaErrorLine(c.Writer, err)
		flusher.Flush()
		return
	}
	writeOllamaJSONLine(c.Writer, state.finalChunk())
	flusher.Flush()
}

type ollamaToolCallAccumulator struct {
	ID        string
	Name      string
	Arguments string
}

type ollamaStreamState struct {
	model            string
	content          string
	thinking         string
	promptTokens     int64
	completionTokens int64
	doneReason       string
	toolCalls        map[int]*ollamaToolCallAccumulator
}

func newOllamaStreamState(model string) *ollamaStreamState {
	return &ollamaStreamState{
		model:      model,
		doneReason: "stop",
		toolCalls:  map[int]*ollamaToolCallAccumulator{},
	}
}

func (s *ollamaStreamState) applyOpenAIChunk(payload []byte) []gin.H {
	payload = bytes.TrimSpace(payload)
	if len(payload) == 0 || bytes.Equal(payload, []byte("[DONE]")) {
		return nil
	}
	var root map[string]any
	if err := json.Unmarshal(payload, &root); err != nil {
		return nil
	}
	if usage, ok := root["usage"].(map[string]any); ok {
		if value, ok := numericInt64(usage["prompt_tokens"]); ok {
			s.promptTokens = value
		}
		if value, ok := numericInt64(usage["completion_tokens"]); ok {
			s.completionTokens = value
		}
	}
	choices, _ := root["choices"].([]any)
	if len(choices) == 0 {
		return nil
	}
	choice, _ := choices[0].(map[string]any)
	if reason, _ := choice["finish_reason"].(string); reason != "" {
		s.doneReason = mapOpenAIFinishReasonToOllama(reason)
	}
	delta, _ := choice["delta"].(map[string]any)
	events := []gin.H{}
	if thinking, _ := delta["reasoning_content"].(string); thinking != "" {
		s.thinking += thinking
		events = append(events, gin.H{
			"model":      s.model,
			"created_at": time.Now().Format(time.RFC3339Nano),
			"message":    gin.H{"role": "assistant", "content": "", "thinking": thinking},
			"done":       false,
		})
	}
	if content, _ := delta["content"].(string); content != "" {
		s.content += content
		events = append(events, gin.H{
			"model":      s.model,
			"created_at": time.Now().Format(time.RFC3339Nano),
			"message":    gin.H{"role": "assistant", "content": content},
			"done":       false,
		})
	}
	if toolCalls, ok := delta["tool_calls"].([]any); ok {
		s.applyToolCallDeltas(toolCalls)
	}
	return events
}

func (s *ollamaStreamState) applyToolCallDeltas(toolCalls []any) {
	for _, raw := range toolCalls {
		item, _ := raw.(map[string]any)
		index := 0
		if value, ok := numericInt64(item["index"]); ok {
			index = int(value)
		}
		acc := s.toolCalls[index]
		if acc == nil {
			acc = &ollamaToolCallAccumulator{ID: fmt.Sprintf("tool_%d", index), Name: "tool"}
			s.toolCalls[index] = acc
		}
		if id, _ := item["id"].(string); id != "" {
			acc.ID = id
		}
		fn, _ := item["function"].(map[string]any)
		if name, _ := fn["name"].(string); name != "" {
			acc.Name = name
		}
		if arguments, _ := fn["arguments"].(string); arguments != "" {
			acc.Arguments += arguments
		}
	}
}

func (s *ollamaStreamState) finalChunk() gin.H {
	message := gin.H{
		"role":    "assistant",
		"content": s.content,
	}
	if s.thinking != "" {
		message["thinking"] = s.thinking
	}
	if toolCalls := s.ollamaToolCalls(); len(toolCalls) > 0 {
		message["tool_calls"] = toolCalls
	}
	return gin.H{
		"model":                s.model,
		"created_at":           time.Now().Format(time.RFC3339Nano),
		"message":              message,
		"done":                 true,
		"done_reason":          s.doneReason,
		"total_duration":       0,
		"load_duration":        0,
		"prompt_eval_count":    s.promptTokens,
		"prompt_eval_duration": 0,
		"eval_count":           s.completionTokens,
		"eval_duration":        0,
	}
}

func (s *ollamaStreamState) ollamaToolCalls() []gin.H {
	if len(s.toolCalls) == 0 {
		return nil
	}
	indexes := make([]int, 0, len(s.toolCalls))
	for index := range s.toolCalls {
		indexes = append(indexes, index)
	}
	sort.Ints(indexes)
	out := make([]gin.H, 0, len(indexes))
	for _, index := range indexes {
		acc := s.toolCalls[index]
		if acc == nil {
			continue
		}
		out = append(out, gin.H{
			"id":   acc.ID,
			"type": "function",
			"function": gin.H{
				"name":      acc.Name,
				"arguments": parseOllamaToolArguments(acc.Arguments),
			},
		})
	}
	return out
}

func convertOpenAIChatResponseToOllama(payload []byte, fallbackModel string) []byte {
	var root map[string]any
	if err := json.Unmarshal(payload, &root); err != nil {
		return payload
	}
	model, _ := root["model"].(string)
	if strings.TrimSpace(model) == "" {
		model = fallbackModel
	}
	createdSeconds := time.Now().Unix()
	if value, ok := numericInt64(root["created"]); ok && value > 0 {
		createdSeconds = value
	}
	choice := firstOpenAIChoice(root)
	message, _ := choice["message"].(map[string]any)
	usage, _ := root["usage"].(map[string]any)
	promptTokens, _ := numericInt64(usage["prompt_tokens"])
	completionTokens, _ := numericInt64(usage["completion_tokens"])
	outMessage := gin.H{
		"role":    "assistant",
		"content": stringFieldFromAny(message["content"]),
	}
	if thinking := stringFieldFromAny(message["reasoning_content"]); thinking != "" {
		outMessage["thinking"] = thinking
	}
	if toolCalls, ok := message["tool_calls"].([]any); ok && len(toolCalls) > 0 {
		outMessage["tool_calls"] = openAIToolCallsToOllama(toolCalls)
	}
	out := gin.H{
		"model":                model,
		"created_at":           time.Unix(createdSeconds, 0).Format(time.RFC3339Nano),
		"message":              outMessage,
		"done":                 true,
		"done_reason":          mapOpenAIFinishReasonToOllama(stringFieldFromAny(choice["finish_reason"])),
		"total_duration":       0,
		"load_duration":        0,
		"prompt_eval_count":    promptTokens,
		"prompt_eval_duration": 0,
		"eval_count":           completionTokens,
		"eval_duration":        0,
	}
	raw, err := json.Marshal(out)
	if err != nil {
		return payload
	}
	return raw
}

func firstOpenAIChoice(root map[string]any) map[string]any {
	choices, _ := root["choices"].([]any)
	if len(choices) == 0 {
		return map[string]any{}
	}
	choice, _ := choices[0].(map[string]any)
	if choice == nil {
		return map[string]any{}
	}
	return choice
}

func openAIToolCallsToOllama(toolCalls []any) []gin.H {
	out := make([]gin.H, 0, len(toolCalls))
	for index, raw := range toolCalls {
		item, _ := raw.(map[string]any)
		fn, _ := item["function"].(map[string]any)
		id := stringFieldFromAny(item["id"])
		if id == "" {
			id = fmt.Sprintf("tool_%d", index)
		}
		name := stringFieldFromAny(fn["name"])
		if name == "" {
			name = "tool"
		}
		out = append(out, gin.H{
			"id":   id,
			"type": "function",
			"function": gin.H{
				"name":      name,
				"arguments": parseOllamaToolArguments(stringFieldFromAny(fn["arguments"])),
			},
		})
	}
	return out
}

func parseOllamaToolArguments(raw string) any {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return gin.H{}
	}
	var parsed any
	if err := json.Unmarshal([]byte(raw), &parsed); err == nil {
		return parsed
	}
	return raw
}

func openAIStreamPayloadsFromChunk(chunk []byte) [][]byte {
	trimmed := bytes.TrimSpace(chunk)
	if len(trimmed) == 0 {
		return nil
	}
	var payloads [][]byte
	for _, line := range bytes.Split(trimmed, []byte("\n")) {
		line = bytes.TrimSpace(line)
		if len(line) == 0 {
			continue
		}
		if bytes.HasPrefix(line, []byte("data:")) {
			payload := bytes.TrimSpace(line[len("data:"):])
			if len(payload) > 0 {
				payloads = append(payloads, append([]byte(nil), payload...))
			}
			continue
		}
		if bytes.HasPrefix(line, []byte("event:")) || bytes.HasPrefix(line, []byte(":")) {
			continue
		}
		payloads = append(payloads, append([]byte(nil), line...))
	}
	return payloads
}

func writeOllamaJSONLine(w io.Writer, value any) {
	if w == nil {
		return
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return
	}
	_, _ = w.Write(raw)
	_, _ = w.Write([]byte("\n"))
}

func writeOllamaErrorLine(w io.Writer, err error) {
	writeOllamaJSONLine(w, gin.H{"error": errorMessage(err)})
}

func mapOpenAIFinishReasonToOllama(reason string) string {
	switch strings.TrimSpace(reason) {
	case "length":
		return "length"
	case "tool_calls", "function_call":
		return "tool_calls"
	default:
		return "stop"
	}
}

func numericInt64(value any) (int64, bool) {
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

func stringFieldFromAny(value any) string {
	if value == nil {
		return ""
	}
	if s, ok := value.(string); ok {
		return s
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return ""
	}
	return string(raw)
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
	monitorParentProcessPlatform(ctx, parentPID, cancel, emitter)
}

func main() {
	configPath := flag.String("config", "", "CLIProxyAPI config file")
	manifestPath := flag.String("manifest", "", "Cockpit sidecar manifest file")
	quotaReserveStatePath := flag.String("quota-reserve-state", "", "Cockpit OAuth quota reserve state file")
	quotaPoolStatePath := flag.String("quota-pool-state", "", "Cockpit account-pool quota state file")
	parentPID := flag.Int("parent-pid", 0, "Cockpit Tools parent process id")
	flag.Parse()

	emitter := &eventEmitter{}
	if strings.TrimSpace(*configPath) == "" || strings.TrimSpace(*manifestPath) == "" {
		emitter.emit(map[string]any{"type": "error", "message": "missing --config or --manifest"})
		os.Exit(2)
	}

	emitter.emitStartupStage("resolve_config_path")
	absConfigPath, err := filepath.Abs(*configPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}
	emitter.emitStartupStage("load_config")
	cfg, err := config.LoadConfig(absConfigPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}
	emitter.emitStartupStage("load_manifest")
	m, err := loadManifest(*manifestPath)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(2)
	}
	emitter.emitStartupStage("init_runtime")
	quotaState := newQuotaReserveStateStore(*quotaReserveStatePath, m)
	if err := quotaState.load(); err != nil {
		emitter.emit(map[string]any{
			"type":    "quota_reserve_state_error",
			"message": err.Error(),
		})
	}

	usageTracker := newRequestUsageTracker()
	policy := &requestPolicy{manifest: m, emitter: emitter, tracker: usageTracker}
	hook := &authHook{manifest: m, emitter: emitter}
	priorityState := newAPIKeyPriorityStateStore(*manifestPath)
	selector := &cockpitSelector{
		manifest:   m,
		emitter:    emitter,
		quota:      quotaState,
		priorities: priorityState,
		tracker:    usageTracker,
	}
	coreManager := buildCoreAuthManager(cfg, selector, hook, m, quotaState, usageTracker)

	signalCtx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	ctx, cancel := context.WithCancel(signalCtx)
	defer cancel()
	quotaState.start(ctx, emitter)
	monitorParentProcess(ctx, *parentPID, cancel, emitter)

	coreusage.RegisterPlugin(&usagePlugin{manifest: m, tracker: usageTracker})

	runtime, err := newSidecarRuntime(ctx, absConfigPath, cfg, m, coreManager)
	if err != nil {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(1)
	}
	defer runtime.Stop()
	emitter.emitStartupStage("start_http_server")

	// Reuse the same coreManager so WS upgrades share OAuth pool, routing and
	// session affinity with POST /v1/responses.
	var sdkCfg *config.SDKConfig
	if cfg != nil {
		sdkCfg = &cfg.SDKConfig
	}
	baseHandlers := sdkhandlers.NewBaseAPIHandlers(sdkCfg, coreManager)
	responsesHandler := sdkopenai.NewOpenAIResponsesAPIHandler(baseHandlers)
	relay := &relayServer{
		runtime:            runtime,
		cfg:                cfg,
		manifest:           m,
		authManager:        coreManager,
		emitter:            emitter,
		policy:             policy,
		responsesWebsocket: responsesHandler.ResponsesWebsocket,
		quotaPoolStatePath: *quotaPoolStatePath,
	}
	if err := runRelayHTTPServer(ctx, cfg, relay.router(), emitter); err != nil && !errors.Is(err, context.Canceled) {
		emitter.emit(map[string]any{"type": "error", "message": err.Error()})
		os.Exit(1)
	}
}
