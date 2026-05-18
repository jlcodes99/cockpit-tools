package scheduler

import (
	"context"
	"fmt"
	"log"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/transitions"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/warmup"
)

// ChannelScheduler 多渠道调度器
type ChannelScheduler struct {
	mu                       sync.RWMutex
	configManager            *config.ConfigManager
	messagesMetricsManager   *metrics.MetricsManager // Messages 渠道指标
	responsesMetricsManager  *metrics.MetricsManager // Responses 渠道指标
	geminiMetricsManager     *metrics.MetricsManager // Gemini 渠道指标
	chatMetricsManager       *metrics.MetricsManager // Chat 渠道指标
	imagesMetricsManager     *metrics.MetricsManager // Images 渠道指标
	traceAffinity            *session.TraceAffinityManager
	urlManager               *warmup.URLManager       // URL 管理器（非阻塞，动态排序）
	messagesChannelLogStore  *metrics.ChannelLogStore // Messages 渠道请求日志
	responsesChannelLogStore *metrics.ChannelLogStore // Responses 渠道请求日志
	geminiChannelLogStore    *metrics.ChannelLogStore // Gemini 渠道请求日志
	chatChannelLogStore      *metrics.ChannelLogStore // Chat 渠道请求日志
	imagesChannelLogStore    *metrics.ChannelLogStore // Images 渠道请求日志
}

// ChannelKind 标识调度器所处理的渠道类型
// 注意：这里的 kind 与 upstream.ServiceType（openai/claude/gemini）不同，
// kind 对应的是本代理对外暴露的三类入口：messages / responses / gemini。
type ChannelKind string

const (
	ChannelKindMessages  ChannelKind = "messages"
	ChannelKindResponses ChannelKind = "responses"
	ChannelKindGemini    ChannelKind = "gemini"
	ChannelKindChat      ChannelKind = "chat"
	ChannelKindImages    ChannelKind = "images"
)

// NewChannelScheduler 创建多渠道调度器
func NewChannelScheduler(
	cfgManager *config.ConfigManager,
	messagesMetrics *metrics.MetricsManager,
	responsesMetrics *metrics.MetricsManager,
	geminiMetrics *metrics.MetricsManager,
	chatMetrics *metrics.MetricsManager,
	imagesMetrics *metrics.MetricsManager,
	traceAffinity *session.TraceAffinityManager,
	urlMgr *warmup.URLManager,
) *ChannelScheduler {
	return &ChannelScheduler{
		configManager:            cfgManager,
		messagesMetricsManager:   messagesMetrics,
		responsesMetricsManager:  responsesMetrics,
		geminiMetricsManager:     geminiMetrics,
		chatMetricsManager:       chatMetrics,
		imagesMetricsManager:     imagesMetrics,
		traceAffinity:            traceAffinity,
		urlManager:               urlMgr,
		messagesChannelLogStore:  metrics.NewChannelLogStore(),
		responsesChannelLogStore: metrics.NewChannelLogStore(),
		geminiChannelLogStore:    metrics.NewChannelLogStore(),
		chatChannelLogStore:      metrics.NewChannelLogStore(),
		imagesChannelLogStore:    metrics.NewChannelLogStore(),
	}
}

// getMetricsManager 根据类型获取对应的指标管理器
func (s *ChannelScheduler) getMetricsManager(kind ChannelKind) *metrics.MetricsManager {
	switch kind {
	case ChannelKindResponses:
		return s.responsesMetricsManager
	case ChannelKindGemini:
		return s.geminiMetricsManager
	case ChannelKindChat:
		return s.chatMetricsManager
	case ChannelKindImages:
		return s.imagesMetricsManager
	default:
		return s.messagesMetricsManager
	}
}

func metricsLookupKeys(baseURL, apiKey, serviceType string) []string {
	seen := make(map[string]struct{}, 4)
	keys := make([]string, 0, 4)
	add := func(metricsKey string) {
		if metricsKey == "" {
			return
		}
		if _, exists := seen[metricsKey]; exists {
			return
		}
		seen[metricsKey] = struct{}{}
		keys = append(keys, metricsKey)
	}

	add(metrics.GenerateMetricsIdentityKey(baseURL, apiKey, serviceType))
	for _, variant := range utils.EquivalentBaseURLVariants(baseURL, serviceType) {
		add(metrics.GenerateMetricsKey(variant, apiKey))
	}
	return keys
}

func NormalizedMetricsServiceType(kind ChannelKind, configured string) string {
	if configured != "" {
		return configured
	}
	switch kind {
	case ChannelKindGemini:
		return "gemini"
	case ChannelKindResponses:
		return "responses"
	case ChannelKindChat:
		return "openai"
	case ChannelKindImages:
		return "openai"
	default:
		return "claude"
	}
}

func (s *ChannelScheduler) setChannelStatusByKind(index int, kind ChannelKind, status string) error {
	switch kind {
	case ChannelKindResponses:
		return s.configManager.SetResponsesChannelStatus(index, status)
	case ChannelKindGemini:
		return s.configManager.SetGeminiChannelStatus(index, status)
	case ChannelKindChat:
		return s.configManager.SetChatChannelStatus(index, status)
	case ChannelKindImages:
		return s.configManager.SetImagesChannelStatus(index, status)
	default:
		return s.configManager.SetChannelStatus(index, status)
	}
}

type ScheduledRecoveryResult struct {
	Kind             ChannelKind
	ChannelIndex     int
	ChannelName      string
	RestoredKeys     []string
	ActivatedChannel bool
}

// SelectionResult 渠道选择结果
type SelectionResult struct {
	Upstream     *config.UpstreamConfig
	ChannelIndex int
	Reason       string // 选择原因（用于日志）
}

