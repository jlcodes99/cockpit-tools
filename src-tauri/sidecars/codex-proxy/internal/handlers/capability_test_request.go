package handlers

import (
	"bytes"
	"context"
	"crypto/sha1"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/providers"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
)

// ============== 缓存定义 ==============

const (
	capabilityCacheTTL    = 30 * time.Minute
	capabilityCacheMaxTTL = 4 * time.Hour
)

type capabilityCacheEntry struct {
	response  CapabilityTestResponse
	createdAt time.Time
	expiresAt time.Time
}

var capabilityCache = struct {
	sync.RWMutex
	entries map[string]*capabilityCacheEntry
}{
	entries: make(map[string]*capabilityCacheEntry),
}

var (
	capabilityClaudeProvider    providers.Provider = &providers.ClaudeProvider{}
	capabilityOpenAIProvider    providers.Provider = &providers.OpenAIProvider{}
	capabilityGeminiProvider    providers.Provider = &providers.GeminiProvider{}
	capabilityResponsesProvider providers.Provider = &providers.ResponsesProvider{}
)

func buildCapabilityCacheKey(baseURL string, apiKey string, serviceType string, protocols []string, models []string, modelMappingHash string) string {
	sorted := make([]string, len(protocols))
	copy(sorted, protocols)
	sort.Strings(sorted)

	normalizedModels := normalizeCapabilityModels(models)
	metricsKey := metrics.GenerateMetricsIdentityKey(baseURL, apiKey, serviceType)
	key := fmt.Sprintf("%s:%s:%s", metricsKey, strings.Join(sorted, ","), strings.Join(normalizedModels, ","))
	if modelMappingHash != "" {
		key += ":" + modelMappingHash
	}
	return key
}

func hashModelMapping(m map[string]string) string {
	if len(m) == 0 {
		return ""
	}
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	var sb strings.Builder
	for _, k := range keys {
		sb.WriteString(k)
		sb.WriteString("=>")
		sb.WriteString(m[k])
		sb.WriteByte(';')
	}
	h := sha1.Sum([]byte(sb.String()))
	return hex.EncodeToString(h[:8])
}

func normalizeCapabilityModels(models []string) []string {
	if len(models) == 0 {
		return nil
	}
	unique := make(map[string]struct{}, len(models))
	normalized := make([]string, 0, len(models))
	for _, model := range models {
		trimmed := strings.TrimSpace(model)
		if trimmed == "" {
			continue
		}
		if _, exists := unique[trimmed]; exists {
			continue
		}
		unique[trimmed] = struct{}{}
		normalized = append(normalized, trimmed)
	}
	sort.Strings(normalized)
	return normalized
}

func buildCapabilityTestURL(baseURL, versionPrefix, endpoint string) string {
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	if skipVersionPrefix {
		baseURL = strings.TrimSuffix(baseURL, "#")
	}
	baseURL = strings.TrimSuffix(baseURL, "/")

	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
	hasVersionSuffix := versionPattern.MatchString(baseURL)
	if !hasVersionSuffix && !skipVersionPrefix {
		baseURL += versionPrefix
	}

	return baseURL + endpoint
}

func getCapabilityCache(key string) (*CapabilityTestResponse, bool) {
	capabilityCache.Lock()
	defer capabilityCache.Unlock()

	entry, ok := capabilityCache.entries[key]
	if !ok {
		return nil, false
	}

	now := time.Now()
	if now.After(entry.expiresAt) {
		delete(capabilityCache.entries, key)
		return nil, false
	}

	newExpiry := now.Add(capabilityCacheTTL)
	maxExpiry := entry.createdAt.Add(capabilityCacheMaxTTL)
	if newExpiry.After(maxExpiry) {
		newExpiry = maxExpiry
	}
	entry.expiresAt = newExpiry

	return &entry.response, true
}

func setCapabilityCache(key string, resp CapabilityTestResponse) {
	now := time.Now()
	capabilityCache.Lock()
	capabilityCache.entries[key] = &capabilityCacheEntry{
		response:  resp,
		createdAt: now,
		expiresAt: now.Add(capabilityCacheTTL),
	}
	capabilityCache.Unlock()
}

// PLACEHOLDER_REQUEST_BUILD

func channelKindToApiType(channelKind string) string {
	switch channelKind {
	case "messages":
		return "Messages"
	case "chat":
		return "Chat"
	case "gemini":
		return "Gemini"
	case "responses":
		return "Responses"
	case "images":
		return "Images"
	default:
		return "Messages"
	}
}

// ============== 请求构建 ==============

