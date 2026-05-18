package config

import (
	"fmt"
	"log"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/fsnotify/fsnotify"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/statelog"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
)

// ============== 核心类型定义 ==============

// UpstreamConfig 上游配置
type UpstreamConfig struct {
	BaseURL              string            `json:"baseUrl"`
	BaseURLs             []string          `json:"baseUrls,omitempty"` // 多 BaseURL 支持（failover 模式）
	APIKeys              []string          `json:"apiKeys"`
	HistoricalAPIKeys    []string          `json:"historicalApiKeys,omitempty"` // 历史 API Key（用于统计聚合，换 Key 后保留旧 Key 的统计数据）
	DisabledAPIKeys      []DisabledKeyInfo `json:"disabledApiKeys,omitempty"`   // 被拉黑的 API Key（持久化，需手动恢复）
	ServiceType          string            `json:"serviceType"`                 // gemini, openai, claude
	Name                 string            `json:"name,omitempty"`
	Description          string            `json:"description,omitempty"`
	Website              string            `json:"website,omitempty"`
	InsecureSkipVerify   bool              `json:"insecureSkipVerify,omitempty"`
	ModelMapping         map[string]string `json:"modelMapping,omitempty"`
	ModelMappingExplicit map[string]string `json:"modelMappingExplicit,omitempty"`
	ReasoningMapping     map[string]string `json:"reasoningMapping,omitempty"`
	ReasoningParamStyle  string            `json:"reasoningParamStyle,omitempty"`
	TextVerbosity        string            `json:"textVerbosity,omitempty"`
	FastMode             bool              `json:"fastMode,omitempty"`
	// OpenAI Chat 上游配置：启用后将非标准 Chat role 改写为 user（默认 false）
	NormalizeNonstandardChatRoles bool `json:"normalizeNonstandardChatRoles,omitempty"`
	// Codex 工具兼容开关（默认 false）。
	// 透传分支中将 Codex 原生工具转换为 OpenAI function 格式（默认 false）。
	CodexNativeToolPassthrough bool  `json:"codexNativeToolPassthrough,omitempty"`
	CodexToolCompat            *bool `json:"codexToolCompat,omitempty"`
	// Deprecated: 使用 codexToolCompat；保留旧字段仅用于配置读取和旧前端写入兼容。
	StripCodexClientTools bool `json:"stripCodexClientTools,omitempty"`
	// 多渠道调度相关字段
	Priority       int        `json:"priority"`                 // 渠道优先级（数字越小优先级越高，默认按索引）
	Status         string     `json:"status"`                   // 渠道状态：active（正常）, suspended（暂停）, disabled（备用池）
	PromotionUntil *time.Time `json:"promotionUntil,omitempty"` // 促销期截止时间，在此期间内优先使用此渠道（忽略trace亲和）
	LowQuality     bool       `json:"lowQuality,omitempty"`     // 低质量渠道标记：启用后强制本地估算 token，偏差>5%时使用本地值
	// 自动拉黑开关
	AutoBlacklistBalance *bool `json:"autoBlacklistBalance,omitempty"` // 余额不足时自动拉黑 Key（默认 true）
	// metadata.user_id 规范化开关
	NormalizeMetadataUserID *bool `json:"normalizeMetadataUserId,omitempty"` // 规范化 metadata.user_id（默认 true）
	// Gemini 特定配置
	InjectDummyThoughtSignature bool `json:"injectDummyThoughtSignature,omitempty"` // 给空 thought_signature 注入 dummy 值（兼容 x666.me 等要求必须有该字段的 API）
	StripThoughtSignature       bool `json:"stripThoughtSignature,omitempty"`       // 移除 thought_signature 字段（兼容旧版 Gemini API）
	// Claude 协议 thinking 回传配置
	PassbackReasoningContent bool `json:"passbackReasoningContent,omitempty"` // 将 thinking 块转为 reasoning_content 回传（兼容 mimo 等要求 OpenAI 风格 reasoning_content 的 Claude 协议上游）
	// 自定义请求头
	CustomHeaders map[string]string `json:"customHeaders,omitempty"` // 自定义请求头（覆盖或添加到上游请求）
	// 渠道级代理
	ProxyURL string `json:"proxyUrl,omitempty"` // HTTP/HTTPS/SOCKS5 代理地址
	// 模型白名单
	SupportedModels []string `json:"supportedModels,omitempty"` // 支持的模型白名单（空=全部）；支持精确匹配，以及 prefix* / *suffix / *contains* 形式的包含与排除规则（排除用 ! 前缀）
	// 路由前缀
	RoutePrefix string `json:"routePrefix,omitempty"` // 路由前缀（如 "kimi"），客户端可通过 /:routePrefix/v1/messages 访问
	// Vision 能力配置
	NoVision            bool     `json:"noVision,omitempty"`            // 整个渠道不支持图片输入
	NoVisionModels      []string `json:"noVisionModels,omitempty"`      // 不支持图片输入的模型列表（匹配 modelMapping 后的实际模型名）
	VisionFallbackModel string   `json:"visionFallbackModel,omitempty"` // 含图请求命中 noVisionModels 时使用的替代模型
}