// NextScheduledRecoveryTimeUTC 返回下一个 UTC 0/8/16 点后 1 秒的恢复时刻。
func NextScheduledRecoveryTimeUTC(now time.Time) time.Time {
	now = now.UTC()
	base := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 1, 0, time.UTC)
	for _, hour := range []int{0, 8, 16} {
		candidate := time.Date(base.Year(), base.Month(), base.Day(), hour, 0, 1, 0, time.UTC)
		if now.Before(candidate) {
			return candidate
		}
	}
	return base.Add(24 * time.Hour)
}

// LastScheduledRecoveryTimeUTC 返回当前时刻之前最近一个 UTC 0/8/16 点后 1 秒的恢复时刻。
func LastScheduledRecoveryTimeUTC(now time.Time) time.Time {
	now = now.UTC()
	base := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 1, 0, time.UTC)
	for i := len([]int{0, 8, 16}) - 1; i >= 0; i-- {
		hour := []int{0, 8, 16}[i]
		candidate := time.Date(base.Year(), base.Month(), base.Day(), hour, 0, 1, 0, time.UTC)
		if !now.Before(candidate) {
			return candidate
		}
	}
	return base.Add(-8 * time.Hour)
}

// MissedScheduledRecoveryTimeUTC 返回 (lastChecked, now] 区间内最近错过的恢复槽位。
func MissedScheduledRecoveryTimeUTC(lastChecked, now time.Time) (time.Time, bool) {
	lastChecked = lastChecked.UTC()
	now = now.UTC()
	if !now.After(lastChecked) {
		return time.Time{}, false
	}
	candidate := LastScheduledRecoveryTimeUTC(now)
	if candidate.After(lastChecked) {
		return candidate, true
	}
	return time.Time{}, false
}

func shouldSkipScheduledRecovery(disabledAt, recoverAt string, now time.Time) bool {
	if recoverAt != "" {
		parsed, err := time.Parse(time.RFC3339, recoverAt)
		if err == nil {
			return now.Before(parsed.UTC())
		}
	}
	if disabledAt == "" {
		return false
	}
	parsed, err := time.Parse(time.RFC3339, disabledAt)
	if err != nil {
		return false
	}
	return now.Sub(parsed.UTC()) < time.Hour
}

func kindAPIType(kind ChannelKind) string {
	switch kind {
	case ChannelKindResponses:
		return "Responses"
	case ChannelKindGemini:
		return "Gemini"
	case ChannelKindChat:
		return "Chat"
	case ChannelKindImages:
		return "Images"
	default:
		return "Messages"
	}
}

func (s *ChannelScheduler) scheduledRecoveryKinds() []ChannelKind {
	return []ChannelKind{ChannelKindMessages, ChannelKindResponses, ChannelKindGemini, ChannelKindChat, ChannelKindImages}
}

func (s *ChannelScheduler) restoreScheduledKeysForKind(kind ChannelKind, now time.Time) ([]ScheduledRecoveryResult, error) {
	cfg := s.configManager.GetConfig()
	var upstreams []config.UpstreamConfig
	switch kind {
	case ChannelKindResponses:
		upstreams = cfg.ResponsesUpstream
	case ChannelKindGemini:
		upstreams = cfg.GeminiUpstream
	case ChannelKindChat:
		upstreams = cfg.ChatUpstream
	case ChannelKindImages:
		upstreams = cfg.ImagesUpstream
	default:
		upstreams = cfg.Upstream
	}

	metricsManager := s.getMetricsManager(kind)
	apiType := kindAPIType(kind)
	results := make([]ScheduledRecoveryResult, 0)

	for idx, upstream := range upstreams {
		if upstream.Status == "disabled" || len(upstream.DisabledAPIKeys) == 0 {
			continue
		}

		keysToRestore := make([]string, 0)
		for _, dk := range upstream.DisabledAPIKeys {
			if !config.IsAutoRecoverableDisabledReason(dk.Reason) {
				continue
			}
			if shouldSkipScheduledRecovery(dk.DisabledAt, dk.RecoverAt, now) {
				continue
			}
			keysToRestore = append(keysToRestore, dk.Key)
		}
		if len(keysToRestore) == 0 {
			continue
		}

		restoreResult, err := transitions.RestoreDisabledKeysAndActivate(
			func(keys []string) ([]string, error) {
				return s.configManager.RestoreDisabledKeys(apiType, idx, keys)
			},
			func(_ string, apiKey string) {
				for _, baseURL := range upstream.GetAllBaseURLs() {
					metricsManager.MoveKeyToHalfOpen(baseURL, apiKey, NormalizedMetricsServiceType(kind, upstream.ServiceType))
				}
			},
			func(status string) error {
				return s.setChannelStatusByKind(idx, kind, status)
			},
			func() bool {
				latest := s.getUpstreamByIndex(idx, kind)
				return latest != nil && upstream.Status == "suspended" && len(upstream.APIKeys) == 0 && latest.Status == "suspended"
			},
			keysToRestore,
		)
		if err != nil {
			return nil, err
		}
		if len(restoreResult.RestoredKeys) == 0 {
			continue
		}

		updatedUpstream := s.getUpstreamByIndex(idx, kind)
		if updatedUpstream == nil {
			continue
		}

		results = append(results, ScheduledRecoveryResult{
			Kind:             kind,
			ChannelIndex:     idx,
			ChannelName:      updatedUpstream.Name,
			RestoredKeys:     restoreResult.RestoredKeys,
			ActivatedChannel: restoreResult.ActivatedChannel,
		})
	}

	return results, nil
}

