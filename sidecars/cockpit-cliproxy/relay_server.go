package main

import (
	"bytes"

	"encoding/json"
	"errors"

	"fmt"
	"io"

	"net/http"

	"os"

	"strings"

	"time"

	"github.com/gin-gonic/gin"

	codexlive "github.com/router-for-me/CLIProxyAPI/v7/internal/client/codex/live"

	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"

	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

type relayServer struct {
	runtime            executorRuntime
	cfg                *config.Config
	manifest           *manifest
	authManager        *coreauth.Manager
	emitter            *eventEmitter
	policy             *requestPolicy
	responsesWebsocket gin.HandlerFunc
	codexLive          *codexlive.Handler
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
	router.POST("/v1/cockpit/accounts/reset-scheduler", s.handleResetSchedulerState)
	router.POST("/v1/live", s.handleCodexLive)
	router.GET("/v1/live/:call_id", s.handleCodexLiveSideband)
	router.POST("/v1/realtime/calls", s.handleCodexLive)
	router.GET("/v1/realtime/calls/:call_id", s.handleCodexLiveSideband)
	router.GET("/v1/realtime", s.handleCodexRealtimeWebsocket)
	router.POST("/v1/realtime", s.handleCodexRealtime)
	router.POST("/v1/realtime/client_secrets", s.handleCodexClientSecret)
	router.POST("/v1/realtime/sessions", s.handleCodexLegacySession)
	router.POST("/v1/realtime/transcription_sessions", s.handleCodexTranscriptionSession)
	router.GET("/v1/realtime/translations", s.handleCodexTranslation)
	router.POST("/v1/realtime/translations", s.handleCodexTranslation)
	router.POST("/v1/realtime/translations/client_secrets", s.handleCodexTranslation)
	router.POST("/v1/realtime/calls/:call_id/hangup", s.handleCodexHangup)
	router.POST("/v1/realtime/calls/:call_id/accept", s.handleCodexSIPControl)
	router.POST("/v1/realtime/calls/:call_id/reject", s.handleCodexSIPControl)
	router.POST("/v1/realtime/calls/:call_id/refer", s.handleCodexSIPControl)
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
	Cooldown  *quotaCooldownState   `json:"cooldown,omitempty"`
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
		// The API Service account view treats an exhausted weekly window as
		// blocking the shorter window too. Keep the CDP quota endpoint on the
		// same effective-quota semantics instead of exposing raw primary values
		// such as 400% for a four-account pool.
		if primaryOK && secondaryOK && secondaryValue == 0 {
			primaryValue = 0
		}
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
	quotaBlocked := false
	for _, accountID := range accountIDs {
		account := s.manifest.accountByID[accountID]
		if account == nil || strings.TrimSpace(account.AuthID) == "" {
			continue
		}
		auth, _ := s.authManager.GetByID(account.AuthID)
		if accountQuotaExhausted(s.manifest, account, time.Now()) || authHasQuotaCooldown(auth, time.Now()) {
			quotaBlocked = true
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
		if quotaBlocked {
			writeAPIError(c, http.StatusConflict, "quota cooldown cannot be cleared manually; wait for quota recovery", "quota_cooldown")
			return
		}
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

// handleResetSchedulerState resets the runtime scheduler state for accounts in
// the current API key scope. It resolves auth-manager entries through manifest
// identity data so both OAuth accounts and API-key accounts without authId are
// handled by the same endpoint.
func (s *relayServer) handleResetSchedulerState(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	if s.manifest == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "account manifest unavailable", "service_unavailable")
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
		if accountID = strings.TrimSpace(accountID); accountID != "" {
			allowed[accountID] = struct{}{}
		}
	}

	selected := make([]string, 0, len(accountIDs))
	quotaBlocked := false
	blockedAccounts, blockedAuths := make(map[string]bool), make(map[string]bool)
	if s.authManager != nil {
		for _, auth := range s.authManager.List() {
			if authHasQuotaCooldown(auth, time.Now()) {
				blockedAuths[auth.ID] = true
				if account := accountForAuthInManifest(s.manifest, auth); account != nil {
					blockedAccounts[account.ID] = true
				}
			}
		}
	}
	for _, accountID := range accountIDs {
		account := s.manifest.accountByID[accountID]
		if account == nil {
			continue
		}
		if accountQuotaExhausted(s.manifest, account, time.Now()) || blockedAccounts[accountID] || blockedAuths[account.AuthID] {
			quotaBlocked = true
			continue
		}
		if len(allowed) > 0 {
			if _, ok := allowed[accountID]; !ok {
				continue
			}
		}
		selected = append(selected, accountID)
	}
	if len(selected) == 0 {
		if quotaBlocked {
			writeAPIError(c, http.StatusConflict, "quota cooldown cannot be cleared manually; wait for quota recovery", "quota_cooldown")
			return
		}
		writeAPIError(c, http.StatusNotFound, "no matching accounts found", "account_not_found")
		return
	}

	selectedSet := make(map[string]struct{}, len(selected))
	for _, accountID := range selected {
		selectedSet[accountID] = struct{}{}
	}
	authIDs := make(map[string]struct{})
	if s.authManager != nil {
		for _, auth := range s.authManager.List() {
			account := accountForAuthInManifest(s.manifest, auth)
			if account == nil {
				continue
			}
			if _, ok := selectedSet[account.ID]; ok && strings.TrimSpace(auth.ID) != "" {
				authIDs[auth.ID] = struct{}{}
			}
		}
	}
	for _, accountID := range selected {
		if authID := strings.TrimSpace(s.manifest.accountByID[accountID].AuthID); authID != "" {
			authIDs[authID] = struct{}{}
		}
	}
	if len(authIDs) > 0 && s.authManager == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "auth manager unavailable", "service_unavailable")
		return
	}

	resetAuthCount := 0
	for authID := range authIDs {
		updated, err := s.authManager.ResetAuthState(c.Request.Context(), authID)
		if err != nil {
			writeAPIError(c, http.StatusBadGateway, err.Error(), "scheduler_reset_failed")
			return
		}
		if updated != nil {
			resetAuthCount++
		}
	}

	c.JSON(http.StatusOK, gin.H{
		"status":         "ok",
		"reset":          resetAuthCount,
		"accountIds":     selected,
		"authReset":      resetAuthCount,
		"schedulerReset": len(selected),
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
		c.JSON(http.StatusOK, buildCodexClientModelsResponse(models, spec, contextWindowsForAPIKey(s.manifest, spec)))
		return
	}
	c.JSON(http.StatusOK, buildModelsResponse(models))
}

func (s *relayServer) handleResponses(c *gin.Context) {
	s.handleExecutorRequest(c, sdktranslator.FormatOpenAIResponse, "")
}

func (s *relayServer) handleCodexLive(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	if spec.ProviderGateway != nil {
		writeAPIError(c, http.StatusBadRequest, "provider gateway does not support Codex live", "live_not_supported")
		return
	}
	if s.codexLive == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "Codex live unavailable", "service_unavailable")
		return
	}
	s.codexLive.Handle(c)
}

