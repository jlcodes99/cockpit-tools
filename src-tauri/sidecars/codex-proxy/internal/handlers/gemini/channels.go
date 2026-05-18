// Package gemini 提供 Gemini API 的渠道管理
package gemini

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	handlers "github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/httpclient"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

// GetUpstreams 获取 Gemini 上游列表
func GetUpstreams(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		cfg := cfgManager.GetConfig()

		upstreams := make([]gin.H, len(cfg.GeminiUpstream))
		for i, up := range cfg.GeminiUpstream {
			upstreams[i] = common.BuildChannelView(up, i)
			upstreams[i]["injectDummyThoughtSignature"] = up.InjectDummyThoughtSignature
			upstreams[i]["stripThoughtSignature"] = up.StripThoughtSignature
		}

		c.JSON(200, gin.H{
			"channels": upstreams,
		})
	}
}

// AddUpstream 添加 Gemini 上游
func AddUpstream(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		var upstream config.UpstreamConfig
		if err := c.ShouldBindJSON(&upstream); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}

		if err := cfgManager.AddGeminiUpstream(upstream); err != nil {
			c.JSON(500, gin.H{"error": err.Error()})
			return
		}

		c.JSON(200, gin.H{"message": "Gemini upstream added successfully"})
	}
}

// UpdateUpstream 更新 Gemini 上游
func UpdateUpstream(cfgManager *config.ConfigManager, sch *scheduler.ChannelScheduler) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(400, gin.H{"error": "Invalid upstream ID"})
			return
		}

		var updates config.UpstreamUpdate
		if err := c.ShouldBindJSON(&updates); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}

		shouldResetMetrics, err := cfgManager.UpdateGeminiUpstream(id, updates)
		if err != nil {
			c.JSON(500, gin.H{"error": err.Error()})
			return
		}

		// 单 key 更换时重置熔断状态
		if shouldResetMetrics {
			sch.ResetChannelMetrics(id, scheduler.ChannelKindGemini)
		}

		c.JSON(200, gin.H{"message": "Gemini upstream updated successfully"})
	}
}

// DeleteUpstream 删除 Gemini 上游
func DeleteUpstream(cfgManager *config.ConfigManager, channelScheduler *scheduler.ChannelScheduler) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(400, gin.H{"error": "Invalid upstream ID"})
			return
		}

		removed, err := cfgManager.RemoveGeminiUpstream(id)
		if err != nil {
			if strings.Contains(err.Error(), "无效的") {
				c.JSON(404, gin.H{"error": "Upstream not found"})
			} else {
				c.JSON(500, gin.H{"error": err.Error()})
			}
			return
		}

		channelScheduler.GetChannelLogStore(scheduler.ChannelKindGemini).RemoveAndShift(id)
		channelScheduler.DeleteChannelMetrics(removed, scheduler.ChannelKindGemini)

		c.JSON(200, gin.H{"message": "Gemini upstream deleted successfully"})
	}
}

// AddApiKey 添加 Gemini 渠道 API 密钥
func AddApiKey(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(400, gin.H{"error": "Invalid upstream ID"})
			return
		}

		var req struct {
			APIKey string `json:"apiKey"`
		}
		if err := c.ShouldBindJSON(&req); err != nil {
			c.JSON(400, gin.H{"error": "Invalid request body"})
			return
		}

		if err := cfgManager.AddGeminiAPIKey(id, req.APIKey); err != nil {
			if strings.Contains(err.Error(), "无效的上游索引") {
				c.JSON(404, gin.H{"error": "Upstream not found"})
			} else if strings.Contains(err.Error(), "API密钥已存在") {
				c.JSON(400, gin.H{"error": "API密钥已存在"})
			} else {
				c.JSON(500, gin.H{"error": "Failed to save config"})
			}
			return
		}

		c.JSON(200, gin.H{
			"message": "API密钥已添加",
			"success": true,
		})
	}
}