// RunScheduledRecoveries 执行一次自动恢复扫描。
func (s *ChannelScheduler) RunScheduledRecoveries(now time.Time) ([]ScheduledRecoveryResult, error) {
	results := make([]ScheduledRecoveryResult, 0)
	for _, kind := range s.scheduledRecoveryKinds() {
		kindResults, err := s.restoreScheduledKeysForKind(kind, now.UTC())
		if err != nil {
			return nil, err
		}
		results = append(results, kindResults...)
	}
	return results, nil
}

// 优先级: 指定渠道 > 促销期渠道 > Trace亲和（促销渠道失败时回退） > 渠道优先级顺序
func (s *ChannelScheduler) SelectChannel(
	ctx context.Context,
	userID string,
	failedChannels map[int]bool,
	kind ChannelKind,
	model string,
	routePrefix string,
	channelName string,
) (*SelectionResult, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()

	// 获取活跃渠道列表（含模型过滤）
	activeChannels := s.getActiveChannels(kind, model)
	if len(activeChannels) == 0 {
		// 区分"无活跃渠道"和"无渠道支持该模型"
		kindName := "Messages"
		switch kind {
		case ChannelKindGemini:
			kindName = "Gemini"
		case ChannelKindResponses:
			kindName = "Responses"
		case ChannelKindChat:
			kindName = "Chat"
		case ChannelKindImages:
			kindName = "Images"
		}
		if model != "" && len(s.getActiveChannels(kind, "")) > 0 {
			return nil, fmt.Errorf("没有 %s 渠道支持模型 %q，请检查渠道的 supportedModels 配置", kindName, model)
		}
		return nil, fmt.Errorf("没有可用的活跃 %s 渠道", kindName)
	}

	// 指定渠道名（X-Channel 头）：直接定位，跳过所有自动选择逻辑
	if channelName != "" {
		for _, ch := range activeChannels {
			if ch.Name == channelName {
				if failedChannels[ch.Index] {
					return nil, fmt.Errorf("指定渠道 %q 在本次请求中已失败", channelName)
				}
				upstream := s.getUpstreamByIndex(ch.Index, kind)
				if upstream == nil {
					return nil, fmt.Errorf("指定渠道 %q 配置异常", channelName)
				}
				prefix := kindSchedulerLogPrefix(kind)
				log.Printf("[%s-Pin] 通过 X-Channel 指定渠道: [%d] %s", prefix, ch.Index, ch.Name)
				return &SelectionResult{
					Upstream:     upstream,
					ChannelIndex: ch.Index,
					Reason:       "channel_pin",
				}, nil
			}
		}
		return nil, fmt.Errorf("未找到名为 %q 的活跃渠道", channelName)
	}

	// 按路由前缀过滤渠道
	if routePrefix != "" {
		// 有前缀：仅选择匹配的渠道
		var filtered []ChannelInfo
		for _, ch := range activeChannels {
			upstream := s.getUpstreamByIndex(ch.Index, kind)
			if upstream != nil && upstream.RoutePrefix == routePrefix {
				filtered = append(filtered, ch)
			}
		}
		if len(filtered) == 0 {
			return nil, fmt.Errorf("route prefix %q has no available %s channel: %s", routePrefix, kindDisplayName(kind), s.describeRoutePrefixChannels(kind, model, routePrefix))
		}
		activeChannels = filtered
	} else {
		// 无前缀：排除设了路由前缀的渠道（它们只能通过前缀访问）
		var filtered []ChannelInfo
		for _, ch := range activeChannels {
			upstream := s.getUpstreamByIndex(ch.Index, kind)
			if upstream != nil && upstream.RoutePrefix == "" {
				filtered = append(filtered, ch)
			}
		}
		if len(filtered) == 0 {
			kindName := "Messages"
			switch kind {
			case ChannelKindGemini:
				kindName = "Gemini"
			case ChannelKindResponses:
				kindName = "Responses"
			case ChannelKindChat:
				kindName = "Chat"
			case ChannelKindImages:
				kindName = "Images"
			}
			return nil, fmt.Errorf("没有可用于默认路由的 %s 渠道，请使用带前缀路由访问", kindName)
		}
		activeChannels = filtered
	}

	// 0. 检查促销期渠道（最高优先级，绕过健康检查）
	promotedChannel := s.findPromotedChannel(activeChannels, kind)
	if promotedChannel != nil && !failedChannels[promotedChannel.Index] {
		// 促销渠道存在且未失败，直接使用（不检查健康状态，让用户设置的促销渠道有机会尝试）
		upstream := s.getUpstreamByIndex(promotedChannel.Index, kind)
		if upstream != nil && len(upstream.APIKeys) > 0 {
			failureRate := s.channelFailureRate(upstream, kind)
			prefix := kindSchedulerLogPrefix(kind)
			log.Printf("[%s-Promotion] 促销期优先选择渠道: [%d] %s (失败率: %.1f%%, 绕过健康检查)", prefix, promotedChannel.Index, upstream.Name, failureRate*100)
			return &SelectionResult{
				Upstream:     upstream,
				ChannelIndex: promotedChannel.Index,
				Reason:       "promotion_priority",
			}, nil
		} else if upstream != nil {
			prefix := kindSchedulerLogPrefix(kind)
			log.Printf("[%s-Promotion] 警告: 促销渠道 [%d] %s 无可用密钥，跳过", prefix, promotedChannel.Index, upstream.Name)
		}
	} else if promotedChannel != nil {
		prefix := kindSchedulerLogPrefix(kind)
		log.Printf("[%s-Promotion] 警告: 促销渠道 [%d] %s 已在本次请求中失败，跳过", prefix, promotedChannel.Index, promotedChannel.Name)
	}

	// 1. 检查 Trace 亲和性（促销渠道失败时或无促销渠道时）
	if userID != "" {
		compositeKey := string(kind) + ":" + userID
		if preferredIdx, ok := s.traceAffinity.GetPreferredChannel(compositeKey); ok {
			bestPriority := s.findBestAvailableChannelPriority(activeChannels, failedChannels, kind)
			for _, ch := range activeChannels {
				if ch.Index == preferredIdx && !failedChannels[preferredIdx] {
					// 检查渠道状态：只有 active 状态才使用亲和性
					if ch.Status != "active" {
						prefix := kindSchedulerLogPrefix(kind)
						log.Printf("[%s-Affinity] 跳过亲和渠道 [%d] %s: 状态为 %s (user: %s)", prefix, preferredIdx, ch.Name, ch.Status, maskUserID(userID))
						continue
					}
					// 如果存在更高优先级且健康的候选渠道，允许优先级覆盖亲和性
					if bestPriority >= 0 && ch.Priority > bestPriority {
						prefix := kindSchedulerLogPrefix(kind)
						log.Printf("[%s-Affinity] 跳过亲和渠道 [%d] %s: 存在更高优先级可用渠道 (亲和优先级: %d, 最优优先级: %d, user: %s)", prefix, preferredIdx, ch.Name, ch.Priority, bestPriority, maskUserID(userID))
						continue
					}
					// 检查渠道是否健康
					upstream := s.getUpstreamByIndex(preferredIdx, kind)
					if upstream != nil && s.channelIsHealthy(upstream, kind) {
						prefix := kindSchedulerLogPrefix(kind)
						log.Printf("[%s-Affinity] Trace亲和选择渠道: [%d] %s (user: %s)", prefix, preferredIdx, upstream.Name, maskUserID(userID))
						return &SelectionResult{
							Upstream:     upstream,
							ChannelIndex: preferredIdx,
							Reason:       "trace_affinity",
						}, nil
					}
				}
			}
		}
	}

	// 2. 按优先级遍历活跃渠道
	for _, ch := range activeChannels {
		// 跳过本次请求已经失败的渠道
		if failedChannels[ch.Index] {
			continue
		}

		// 跳过非 active 状态的渠道（suspended 等）
		if ch.Status != "active" {
			prefix := kindSchedulerLogPrefix(kind)
			log.Printf("[%s-Channel] 跳过非活跃渠道: [%d] %s (状态: %s)", prefix, ch.Index, ch.Name, ch.Status)
			continue
		}

		upstream := s.getUpstreamByIndex(ch.Index, kind)
		if upstream == nil || len(upstream.APIKeys) == 0 {
			continue
		}

		// 跳过失败率过高的渠道（已熔断或即将熔断）
		channelState := s.channelCircuitState(upstream, kind)
		if channelState == metrics.CircuitStateOpen || !s.channelIsHealthy(upstream, kind) {
			failureRate := s.channelFailureRate(upstream, kind)
			prefix := kindSchedulerLogPrefix(kind)
			if channelState == metrics.CircuitStateOpen {
				log.Printf("[%s-Channel] 警告: 跳过 open 渠道: [%d] %s (失败率: %.1f%%)", prefix, ch.Index, ch.Name, failureRate*100)
			} else {
				log.Printf("[%s-Channel] 警告: 跳过不健康渠道: [%d] %s (失败率: %.1f%%)", prefix, ch.Index, ch.Name, failureRate*100)
			}
			continue
		}

		prefix := kindSchedulerLogPrefix(kind)
		log.Printf("[%s-Channel] 选择渠道: [%d] %s (优先级: %d)", prefix, ch.Index, upstream.Name, ch.Priority)
		return &SelectionResult{
			Upstream:     upstream,
			ChannelIndex: ch.Index,
			Reason:       "priority_order",
		}, nil
	}

	// 3. 所有健康渠道都失败，选择失败率最低的作为降级
	return s.selectFallbackChannel(activeChannels, failedChannels, kind)
}

