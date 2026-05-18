// Package responses 提供 Responses API 的处理器
package responses

import (
	"bytes"
	"encoding/json"
	"io"
	"log"
	"net/http"
	"regexp"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/middleware"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

// compactError 封装 compact 请求错误
type compactError struct {
	status         int
	body           []byte
	shouldFailover bool
	err            error
}

func (e *compactError) errorInfo() string {
	if e == nil {
		return ""
	}
	if len(e.body) > 0 {
		return strings.TrimSpace(string(e.body))
	}
	if e.err != nil {
		return e.err.Error()
	}
	if e.status != 0 {
		return http.StatusText(e.status)
	}
	return ""
}

// CompactHandler Responses API compact 端点处理器
// POST /v1/responses/compact - 压缩对话上下文，用于长期代理工作流
func CompactHandler(
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	_ *session.SessionManager,
	channelScheduler *scheduler.ChannelScheduler,
) gin.HandlerFunc {
	return gin.HandlerFunc(func(c *gin.Context) {
		// 认证
		middleware.ProxyAuthMiddleware(envCfg)(c)
		if c.IsAborted() {
			return
		}

		// 读取请求体
		maxBodySize := envCfg.MaxRequestBodySize
		bodyBytes, err := common.ReadRequestBody(c, maxBodySize)
		if err != nil {
			return
		}

		// 提取对话标识用于 Trace 亲和性
		userID := common.ExtractConversationID(c, bodyBytes)

		// 检查是否为多渠道模式
		isMultiChannel := channelScheduler.IsMultiChannelMode(scheduler.ChannelKindResponses)

		if isMultiChannel {
			handleMultiChannelCompact(c, envCfg, cfgManager, channelScheduler, bodyBytes, userID)
		} else {
			handleSingleChannelCompact(c, envCfg, cfgManager, channelScheduler, bodyBytes)
		}
	})
}

// handleSingleChannelCompact 单渠道 compact 请求（带 key 轮转）
func handleSingleChannelCompact(
	c *gin.Context,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	bodyBytes []byte,
) {
	upstream, channelIndex, err := cfgManager.GetCurrentResponsesUpstreamWithIndex()
	if err != nil {
		c.JSON(503, gin.H{"error": "未配置任何 Responses 渠道"})
		return
	}

	if len(upstream.APIKeys) == 0 {
		c.JSON(503, gin.H{"error": "当前渠道未配置 API 密钥"})
		return
	}

	requestModel := extractCompactRequestModel(bodyBytes)
	channelLogStore := channelScheduler.GetChannelLogStore(scheduler.ChannelKindResponses)
	metricsServiceType := scheduler.NormalizedMetricsServiceType(scheduler.ChannelKindResponses, upstream.ServiceType)

	// Key 轮转：尝试所有可用 key
	failedKeys := make(map[string]bool)
	var lastErr *compactError

	for attempt := 0; attempt < len(upstream.APIKeys); attempt++ {
		apiKey, err := cfgManager.GetNextResponsesAPIKey(upstream, failedKeys)
		if err != nil {
			break
		}

		attemptStart := time.Now()
		success, compactErr := tryCompactWithKey(c, upstream, apiKey, bodyBytes, envCfg, cfgManager)
		if success {
			common.RecordChannelLog(channelLogStore, channelIndex, requestModel, "", http.StatusOK, time.Since(attemptStart).Milliseconds(), true, apiKey, upstream.BaseURL, "", "Responses", attempt > 0)
			channelScheduler.RecordSuccessWithUsage(upstream.BaseURL, apiKey, metricsServiceType, nil, scheduler.ChannelKindResponses)
			return
		}

		if compactErr != nil {
			lastErr = compactErr
			if compactErr.shouldFailover {
				failedKeys[apiKey] = true
				cfgManager.MarkKeyAsFailed(apiKey, "Responses")
				channelScheduler.RecordFailure(upstream.BaseURL, apiKey, metricsServiceType, scheduler.ChannelKindResponses)
				common.RecordChannelLog(channelLogStore, channelIndex, requestModel, "", compactErr.status, time.Since(attemptStart).Milliseconds(), false, apiKey, upstream.BaseURL, compactErr.errorInfo(), "Responses", attempt > 0)
				continue
			}
			// 非故障转移错误，直接返回
			common.RecordChannelLog(channelLogStore, channelIndex, requestModel, "", compactErr.status, time.Since(attemptStart).Milliseconds(), false, apiKey, upstream.BaseURL, compactErr.errorInfo(), "Responses", attempt > 0)
			c.Data(compactErr.status, "application/json", compactErr.body)
			return
		}
	}

	// 所有 key 都失败
	if cfgManager.GetFuzzyModeEnabled() {
		c.JSON(503, gin.H{
			"type": "error",
			"error": gin.H{
				"type":    "service_unavailable",
				"message": "All upstream channels are currently unavailable",
			},
		})
		return
	}

	if lastErr != nil {
		c.Data(lastErr.status, "application/json", lastErr.body)
	} else {
		c.JSON(503, gin.H{"error": "所有 API 密钥都不可用"})
	}
}

// handleMultiChannelCompact 多渠道 compact 请求（带故障转移和亲和性）
func handleMultiChannelCompact(
	c *gin.Context,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	bodyBytes []byte,
	userID string,
) {
	failedChannels := make(map[int]bool)
	maxAttempts := channelScheduler.GetActiveChannelCount(scheduler.ChannelKindResponses)
	var lastErr *compactError
	var selectionErr error
	requestModel := extractCompactRequestModel(bodyBytes)
	channelLogStore := channelScheduler.GetChannelLogStore(scheduler.ChannelKindResponses)

	for attempt := 0; attempt < maxAttempts; attempt++ {
		selection, err := channelScheduler.SelectChannel(c.Request.Context(), userID, failedChannels, scheduler.ChannelKindResponses, requestModel, c.Param("routePrefix"), c.GetHeader("X-Channel"))
		if err != nil {
			selectionErr = err
			break
		}

		upstream := selection.Upstream
		channelIndex := selection.ChannelIndex

		// 每个渠道尝试所有 key
		success, successKey, compactErr := tryCompactChannelWithAllKeys(c, upstream, channelIndex, requestModel, cfgManager, channelScheduler, channelLogStore, bodyBytes, envCfg)
		if success {
			// 只有真正成功的请求才设置 Trace 亲和
			if successKey != "" {
				channelScheduler.SetTraceAffinity(userID, channelIndex, scheduler.ChannelKindResponses)
			}
			return
		}

		failedChannels[channelIndex] = true
		if compactErr != nil {
			lastErr = compactErr
		}
	}

	// 所有渠道都失败
	if cfgManager.GetFuzzyModeEnabled() {
		c.JSON(503, gin.H{
			"type": "error",
			"error": gin.H{
				"type":    "service_unavailable",
				"message": "All upstream channels are currently unavailable",
			},
		})
		return
	}

	if lastErr != nil {
		c.Data(lastErr.status, "application/json", lastErr.body)
	} else if selectionErr != nil {
		c.JSON(503, gin.H{"error": selectionErr.Error()})
	} else {
		c.JSON(503, gin.H{"error": "所有 Responses 渠道都不可用"})
	}
}

// tryCompactChannelWithAllKeys 尝试渠道的所有 key
func tryCompactChannelWithAllKeys(
	c *gin.Context,
	upstream *config.UpstreamConfig,
	channelIndex int,
	requestModel string,
	cfgManager *config.ConfigManager,
	channelScheduler *scheduler.ChannelScheduler,
	channelLogStore *metrics.ChannelLogStore,
	bodyBytes []byte,
	envCfg *config.EnvConfig,
) (bool, string, *compactError) {
	if len(upstream.APIKeys) == 0 {
		return false, "", nil
	}

	metricsServiceType := scheduler.NormalizedMetricsServiceType(scheduler.ChannelKindResponses, upstream.ServiceType)
	metricsManager := channelScheduler.GetResponsesMetricsManager()

	failedKeys := make(map[string]bool)
	probeAcquired := make(map[string]bool)
	var lastErr *compactError

	for attempt := 0; attempt < len(upstream.APIKeys); attempt++ {
		apiKey, err := cfgManager.GetNextResponsesAPIKey(upstream, failedKeys)
		if err != nil {
			break
		}

		// 检查熔断状态
		circuitState := metricsManager.GetKeyCircuitState(upstream.BaseURL, apiKey, metricsServiceType)
		if circuitState == metrics.CircuitStateOpen {
			failedKeys[apiKey] = true
			log.Printf("[Compact-Circuit] 跳过 open 状态中的 Key: %s", utils.MaskAPIKey(apiKey))
			continue
		}
		if circuitState == metrics.CircuitStateHalfOpen {
			probeKey := upstream.BaseURL + "|" + apiKey
			if !metricsManager.TryAcquireProbe(upstream.BaseURL, apiKey, metricsServiceType) {
				failedKeys[apiKey] = true
				log.Printf("[Compact-Circuit] 跳过 half-open 探针已占用的 Key: %s", utils.MaskAPIKey(apiKey))
				continue
			}
			probeAcquired[probeKey] = true
			log.Printf("[Compact-Circuit] 使用 half-open 探针 Key: %s", utils.MaskAPIKey(apiKey))
		}

		attemptStart := time.Now()
		success, compactErr := tryCompactWithKey(c, upstream, apiKey, bodyBytes, envCfg, cfgManager)

		if success {
			common.RecordChannelLog(channelLogStore, channelIndex, requestModel, "", http.StatusOK, time.Since(attemptStart).Milliseconds(), true, apiKey, upstream.BaseURL, "", "Responses", attempt > 0)
			channelScheduler.RecordSuccessWithUsage(upstream.BaseURL, apiKey, metricsServiceType, nil, scheduler.ChannelKindResponses)
			// 释放探针
			probeKey := upstream.BaseURL + "|" + apiKey
			if probeAcquired[probeKey] {
				metricsManager.ReleaseProbe(upstream.BaseURL, apiKey, metricsServiceType)
				delete(probeAcquired, probeKey)
			}
			return true, apiKey, nil
		}

		if compactErr != nil {
			lastErr = compactErr
			if compactErr.shouldFailover {
				failedKeys[apiKey] = true
				cfgManager.MarkKeyAsFailed(apiKey, "Responses")
				channelScheduler.RecordFailure(upstream.BaseURL, apiKey, metricsServiceType, scheduler.ChannelKindResponses)
				common.RecordChannelLog(channelLogStore, channelIndex, requestModel, "", compactErr.status, time.Since(attemptStart).Milliseconds(), false, apiKey, upstream.BaseURL, compactErr.errorInfo(), "Responses", attempt > 0)
				// 释放探针
				probeKey := upstream.BaseURL + "|" + apiKey
				if probeAcquired[probeKey] {
					metricsManager.ReleaseProbe(upstream.BaseURL, apiKey, metricsServiceType)
					delete(probeAcquired, probeKey)
				}
				continue
			}
			// 非故障转移错误，返回但标记渠道成功（请求已处理）
			common.RecordChannelLog(channelLogStore, channelIndex, requestModel, "", compactErr.status, time.Since(attemptStart).Milliseconds(), false, apiKey, upstream.BaseURL, compactErr.errorInfo(), "Responses", attempt > 0)
			// 释放探针
			probeKey := upstream.BaseURL + "|" + apiKey
			if probeAcquired[probeKey] {
				metricsManager.ReleaseProbe(upstream.BaseURL, apiKey, metricsServiceType)
				delete(probeAcquired, probeKey)
			}
			c.Data(compactErr.status, "application/json", compactErr.body)
			return true, "", nil
		}
	}

	return false, "", lastErr
}

// tryCompactWithKey 使用单个 key 尝试 compact 请求
func tryCompactWithKey(
	c *gin.Context,
	upstream *config.UpstreamConfig,
	apiKey string,
	bodyBytes []byte,
	envCfg *config.EnvConfig,
	cfgManager *config.ConfigManager,
) (bool, *compactError) {
	targetURL := buildCompactURL(upstream)
	req, err := http.NewRequestWithContext(c.Request.Context(), "POST", targetURL, bytes.NewReader(bodyBytes))
	if err != nil {
		return false, &compactError{status: 500, body: []byte(`{"error":"创建请求失败"}`), shouldFailover: true, err: err}
	}

	req.Header = utils.PrepareUpstreamHeaders(c, req.URL.Host)
	req.Header.Del("authorization")
	req.Header.Del("x-api-key")
	utils.SetAuthenticationHeader(req.Header, apiKey)
	req.Header.Set("Content-Type", "application/json")
	utils.ApplyCustomHeaders(req.Header, upstream.CustomHeaders)

	resp, err := common.SendRequest(req, upstream, envCfg, false, "Responses")
	if err != nil {
		return false, &compactError{status: 502, body: []byte(`{"error":"上游请求失败"}`), shouldFailover: true, err: err}
	}
	defer resp.Body.Close()

	respBody, _ := io.ReadAll(resp.Body)
	respBody = utils.DecompressGzipIfNeeded(resp, respBody)

	// 判断是否需要故障转移
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		shouldFailover, _ := common.ShouldRetryWithNextKey(resp.StatusCode, respBody, cfgManager.GetFuzzyModeEnabled(), "Responses")
		return false, &compactError{status: resp.StatusCode, body: respBody, shouldFailover: shouldFailover}
	}

	// 成功
	utils.ForwardResponseHeaders(resp.Header, c.Writer)
	c.Data(resp.StatusCode, "application/json", respBody)
	return true, nil
}

func extractCompactRequestModel(bodyBytes []byte) string {
	if len(bodyBytes) == 0 {
		return ""
	}

	var req types.ResponsesRequest
	if err := json.Unmarshal(bodyBytes, &req); err != nil {
		return ""
	}
	return req.Model
}

// buildCompactURL 构建 compact 端点 URL
func buildCompactURL(upstream *config.UpstreamConfig) string {
	baseURL := strings.TrimSuffix(upstream.BaseURL, "/")
	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
	if versionPattern.MatchString(baseURL) {
		return baseURL + "/responses/compact"
	}
	return baseURL + "/v1/responses/compact"
}
