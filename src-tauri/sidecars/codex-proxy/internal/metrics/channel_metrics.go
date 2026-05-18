package metrics

import (
	"crypto/sha256"
	"encoding/hex"
	"log"
	"sort"
	"sync"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/statelog"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
)

// FailureClass 表示请求失败分类，用于区分是否应影响熔断状态机。
type FailureClass string

const (
	FailureClassNone         FailureClass = ""
	FailureClassRetryable    FailureClass = "retryable"
	FailureClassNonRetryable FailureClass = "non_retryable"
	FailureClassQuota        FailureClass = "quota"
	FailureClassClientCancel FailureClass = "client_cancel"
)

// IsBreakerRelevant 判断失败类型是否应影响 breaker 状态机。
func (fc FailureClass) IsBreakerRelevant() bool {
	return fc == FailureClassRetryable
}

// CircuitState 表示 Key 当前的熔断状态。
type CircuitState uint8

const (
	CircuitStateClosed CircuitState = iota
	CircuitStateOpen
	CircuitStateHalfOpen
)

func (s CircuitState) String() string {
	switch s {
	case CircuitStateOpen:
		return "open"
	case CircuitStateHalfOpen:
		return "half_open"
	default:
		return "closed"
	}
}

// ParseCircuitState 解析持久化的状态字符串。
func ParseCircuitState(text string) CircuitState {
	switch text {
	case "open":
		return CircuitStateOpen
	case "half_open":
		return CircuitStateHalfOpen
	default:
		return CircuitStateClosed
	}
}

const (
	consecutiveRetryableFailuresThreshold int64         = 3
	halfOpenSuccessThreshold              int           = 1
	defaultCircuitBackoffBase             time.Duration = 30 * time.Second
	defaultCircuitBackoffMax              time.Duration = 10 * time.Minute
)

// RequestRecord 带时间戳的请求记录（扩展版，支持 Token、Cache 和失败分类数据）。
type RequestRecord struct {
	Model                    string
	Timestamp                time.Time
	Success                  bool
	FailureClass             FailureClass
	InputTokens              int64
	OutputTokens             int64
	CacheCreationInputTokens int64
	CacheReadInputTokens     int64
}

// KeyMetrics 单个 Key 的指标（绑定到 BaseURL + Key 组合）
type KeyMetrics struct {
	MetricsKey          string       `json:"metricsKey"`          // hash(baseURL + apiKey)
	BaseURL             string       `json:"baseUrl"`             // 用于显示
	KeyMask             string       `json:"keyMask"`             // 脱敏的 key（用于显示）
	RequestCount        int64        `json:"requestCount"`        // 总请求数
	SuccessCount        int64        `json:"successCount"`        // 成功数
	FailureCount        int64        `json:"failureCount"`        // 失败数
	ConsecutiveFailures int64        `json:"consecutiveFailures"` // 连续可重试失败数
	ActiveRequests      int64        `json:"activeRequests"`      // 进行中的请求数
	LastSuccessAt       *time.Time   `json:"lastSuccessAt,omitempty"`
	LastFailureAt       *time.Time   `json:"lastFailureAt,omitempty"`
	CircuitBrokenAt     *time.Time   `json:"circuitBrokenAt,omitempty"` // breaker 进入 open 的时间（兼容旧字段）
	CircuitState        CircuitState `json:"-"`
	HalfOpenAt          *time.Time   `json:"halfOpenAt,omitempty"`
	NextRetryAt         *time.Time   `json:"nextRetryAt,omitempty"`
	BackoffLevel        int          `json:"backoffLevel"`
	HalfOpenSuccesses   int          `json:"halfOpenSuccesses"`
	ProbeInFlight       bool         `json:"-"`
	// 滑动窗口记录（最近 N 次请求的结果，用于展示综合成功率）
	recentResults []bool // true=success, false=failure
	// breaker 滑动窗口（仅记录成功和可重试失败）
	breakerResults []bool
	// 带时间戳的请求记录（用于分时段统计，保留24小时）
	requestHistory []RequestRecord
	// 进行中请求在 requestHistory 中的索引（用于“连接即计数”，结束后回写成功/失败与 token）
	pendingHistoryIdx map[uint64]int
}

// ChannelMetrics 渠道聚合指标（用于 API 返回，兼容旧结构）
type ChannelMetrics struct {
	ChannelIndex        int          `json:"channelIndex"`
	RequestCount        int64        `json:"requestCount"`
	SuccessCount        int64        `json:"successCount"`
	FailureCount        int64        `json:"failureCount"`
	ConsecutiveFailures int64        `json:"consecutiveFailures"`
	LastSuccessAt       *time.Time   `json:"lastSuccessAt,omitempty"`
	LastFailureAt       *time.Time   `json:"lastFailureAt,omitempty"`
	CircuitBrokenAt     *time.Time   `json:"circuitBrokenAt,omitempty"`
	CircuitState        CircuitState `json:"-"`
	NextRetryAt         *time.Time   `json:"nextRetryAt,omitempty"`
	HalfOpenSuccesses   int          `json:"halfOpenSuccesses"`
	// 滑动窗口记录（兼容旧代码）
	recentResults  []bool
	breakerResults []bool
	// 带时间戳的请求记录
	requestHistory []RequestRecord
}

// TimeWindowStats 分时段统计
// 使用 omitempty 减少 JSON 体积，0 值字段不输出
// 注意：successRate 不使用 omitempty，因为 0% 是有意义的值（全失败）
type TimeWindowStats struct {
	RequestCount int64   `json:"requestCount,omitempty"`
	SuccessCount int64   `json:"successCount,omitempty"`
	FailureCount int64   `json:"failureCount,omitempty"`
	SuccessRate  float64 `json:"successRate"`
	// Token 统计（按时间窗口聚合）
	InputTokens         int64 `json:"inputTokens,omitempty"`
	OutputTokens        int64 `json:"outputTokens,omitempty"`
	CacheCreationTokens int64 `json:"cacheCreationTokens,omitempty"`
	CacheReadTokens     int64 `json:"cacheReadTokens,omitempty"`
	// CacheHitRate 缓存命中率（Token口径），范围 0-100
	// 定义：cacheReadTokens / (cacheReadTokens + inputTokens) * 100
	CacheHitRate float64 `json:"cacheHitRate,omitempty"`
}

// MetricsManager 指标管理器
type MetricsManager struct {
	mu                    sync.RWMutex
	keyMetrics            map[string]*KeyMetrics // key: hash(baseURL + apiKey)
	windowSize            int                    // 滑动窗口大小
	failureThreshold      float64                // 失败率阈值
	circuitRecoveryTime   time.Duration          // 兼容旧统计字段，表示基础探测冷却时间
	circuitBackoffBase    time.Duration
	circuitBackoffMax     time.Duration
	halfOpenSuccessTarget int
	stopCh                chan struct{} // 用于停止清理 goroutine
	nextRequestID         uint64        // 单进程递增请求ID（用于 pendingHistoryIdx）

	// 持久化存储（可选）
	store   PersistenceStore
	apiType string // "messages"、"responses"、"gemini" 或 "chat"
}

// GetPersistenceStore 获取持久化存储（可能为 nil）
func (m *MetricsManager) GetPersistenceStore() PersistenceStore {
	return m.store
}

// GetAPIType 获取 API 类型
func (m *MetricsManager) GetAPIType() string {
	return m.apiType
}

// NewMetricsManager 创建指标管理器
func NewMetricsManager() *MetricsManager {
	m := &MetricsManager{
		keyMetrics:            make(map[string]*KeyMetrics),
		windowSize:            10,  // 默认基于最近 10 次请求计算失败率
		failureThreshold:      0.5, // 默认 50% 失败率阈值
		circuitRecoveryTime:   defaultCircuitBackoffBase,
		circuitBackoffBase:    defaultCircuitBackoffBase,
		circuitBackoffMax:     defaultCircuitBackoffMax,
		halfOpenSuccessTarget: halfOpenSuccessThreshold,
		stopCh:                make(chan struct{}),
	}
	// 启动后台熔断恢复任务
	go m.cleanupCircuitBreakers()
	return m
}

// NewMetricsManagerWithConfig 创建带配置的指标管理器
func NewMetricsManagerWithConfig(windowSize int, failureThreshold float64) *MetricsManager {
	if windowSize < 3 {
		windowSize = 3 // 最小 3
	}
	if failureThreshold <= 0 || failureThreshold > 1 {
		failureThreshold = 0.5
	}
	m := &MetricsManager{
		keyMetrics:            make(map[string]*KeyMetrics),
		windowSize:            windowSize,
		failureThreshold:      failureThreshold,
		circuitRecoveryTime:   defaultCircuitBackoffBase,
		circuitBackoffBase:    defaultCircuitBackoffBase,
		circuitBackoffMax:     defaultCircuitBackoffMax,
		halfOpenSuccessTarget: halfOpenSuccessThreshold,
		stopCh:                make(chan struct{}),
	}
	// 启动后台熔断恢复任务
	go m.cleanupCircuitBreakers()
	return m
}

// NewMetricsManagerWithPersistence 创建带持久化的指标管理器
func NewMetricsManagerWithPersistence(windowSize int, failureThreshold float64, store PersistenceStore, apiType string) *MetricsManager {
	if windowSize < 3 {
		windowSize = 3
	}
	if failureThreshold <= 0 || failureThreshold > 1 {
		failureThreshold = 0.5
	}
	m := &MetricsManager{
		keyMetrics:            make(map[string]*KeyMetrics),
		windowSize:            windowSize,
		failureThreshold:      failureThreshold,
		circuitRecoveryTime:   defaultCircuitBackoffBase,
		circuitBackoffBase:    defaultCircuitBackoffBase,
		circuitBackoffMax:     defaultCircuitBackoffMax,
		halfOpenSuccessTarget: halfOpenSuccessThreshold,
		stopCh:                make(chan struct{}),
		store:                 store,
		apiType:               apiType,
	}

	// 从持久化存储加载历史数据
	if store != nil {
		if err := m.loadFromStore(); err != nil {
			log.Printf("[Metrics-Load] 警告: [%s] 加载历史指标数据失败: %v", apiType, err)
		}
	}

	// 启动后台熔断恢复任务
	go m.cleanupCircuitBreakers()
	return m
}

// loadFromStore 从持久化存储加载数据
func (m *MetricsManager) loadFromStore() error {
	if m.store == nil {
		return nil
	}

	// 加载最近 24 小时的数据
	since := time.Now().Add(-24 * time.Hour)
	records, err := m.store.LoadRecords(since, m.apiType)
	if err != nil {
		return err
	}

	m.mu.Lock()
	defer m.mu.Unlock()

	if len(records) == 0 {
		log.Printf("[Metrics-Load] [%s] 无历史指标数据需要加载", m.apiType)
	} else {
		for _, r := range records {
			metrics := m.getOrCreateKeyLocked(r.BaseURL, r.MetricsKey, r.KeyMask)

			metrics.requestHistory = append(metrics.requestHistory, RequestRecord{
				Model:                    r.Model,
				Timestamp:                r.Timestamp,
				Success:                  r.Success,
				FailureClass:             normalizeFailureClass(r.Success, r.FailureClass),
				InputTokens:              r.InputTokens,
				OutputTokens:             r.OutputTokens,
				CacheCreationInputTokens: r.CacheCreationTokens,
				CacheReadInputTokens:     r.CacheReadTokens,
			})

			metrics.RequestCount++
			if r.Success {
				metrics.SuccessCount++
				if metrics.LastSuccessAt == nil || r.Timestamp.After(*metrics.LastSuccessAt) {
					t := r.Timestamp
					metrics.LastSuccessAt = &t
				}
			} else {
				metrics.FailureCount++
				if metrics.LastFailureAt == nil || r.Timestamp.After(*metrics.LastFailureAt) {
					t := r.Timestamp
					metrics.LastFailureAt = &t
				}
				if normalizeFailureClass(r.Success, r.FailureClass).IsBreakerRelevant() {
					metrics.ConsecutiveFailures++
				}
			}
		}

		windowCutoff := time.Now().Add(-15 * time.Minute)
		for _, metrics := range m.keyMetrics {
			metrics.recentResults = make([]bool, 0, m.windowSize)
			metrics.breakerResults = make([]bool, 0, m.windowSize)
			var recentRecords []bool
			var breakerRecords []bool
			var consecutiveRetryable int64
			for _, record := range metrics.requestHistory {
				if record.Timestamp.After(windowCutoff) {
					recentRecords = append(recentRecords, record.Success)
					if isBreakerRelevantFailure(record.Success, record.FailureClass) {
						breakerRecords = append(breakerRecords, record.Success)
					}
				}
				if record.Success {
					consecutiveRetryable = 0
				} else if record.FailureClass.IsBreakerRelevant() {
					consecutiveRetryable++
				}
			}
			metrics.ConsecutiveFailures = consecutiveRetryable
			if len(recentRecords) > m.windowSize {
				recentRecords = recentRecords[len(recentRecords)-m.windowSize:]
			}
			if len(breakerRecords) > m.windowSize {
				breakerRecords = breakerRecords[len(breakerRecords)-m.windowSize:]
			}
			metrics.recentResults = append(metrics.recentResults, recentRecords...)
			metrics.breakerResults = append(metrics.breakerResults, breakerRecords...)
		}
	}

	m.loadHistoricalTimestamps()

	states, err := m.store.LoadCircuitStates(m.apiType)
	if err != nil {
		return err
	}
	for metricsKey, state := range states {
		metrics, ok := m.keyMetrics[metricsKey]
		if !ok {
			metrics = m.getOrCreateKeyLocked(state.BaseURL, state.MetricsKey, state.KeyMask)
		}
		metrics.CircuitState = ParseCircuitState(state.CircuitState)
		metrics.CircuitBrokenAt = state.CircuitOpenedAt
		metrics.HalfOpenAt = state.HalfOpenAt
		metrics.NextRetryAt = state.NextRetryAt
		metrics.BackoffLevel = state.BackoffLevel
		metrics.HalfOpenSuccesses = state.HalfOpenSuccesses
		metrics.ConsecutiveFailures = state.ConsecutiveFailures
		metrics.ProbeInFlight = false
	}

	log.Printf("[Metrics-Load] [%s] 已从持久化存储加载 %d 条历史记录、%d 条熔断状态，重建 %d 个 Key 指标",
		m.apiType, len(records), len(states), len(m.keyMetrics))
	return nil
}

// loadHistoricalTimestamps 加载全量历史时间戳，补全超出 24h 窗口的 LastSuccessAt/LastFailureAt。
// 调用前必须已持有 m.mu.Lock()。
func (m *MetricsManager) loadHistoricalTimestamps() {
	timestamps, err := m.store.LoadLatestTimestamps(m.apiType)
	if err != nil {
		log.Printf("[Metrics-Load] 警告: [%s] 加载历史时间戳失败: %v", m.apiType, err)
		return
	}
	for metricsKey, kt := range timestamps {
		existing, ok := m.keyMetrics[metricsKey]
		if !ok {
			// 24h 内无记录但历史有请求：创建空壳，只携带时间戳
			existing = m.getOrCreateKeyLocked(kt.BaseURL, metricsKey, kt.KeyMask)
		}
		// 只在持久化值更新时覆盖（防回退）
		if kt.LastSuccessAt != nil && (existing.LastSuccessAt == nil || kt.LastSuccessAt.After(*existing.LastSuccessAt)) {
			existing.LastSuccessAt = kt.LastSuccessAt
		}
		if kt.LastFailureAt != nil && (existing.LastFailureAt == nil || kt.LastFailureAt.After(*existing.LastFailureAt)) {
			existing.LastFailureAt = kt.LastFailureAt
		}
	}
}

// getOrCreateKeyLocked 获取或创建 Key 指标（用于加载时，已知 metricsKey 和 keyMask）
func (m *MetricsManager) getOrCreateKeyLocked(baseURL, metricsKey, keyMask string) *KeyMetrics {
	if metrics, exists := m.keyMetrics[metricsKey]; exists {
		return metrics
	}
	metrics := &KeyMetrics{
		MetricsKey:        metricsKey,
		BaseURL:           baseURL,
		KeyMask:           keyMask,
		CircuitState:      CircuitStateClosed,
		recentResults:     make([]bool, 0, m.windowSize),
		breakerResults:    make([]bool, 0, m.windowSize),
		pendingHistoryIdx: make(map[uint64]int),
	}
	m.keyMetrics[metricsKey] = metrics
	return metrics
}

// generateMetricsKey 生成指标键 hash(baseURL + apiKey)（内部使用）
func generateMetricsKey(baseURL, apiKey string) string {
	h := sha256.New()
	h.Write([]byte(baseURL + "|" + apiKey))
	return hex.EncodeToString(h.Sum(nil))[:16] // 取前16位作为键
}

// GenerateMetricsKey 生成指标键 hash(baseURL + apiKey)（导出供外部使用）
func GenerateMetricsKey(baseURL, apiKey string) string {
	return generateMetricsKey(baseURL, apiKey)
}