func kindDisplayName(kind ChannelKind) string {
	switch kind {
	case ChannelKindResponses:
		return "Responses"
	case ChannelKindGemini:
		return "Gemini"
	case ChannelKindChat:
		return "Chat"
	case ChannelKindImages:
		return "Images"
	default:
		return "Messages"
	}
}

func upstreamsByKind(cfg config.Config, kind ChannelKind) []config.UpstreamConfig {
	switch kind {
	case ChannelKindResponses:
		return cfg.ResponsesUpstream
	case ChannelKindGemini:
		return cfg.GeminiUpstream
	case ChannelKindChat:
		return cfg.ChatUpstream
	case ChannelKindImages:
		return cfg.ImagesUpstream
	default:
		return cfg.Upstream
	}
}

func (s *ChannelScheduler) describeRoutePrefixChannels(kind ChannelKind, model, routePrefix string) string {
	cfg := s.configManager.GetConfig()
	upstreams := upstreamsByKind(cfg, kind)
	details := make([]string, 0)

	for i, upstream := range upstreams {
		if upstream.RoutePrefix != routePrefix {
			continue
		}

		status := upstream.Status
		if status == "" {
			status = "active"
		}

		parts := []string{
			fmt.Sprintf("[%d] %s", i, upstream.Name),
			"status=" + status,
			fmt.Sprintf("apiKeys=%d", len(upstream.APIKeys)),
		}

		if model != "" {
			if supported, reason := upstream.ExplainModelSupport(model); !supported {
				parts = append(parts, fmt.Sprintf("model %q unsupported: %s", model, reason))
			}
		}

		if len(upstream.DisabledAPIKeys) > 0 {
			disabled := upstream.DisabledAPIKeys[0]
			disabledParts := []string{fmt.Sprintf("disabledKeys=%d", len(upstream.DisabledAPIKeys))}
			if disabled.Reason != "" {
				disabledParts = append(disabledParts, "reason="+disabled.Reason)
			}
			if disabled.Message != "" {
				disabledParts = append(disabledParts, "message="+disabled.Message)
			}
			if disabled.RecoverAt != "" {
				disabledParts = append(disabledParts, "recoverAt="+disabled.RecoverAt)
			}
			parts = append(parts, strings.Join(disabledParts, ", "))
		}

		if len(upstream.APIKeys) > 0 {
			state := s.channelCircuitState(&upstream, kind)
			failureRate := s.channelFailureRate(&upstream, kind)
			parts = append(parts, fmt.Sprintf("circuit=%s", state.String()))
			parts = append(parts, fmt.Sprintf("failureRate=%.1f%%", failureRate*100))
		}

		details = append(details, strings.Join(parts, "; "))
	}

	if len(details) == 0 {
		return fmt.Sprintf("no configured %s channel has routePrefix=%q", kindDisplayName(kind), routePrefix)
	}
	return strings.Join(details, " | ")
}