// DisabledKeyInfo 被拉黑的 API Key 信息
type DisabledKeyInfo struct {
	Key        string `json:"key"`
	Reason     string `json:"reason"`              // "authentication_error" / "permission_error" / "insufficient_balance"
	Message    string `json:"message"`             // 原始错误信息
	DisabledAt string `json:"disabledAt"`          // ISO8601 时间戳
	RecoverAt  string `json:"recoverAt,omitempty"` // 自动恢复时间（可选）
}

// IsAutoRecoverableDisabledReason 判断是否属于可自动恢复的拉黑原因。
func IsAutoRecoverableDisabledReason(reason string) bool {
	reason = strings.ToLower(strings.TrimSpace(reason))
	switch reason {
	case "insufficient_balance", "insufficient_quota", "billing_error", "quota":
		return true
	default:
		return false
	}
}

// IsAutoBlacklistBalanceEnabled 检查余额不足自动拉黑是否启用（默认 true）
func (u *UpstreamConfig) IsAutoBlacklistBalanceEnabled() bool {
	if u.AutoBlacklistBalance == nil {
		return true
	}
	return *u.AutoBlacklistBalance
}

// IsNormalizeMetadataUserIDEnabled 检查 metadata.user_id 规范化是否启用（默认 true）
func (u *UpstreamConfig) IsNormalizeMetadataUserIDEnabled() bool {
	if u.NormalizeMetadataUserID == nil {
		return true
	}
	return *u.NormalizeMetadataUserID
}

// IsCodexToolCompatEnabled 检查 Codex 工具兼容是否启用（默认 false）。
func (u *UpstreamConfig) IsCodexToolCompatEnabled() bool {
	if u.CodexToolCompat != nil {
		return *u.CodexToolCompat
	}
	return u.StripCodexClientTools
}

// ShouldOmitVisionContentForCompatibility returns true for OpenAI-compatible
// text-only upstreams that reject image_url content blocks in chat messages.
func ShouldOmitVisionContentForCompatibility(upstream *UpstreamConfig, model string) bool {
	if upstream == nil {
		return false
	}
	redirectedModel := RedirectModel(model, upstream)
	if upstream.NoVision {
		return true
	}
	for _, noVisionModel := range upstream.NoVisionModels {
		if noVisionModel == redirectedModel {
			return true
		}
	}

	serviceType := strings.ToLower(strings.TrimSpace(upstream.ServiceType))
	if serviceType != "" && serviceType != "openai" && serviceType != "responses" {
		return false
	}
	textOnlyHints := []string{
		strings.ToLower(upstream.Name),
		strings.ToLower(upstream.BaseURL),
		strings.ToLower(redirectedModel),
	}
	for _, hint := range textOnlyHints {
		if strings.Contains(hint, "deepseek") {
			return true
		}
	}
	return false
}

