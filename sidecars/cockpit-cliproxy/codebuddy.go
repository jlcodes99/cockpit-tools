package main

import (
	"encoding/json"
	"net/http"
	"strings"
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
			ID   string   `json:"id"`
			Name string   `json:"name"`
			Tags []string `json:"tags"`
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
	ids := make([]string, 0, len(envelope.Data.Models))
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
		ids = append(ids, m.ID)
	}

	// 安装到 registry（去重、排序、变更检测、刷新通知）。
	return internalregistry.InstallCodebuddyModelIDs(ids)
}

// handleCodebuddySyncModels 立即触发一次 CodeBuddy 模型同步，刷新 manifest 与
// /v1/models 响应。优先从腾讯官方后端拉取实时模型清单（含 app.asar 未打包的
// 新模型，如 glm-5.3）；后端不可用时回退到 app.asar 静态提取。
func (s *relayServer) handleCodebuddySyncModels(c *gin.Context) {
	if _, ok := s.requireAPIKey(c); !ok {
		return
	}

	var synced []string
	source := "official-client-asar"

	// 1) 优先：从腾讯后端动态拉取（需要 CodeBuddy 账号 access_token）。
	if s.authManager != nil {
		if ids := syncCodebuddyModelsFromBackend(s.authManager.List()); len(ids) > 0 {
			synced = ids
			source = "tencent-backend"
		}
	}
	// 2) 回退：从本机官方客户端 app.asar 静态提取。
	if len(synced) == 0 {
		synced = internalregistry.SyncCodebuddyModelsFromOfficialClient()
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