func (s *ChannelScheduler) channelCircuitState(upstream *config.UpstreamConfig, kind ChannelKind) metrics.CircuitState {
	if upstream == nil {
		return metrics.CircuitStateClosed
	}
	return s.getMetricsManager(kind).GetChannelCircuitStateMultiURL(upstream.GetAllBaseURLs(), upstream.APIKeys, NormalizedMetricsServiceType(kind, upstream.ServiceType))
}

func (s *ChannelScheduler) channelFailureRate(upstream *config.UpstreamConfig, kind ChannelKind) float64 {
	if upstream == nil {
		return 0
	}
	return s.getMetricsManager(kind).CalculateChannelFailureRateMultiURL(upstream.GetAllBaseURLs(), upstream.APIKeys, NormalizedMetricsServiceType(kind, upstream.ServiceType))
}

func (s *ChannelScheduler) channelIsHealthy(upstream *config.UpstreamConfig, kind ChannelKind) bool {
	if upstream == nil {
		return false
	}
	return s.getMetricsManager(kind).IsChannelHealthyMultiURL(upstream.GetAllBaseURLs(), upstream.APIKeys, NormalizedMetricsServiceType(kind, upstream.ServiceType))
}

// findPromotedChannel 查找处于促销期的渠道
func (s *ChannelScheduler) findPromotedChannel(activeChannels []ChannelInfo, kind ChannelKind) *ChannelInfo {
	for i := range activeChannels {
		ch := &activeChannels[i]
		if ch.Status != "active" {
			continue
		}
		upstream := s.getUpstreamByIndex(ch.Index, kind)
		if upstream != nil {
			if config.IsChannelInPromotion(upstream) {
				prefix := kindSchedulerLogPrefix(kind)
				log.Printf("[%s-Promotion] 找到促销渠道: [%d] %s (promotionUntil: %v)", prefix, ch.Index, upstream.Name, upstream.PromotionUntil)
				return ch
			}
		}
	}
	return nil
}

// selectFallbackChannel 选择降级渠道（失败率最低的）
func (s *ChannelScheduler) selectFallbackChannel(
	activeChannels []ChannelInfo,
	failedChannels map[int]bool,
	kind ChannelKind,
) (*SelectionResult, error) {
	var bestChannel *ChannelInfo
	var bestUpstream *config.UpstreamConfig
	bestFailureRate := float64(2) // 初始化为不可能的值

	for i := range activeChannels {
		ch := &activeChannels[i]
		if failedChannels[ch.Index] {
			continue
		}
		// 跳过非 active 状态的渠道
		if ch.Status != "active" {
			continue
		}

		upstream := s.getUpstreamByIndex(ch.Index, kind)
		if upstream == nil || len(upstream.APIKeys) == 0 {
			continue
		}

		channelState := s.channelCircuitState(upstream, kind)
		if channelState == metrics.CircuitStateOpen {
			continue
		}

		failureRate := s.channelFailureRate(upstream, kind)
		if failureRate < bestFailureRate {
			bestFailureRate = failureRate
			bestChannel = ch
			bestUpstream = upstream
		}
	}

	if bestChannel != nil && bestUpstream != nil {
		prefix := kindSchedulerLogPrefix(kind)
		log.Printf("[%s-Fallback] 警告: 降级选择渠道: [%d] %s (失败率: %.1f%%)",
			prefix, bestChannel.Index, bestUpstream.Name, bestFailureRate*100)
		return &SelectionResult{
			Upstream:     bestUpstream,
			ChannelIndex: bestChannel.Index,
			Reason:       "fallback",
		}, nil
	}

	return nil, fmt.Errorf("所有渠道都不可用")
}

// ChannelInfo 渠道信息（用于排序）
// Priority 约定为非负整数，数字越小优先级越高；0 表示未显式配置，将回退为渠道索引。
type ChannelInfo struct {
	Index    int
	Name     string
	Priority int
	Status   string
}

