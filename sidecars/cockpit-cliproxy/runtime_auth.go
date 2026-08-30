package main

import (
	"context"

	"encoding/json"
	"errors"

	"fmt"
	"io"

	"net/http"

	"os"

	"path/filepath"
	"runtime"
	"sort"

	"strings"
	"sync"

	"time"

	internalregistry "github.com/router-for-me/CLIProxyAPI/v7/internal/registry"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/watcher/synthesizer"

	sdkauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/auth"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"

	"github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/proxyutil"
)

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
		"Session-Id",
		"Session_id",
		"X-Session-ID",
		"X-Client-Request-Id",
		"X-Codex-Window-Id",
		"Thread-Id",
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
	if excluded := extractExcludedModelsFromMetadataMap(metadata); len(excluded) > 0 {
		auth.Attributes["excluded_models"] = strings.Join(excluded, ",")
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
			if value := strings.TrimSpace(auth.Attributes["excluded_models"]); value != "" {
				add(strings.Split(value, ","))
			}
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
	if thinking := codexClientThinkingSupport(lookupID); thinking != nil {
		info.Thinking = thinking
		return info
	}
	info.UserDefined = true
	return info
}

func codexClientThinkingSupport(modelID string) *internalregistry.ThinkingSupport {
	var catalog struct {
		Models []map[string]any `json:"models"`
	}
	if errDecode := json.Unmarshal(internalregistry.GetCodexClientModelsJSON(), &catalog); errDecode != nil {
		return nil
	}
	for _, model := range catalog.Models {
		if !strings.EqualFold(strings.TrimSpace(stringFieldFromAny(model["slug"])), strings.TrimSpace(modelID)) {
			continue
		}
		levels, ok := model["supported_reasoning_levels"].([]any)
		if !ok || len(levels) == 0 {
			return nil
		}
		out := &internalregistry.ThinkingSupport{}
		for _, raw := range levels {
			if level, ok := raw.(map[string]any); ok {
				if effort := strings.TrimSpace(stringFieldFromAny(level["effort"])); effort != "" {
					out.Levels = append(out.Levels, effort)
				}
			}
		}
		if len(out.Levels) == 0 {
			return nil
		}
		return out
	}
	return nil
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
