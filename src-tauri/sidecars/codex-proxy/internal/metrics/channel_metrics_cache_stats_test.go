package metrics

import (
	"math"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
)

func TestToResponse_TimeWindowsIncludesCacheStats(t *testing.T) {
	m := NewMetricsManagerWithConfig(10, 0.5)

	baseURL := "https://example.com"
	key1 := "k1"
	key2 := "k2"

	m.RecordSuccessWithUsage(baseURL, key1, "openai", &types.Usage{
		InputTokens:              100,
		OutputTokens:             10,
		CacheCreationInputTokens: 20,
		CacheReadInputTokens:     50,
	})
	m.RecordSuccessWithUsage(baseURL, key2, "openai", &types.Usage{
		InputTokens:  200,
		OutputTokens: 20,
	})

	resp := m.ToResponse(0, baseURL, []string{key1, key2}, "openai", 0)
	stats, ok := resp.TimeWindows["15m"]
	if !ok {
		t.Fatalf("expected timeWindows[15m] to exist")
	}

	if stats.InputTokens != 300 {
		t.Fatalf("expected inputTokens=300, got %d", stats.InputTokens)
	}
	if stats.OutputTokens != 30 {
		t.Fatalf("expected outputTokens=30, got %d", stats.OutputTokens)
	}
	if stats.CacheCreationTokens != 20 {
		t.Fatalf("expected cacheCreationTokens=20, got %d", stats.CacheCreationTokens)
	}
	if stats.CacheReadTokens != 50 {
		t.Fatalf("expected cacheReadTokens=50, got %d", stats.CacheReadTokens)
	}

	wantHitRate := float64(50) / float64(50+300) * 100
	if math.Abs(stats.CacheHitRate-wantHitRate) > 0.01 {
		t.Fatalf("expected cacheHitRate=%.4f, got %.4f", wantHitRate, stats.CacheHitRate)
	}
}

func TestRecordSuccessWithUsage_NormalizesResponsesPromptTotalsForCacheHitRate(t *testing.T) {
	m := NewMetricsManagerWithConfig(10, 0.5)

	baseURL := "https://example.com"
	key := "k1"

	m.RecordSuccessWithUsage(baseURL, key, "openai", &types.Usage{
		InputTokens:          114931,
		PromptTokensTotal:    114931,
		OutputTokens:         100,
		CacheReadInputTokens: 112256,
	})

	resp := m.ToResponse(0, baseURL, []string{key}, "openai", 0)
	stats, ok := resp.TimeWindows["15m"]
	if !ok {
		t.Fatalf("expected timeWindows[15m] to exist")
	}

	wantInput := int64(114931 - 112256)
	if stats.InputTokens != wantInput {
		t.Fatalf("expected normalized inputTokens=%d, got %d", wantInput, stats.InputTokens)
	}
	if stats.CacheReadTokens != 112256 {
		t.Fatalf("expected cacheReadTokens=112256, got %d", stats.CacheReadTokens)
	}

	wantHitRate := float64(112256) / float64(114931) * 100
	if math.Abs(stats.CacheHitRate-wantHitRate) > 0.01 {
		t.Fatalf("expected cacheHitRate=%.4f, got %.4f", wantHitRate, stats.CacheHitRate)
	}
}

func TestRecordSuccessWithUsage_CacheCreationFallbackFromTTLBreakdown(t *testing.T) {
	m := NewMetricsManagerWithConfig(10, 0.5)

	baseURL := "https://example.com"
	key := "k1"

	// 上游有时只返回 TTL 细分字段（5m/1h），不返回 cache_creation_input_tokens。
	m.RecordSuccessWithUsage(baseURL, key, "openai", &types.Usage{
		InputTokens:                100,
		OutputTokens:               10,
		CacheCreationInputTokens:   0,
		CacheCreation5mInputTokens: 20,
		CacheCreation1hInputTokens: 30,
		CacheReadInputTokens:       50,
	})

	resp := m.ToResponse(0, baseURL, []string{key}, "openai", 0)
	stats, ok := resp.TimeWindows["15m"]
	if !ok {
		t.Fatalf("expected timeWindows[15m] to exist")
	}

	if stats.CacheCreationTokens != 50 {
		t.Fatalf("expected cacheCreationTokens=50, got %d", stats.CacheCreationTokens)
	}
	if stats.CacheReadTokens != 50 {
		t.Fatalf("expected cacheReadTokens=50, got %d", stats.CacheReadTokens)
	}
}