func GenerateMetricsIdentityKey(baseURL, apiKey, serviceType string) string {
	return generateMetricsKey(utils.MetricsIdentityBaseURL(baseURL, serviceType), apiKey)
}

func (m *MetricsManager) metricsLookupKeys(baseURL, apiKey, serviceType string) []string {
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

	add(m.metricsIdentityKey(baseURL, apiKey, serviceType))
	for _, variant := range utils.EquivalentBaseURLVariants(baseURL, serviceType) {
		add(generateMetricsKey(variant, apiKey))
	}
	return keys
}

func (m *MetricsManager) getIdentityMetricsLocked(baseURL, apiKey, serviceType string) *KeyMetrics {
	for _, metricsKey := range m.metricsLookupKeys(baseURL, apiKey, serviceType) {
		if metrics, exists := m.keyMetrics[metricsKey]; exists {
			return metrics
		}
	}
	return nil
}

func (m *MetricsManager) getMetricsVariantsLocked(baseURL, apiKey, serviceType string) []*KeyMetrics {
	lookupKeys := m.metricsLookupKeys(baseURL, apiKey, serviceType)
	seen := make(map[*KeyMetrics]struct{}, len(lookupKeys))
	variants := make([]*KeyMetrics, 0, len(lookupKeys))
	for _, metricsKey := range lookupKeys {
		metrics, exists := m.keyMetrics[metricsKey]
		if !exists {
			continue
		}
		if _, duplicated := seen[metrics]; duplicated {
			continue
		}
		seen[metrics] = struct{}{}
		variants = append(variants, metrics)
	}
	return variants
}

func (m *MetricsManager) getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType string) *KeyMetrics {
	return m.getIdentityMetricsLocked(baseURL, apiKey, serviceType)
}

func (m *MetricsManager) metricsIdentityKey(baseURL, apiKey, serviceType string) string {
	return generateMetricsKey(utils.MetricsIdentityBaseURL(baseURL, serviceType), apiKey)
}

func (m *MetricsManager) circuitStateSeverity(state CircuitState) int {
	switch state {
	case CircuitStateOpen:
		return 3
	case CircuitStateHalfOpen:
		return 2
	default:
		return 1
	}
}

func (m *MetricsManager) mergeKeyMetricsLocked(dst, src *KeyMetrics) {
	if dst == nil || src == nil || dst == src {
		return
	}

	dst.RequestCount += src.RequestCount
	dst.SuccessCount += src.SuccessCount
	dst.FailureCount += src.FailureCount
	dst.ActiveRequests += src.ActiveRequests
	if src.ConsecutiveFailures > dst.ConsecutiveFailures {
		dst.ConsecutiveFailures = src.ConsecutiveFailures
	}
	if dst.LastSuccessAt == nil || (src.LastSuccessAt != nil && src.LastSuccessAt.After(*dst.LastSuccessAt)) {
		dst.LastSuccessAt = src.LastSuccessAt
	}
	if dst.LastFailureAt == nil || (src.LastFailureAt != nil && src.LastFailureAt.After(*dst.LastFailureAt)) {
		dst.LastFailureAt = src.LastFailureAt
	}
	if len(src.recentResults) > 0 {
		dst.recentResults = append(dst.recentResults, src.recentResults...)
		if len(dst.recentResults) > m.windowSize {
			dst.recentResults = dst.recentResults[len(dst.recentResults)-m.windowSize:]
		}
	}
	if len(src.breakerResults) > 0 {
		dst.breakerResults = append(dst.breakerResults, src.breakerResults...)
		if len(dst.breakerResults) > m.windowSize {
			dst.breakerResults = dst.breakerResults[len(dst.breakerResults)-m.windowSize:]
		}
	}
	if len(src.requestHistory) > 0 {
		offset := len(dst.requestHistory)
		dst.requestHistory = append(dst.requestHistory, src.requestHistory...)
		if dst.pendingHistoryIdx == nil {
			dst.pendingHistoryIdx = make(map[uint64]int, len(src.pendingHistoryIdx))
		}
		for requestID, idx := range src.pendingHistoryIdx {
			dst.pendingHistoryIdx[requestID] = offset + idx
		}
	}
	if src.ProbeInFlight {
		dst.ProbeInFlight = true
	}
	if src.BackoffLevel > dst.BackoffLevel {
		dst.BackoffLevel = src.BackoffLevel
	}
	if src.HalfOpenSuccesses > dst.HalfOpenSuccesses {
		dst.HalfOpenSuccesses = src.HalfOpenSuccesses
	}
	if dst.CircuitBrokenAt == nil || (src.CircuitBrokenAt != nil && src.CircuitBrokenAt.After(*dst.CircuitBrokenAt)) {
		dst.CircuitBrokenAt = src.CircuitBrokenAt
	}
	if dst.HalfOpenAt == nil || (src.HalfOpenAt != nil && src.HalfOpenAt.After(*dst.HalfOpenAt)) {
		dst.HalfOpenAt = src.HalfOpenAt
	}
	if dst.NextRetryAt == nil || (src.NextRetryAt != nil && src.NextRetryAt.After(*dst.NextRetryAt)) {
		dst.NextRetryAt = src.NextRetryAt
	}
	if m.circuitStateSeverity(src.CircuitState) > m.circuitStateSeverity(dst.CircuitState) {
		dst.CircuitState = src.CircuitState
	}
}

func (m *MetricsManager) getWritableMetricsLocked(baseURL, apiKey, serviceType string) *KeyMetrics {
	if metrics := m.getIdentityMetricsLocked(baseURL, apiKey, serviceType); metrics != nil {
		if metrics.MetricsKey != m.metricsIdentityKey(baseURL, apiKey, serviceType) {
			return m.getOrCreateKey(baseURL, apiKey, serviceType)
		}
		return metrics
	}
	return m.getOrCreateKey(baseURL, apiKey, serviceType)
}

func (m *MetricsManager) getIdentityMetricsByMultiURLLocked(baseURLs []string, apiKey, serviceType string) []*KeyMetrics {
	seen := make(map[*KeyMetrics]struct{}, len(baseURLs))
	result := make([]*KeyMetrics, 0, len(baseURLs))
	for _, baseURL := range baseURLs {
		for _, metrics := range m.getMetricsVariantsLocked(baseURL, apiKey, serviceType) {
			if _, exists := seen[metrics]; exists {
				continue
			}
			seen[metrics] = struct{}{}
			result = append(result, metrics)
		}
	}
	return result
}

func (m *MetricsManager) getIdentityMetricsByMultiURLAndKeysLocked(baseURLs, apiKeys []string, serviceType string) []*KeyMetrics {
	seen := make(map[string]struct{}, len(baseURLs)*max(1, len(apiKeys)))
	result := make([]*KeyMetrics, 0, len(baseURLs)*max(1, len(apiKeys)))
	for _, apiKey := range apiKeys {
		for _, metrics := range m.getIdentityMetricsByMultiURLLocked(baseURLs, apiKey, serviceType) {
			if _, exists := seen[metrics.MetricsKey]; exists {
				continue
			}
			seen[metrics.MetricsKey] = struct{}{}
			result = append(result, metrics)
		}
	}
	return result
}

func (m *MetricsManager) hasAvailableIdentityCandidateLocked(baseURLs, apiKeys []string, serviceType string) bool {
	seen := make(map[string]struct{}, len(baseURLs)*max(1, len(apiKeys)))
	for _, apiKey := range apiKeys {
		for _, baseURL := range baseURLs {
			identityKey := m.metricsIdentityKey(baseURL, apiKey, serviceType)
			if _, exists := seen[identityKey]; exists {
				continue
			}
			seen[identityKey] = struct{}{}
			metrics, exists := m.keyMetrics[identityKey]
			if !exists {
				for _, lookupKey := range m.metricsLookupKeys(baseURL, apiKey, serviceType) {
					if lookupKey == identityKey {
						continue
					}
					if metrics, exists = m.keyMetrics[lookupKey]; exists {
						break
					}
				}
			}
			if !exists || metrics.CircuitState != CircuitStateOpen {
				return true
			}
		}
	}
	return false
}

func (m *MetricsManager) findPendingRequestMetricsLocked(baseURL, apiKey, serviceType string, requestID uint64) *KeyMetrics {
	for _, metrics := range m.getMetricsVariantsLocked(baseURL, apiKey, serviceType) {
		if metrics == nil || metrics.pendingHistoryIdx == nil {
			continue
		}
		if _, ok := metrics.pendingHistoryIdx[requestID]; ok {
			return metrics
		}
	}
	return nil
}

// getOrCreateKey 获取或创建 Key 指标
func (m *MetricsManager) getOrCreateKey(baseURL, apiKey, serviceType string) *KeyMetrics {
	identityBaseURL := utils.MetricsIdentityBaseURL(baseURL, serviceType)
	metricsKey := generateMetricsKey(identityBaseURL, apiKey)
	if metrics, exists := m.keyMetrics[metricsKey]; exists {
		return metrics
	}

	var primary *KeyMetrics
	for _, lookupKey := range m.metricsLookupKeys(baseURL, apiKey, serviceType) {
		if lookupKey == metricsKey {
			continue
		}
		metrics, exists := m.keyMetrics[lookupKey]
		if !exists {
			continue
		}
		if primary == nil {
			primary = metrics
			continue
		}
		m.mergeKeyMetricsLocked(primary, metrics)
		delete(m.keyMetrics, lookupKey)
	}
	if primary != nil {
		primary.MetricsKey = metricsKey
		primary.BaseURL = identityBaseURL
		m.keyMetrics[metricsKey] = primary
		for _, lookupKey := range m.metricsLookupKeys(baseURL, apiKey, serviceType) {
			if lookupKey == metricsKey {
				continue
			}
			if current, exists := m.keyMetrics[lookupKey]; exists && current == primary {
				delete(m.keyMetrics, lookupKey)
			}
		}
		return primary
	}

	metrics := &KeyMetrics{
		MetricsKey:        metricsKey,
		BaseURL:           identityBaseURL,
		KeyMask:           utils.MaskAPIKey(apiKey),
		CircuitState:      CircuitStateClosed,
		recentResults:     make([]bool, 0, m.windowSize),
		breakerResults:    make([]bool, 0, m.windowSize),
		pendingHistoryIdx: make(map[uint64]int),
	}
	m.keyMetrics[metricsKey] = metrics
	return metrics
}

func normalizeFailureClass(success bool, failureClass FailureClass) FailureClass {
	if success {
		return FailureClassNone
	}
	if failureClass == FailureClassNone {
		return FailureClassRetryable
	}
	return failureClass
}

func isBreakerRelevantFailure(success bool, failureClass FailureClass) bool {
	if success {
		return true
	}
	return normalizeFailureClass(success, failureClass).IsBreakerRelevant()
}

func extractUsageTokens(usage *types.Usage) (int64, int64, int64, int64) {
	if usage == nil {
		return 0, 0, 0, 0
	}
	inputTokens := int64(usage.InputTokens)
	cacheReadTokens := int64(usage.CacheReadInputTokens)
	if usage.PromptTokensTotal > 0 && cacheReadTokens > 0 {
		normalizedInput := usage.PromptTokensTotal - usage.CacheReadInputTokens
		if normalizedInput < 0 {
			normalizedInput = 0
		}
		inputTokens = int64(normalizedInput)
	}
	outputTokens := int64(usage.OutputTokens)
	cacheCreationTokens := int64(usage.CacheCreationInputTokens)
	if cacheCreationTokens <= 0 {
		cacheCreationTokens = int64(usage.CacheCreation5mInputTokens + usage.CacheCreation1hInputTokens)
	}
	return inputTokens, outputTokens, cacheCreationTokens, cacheReadTokens
}

func (m *MetricsManager) appendToBreakerWindowKey(metrics *KeyMetrics, success bool) {
	metrics.breakerResults = append(metrics.breakerResults, success)
	if len(metrics.breakerResults) > m.windowSize {
		metrics.breakerResults = metrics.breakerResults[1:]
	}
}

func (m *MetricsManager) calculateKeyBreakerFailureRateInternal(metrics *KeyMetrics) float64 {
	if len(metrics.breakerResults) == 0 {
		return 0
	}
	failures := 0
	for _, success := range metrics.breakerResults {
		if !success {
			failures++
		}
	}
	return float64(failures) / float64(len(metrics.breakerResults))
}

func (m *MetricsManager) nextBackoffDuration(level int) time.Duration {
	if level < 0 {
		level = 0
	}
	delay := m.circuitBackoffBase
	for i := 0; i < level; i++ {
		delay *= 2
		if delay >= m.circuitBackoffMax {
			return m.circuitBackoffMax
		}
	}
	if delay > m.circuitBackoffMax {
		return m.circuitBackoffMax
	}
	return delay
}

func (m *MetricsManager) persistCircuitStateLocked(metrics *KeyMetrics) {
	if m.store == nil || metrics == nil {
		return
	}
	var circuitOpenedAt *time.Time
	if metrics.CircuitBrokenAt != nil {
		t := *metrics.CircuitBrokenAt
		circuitOpenedAt = &t
	}
	var halfOpenAt *time.Time
	if metrics.HalfOpenAt != nil {
		t := *metrics.HalfOpenAt
		halfOpenAt = &t
	}
	var nextRetryAt *time.Time
	if metrics.NextRetryAt != nil {
		t := *metrics.NextRetryAt
		nextRetryAt = &t
	}
	if err := m.store.UpsertCircuitState(PersistentCircuitState{
		MetricsKey:          metrics.MetricsKey,
		BaseURL:             metrics.BaseURL,
		KeyMask:             metrics.KeyMask,
		APIType:             m.apiType,
		CircuitState:        metrics.CircuitState.String(),
		CircuitOpenedAt:     circuitOpenedAt,
		HalfOpenAt:          halfOpenAt,
		NextRetryAt:         nextRetryAt,
		BackoffLevel:        metrics.BackoffLevel,
		HalfOpenSuccesses:   metrics.HalfOpenSuccesses,
		ConsecutiveFailures: metrics.ConsecutiveFailures,
	}); err != nil {
		log.Printf("[Metrics-Circuit] 警告: 持久化熔断状态失败 (key=%s, state=%s): %v", metrics.KeyMask, metrics.CircuitState.String(), err)
	}
}

func (m *MetricsManager) resetCircuitStateLocked(metrics *KeyMetrics, clearBreakerWindow bool) {
	metrics.CircuitState = CircuitStateClosed
	metrics.CircuitBrokenAt = nil
	metrics.HalfOpenAt = nil
	metrics.NextRetryAt = nil
	metrics.BackoffLevel = 0
	metrics.HalfOpenSuccesses = 0
	metrics.ProbeInFlight = false
	metrics.ConsecutiveFailures = 0
	if clearBreakerWindow {
		metrics.breakerResults = make([]bool, 0, m.windowSize)
	}
	m.persistCircuitStateLocked(metrics)
}

func (m *MetricsManager) moveCircuitToOpenLocked(metrics *KeyMetrics, now time.Time, escalate bool) {
	if escalate {
		metrics.BackoffLevel++
	}
	delay := m.nextBackoffDuration(metrics.BackoffLevel)
	nextRetryAt := now.Add(delay)
	metrics.CircuitState = CircuitStateOpen
	metrics.CircuitBrokenAt = &now
	metrics.HalfOpenAt = nil
	metrics.NextRetryAt = &nextRetryAt
	metrics.HalfOpenSuccesses = 0
	metrics.ProbeInFlight = false
	m.persistCircuitStateLocked(metrics)
}

func (m *MetricsManager) moveCircuitToHalfOpenLocked(metrics *KeyMetrics, now time.Time) {
	metrics.CircuitState = CircuitStateHalfOpen
	metrics.HalfOpenAt = &now
	metrics.NextRetryAt = nil
	metrics.HalfOpenSuccesses = 0
	metrics.ProbeInFlight = false
	m.persistCircuitStateLocked(metrics)
}

func (m *MetricsManager) advanceCircuitStateIfDueLocked(metrics *KeyMetrics, now time.Time) {
	if metrics == nil {
		return
	}
	if metrics.CircuitState == CircuitStateOpen && metrics.NextRetryAt != nil && !now.Before(*metrics.NextRetryAt) {
		m.moveCircuitToHalfOpenLocked(metrics, now)
	}
}

func (m *MetricsManager) handleBreakerSuccessLocked(metrics *KeyMetrics, now time.Time) {
	m.advanceCircuitStateIfDueLocked(metrics, now)
	m.appendToBreakerWindowKey(metrics, true)
	metrics.ConsecutiveFailures = 0

	switch metrics.CircuitState {
	case CircuitStateHalfOpen:
		metrics.HalfOpenSuccesses++
		if metrics.HalfOpenSuccesses >= m.halfOpenSuccessTarget {
			m.resetCircuitStateLocked(metrics, true)
			log.Printf("[Metrics-Circuit] Key [%s] (%s) half-open 探针成功，恢复 closed", metrics.KeyMask, metrics.BaseURL)
			statelog.LogStateTransition("Metrics-Circuit", "key", metrics.KeyMask, "half_open", "closed", "probe_success", "baseURL="+metrics.BaseURL)
		} else {
			m.persistCircuitStateLocked(metrics)
		}
	case CircuitStateOpen:
		m.resetCircuitStateLocked(metrics, true)
		log.Printf("[Metrics-Circuit] Key [%s] (%s) 因请求成功退出熔断状态", metrics.KeyMask, metrics.BaseURL)
	default:
		// closed 状态成功仅更新内存统计，不需要同步持久化熔断状态
	}
}