// getActiveChannels 获取活跃渠道列表（按优先级排序）
func (s *ChannelScheduler) getActiveChannels(kind ChannelKind, model string) []ChannelInfo {
	cfg := s.configManager.GetConfig()

	var upstreams []config.UpstreamConfig
	switch kind {
	case ChannelKindResponses:
		upstreams = cfg.ResponsesUpstream
	case ChannelKindGemini:
		upstreams = cfg.GeminiUpstream
	case ChannelKindChat:
		upstreams = cfg.ChatUpstream
	case ChannelKindImages:
		upstreams = cfg.ImagesUpstream
	default:
		upstreams = cfg.Upstream
	}

	// 筛选活跃渠道
	var activeChannels []ChannelInfo
	for i, upstream := range upstreams {
		status := upstream.Status
		if status == "" {
			status = "active" // 默认为活跃
		}

		// 只选择 active 状态的渠道（suspended 也算在活跃序列中，但会被健康检查过滤）
		if status != "disabled" {
			// 过滤不支持当前模型的渠道
			if model != "" {
				supported, reason := upstream.ExplainModelSupport(model)
				if !supported {
					prefix := kindSchedulerLogPrefix(kind)
					log.Printf("[%s-ModelFilter] 跳过渠道 [%d] %s: 模型 %q 不被 supportedModels 支持 (%s)", prefix, i, upstream.Name, model, reason)
					continue
				}
			}

			priority := upstream.Priority
			if priority == 0 {
				priority = i // 默认优先级为索引
			}

			activeChannels = append(activeChannels, ChannelInfo{
				Index:    i,
				Name:     upstream.Name,
				Priority: priority,
				Status:   status,
			})
		}
	}

	// 按优先级排序（数字越小优先级越高）
	sort.Slice(activeChannels, func(i, j int) bool {
		return activeChannels[i].Priority < activeChannels[j].Priority
	})

	return activeChannels
}

// findBestAvailableChannelPriority 找到当前最佳可用渠道的优先级（用于 affinity 覆盖判断）
// 返回 -1 表示没有可用渠道
func (s *ChannelScheduler) findBestAvailableChannelPriority(
	activeChannels []ChannelInfo,
	failedChannels map[int]bool,
	kind ChannelKind,
) int {
	bestPriority := -1

	for _, ch := range activeChannels {
		if failedChannels[ch.Index] || ch.Status != "active" {
			continue
		}

		upstream := s.getUpstreamByIndex(ch.Index, kind)
		if upstream == nil || len(upstream.APIKeys) == 0 {
			continue
		}
		if s.channelCircuitState(upstream, kind) == metrics.CircuitStateOpen || !s.channelIsHealthy(upstream, kind) {
			continue
		}

		if bestPriority == -1 || ch.Priority < bestPriority {
			bestPriority = ch.Priority
		}
	}

	return bestPriority
}

// getUpstreamByIndex 根据索引获取上游配置
// 注意：返回的是副本，避免指向 slice 元素的指针在 slice 重分配后失效
func (s *ChannelScheduler) getUpstreamByIndex(index int, kind ChannelKind) *config.UpstreamConfig {
	cfg := s.configManager.GetConfig()

	var upstreams []config.UpstreamConfig
	switch kind {
	case ChannelKindResponses:
		upstreams = cfg.ResponsesUpstream
	case ChannelKindGemini:
		upstreams = cfg.GeminiUpstream
	case ChannelKindChat:
		upstreams = cfg.ChatUpstream
	case ChannelKindImages:
		upstreams = cfg.ImagesUpstream
	default:
		upstreams = cfg.Upstream
	}

	if index >= 0 && index < len(upstreams) {
		// 返回副本，避免返回指向 slice 元素的指针
		upstream := upstreams[index]
		return &upstream
	}
	return nil
}

// RecordSuccess 记录渠道成功（使用 baseURL + apiKey）
func (s *ChannelScheduler) RecordSuccess(baseURL, apiKey, serviceType string, kind ChannelKind) {
	s.getMetricsManager(kind).RecordSuccess(baseURL, apiKey, serviceType)
}

// RecordSuccessWithUsage 记录渠道成功（带 Usage 数据）
func (s *ChannelScheduler) RecordSuccessWithUsage(baseURL, apiKey, serviceType string, usage *types.Usage, kind ChannelKind) {
	s.getMetricsManager(kind).RecordSuccessWithUsage(baseURL, apiKey, serviceType, usage)
}

// RecordFailure 记录渠道失败（使用 baseURL + apiKey）
func (s *ChannelScheduler) RecordFailure(baseURL, apiKey, serviceType string, kind ChannelKind) {
	s.getMetricsManager(kind).RecordFailure(baseURL, apiKey, serviceType)
}

// RecordRequestStart 记录请求开始
func (s *ChannelScheduler) RecordRequestStart(baseURL, apiKey, serviceType string, kind ChannelKind) {
	s.getMetricsManager(kind).RecordRequestStart(baseURL, apiKey, serviceType)
}

// RecordRequestEnd 记录请求结束
func (s *ChannelScheduler) RecordRequestEnd(baseURL, apiKey, serviceType string, kind ChannelKind) {
	s.getMetricsManager(kind).RecordRequestEnd(baseURL, apiKey, serviceType)
}

// SetTraceAffinity 设置 Trace 亲和（按 kind 隔离）
func (s *ChannelScheduler) SetTraceAffinity(userID string, channelIndex int, kind ChannelKind) {
	if userID != "" {
		compositeKey := string(kind) + ":" + userID
		s.traceAffinity.SetPreferredChannel(compositeKey, channelIndex)
	}
}

// UpdateTraceAffinity 更新 Trace 亲和时间（续期，按 kind 隔离）
func (s *ChannelScheduler) UpdateTraceAffinity(userID string, kind ChannelKind) {
	if userID != "" {
		compositeKey := string(kind) + ":" + userID
		s.traceAffinity.UpdateLastUsed(compositeKey)
	}
}

// GetMessagesMetricsManager 获取 Messages 渠道指标管理器
func (s *ChannelScheduler) GetMessagesMetricsManager() *metrics.MetricsManager {
	return s.messagesMetricsManager
}

// GetResponsesMetricsManager 获取 Responses 渠道指标管理器
func (s *ChannelScheduler) GetResponsesMetricsManager() *metrics.MetricsManager {
	return s.responsesMetricsManager
}

