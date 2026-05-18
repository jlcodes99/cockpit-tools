package config

import (
	"sort"
	"strings"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
)

// ============== 工具函数 ==============

// deduplicateStrings 去重字符串切片，保持原始顺序
func deduplicateStrings(items []string) []string {
	if len(items) <= 1 {
		return items
	}
	seen := make(map[string]struct{}, len(items))
	result := make([]string, 0, len(items))
	for _, item := range items {
		if _, exists := seen[item]; !exists {
			seen[item] = struct{}{}
			result = append(result, item)
		}
	}
	return result
}

func normalizeUpstreamServiceType(serviceType, fallback string) string {
	trimmed := strings.TrimSpace(serviceType)
	if trimmed != "" {
		return trimmed
	}
	return fallback
}

// deduplicateBaseURLs 去重 BaseURLs，忽略尾部 / 和默认版本前缀差异，保留 # 语义。
func deduplicateBaseURLs(urls []string, serviceType string) []string {
	if len(urls) == 0 {
		return urls
	}
	seen := make(map[string]struct{}, len(urls))
	result := make([]string, 0, len(urls))
	for _, rawURL := range urls {
		canonical := utils.CanonicalBaseURL(rawURL, serviceType)
		if canonical == "" {
			continue
		}
		if _, exists := seen[canonical]; !exists {
			seen[canonical] = struct{}{}
			result = append(result, canonical)
		}
	}
	return result
}

// ConfigError 配置错误
type ConfigError struct {
	Message string
}

func (e *ConfigError) Error() string {
	return e.Message
}

// ============== 模型重定向 ==============

// RedirectModel 模型重定向
func RedirectModel(model string, upstream *UpstreamConfig) string {
	if upstream.ModelMapping == nil || len(upstream.ModelMapping) == 0 {
		return model
	}

	// 直接匹配（精确匹配优先）
	if mapped, ok := upstream.ModelMapping[model]; ok {
		return mapped
	}

	// 模糊匹配：按源模型长度从长到短排序，确保最长匹配优先
	type mapping struct {
		source string
		target string
	}
	mappings := make([]mapping, 0, len(upstream.ModelMapping))
	for source, target := range upstream.ModelMapping {
		mappings = append(mappings, mapping{source, target})
	}
	sort.Slice(mappings, func(i, j int) bool {
		return len(mappings[i].source) > len(mappings[j].source)
	})

	for _, m := range mappings {
		if strings.Contains(model, m.source) {
			return m.target
		}
	}

	return model
}

// ResolveReasoningEffort 根据原始模型名解析 reasoning effort
func ResolveReasoningEffort(model string, upstream *UpstreamConfig) string {
	if upstream == nil || upstream.ReasoningMapping == nil || len(upstream.ReasoningMapping) == 0 {
		return ""
	}
	if effort, ok := upstream.ReasoningMapping[model]; ok {
		return effort
	}
	type mapping struct {
		source string
		effort string
	}
	mappings := make([]mapping, 0, len(upstream.ReasoningMapping))
	for source, effort := range upstream.ReasoningMapping {
		mappings = append(mappings, mapping{source, effort})
	}
	sort.Slice(mappings, func(i, j int) bool {
		return len(mappings[i].source) > len(mappings[j].source)
	})
	for _, m := range mappings {
		if strings.Contains(model, m.source) {
			return m.effort
		}
	}
	return ""
}

// ============== 渠道状态与优先级辅助函数 ==============

// GetChannelStatus 获取渠道状态（带默认值处理）
func GetChannelStatus(upstream *UpstreamConfig) string {
	if upstream.Status == "" {
		return "active"
	}
	return upstream.Status
}

// GetChannelAdminState 获取渠道管理员配置状态。
func GetChannelAdminState(upstream *UpstreamConfig) string {
	return GetChannelStatus(upstream)
}

// GetChannelRuntimeState 获取渠道运行时状态视图（不依赖 metrics，仅反映配置侧可观察状态）。
func GetChannelRuntimeState(upstream *UpstreamConfig) string {
	if upstream == nil {
		return "unknown"
	}
	if len(upstream.DisabledAPIKeys) > 0 {
		return "disabled_keys_present"
	}
	if len(upstream.APIKeys) == 0 {
		return "no_active_keys"
	}
	return "ready"
}

// GetChannelEffectiveState 获取渠道当前有效状态视图。
func GetChannelEffectiveState(upstream *UpstreamConfig) string {
	if upstream == nil {
		return "unknown"
	}
	adminState := GetChannelAdminState(upstream)
	if adminState != "active" {
		return adminState
	}
	if len(upstream.APIKeys) == 0 {
		return "degraded"
	}
	return "active"
}

