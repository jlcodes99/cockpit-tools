package main

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/gin-gonic/gin"
	codebuddyauth "github.com/router-for-me/CLIProxyAPI/v7/internal/auth/codebuddy"
	internalregistry "github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/util"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

// codebuddyImageToolModel is the placeholder CodeBuddy image model. The
// backend's dedicated image endpoint (/v2/images/generations) resolves the real
// image model from the client's requested model; this placeholder is registered
// so the model is visible under image generation mode and routable to the
// codebuddy provider.
const codebuddyImageToolModel = "codebuddy-image-1" // placeholder image model

func equalStringSlices(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// visionProxyEnabled reports whether the vision-proxy layer is active (mode is
// "routing" or "preprocess"). Used by /v1/models to report `input_modalities:
// ["text","image"]` for non-vision models that the proxy will transparently
// handle — otherwise clients (e.g. Cursor) filter image inputs client-side and
// the image never reaches the relay.
func (m *manifest) visionProxyEnabled() bool {
	if m == nil {
		return false
	}
	mode := strings.ToLower(strings.TrimSpace(m.VisionMode))
	return mode == "routing" || mode == "preprocess" || mode == "agentic"
}

// modelIDs returns a snapshot of the current model ID list. It is safe for
// concurrent use with setModelIDs.
func (m *manifest) modelIDs() []string {
	if m == nil {
		return nil
	}
	m.modelMu.RLock()
	defer m.modelMu.RUnlock()
	return m.ModelIDs
}

// setModelIDs atomically replaces the model ID list and reports whether the
// list actually changed.
func (m *manifest) setModelIDs(ids []string) bool {
	if m == nil {
		return false
	}
	m.modelMu.Lock()
	defer m.modelMu.Unlock()
	if equalStringSlices(m.ModelIDs, ids) {
		return false
	}
	m.ModelIDs = ids
	return true
}

// resolveRequestProviders determines the provider list backing a request model by
// consulting the global model registry. CodeBuddy models resolve to ["codebuddy"]
// while Codex models resolve to ["codex"]. Falls back to the legacy codex-only
// behavior when the model has no registered provider.
func resolveRequestProviders(model string) []string {
	seen := make(map[string]struct{})
	for _, candidate := range []string{util.ResolveAutoModel(model), model} {
		if candidate == "" {
			continue
		}
		if _, ok := seen[candidate]; ok {
			continue
		}
		seen[candidate] = struct{}{}
		if providers := util.GetProviderName(candidate); len(providers) > 0 {
			return providers
		}
	}
	return []string{"codex"}
}

func providersContain(providers []string, name string) bool {
	for _, p := range providers {
		if strings.EqualFold(strings.TrimSpace(p), name) {
			return true
		}
	}
	return false
}

// codebuddyModelsResponse mirrors the envelope returned by the official
// CodeBuddy backend model-list endpoint:
//
//	GET /v2/enterprises/personal/models
//	-> { "code":0, "msg":"OK", "data":{ "models":[ { "id":"glm-5.3", "tags":["craft"], ... } ] } }
type codebuddyModelsResponse struct {
	Code int    `json:"code"`
	Msg  string `json:"msg"`
	Data struct {
		Models []struct {
			ID               string   `json:"id"`
			Name             string   `json:"name"`
			Tags             []string `json:"tags"`
			SupportsImages   bool     `json:"supportsImages"`
			SupportsToolCall bool     `json:"supportsToolCall"`
			MaxOutputTokens  int      `json:"maxOutputTokens"`
			MaxInputTokens   int      `json:"maxInputTokens"`
		} `json:"models"`
	} `json:"data"`
}

// syncCodebuddyModelsFromBackend fetches the authoritative CodeBuddy model list
// from the official Tencent backend using the credentials of the first
// CodeBuddy auth record that carries an access token. It installs the result
// into the registry and returns the de-duplicated, sorted model IDs, or nil
// when no suitable auth is present or the backend cannot be reached/parsed.
//
// This is the preferred source over app.asar extraction because the backend
// exposes models (e.g. glm-5.3) that may not yet be bundled in the local client.
func syncCodebuddyModelsFromBackend(auths []*coreauth.Auth) []string {
	var creds codebuddyauth.Creds
	for _, a := range auths {
		if a == nil {
			continue
		}
		c := codebuddyauth.CredsFromAuth(a)
		// 仅使用中国站账号拉取模型清单：国际站（www.codebuddy.ai）账号体系
		// 暂未对外暴露，避免误取到国际站的模型目录。
		if !strings.EqualFold(strings.TrimSpace(c.Region), codebuddyauth.RegionCN) {
			continue
		}
		if strings.TrimSpace(c.AccessToken) != "" {
			creds = c
			break
		}
	}
	if creds.AccessToken == "" {
		return nil
	}

	req, err := http.NewRequest(http.MethodGet, creds.ResolveModelsURL(), nil)
	if err != nil {
		return nil
	}
	codebuddyauth.ApplyHeaders(req, creds)

	client := &http.Client{Timeout: 15 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil
	}

	var envelope codebuddyModelsResponse
	if err := json.NewDecoder(resp.Body).Decode(&envelope); err != nil {
		return nil
	}
	// 后端用 code != 0 表示业务错误。
	if envelope.Code != 0 {
		return nil
	}

	// 过滤非对话模型（如 text-to-image），与官方客户端 listAvailableModels 行为一致。
	nonChatTags := map[string]bool{"text-to-image": true}
	models := make([]*internalregistry.ModelInfo, 0, len(envelope.Data.Models))
	for _, m := range envelope.Data.Models {
		if strings.TrimSpace(m.ID) == "" {
			continue
		}
		hasNonChat := false
		for _, t := range m.Tags {
			if nonChatTags[strings.ToLower(strings.TrimSpace(t))] {
				hasNonChat = true
				break
			}
		}
		if hasNonChat {
			continue
		}
		models = append(models, &internalregistry.ModelInfo{
			ID:                  m.ID,
			Name:                m.Name,
			Object:              "model",
			OwnedBy:             "tencent",
			Type:                "codebuddy",
			SupportsImages:      m.SupportsImages,
			ContextLength:       m.MaxInputTokens,
			MaxCompletionTokens: m.MaxOutputTokens,
		})
	}

	// 探测过滤已下线模型：清单里存在但推理路由已返回 11102 的模型（如
	// glm-4.6v、kimi-k2-thinking）不应暴露给客户端。
	models = probeCodebuddyModelAvailability(creds, models)

	// 安装到 registry（去重、排序、变更检测、刷新通知）。
	return internalregistry.InstallCodebuddyModels(models)
}

// codebuddyAvailabilityProbeConcurrency caps the number of concurrent
// availability probe requests issued during a model sync.
const codebuddyAvailabilityProbeConcurrency = 5

// probeCodebuddyModelAvailability filters out models that the backend lists but
// whose inference route is already decommissioned (HTTP 400 with code 11102
// "service info not found", e.g. glm-4.6v, kimi-k2-thinking). A minimal
// 1-token chat request is issued per model; only a definitive 11102 marks a
// model unavailable, while network errors and other business errors keep the
// model so upstream flakiness does not thrash the catalog.
func probeCodebuddyModelAvailability(creds codebuddyauth.Creds, models []*internalregistry.ModelInfo) []*internalregistry.ModelInfo {
	if len(models) == 0 {
		return models
	}
	out := make([]*internalregistry.ModelInfo, 0, len(models))
	var mu sync.Mutex
	var wg sync.WaitGroup
	sem := make(chan struct{}, codebuddyAvailabilityProbeConcurrency)
	for _, m := range models {
		if m == nil {
			continue
		}
		wg.Add(1)
		sem <- struct{}{}
		go func(m *internalregistry.ModelInfo) {
			defer wg.Done()
			defer func() { <-sem }()
			if codebuddyModelAvailable(creds, m.ID) {
				mu.Lock()
				out = append(out, m)
				mu.Unlock()
			}
		}(m)
	}
	wg.Wait()
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// codebuddyModelAvailable reports whether a model's inference route is live by
// sending a minimal chat request. It returns true on HTTP 2xx, on network
// errors, and on any business error other than 11102 (service info not found),
// so that only definitively decommissioned models are filtered out.
func codebuddyModelAvailable(creds codebuddyauth.Creds, modelID string) bool {
	payload, err := json.Marshal(map[string]any{
		"model":      modelID,
		"messages":   []map[string]string{{"role": "user", "content": "hi"}},
		"max_tokens": 1,
		"stream":     true,
	})
	if err != nil {
		return true
	}
	req, err := http.NewRequest(http.MethodPost, creds.ResolveBaseURL()+codebuddyauth.ChatPath, bytes.NewReader(payload))
	if err != nil {
		return true
	}
	codebuddyauth.ApplyHeaders(req, creds)
	req.Header.Set("Accept", "text/event-stream")

	client := &http.Client{Timeout: 15 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return true
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode >= 200 && resp.StatusCode < 300 {
		return true
	}
	b, _ := io.ReadAll(resp.Body)
	// 11102 = "service info not found" — the model's inference route is gone.
	return !bytes.Contains(b, []byte("11102"))
}

// handleCodebuddySyncModels 立即触发一次 CodeBuddy 模型同步，刷新 manifest 与
// /v1/models 响应。模型清单仅以腾讯后端为准（含 app.asar 未打包的新模型，
// 如 glm-5.3），不做 app.asar / 本地注册表回退。
func (s *relayServer) handleCodebuddySyncModels(c *gin.Context) {
	if _, ok := s.requireAPIKey(c); !ok {
		return
	}

	var synced []string
	source := "tencent-backend"

	// 仅从腾讯后端动态拉取（需要 CodeBuddy 账号 access_token）。
	if s.authManager != nil {
		synced = syncCodebuddyModelsFromBackend(s.authManager.List())
	}

	refreshed := false
	if s.manifest != nil && len(synced) > 0 {
		refreshed = s.manifest.setModelIDs(synced)
	}
	if refreshed {
		internalregistry.NotifyCodebuddyModelRefresh()
	}
	c.JSON(http.StatusOK, gin.H{
		"version":   1,
		"count":     len(synced),
		"refreshed": refreshed,
		"source":    source,
		"models":    synced,
	})
}

// handleCodebuddyImagesRelay relays an OpenAI Images API request to the
// CodeBuddy backend's dedicated image endpoint (/v2/images/generations or
// /v2/images/edits). That endpoint returns a single JSON document (not an SSE
// stream), so this path is non-streaming.
func (s *relayServer) handleCodebuddyImagesRelay(c *gin.Context, imageReq imageRelayRequest, requestedModel string) {
	body := imageReq.rawBody
	if len(body) == 0 {
		// Multipart edits have no raw JSON body; fall back to the Responses-style
		// body is not applicable to CodeBuddy, so surface a clear error instead.
		writeAPIError(c, http.StatusBadRequest, "multipart image edits are not supported for CodeBuddy; use the JSON images API", "unsupported_media_type")
		return
	}
	req, opts := buildExecutorRequest(c, body, requestedModel, sdktranslator.FromString("openai-image"), "", false)
	startedAt := time.Now()
	s.emitExecutorDiagnostic(c, "image_execute", requestedModel, "execute", startedAt, "codebuddy images endpoint")
	resp, err := s.runtime.Execute(relayContext(c), []string{"codebuddy"}, req, opts)
	if err != nil {
		s.writeExecutorError(c, err)
		return
	}
	writeUpstreamHeaders(c.Writer.Header(), resp.Headers)
	c.Data(http.StatusOK, "application/json", resp.Payload)
}