// GetGeminiMetricsManager 获取 Gemini 渠道指标管理器
func (s *ChannelScheduler) GetGeminiMetricsManager() *metrics.MetricsManager {
	return s.geminiMetricsManager
}

// GetChatMetricsManager 获取 Chat 指标管理器
func (s *ChannelScheduler) GetChatMetricsManager() *metrics.MetricsManager {
	return s.chatMetricsManager
}

// GetImagesMetricsManager 获取 Images 指标管理器
func (s *ChannelScheduler) GetImagesMetricsManager() *metrics.MetricsManager {
	return s.imagesMetricsManager
}

// GetTraceAffinityManager 获取 Trace 亲和性管理器
func (s *ChannelScheduler) GetTraceAffinityManager() *session.TraceAffinityManager {
	return s.traceAffinity
}

// GetChannelLogStore 根据渠道类型获取对应的日志存储
func (s *ChannelScheduler) GetChannelLogStore(kind ChannelKind) *metrics.ChannelLogStore {
	switch kind {
	case ChannelKindResponses:
		return s.responsesChannelLogStore
	case ChannelKindGemini:
		return s.geminiChannelLogStore
	case ChannelKindChat:
		return s.chatChannelLogStore
	case ChannelKindImages:
		return s.imagesChannelLogStore
	default:
		return s.messagesChannelLogStore
	}
}

// ResetChannelMetrics 重置渠道所有 Key 的熔断/失败状态（保留历史统计）
// 用于：1) 手动恢复熔断 2) 更换 API Key 后重置熔断状态
func (s *ChannelScheduler) ResetChannelMetrics(channelIndex int, kind ChannelKind) {
	upstream := s.getUpstreamByIndex(channelIndex, kind)
	if upstream == nil {
		return
	}
	metricsManager := s.getMetricsManager(kind)
	for _, baseURL := range upstream.GetAllBaseURLs() {
		for _, apiKey := range upstream.APIKeys {
			metricsManager.ResetKeyFailureState(baseURL, apiKey, NormalizedMetricsServiceType(kind, upstream.ServiceType))
		}
	}
	prefix := kindSchedulerLogPrefix(kind)
	log.Printf("[%s-Reset] 渠道 [%d] %s 的熔断状态已重置（保留历史统计）", prefix, channelIndex, upstream.Name)
}

// ResetKeyMetrics 重置单个 Key 的指标
func (s *ChannelScheduler) ResetKeyMetrics(baseURL, apiKey, serviceType string, kind ChannelKind) {
	s.getMetricsManager(kind).ResetKey(baseURL, apiKey, serviceType)
}

// DeleteChannelMetrics 删除渠道的所有指标数据（内存 + 持久化）
// 用于删除渠道时清理相关的统计数据
// 注意：如果其他渠道使用相同的 (BaseURL, APIKey) 组合，则保留对应的 MetricsKey
// 前置条件：调用此方法前，被删除的渠道应已从 config 中移除
func (s *ChannelScheduler) DeleteChannelMetrics(upstream *config.UpstreamConfig, kind ChannelKind) {
	if upstream == nil {
		return
	}

	prefix := kindSchedulerLogPrefix(kind)

	// 前置条件守卫：检查被删除渠道是否仍在配置中
	// 如果仍在配置中，说明调用时机不对，记录警告并继续执行（但结果可能不正确）
	if s.isUpstreamInConfig(upstream, kind) {
		log.Printf("[%s-Delete] 警告: 渠道 %s 仍在配置中，删除指标可能不完整（应先从配置中移除）", prefix, upstream.Name)
	}

	// 获取被删除渠道的所有 (BaseURL, APIKey) 组合
	deletedBaseURLs := upstream.GetAllBaseURLs()
	deletedKeys := append([]string{}, upstream.APIKeys...)
	deletedKeys = append(deletedKeys, upstream.HistoricalAPIKeys...)

	// 收集当前配置中所有渠道使用的 (BaseURL, APIKey) 组合
	// 注意：此时被删除渠道应已从 config 中移除
	usedMetricsKeys := s.collectUsedMetricsKeys(kind)

	// 收集只被删除渠道独占的 metricsKey 列表（使用 map 去重）
	exclusiveKeysSet := make(map[string]struct{})
	serviceType := NormalizedMetricsServiceType(kind, upstream.ServiceType)

	for _, baseURL := range deletedBaseURLs {
		for _, apiKey := range deletedKeys {
			for _, metricsKey := range metricsLookupKeys(baseURL, apiKey, serviceType) {
				if !usedMetricsKeys[metricsKey] {
					exclusiveKeysSet[metricsKey] = struct{}{}
				}
			}
		}
	}

	// 转换为切片
	exclusiveMetricsKeys := make([]string, 0, len(exclusiveKeysSet))
	for key := range exclusiveKeysSet {
		exclusiveMetricsKeys = append(exclusiveMetricsKeys, key)
	}

	metricsManager := s.getMetricsManager(kind)

	// 只删除独占的 MetricsKey
	if len(exclusiveMetricsKeys) > 0 {
		metricsManager.DeleteByMetricsKeys(exclusiveMetricsKeys)
		log.Printf("[%s-Delete] 渠道 %s 的 %d 个独占指标数据已清理", prefix, upstream.Name, len(exclusiveMetricsKeys))
	} else {
		log.Printf("[%s-Delete] 渠道 %s 的指标数据被其他渠道共享，已保留", prefix, upstream.Name)
	}
}