// DeleteApiKey 删除 Gemini 渠道 API 密钥
func DeleteApiKey(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(400, gin.H{"error": "Invalid upstream ID"})
			return
		}

		apiKey := c.Param("apiKey")
		if apiKey == "" {
			c.JSON(400, gin.H{"error": "API key is required"})
			return
		}

		if err := cfgManager.RemoveGeminiAPIKey(id, apiKey); err != nil {
			if strings.Contains(err.Error(), "无效的上游索引") {
				c.JSON(404, gin.H{"error": "Upstream not found"})
			} else if strings.Contains(err.Error(), "API密钥不存在") {
				c.JSON(404, gin.H{"error": "API key not found"})
			} else {
				c.JSON(500, gin.H{"error": "Failed to save config"})
			}
			return
		}

		c.JSON(200, gin.H{
			"message": "API密钥已删除",
		})
	}
}

// MoveApiKeyToTop 将 Gemini 渠道 API 密钥移到最前面
func MoveApiKeyToTop(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		id, _ := strconv.Atoi(c.Param("id"))
		apiKey := c.Param("apiKey")

		if err := cfgManager.MoveGeminiAPIKeyToTop(id, apiKey); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}
		c.JSON(200, gin.H{"message": "API密钥已置顶"})
	}
}

// MoveApiKeyToBottom 将 Gemini 渠道 API 密钥移到最后面
func MoveApiKeyToBottom(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		id, _ := strconv.Atoi(c.Param("id"))
		apiKey := c.Param("apiKey")

		if err := cfgManager.MoveGeminiAPIKeyToBottom(id, apiKey); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}
		c.JSON(200, gin.H{"message": "API密钥已置底"})
	}
}

// ReorderChannels 重新排序 Gemini 渠道优先级
func ReorderChannels(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		var req struct {
			Order []int `json:"order"`
		}
		if err := c.ShouldBindJSON(&req); err != nil {
			c.JSON(400, gin.H{"error": "Invalid request body"})
			return
		}

		if err := cfgManager.ReorderGeminiUpstreams(req.Order); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}

		c.JSON(200, gin.H{
			"success": true,
			"message": "Gemini 渠道优先级已更新",
		})
	}
}

// SetChannelStatus 设置 Gemini 渠道状态
func SetChannelStatus(cfgManager *config.ConfigManager) gin.HandlerFunc {
	adapter := handlers.ChannelStatusConfigManagerFunc(func(index int, status string) error {
		return cfgManager.SetGeminiChannelStatus(index, status)
	})
	return handlers.NamedChannelStatusHandler(adapter, "Gemini 渠道状态已更新")
}

// SetChannelPromotion 设置 Gemini 渠道促销期
func SetChannelPromotion(cfgManager *config.ConfigManager) gin.HandlerFunc {
	adapter := handlers.PromotionConfigManagerFunc(func(index int, duration time.Duration) error {
		return cfgManager.SetGeminiChannelPromotion(index, duration)
	})
	return handlers.NamedChannelPromotionHandler(adapter, "Invalid channel ID", "Invalid request body", "Gemini 渠道促销期已清除", "Gemini 渠道促销期已设置")
}

// PingChannel 测试 Gemini 渠道连通性
func PingChannel(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(400, gin.H{"error": "Invalid channel ID"})
			return
		}

		cfg := cfgManager.GetConfig()
		if id < 0 || id >= len(cfg.GeminiUpstream) {
			c.JSON(404, gin.H{"error": "Channel not found"})
			return
		}

		c.JSON(200, common.PingSingleBaseURLUpstream(cfg.GeminiUpstream[id], buildPingRequest))
	}
}

// PingAllChannels 测试所有 Gemini 渠道连通性
func PingAllChannels(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		cfg := cfgManager.GetConfig()
		c.JSON(200, gin.H{"channels": common.PingAllSingleBaseURLUpstreams(cfg.GeminiUpstream, buildPingRequest, true)["channels"]})
	}
}