// UpstreamUpdate 用于部分更新 UpstreamConfig
type UpstreamUpdate struct {
	Name                          *string           `json:"name"`
	ServiceType                   *string           `json:"serviceType"`
	BaseURL                       *string           `json:"baseUrl"`
	BaseURLs                      []string          `json:"baseUrls"`
	APIKeys                       []string          `json:"apiKeys"`
	Description                   *string           `json:"description"`
	Website                       *string           `json:"website"`
	InsecureSkipVerify            *bool             `json:"insecureSkipVerify"`
	ModelMapping                  map[string]string `json:"modelMapping"`
	ModelMappingExplicit          map[string]string `json:"modelMappingExplicit"`
	ReasoningMapping              map[string]string `json:"reasoningMapping"`
	ReasoningParamStyle           *string           `json:"reasoningParamStyle"`
	TextVerbosity                 *string           `json:"textVerbosity"`
	FastMode                      *bool             `json:"fastMode"`
	NormalizeNonstandardChatRoles *bool             `json:"normalizeNonstandardChatRoles"`
	CodexNativeToolPassthrough    *bool             `json:"codexNativeToolPassthrough"`
	CodexToolCompat               *bool             `json:"codexToolCompat"`
	StripCodexClientTools         *bool             `json:"stripCodexClientTools"`
	// 多渠道调度相关字段
	Priority                *int       `json:"priority"`
	Status                  *string    `json:"status"`
	PromotionUntil          *time.Time `json:"promotionUntil"`
	LowQuality              *bool      `json:"lowQuality"`
	AutoBlacklistBalance    *bool      `json:"autoBlacklistBalance"`
	NormalizeMetadataUserID *bool      `json:"normalizeMetadataUserId"`
	// Gemini 特定配置
	InjectDummyThoughtSignature *bool `json:"injectDummyThoughtSignature"`
	StripThoughtSignature       *bool `json:"stripThoughtSignature"`
	PassbackReasoningContent    *bool `json:"passbackReasoningContent"`
	// 自定义请求头
	CustomHeaders map[string]string `json:"customHeaders"`
	// 渠道级代理
	ProxyURL *string `json:"proxyUrl"`
	// 模型白名单
	SupportedModels []string `json:"supportedModels"` // 支持的模型白名单（空=全部）；支持精确匹配，以及 prefix* / *suffix / *contains* 形式的包含与排除规则（排除用 ! 前缀）
	// 路由前缀
	RoutePrefix *string `json:"routePrefix"` // 路由前缀（如 "kimi"）
	// Vision 能力配置
	NoVision            *bool    `json:"noVision"`
	NoVisionModels      []string `json:"noVisionModels"`
	VisionFallbackModel *string  `json:"visionFallbackModel"`
}

// Config 配置结构
type Config struct {
	Upstream        []UpstreamConfig `json:"upstream"`
	CurrentUpstream int              `json:"currentUpstream,omitempty"` // 已废弃：旧格式兼容用

	// Responses 接口专用配置（独立于 /v1/messages）
	ResponsesUpstream        []UpstreamConfig `json:"responsesUpstream"`
	CurrentResponsesUpstream int              `json:"currentResponsesUpstream,omitempty"` // 已废弃：旧格式兼容用

	// Gemini 接口专用配置（独立于 /v1/messages 和 /v1/responses）
	GeminiUpstream []UpstreamConfig `json:"geminiUpstream"`

	// Chat Completions 接口专用配置（OpenAI /v1/chat/completions 兼容）
	ChatUpstream []UpstreamConfig `json:"chatUpstream,omitempty"`

	// Images 接口专用配置（OpenAI /v1/images/generations 兼容）
	ImagesUpstream []UpstreamConfig `json:"imagesUpstream,omitempty"`

	// Fuzzy 模式：启用时模糊处理错误，所有非 2xx 错误都尝试 failover
	FuzzyModeEnabled bool `json:"fuzzyModeEnabled"`

	// 移除计费头中的 cch= 参数：启用时自动从 system 数组中移除 cch=xxx; 部分
	StripBillingHeader bool `json:"stripBillingHeader"`
}

// FailedKey 失败密钥记录
type FailedKey struct {
	Timestamp    time.Time
	FailureCount int
}