func (m *MetricsManager) handleBreakerFailureLocked(metrics *KeyMetrics, failureClass FailureClass, now time.Time) {
	failureClass = normalizeFailureClass(false, failureClass)
	m.advanceCircuitStateIfDueLocked(metrics, now)
	if !failureClass.IsBreakerRelevant() {
		// 非 breaker 相关失败仅更新观测统计，不需要同步持久化熔断状态
		return
	}

	metrics.ConsecutiveFailures++
	m.appendToBreakerWindowKey(metrics, false)

	switch metrics.CircuitState {
	case CircuitStateHalfOpen:
		m.moveCircuitToOpenLocked(metrics, now, true)
		log.Printf("[Metrics-Circuit] Key [%s] (%s) half-open 探针失败，重新进入 open（失败率: %.1f%%）", metrics.KeyMask, metrics.BaseURL, m.calculateKeyBreakerFailureRateInternal(metrics)*100)
	case CircuitStateClosed:
		if m.isKeyCircuitBroken(metrics) {
			m.moveCircuitToOpenLocked(metrics, now, false)
			log.Printf("[Metrics-Circuit] Key [%s] (%s) 进入熔断状态（失败率: %.1f%%）", metrics.KeyMask, metrics.BaseURL, m.calculateKeyBreakerFailureRateInternal(metrics)*100)
			statelog.LogStateTransition("Metrics-Circuit", "key", metrics.KeyMask, "closed", "open", "breaker_threshold", "baseURL="+metrics.BaseURL)
		}
	default:
		// open 状态下继续记录内存统计，持久化仅在状态迁移时发生
	}
}

// RecordSuccess 记录成功请求（新方法，使用 baseURL + apiKey）
func (m *MetricsManager) RecordSuccess(baseURL, apiKey, serviceType string) {
	m.RecordSuccessWithUsage(baseURL, apiKey, serviceType, nil)
}

// RecordSuccessWithUsage 记录成功请求（带 Usage 数据）
func (m *MetricsManager) RecordSuccessWithUsage(baseURL, apiKey, serviceType string, usage *types.Usage) {
	m.mu.Lock()
	defer m.mu.Unlock()

	m.recordSuccessWithUsageLocked(baseURL, apiKey, serviceType, usage, time.Now())
}

func (m *MetricsManager) recordSuccessWithUsageLocked(baseURL, apiKey, serviceType string, usage *types.Usage, now time.Time) {
	metrics := m.getWritableMetricsLocked(baseURL, apiKey, serviceType)
	metrics.RequestCount++
	metrics.SuccessCount++
	metrics.LastSuccessAt = &now

	m.appendToWindowKey(metrics, true)
	m.handleBreakerSuccessLocked(metrics, now)

	inputTokens, outputTokens, cacheCreationTokens, cacheReadTokens := extractUsageTokens(usage)

	m.appendToHistoryKeyWithUsage(metrics, now, true, FailureClassNone, inputTokens, outputTokens, cacheCreationTokens, cacheReadTokens)

	if m.store != nil {
		m.store.AddRecord(PersistentRecord{
			MetricsKey:          metrics.MetricsKey,
			BaseURL:             metrics.BaseURL,
			KeyMask:             metrics.KeyMask,
			Timestamp:           now,
			Success:             true,
			FailureClass:        FailureClassNone,
			InputTokens:         inputTokens,
			OutputTokens:        outputTokens,
			CacheCreationTokens: cacheCreationTokens,
			CacheReadTokens:     cacheReadTokens,
			APIType:             m.apiType,
		})
	}
}

// RecordFailure 记录失败请求（新方法，使用 baseURL + apiKey）
func (m *MetricsManager) RecordFailure(baseURL, apiKey, serviceType string) {
	m.RecordFailureWithClass(baseURL, apiKey, serviceType, FailureClassRetryable)
}

// RecordFailureWithClass 记录失败请求并指定失败分类。
func (m *MetricsManager) RecordFailureWithClass(baseURL, apiKey, serviceType string, failureClass FailureClass) {
	m.mu.Lock()
	defer m.mu.Unlock()

	m.recordFailureLocked(baseURL, apiKey, serviceType, normalizeFailureClass(false, failureClass), time.Now())
}

func (m *MetricsManager) recordFailureLocked(baseURL, apiKey, serviceType string, failureClass FailureClass, now time.Time) {
	metrics := m.getWritableMetricsLocked(baseURL, apiKey, serviceType)
	metrics.RequestCount++
	metrics.FailureCount++
	metrics.LastFailureAt = &now

	m.appendToWindowKey(metrics, false)
	m.handleBreakerFailureLocked(metrics, failureClass, now)
	m.appendToHistoryKey(metrics, now, false, normalizeFailureClass(false, failureClass))

	if m.store != nil {
		m.store.AddRecord(PersistentRecord{
			MetricsKey:          metrics.MetricsKey,
			BaseURL:             metrics.BaseURL,
			KeyMask:             metrics.KeyMask,
			Timestamp:           now,
			Success:             false,
			FailureClass:        normalizeFailureClass(false, failureClass),
			InputTokens:         0,
			OutputTokens:        0,
			CacheCreationTokens: 0,
			CacheReadTokens:     0,
			APIType:             m.apiType,
		})
	}
}

// RecordRequestConnected 记录“开始发起上游请求（TCP 建连阶段）”的请求（用于更实时的活跃度统计）。
// 返回 requestID，用于后续在请求结束时回写成功/失败与 token。
func (m *MetricsManager) RecordRequestConnected(baseURL, apiKey, serviceType string, model string) uint64 {
	return m.RecordRequestConnectedAt(baseURL, apiKey, serviceType, model, time.Now())
}

// RecordRequestConnectedAt 与 RecordRequestConnected 相同，但允许注入时间戳（用于测试）。
func (m *MetricsManager) RecordRequestConnectedAt(baseURL, apiKey, serviceType string, model string, timestamp time.Time) uint64 {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getWritableMetricsLocked(baseURL, apiKey, serviceType)
	m.advanceCircuitStateIfDueLocked(metrics, timestamp)

	m.nextRequestID++
	requestID := m.nextRequestID

	if metrics.pendingHistoryIdx == nil {
		metrics.pendingHistoryIdx = make(map[uint64]int)
	}

	metrics.requestHistory = append(metrics.requestHistory, RequestRecord{
		Timestamp:    timestamp,
		Success:      true, // 先按成功计数；结束时会回写真实结果
		FailureClass: FailureClassNone,
		Model:        model,
	})
	metrics.pendingHistoryIdx[requestID] = len(metrics.requestHistory) - 1

	m.cleanupHistoryLocked(metrics)

	return requestID
}

// RecordRequestFinalizeSuccess 回写成功结果与 token（requestID 来自 RecordRequestConnected）。
func (m *MetricsManager) RecordRequestFinalizeSuccess(baseURL, apiKey, serviceType string, requestID uint64, usage *types.Usage) {
	m.RecordRequestFinalizeOutcome(baseURL, apiKey, serviceType, requestID, true, FailureClassNone, usage)
}

// RecordRequestFinalizeFailure 回写失败结果（requestID 来自 RecordRequestConnected）。
func (m *MetricsManager) RecordRequestFinalizeFailure(baseURL, apiKey, serviceType string, requestID uint64) {
	m.RecordRequestFinalizeFailureWithClass(baseURL, apiKey, serviceType, requestID, FailureClassRetryable)
}

// RecordRequestFinalizeFailureWithClass 回写失败结果并显式指定失败分类。
func (m *MetricsManager) RecordRequestFinalizeFailureWithClass(baseURL, apiKey, serviceType string, requestID uint64, failureClass FailureClass) {
	m.RecordRequestFinalizeOutcome(baseURL, apiKey, serviceType, requestID, false, failureClass, nil)
}

// RecordRequestFinalizeOutcome 根据最终结果统一回写请求指标与 breaker 状态。
func (m *MetricsManager) RecordRequestFinalizeOutcome(baseURL, apiKey, serviceType string, requestID uint64, success bool, failureClass FailureClass, usage *types.Usage) {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.findPendingRequestMetricsLocked(baseURL, apiKey, serviceType, requestID)
	if metrics == nil {
		metrics = m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	}
	if metrics != nil && metrics.MetricsKey != m.metricsIdentityKey(baseURL, apiKey, serviceType) {
		metrics = m.getOrCreateKey(baseURL, apiKey, serviceType)
	}
	if metrics == nil {
		if success {
			m.recordSuccessWithUsageLocked(baseURL, apiKey, serviceType, usage, time.Now())
		} else {
			m.recordFailureLocked(baseURL, apiKey, serviceType, normalizeFailureClass(false, failureClass), time.Now())
		}
		return
	}

	idx, ok := metrics.pendingHistoryIdx[requestID]
	if !ok || idx < 0 || idx >= len(metrics.requestHistory) {
		if success {
			m.recordSuccessWithUsageLocked(baseURL, apiKey, serviceType, usage, time.Now())
		} else {
			m.recordFailureLocked(baseURL, apiKey, serviceType, normalizeFailureClass(false, failureClass), time.Now())
		}
		return
	}
	delete(metrics.pendingHistoryIdx, requestID)

	now := time.Now()
	metrics.RequestCount++
	record := &metrics.requestHistory[idx]
	record.Success = success
	record.FailureClass = normalizeFailureClass(success, failureClass)

	if success {
		metrics.SuccessCount++
		metrics.LastSuccessAt = &now
		m.appendToWindowKey(metrics, true)
		m.handleBreakerSuccessLocked(metrics, now)

		inputTokens, outputTokens, cacheCreationTokens, cacheReadTokens := extractUsageTokens(usage)
		record.InputTokens = inputTokens
		record.OutputTokens = outputTokens
		record.CacheCreationInputTokens = cacheCreationTokens
		record.CacheReadInputTokens = cacheReadTokens

		if m.store != nil {
			m.store.AddRecord(PersistentRecord{
				MetricsKey:          metrics.MetricsKey,
				BaseURL:             metrics.BaseURL,
				KeyMask:             metrics.KeyMask,
				Timestamp:           record.Timestamp,
				Success:             true,
				FailureClass:        FailureClassNone,
				InputTokens:         inputTokens,
				OutputTokens:        outputTokens,
				CacheCreationTokens: cacheCreationTokens,
				CacheReadTokens:     cacheReadTokens,
				APIType:             m.apiType,
				Model:               record.Model,
			})
		}
		return
	}

	failureClass = normalizeFailureClass(false, failureClass)
	metrics.FailureCount++
	metrics.LastFailureAt = &now
	m.appendToWindowKey(metrics, false)
	m.handleBreakerFailureLocked(metrics, failureClass, now)
	record.InputTokens = 0
	record.OutputTokens = 0
	record.CacheCreationInputTokens = 0
	record.CacheReadInputTokens = 0

	if m.store != nil {
		m.store.AddRecord(PersistentRecord{
			MetricsKey:          metrics.MetricsKey,
			BaseURL:             metrics.BaseURL,
			KeyMask:             metrics.KeyMask,
			Timestamp:           record.Timestamp,
			Success:             false,
			FailureClass:        failureClass,
			InputTokens:         0,
			OutputTokens:        0,
			CacheCreationTokens: 0,
			CacheReadTokens:     0,
			APIType:             m.apiType,
			Model:               record.Model,
		})
	}
}

// RecordRequestFinalizeClientCancel 记录客户端取消的请求（计入总请求数但不计入失败）
func (m *MetricsManager) RecordRequestFinalizeClientCancel(baseURL, apiKey, serviceType string, requestID uint64) {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.findPendingRequestMetricsLocked(baseURL, apiKey, serviceType, requestID)
	if metrics == nil {
		metrics = m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	}
	if metrics != nil && metrics.MetricsKey != m.metricsIdentityKey(baseURL, apiKey, serviceType) {
		metrics = m.getOrCreateKey(baseURL, apiKey, serviceType)
	}
	if metrics == nil {
		return
	}

	idx, ok := metrics.pendingHistoryIdx[requestID]
	if !ok || idx < 0 || idx >= len(metrics.requestHistory) {
		return
	}
	delete(metrics.pendingHistoryIdx, requestID)

	// 仅计入总请求数，不计入失败数
	metrics.RequestCount++
	// 注意：不重置 ConsecutiveFailures，客户端取消不应影响连续失败计数

	// 不更新滑动窗口（不影响失败率计算）
	// 不检查熔断状态（客户端取消不应触发熔断）

	// 从历史记录中移除（客户端取消不记录）
	metrics.requestHistory = append(metrics.requestHistory[:idx], metrics.requestHistory[idx+1:]...)
	// 更新后续索引
	for rid, ridx := range metrics.pendingHistoryIdx {
		if ridx > idx {
			metrics.pendingHistoryIdx[rid] = ridx - 1
		}
	}
}

// RecordRequestStart 记录请求开始（增加进行中计数）
func (m *MetricsManager) RecordRequestStart(baseURL, apiKey, serviceType string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getWritableMetricsLocked(baseURL, apiKey, serviceType)
	metrics.ActiveRequests++
}

// RecordRequestEnd 记录请求结束（减少进行中计数）
func (m *MetricsManager) RecordRequestEnd(baseURL, apiKey, serviceType string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics != nil {
		if metrics.ActiveRequests > 0 {
			metrics.ActiveRequests--
		}
	}
}

// isKeyCircuitBroken 判断 Key 是否达到熔断条件（内部方法，调用前需持有锁）
func (m *MetricsManager) isKeyCircuitBroken(metrics *KeyMetrics) bool {
	if metrics == nil {
		return false
	}
	if metrics.ConsecutiveFailures >= consecutiveRetryableFailuresThreshold {
		return true
	}
	minRequests := max(3, m.windowSize/2)
	if len(metrics.breakerResults) < minRequests {
		return false
	}
	return m.calculateKeyBreakerFailureRateInternal(metrics) >= m.failureThreshold
}

// calculateKeyFailureRateInternal 计算 Key 综合失败率（内部方法，调用前需持有锁）
func (m *MetricsManager) calculateKeyFailureRateInternal(metrics *KeyMetrics) float64 {
	if len(metrics.recentResults) == 0 {
		return 0
	}
	failures := 0
	for _, success := range metrics.recentResults {
		if !success {
			failures++
		}
	}
	return float64(failures) / float64(len(metrics.recentResults))
}

// appendToWindowKey 向 Key 滑动窗口添加记录
func (m *MetricsManager) appendToWindowKey(metrics *KeyMetrics, success bool) {
	metrics.recentResults = append(metrics.recentResults, success)
	// 保持窗口大小
	if len(metrics.recentResults) > m.windowSize {
		metrics.recentResults = metrics.recentResults[1:]
	}
}

// appendToHistoryKey 向 Key 历史记录添加请求（保留24小时）
func (m *MetricsManager) appendToHistoryKey(metrics *KeyMetrics, timestamp time.Time, success bool, failureClass FailureClass) {
	m.appendToHistoryKeyWithUsage(metrics, timestamp, success, failureClass, 0, 0, 0, 0)
}

// cleanupHistoryLocked 清理超过 24 小时的历史记录，并同步修正 pendingHistoryIdx 索引。
// 注意：调用方需要持有写锁。
func (m *MetricsManager) cleanupHistoryLocked(metrics *KeyMetrics) {
	if metrics == nil || len(metrics.requestHistory) == 0 {
		return
	}

	cutoff := time.Now().Add(-24 * time.Hour)

	newStart := -1
	for i, record := range metrics.requestHistory {
		if record.Timestamp.After(cutoff) {
			newStart = i
			break
		}
	}

	if newStart > 0 {
		metrics.requestHistory = metrics.requestHistory[newStart:]
		// 索引平移：老数据被切走后，pending 索引需要整体减去 newStart
		if metrics.pendingHistoryIdx != nil && len(metrics.pendingHistoryIdx) > 0 {
			for id, idx := range metrics.pendingHistoryIdx {
				if idx < newStart {
					delete(metrics.pendingHistoryIdx, id)
					continue
				}
				metrics.pendingHistoryIdx[id] = idx - newStart
			}
		}
		return
	}

	if newStart == -1 {
		// 所有记录都过期，清空切片
		metrics.requestHistory = metrics.requestHistory[:0]
		if metrics.pendingHistoryIdx != nil {
			for id := range metrics.pendingHistoryIdx {
				delete(metrics.pendingHistoryIdx, id)
			}
		}
	}
}