func buildPingRequest(upstream config.UpstreamConfig, baseURL string) (*http.Request, error) {
	req, _ := http.NewRequest(http.MethodGet, buildModelsURL(baseURL), nil)
	if len(upstream.APIKeys) > 0 {
		req.Header.Set("x-goog-api-key", upstream.APIKeys[0])
	}
	return req, nil
}

// buildEndpointURL 构建带版本前缀的端点 URL
func buildEndpointURL(baseURL, versionPrefix, endpoint string) string {
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

// buildModelsURL 构建 models 端点的 URL（Gemini 使用 v1beta）
func buildModelsURL(baseURL string) string {
	return buildEndpointURL(baseURL, "/v1beta", "/models")
}

// GetModelsRequest 获取模型列表的请求体
type GetModelsRequest struct {
	Key                string            `json:"key"`
	BaseURL            string            `json:"baseUrl"`
	BaseURLs           []string          `json:"baseUrls"`
	ProxyURL           string            `json:"proxyUrl"`
	InsecureSkipVerify *bool             `json:"insecureSkipVerify"`
	CustomHeaders      map[string]string `json:"customHeaders"`
}

// GetChannelModels 获取指定渠道的模型列表（支持临时 Key）
func GetChannelModels(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		// 1. 解析渠道 ID
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid channel ID"})
			return
		}

		// 2. 从请求体读取参数
		var req GetModelsRequest
		if err := c.ShouldBindJSON(&req); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request body"})
			return
		}

		// 3. 获取 baseUrl（优先使用请求体中的临时 baseUrl，用于新增渠道场景）
		var baseURL string
		var channelName string
		var insecureSkipVerify bool
		var proxyURL string

		if req.BaseURL != "" {
			// 新增模式：使用临时 baseUrl
			// SSRF 防护：验证用户提供的 baseURL
			if err := utils.ValidateBaseURL(req.BaseURL); err != nil {
				log.Printf("[Gemini-Models] SSRF 防护拦截: %v", err)
				c.JSON(http.StatusBadRequest, gin.H{"error": fmt.Sprintf("无效的 baseUrl: %v", err)})
				return
			}
			baseURL = req.BaseURL
			channelName = "临时渠道"
			insecureSkipVerify = false
			proxyURL = ""
			if req.InsecureSkipVerify != nil {
				insecureSkipVerify = *req.InsecureSkipVerify
			}
			if req.ProxyURL != "" {
				proxyURL = req.ProxyURL
			}
			log.Printf("[Gemini-Models] 使用临时 baseUrl: %s", baseURL)
		} else {
			// 编辑模式：从配置中读取渠道信息
			cfg := cfgManager.GetConfig()
			if id < 0 || id >= len(cfg.GeminiUpstream) {
				c.JSON(http.StatusNotFound, gin.H{"error": "Channel not found"})
				return
			}

			channel := cfg.GeminiUpstream[id]
			baseURL = channel.BaseURL
			channelName = channel.Name
			insecureSkipVerify = channel.InsecureSkipVerify
			proxyURL = channel.ProxyURL
			if req.BaseURL != "" {
				if err := utils.ValidateBaseURL(req.BaseURL); err != nil {
					log.Printf("[Gemini-Models] SSRF 防护拦截: %v", err)
					c.JSON(http.StatusBadRequest, gin.H{"error": fmt.Sprintf("无效的 baseUrl: %v", err)})
					return
				}
				baseURL = req.BaseURL
			}
			if req.InsecureSkipVerify != nil {
				insecureSkipVerify = *req.InsecureSkipVerify
			}
			if req.ProxyURL != "" {
				proxyURL = req.ProxyURL
			}
		}

		// 4. 验证 API Key
		apiKey := req.Key
		if apiKey == "" {
			c.JSON(http.StatusBadRequest, gin.H{"error": "No API key provided"})
			return
		}

		log.Printf("[Gemini-Models] 请求模型列表: channel=%s, key=%s", channelName, utils.MaskAPIKey(apiKey))

		// 5. 发起请求
		url := buildModelsURL(baseURL)
		client := httpclient.GetManager().GetStandardClient(10*time.Second, insecureSkipVerify, proxyURL)
		if req.BaseURL != "" && req.ProxyURL != "" {
			client = httpclient.GetManager().NewStandardClient(10*time.Second, insecureSkipVerify, proxyURL)
		}

		httpReq, err := http.NewRequestWithContext(c.Request.Context(), "GET", url, nil)
		if err != nil {
			log.Printf("[Gemini-Models] 创建请求失败: channel=%s, url=%s, error=%v", channelName, url, err)
			c.JSON(http.StatusInternalServerError, gin.H{"error": fmt.Sprintf("Failed to create request: %v", err)})
			return
		}
		httpReq.Header.Set("x-goog-api-key", apiKey)
		httpReq.Header.Set("Content-Type", "application/json")
		utils.ApplyCustomHeaders(httpReq.Header, req.CustomHeaders)

		resp, err := client.Do(httpReq)
		if err != nil {
			log.Printf("[Gemini-Models] 请求失败: channel=%s, key=%s, url=%s, error=%v",
				channelName, utils.MaskAPIKey(apiKey), url, err)
			c.JSON(http.StatusBadGateway, gin.H{"error": fmt.Sprintf("Failed to fetch models: %v", err)})
			return
		}

		body, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			log.Printf("[Gemini-Models] 读取响应失败: channel=%s, error=%v", channelName, err)
			c.JSON(http.StatusInternalServerError, gin.H{"error": fmt.Sprintf("Failed to read response: %v", err)})
			return
		}

		log.Printf("[Gemini-Models] 上游响应: channel=%s, key=%s, status=%d, url=%s",
			channelName, utils.MaskAPIKey(apiKey), resp.StatusCode, url)

		// 401 包装返回，避免前端误判为管理 API 认证失败
		if resp.StatusCode == 401 {
			c.JSON(http.StatusBadRequest, gin.H{
				"error":      "上游 API Key 无效",
				"statusCode": 401,
				"details":    string(body),
			})
			return
		}

		// 非 200 直接透传
		if resp.StatusCode != http.StatusOK {
			c.Data(resp.StatusCode, "application/json", body)
			return
		}

		// Gemini 返回 {"models": [{"name": "models/gemini-...", ...}]}
		// 转换为 OpenAI 兼容格式 {"object": "list", "data": [{"id": "gemini-...", ...}]}
		// 先检测是否包含 "models" 字段，若无则透传原始响应（如分页空结果或非标准格式）
		var raw map[string]json.RawMessage
		if err := json.Unmarshal(body, &raw); err != nil || raw["models"] == nil {
			if err != nil {
				log.Printf("[Gemini-Models] 响应格式解析失败，透传原始响应: %v", err)
			} else {
				log.Printf("[Gemini-Models] 响应无 models 字段，透传原始响应")
			}
			c.Data(http.StatusOK, "application/json", body)
			return
		}

		var geminiResp struct {
			Models []struct {
				Name string `json:"name"`
			} `json:"models"`
		}
		if err := json.Unmarshal(body, &geminiResp); err != nil {
			log.Printf("[Gemini-Models] 响应结构解析失败，透传原始响应: %v", err)
			c.Data(http.StatusOK, "application/json", body)
			return
		}

		type modelEntry struct {
			ID     string `json:"id"`
			Object string `json:"object"`
		}
		entries := make([]modelEntry, 0, len(geminiResp.Models))
		for _, m := range geminiResp.Models {
			// name 格式为 "models/gemini-1.5-pro"，取 "/" 后的部分作为 id
			id := m.Name
			if idx := strings.LastIndex(m.Name, "/"); idx >= 0 {
				id = m.Name[idx+1:]
			}
			entries = append(entries, modelEntry{ID: id, Object: "model"})
		}

		converted, err := json.Marshal(map[string]any{
			"object": "list",
			"data":   entries,
		})
		if err != nil {
			c.Data(http.StatusOK, "application/json", body)
			return
		}
		c.Data(http.StatusOK, "application/json", converted)
	}
}