// ConfigManager 配置管理器
type ConfigManager struct {
	mu              sync.RWMutex
	config          Config
	configFile      string
	watcher         *fsnotify.Watcher
	failedKeysCache map[string]*FailedKey
	keyRecoveryTime time.Duration
	maxFailureCount int
	stopChan        chan struct{} // 用于通知 goroutine 停止
	closeOnce       sync.Once     // 确保 Close 只执行一次
}

// failedKeyCacheKey 构造 FailedKeysCache 的复合键（apiType:apiKey）
func failedKeyCacheKey(apiType, apiKey string) string {
	return apiType + ":" + apiKey
}

// ============== 核心共享方法 ==============

// GetConfig 获取配置（返回深拷贝，确保并发安全）
func (cm *ConfigManager) GetConfig() Config {
	cm.mu.RLock()
	defer cm.mu.RUnlock()

	// 深拷贝整个 Config 结构体
	cloned := cm.config

	// 深拷贝 Upstream slice
	if cm.config.Upstream != nil {
		cloned.Upstream = make([]UpstreamConfig, len(cm.config.Upstream))
		for i := range cm.config.Upstream {
			cloned.Upstream[i] = *cm.config.Upstream[i].Clone()
		}
	}

	// 深拷贝 ResponsesUpstream slice
	if cm.config.ResponsesUpstream != nil {
		cloned.ResponsesUpstream = make([]UpstreamConfig, len(cm.config.ResponsesUpstream))
		for i := range cm.config.ResponsesUpstream {
			cloned.ResponsesUpstream[i] = *cm.config.ResponsesUpstream[i].Clone()
		}
	}

	// 深拷贝 GeminiUpstream slice
	if cm.config.GeminiUpstream != nil {
		cloned.GeminiUpstream = make([]UpstreamConfig, len(cm.config.GeminiUpstream))
		for i := range cm.config.GeminiUpstream {
			cloned.GeminiUpstream[i] = *cm.config.GeminiUpstream[i].Clone()
		}
	}

	// 深拷贝 ChatUpstream slice
	if len(cm.config.ChatUpstream) > 0 {
		cloned.ChatUpstream = make([]UpstreamConfig, len(cm.config.ChatUpstream))
		for i := range cm.config.ChatUpstream {
			cloned.ChatUpstream[i] = *cm.config.ChatUpstream[i].Clone()
		}
	}

	// 深拷贝 ImagesUpstream slice
	if len(cm.config.ImagesUpstream) > 0 {
		cloned.ImagesUpstream = make([]UpstreamConfig, len(cm.config.ImagesUpstream))
		for i := range cm.config.ImagesUpstream {
			cloned.ImagesUpstream[i] = *cm.config.ImagesUpstream[i].Clone()
		}
	}

	return cloned
}

// GetNextAPIKey 获取下一个 API 密钥（纯 failover 模式）
// apiType: 接口类型（Messages/Responses/Gemini），用于日志标签前缀
func (cm *ConfigManager) GetNextAPIKey(upstream *UpstreamConfig, failedKeys map[string]bool, apiType string) (string, error) {
	if len(upstream.APIKeys) == 0 {
		return "", fmt.Errorf("上游 %s 没有可用的API密钥", upstream.Name)
	}

	// 单 Key 直接返回
	if len(upstream.APIKeys) == 1 {
		return upstream.APIKeys[0], nil
	}

	// 筛选可用密钥：排除临时失败密钥和内存中的失败密钥
	availableKeys := []string{}
	for _, key := range upstream.APIKeys {
		if !failedKeys[key] && !cm.isKeyFailed(key, apiType) {
			availableKeys = append(availableKeys, key)
		}
	}

	if len(availableKeys) == 0 {
		// 如果所有密钥都失效，尝试选择失败时间最早的密钥（恢复尝试）
		var oldestFailedKey string
		oldestTime := time.Now()

		cm.mu.RLock()
		for _, key := range upstream.APIKeys {
			if !failedKeys[key] { // 排除本次请求已经尝试过的密钥
				cacheKey := failedKeyCacheKey(apiType, key)
				if failure, exists := cm.failedKeysCache[cacheKey]; exists {
					if failure.Timestamp.Before(oldestTime) {
						oldestTime = failure.Timestamp
						oldestFailedKey = key
					}
				}
			}
		}
		cm.mu.RUnlock()

		if oldestFailedKey != "" {
			log.Printf("[%s-Key] 警告: 所有密钥都失效，尝试最早失败的密钥: %s", apiType, utils.MaskAPIKey(oldestFailedKey))
			return oldestFailedKey, nil
		}

		return "", fmt.Errorf("上游 %s 的所有API密钥都暂时不可用", upstream.Name)
	}

	// 纯 failover：按优先级顺序选择第一个可用密钥
	selectedKey := availableKeys[0]
	// 获取该密钥在原始列表中的索引
	keyIndex := 0
	for i, key := range upstream.APIKeys {
		if key == selectedKey {
			keyIndex = i + 1
			break
		}
	}
	log.Printf("[%s-Key] 故障转移选择密钥 %s (%d/%d)", apiType, utils.MaskAPIKey(selectedKey), keyIndex, len(upstream.APIKeys))
	return selectedKey, nil
}