// appendToHistoryKeyWithUsage 向 Key 历史记录添加请求（带 Usage 数据）
func (m *MetricsManager) appendToHistoryKeyWithUsage(metrics *KeyMetrics, timestamp time.Time, success bool, failureClass FailureClass, inputTokens, outputTokens, cacheCreationTokens, cacheReadTokens int64) {
	metrics.requestHistory = append(metrics.requestHistory, RequestRecord{
		Timestamp:                timestamp,
		Success:                  success,
		FailureClass:             normalizeFailureClass(success, failureClass),
		InputTokens:              inputTokens,
		OutputTokens:             outputTokens,
		CacheCreationInputTokens: cacheCreationTokens,
		CacheReadInputTokens:     cacheReadTokens,
	})

	// 清理超过 24 小时的记录
	m.cleanupHistoryLocked(metrics)
}

// IsKeyHealthy 判断单个 Key 是否健康
func (m *MetricsManager) IsKeyHealthy(baseURL, apiKey, serviceType string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics == nil {
		return true // 没有记录，默认健康
	}
	m.advanceCircuitStateIfDueLocked(metrics, time.Now())
	if metrics.CircuitState == CircuitStateOpen {
		return false
	}
	if len(metrics.breakerResults) == 0 {
		return true
	}

	return m.calculateKeyBreakerFailureRateInternal(metrics) < m.failureThreshold
}

// IsChannelHealthy 判断渠道是否健康（基于当前活跃 Keys 聚合计算）
// activeKeys: 当前渠道配置的所有活跃 API Keys
func (m *MetricsManager) IsChannelHealthyWithKeys(baseURL string, activeKeys []string, serviceType string) bool {
	if len(activeKeys) == 0 {
		return false // 没有 Key，不健康
	}

	m.mu.Lock()
	defer m.mu.Unlock()

	var totalResults []bool
	hasOpenOnly := false
	hasAvailableCandidate := false
	now := time.Now()
	for _, apiKey := range activeKeys {
		variants := m.getMetricsVariantsLocked(baseURL, apiKey, serviceType)
		if len(variants) == 0 {
			hasAvailableCandidate = true
			continue
		}
		for _, metrics := range variants {
			m.advanceCircuitStateIfDueLocked(metrics, now)
			if metrics.CircuitState == CircuitStateOpen {
				hasOpenOnly = true
				continue
			}
			hasAvailableCandidate = true
			totalResults = append(totalResults, metrics.breakerResults...)
		}
	}

	if len(totalResults) == 0 {
		if hasOpenOnly && !hasAvailableCandidate {
			return false
		}
		return true
	}

	minRequests := max(3, m.windowSize/2)
	if len(totalResults) < minRequests {
		return true
	}

	failures := 0
	for _, success := range totalResults {
		if !success {
			failures++
		}
	}
	failureRate := float64(failures) / float64(len(totalResults))

	return failureRate < m.failureThreshold
}

// IsChannelHealthyMultiURL 判断多 BaseURL 聚合渠道是否健康。
func (m *MetricsManager) IsChannelHealthyMultiURL(baseURLs []string, activeKeys []string, serviceType string) bool {
	if len(baseURLs) == 0 {
		return false
	}
	if len(activeKeys) == 0 {
		return false
	}

	m.mu.Lock()
	defer m.mu.Unlock()

	var totalResults []bool
	hasOpenOnly := false
	hasAvailableCandidate := m.hasAvailableIdentityCandidateLocked(baseURLs, activeKeys, serviceType)
	now := time.Now()
	for _, metrics := range m.getIdentityMetricsByMultiURLAndKeysLocked(baseURLs, activeKeys, serviceType) {
		m.advanceCircuitStateIfDueLocked(metrics, now)
		if metrics.CircuitState == CircuitStateOpen {
			hasOpenOnly = true
			continue
		}
		hasAvailableCandidate = true
		totalResults = append(totalResults, metrics.breakerResults...)
	}
	if len(totalResults) == 0 {
		if hasOpenOnly && !hasAvailableCandidate {
			return false
		}
		return true
	}

	minRequests := max(3, m.windowSize/2)
	if len(totalResults) < minRequests {
		return true
	}

	failures := 0
	for _, success := range totalResults {
		if !success {
			failures++
		}
	}
	failureRate := float64(failures) / float64(len(totalResults))
	return failureRate < m.failureThreshold
}

// CalculateChannelFailureRateMultiURL 计算多 BaseURL 聚合 breaker 失败率。
func (m *MetricsManager) CalculateChannelFailureRateMultiURL(baseURLs []string, activeKeys []string, serviceType string) float64 {
	if len(baseURLs) == 0 || len(activeKeys) == 0 {
		return 0
	}

	m.mu.Lock()
	defer m.mu.Unlock()

	var totalResults []bool
	hasOpenOnly := false
	hasAvailableCandidate := m.hasAvailableIdentityCandidateLocked(baseURLs, activeKeys, serviceType)
	now := time.Now()
	for _, metrics := range m.getIdentityMetricsByMultiURLAndKeysLocked(baseURLs, activeKeys, serviceType) {
		m.advanceCircuitStateIfDueLocked(metrics, now)
		if metrics.CircuitState == CircuitStateOpen {
			hasOpenOnly = true
			continue
		}
		hasAvailableCandidate = true
		totalResults = append(totalResults, metrics.breakerResults...)
	}

	if len(totalResults) == 0 {
		if hasOpenOnly && !hasAvailableCandidate {
			return 1
		}
		return 0
	}

	failures := 0
	for _, success := range totalResults {
		if !success {
			failures++
		}
	}
	return float64(failures) / float64(len(totalResults))
}

// CalculateKeyFailureRate 计算单个 Key 的 breaker 失败率
func (m *MetricsManager) CalculateKeyFailureRate(baseURL, apiKey, serviceType string) float64 {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics == nil {
		return 0
	}
	m.advanceCircuitStateIfDueLocked(metrics, time.Now())
	if metrics.CircuitState == CircuitStateOpen && len(metrics.breakerResults) == 0 {
		return 1
	}

	return m.calculateKeyBreakerFailureRateInternal(metrics)
}

// CalculateChannelFailureRate 计算渠道聚合 breaker 失败率
func (m *MetricsManager) CalculateChannelFailureRate(baseURL string, activeKeys []string, serviceType string) float64 {
	if len(activeKeys) == 0 {
		return 0
	}

	m.mu.Lock()
	defer m.mu.Unlock()

	var totalResults []bool
	hasOpenOnly := false
	hasAvailableCandidate := false
	now := time.Now()
	for _, apiKey := range activeKeys {
		variants := m.getMetricsVariantsLocked(baseURL, apiKey, serviceType)
		if len(variants) == 0 {
			hasAvailableCandidate = true
			continue
		}
		for _, metrics := range variants {
			m.advanceCircuitStateIfDueLocked(metrics, now)
			if metrics.CircuitState == CircuitStateOpen {
				hasOpenOnly = true
				continue
			}
			hasAvailableCandidate = true
			totalResults = append(totalResults, metrics.breakerResults...)
		}
	}

	if len(totalResults) == 0 {
		if hasOpenOnly && !hasAvailableCandidate {
			return 1
		}
		return 0
	}

	failures := 0
	for _, success := range totalResults {
		if !success {
			failures++
		}
	}

	return float64(failures) / float64(len(totalResults))
}

// GetKeyMetrics 获取单个 Key 的指标
func (m *MetricsManager) GetKeyMetrics(baseURL, apiKey, serviceType string) *KeyMetrics {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics != nil {
		m.advanceCircuitStateIfDueLocked(metrics, time.Now())
		return &KeyMetrics{
			MetricsKey:          metrics.MetricsKey,
			BaseURL:             metrics.BaseURL,
			KeyMask:             metrics.KeyMask,
			RequestCount:        metrics.RequestCount,
			SuccessCount:        metrics.SuccessCount,
			FailureCount:        metrics.FailureCount,
			ConsecutiveFailures: metrics.ConsecutiveFailures,
			ActiveRequests:      metrics.ActiveRequests,
			LastSuccessAt:       metrics.LastSuccessAt,
			LastFailureAt:       metrics.LastFailureAt,
			CircuitBrokenAt:     metrics.CircuitBrokenAt,
			CircuitState:        metrics.CircuitState,
			HalfOpenAt:          metrics.HalfOpenAt,
			NextRetryAt:         metrics.NextRetryAt,
			BackoffLevel:        metrics.BackoffLevel,
			HalfOpenSuccesses:   metrics.HalfOpenSuccesses,
		}
	}
	return nil
}

// GetChannelAggregatedMetrics 获取渠道聚合指标（基于活跃 Keys）
func (m *MetricsManager) GetChannelAggregatedMetrics(channelIndex int, baseURL string, activeKeys []string, serviceType string) *ChannelMetrics {
	m.mu.Lock()
	defer m.mu.Unlock()

	aggregated := &ChannelMetrics{
		ChannelIndex: channelIndex,
		CircuitState: CircuitStateClosed,
	}

	var latestSuccess, latestFailure, latestCircuitBroken, latestNextRetry *time.Time
	var maxConsecutiveFailures int64
	var maxHalfOpenSuccesses int
	channelState := CircuitStateClosed

	for _, apiKey := range activeKeys {
		for _, metrics := range m.getMetricsVariantsLocked(baseURL, apiKey, serviceType) {
			m.advanceCircuitStateIfDueLocked(metrics, time.Now())
			aggregated.RequestCount += metrics.RequestCount
			aggregated.SuccessCount += metrics.SuccessCount
			aggregated.FailureCount += metrics.FailureCount
			if metrics.ConsecutiveFailures > maxConsecutiveFailures {
				maxConsecutiveFailures = metrics.ConsecutiveFailures
			}
			if metrics.HalfOpenSuccesses > maxHalfOpenSuccesses {
				maxHalfOpenSuccesses = metrics.HalfOpenSuccesses
			}
			aggregated.recentResults = append(aggregated.recentResults, metrics.recentResults...)
			aggregated.breakerResults = append(aggregated.breakerResults, metrics.breakerResults...)
			aggregated.requestHistory = append(aggregated.requestHistory, metrics.requestHistory...)

			if metrics.LastSuccessAt != nil && (latestSuccess == nil || metrics.LastSuccessAt.After(*latestSuccess)) {
				latestSuccess = metrics.LastSuccessAt
			}
			if metrics.LastFailureAt != nil && (latestFailure == nil || metrics.LastFailureAt.After(*latestFailure)) {
				latestFailure = metrics.LastFailureAt
			}
			if metrics.CircuitBrokenAt != nil && (latestCircuitBroken == nil || metrics.CircuitBrokenAt.After(*latestCircuitBroken)) {
				latestCircuitBroken = metrics.CircuitBrokenAt
			}
			if metrics.NextRetryAt != nil && (latestNextRetry == nil || metrics.NextRetryAt.After(*latestNextRetry)) {
				latestNextRetry = metrics.NextRetryAt
			}
			if metrics.CircuitState > channelState {
				channelState = metrics.CircuitState
			}
		}
	}

	aggregated.LastSuccessAt = latestSuccess
	aggregated.LastFailureAt = latestFailure
	aggregated.CircuitBrokenAt = latestCircuitBroken
	aggregated.NextRetryAt = latestNextRetry
	aggregated.CircuitState = channelState
	aggregated.HalfOpenSuccesses = maxHalfOpenSuccesses
	aggregated.ConsecutiveFailures = maxConsecutiveFailures

	return aggregated
}

// KeyUsageInfo Key 使用信息（用于排序筛选）
type KeyUsageInfo struct {
	APIKey       string
	KeyMask      string
	RequestCount int64
	LastUsedAt   *time.Time
}

// GetChannelKeyUsageInfo 获取渠道下所有 Key 的使用信息（用于排序筛选）
// 返回的 keys 已按最近使用时间排序
func (m *MetricsManager) GetChannelKeyUsageInfo(baseURL string, apiKeys []string, serviceType string) []KeyUsageInfo {
	m.mu.RLock()
	defer m.mu.RUnlock()

	infos := make([]KeyUsageInfo, 0, len(apiKeys))

	for _, apiKey := range apiKeys {
		metrics := m.getIdentityMetricsLocked(baseURL, apiKey, serviceType)
		if metrics == nil {
			infos = append(infos, KeyUsageInfo{
				APIKey:       apiKey,
				KeyMask:      utils.MaskAPIKey(apiKey),
				RequestCount: 0,
				LastUsedAt:   nil,
			})
			continue
		}
		usedAt := metrics.LastSuccessAt
		if usedAt == nil {
			usedAt = metrics.LastFailureAt
		}
		infos = append(infos, KeyUsageInfo{
			APIKey:       apiKey,
			KeyMask:      metrics.KeyMask,
			RequestCount: metrics.RequestCount,
			LastUsedAt:   usedAt,
		})
	}

	// 按最近使用时间排序（最近的在前面）
	sort.Slice(infos, func(i, j int) bool {
		if infos[i].LastUsedAt == nil && infos[j].LastUsedAt == nil {
			return infos[i].RequestCount > infos[j].RequestCount // 都未使用时，按访问量排序
		}
		if infos[i].LastUsedAt == nil {
			return false // i 未使用，排后面
		}
		if infos[j].LastUsedAt == nil {
			return true // j 未使用，i 排前面
		}
		return infos[i].LastUsedAt.After(*infos[j].LastUsedAt)
	})

	return infos
}