func buildTestRequestWithModel(protocol string, channel *config.UpstreamConfig, model string) (*http.Request, error) {
	urls := channel.GetAllBaseURLs()
	if len(urls) == 0 {
		return nil, fmt.Errorf("no base URL configured")
	}
	baseURL := urls[0]

	apiKey := ""
	if len(channel.APIKeys) > 0 {
		apiKey = channel.APIKeys[0]
	} else if len(channel.DisabledAPIKeys) > 0 {
		apiKey = channel.DisabledAPIKeys[0].Key
	} else {
		return nil, fmt.Errorf("no_api_key")
	}

	var (
		requestURL string
		body       []byte
		err        error
		isGemini   bool
	)

	switch protocol {
	case "messages":
		requestURL = buildCapabilityTestURL(baseURL, "/v1", "/messages")
		body, err = json.Marshal(map[string]interface{}{
			"model": model,
			"system": []map[string]interface{}{
				{
					"type": "text",
					"text": "x-anthropic-billing-header: cc_version=2.1.71.2f9; cc_entrypoint=cli;",
				},
				{
					"type": "text",
					"text": "You are a Claude agent, built on Anthropic's Claude Agent SDK.",
					"cache_control": map[string]string{
						"type": "ephemeral",
					},
				},
			},
			"messages":   []map[string]string{{"role": "user", "content": "What are you best at: code generation, creative writing, or math problem solving?"}},
			"max_tokens": 100,
			"stream":     true,
			"thinking": map[string]interface{}{
				"type": "disabled",
			},
		})

	case "chat":
		requestURL = buildCapabilityTestURL(baseURL, "/v1", "/chat/completions")
		body, err = json.Marshal(map[string]interface{}{
			"model": model,
			"messages": []map[string]string{
				{"role": "system", "content": "You are a helpful assistant."},
				{"role": "user", "content": "What are you best at: code generation, creative writing, or math problem solving?"},
			},
			"max_tokens":       100,
			"stream":           true,
			"reasoning_effort": "low",
		})

		// PLACEHOLDER_REQUEST_BUILD_2

	case "gemini":
		requestURL = buildCapabilityTestURL(baseURL, "/v1beta", "/models/"+model+":streamGenerateContent?alt=sse")
		body, err = json.Marshal(map[string]interface{}{
			"contents": []map[string]interface{}{
				{
					"role":  "user",
					"parts": []map[string]string{{"text": "What are you best at: code generation, creative writing, or math problem solving?"}},
				},
			},
			"systemInstruction": map[string]interface{}{
				"parts": []map[string]string{{"text": "You are Gemini CLI, an interactive CLI agent specializing in software engineering tasks."}},
			},
			"generationConfig": map[string]interface{}{
				"maxOutputTokens": 100,
				"thinkingConfig": map[string]interface{}{
					"thinkingLevel": "low",
				},
			},
		})
		isGemini = true

	case "responses":
		requestURL = buildCapabilityTestURL(baseURL, "/v1", "/responses")
		body, err = json.Marshal(map[string]interface{}{
			"model":             model,
			"input":             "What are you best at: code generation, creative writing, or math problem solving?",
			"instructions":      "You are Codex, a coding agent based on GPT-5.",
			"max_output_tokens": 100,
			"stream":            true,
			"reasoning": map[string]interface{}{
				"effort": "low",
			},
		})

	default:
		return nil, fmt.Errorf("unsupported protocol: %s", protocol)
	}

	if err != nil {
		return nil, fmt.Errorf("marshal request body failed: %w", err)
	}

	req, err := http.NewRequest("POST", requestURL, bytes.NewReader(body))
	if err != nil {
		return nil, fmt.Errorf("create request failed: %w", err)
	}

	req.Header.Set("Content-Type", "application/json")

	if isGemini {
		utils.SetGeminiAuthenticationHeader(req.Header, apiKey)
	} else {
		utils.SetAuthenticationHeader(req.Header, apiKey)
		if protocol == "messages" {
			req.Header.Set("anthropic-version", "2023-06-01")
			req.Header.Set("anthropic-beta", "claude-code-20250219,adaptive-thinking-2026-01-28,prompt-caching-scope-2026-01-05,effort-2025-11-24")
			req.Header.Set("User-Agent", "claude-cli/2.1.71 (external, cli)")
			req.Header.Set("X-App", "cli")
		}
		if protocol == "responses" {
			req.Header.Set("Originator", "codex_cli_rs")
			req.Header.Set("User-Agent", "codex_cli_rs/0.111.0 (Mac OS 26.3.0; arm64) iTerm.app/3.6.6")
		}
	}

	if channel.CustomHeaders != nil {
		for key, value := range channel.CustomHeaders {
			req.Header.Set(key, value)
		}
	}

	return req, nil
}

func buildTestRequest(protocol string, channel *config.UpstreamConfig) (*http.Request, error) {
	model, err := getCapabilityProbeModel(protocol)
	if err != nil {
		return nil, err
	}
	return buildTestRequestWithModel(protocol, channel, model)
}