// GetAdminAPIKey 获取管理/探测场景下的 API 密钥。
// 优先使用活跃 APIKeys；若活跃密钥不可用，则临时借用 DisabledAPIKeys 中的密钥。
// 返回值 fallback=true 表示本次借用了已拉黑密钥。
func (cm *ConfigManager) GetAdminAPIKey(upstream *UpstreamConfig, failedKeys map[string]bool, apiType string) (apiKey string, fallback bool, err error) {
	apiKey, err = cm.GetNextAPIKey(upstream, failedKeys, apiType)
	if err == nil {
		return apiKey, false, nil
	}

	for _, disabledKey := range upstream.DisabledAPIKeys {
		if failedKeys[disabledKey.Key] {
			continue
		}
		log.Printf("[%s-Key] 警告: 活跃密钥不可用，临时借用已拉黑密钥用于管理操作: %s", apiType, utils.MaskAPIKey(disabledKey.Key))
		return disabledKey.Key, true, nil
	}

	return "", false, err
}

// MarkKeyAsFailed 标记密钥失败
// apiType: 接口类型（Messages/Responses/Gemini/Chat），用于日志标签前缀和缓存键隔离
func (cm *ConfigManager) MarkKeyAsFailed(apiKey string, apiType string) {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	cacheKey := failedKeyCacheKey(apiType, apiKey)
	if failure, exists := cm.failedKeysCache[cacheKey]; exists {
		failure.FailureCount++
		failure.Timestamp = time.Now()
	} else {
		cm.failedKeysCache[cacheKey] = &FailedKey{
			Timestamp:    time.Now(),
			FailureCount: 1,
		}
	}

	failure := cm.failedKeysCache[cacheKey]
	recoveryTime := cm.keyRecoveryTime
	if failure.FailureCount > cm.maxFailureCount {
		recoveryTime = cm.keyRecoveryTime * 2
	}

	log.Printf("[%s-Key] 标记API密钥失败: %s (失败次数: %d, 恢复时间: %v)",
		apiType, utils.MaskAPIKey(apiKey), failure.FailureCount, recoveryTime)
}

// isKeyFailed 检查密钥是否失败
func (cm *ConfigManager) isKeyFailed(apiKey, apiType string) bool {
	cm.mu.RLock()
	defer cm.mu.RUnlock()

	cacheKey := failedKeyCacheKey(apiType, apiKey)
	failure, exists := cm.failedKeysCache[cacheKey]
	if !exists {
		return false
	}

	recoveryTime := cm.keyRecoveryTime
	if failure.FailureCount > cm.maxFailureCount {
		recoveryTime = cm.keyRecoveryTime * 2
	}

	return time.Since(failure.Timestamp) < recoveryTime
}

// IsKeyFailed 检查 Key 是否在冷却期（公开方法）
func (cm *ConfigManager) IsKeyFailed(apiKey, apiType string) bool {
	return cm.isKeyFailed(apiKey, apiType)
}