func TestRecordRequestFinalizeOutcome_PromotesLegacyMetricsToIdentityWhenOnlyLegacyDataExists(t *testing.T) {
	m := NewMetricsManagerWithConfig(10, 0.5)

	baseURL := "https://api.example.com"
	apiKey := "sk-test"
	serviceType := "openai"
	legacyKey := GenerateMetricsKey(baseURL, apiKey)
	identityKey := GenerateMetricsIdentityKey(baseURL, apiKey, serviceType)
	identityBaseURL := utils.MetricsIdentityBaseURL(baseURL, serviceType)
	if legacyKey == identityKey {
		t.Fatalf("expected legacy and identity keys to differ")
	}

	m.mu.Lock()
	legacyMetrics := &KeyMetrics{
		MetricsKey:          legacyKey,
		BaseURL:             baseURL,
		KeyMask:             utils.MaskAPIKey(apiKey),
		CircuitState:        CircuitStateHalfOpen,
		ProbeInFlight:       true,
		recentResults:       make([]bool, 0, m.windowSize),
		breakerResults:      make([]bool, 0, m.windowSize),
		requestHistory:      []RequestRecord{{Timestamp: time.Now(), Success: true}},
		pendingHistoryIdx:   map[uint64]int{1: 0},
		ConsecutiveFailures: 2,
	}
	m.keyMetrics[legacyKey] = legacyMetrics
	m.mu.Unlock()

	m.RecordRequestFinalizeFailureWithClass(baseURL, apiKey, serviceType, 1, FailureClassRetryable)

	m.mu.RLock()
	defer m.mu.RUnlock()
	identityMetrics, exists := m.keyMetrics[identityKey]
	if !exists {
		t.Fatalf("expected legacy metrics to be promoted to identity during finalize")
	}
	if identityMetrics != legacyMetrics {
		t.Fatalf("expected promoted identity metrics to reuse legacy instance")
	}
	if _, exists := m.keyMetrics[legacyKey]; exists {
		t.Fatalf("expected legacy key entry to be removed after promotion")
	}
	if identityMetrics.MetricsKey != identityKey {
		t.Fatalf("identity metrics key = %s, want %s", identityMetrics.MetricsKey, identityKey)
	}
	if identityMetrics.BaseURL != identityBaseURL {
		t.Fatalf("identity baseURL = %s, want %s", identityMetrics.BaseURL, identityBaseURL)
	}
	if identityMetrics.CircuitState != CircuitStateOpen {
		t.Fatalf("identity circuit state = %v, want %v", identityMetrics.CircuitState, CircuitStateOpen)
	}
	if identityMetrics.FailureCount != 1 {
		t.Fatalf("identity failure count = %d, want 1", identityMetrics.FailureCount)
	}
	if identityMetrics.ProbeInFlight {
		t.Fatalf("expected probe state to be cleared after half-open failure")
	}
}

func TestRecordRequestFinalizeClientCancel_UsesIdentityMetrics(t *testing.T) {
	m := NewMetricsManagerWithConfig(10, 0.5)

	baseURL := "https://api.example.com"
	apiKey := "sk-test"
	serviceType := "openai"
	identityKey := GenerateMetricsIdentityKey(baseURL, apiKey, serviceType)
	identityBaseURL := utils.MetricsIdentityBaseURL(baseURL, serviceType)

	requestID := m.RecordRequestConnectedAt(baseURL, apiKey, serviceType, "", time.Now())
	m.RecordRequestFinalizeClientCancel(baseURL, apiKey, serviceType, requestID)

	m.mu.RLock()
	defer m.mu.RUnlock()
	identityMetrics, exists := m.keyMetrics[identityKey]
	if !exists {
		t.Fatalf("expected identity metrics to exist after client cancel")
	}
	if identityMetrics.BaseURL != identityBaseURL {
		t.Fatalf("identity baseURL = %s, want %s", identityMetrics.BaseURL, identityBaseURL)
	}
	if identityMetrics.RequestCount != 1 {
		t.Fatalf("identity request count = %d, want 1", identityMetrics.RequestCount)
	}
	if len(identityMetrics.requestHistory) != 0 {
		t.Fatalf("identity request history len = %d, want 0", len(identityMetrics.requestHistory))
	}
	if _, ok := identityMetrics.pendingHistoryIdx[requestID]; ok {
		t.Fatalf("pending request id should be cleared")
	}
}
