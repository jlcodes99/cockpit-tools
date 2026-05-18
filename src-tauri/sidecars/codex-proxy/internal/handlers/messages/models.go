// Package messages 提供 Claude Messages API 的处理器
package messages

import (
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"
	"regexp"
	"sort"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/httpclient"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/middleware"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

const modelsRequestTimeout = 30 * time.Second

var errNoChannelWithDisabledKeys = errors.New("no channel with disabled keys")

// ModelsResponse OpenAI 兼容的 models 响应格式
type ModelsResponse struct {
	Object string       `json:"object"`
	Data   []ModelEntry `json:"data"`
}

// ModelEntry 单个模型条目
type ModelEntry struct {
	ID      string `json:"id"`
	Object  string `json:"object"`
	Created int64  `json:"created"`
	OwnedBy string `json:"owned_by"`
}

// ModelsHandler 处理 /v1/models 请求，从 Messages、Responses 和 Chat 渠道获取并合并模型列表
func ModelsHandler(envCfg *config.EnvConfig, cfgManager *config.ConfigManager, channelScheduler *scheduler.ChannelScheduler) gin.HandlerFunc {
	return func(c *gin.Context) {
		middleware.ProxyAuthMiddleware(envCfg)(c)
		if c.IsAborted() {
			return
		}

		messagesModels := fetchModelsFromChannels(c, cfgManager, channelScheduler, scheduler.ChannelKindMessages)
		responsesModels := fetchModelsFromChannels(c, cfgManager, channelScheduler, scheduler.ChannelKindResponses)
		chatModels := fetchModelsFromChannels(c, cfgManager, channelScheduler, scheduler.ChannelKindChat)
		geminiModels := fetchModelsFromChannels(c, cfgManager, channelScheduler, scheduler.ChannelKindGemini)

		mergedModels := mergeModels(messagesModels, responsesModels, chatModels, geminiModels)

		if len(mergedModels) == 0 {
			c.JSON(http.StatusNotFound, gin.H{
				"error": gin.H{
					"message": "models endpoint not available from any upstream",
					"type":    "not_found_error",
				},
			})
			return
		}

		response := ModelsResponse{
			Object: "list",
			Data:   mergedModels,
		}

		log.Printf("[Models] 合并完成: messages=%d, responses=%d, chat=%d, gemini=%d, merged=%d",
			len(messagesModels), len(responsesModels), len(chatModels), len(geminiModels), len(mergedModels))

		c.JSON(http.StatusOK, response)
	}
}

// ModelsDetailHandler 处理 /v1/models/:model 请求，转发到上游
func ModelsDetailHandler(envCfg *config.EnvConfig, cfgManager *config.ConfigManager, channelScheduler *scheduler.ChannelScheduler) gin.HandlerFunc {
	return func(c *gin.Context) {
		middleware.ProxyAuthMiddleware(envCfg)(c)
		if c.IsAborted() {
			return
		}

		modelID := c.Param("model")
		if modelID == "" {
			c.JSON(http.StatusBadRequest, gin.H{
				"error": gin.H{
					"message": "model id is required",
					"type":    "invalid_request_error",
				},
			})
			return
		}

		for _, kind := range []scheduler.ChannelKind{
			scheduler.ChannelKindMessages,
			scheduler.ChannelKindResponses,
			scheduler.ChannelKindChat,
			scheduler.ChannelKindGemini,
		} {
			if body, ok := tryModelsRequest(c, cfgManager, channelScheduler, "GET", "/"+modelID, kind); ok {
				c.Data(http.StatusOK, "application/json", body)
				return
			}
		}

		c.JSON(http.StatusNotFound, gin.H{
			"error": gin.H{
				"message": "model not found",
				"type":    "not_found_error",
			},
		})
	}
}

// fetchModelsFromChannels 从指定类型的渠道获取模型列表
func fetchModelsFromChannels(c *gin.Context, cfgManager *config.ConfigManager, channelScheduler *scheduler.ChannelScheduler, kind scheduler.ChannelKind) []ModelEntry {
	body, ok := tryModelsRequest(c, cfgManager, channelScheduler, "GET", "", kind)
	if !ok {
		return nil
	}

	// Gemini 渠道或 serviceType=gemini 的渠道返回 {"models": [...]} 格式
	if kind == scheduler.ChannelKindGemini {
		return parseGeminiModelsResponse(body)
	}

	// 尝试 OpenAI 格式解析
	var resp ModelsResponse
	if err := json.Unmarshal(body, &resp); err != nil {
		log.Printf("[%s-Models] 解析渠道响应失败: %v", channelKindLabel(kind), err)
		return nil
	}

	// 如果 data 为空，尝试 Gemini 格式（Responses 渠道中 serviceType=gemini 的情况）
	if len(resp.Data) == 0 {
		if geminiModels := parseGeminiModelsResponse(body); len(geminiModels) > 0 {
			return geminiModels
		}
	}

	return resp.Data
}

// parseGeminiModelsResponse 解析 Gemini 格式的模型列表响应
func parseGeminiModelsResponse(body []byte) []ModelEntry {
	var geminiResp struct {
		Models []struct {
			Name string `json:"name"`
		} `json:"models"`
	}
	if err := json.Unmarshal(body, &geminiResp); err != nil {
		log.Printf("[Gemini-Models] 解析响应失败: %v", err)
		return nil
	}

	entries := make([]ModelEntry, 0, len(geminiResp.Models))
	for _, m := range geminiResp.Models {
		id := m.Name
		if idx := strings.LastIndex(m.Name, "/"); idx >= 0 {
			id = m.Name[idx+1:]
		}
		entries = append(entries, ModelEntry{ID: id, Object: "model"})
	}
	return entries
}

// mergeModels 合并多个模型列表并去重（按 ID）
func mergeModels(modelLists ...[]ModelEntry) []ModelEntry {
	seen := make(map[string]bool)
	var result []ModelEntry

	for _, models := range modelLists {
		for _, m := range models {
			if !seen[m.ID] {
				seen[m.ID] = true
				result = append(result, m)
			}
		}
	}

	return result
}

// tryModelsRequest 使用调度器选择渠道，按故障转移顺序尝试请求 models 端点
func tryModelsRequest(c *gin.Context, cfgManager *config.ConfigManager, channelScheduler *scheduler.ChannelScheduler, method, suffix string, kind scheduler.ChannelKind) ([]byte, bool) {
	failedChannels := make(map[int]bool)
	maxChannelRetries := 10 // 最多尝试 10 个渠道
	channelType := channelKindLabel(kind)

	for attempt := 0; attempt < maxChannelRetries; attempt++ {
		selection, err := channelScheduler.SelectChannel(c.Request.Context(), "", failedChannels, kind, "", c.Param("routePrefix"), c.GetHeader("X-Channel"))
		if err != nil {
			fallbackSelection, fallbackErr := selectChannelWithDisabledKeys(cfgManager, failedChannels, kind, c.Param("routePrefix"))
			if fallbackErr != nil {
				log.Printf("[%s-Models] 渠道无可用: %v", channelType, err)
				break
			}
			selection = fallbackSelection
			log.Printf("[%s-Models] 活跃渠道不可用，回退到挂起渠道查询模型: channel=%s, reason=%s", channelType, selection.Upstream.Name, selection.Reason)
		}

		upstream := selection.Upstream

		var url string
		if upstream.ServiceType == "gemini" || kind == scheduler.ChannelKindGemini {
			url = buildGeminiModelsURL(upstream.BaseURL) + suffix
		} else {
			url = buildModelsURL(upstream.BaseURL) + suffix
		}
		client := httpclient.GetManager().GetStandardClient(modelsRequestTimeout, upstream.InsecureSkipVerify, upstream.ProxyURL)

		apiKey, usedDisabledFallback, err := cfgManager.GetAdminAPIKey(upstream, nil, channelType)
		if err != nil {
			log.Printf("[%s-Models] 获取 API Key 失败: channel=%s, error=%v", channelType, upstream.Name, err)
			failedChannels[selection.ChannelIndex] = true
			continue
		}
		if usedDisabledFallback {
			log.Printf("[%s-Models] 使用已拉黑密钥查询模型列表: channel=%s, key=%s", channelType, upstream.Name, utils.MaskAPIKey(apiKey))
		}

		req, err := http.NewRequestWithContext(c.Request.Context(), method, url, nil)
		if err != nil {
			log.Printf("[%s-Models] 创建请求失败: channel=%s, url=%s, error=%v", channelType, upstream.Name, url, err)
			failedChannels[selection.ChannelIndex] = true
			continue
		}
		if upstream.ServiceType == "gemini" || kind == scheduler.ChannelKindGemini {
			utils.SetGeminiAuthenticationHeader(req.Header, apiKey)
		} else {
			utils.SetAuthenticationHeader(req.Header, apiKey)
		}
		req.Header.Set("Content-Type", "application/json")
		utils.ApplyCustomHeaders(req.Header, upstream.CustomHeaders)

		resp, err := client.Do(req)
		if err != nil {
			log.Printf("[%s-Models] 请求失败: channel=%s, key=%s, url=%s, error=%v",
				channelType, upstream.Name, utils.MaskAPIKey(apiKey), url, err)
			failedChannels[selection.ChannelIndex] = true
			continue
		}

		if resp.StatusCode == http.StatusOK {
			body, err := io.ReadAll(resp.Body)
			resp.Body.Close()
			if err != nil {
				log.Printf("[%s-Models] 读取响应失败: channel=%s, error=%v", channelType, upstream.Name, err)
				failedChannels[selection.ChannelIndex] = true
				continue
			}
			log.Printf("[%s-Models] 请求成功: method=%s, channel=%s, key=%s, url=%s, reason=%s",
				channelType, method, upstream.Name, utils.MaskAPIKey(apiKey), url, selection.Reason)
			return body, true
		}

		log.Printf("[%s-Models] 上游返回非 200: channel=%s, key=%s, status=%d, url=%s",
			channelType, upstream.Name, utils.MaskAPIKey(apiKey), resp.StatusCode, url)
		resp.Body.Close()
		failedChannels[selection.ChannelIndex] = true
	}

	log.Printf("[%s-Models] 所有渠道均失败: method=%s, suffix=%s", channelType, method, suffix)
	return nil, false
}

func channelKindLabel(kind scheduler.ChannelKind) string {
	switch kind {
	case scheduler.ChannelKindResponses:
		return "Responses"
	case scheduler.ChannelKindChat:
		return "Chat"
	case scheduler.ChannelKindGemini:
		return "Gemini"
	default:
		return "Messages"
	}
}

func selectChannelWithDisabledKeys(cfgManager *config.ConfigManager, failedChannels map[int]bool, kind scheduler.ChannelKind, routePrefix string) (*scheduler.SelectionResult, error) {
	cfg := cfgManager.GetConfig()

	var upstreams []config.UpstreamConfig
	switch kind {
	case scheduler.ChannelKindResponses:
		upstreams = cfg.ResponsesUpstream
	case scheduler.ChannelKindGemini:
		upstreams = cfg.GeminiUpstream
	case scheduler.ChannelKindChat:
		upstreams = cfg.ChatUpstream
	case scheduler.ChannelKindImages:
		upstreams = cfg.ImagesUpstream
	default:
		upstreams = cfg.Upstream
	}

	type candidate struct {
		index    int
		upstream config.UpstreamConfig
		priority int
	}

	candidates := make([]candidate, 0)
	for i, upstream := range upstreams {
		if failedChannels[i] {
			continue
		}
		if config.GetChannelStatus(&upstream) == "disabled" {
			continue
		}
		if len(upstream.APIKeys) > 0 || len(upstream.DisabledAPIKeys) == 0 {
			continue
		}
		if routePrefix != "" {
			if upstream.RoutePrefix != routePrefix {
				continue
			}
		} else if upstream.RoutePrefix != "" {
			continue
		}
		candidates = append(candidates, candidate{
			index:    i,
			upstream: upstream,
			priority: config.GetChannelPriority(&upstream, i),
		})
	}

	if len(candidates) == 0 {
		return nil, errNoChannelWithDisabledKeys
	}

	sort.Slice(candidates, func(i, j int) bool {
		return candidates[i].priority < candidates[j].priority
	})

	selected := candidates[0]
	upstreamCopy := selected.upstream
	return &scheduler.SelectionResult{
		Upstream:     &upstreamCopy,
		ChannelIndex: selected.index,
		Reason:       "disabled_key_fallback",
	}, nil
}

// buildModelsURL 构建 models 端点的 URL
func buildModelsURL(baseURL string) string {
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	if skipVersionPrefix {
		baseURL = strings.TrimSuffix(baseURL, "#")
	}
	baseURL = strings.TrimSuffix(baseURL, "/")

	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
	hasVersionSuffix := versionPattern.MatchString(baseURL)

	endpoint := "/models"
	if !hasVersionSuffix && !skipVersionPrefix {
		endpoint = "/v1" + endpoint
	}

	return baseURL + endpoint
}

// buildGeminiModelsURL 构建 Gemini models 端点的 URL（使用 v1beta 前缀）
func buildGeminiModelsURL(baseURL string) string {
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	if skipVersionPrefix {
		baseURL = strings.TrimSuffix(baseURL, "#")
	}
	baseURL = strings.TrimSuffix(baseURL, "/")

	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
	hasVersionSuffix := versionPattern.MatchString(baseURL)

	endpoint := "/models"
	if !hasVersionSuffix && !skipVersionPrefix {
		endpoint = "/v1beta" + endpoint
	}

	return baseURL + endpoint
}