// clearFailedKeysForUpstream 清理指定渠道的所有失败 key 记录
// 当渠道被删除时调用，避免内存泄漏和冷却状态残留
// apiType: 接口类型（Messages/Responses/Gemini/Chat），用于日志标签前缀和缓存键隔离
func (cm *ConfigManager) clearFailedKeysForUpstream(upstream *UpstreamConfig, apiType string) {
	for _, key := range upstream.APIKeys {
		cacheKey := failedKeyCacheKey(apiType, key)
		if _, exists := cm.failedKeysCache[cacheKey]; exists {
			delete(cm.failedKeysCache, cacheKey)
			log.Printf("[%s-Key] 已清理被删除渠道 %s 的失败密钥记录: %s", apiType, upstream.Name, utils.MaskAPIKey(key))
		}
	}
}

// cleanupExpiredFailures 清理过期的失败记录
func (cm *ConfigManager) cleanupExpiredFailures() {
	ticker := time.NewTicker(1 * time.Minute)
	defer ticker.Stop()

	for {
		select {
		case <-cm.stopChan:
			return
		case <-ticker.C:
			cm.mu.Lock()
			now := time.Now()
			for key, failure := range cm.failedKeysCache {
				recoveryTime := cm.keyRecoveryTime
				if failure.FailureCount > cm.maxFailureCount {
					recoveryTime = cm.keyRecoveryTime * 2
				}

				if now.Sub(failure.Timestamp) > recoveryTime {
					delete(cm.failedKeysCache, key)
					log.Printf("[Config-Key] API密钥 %s 已从失败列表中恢复", utils.MaskAPIKey(key))
				}
			}
			cm.mu.Unlock()
		}
	}
}

// ============== Fuzzy 模式相关方法 ==============

// GetFuzzyModeEnabled 获取 Fuzzy 模式状态
func (cm *ConfigManager) GetFuzzyModeEnabled() bool {
	cm.mu.RLock()
	defer cm.mu.RUnlock()
	return cm.config.FuzzyModeEnabled
}

// SetFuzzyModeEnabled 设置 Fuzzy 模式状态
func (cm *ConfigManager) SetFuzzyModeEnabled(enabled bool) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	cm.config.FuzzyModeEnabled = enabled

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return err
	}

	status := "关闭"
	if enabled {
		status = "启用"
	}
	log.Printf("[Config-FuzzyMode] Fuzzy 模式已%s", status)
	return nil
}

// ============== StripBillingHeader 相关方法 ==============

// GetStripBillingHeader 获取移除计费头状态
func (cm *ConfigManager) GetStripBillingHeader() bool {
	cm.mu.RLock()
	defer cm.mu.RUnlock()
	return cm.config.StripBillingHeader
}

// SetStripBillingHeader 设置移除计费头状态
func (cm *ConfigManager) SetStripBillingHeader(enabled bool) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	cm.config.StripBillingHeader = enabled

	if err := cm.saveConfigLocked(cm.config); err != nil {
		return err
	}

	status := "关闭"
	if enabled {
		status = "启用"
	}
	log.Printf("[Config-StripBillingHeader] 移除计费头已%s", status)
	return nil
}

// ============== API Key 拉黑相关方法 ==============

