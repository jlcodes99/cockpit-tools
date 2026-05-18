// Package messages 提供 Claude Messages API 的渠道管理
package messages

import (
	"fmt"
	"io"
	"log"
	"net/http"
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

// GetUpstreams 获取上游列表 (兼容前端 channels 字段名)
func GetUpstreams(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		cfg := cfgManager.GetConfig()

		upstreams := make([]gin.H, len(cfg.Upstream))
		for i, up := range cfg.Upstream {
			upstreams[i] = common.BuildChannelView(up, i)
		}

		c.JSON(200, gin.H{
			"channels": upstreams,
		})
	}
}

// AddUpstream 添加上游
func AddUpstream(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		var upstream config.UpstreamConfig
		if err := c.ShouldBindJSON(&upstream); err != nil {
			c.JSON(400, gin.H{"error": "Invalid request body"})
			return
		}

		if err := cfgManager.AddUpstream(upstream); err != nil {
			c.JSON(500, gin.H{"error": "Failed to save config"})
			return
		}

		c.JSON(200, gin.H{
			"message":  "上游已添加",
			"upstream": upstream,
		})
	}
}

// UpdateUpstream 更新上游
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
			c.JSON(400, gin.H{"error": "Invalid request body"})
			return
		}

		shouldResetMetrics, err := cfgManager.UpdateUpstream(id, updates)
		if err != nil {
			if strings.Contains(err.Error(), "无效的上游索引") {
				c.JSON(404, gin.H{"error": "Upstream not found"})
			} else {
				c.JSON(500, gin.H{"error": "Failed to save config"})
			}
			return
		}

		if shouldResetMetrics {
			sch.ResetChannelMetrics(id, scheduler.ChannelKindMessages)
		}

		cfg := cfgManager.GetConfig()
		c.JSON(200, gin.H{
			"message":  "上游已更新",
			"upstream": cfg.Upstream[id],
		})
	}
}

// DeleteUpstream 删除上游
func DeleteUpstream(cfgManager *config.ConfigManager, channelScheduler *scheduler.ChannelScheduler) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(400, gin.H{"error": "Invalid upstream ID"})
			return
		}

		removed, err := cfgManager.RemoveUpstream(id)
		if err != nil {
			if strings.Contains(err.Error(), "无效的上游索引") {
				c.JSON(404, gin.H{"error": "Upstream not found"})
			} else {
				c.JSON(500, gin.H{"error": "Failed to save config"})
			}
			return
		}

		channelScheduler.GetChannelLogStore(scheduler.ChannelKindMessages).RemoveAndShift(id)
		channelScheduler.DeleteChannelMetrics(removed, scheduler.ChannelKindMessages)

		c.JSON(200, gin.H{
			"message": "上游已删除",
			"removed": removed,
		})
	}
}

// AddApiKey 添加 API 密钥
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

		if err := cfgManager.AddAPIKey(id, req.APIKey); err != nil {
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

// DeleteApiKey 删除 API 密钥
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

		if err := cfgManager.RemoveAPIKey(id, apiKey); err != nil {
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

// MoveApiKeyToTop 将 API 密钥移到顶部
func MoveApiKeyToTop(cfgManager *config.ConfigManager) gin.HandlerFunc {
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

		if err := cfgManager.MoveAPIKeyToTop(id, apiKey); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}

		c.JSON(200, gin.H{"message": "API密钥已移到顶部"})
	}
}

// MoveApiKeyToBottom 将 API 密钥移到底部
func MoveApiKeyToBottom(cfgManager *config.ConfigManager) gin.HandlerFunc {
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

		if err := cfgManager.MoveAPIKeyToBottom(id, apiKey); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}

		c.JSON(200, gin.H{"message": "API密钥已移到底部"})
	}
}

// ReorderChannels 重新排序渠道
func ReorderChannels(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		var req struct {
			Order []int `json:"order"`
		}
		if err := c.ShouldBindJSON(&req); err != nil {
			c.JSON(400, gin.H{"error": "Invalid request body"})
			return
		}

		if err := cfgManager.ReorderUpstreams(req.Order); err != nil {
			c.JSON(400, gin.H{"error": err.Error()})
			return
		}

		c.JSON(200, gin.H{"message": "渠道顺序已更新"})
	}
}

// SetChannelStatus 设置渠道状态
func SetChannelStatus(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return handlers.NamedChannelStatusHandler(cfgManager, "渠道状态已更新")
}