func (s *relayServer) handleCodexLiveSideband(c *gin.Context) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	if spec.ProviderGateway != nil {
		writeAPIError(c, http.StatusBadRequest, "provider gateway does not support Codex live", "live_not_supported")
		return
	}
	if s.codexLive == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "Codex live unavailable", "service_unavailable")
		return
	}
	s.codexLive.HandleSideband(c)
}

func (s *relayServer) codexRealtimeHandler(c *gin.Context, handle func(*codexlive.Handler, *gin.Context)) {
	spec, ok := s.requireAPIKey(c)
	if !ok {
		return
	}
	if spec.ProviderGateway != nil {
		writeAPIError(c, http.StatusBadRequest, "provider gateway does not support Codex realtime", "realtime_not_supported")
		return
	}
	if s.codexLive == nil {
		writeAPIError(c, http.StatusServiceUnavailable, "Codex realtime unavailable", "service_unavailable")
		return
	}
	handle(s.codexLive, c)
}

func (s *relayServer) handleCodexRealtimeWebsocket(c *gin.Context) {
	s.codexRealtimeHandler(c, func(h *codexlive.Handler, ctx *gin.Context) { h.HandleRealtimeWebsocket(ctx) })
}

func (s *relayServer) handleCodexRealtime(c *gin.Context) {
	s.codexRealtimeHandler(c, func(h *codexlive.Handler, ctx *gin.Context) { h.Handle(ctx) })
}

func (s *relayServer) handleCodexClientSecret(c *gin.Context) {
	s.codexRealtimeHandler(c, func(h *codexlive.Handler, ctx *gin.Context) { h.CreateClientSecret(ctx) })
}

func (s *relayServer) handleCodexLegacySession(c *gin.Context) {
	s.codexRealtimeHandler(c, func(h *codexlive.Handler, ctx *gin.Context) { h.CreateLegacySession(ctx) })
}

func (s *relayServer) handleCodexTranscriptionSession(c *gin.Context) {
	s.codexRealtimeHandler(c, func(h *codexlive.Handler, ctx *gin.Context) { h.HandleTranscriptionSession(ctx) })
}

func (s *relayServer) handleCodexTranslation(c *gin.Context) {
	s.codexRealtimeHandler(c, func(h *codexlive.Handler, ctx *gin.Context) { h.HandleTranslation(ctx) })
}

func (s *relayServer) handleCodexHangup(c *gin.Context) {
	s.codexRealtimeHandler(c, func(h *codexlive.Handler, ctx *gin.Context) { h.HandleHangup(ctx) })
}

func (s *relayServer) handleCodexSIPControl(c *gin.Context) {
	s.codexRealtimeHandler(c, func(h *codexlive.Handler, ctx *gin.Context) { h.HandleSIPControl(ctx) })
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
		if headers.Get("Session-Id") == "" && headers.Get("Session_id") == "" {
			headers.Set("Session-Id", sessionID)
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