// BlacklistKey 将指定 Key 从活跃列表移到拉黑列表（持久化）
// apiType: Messages/Responses/Gemini/Chat，用于定位 upstream slice
// channelIndex: 渠道在 upstream slice 中的索引
func (cm *ConfigManager) BlacklistKey(apiType string, channelIndex int, apiKey string, reason string, message string) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	upstreams := cm.getUpstreamSliceLocked(apiType)
	if upstreams == nil || channelIndex < 0 || channelIndex >= len(*upstreams) {
		return fmt.Errorf("无效的渠道索引: %s[%d]", apiType, channelIndex)
	}

	upstream := &(*upstreams)[channelIndex]

	// 检查 key 是否在活跃列表中
	keyIdx := -1
	for i, k := range upstream.APIKeys {
		if k == apiKey {
			keyIdx = i
			break
		}
	}
	if keyIdx == -1 {
		return nil // key 不在活跃列表，可能已被拉黑，忽略
	}

	// 从 APIKeys 中移除
	upstream.APIKeys = append(upstream.APIKeys[:keyIdx], upstream.APIKeys[keyIdx+1:]...)

	// 添加到 DisabledAPIKeys
	disabledAt := time.Now().Format(time.RFC3339)
	recoverAt := ""
	if IsAutoRecoverableDisabledReason(reason) {
		recoverAt = time.Now().Add(time.Hour).Format(time.RFC3339)
	}
	upstream.DisabledAPIKeys = append(upstream.DisabledAPIKeys, DisabledKeyInfo{
		Key:        apiKey,
		Reason:     reason,
		Message:    message,
		DisabledAt: disabledAt,
		RecoverAt:  recoverAt,
	})

	// 同时添加到 HistoricalAPIKeys（保留统计数据）
	if !slices.Contains(upstream.HistoricalAPIKeys, apiKey) {
		upstream.HistoricalAPIKeys = append(upstream.HistoricalAPIKeys, apiKey)
	}

	log.Printf("[%s-Blacklist] Key %s 已被拉黑 (原因: %s, 渠道: %s, 剩余Key: %d)",
		apiType, utils.MaskAPIKey(apiKey), reason, upstream.Name, len(upstream.APIKeys))
	statelog.LogStateTransition(apiType+"-Blacklist", "key", utils.MaskAPIKey(apiKey), "active", "disabled", reason, "channel="+upstream.Name)

	if len(upstream.APIKeys) == 0 {
		log.Printf("[%s-Blacklist] 警告: 渠道 %s 的所有 Key 都已被拉黑！", apiType, upstream.Name)
	}

	return cm.saveConfigLocked(cm.config)
}

// RestoreKey 将指定 Key 从拉黑列表恢复到活跃列表（持久化）
func (cm *ConfigManager) RestoreKey(apiType string, channelIndex int, apiKey string) error {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	upstreams := cm.getUpstreamSliceLocked(apiType)
	if upstreams == nil || channelIndex < 0 || channelIndex >= len(*upstreams) {
		return fmt.Errorf("无效的渠道索引: %s[%d]", apiType, channelIndex)
	}

	upstream := &(*upstreams)[channelIndex]

	// 查找并移除
	disabledIdx := -1
	for i, dk := range upstream.DisabledAPIKeys {
		if dk.Key == apiKey {
			disabledIdx = i
			break
		}
	}
	if disabledIdx == -1 {
		return fmt.Errorf("Key %s 不在拉黑列表中", utils.MaskAPIKey(apiKey))
	}

	upstream.DisabledAPIKeys = append(upstream.DisabledAPIKeys[:disabledIdx], upstream.DisabledAPIKeys[disabledIdx+1:]...)
	if !slices.Contains(upstream.APIKeys, apiKey) {
		upstream.APIKeys = append(upstream.APIKeys, apiKey)
	}

	// 从 HistoricalAPIKeys 移除，避免 active∩historical 重复导致统计重复计数
	upstream.HistoricalAPIKeys = slices.DeleteFunc(upstream.HistoricalAPIKeys, func(k string) bool {
		return k == apiKey
	})

	// 清除内存中的失败记录
	cacheKey := failedKeyCacheKey(apiType, apiKey)
	delete(cm.failedKeysCache, cacheKey)

	log.Printf("[%s-Blacklist] Key %s 已恢复 (渠道: %s)", apiType, utils.MaskAPIKey(apiKey), upstream.Name)
	statelog.LogStateTransition(apiType+"-Blacklist", "key", utils.MaskAPIKey(apiKey), "disabled", "active", "manual_restore", "channel="+upstream.Name)

	return cm.saveConfigLocked(cm.config)
}