// SetChannelPromotion 设置渠道促销期
// 促销期内的渠道会被优先选择，忽略 trace 亲和性
func SetChannelPromotion(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return handlers.SetChannelPromotion(cfgManager)
}

// PingChannel Ping单个渠道
func PingChannel(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		idStr := c.Param("id")
		id, err := strconv.Atoi(idStr)
		if err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid channel ID"})
			return
		}

		cfg := cfgManager.GetConfig()
		if id < 0 || id >= len(cfg.Upstream) {
			c.JSON(http.StatusNotFound, gin.H{"error": "Channel not found"})
			return
		}

		c.JSON(http.StatusOK, common.PingMultiBaseURLUpstream(cfg.Upstream[id], buildPingRequest))
	}
}

// PingAllChannels Ping所有渠道
func PingAllChannels(cfgManager *config.ConfigManager) gin.HandlerFunc {
	return func(c *gin.Context) {
		cfg := cfgManager.GetConfig()
		c.JSON(http.StatusOK, common.PingAllMultiBaseURLUpstreams(cfg.Upstream, buildPingRequest))
	}
}

func buildPingRequest(_ config.UpstreamConfig, baseURL string) (*http.Request, error) {
	return http.NewRequest(http.MethodHead, baseURL, nil)
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
				log.Printf("[Messages-Models] SSRF 防护拦截: %v", err)
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
			log.Printf("[Messages-Models] 使用临时 baseUrl: %s", baseURL)
		} else {
			// 编辑模式：从配置中读取渠道信息
			cfg := cfgManager.GetConfig()
			if id < 0 || id >= len(cfg.Upstream) {
				c.JSON(http.StatusNotFound, gin.H{"error": "Channel not found"})
				return
			}

			channel := cfg.Upstream[id]
			baseURL = channel.BaseURL
			channelName = channel.Name
			insecureSkipVerify = channel.InsecureSkipVerify
			proxyURL = channel.ProxyURL
			if req.BaseURL != "" {
				if err := utils.ValidateBaseURL(req.BaseURL); err != nil {
					log.Printf("[Messages-Models] SSRF 防护拦截: %v", err)
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

		log.Printf("[Messages-Models] 请求模型列表: channel=%s, key=%s", channelName, utils.MaskAPIKey(apiKey))

		// 5. 发起请求
		url := buildModelsURL(baseURL)
		client := httpclient.GetManager().GetStandardClient(10*time.Second, insecureSkipVerify, proxyURL)
		if req.BaseURL != "" && req.ProxyURL != "" {
			client = httpclient.GetManager().NewStandardClient(10*time.Second, insecureSkipVerify, proxyURL)
		}

		httpReq, err := http.NewRequestWithContext(c.Request.Context(), "GET", url, nil)
		if err != nil {
			log.Printf("[Messages-Models] 创建请求失败: channel=%s, url=%s, error=%v", channelName, url, err)
			c.JSON(http.StatusInternalServerError, gin.H{"error": fmt.Sprintf("Failed to create request: %v", err)})
			return
		}
		httpReq.Header.Set("Authorization", "Bearer "+apiKey)
		httpReq.Header.Set("Content-Type", "application/json")
		utils.ApplyCustomHeaders(httpReq.Header, req.CustomHeaders)

		resp, err := client.Do(httpReq)
		if err != nil {
			log.Printf("[Messages-Models] 请求失败: channel=%s, key=%s, url=%s, error=%v",
				channelName, utils.MaskAPIKey(apiKey), url, err)
			c.JSON(http.StatusBadGateway, gin.H{"error": fmt.Sprintf("Failed to fetch models: %v", err)})
			return
		}

		body, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			log.Printf("[Messages-Models] 读取响应失败: channel=%s, error=%v", channelName, err)
			c.JSON(http.StatusInternalServerError, gin.H{"error": fmt.Sprintf("Failed to read response: %v", err)})
			return
		}

		log.Printf("[Messages-Models] 上游响应: channel=%s, key=%s, status=%d, url=%s",
			channelName, utils.MaskAPIKey(apiKey), resp.StatusCode, url)
		// 包装上游 401 错误，避免前端误判为管理 API 认证失败
		if resp.StatusCode == 401 {
			c.JSON(http.StatusBadRequest, gin.H{
				"error":      "上游 API Key 无效",
				"statusCode": 401,
				"details":    string(body),
			})
			return
		}

		c.Data(resp.StatusCode, "application/json", body)
	}
}