// applySingleKeyReplacementTransition 统一处理“单 key 更换”带来的自动激活与熔断重置判定。
func applySingleKeyReplacementTransition(upstream *UpstreamConfig, newKeys []string) (shouldResetMetrics bool) {
	if upstream == nil {
		return false
	}
	if len(upstream.APIKeys) == 1 && len(newKeys) == 1 && upstream.APIKeys[0] != newKeys[0] {
		if upstream.Status == "suspended" {
			upstream.Status = "active"
		}
		return true
	}
	return false
}

// GetChannelPriority 获取渠道优先级（带默认值处理）
func GetChannelPriority(upstream *UpstreamConfig, index int) int {
	if upstream.Priority == 0 {
		return index
	}
	return upstream.Priority
}

// IsChannelInPromotion 检查渠道是否处于促销期
func IsChannelInPromotion(upstream *UpstreamConfig) bool {
	if upstream.PromotionUntil == nil {
		return false
	}
	return time.Now().Before(*upstream.PromotionUntil)
}

// ============== UpstreamConfig 方法 ==============

// Clone 深拷贝 UpstreamConfig（用于避免并发修改问题）
// 在多 BaseURL failover 场景下，需要临时修改 BaseURL 字段，
// 使用深拷贝可避免并发请求之间的竞态条件
func (u *UpstreamConfig) Clone() *UpstreamConfig {
	cloned := *u // 浅拷贝

	// 深拷贝切片字段
	if u.BaseURLs != nil {
		cloned.BaseURLs = make([]string, len(u.BaseURLs))
		copy(cloned.BaseURLs, u.BaseURLs)
	}
	if u.APIKeys != nil {
		cloned.APIKeys = make([]string, len(u.APIKeys))
		copy(cloned.APIKeys, u.APIKeys)
	}
	if u.HistoricalAPIKeys != nil {
		cloned.HistoricalAPIKeys = make([]string, len(u.HistoricalAPIKeys))
		copy(cloned.HistoricalAPIKeys, u.HistoricalAPIKeys)
	}
	if u.ModelMapping != nil {
		cloned.ModelMapping = make(map[string]string, len(u.ModelMapping))
		for k, v := range u.ModelMapping {
			cloned.ModelMapping[k] = v
		}
	}
	if u.ModelMappingExplicit != nil {
		cloned.ModelMappingExplicit = make(map[string]string, len(u.ModelMappingExplicit))
		for k, v := range u.ModelMappingExplicit {
			cloned.ModelMappingExplicit[k] = v
		}
	}
	if u.CustomHeaders != nil {
		cloned.CustomHeaders = make(map[string]string, len(u.CustomHeaders))
		for k, v := range u.CustomHeaders {
			cloned.CustomHeaders[k] = v
		}
	}
	if u.PromotionUntil != nil {
		t := *u.PromotionUntil
		cloned.PromotionUntil = &t
	}
	if u.SupportedModels != nil {
		cloned.SupportedModels = make([]string, len(u.SupportedModels))
		copy(cloned.SupportedModels, u.SupportedModels)
	}
	if u.DisabledAPIKeys != nil {
		cloned.DisabledAPIKeys = make([]DisabledKeyInfo, len(u.DisabledAPIKeys))
		copy(cloned.DisabledAPIKeys, u.DisabledAPIKeys)
	}
	if u.AutoBlacklistBalance != nil {
		v := *u.AutoBlacklistBalance
		cloned.AutoBlacklistBalance = &v
	}
	if u.NormalizeMetadataUserID != nil {
		v := *u.NormalizeMetadataUserID
		cloned.NormalizeMetadataUserID = &v
	}
	if u.CodexToolCompat != nil {
		v := *u.CodexToolCompat
		cloned.CodexToolCompat = &v
	}
	if u.NoVisionModels != nil {
		cloned.NoVisionModels = make([]string, len(u.NoVisionModels))
		copy(cloned.NoVisionModels, u.NoVisionModels)
	}

	return &cloned
}

// SupportsModel 检查渠道是否支持指定模型
// 空列表表示支持所有模型；支持精确匹配，以及 prefix* / *suffix / *contains* 形式的包含与排除规则。
func (u *UpstreamConfig) SupportsModel(model string) bool {
	supported, _ := u.ExplainModelSupport(model)
	return supported
}