func getCapabilityStreamProvider(protocol string) providers.Provider {
	switch protocol {
	case "messages":
		return capabilityClaudeProvider
	case "chat":
		return capabilityOpenAIProvider
	case "gemini":
		return capabilityGeminiProvider
	case "responses":
		return capabilityResponsesProvider
	default:
		return nil
	}
}

// PLACEHOLDER_STREAM

// ============== 流式响应检测 ==============

func sendAndCheckStream(ctx context.Context, channel *config.UpstreamConfig, req *http.Request, protocol string) (bool, bool, int, []byte, error) {
	envCfg := config.NewEnvConfig()
	apiType := channelKindToApiType(protocol)
	resp, err := common.SendRequest(req, channel, envCfg, true, apiType)
	if err != nil {
		return false, false, 0, nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		bodyBytes, _ := io.ReadAll(resp.Body)
		common.LogUpstreamResponse(resp, bodyBytes, envCfg, apiType)
		return false, false, resp.StatusCode, bodyBytes, fmt.Errorf("HTTP %d", resp.StatusCode)
	}

	common.LogUpstreamResponseHeaders(resp, envCfg, apiType)

	provider := getCapabilityStreamProvider(protocol)
	if provider == nil {
		return false, false, resp.StatusCode, nil, fmt.Errorf("unsupported protocol provider: %s", protocol)
	}

	responseLogBuffer := common.NewLimitedLogBuffer(common.MaxUpstreamResponseLogBytes)
	bodyReader := io.Reader(resp.Body)
	if envCfg.EnableResponseLogs && envCfg.IsDevelopment() {
		bodyReader = io.TeeReader(resp.Body, responseLogBuffer)
	}
	eventChan, errChan, err := provider.HandleStreamResponse(io.NopCloser(bodyReader))
	if err != nil {
		common.LogUpstreamResponseBody(responseLogBuffer.Bytes(), envCfg, apiType)
		return false, false, resp.StatusCode, nil, err
	}

	type streamResult struct {
		preflight *common.StreamPreflightResult
		err       error
	}
	doneCh := make(chan streamResult, 1)

	go func() {
		preflight := common.PreflightStreamEvents(eventChan, errChan)
		if preflight.HasError {
			doneCh <- streamResult{err: preflight.Error}
			return
		}
		doneCh <- streamResult{preflight: preflight}
	}()

	readCtx, readCancel := context.WithTimeout(ctx, 30*time.Second)
	defer readCancel()

	var result streamResult
	select {
	case result = <-doneCh:
	case <-readCtx.Done():
		common.LogUpstreamResponseBody(responseLogBuffer.Bytes(), envCfg, apiType)
		return false, false, resp.StatusCode, nil, fmt.Errorf("流式响应读取超时")
	}

	common.LogUpstreamResponseBody(responseLogBuffer.Bytes(), envCfg, apiType)

	if result.err != nil {
		return false, false, resp.StatusCode, nil, result.err
	}

	if result.preflight == nil {
		return false, false, resp.StatusCode, nil, fmt.Errorf("流式响应预检失败")
	}

	if result.preflight.IsEmpty {
		if result.preflight.Diagnostic != "" {
			return false, false, 0, nil, fmt.Errorf("上游返回空响应 (%s)", result.preflight.Diagnostic)
		}
		return false, false, 0, nil, common.ErrEmptyStreamResponse
	}

	if isTimedOutPreflightResult(result.preflight) {
		return false, false, 0, nil, fmt.Errorf("流式响应预检超时，未收到任何 SSE 事件")
	}

	go func(eventChan <-chan string) {
		for range eventChan {
		}
	}(eventChan)

	return true, true, resp.StatusCode, nil, nil
}

func isTimedOutPreflightResult(preflight *common.StreamPreflightResult) bool {
	if preflight == nil {
		return false
	}
	return !preflight.HasError && !preflight.IsEmpty && len(preflight.BufferedEvents) == 0 && preflight.Diagnostic == "" && preflight.UnknownEventType == ""
}

// ============== 错误分类 ==============

func classifyError(err error, statusCode int, ctx context.Context) string {
	if ctx.Err() == context.DeadlineExceeded {
		return "timeout"
	}

	errStr := ""
	if err != nil {
		errStr = err.Error()
		if errors.Is(err, common.ErrEmptyStreamResponse) || strings.Contains(errStr, "上游返回空响应") {
			return "empty_response"
		}
	}

	if statusCode == 429 {
		return "rate_limited"
	}

	if statusCode > 0 {
		return fmt.Sprintf("http_error_%d", statusCode)
	}

	if strings.Contains(errStr, "timeout") || strings.Contains(errStr, "deadline exceeded") {
		return "timeout"
	}

	if errStr == "" {
		return "request_failed"
	}
	return "request_failed: " + errStr
}