// collectUsedMetricsKeys 收集当前配置中所有渠道仍在使用的 identity metricsKey。
// 注意：调用此方法前，被删除的渠道应已从 config 中移除。
func (s *ChannelScheduler) collectUsedMetricsKeys(kind ChannelKind) map[string]bool {
	cfg := s.configManager.GetConfig()

	var upstreams []config.UpstreamConfig
	switch kind {
	case ChannelKindResponses:
		upstreams = cfg.ResponsesUpstream
	case ChannelKindGemini:
		upstreams = cfg.GeminiUpstream
	case ChannelKindChat:
		upstreams = cfg.ChatUpstream
	case ChannelKindImages:
		upstreams = cfg.ImagesUpstream
	default:
		upstreams = cfg.Upstream
	}

	usedMetricsKeys := make(map[string]bool)
	for _, upstream := range upstreams {
		baseURLs := upstream.GetAllBaseURLs()
		allKeys := append([]string{}, upstream.APIKeys...)
		allKeys = append(allKeys, upstream.HistoricalAPIKeys...)
		serviceType := NormalizedMetricsServiceType(kind, upstream.ServiceType)

		for _, baseURL := range baseURLs {
			for _, apiKey := range allKeys {
				for _, metricsKey := range metricsLookupKeys(baseURL, apiKey, serviceType) {
					usedMetricsKeys[metricsKey] = true
				}
			}
		}
	}

	return usedMetricsKeys
}

// isUpstreamInConfig 检查指定的 upstream 是否仍在当前配置中
// 通过比较 Name 字段判断（Name 在同类型渠道中应唯一）
func (s *ChannelScheduler) isUpstreamInConfig(upstream *config.UpstreamConfig, kind ChannelKind) bool {
	cfg := s.configManager.GetConfig()

	var upstreams []config.UpstreamConfig
	switch kind {
	case ChannelKindResponses:
		upstreams = cfg.ResponsesUpstream
	case ChannelKindGemini:
		upstreams = cfg.GeminiUpstream
	case ChannelKindChat:
		upstreams = cfg.ChatUpstream
	case ChannelKindImages:
		upstreams = cfg.ImagesUpstream
	default:
		upstreams = cfg.Upstream
	}

	for _, u := range upstreams {
		if u.Name == upstream.Name {
			return true
		}
	}
	return false
}

// GetActiveChannelCount 获取活跃渠道数量
func (s *ChannelScheduler) GetActiveChannelCount(kind ChannelKind) int {
	return len(s.getActiveChannels(kind, ""))
}

// IsMultiChannelMode 判断是否为多渠道模式
func (s *ChannelScheduler) IsMultiChannelMode(kind ChannelKind) bool {
	return s.GetActiveChannelCount(kind) > 1
}

// maskUserID 掩码 user_id（保护隐私）
func maskUserID(userID string) string {
	if len(userID) <= 16 {
		return "***"
	}
	return userID[:8] + "***" + userID[len(userID)-4:]
}

// GetSortedURLsForChannel 获取渠道排序后的 URL 列表（非阻塞，立即返回）
// 返回按动态排序的 URL 结果列表，包含原始索引用于指标记录
func (s *ChannelScheduler) GetSortedURLsForChannel(
	kind ChannelKind,
	channelIndex int,
	urls []string,
) []warmup.URLLatencyResult {
	if s.urlManager == nil || len(urls) <= 1 {
		// 无 URL 管理器或单 URL，返回默认结果
		results := make([]warmup.URLLatencyResult, len(urls))
		for i, url := range urls {
			results[i] = warmup.URLLatencyResult{
				URL:         url,
				OriginalIdx: i,
				Success:     true,
			}
		}
		return results
	}
	return s.urlManager.GetSortedURLs(urlManagerChannelKey(kind, channelIndex), urls)
}

// MarkURLSuccess 标记 URL 成功
func (s *ChannelScheduler) MarkURLSuccess(kind ChannelKind, channelIndex int, url string) {
	if s.urlManager != nil {
		s.urlManager.MarkSuccess(urlManagerChannelKey(kind, channelIndex), url)
	}
}

// MarkURLFailure 标记 URL 失败，触发动态排序
func (s *ChannelScheduler) MarkURLFailure(kind ChannelKind, channelIndex int, url string) {
	if s.urlManager != nil {
		s.urlManager.MarkFailure(urlManagerChannelKey(kind, channelIndex), url)
	}
}

// InvalidateURLCache 使渠道 URL 状态失效
func (s *ChannelScheduler) InvalidateURLCache(kind ChannelKind, channelIndex int) {
	if s.urlManager != nil {
		s.urlManager.InvalidateChannel(urlManagerChannelKey(kind, channelIndex))
	}
}

// GetURLManagerStats 获取 URL 管理器统计
func (s *ChannelScheduler) GetURLManagerStats() map[string]interface{} {
	if s.urlManager != nil {
		return s.urlManager.GetStats()
	}
	return nil
}

func kindSchedulerLogPrefix(kind ChannelKind) string {
	switch kind {
	case ChannelKindResponses:
		return "Scheduler-Responses"
	case ChannelKindGemini:
		return "Scheduler-Gemini"
	case ChannelKindChat:
		return "Scheduler-Chat"
	case ChannelKindImages:
		return "Scheduler-Images"
	default:
		return "Scheduler"
	}
}

func urlManagerChannelKey(kind ChannelKind, channelIndex int) int {
	const stride = 1_000_000
	return urlManagerChannelKeyOrdinal(kind)*stride + channelIndex
}

func urlManagerChannelKeyOrdinal(kind ChannelKind) int {
	switch kind {
	case ChannelKindResponses:
		return 1
	case ChannelKindGemini:
		return 2
	case ChannelKindChat:
		return 3
	case ChannelKindImages:
		return 4
	default:
		return 0
	}
}