// ExplainModelSupport 返回渠道是否支持指定模型，以及不支持时的原因。
func (u *UpstreamConfig) ExplainModelSupport(model string) (bool, string) {
	if len(u.SupportedModels) == 0 {
		return true, ""
	}

	includes, excludes := splitSupportedModelRules(u.SupportedModels)
	for _, pattern := range excludes {
		if matchSupportedModelPattern(pattern, model) {
			return false, "命中排除规则 !" + pattern
		}
	}
	if len(includes) == 0 {
		return true, ""
	}
	for _, pattern := range includes {
		if matchSupportedModelPattern(pattern, model) {
			return true, ""
		}
	}
	return false, "未命中包含规则"
}

func splitSupportedModelRules(rules []string) (includes []string, excludes []string) {
	includes = make([]string, 0, len(rules))
	excludes = make([]string, 0, len(rules))
	for _, rawRule := range rules {
		rule := strings.TrimSpace(rawRule)
		if rule == "" {
			continue
		}
		if strings.HasPrefix(rule, "!") {
			pattern := strings.TrimSpace(strings.TrimPrefix(rule, "!"))
			if strings.HasPrefix(pattern, "!") {
				continue
			}
			if isValidSupportedModelPattern(pattern) {
				excludes = append(excludes, pattern)
			}
			continue
		}
		if isValidSupportedModelPattern(rule) {
			includes = append(includes, rule)
		}
	}
	return includes, excludes
}

func isValidSupportedModelPattern(pattern string) bool {
	trimmed := strings.TrimSpace(pattern)
	if trimmed == "" {
		return false
	}
	if strings.Count(trimmed, "!") > 1 {
		return false
	}
	normalized := trimmed
	if strings.HasPrefix(normalized, "!") {
		normalized = strings.TrimSpace(strings.TrimPrefix(normalized, "!"))
	}
	if normalized == "" || strings.HasPrefix(normalized, "!") {
		return false
	}
	starCount := strings.Count(normalized, "*")
	if starCount == 0 {
		return true
	}
	if normalized == "*" {
		return true
	}
	if starCount == 1 {
		return strings.HasPrefix(normalized, "*") || strings.HasSuffix(normalized, "*")
	}
	if starCount == 2 {
		return strings.HasPrefix(normalized, "*") && strings.HasSuffix(normalized, "*") && strings.Trim(normalized, "*") != ""
	}
	return false
}

func matchSupportedModelPattern(pattern, model string) bool {
	if !isValidSupportedModelPattern(pattern) {
		return false
	}
	if strings.HasPrefix(pattern, "!") {
		pattern = strings.TrimSpace(strings.TrimPrefix(pattern, "!"))
	}
	if pattern == "*" {
		return true
	}
	starCount := strings.Count(pattern, "*")
	if starCount == 0 {
		return pattern == model
	}
	if strings.HasPrefix(pattern, "*") && strings.HasSuffix(pattern, "*") {
		return strings.Contains(model, strings.Trim(pattern, "*"))
	}
	if strings.HasPrefix(pattern, "*") {
		return strings.HasSuffix(model, strings.TrimPrefix(pattern, "*"))
	}
	if strings.HasSuffix(pattern, "*") {
		return strings.HasPrefix(model, strings.TrimSuffix(pattern, "*"))
	}
	return false
}

// GetEffectiveBaseURL 获取当前应使用的 BaseURL（纯 failover 模式）
// 优先使用 BaseURL 字段（支持调用方临时覆盖），否则从 BaseURLs 数组获取
func (u *UpstreamConfig) GetEffectiveBaseURL() string {
	// 优先使用 BaseURL（可能被调用方临时设置用于指定本次请求的 URL）
	if u.BaseURL != "" {
		return utils.CanonicalBaseURL(u.BaseURL, u.ServiceType)
	}

	// 回退到 BaseURLs 数组
	if len(u.BaseURLs) > 0 {
		return utils.CanonicalBaseURL(u.BaseURLs[0], u.ServiceType)
	}

	return ""
}

// GetAllBaseURLs 获取所有 BaseURL（用于延迟测试）
func (u *UpstreamConfig) GetAllBaseURLs() []string {
	if len(u.BaseURLs) > 0 {
		return deduplicateBaseURLs(u.BaseURLs, u.ServiceType)
	}
	if u.BaseURL != "" {
		canonical := utils.CanonicalBaseURL(u.BaseURL, u.ServiceType)
		if canonical == "" {
			return nil
		}
		return []string{canonical}
	}
	return nil
}