// RestoreAllKeys 恢复指定渠道所有被拉黑的 Key（持久化）
// 返回恢复的 Key 数量
func (cm *ConfigManager) RestoreAllKeys(apiType string, channelIndex int) (int, error) {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	upstreams := cm.getUpstreamSliceLocked(apiType)
	if upstreams == nil || channelIndex < 0 || channelIndex >= len(*upstreams) {
		return 0, fmt.Errorf("无效的渠道索引: %s[%d]", apiType, channelIndex)
	}

	upstream := &(*upstreams)[channelIndex]
	restoredCount := len(upstream.DisabledAPIKeys)
	if restoredCount == 0 {
		return 0, nil
	}

	// 将所有被拉黑的 Key 移回活跃列表
	for _, dk := range upstream.DisabledAPIKeys {
		if !slices.Contains(upstream.APIKeys, dk.Key) {
			upstream.APIKeys = append(upstream.APIKeys, dk.Key)
		}
		// 从 HistoricalAPIKeys 移除，避免 active∩historical 重复
		upstream.HistoricalAPIKeys = slices.DeleteFunc(upstream.HistoricalAPIKeys, func(k string) bool {
			return k == dk.Key
		})
		// 清除内存中的失败记录
		cacheKey := failedKeyCacheKey(apiType, dk.Key)
		delete(cm.failedKeysCache, cacheKey)
	}

	log.Printf("[%s-Blacklist] 渠道 [%d] %s 的 %d 个 Key 已全部恢复", apiType, channelIndex, upstream.Name, restoredCount)
	upstream.DisabledAPIKeys = nil

	return restoredCount, cm.saveConfigLocked(cm.config)
}

// RestoreDisabledKeys 恢复指定渠道中命中的被拉黑 Key，并返回实际恢复的 key 列表。
func (cm *ConfigManager) RestoreDisabledKeys(apiType string, channelIndex int, keys []string) ([]string, error) {
	cm.mu.Lock()
	defer cm.mu.Unlock()

	upstreams := cm.getUpstreamSliceLocked(apiType)
	if upstreams == nil || channelIndex < 0 || channelIndex >= len(*upstreams) {
		return nil, fmt.Errorf("无效的渠道索引: %s[%d]", apiType, channelIndex)
	}
	if len(keys) == 0 {
		return nil, nil
	}

	keySet := make(map[string]struct{}, len(keys))
	for _, key := range keys {
		if key == "" {
			continue
		}
		keySet[key] = struct{}{}
	}
	if len(keySet) == 0 {
		return nil, nil
	}

	upstream := &(*upstreams)[channelIndex]
	restored := make([]string, 0, len(keySet))
	newDisabled := make([]DisabledKeyInfo, 0, len(upstream.DisabledAPIKeys))
	for _, dk := range upstream.DisabledAPIKeys {
		if _, ok := keySet[dk.Key]; !ok {
			newDisabled = append(newDisabled, dk)
			continue
		}
		if !slices.Contains(upstream.APIKeys, dk.Key) {
			upstream.APIKeys = append(upstream.APIKeys, dk.Key)
		}
		upstream.HistoricalAPIKeys = slices.DeleteFunc(upstream.HistoricalAPIKeys, func(k string) bool {
			return k == dk.Key
		})
		delete(cm.failedKeysCache, failedKeyCacheKey(apiType, dk.Key))
		restored = append(restored, dk.Key)
	}

	if len(restored) == 0 {
		return nil, nil
	}

	upstream.DisabledAPIKeys = newDisabled
	log.Printf("[%s-Blacklist] 渠道 [%d] %s 自动恢复了 %d 个 Key", apiType, channelIndex, upstream.Name, len(restored))
	if err := cm.saveConfigLocked(cm.config); err != nil {
		return nil, err
	}
	return restored, nil
}

// getUpstreamSliceLocked 根据 apiType 获取对应的 upstream slice 指针（调用方需持有锁）
func (cm *ConfigManager) getUpstreamSliceLocked(apiType string) *[]UpstreamConfig {
	switch apiType {
	case "Messages":
		return &cm.config.Upstream
	case "Responses":
		return &cm.config.ResponsesUpstream
	case "Gemini":
		return &cm.config.GeminiUpstream
	case "Chat":
		return &cm.config.ChatUpstream
	case "Images":
		return &cm.config.ImagesUpstream
	default:
		return nil
	}
}