// GetChannelKeyUsageInfoMultiURL 获取渠道 Key 使用信息（支持多 URL 聚合）
func (m *MetricsManager) GetChannelKeyUsageInfoMultiURL(baseURLs []string, apiKeys []string, serviceType string) []KeyUsageInfo {
	if len(baseURLs) == 0 {
		return []KeyUsageInfo{}
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	infos := make([]KeyUsageInfo, 0, len(apiKeys))

	for _, apiKey := range apiKeys {
		var keyMask string
		var requestCount int64
		var lastUsedAt *time.Time

		for _, metrics := range m.getIdentityMetricsByMultiURLLocked(baseURLs, apiKey, serviceType) {
			if keyMask == "" {
				keyMask = metrics.KeyMask
			}
			requestCount += metrics.RequestCount
			usedAt := metrics.LastSuccessAt
			if usedAt == nil {
				usedAt = metrics.LastFailureAt
			}
			if usedAt != nil && (lastUsedAt == nil || usedAt.After(*lastUsedAt)) {
				lastUsedAt = usedAt
			}
		}

		if keyMask == "" {
			keyMask = utils.MaskAPIKey(apiKey)
		}

		infos = append(infos, KeyUsageInfo{
			APIKey:       apiKey,
			KeyMask:      keyMask,
			RequestCount: requestCount,
			LastUsedAt:   lastUsedAt,
		})
	}

	// 按最近使用时间排序（最近的在前面）
	sort.Slice(infos, func(i, j int) bool {
		if infos[i].LastUsedAt == nil && infos[j].LastUsedAt == nil {
			return infos[i].RequestCount > infos[j].RequestCount // 都未使用时，按访问量排序
		}
		if infos[i].LastUsedAt == nil {
			return false // i 未使用，排后面
		}
		if infos[j].LastUsedAt == nil {
			return true // j 未使用，i 排前面
		}
		return infos[i].LastUsedAt.After(*infos[j].LastUsedAt)
	})

	return infos
}

// SelectTopKeys 筛选展示的 Key
// 策略：先取最近使用的 5 个，再从其他 Key 中按访问量补全到 10 个
func SelectTopKeys(infos []KeyUsageInfo, maxDisplay int) []KeyUsageInfo {
	if len(infos) <= maxDisplay {
		return infos
	}

	// 分离：最近使用的和未使用的
	var recentKeys []KeyUsageInfo
	var otherKeys []KeyUsageInfo

	for i, info := range infos {
		if i < 5 {
			recentKeys = append(recentKeys, info)
		} else {
			otherKeys = append(otherKeys, info)
		}
	}

	// 其他 Key 按访问量排序（降序）
	sort.Slice(otherKeys, func(i, j int) bool {
		return otherKeys[i].RequestCount > otherKeys[j].RequestCount
	})

	// 补全到 maxDisplay 个
	result := make([]KeyUsageInfo, 0, maxDisplay)
	result = append(result, recentKeys...)

	needCount := maxDisplay - len(recentKeys)
	if needCount > 0 && len(otherKeys) > 0 {
		if len(otherKeys) > needCount {
			otherKeys = otherKeys[:needCount]
		}
		result = append(result, otherKeys...)
	}

	return result
}

// GetAllKeyMetrics 获取所有 Key 的指标
func (m *MetricsManager) GetAllKeyMetrics() []*KeyMetrics {
	m.mu.Lock()
	defer m.mu.Unlock()

	result := make([]*KeyMetrics, 0, len(m.keyMetrics))
	now := time.Now()
	for _, metrics := range m.keyMetrics {
		m.advanceCircuitStateIfDueLocked(metrics, now)
		result = append(result, &KeyMetrics{
			MetricsKey:          metrics.MetricsKey,
			BaseURL:             metrics.BaseURL,
			KeyMask:             metrics.KeyMask,
			RequestCount:        metrics.RequestCount,
			SuccessCount:        metrics.SuccessCount,
			FailureCount:        metrics.FailureCount,
			ConsecutiveFailures: metrics.ConsecutiveFailures,
			ActiveRequests:      metrics.ActiveRequests,
			LastSuccessAt:       metrics.LastSuccessAt,
			LastFailureAt:       metrics.LastFailureAt,
			CircuitBrokenAt:     metrics.CircuitBrokenAt,
			CircuitState:        metrics.CircuitState,
			HalfOpenAt:          metrics.HalfOpenAt,
			NextRetryAt:         metrics.NextRetryAt,
			BackoffLevel:        metrics.BackoffLevel,
			HalfOpenSuccesses:   metrics.HalfOpenSuccesses,
		})
	}
	return result
}

// GetTimeWindowStatsForKey 获取指定 Key 在时间窗口内的统计
func (m *MetricsManager) GetTimeWindowStatsForKey(baseURL, apiKey, serviceType string, duration time.Duration) TimeWindowStats {
	m.mu.RLock()
	defer m.mu.RUnlock()

	cutoff := time.Now().Add(-duration)
	var requestCount, successCount, failureCount int64

	metrics := m.getIdentityMetricsLocked(baseURL, apiKey, serviceType)
	if metrics != nil {
		for _, record := range metrics.requestHistory {
			if record.Timestamp.After(cutoff) {
				requestCount++
				if record.Success {
					successCount++
				} else {
					failureCount++
				}
			}
		}
	}

	if requestCount == 0 {
		return TimeWindowStats{SuccessRate: 100}
	}

	successRate := float64(successCount) / float64(requestCount) * 100

	return TimeWindowStats{
		RequestCount: requestCount,
		SuccessCount: successCount,
		FailureCount: failureCount,
		SuccessRate:  successRate,
	}
}

// GetAllTimeWindowStatsForKey 获取单个 Key 所有时间窗口的统计
func (m *MetricsManager) GetAllTimeWindowStatsForKey(baseURL, apiKey, serviceType string) map[string]TimeWindowStats {
	return map[string]TimeWindowStats{
		"15m": m.GetTimeWindowStatsForKey(baseURL, apiKey, serviceType, 15*time.Minute),
		"1h":  m.GetTimeWindowStatsForKey(baseURL, apiKey, serviceType, 1*time.Hour),
		"6h":  m.GetTimeWindowStatsForKey(baseURL, apiKey, serviceType, 6*time.Hour),
		"24h": m.GetTimeWindowStatsForKey(baseURL, apiKey, serviceType, 24*time.Hour),
	}
}

// MoveKeyToHalfOpen 强制将指定 Key 切换到 half-open 探测状态。
func (m *MetricsManager) MoveKeyToHalfOpen(baseURL, apiKey, serviceType string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getOrCreateKey(baseURL, apiKey, serviceType)
	m.moveCircuitToHalfOpenLocked(metrics, time.Now())
	log.Printf("[Metrics-Circuit] Key [%s] (%s) 已切换到 half-open", metrics.KeyMask, metrics.BaseURL)
}

// ResetKeyFailureState 重置单个 Key 的熔断/失败状态（保留历史统计与总量计数）。
// 用于“恢复熔断”场景：清零连续失败、清空 breaker 滑动窗口、解除熔断标记。
func (m *MetricsManager) ResetKeyFailureState(baseURL, apiKey, serviceType string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics != nil {
		metrics.recentResults = make([]bool, 0, m.windowSize)
		m.resetCircuitStateLocked(metrics, true)
		log.Printf("[Metrics-Reset] Key [%s] (%s) 熔断状态已重置（保留历史统计）", metrics.KeyMask, metrics.BaseURL)
	}
}

// ResetKey 重置单个 Key 的指标
func (m *MetricsManager) ResetKey(baseURL, apiKey, serviceType string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics != nil {
		metrics.RequestCount = 0
		metrics.SuccessCount = 0
		metrics.FailureCount = 0
		metrics.ActiveRequests = 0
		metrics.LastSuccessAt = nil
		metrics.LastFailureAt = nil
		metrics.recentResults = make([]bool, 0, m.windowSize)
		metrics.breakerResults = make([]bool, 0, m.windowSize)
		metrics.requestHistory = nil
		m.resetCircuitStateLocked(metrics, true)
		if metrics.pendingHistoryIdx != nil {
			for id := range metrics.pendingHistoryIdx {
				delete(metrics.pendingHistoryIdx, id)
			}
		}
		log.Printf("[Metrics-Reset] Key [%s] (%s) 指标已完全重置", metrics.KeyMask, metrics.BaseURL)
	}
}

// ResetAll 重置所有指标
func (m *MetricsManager) ResetAll() {
	m.mu.Lock()
	defer m.mu.Unlock()

	m.keyMetrics = make(map[string]*KeyMetrics)
}

// Stop 停止后台清理任务
func (m *MetricsManager) Stop() {
	close(m.stopCh)
}

// DeleteKeysForChannel 删除指定渠道的所有内存指标
// baseURLs: 渠道的所有 BaseURL（支持多端点 failover）
// apiKeys: 渠道的所有 API Key
// 返回所有可能的 metricsKey 列表（无论内存中是否存在，用于后续清理持久化数据）
func (m *MetricsManager) DeleteKeysForChannel(baseURLs, apiKeys []string, serviceType string) []string {
	m.mu.Lock()
	defer m.mu.Unlock()

	seenKeys := make(map[string]struct{})
	allKeys := make([]string, 0, len(baseURLs)*max(1, len(apiKeys)))
	var deletedFromMemory int

	for _, baseURL := range baseURLs {
		for _, apiKey := range apiKeys {
			for _, metricsKey := range m.metricsLookupKeys(baseURL, apiKey, serviceType) {
				if _, exists := seenKeys[metricsKey]; exists {
					continue
				}
				seenKeys[metricsKey] = struct{}{}
				allKeys = append(allKeys, metricsKey)
				if _, exists := m.keyMetrics[metricsKey]; exists {
					delete(m.keyMetrics, metricsKey)
					deletedFromMemory++
				}
			}
		}
	}

	if deletedFromMemory > 0 {
		log.Printf("[Metrics-Delete] 已删除 %d 个内存指标记录", deletedFromMemory)
	}

	return allKeys
}

// DeleteChannelMetrics 删除渠道的所有指标数据（内存 + 持久化）
// baseURLs: 渠道的所有 BaseURL（支持多端点 failover）
// apiKeys: 渠道的所有 API Key
// 返回被删除的持久化记录数
func (m *MetricsManager) DeleteChannelMetrics(baseURLs, apiKeys []string, serviceType string) int64 {
	deletedKeys := m.DeleteKeysForChannel(baseURLs, apiKeys, serviceType)

	if m.store != nil && len(deletedKeys) > 0 {
		deleted, err := m.store.DeleteRecordsByMetricsKeys(deletedKeys, m.apiType)
		if err != nil {
			log.Printf("[Metrics-Delete] 警告: 删除持久化指标记录失败: %v", err)
			return 0
		}
		if _, err := m.store.DeleteCircuitStatesByMetricsKeys(deletedKeys, m.apiType); err != nil {
			log.Printf("[Metrics-Delete] 警告: 删除持久化熔断状态失败: %v", err)
		}
		if deleted > 0 {
			log.Printf("[Metrics-Delete] 已删除 %d 条 %s 持久化指标记录", deleted, m.apiType)
		}
		return deleted
	}

	return 0
}

// DeleteByMetricsKeys 按 metricsKey 列表直接删除指标数据（内存 + 持久化）
// 用于精确删除特定的 (BaseURL, APIKey) 组合，避免笛卡尔积误删
//
// 返回值语义：
//   - 如果配置了持久化存储：返回从持久化存储中删除的记录数
//   - 如果未配置持久化存储或删除失败：返回 0
//   - 注意：内存中的删除数量通过日志输出，不影响返回值
func (m *MetricsManager) DeleteByMetricsKeys(metricsKeys []string) int64 {
	if len(metricsKeys) == 0 {
		return 0
	}

	m.mu.Lock()
	var deletedFromMemory int
	for _, metricsKey := range metricsKeys {
		if _, exists := m.keyMetrics[metricsKey]; exists {
			delete(m.keyMetrics, metricsKey)
			deletedFromMemory++
		}
	}
	m.mu.Unlock()

	if deletedFromMemory > 0 {
		log.Printf("[Metrics-Delete] 已删除 %d 个内存指标记录", deletedFromMemory)
	}

	if m.store != nil {
		deleted, err := m.store.DeleteRecordsByMetricsKeys(metricsKeys, m.apiType)
		if err != nil {
			log.Printf("[Metrics-Delete] 警告: 删除持久化指标记录失败: %v", err)
			return 0
		}
		if _, err := m.store.DeleteCircuitStatesByMetricsKeys(metricsKeys, m.apiType); err != nil {
			log.Printf("[Metrics-Delete] 警告: 删除持久化熔断状态失败: %v", err)
		}
		if deleted > 0 {
			log.Printf("[Metrics-Delete] 已删除 %d 条 %s 持久化指标记录", deleted, m.apiType)
		}
		return deleted
	}

	return 0
}

// cleanupCircuitBreakers 后台任务：推进到期的熔断状态并清理过期指标
func (m *MetricsManager) cleanupCircuitBreakers() {
	ticker := time.NewTicker(30 * time.Second)
	defer ticker.Stop()

	cleanupTicker := time.NewTicker(1 * time.Hour)
	defer cleanupTicker.Stop()

	for {
		select {
		case <-ticker.C:
			m.recoverExpiredCircuitBreakers()
		case <-cleanupTicker.C:
			m.cleanupStaleKeys()
		case <-m.stopCh:
			return
		}
	}
}

// recoverExpiredCircuitBreakers 推进超时的熔断 Key（open -> half_open）。
func (m *MetricsManager) recoverExpiredCircuitBreakers() {
	m.mu.Lock()
	defer m.mu.Unlock()

	now := time.Now()
	for _, metrics := range m.keyMetrics {
		m.advanceCircuitStateIfDueLocked(metrics, now)
	}
}

// cleanupStaleKeys 清理过期的 Key 指标（超过 48 小时无活动）
func (m *MetricsManager) cleanupStaleKeys() {
	m.mu.Lock()
	defer m.mu.Unlock()

	now := time.Now()
	staleThreshold := 48 * time.Hour
	var removedMetricsKeys []string
	var removed []string

	for key, metrics := range m.keyMetrics {
		var lastActivity time.Time
		if metrics.LastSuccessAt != nil {
			lastActivity = *metrics.LastSuccessAt
		}
		if metrics.LastFailureAt != nil && metrics.LastFailureAt.After(lastActivity) {
			lastActivity = *metrics.LastFailureAt
		}

		if lastActivity.IsZero() || now.Sub(lastActivity) > staleThreshold {
			delete(m.keyMetrics, key)
			removedMetricsKeys = append(removedMetricsKeys, key)
			removed = append(removed, metrics.KeyMask)
		}
	}

	if m.store != nil && len(removedMetricsKeys) > 0 {
		if _, err := m.store.DeleteCircuitStatesByMetricsKeys(removedMetricsKeys, m.apiType); err != nil {
			log.Printf("[Metrics-Cleanup] 警告: 删除过期熔断状态失败: %v", err)
		}
	}

	if len(removed) > 0 {
		log.Printf("[Metrics-Cleanup] 清理了 %d 个过期 Key 指标: %v", len(removed), removed)
	}
}

// GetCircuitRecoveryTime 获取基础熔断冷却时间（兼容旧接口）
func (m *MetricsManager) GetCircuitRecoveryTime() time.Duration {
	return m.circuitRecoveryTime
}

// GetCircuitBackoffBase 获取 breaker 基础退避时间。
func (m *MetricsManager) GetCircuitBackoffBase() time.Duration {
	return m.circuitBackoffBase
}

// GetCircuitBackoffMax 获取 breaker 最大退避时间。
func (m *MetricsManager) GetCircuitBackoffMax() time.Duration {
	return m.circuitBackoffMax
}

// GetHalfOpenSuccessTarget 获取 half-open 恢复所需连续成功次数。
func (m *MetricsManager) GetHalfOpenSuccessTarget() int {
	return m.halfOpenSuccessTarget
}

// GetConsecutiveRetryableFailuresThreshold 获取连续可重试失败触发阈值。
func (m *MetricsManager) GetConsecutiveRetryableFailuresThreshold() int64 {
	return consecutiveRetryableFailuresThreshold
}

// GetFailureThreshold 获取失败率阈值
func (m *MetricsManager) GetFailureThreshold() float64 {
	return m.failureThreshold
}

// GetWindowSize 获取滑动窗口大小
func (m *MetricsManager) GetWindowSize() int {
	return m.windowSize
}

// ============ 兼容旧 API 的方法（基于 channelIndex，需要调用方提供 baseURL 和 keys）============

// MetricsResponse API 响应结构
// 使用 omitempty 减少 JSON 体积，0 值字段不输出
// 注意：successRate/errorRate 不使用 omitempty，因为 0% 是有意义的值
type MetricsResponse struct {
	ChannelIndex        int                        `json:"channelIndex"`
	RequestCount        int64                      `json:"requestCount,omitempty"`
	SuccessCount        int64                      `json:"successCount,omitempty"`
	FailureCount        int64                      `json:"failureCount,omitempty"`
	SuccessRate         float64                    `json:"successRate"`
	ErrorRate           float64                    `json:"errorRate"`
	ConsecutiveFailures int64                      `json:"consecutiveFailures,omitempty"`
	ActiveRequests      int64                      `json:"activeRequests,omitempty"` // 进行中请求数
	Latency             int64                      `json:"latency,omitempty"`
	LastSuccessAt       *string                    `json:"lastSuccessAt,omitempty"`
	LastFailureAt       *string                    `json:"lastFailureAt,omitempty"`
	CircuitBrokenAt     *string                    `json:"circuitBrokenAt,omitempty"`
	CircuitState        string                     `json:"circuitState,omitempty"`
	NextRetryAt         *string                    `json:"nextRetryAt,omitempty"`
	HalfOpenSuccesses   int                        `json:"halfOpenSuccesses,omitempty"`
	BreakerFailureRate  float64                    `json:"breakerFailureRate,omitempty"`
	TimeWindows         map[string]TimeWindowStats `json:"timeWindows,omitempty"`
	KeyMetrics          []*KeyMetricsResponse      `json:"keyMetrics,omitempty"` // 各 Key 的详细指标
}

// KeyMetricsResponse 单个 Key 的 API 响应
// 使用 omitempty 减少 JSON 体积，0 值字段不输出
// 注意：successRate 不使用 omitempty，因为 0% 是有意义的值
type KeyMetricsResponse struct {
	KeyMask             string  `json:"keyMask"`
	RequestCount        int64   `json:"requestCount,omitempty"`
	SuccessCount        int64   `json:"successCount,omitempty"`
	FailureCount        int64   `json:"failureCount,omitempty"`
	SuccessRate         float64 `json:"successRate"`
	ConsecutiveFailures int64   `json:"consecutiveFailures,omitempty"`
	CircuitBroken       bool    `json:"circuitBroken,omitempty"`
	CircuitState        string  `json:"circuitState,omitempty"`
	NextRetryAt         *string `json:"nextRetryAt,omitempty"`
	HalfOpenSuccesses   int     `json:"halfOpenSuccesses,omitempty"`
	BreakerFailureRate  float64 `json:"breakerFailureRate,omitempty"`
}

// ToResponseMultiURL 转换为 API 响应格式（支持多 BaseURL 聚合）
// baseURLs: 渠道配置的所有 BaseURL（用于多端点 failover 场景）
// historicalKeys: 历史 API Key（用于统计聚合，只计入总数不显示在 KeyMetrics 中）
func (m *MetricsManager) ToResponseMultiURL(channelIndex int, baseURLs []string, activeKeys []string, serviceType string, latency int64, historicalKeys ...[]string) *MetricsResponse {
	if len(baseURLs) == 0 {
		return &MetricsResponse{
			ChannelIndex: channelIndex,
			Latency:      latency,
			SuccessRate:  100,
			ErrorRate:    0,
		}
	}

	m.mu.Lock()
	defer m.mu.Unlock()

	resp := &MetricsResponse{
		ChannelIndex: channelIndex,
		Latency:      latency,
	}

	if len(activeKeys) == 0 {
		resp.SuccessRate = 100
		resp.ErrorRate = 0
		return resp
	}

	type keyAggregation struct {
		keyMask             string
		requestCount        int64
		successCount        int64
		failureCount        int64
		consecutiveFailures int64
		circuitBroken       bool
		circuitState        CircuitState
		nextRetryAt         *time.Time
		halfOpenSuccesses   int
		breakerFailureRate  float64
	}
	keyAggMap := make(map[string]*keyAggregation)
	seenMetrics := make(map[string]*KeyMetrics)
	metricsToAPIKey := make(map[string]string)

	for _, apiKey := range activeKeys {
		for _, metrics := range m.getIdentityMetricsByMultiURLLocked(baseURLs, apiKey, serviceType) {
			seenMetrics[metrics.MetricsKey] = metrics
			if _, exists := metricsToAPIKey[metrics.MetricsKey]; !exists {
				metricsToAPIKey[metrics.MetricsKey] = apiKey
			}
		}
	}

	now := time.Now()
	var latestSuccess, latestFailure, latestCircuitBroken, latestNextRetry *time.Time
	var totalResults []bool
	var maxConsecutiveFailures int64
	var maxHalfOpenSuccesses int
	channelState := m.channelCircuitStateMultiURLLocked(baseURLs, activeKeys, serviceType, now)

	for _, metrics := range seenMetrics {
		m.advanceCircuitStateIfDueLocked(metrics, now)
		resp.RequestCount += metrics.RequestCount
		resp.SuccessCount += metrics.SuccessCount
		resp.FailureCount += metrics.FailureCount
		resp.ActiveRequests += metrics.ActiveRequests
		if metrics.ConsecutiveFailures > maxConsecutiveFailures {
			maxConsecutiveFailures = metrics.ConsecutiveFailures
		}
		if metrics.HalfOpenSuccesses > maxHalfOpenSuccesses {
			maxHalfOpenSuccesses = metrics.HalfOpenSuccesses
		}
		totalResults = append(totalResults, metrics.breakerResults...)

		if metrics.LastSuccessAt != nil && (latestSuccess == nil || metrics.LastSuccessAt.After(*latestSuccess)) {
			latestSuccess = metrics.LastSuccessAt
		}
		if metrics.LastFailureAt != nil && (latestFailure == nil || metrics.LastFailureAt.After(*latestFailure)) {
			latestFailure = metrics.LastFailureAt
		}
		if metrics.CircuitBrokenAt != nil && (latestCircuitBroken == nil || metrics.CircuitBrokenAt.After(*latestCircuitBroken)) {
			latestCircuitBroken = metrics.CircuitBrokenAt
		}
		if metrics.NextRetryAt != nil && (latestNextRetry == nil || metrics.NextRetryAt.After(*latestNextRetry)) {
			latestNextRetry = metrics.NextRetryAt
		}

		breakerFailureRate := m.calculateKeyBreakerFailureRateInternal(metrics) * 100
		apiKey := metricsToAPIKey[metrics.MetricsKey]
		if agg, ok := keyAggMap[apiKey]; ok {
			agg.requestCount += metrics.RequestCount
			agg.successCount += metrics.SuccessCount
			agg.failureCount += metrics.FailureCount
			if metrics.ConsecutiveFailures > agg.consecutiveFailures {
				agg.consecutiveFailures = metrics.ConsecutiveFailures
			}
			if metrics.CircuitBrokenAt != nil {
				agg.circuitBroken = true
			}
			if metrics.CircuitState > agg.circuitState {
				agg.circuitState = metrics.CircuitState
			}
			if metrics.NextRetryAt != nil && (agg.nextRetryAt == nil || metrics.NextRetryAt.After(*agg.nextRetryAt)) {
				t := *metrics.NextRetryAt
				agg.nextRetryAt = &t
			}
			if metrics.HalfOpenSuccesses > agg.halfOpenSuccesses {
				agg.halfOpenSuccesses = metrics.HalfOpenSuccesses
			}
			if breakerFailureRate > agg.breakerFailureRate {
				agg.breakerFailureRate = breakerFailureRate
			}
		} else {
			var nextRetryCopy *time.Time
			if metrics.NextRetryAt != nil {
				t := *metrics.NextRetryAt
				nextRetryCopy = &t
			}
			keyAggMap[apiKey] = &keyAggregation{
				keyMask:             metrics.KeyMask,
				requestCount:        metrics.RequestCount,
				successCount:        metrics.SuccessCount,
				failureCount:        metrics.FailureCount,
				consecutiveFailures: metrics.ConsecutiveFailures,
				circuitBroken:       metrics.CircuitBrokenAt != nil,
				circuitState:        metrics.CircuitState,
				nextRetryAt:         nextRetryCopy,
				halfOpenSuccesses:   metrics.HalfOpenSuccesses,
				breakerFailureRate:  breakerFailureRate,
			}
		}
	}

	if len(historicalKeys) > 0 && len(historicalKeys[0]) > 0 {
		seenHistorical := make(map[string]struct{})
		for _, apiKey := range historicalKeys[0] {
			for _, metrics := range m.getIdentityMetricsByMultiURLLocked(baseURLs, apiKey, serviceType) {
				if _, ok := seenHistorical[metrics.MetricsKey]; ok {
					continue
				}
				seenHistorical[metrics.MetricsKey] = struct{}{}
				resp.RequestCount += metrics.RequestCount
				resp.SuccessCount += metrics.SuccessCount
				resp.FailureCount += metrics.FailureCount
			}
		}
	}

	var keyResponses []*KeyMetricsResponse
	for _, apiKey := range activeKeys {
		if agg, ok := keyAggMap[apiKey]; ok {
			keySuccessRate := float64(100)
			if agg.requestCount > 0 {
				keySuccessRate = float64(agg.successCount) / float64(agg.requestCount) * 100
			}
			var nextRetryText *string
			if agg.nextRetryAt != nil {
				t := agg.nextRetryAt.Format(time.RFC3339)
				nextRetryText = &t
			}
			keyResponses = append(keyResponses, &KeyMetricsResponse{
				KeyMask:             agg.keyMask,
				RequestCount:        agg.requestCount,
				SuccessCount:        agg.successCount,
				FailureCount:        agg.failureCount,
				SuccessRate:         keySuccessRate,
				ConsecutiveFailures: agg.consecutiveFailures,
				CircuitBroken:       agg.circuitBroken,
				CircuitState:        agg.circuitState.String(),
				NextRetryAt:         nextRetryText,
				HalfOpenSuccesses:   agg.halfOpenSuccesses,
				BreakerFailureRate:  agg.breakerFailureRate,
			})
		}
	}

	resp.ConsecutiveFailures = maxConsecutiveFailures
	resp.HalfOpenSuccesses = maxHalfOpenSuccesses
	resp.CircuitState = channelState.String()

	if resp.RequestCount > 0 {
		resp.SuccessRate = float64(resp.SuccessCount) / float64(resp.RequestCount) * 100
		resp.ErrorRate = float64(resp.FailureCount) / float64(resp.RequestCount) * 100
	} else {
		resp.SuccessRate = 100
		resp.ErrorRate = 0
	}

	if len(totalResults) > 0 {
		failures := 0
		for _, success := range totalResults {
			if !success {
				failures++
			}
		}
		failureRate := float64(failures) / float64(len(totalResults))
		resp.BreakerFailureRate = failureRate * 100
	} else {
		resp.BreakerFailureRate = 0
	}

	if latestSuccess != nil {
		t := latestSuccess.Format(time.RFC3339)
		resp.LastSuccessAt = &t
	}
	if latestFailure != nil {
		t := latestFailure.Format(time.RFC3339)
		resp.LastFailureAt = &t
	}
	if latestCircuitBroken != nil {
		t := latestCircuitBroken.Format(time.RFC3339)
		resp.CircuitBrokenAt = &t
	}
	if latestNextRetry != nil {
		t := latestNextRetry.Format(time.RFC3339)
		resp.NextRetryAt = &t
	}

	resp.KeyMetrics = keyResponses
	resp.TimeWindows = m.calculateAggregatedTimeWindowsMultiURL(baseURLs, activeKeys, serviceType)

	return resp
}

// ToResponse 转换为 API 响应格式（需要提供 baseURL 和 activeKeys）
func (m *MetricsManager) ToResponse(channelIndex int, baseURL string, activeKeys []string, serviceType string, latency int64) *MetricsResponse {
	return m.ToResponseMultiURL(channelIndex, []string{baseURL}, activeKeys, serviceType, latency)
}

// calculateAggregatedTimeWindowsInternal 计算聚合的时间窗口统计（内部方法，调用前需持有锁）
func (m *MetricsManager) calculateAggregatedTimeWindowsInternal(baseURL string, activeKeys []string, serviceType string) map[string]TimeWindowStats {
	windows := map[string]time.Duration{
		"15m": 15 * time.Minute,
		"1h":  1 * time.Hour,
		"6h":  6 * time.Hour,
		"24h": 24 * time.Hour,
	}

	result := make(map[string]TimeWindowStats)
	now := time.Now()

	for label, duration := range windows {
		cutoff := now.Add(-duration)
		var requestCount, successCount, failureCount int64
		var inputTokens, outputTokens, cacheCreationTokens, cacheReadTokens int64

		for _, apiKey := range activeKeys {
			for _, metrics := range m.getMetricsVariantsLocked(baseURL, apiKey, serviceType) {
				for _, record := range metrics.requestHistory {
					if record.Timestamp.After(cutoff) {
						requestCount++
						if record.Success {
							successCount++
						} else {
							failureCount++
						}
						inputTokens += record.InputTokens
						outputTokens += record.OutputTokens
						cacheCreationTokens += record.CacheCreationInputTokens
						cacheReadTokens += record.CacheReadInputTokens
					}
				}
			}
		}

		successRate := float64(100)
		if requestCount > 0 {
			successRate = float64(successCount) / float64(requestCount) * 100
		}

		cacheHitRate := float64(0)
		denom := cacheReadTokens + inputTokens
		if denom > 0 {
			cacheHitRate = float64(cacheReadTokens) / float64(denom) * 100
		}

		result[label] = TimeWindowStats{
			RequestCount:        requestCount,
			SuccessCount:        successCount,
			FailureCount:        failureCount,
			SuccessRate:         successRate,
			InputTokens:         inputTokens,
			OutputTokens:        outputTokens,
			CacheCreationTokens: cacheCreationTokens,
			CacheReadTokens:     cacheReadTokens,
			CacheHitRate:        cacheHitRate,
		}
	}

	return result
}

// calculateAggregatedTimeWindowsMultiURL 计算聚合的时间窗口统计（多 URL 版本，内部方法，调用前需持有锁）
func (m *MetricsManager) calculateAggregatedTimeWindowsMultiURL(baseURLs []string, activeKeys []string, serviceType string) map[string]TimeWindowStats {
	windows := map[string]time.Duration{
		"15m": 15 * time.Minute,
		"1h":  1 * time.Hour,
		"6h":  6 * time.Hour,
		"24h": 24 * time.Hour,
	}

	result := make(map[string]TimeWindowStats)
	now := time.Now()

	for label, duration := range windows {
		cutoff := now.Add(-duration)
		var requestCount, successCount, failureCount int64
		var inputTokens, outputTokens, cacheCreationTokens, cacheReadTokens int64

		// 遍历所有 BaseURL 和 Key 的组合
		for _, metrics := range m.getIdentityMetricsByMultiURLAndKeysLocked(baseURLs, activeKeys, serviceType) {
			for _, record := range metrics.requestHistory {
				if record.Timestamp.After(cutoff) {
					requestCount++
					if record.Success {
						successCount++
					} else {
						failureCount++
					}
					inputTokens += record.InputTokens
					outputTokens += record.OutputTokens
					cacheCreationTokens += record.CacheCreationInputTokens
					cacheReadTokens += record.CacheReadInputTokens
				}
			}
		}

		successRate := float64(100)
		if requestCount > 0 {
			successRate = float64(successCount) / float64(requestCount) * 100
		}

		cacheHitRate := float64(0)
		denom := cacheReadTokens + inputTokens
		if denom > 0 {
			cacheHitRate = float64(cacheReadTokens) / float64(denom) * 100
		}

		result[label] = TimeWindowStats{
			RequestCount:        requestCount,
			SuccessCount:        successCount,
			FailureCount:        failureCount,
			SuccessRate:         successRate,
			InputTokens:         inputTokens,
			OutputTokens:        outputTokens,
			CacheCreationTokens: cacheCreationTokens,
			CacheReadTokens:     cacheReadTokens,
			CacheHitRate:        cacheHitRate,
		}
	}

	return result
}

// ============ 废弃的旧方法（保留签名以便编译，但标记为废弃）============

// Deprecated: 使用 IsChannelHealthyWithKeys 代替
// IsChannelHealthy 判断渠道是否健康（旧方法，不再使用 channelIndex）
// 此方法保留是为了兼容，但始终返回 true，调用方应迁移到新方法
func (m *MetricsManager) IsChannelHealthy(channelIndex int) bool {
	log.Printf("[Metrics-Deprecated] 警告: 调用了废弃的 IsChannelHealthy(channelIndex=%d)，请迁移到 IsChannelHealthyWithKeys", channelIndex)
	return true // 默认健康，避免影响现有逻辑
}

// Deprecated: 使用 CalculateChannelFailureRate 代替
func (m *MetricsManager) CalculateFailureRate(channelIndex int) float64 {
	return 0
}

// Deprecated: 使用 CalculateChannelFailureRate 代替
func (m *MetricsManager) CalculateSuccessRate(channelIndex int) float64 {
	return 1
}

// Deprecated: 使用 ResetKey 代替
func (m *MetricsManager) Reset(channelIndex int) {
	log.Printf("[Metrics-Deprecated] 警告: 调用了废弃的 Reset(channelIndex=%d)，请迁移到 ResetKey", channelIndex)
}

// Deprecated: 使用 GetChannelAggregatedMetrics 代替
func (m *MetricsManager) GetMetrics(channelIndex int) *ChannelMetrics {
	return nil
}

// Deprecated: 使用 GetAllKeyMetrics 代替
func (m *MetricsManager) GetAllMetrics() []*ChannelMetrics {
	return nil
}

// Deprecated: 使用 GetTimeWindowStatsForKey 代替
func (m *MetricsManager) GetTimeWindowStats(channelIndex int, duration time.Duration) TimeWindowStats {
	return TimeWindowStats{SuccessRate: 100}
}

// Deprecated: 使用 GetAllTimeWindowStatsForKey 代替
func (m *MetricsManager) GetAllTimeWindowStats(channelIndex int) map[string]TimeWindowStats {
	return map[string]TimeWindowStats{
		"15m": {SuccessRate: 100},
		"1h":  {SuccessRate: 100},
		"6h":  {SuccessRate: 100},
		"24h": {SuccessRate: 100},
	}
}

// Deprecated: 使用新的 ShouldSuspendKey 代替
func (m *MetricsManager) ShouldSuspend(channelIndex int) bool {
	return false
}

// GetKeyCircuitState 获取单个 Key 当前的 breaker 状态。
func (m *MetricsManager) GetKeyCircuitState(baseURL, apiKey, serviceType string) CircuitState {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics == nil {
		return CircuitStateClosed
	}
	m.advanceCircuitStateIfDueLocked(metrics, time.Now())
	return metrics.CircuitState
}

// TryAcquireProbe 尝试占用 half-open 探针资格。
func (m *MetricsManager) TryAcquireProbe(baseURL, apiKey, serviceType string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics == nil {
		return false
	}
	m.advanceCircuitStateIfDueLocked(metrics, time.Now())
	if metrics.CircuitState != CircuitStateHalfOpen || metrics.ProbeInFlight {
		return false
	}
	metrics.ProbeInFlight = true
	return true
}

// ReleaseProbe 释放 half-open 探针占用。
func (m *MetricsManager) ReleaseProbe(baseURL, apiKey, serviceType string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics != nil {
		metrics.ProbeInFlight = false
	}
}

// GetChannelCircuitStateMultiURL 获取多 BaseURL 聚合后的 channel breaker 状态。
func (m *MetricsManager) GetChannelCircuitStateMultiURL(baseURLs []string, activeKeys []string, serviceType string) CircuitState {
	m.mu.Lock()
	defer m.mu.Unlock()

	return m.channelCircuitStateMultiURLLocked(baseURLs, activeKeys, serviceType, time.Now())
}

func (m *MetricsManager) channelCircuitStateMultiURLLocked(baseURLs []string, activeKeys []string, serviceType string, now time.Time) CircuitState {
	hasHalfOpen := false
	for _, baseURL := range baseURLs {
		for _, apiKey := range activeKeys {
			metrics := m.getIdentityMetricsLocked(baseURL, apiKey, serviceType)
			if metrics == nil {
				return CircuitStateClosed
			}
			m.advanceCircuitStateIfDueLocked(metrics, now)
			if metrics.CircuitState == CircuitStateClosed {
				return CircuitStateClosed
			}
			if metrics.CircuitState == CircuitStateHalfOpen {
				hasHalfOpen = true
			}
		}
	}
	if hasHalfOpen {
		return CircuitStateHalfOpen
	}
	return CircuitStateOpen
}

// HasProbeCandidateMultiURL 判断渠道是否存在可用的 half-open 探针候选。
func (m *MetricsManager) HasProbeCandidateMultiURL(baseURLs []string, activeKeys []string, serviceType string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()

	now := time.Now()
	for _, apiKey := range activeKeys {
		for _, metrics := range m.getIdentityMetricsByMultiURLLocked(baseURLs, apiKey, serviceType) {
			m.advanceCircuitStateIfDueLocked(metrics, now)
			if metrics.CircuitState == CircuitStateHalfOpen && !metrics.ProbeInFlight {
				return true
			}
		}
	}
	return false
}

// ShouldSuspendKey 判断单个 Key 是否应该熔断
func (m *MetricsManager) ShouldSuspendKey(baseURL, apiKey, serviceType string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()

	metrics := m.getFirstMatchingMetricsLocked(baseURL, apiKey, serviceType)
	if metrics == nil {
		return false
	}
	m.advanceCircuitStateIfDueLocked(metrics, time.Now())
	return metrics.CircuitState == CircuitStateOpen
}

// ============ 历史数据查询方法（用于图表可视化）============

// HistoryDataPoint 历史数据点（用于时间序列图表）
type HistoryDataPoint struct {
	Timestamp    time.Time `json:"timestamp"`
	RequestCount int64     `json:"requestCount"`
	SuccessCount int64     `json:"successCount"`
	FailureCount int64     `json:"failureCount"`
	SuccessRate  float64   `json:"successRate"`
}

// KeyHistoryDataPoint Key 级别历史数据点（包含 Token 和 Cache 数据）
type KeyHistoryDataPoint struct {
	Timestamp                time.Time `json:"timestamp"`
	RequestCount             int64     `json:"requestCount"`
	SuccessCount             int64     `json:"successCount"`
	FailureCount             int64     `json:"failureCount"`
	SuccessRate              float64   `json:"successRate"`
	InputTokens              int64     `json:"inputTokens"`
	OutputTokens             int64     `json:"outputTokens"`
	CacheCreationInputTokens int64     `json:"cacheCreationTokens"`
	CacheReadInputTokens     int64     `json:"cacheReadTokens"`
}

// GetHistoricalStats 获取历史统计数据（按时间间隔聚合）
// duration: 查询时间范围 (如 1h, 6h, 24h)
// interval: 聚合间隔 (如 5m, 15m, 1h)
func (m *MetricsManager) GetHistoricalStats(baseURL string, activeKeys []string, serviceType string, duration, interval time.Duration) []HistoryDataPoint {
	// 参数验证
	if interval <= 0 || duration <= 0 {
		return []HistoryDataPoint{}
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	now := time.Now()
	// 时间对齐到 interval 边界
	startTime := now.Add(-duration).Truncate(interval)
	// endTime 延伸一个 interval，确保当前时间段的请求也被包含
	endTime := now.Truncate(interval).Add(interval)

	// 计算需要多少个数据点（+1 用于包含延伸的当前时间段）
	numPoints := int(duration / interval)
	if numPoints <= 0 {
		numPoints = 1
	}
	numPoints++ // 额外的一个桶用于当前时间段

	// 使用 map 按时间分桶，优化性能：O(records) 而不是 O(records * numPoints)
	buckets := make(map[int64]*bucketData)
	for i := 0; i < numPoints; i++ {
		buckets[int64(i)] = &bucketData{}
	}

	// 收集所有相关 Key 的请求历史并放入对应桶
	for _, apiKey := range activeKeys {
		metrics := m.getIdentityMetricsLocked(baseURL, apiKey, serviceType)
		if metrics == nil {
			continue
		}
		for _, record := range metrics.requestHistory {
			if !record.Timestamp.Before(startTime) && record.Timestamp.Before(endTime) {
				offset := int64(record.Timestamp.Sub(startTime) / interval)
				if offset >= 0 && offset < int64(numPoints) {
					b := buckets[offset]
					b.requestCount++
					if record.Success {
						b.successCount++
					} else {
						b.failureCount++
					}
				}
			}
		}
	}

	// 构建结果
	result := make([]HistoryDataPoint, numPoints)
	for i := 0; i < numPoints; i++ {
		b := buckets[int64(i)]
		// 空桶成功率默认为 0，避免误导（100% 暗示完美成功）
		successRate := float64(0)
		if b.requestCount > 0 {
			successRate = float64(b.successCount) / float64(b.requestCount) * 100
		}
		result[i] = HistoryDataPoint{
			Timestamp:    startTime.Add(time.Duration(i) * interval),
			RequestCount: b.requestCount,
			SuccessCount: b.successCount,
			FailureCount: b.failureCount,
			SuccessRate:  successRate,
		}
	}

	return result
}

// GetHistoricalStatsMultiURL 获取多 URL 聚合的历史统计数据
func (m *MetricsManager) GetHistoricalStatsMultiURL(baseURLs []string, activeKeys []string, serviceType string, duration, interval time.Duration) []HistoryDataPoint {
	// 参数验证
	if interval <= 0 || duration <= 0 || len(baseURLs) == 0 {
		return []HistoryDataPoint{}
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	now := time.Now()
	// 时间对齐到 interval 边界
	startTime := now.Add(-duration).Truncate(interval)
	// endTime 延伸一个 interval，确保当前时间段的请求也被包含
	endTime := now.Truncate(interval).Add(interval)

	// 计算需要多少个数据点（+1 用于包含延伸的当前时间段）
	numPoints := int(duration / interval)
	if numPoints <= 0 {
		numPoints = 1
	}
	numPoints++ // 额外的一个桶用于当前时间段

	// 使用 map 按时间分桶，优化性能：O(records) 而不是 O(records * numPoints)
	buckets := make(map[int64]*bucketData)
	for i := 0; i < numPoints; i++ {
		buckets[int64(i)] = &bucketData{}
	}

	// 收集所有 BaseURL 和 Key 组合的请求历史并放入对应桶
	for _, metrics := range m.getIdentityMetricsByMultiURLAndKeysLocked(baseURLs, activeKeys, serviceType) {
		for _, record := range metrics.requestHistory {
			if !record.Timestamp.Before(startTime) && record.Timestamp.Before(endTime) {
				offset := int64(record.Timestamp.Sub(startTime) / interval)
				if offset >= 0 && offset < int64(numPoints) {
					b := buckets[offset]
					b.requestCount++
					if record.Success {
						b.successCount++
					} else {
						b.failureCount++
					}
				}
			}
		}
	}

	// 构建结果
	result := make([]HistoryDataPoint, numPoints)
	for i := 0; i < numPoints; i++ {
		b := buckets[int64(i)]
		// 空桶成功率默认为 0，避免误导（100% 暗示完美成功）
		successRate := float64(0)
		if b.requestCount > 0 {
			successRate = float64(b.successCount) / float64(b.requestCount) * 100
		}
		result[i] = HistoryDataPoint{
			Timestamp:    startTime.Add(time.Duration(i) * interval),
			RequestCount: b.requestCount,
			SuccessCount: b.successCount,
			FailureCount: b.failureCount,
			SuccessRate:  successRate,
		}
	}

	return result
}

// bucketData 用于时间分桶的辅助结构
type bucketData struct {
	requestCount int64
	successCount int64
	failureCount int64
}

func (m *MetricsManager) GetAllKeysHistoricalStats(duration, interval time.Duration) []HistoryDataPoint {
	// 参数验证
	if interval <= 0 || duration <= 0 {
		return []HistoryDataPoint{}
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	now := time.Now()
	// 时间对齐到 interval 边界
	startTime := now.Add(-duration).Truncate(interval)
	// endTime 延伸一个 interval，确保当前时间段的请求也被包含
	endTime := now.Truncate(interval).Add(interval)

	numPoints := int(duration / interval)
	if numPoints <= 0 {
		numPoints = 1
	}
	numPoints++ // 额外的一个桶用于当前时间段

	// 使用 map 按时间分桶，优化性能
	buckets := make(map[int64]*bucketData)
	for i := 0; i < numPoints; i++ {
		buckets[int64(i)] = &bucketData{}
	}

	// 收集所有 Key 的请求历史并放入对应桶
	for _, metrics := range m.keyMetrics {
		for _, record := range metrics.requestHistory {
			// 使用 [startTime, endTime) 的区间，避免 endTime 处 offset 越界
			if !record.Timestamp.Before(startTime) && record.Timestamp.Before(endTime) {
				offset := int64(record.Timestamp.Sub(startTime) / interval)
				if offset >= 0 && offset < int64(numPoints) {
					b := buckets[offset]
					b.requestCount++
					if record.Success {
						b.successCount++
					} else {
						b.failureCount++
					}
				}
			}
		}
	}

	// 构建结果
	result := make([]HistoryDataPoint, numPoints)
	for i := 0; i < numPoints; i++ {
		b := buckets[int64(i)]
		// 空桶成功率默认为 0，避免误导（100% 暗示完美成功）
		successRate := float64(0)
		if b.requestCount > 0 {
			successRate = float64(b.successCount) / float64(b.requestCount) * 100
		}
		result[i] = HistoryDataPoint{
			Timestamp:    startTime.Add(time.Duration(i) * interval),
			RequestCount: b.requestCount,
			SuccessCount: b.successCount,
			FailureCount: b.failureCount,
			SuccessRate:  successRate,
		}
	}

	return result
}

// GetKeyHistoricalStats 获取单个 Key 的历史统计数据（包含 Token 和 Cache 数据）
func (m *MetricsManager) GetKeyHistoricalStats(baseURL, apiKey, serviceType string, duration, interval time.Duration) []KeyHistoryDataPoint {
	// 参数验证
	if interval <= 0 || duration <= 0 {
		return []KeyHistoryDataPoint{}
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	now := time.Now()
	// 时间对齐到 interval 边界
	startTime := now.Add(-duration).Truncate(interval)
	// endTime 延伸一个 interval，确保当前时间段的请求也被包含
	endTime := now.Truncate(interval).Add(interval)

	numPoints := int(duration / interval)
	if numPoints <= 0 {
		numPoints = 1
	}
	numPoints++ // 额外的一个桶用于当前时间段

	// 使用 map 按时间分桶
	buckets := make(map[int64]*keyBucketData)
	for i := 0; i < numPoints; i++ {
		buckets[int64(i)] = &keyBucketData{}
	}

	metrics := m.getIdentityMetricsLocked(baseURL, apiKey, serviceType)
	if metrics == nil {
		// Key 不存在，返回空数据点
		result := make([]KeyHistoryDataPoint, numPoints)
		for i := 0; i < numPoints; i++ {
			result[i] = KeyHistoryDataPoint{
				Timestamp: startTime.Add(time.Duration(i+1) * interval),
			}
		}
		return result
	}

	// 收集该 Key 的请求历史并放入对应桶
	for _, record := range metrics.requestHistory {
		if record.Timestamp.After(startTime) && record.Timestamp.Before(endTime) {
			offset := int64(record.Timestamp.Sub(startTime) / interval)
			if offset >= 0 && offset < int64(numPoints) {
				b := buckets[offset]
				b.requestCount++
				if record.Success {
					b.successCount++
				} else {
					b.failureCount++
				}
				b.inputTokens += record.InputTokens
				b.outputTokens += record.OutputTokens
				b.cacheCreationTokens += record.CacheCreationInputTokens
				b.cacheReadTokens += record.CacheReadInputTokens
			}
		}
	}

	// 构建结果
	result := make([]KeyHistoryDataPoint, numPoints)
	for i := 0; i < numPoints; i++ {
		b := buckets[int64(i)]
		// 空桶成功率默认为 0，避免误导（100% 暗示完美成功）
		successRate := float64(0)
		if b.requestCount > 0 {
			successRate = float64(b.successCount) / float64(b.requestCount) * 100
		}
		result[i] = KeyHistoryDataPoint{
			Timestamp:                startTime.Add(time.Duration(i+1) * interval),
			RequestCount:             b.requestCount,
			SuccessCount:             b.successCount,
			FailureCount:             b.failureCount,
			SuccessRate:              successRate,
			InputTokens:              b.inputTokens,
			OutputTokens:             b.outputTokens,
			CacheCreationInputTokens: b.cacheCreationTokens,
			CacheReadInputTokens:     b.cacheReadTokens,
		}
	}

	return result
}

// GetKeyHistoricalStatsMultiURL 获取单个 Key 的多 URL 聚合历史统计
func (m *MetricsManager) GetKeyHistoricalStatsMultiURL(baseURLs []string, apiKey, serviceType string, duration, interval time.Duration) []KeyHistoryDataPoint {
	// 参数验证
	if interval <= 0 || duration <= 0 || len(baseURLs) == 0 {
		return []KeyHistoryDataPoint{}
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	now := time.Now()
	// 时间对齐到 interval 边界
	startTime := now.Add(-duration).Truncate(interval)
	// endTime 延伸一个 interval，确保当前时间段的请求也被包含
	endTime := now.Truncate(interval).Add(interval)

	numPoints := int(duration / interval)
	if numPoints <= 0 {
		numPoints = 1
	}
	numPoints++ // 额外的一个桶用于当前时间段

	// 使用 map 按时间分桶
	buckets := make(map[int64]*keyBucketData)
	for i := 0; i < numPoints; i++ {
		buckets[int64(i)] = &keyBucketData{}
	}

	// 遍历所有 BaseURL 聚合同一 Key 的历史数据
	hasData := false
	for _, metrics := range m.getIdentityMetricsByMultiURLLocked(baseURLs, apiKey, serviceType) {
		hasData = true

		for _, record := range metrics.requestHistory {
			if record.Timestamp.After(startTime) && record.Timestamp.Before(endTime) {
				offset := int64(record.Timestamp.Sub(startTime) / interval)
				if offset >= 0 && offset < int64(numPoints) {
					b := buckets[offset]
					b.requestCount++
					if record.Success {
						b.successCount++
					} else {
						b.failureCount++
					}
					b.inputTokens += record.InputTokens
					b.outputTokens += record.OutputTokens
					b.cacheCreationTokens += record.CacheCreationInputTokens
					b.cacheReadTokens += record.CacheReadInputTokens
				}
			}
		}
	}

	// 如果没有任何数据，返回空数据点
	if !hasData {
		result := make([]KeyHistoryDataPoint, numPoints)
		for i := 0; i < numPoints; i++ {
			result[i] = KeyHistoryDataPoint{
				Timestamp: startTime.Add(time.Duration(i+1) * interval),
			}
		}
		return result
	}

	// 构建结果
	result := make([]KeyHistoryDataPoint, numPoints)
	for i := 0; i < numPoints; i++ {
		b := buckets[int64(i)]
		// 空桶成功率默认为 0，避免误导（100% 暗示完美成功）
		successRate := float64(0)
		if b.requestCount > 0 {
			successRate = float64(b.successCount) / float64(b.requestCount) * 100
		}
		result[i] = KeyHistoryDataPoint{
			Timestamp:                startTime.Add(time.Duration(i+1) * interval),
			RequestCount:             b.requestCount,
			SuccessCount:             b.successCount,
			FailureCount:             b.failureCount,
			SuccessRate:              successRate,
			InputTokens:              b.inputTokens,
			OutputTokens:             b.outputTokens,
			CacheCreationInputTokens: b.cacheCreationTokens,
			CacheReadInputTokens:     b.cacheReadTokens,
		}
	}

	return result
}

// KeyModelHistoryDataPoint Key+Model 组合的历史数据点
type KeyModelHistoryDataPoint struct {
	Timestamp                time.Time `json:"timestamp"`
	RequestCount             int64     `json:"requestCount"`
	SuccessCount             int64     `json:"successCount"`
	FailureCount             int64     `json:"failureCount"`
	InputTokens              int64     `json:"inputTokens"`
	OutputTokens             int64     `json:"outputTokens"`
	CacheCreationInputTokens int64     `json:"cacheCreationTokens"`
	CacheReadInputTokens     int64     `json:"cacheReadTokens"`
}

// GetKeyModelHistoricalStatsMultiURL 获取单个 Key 按模型分组的历史数据
func (m *MetricsManager) GetKeyModelHistoricalStatsMultiURL(baseURLs []string, apiKey, serviceType string, duration, interval time.Duration) map[string][]KeyModelHistoryDataPoint {
	if interval <= 0 || duration <= 0 || len(baseURLs) == 0 {
		return nil
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	now := time.Now()
	startTime := now.Add(-duration).Truncate(interval)
	endTime := now.Truncate(interval).Add(interval)
	numPoints := int(duration/interval) + 1

	// 按模型分组的桶: model -> bucketIndex -> data
	modelBuckets := make(map[string]map[int64]*keyBucketData)

	for _, metrics := range m.getIdentityMetricsByMultiURLLocked(baseURLs, apiKey, serviceType) {
		for _, record := range metrics.requestHistory {
			if record.Timestamp.After(startTime) && record.Timestamp.Before(endTime) {
				offset := int64(record.Timestamp.Sub(startTime) / interval)
				if offset >= 0 && offset < int64(numPoints) {
					model := record.Model
					if model == "" {
						model = "unknown"
					}
					if modelBuckets[model] == nil {
						modelBuckets[model] = make(map[int64]*keyBucketData)
						for i := 0; i < numPoints; i++ {
							modelBuckets[model][int64(i)] = &keyBucketData{}
						}
					}
					b := modelBuckets[model][offset]
					b.requestCount++
					if record.Success {
						b.successCount++
					} else {
						b.failureCount++
					}
					b.inputTokens += record.InputTokens
					b.outputTokens += record.OutputTokens
					b.cacheCreationTokens += record.CacheCreationInputTokens
					b.cacheReadTokens += record.CacheReadInputTokens
				}
			}
		}
	}

	// 构建结果
	result := make(map[string][]KeyModelHistoryDataPoint)
	for model, buckets := range modelBuckets {
		points := make([]KeyModelHistoryDataPoint, numPoints)
		for i := 0; i < numPoints; i++ {
			b := buckets[int64(i)]
			points[i] = KeyModelHistoryDataPoint{
				Timestamp:                startTime.Add(time.Duration(i+1) * interval),
				RequestCount:             b.requestCount,
				SuccessCount:             b.successCount,
				FailureCount:             b.failureCount,
				InputTokens:              b.inputTokens,
				OutputTokens:             b.outputTokens,
				CacheCreationInputTokens: b.cacheCreationTokens,
				CacheReadInputTokens:     b.cacheReadTokens,
			}
		}
		result[model] = points
	}

	return result
}

// keyBucketData Key 级别时间分桶的辅助结构（包含 Token 数据）
type keyBucketData struct {
	requestCount        int64
	successCount        int64
	failureCount        int64
	inputTokens         int64
	outputTokens        int64
	cacheCreationTokens int64
	cacheReadTokens     int64
}

// ============ 全局统计数据结构和方法（用于全局流量统计图表）============

// GlobalHistoryDataPoint 全局历史数据点（含 Token 数据）
type GlobalHistoryDataPoint struct {
	Timestamp           time.Time `json:"timestamp"`
	RequestCount        int64     `json:"requestCount"`
	SuccessCount        int64     `json:"successCount"`
	FailureCount        int64     `json:"failureCount"`
	SuccessRate         float64   `json:"successRate"`
	InputTokens         int64     `json:"inputTokens"`
	OutputTokens        int64     `json:"outputTokens"`
	CacheCreationTokens int64     `json:"cacheCreationTokens"`
	CacheReadTokens     int64     `json:"cacheReadTokens"`
}

// GlobalStatsSummary 全局统计汇总
type GlobalStatsSummary struct {
	TotalRequests            int64   `json:"totalRequests"`
	TotalSuccess             int64   `json:"totalSuccess"`
	TotalFailure             int64   `json:"totalFailure"`
	TotalInputTokens         int64   `json:"totalInputTokens"`
	TotalOutputTokens        int64   `json:"totalOutputTokens"`
	TotalCacheCreationTokens int64   `json:"totalCacheCreationTokens"`
	TotalCacheReadTokens     int64   `json:"totalCacheReadTokens"`
	AvgSuccessRate           float64 `json:"avgSuccessRate"`
	Duration                 string  `json:"duration"`
}

// GlobalStatsHistoryResponse 全局统计响应
type GlobalStatsHistoryResponse struct {
	DataPoints      []GlobalHistoryDataPoint           `json:"dataPoints"`
	Summary         GlobalStatsSummary                 `json:"summary"`
	ModelDataPoints map[string][]ModelHistoryDataPoint `json:"modelDataPoints,omitempty"`
}

// GetGlobalHistoricalStatsWithTokens 获取全局历史统计（包含 Token 数据）
// 聚合所有 Key 的数据，按时间间隔分桶
func (m *MetricsManager) GetGlobalHistoricalStatsWithTokens(duration, interval time.Duration) GlobalStatsHistoryResponse {
	// 参数验证
	if interval <= 0 || duration <= 0 {
		return GlobalStatsHistoryResponse{
			DataPoints: []GlobalHistoryDataPoint{},
			Summary:    GlobalStatsSummary{Duration: duration.String()},
		}
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	now := time.Now()
	// 时间对齐到 interval 边界
	startTime := now.Add(-duration).Truncate(interval)
	// endTime 延伸一个 interval，确保当前时间段的请求也被包含
	endTime := now.Truncate(interval).Add(interval)

	numPoints := int(duration / interval)
	if numPoints <= 0 {
		numPoints = 1
	}
	numPoints++ // 额外的一个桶用于当前时间段

	// 使用 map 按时间分桶
	buckets := make(map[int64]*globalBucketData)
	for i := 0; i < numPoints; i++ {
		buckets[int64(i)] = &globalBucketData{}
	}

	// 汇总统计
	var totalRequests, totalSuccess, totalFailure int64
	var totalInputTokens, totalOutputTokens, totalCacheCreation, totalCacheRead int64

	// 按模型分桶（复用 modelBucket 结构）
	type modelBucket struct {
		requestCount int64
		successCount int64
		failureCount int64
		inputTokens  int64
		outputTokens int64
	}
	modelBuckets := make(map[string][]modelBucket)

	// 遍历所有 Key 的请求历史
	for _, metrics := range m.keyMetrics {
		for _, record := range metrics.requestHistory {
			// 使用 Before(endTime) 排除恰好落在 endTime 的记录，避免 offset 越界
			if record.Timestamp.After(startTime) && record.Timestamp.Before(endTime) {
				offset := int64(record.Timestamp.Sub(startTime) / interval)
				if offset >= 0 && offset < int64(numPoints) {
					b := buckets[offset]
					b.requestCount++
					if record.Success {
						b.successCount++
					} else {
						b.failureCount++
					}
					b.inputTokens += record.InputTokens
					b.outputTokens += record.OutputTokens
					b.cacheCreationTokens += record.CacheCreationInputTokens
					b.cacheReadTokens += record.CacheReadInputTokens

					// 累加汇总
					totalRequests++
					if record.Success {
						totalSuccess++
					} else {
						totalFailure++
					}
					totalInputTokens += record.InputTokens
					totalOutputTokens += record.OutputTokens
					totalCacheCreation += record.CacheCreationInputTokens
					totalCacheRead += record.CacheReadInputTokens

					// 同时按模型分桶（跳过无模型信息的记录）
					if model := record.Model; model != "" {
						if _, ok := modelBuckets[model]; !ok {
							modelBuckets[model] = make([]modelBucket, numPoints)
						}
						mb := &modelBuckets[model][offset]
						mb.requestCount++
						if record.Success {
							mb.successCount++
						} else {
							mb.failureCount++
						}
						mb.inputTokens += record.InputTokens
						mb.outputTokens += record.OutputTokens
					}
				}
			}
		}
	}

	// 构建数据点结果
	dataPoints := make([]GlobalHistoryDataPoint, numPoints)
	for i := 0; i < numPoints; i++ {
		b := buckets[int64(i)]
		successRate := float64(0)
		if b.requestCount > 0 {
			successRate = float64(b.successCount) / float64(b.requestCount) * 100
		}
		dataPoints[i] = GlobalHistoryDataPoint{
			Timestamp:           startTime.Add(time.Duration(i+1) * interval),
			RequestCount:        b.requestCount,
			SuccessCount:        b.successCount,
			FailureCount:        b.failureCount,
			SuccessRate:         successRate,
			InputTokens:         b.inputTokens,
			OutputTokens:        b.outputTokens,
			CacheCreationTokens: b.cacheCreationTokens,
			CacheReadTokens:     b.cacheReadTokens,
		}
	}

	// 计算平均成功率
	avgSuccessRate := float64(0)
	if totalRequests > 0 {
		avgSuccessRate = float64(totalSuccess) / float64(totalRequests) * 100
	}

	summary := GlobalStatsSummary{
		TotalRequests:            totalRequests,
		TotalSuccess:             totalSuccess,
		TotalFailure:             totalFailure,
		TotalInputTokens:         totalInputTokens,
		TotalOutputTokens:        totalOutputTokens,
		TotalCacheCreationTokens: totalCacheCreation,
		TotalCacheReadTokens:     totalCacheRead,
		AvgSuccessRate:           avgSuccessRate,
		Duration:                 duration.String(),
	}

	// 构建模型维度数据点
	var modelDataPoints map[string][]ModelHistoryDataPoint
	if len(modelBuckets) > 0 {
		modelDataPoints = make(map[string][]ModelHistoryDataPoint, len(modelBuckets))
		for model, buckets := range modelBuckets {
			points := make([]ModelHistoryDataPoint, numPoints)
			for i := 0; i < numPoints; i++ {
				points[i] = ModelHistoryDataPoint{
					Timestamp:    startTime.Add(time.Duration(i+1) * interval),
					RequestCount: buckets[i].requestCount,
					SuccessCount: buckets[i].successCount,
					FailureCount: buckets[i].failureCount,
					InputTokens:  buckets[i].inputTokens,
					OutputTokens: buckets[i].outputTokens,
				}
			}
			modelDataPoints[model] = points
		}
	}

	return GlobalStatsHistoryResponse{
		DataPoints:      dataPoints,
		Summary:         summary,
		ModelDataPoints: modelDataPoints,
	}
}

// globalBucketData 全局统计时间分桶的辅助结构
type globalBucketData struct {
	requestCount        int64
	successCount        int64
	failureCount        int64
	inputTokens         int64
	outputTokens        int64
	cacheCreationTokens int64
	cacheReadTokens     int64
}

// CalculateTodayDuration 计算"今日"时间范围（从今天 0 点到现在）
func CalculateTodayDuration() time.Duration {
	now := time.Now()
	startOfDay := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
	return now.Sub(startOfDay)
}

// ============ 渠道实时活跃度数据（用于渐变背景显示）============

// ActivitySegment 活跃度分段数据（每 6 秒一段）
// 使用 omitempty 减少 JSON 体积，0 值字段不输出
type ActivitySegment struct {
	RequestCount int64 `json:"requestCount,omitempty"`
	SuccessCount int64 `json:"successCount,omitempty"`
	FailureCount int64 `json:"failureCount,omitempty"`
	InputTokens  int64 `json:"inputTokens,omitempty"`
	OutputTokens int64 `json:"outputTokens,omitempty"`
}

// ChannelRecentActivity 渠道最近活跃度数据
// 使用稀疏 Map 格式存储 segments，只返回有数据的段
type ChannelRecentActivity struct {
	ChannelIndex int                      `json:"channelIndex"`
	Segments     map[int]*ActivitySegment `json:"segments,omitempty"` // 稀疏表示：key=段索引(0-149)，只包含有请求的段
	TotalSegs    int                      `json:"totalSegs"`          // 总段数（固定 150），前端用于展开稀疏数组
	RPM          float64                  `json:"rpm,omitempty"`      // 15分钟平均 RPM
	TPM          float64                  `json:"tpm,omitempty"`      // 15分钟平均 TPM
}

// GetRecentActivityMultiURL 获取渠道最近活跃度数据（支持多 URL 和多 Key 聚合）
// 参数：
//   - channelIndex: 渠道索引
//   - baseURLs: 渠道的所有故障转移 URL（支持多个）
//   - activeKeys: 渠道的所有活跃 API Key（支持多个）
//
// 返回：
//   - 稀疏 Map 格式的活跃度数据（只包含有请求的段，减少 JSON 体积）
//   - 自动聚合所有 URL × Key 组合的请求数据
//   - RPM/TPM 为 15 分钟平均值
func (m *MetricsManager) GetRecentActivityMultiURL(channelIndex int, baseURLs []string, activeKeys []string, serviceType string) *ChannelRecentActivity {
	// 150 段，每段 6 秒 = 900 秒 = 15 分钟
	const numSegments = 150
	const segmentDuration = 6 * time.Second

	if len(baseURLs) == 0 || len(activeKeys) == 0 {
		return &ChannelRecentActivity{
			ChannelIndex: channelIndex,
			TotalSegs:    numSegments,
		}
	}

	m.mu.RLock()
	defer m.mu.RUnlock()

	now := time.Now()

	// 时间边界对齐：将 endTime 向上对齐到下一个 segmentDuration 边界
	// 这样每次请求的分段边界都是固定的，不会因为 now 的微小变化而导致数据跳动
	// 例如：segmentDuration=6s，now=12:34:57，则 endTime=12:35:00（包含当前正在进行的段）
	endTimeUnix := now.Unix()
	segmentSeconds := int64(segmentDuration.Seconds())
	alignedEndUnix := ((endTimeUnix / segmentSeconds) + 1) * segmentSeconds
	endTime := time.Unix(alignedEndUnix, 0)
	startTime := endTime.Add(-time.Duration(numSegments) * segmentDuration)

	// 使用稀疏 Map 存储有数据的分段
	sparseSegments := make(map[int]*ActivitySegment)

	// 汇总统计
	var totalRequests, totalInputTokens, totalOutputTokens int64

	for _, metrics := range m.getIdentityMetricsByMultiURLAndKeysLocked(baseURLs, activeKeys, serviceType) {
		// 遍历该 Key 的请求历史，放入对应分段
		for _, record := range metrics.requestHistory {
			// 检查是否在 [startTime, endTime) 范围内
			if record.Timestamp.Before(startTime) || !record.Timestamp.Before(endTime) {
				continue
			}

			// 计算属于哪个分段
			offset := int(record.Timestamp.Sub(startTime) / segmentDuration)
			if offset < 0 || offset >= numSegments {
				continue
			}

			// 按需创建稀疏 segment
			seg, exists := sparseSegments[offset]
			if !exists {
				seg = &ActivitySegment{}
				sparseSegments[offset] = seg
			}

			seg.RequestCount++
			if record.Success {
				seg.SuccessCount++
			} else {
				seg.FailureCount++
			}
			seg.InputTokens += record.InputTokens
			seg.OutputTokens += record.OutputTokens

			// 累加汇总
			totalRequests++
			totalInputTokens += record.InputTokens
			totalOutputTokens += record.OutputTokens
		}
	}

	// 计算 RPM 和 TPM（基于实际窗口时长）
	// TPM 只计算输出 tokens（包含思考），不包含输入 tokens 和缓存 tokens
	windowMinutes := float64(numSegments) * segmentDuration.Minutes()
	rpm := float64(totalRequests) / windowMinutes
	tpm := float64(totalOutputTokens) / windowMinutes

	return &ChannelRecentActivity{
		ChannelIndex: channelIndex,
		Segments:     sparseSegments,
		TotalSegs:    numSegments,
		RPM:          rpm,
		TPM:          tpm,
	}
}

// ModelHistoryDataPoint 模型级别历史数据点
type ModelHistoryDataPoint struct {
	Timestamp    time.Time `json:"timestamp"`
	RequestCount int64     `json:"requestCount"`
	SuccessCount int64     `json:"successCount"`
	FailureCount int64     `json:"failureCount"`
	InputTokens  int64     `json:"inputTokens"`
	OutputTokens int64     `json:"outputTokens"`
}

// GetModelStatsHistory 获取按模型分组的历史统计
func (m *MetricsManager) GetModelStatsHistory(duration, interval time.Duration) map[string][]ModelHistoryDataPoint {
	if interval <= 0 || duration <= 0 {
		return map[string][]ModelHistoryDataPoint{}
	}

	now := time.Now()
	startTime := now.Add(-duration).Truncate(interval)
	endTime := now.Truncate(interval).Add(interval)
	numPoints := int(duration/interval) + 1

	// 快速拷贝 requestHistory 引用，缩短持锁时间
	type historyRef struct {
		history []RequestRecord
	}
	var historyRefs []historyRef

	m.mu.RLock()
	for _, metrics := range m.keyMetrics {
		// 拷贝 slice 引用（底层数组共享，但遍历时不会修改）
		historyRefs = append(historyRefs, historyRef{history: metrics.requestHistory})
	}
	m.mu.RUnlock()

	// 解锁后进行聚合计算
	// 按模型分组收集记录
	type modelBucket struct {
		requestCount int64
		successCount int64
		failureCount int64
		inputTokens  int64
		outputTokens int64
	}
	// model -> bucketIndex -> data
	modelBuckets := make(map[string][]modelBucket)

	for _, ref := range historyRefs {
		for _, record := range ref.history {
			if record.Timestamp.Before(startTime) || !record.Timestamp.Before(endTime) {
				continue
			}
			model := record.Model
			if model == "" {
				continue // 跳过没有模型信息的记录
			}
			offset := int(record.Timestamp.Sub(startTime) / interval)
			if offset < 0 || offset >= numPoints {
				continue
			}
			if _, ok := modelBuckets[model]; !ok {
				modelBuckets[model] = make([]modelBucket, numPoints)
			}
			b := &modelBuckets[model][offset]
			b.requestCount++
			if record.Success {
				b.successCount++
			} else {
				b.failureCount++
			}
			b.inputTokens += record.InputTokens
			b.outputTokens += record.OutputTokens
		}
	}

	// 构建结果
	result := make(map[string][]ModelHistoryDataPoint, len(modelBuckets))
	for model, buckets := range modelBuckets {
		points := make([]ModelHistoryDataPoint, numPoints)
		for i := 0; i < numPoints; i++ {
			points[i] = ModelHistoryDataPoint{
				Timestamp:    startTime.Add(time.Duration(i) * interval),
				RequestCount: buckets[i].requestCount,
				SuccessCount: buckets[i].successCount,
				FailureCount: buckets[i].failureCount,
				InputTokens:  buckets[i].inputTokens,
				OutputTokens: buckets[i].outputTokens,
			}
		}
		result[model] = points
	}

	return result
}
