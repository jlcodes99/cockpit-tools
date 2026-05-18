package metrics

import (
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
)

func TestMultiURLHealthTreatsMissingKeyAsAvailableCandidate(t *testing.T) {
	m := NewMetricsManager()
	defer m.Stop()

	baseURL := "https://example.com"
	oldKey := "old-key"
	newKey := "new-key"

	m.mu.Lock()
	metrics := m.getOrCreateKey(baseURL, oldKey, "openai")
	metrics.CircuitState = CircuitStateOpen
	m.mu.Unlock()

	if !m.IsChannelHealthyMultiURL([]string{baseURL}, []string{oldKey, newKey}, "openai") {
		t.Fatal("expected channel to remain healthy when a new key has no metrics yet")
	}
	if got := m.CalculateChannelFailureRateMultiURL([]string{baseURL}, []string{oldKey, newKey}, "openai"); got != 0 {
		t.Fatalf("expected failure rate 0 for missing-key candidate, got %v", got)
	}
}

func TestGetHistoricalStatsMultiURL_DeduplicatesEquivalentURLs(t *testing.T) {
	m := NewMetricsManager()
	defer m.Stop()

	baseURL := "https://gemini.example.com"
	apiKey := "test-key"
	now := time.Now()

	m.mu.Lock()
	metrics := m.getOrCreateKey(baseURL, apiKey, "")
	metrics.requestHistory = append(metrics.requestHistory, RequestRecord{
		Timestamp: now,
		Success:   true,
	})
	m.mu.Unlock()

	result := m.GetHistoricalStatsMultiURL([]string{baseURL, baseURL + "/v1"}, []string{apiKey}, "", time.Hour, 5*time.Minute)

	var totalRequests int64
	for _, point := range result {
		totalRequests += point.RequestCount
	}
	if totalRequests != 1 {
		t.Fatalf("expected 1 request after deduplicating equivalent URLs, got %d", totalRequests)
	}
}

func TestGetOrCreateKey_PromotesLegacyMetricsToIdentity(t *testing.T) {
	m := NewMetricsManagerWithConfig(10, 0.5)

	baseURL := "https://api.example.com"
	apiKey := "sk-test"
	serviceType := "openai"
	legacyKey := GenerateMetricsKey(baseURL, apiKey)
	identityKey := GenerateMetricsIdentityKey(baseURL, apiKey, serviceType)
	identityBaseURL := utils.MetricsIdentityBaseURL(baseURL, serviceType)

	legacyMetrics := &KeyMetrics{
		MetricsKey:        legacyKey,
		BaseURL:           baseURL,
		KeyMask:           utils.MaskAPIKey(apiKey),
		CircuitState:      CircuitStateHalfOpen,
		recentResults:     []bool{true},
		breakerResults:    []bool{false},
		pendingHistoryIdx: map[uint64]int{},
	}

	m.mu.Lock()
	m.keyMetrics[legacyKey] = legacyMetrics
	promoted := m.getOrCreateKey(baseURL, apiKey, serviceType)
	m.mu.Unlock()

	if promoted != legacyMetrics {
		t.Fatalf("expected promoted metrics to reuse legacy instance")
	}
	if promoted.MetricsKey != identityKey {
		t.Fatalf("metrics key = %s, want %s", promoted.MetricsKey, identityKey)
	}
	if promoted.BaseURL != identityBaseURL {
		t.Fatalf("baseURL = %s, want %s", promoted.BaseURL, identityBaseURL)
	}
	if _, exists := m.keyMetrics[legacyKey]; exists {
		t.Fatalf("expected legacy key entry removed after promotion")
	}
	if current, exists := m.keyMetrics[identityKey]; !exists || current != legacyMetrics {
		t.Fatalf("expected identity key to point to promoted legacy metrics")
	}
}

func TestGetIdentityMetricsLocked_FindsEquivalentLegacyVariant(t *testing.T) {
	m := NewMetricsManagerWithConfig(10, 0.5)

	baseURL := "https://api.example.com"
	apiKey := "sk-test"
	serviceType := "openai"
	legacyKey := GenerateMetricsKey(baseURL, apiKey)
	legacyMetrics := &KeyMetrics{
		MetricsKey:        legacyKey,
		BaseURL:           baseURL,
		KeyMask:           utils.MaskAPIKey(apiKey),
		CircuitState:      CircuitStateOpen,
		pendingHistoryIdx: map[uint64]int{},
	}

	m.mu.Lock()
	m.keyMetrics[legacyKey] = legacyMetrics
	found := m.getIdentityMetricsLocked(baseURL, apiKey, serviceType)
	m.mu.Unlock()

	if found != legacyMetrics {
		t.Fatalf("expected identity lookup to find equivalent legacy metrics")
	}
}
