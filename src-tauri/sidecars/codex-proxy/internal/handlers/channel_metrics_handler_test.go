package handlers

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/warmup"
	"github.com/gin-gonic/gin"
)

type fakePersistenceStore struct {
	bucketsByMetricsKey map[string][]metrics.AggregatedBucket
}

func (f *fakePersistenceStore) AddRecord(record metrics.PersistentRecord) {}
func (f *fakePersistenceStore) LoadRecords(since time.Time, apiType string) ([]metrics.PersistentRecord, error) {
	return nil, nil
}
func (f *fakePersistenceStore) LoadLatestTimestamps(apiType string) (map[string]*metrics.KeyLatestTimestamps, error) {
	return nil, nil
}
func (f *fakePersistenceStore) LoadCircuitStates(apiType string) (map[string]*metrics.PersistentCircuitState, error) {
	return nil, nil
}
func (f *fakePersistenceStore) UpsertCircuitState(state metrics.PersistentCircuitState) error {
	return nil
}
func (f *fakePersistenceStore) QueryAggregatedHistory(apiType string, since time.Time, intervalSeconds int64, metricsKey string, baseURL string) ([]metrics.AggregatedBucket, error) {
	return append([]metrics.AggregatedBucket(nil), f.bucketsByMetricsKey[metricsKey]...), nil
}
func (f *fakePersistenceStore) CleanupOldRecords(before time.Time) (int64, error) { return 0, nil }
func (f *fakePersistenceStore) DeleteRecordsByMetricsKeys(metricsKeys []string, apiType string) (int64, error) {
	return 0, nil
}
func (f *fakePersistenceStore) DeleteCircuitStatesByMetricsKeys(metricsKeys []string, apiType string) (int64, error) {
	return 0, nil
}
func (f *fakePersistenceStore) Close() error { return nil }

func TestFilterBucketsByURLsIncludesEquivalentLegacyMetricsKeys(t *testing.T) {
	baseURL := "https://shared.example.com"
	apiKey := "sk-a"
	serviceType := "claude"
	legacyKey := metrics.GenerateMetricsKey(baseURL, apiKey)
	identityKey := metrics.GenerateMetricsIdentityKey(baseURL, apiKey, serviceType)

	store := &fakePersistenceStore{
		bucketsByMetricsKey: map[string][]metrics.AggregatedBucket{
			legacyKey: {
				{Timestamp: time.Unix(3600, 0), TotalRequests: 2, SuccessCount: 1},
			},
			identityKey: {
				{Timestamp: time.Unix(3600, 0), TotalRequests: 3, SuccessCount: 3},
			},
		},
	}

	buckets := filterBucketsByURLs(store, "messages", time.Unix(0, 0), 3600, []string{baseURL}, []string{apiKey}, serviceType)
	if len(buckets) != 1 {
		t.Fatalf("buckets len = %d, want 1", len(buckets))
	}
	if buckets[0].TotalRequests != 5 {
		t.Fatalf("total requests = %d, want 5", buckets[0].TotalRequests)
	}
	if buckets[0].SuccessCount != 4 {
		t.Fatalf("success count = %d, want 4", buckets[0].SuccessCount)
	}
}

func TestFilterBucketsByURLsIsolatesChannelsByMetricsKey(t *testing.T) {
	baseURL := "https://shared.example.com"
	keyA := "sk-a"
	keyB := "sk-b"

	store := &fakePersistenceStore{
		bucketsByMetricsKey: map[string][]metrics.AggregatedBucket{
			metrics.GenerateMetricsIdentityKey(baseURL, keyA, "claude"): {
				{Timestamp: time.Unix(3600, 0), TotalRequests: 1, SuccessCount: 1},
			},
			metrics.GenerateMetricsIdentityKey(baseURL, keyB, "claude"): {
				{Timestamp: time.Unix(3600, 0), TotalRequests: 2, SuccessCount: 1},
			},
		},
	}

	channelABuckets := filterBucketsByURLs(store, "messages", time.Unix(0, 0), 3600, []string{baseURL}, []string{keyA}, "claude")
	channelBBuckets := filterBucketsByURLs(store, "messages", time.Unix(0, 0), 3600, []string{baseURL}, []string{keyB}, "claude")

	if len(channelABuckets) != 1 || channelABuckets[0].TotalRequests != 1 {
		t.Fatalf("channel A buckets = %+v, want only keyA data", channelABuckets)
	}
	if len(channelBBuckets) != 1 || channelBBuckets[0].TotalRequests != 2 {
		t.Fatalf("channel B buckets = %+v, want only keyB data", channelBBuckets)
	}
}

func TestChannelMetricsHandlers_FallbackServiceTypeForLegacyConfig(t *testing.T) {
	gin.SetMode(gin.TestMode)

	tests := []struct {
		name        string
		serviceType string
		baseURL     string
		record      func(mm *metrics.MetricsManager, baseURL, apiKey, serviceType string)
		register    func(r *gin.Engine, mm *metrics.MetricsManager, cfgManager *config.ConfigManager)
		requestPath string
		buildConfig func(baseURL string) config.Config
		assertBody  func(t *testing.T, body []byte)
	}{
		{
			name:        "gemini metrics fallback",
			serviceType: "gemini",
			baseURL:     "https://example.com",
			record: func(mm *metrics.MetricsManager, baseURL, apiKey, serviceType string) {
				for i := 0; i < 3; i++ {
					mm.RecordFailure(baseURL, apiKey, serviceType)
				}
			},
			register: func(r *gin.Engine, mm *metrics.MetricsManager, cfgManager *config.ConfigManager) {
				r.GET("/gemini/channels/metrics", GetGeminiChannelMetrics(mm, cfgManager))
			},
			requestPath: "/gemini/channels/metrics",
			buildConfig: func(baseURL string) config.Config {
				return config.Config{GeminiUpstream: []config.UpstreamConfig{{Name: "gemini-legacy", BaseURL: baseURL, APIKeys: []string{"sk-test"}}}}
			},
			assertBody: func(t *testing.T, body []byte) {
				var resp []map[string]any
				if err := json.Unmarshal(body, &resp); err != nil {
					t.Fatalf("unmarshal response: %v", err)
				}
				if len(resp) != 1 || resp[0]["circuitState"] != "open" {
					t.Fatalf("unexpected metrics response: %s", string(body))
				}
			},
		},
		{
			name:        "chat metrics fallback",
			serviceType: "openai",
			baseURL:     "https://example.com",
			record: func(mm *metrics.MetricsManager, baseURL, apiKey, serviceType string) {
				for i := 0; i < 3; i++ {
					mm.RecordFailure(baseURL, apiKey, serviceType)
				}
			},
			register: func(r *gin.Engine, mm *metrics.MetricsManager, cfgManager *config.ConfigManager) {
				r.GET("/chat/channels/metrics", GetChatChannelMetrics(mm, cfgManager))
			},
			requestPath: "/chat/channels/metrics",
			buildConfig: func(baseURL string) config.Config {
				return config.Config{ChatUpstream: []config.UpstreamConfig{{Name: "chat-legacy", BaseURL: baseURL, APIKeys: []string{"sk-test"}}}}
			},
			assertBody: func(t *testing.T, body []byte) {
				var resp []map[string]any
				if err := json.Unmarshal(body, &resp); err != nil {
					t.Fatalf("unmarshal response: %v", err)
				}
				if len(resp) != 1 || resp[0]["circuitState"] != "open" {
					t.Fatalf("unexpected metrics response: %s", string(body))
				}
			},
		},
		{
			name:        "gemini history fallback",
			serviceType: "gemini",
			baseURL:     "https://example.com",
			record: func(mm *metrics.MetricsManager, baseURL, apiKey, serviceType string) {
				mm.RecordSuccess(baseURL, apiKey, serviceType)
			},
			register: func(r *gin.Engine, mm *metrics.MetricsManager, cfgManager *config.ConfigManager) {
				r.GET("/gemini/channels/metrics/history", GetGeminiChannelMetricsHistory(mm, cfgManager))
			},
			requestPath: "/gemini/channels/metrics/history?duration=1h",
			buildConfig: func(baseURL string) config.Config {
				return config.Config{GeminiUpstream: []config.UpstreamConfig{{Name: "gemini-legacy", BaseURL: baseURL, APIKeys: []string{"sk-test"}}}}
			},
			assertBody: func(t *testing.T, body []byte) {
				var resp []struct {
					DataPoints []any `json:"dataPoints"`
				}
				if err := json.Unmarshal(body, &resp); err != nil {
					t.Fatalf("unmarshal response: %v", err)
				}
				if len(resp) != 1 || len(resp[0].DataPoints) == 0 {
					t.Fatalf("unexpected history response: %s", string(body))
				}
			},
		},
		{
			name:        "chat key history fallback",
			serviceType: "openai",
			baseURL:     "https://example.com",
			record: func(mm *metrics.MetricsManager, baseURL, apiKey, serviceType string) {
				mm.RecordSuccess(baseURL, apiKey, serviceType)
			},
			register: func(r *gin.Engine, mm *metrics.MetricsManager, cfgManager *config.ConfigManager) {
				r.GET("/chat/channels/:id/keys/metrics/history", GetChatChannelKeyMetricsHistory(mm, cfgManager))
			},
			requestPath: "/chat/channels/0/keys/metrics/history?duration=1h",
			buildConfig: func(baseURL string) config.Config {
				return config.Config{ChatUpstream: []config.UpstreamConfig{{Name: "chat-legacy", BaseURL: baseURL, APIKeys: []string{"sk-test"}}}}
			},
			assertBody: func(t *testing.T, body []byte) {
				var resp struct {
					Keys []any `json:"keys"`
				}
				if err := json.Unmarshal(body, &resp); err != nil {
					t.Fatalf("unmarshal response: %v", err)
				}
				if len(resp.Keys) == 0 {
					t.Fatalf("unexpected key history response: %s", string(body))
				}
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cfg := tt.buildConfig(tt.baseURL)
			configFile := filepath.Join(t.TempDir(), "config.json")
			data, err := json.MarshalIndent(cfg, "", "  ")
			if err != nil {
				t.Fatalf("marshal config: %v", err)
			}
			if err := os.WriteFile(configFile, data, 0644); err != nil {
				t.Fatalf("write config: %v", err)
			}

			cfgManager, err := config.NewConfigManager(configFile)
			if err != nil {
				t.Fatalf("new config manager: %v", err)
			}
			defer cfgManager.Close()

			metricsManager := metrics.NewMetricsManager()
			defer metricsManager.Stop()
			tt.record(metricsManager, tt.baseURL, "sk-test", tt.serviceType)

			r := gin.New()
			tt.register(r, metricsManager, cfgManager)

			req := httptest.NewRequest(http.MethodGet, tt.requestPath, nil)
			w := httptest.NewRecorder()
			r.ServeHTTP(w, req)

			if w.Code != http.StatusOK {
				t.Fatalf("status=%d, want=200, body=%s", w.Code, w.Body.String())
			}
			tt.assertBody(t, w.Body.Bytes())
		})
	}
}

func TestResumeChannel_RestoresBlacklistedKeys(t *testing.T) {
	gin.SetMode(gin.TestMode)

	tests := []struct {
		name        string
		path        string
		register    func(r *gin.Engine, sch *scheduler.ChannelScheduler, cfgManager *config.ConfigManager)
		buildConfig func() config.Config
		checkResult func(t *testing.T, sch *scheduler.ChannelScheduler, got config.Config)
	}{
		{
			name: "messages",
			path: "/messages/channels/0/resume",
			register: func(r *gin.Engine, sch *scheduler.ChannelScheduler, cfgManager *config.ConfigManager) {
				r.POST("/messages/channels/:id/resume", ResumeChannel(sch, cfgManager, false))
			},
			buildConfig: func() config.Config {
				return config.Config{Upstream: []config.UpstreamConfig{{
					Name:        "msg-test",
					ServiceType: "claude",
					BaseURL:     "https://example.com",
					Status:      "suspended",
					APIKeys:     []string{"sk-active"},
					DisabledAPIKeys: []config.DisabledKeyInfo{{
						Key:        "sk-disabled",
						Reason:     "insufficient_balance",
						Message:    "no balance",
						DisabledAt: "2026-04-11T00:00:00Z",
					}},
				}}}
			},
			checkResult: func(t *testing.T, sch *scheduler.ChannelScheduler, got config.Config) {
				t.Helper()
				if len(got.Upstream[0].DisabledAPIKeys) != 0 {
					t.Fatalf("disabledApiKeys=%v, want empty", got.Upstream[0].DisabledAPIKeys)
				}
				foundActive := false
				for _, key := range got.Upstream[0].APIKeys {
					if key == "sk-disabled" {
						foundActive = true
						break
					}
				}
				if !foundActive {
					t.Fatalf("restored key not found in apiKeys: %v", got.Upstream[0].APIKeys)
				}
				baseURL := got.Upstream[0].BaseURL
				serviceType := scheduler.NormalizedMetricsServiceType(scheduler.ChannelKindMessages, got.Upstream[0].ServiceType)
				for _, key := range got.Upstream[0].APIKeys {
					if state := sch.GetMessagesMetricsManager().GetKeyCircuitState(baseURL, key, serviceType); state != metrics.CircuitStateClosed {
						t.Fatalf("messages circuit state for %s = %v, want closed", key, state)
					}
				}
			},
		},
		{
			name: "responses",
			path: "/responses/channels/0/resume",
			register: func(r *gin.Engine, sch *scheduler.ChannelScheduler, cfgManager *config.ConfigManager) {
				r.POST("/responses/channels/:id/resume", ResumeChannel(sch, cfgManager, true))
			},
			buildConfig: func() config.Config {
				return config.Config{ResponsesUpstream: []config.UpstreamConfig{{
					Name:        "resp-test",
					ServiceType: "responses",
					BaseURL:     "https://example.com",
					Status:      "suspended",
					APIKeys:     []string{"sk-active"},
					DisabledAPIKeys: []config.DisabledKeyInfo{{
						Key:        "sk-disabled",
						Reason:     "insufficient_balance",
						Message:    "no balance",
						DisabledAt: "2026-04-11T00:00:00Z",
					}},
				}}}
			},
			checkResult: func(t *testing.T, sch *scheduler.ChannelScheduler, got config.Config) {
				t.Helper()
				if len(got.ResponsesUpstream[0].DisabledAPIKeys) != 0 {
					t.Fatalf("disabledApiKeys=%v, want empty", got.ResponsesUpstream[0].DisabledAPIKeys)
				}
				foundActive := false
				for _, key := range got.ResponsesUpstream[0].APIKeys {
					if key == "sk-disabled" {
						foundActive = true
						break
					}
				}
				if !foundActive {
					t.Fatalf("restored key not found in apiKeys: %v", got.ResponsesUpstream[0].APIKeys)
				}
				baseURL := got.ResponsesUpstream[0].BaseURL
				serviceType := scheduler.NormalizedMetricsServiceType(scheduler.ChannelKindResponses, got.ResponsesUpstream[0].ServiceType)
				for _, key := range got.ResponsesUpstream[0].APIKeys {
					if state := sch.GetResponsesMetricsManager().GetKeyCircuitState(baseURL, key, serviceType); state != metrics.CircuitStateClosed {
						t.Fatalf("responses circuit state for %s = %v, want closed", key, state)
					}
				}
			},
		},
		{
			name: "chat",
			path: "/chat/channels/0/resume",
			register: func(r *gin.Engine, sch *scheduler.ChannelScheduler, cfgManager *config.ConfigManager) {
				r.POST("/chat/channels/:id/resume", ResumeChannelWithKind(sch, cfgManager, scheduler.ChannelKindChat))
			},
			buildConfig: func() config.Config {
				return config.Config{ChatUpstream: []config.UpstreamConfig{{
					Name:        "chat-test",
					ServiceType: "openai",
					BaseURL:     "https://example.com",
					Status:      "suspended",
					APIKeys:     []string{"sk-active"},
					DisabledAPIKeys: []config.DisabledKeyInfo{{
						Key:        "sk-disabled",
						Reason:     "insufficient_balance",
						Message:    "no balance",
						DisabledAt: "2026-04-11T00:00:00Z",
					}},
				}}}
			},
			checkResult: func(t *testing.T, sch *scheduler.ChannelScheduler, got config.Config) {
				t.Helper()
				if len(got.ChatUpstream[0].DisabledAPIKeys) != 0 {
					t.Fatalf("disabledApiKeys=%v, want empty", got.ChatUpstream[0].DisabledAPIKeys)
				}
				foundActive := false
				for _, key := range got.ChatUpstream[0].APIKeys {
					if key == "sk-disabled" {
						foundActive = true
						break
					}
				}
				if !foundActive {
					t.Fatalf("restored key not found in apiKeys: %v", got.ChatUpstream[0].APIKeys)
				}
				baseURL := got.ChatUpstream[0].BaseURL
				serviceType := scheduler.NormalizedMetricsServiceType(scheduler.ChannelKindChat, got.ChatUpstream[0].ServiceType)
				for _, key := range got.ChatUpstream[0].APIKeys {
					if state := sch.GetChatMetricsManager().GetKeyCircuitState(baseURL, key, serviceType); state != metrics.CircuitStateClosed {
						t.Fatalf("chat circuit state for %s = %v, want closed", key, state)
					}
				}
			},
		},
		{
			name: "gemini",
			path: "/gemini/channels/0/resume",
			register: func(r *gin.Engine, sch *scheduler.ChannelScheduler, cfgManager *config.ConfigManager) {
				r.POST("/gemini/channels/:id/resume", ResumeChannelWithKind(sch, cfgManager, scheduler.ChannelKindGemini))
			},
			buildConfig: func() config.Config {
				return config.Config{GeminiUpstream: []config.UpstreamConfig{{
					Name:        "gemini-test",
					ServiceType: "gemini",
					BaseURL:     "https://example.com",
					Status:      "suspended",
					APIKeys:     []string{"sk-active"},
					DisabledAPIKeys: []config.DisabledKeyInfo{{
						Key:        "sk-disabled",
						Reason:     "insufficient_balance",
						Message:    "no balance",
						DisabledAt: "2026-04-11T00:00:00Z",
					}},
				}}}
			},
			checkResult: func(t *testing.T, sch *scheduler.ChannelScheduler, got config.Config) {
				t.Helper()
				if len(got.GeminiUpstream[0].DisabledAPIKeys) != 0 {
					t.Fatalf("disabledApiKeys=%v, want empty", got.GeminiUpstream[0].DisabledAPIKeys)
				}
				foundActive := false
				for _, key := range got.GeminiUpstream[0].APIKeys {
					if key == "sk-disabled" {
						foundActive = true
						break
					}
				}
				if !foundActive {
					t.Fatalf("restored key not found in apiKeys: %v", got.GeminiUpstream[0].APIKeys)
				}
				baseURL := got.GeminiUpstream[0].BaseURL
				serviceType := scheduler.NormalizedMetricsServiceType(scheduler.ChannelKindGemini, got.GeminiUpstream[0].ServiceType)
				for _, key := range got.GeminiUpstream[0].APIKeys {
					if state := sch.GetGeminiMetricsManager().GetKeyCircuitState(baseURL, key, serviceType); state != metrics.CircuitStateClosed {
						t.Fatalf("gemini circuit state for %s = %v, want closed", key, state)
					}
				}
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cfg := tt.buildConfig()

			configFile := filepath.Join(t.TempDir(), "config.json")
			data, err := json.MarshalIndent(cfg, "", "  ")
			if err != nil {
				t.Fatalf("marshal config: %v", err)
			}
			if err := os.WriteFile(configFile, data, 0644); err != nil {
				t.Fatalf("write config: %v", err)
			}

			cfgManager, err := config.NewConfigManager(configFile)
			if err != nil {
				t.Fatalf("new config manager: %v", err)
			}
			defer cfgManager.Close()

			messagesMetrics := metrics.NewMetricsManager()
			responsesMetrics := metrics.NewMetricsManager()
			geminiMetrics := metrics.NewMetricsManager()
			chatMetrics := metrics.NewMetricsManager()
			imagesMetrics := metrics.NewMetricsManager()
			defer messagesMetrics.Stop()
			defer responsesMetrics.Stop()
			defer geminiMetrics.Stop()
			defer chatMetrics.Stop()

			traceAffinity := session.NewTraceAffinityManager()
			defer traceAffinity.Stop()
			urlManager := warmup.NewURLManager(30*time.Second, 3)
			sch := scheduler.NewChannelScheduler(cfgManager, messagesMetrics, responsesMetrics, geminiMetrics, chatMetrics, imagesMetrics, traceAffinity, urlManager)

			var baseURL string
			var kind scheduler.ChannelKind
			var serviceType string
			switch tt.name {
			case "messages":
				baseURL = cfg.Upstream[0].BaseURL
				kind = scheduler.ChannelKindMessages
				serviceType = scheduler.NormalizedMetricsServiceType(kind, cfg.Upstream[0].ServiceType)
				messagesMetrics.MoveKeyToHalfOpen(baseURL, "sk-active", serviceType)
			case "responses":
				baseURL = cfg.ResponsesUpstream[0].BaseURL
				kind = scheduler.ChannelKindResponses
				serviceType = scheduler.NormalizedMetricsServiceType(kind, cfg.ResponsesUpstream[0].ServiceType)
				responsesMetrics.MoveKeyToHalfOpen(baseURL, "sk-active", serviceType)
			case "chat":
				baseURL = cfg.ChatUpstream[0].BaseURL
				kind = scheduler.ChannelKindChat
				serviceType = scheduler.NormalizedMetricsServiceType(kind, cfg.ChatUpstream[0].ServiceType)
				chatMetrics.MoveKeyToHalfOpen(baseURL, "sk-active", serviceType)
			case "gemini":
				baseURL = cfg.GeminiUpstream[0].BaseURL
				kind = scheduler.ChannelKindGemini
				serviceType = scheduler.NormalizedMetricsServiceType(kind, cfg.GeminiUpstream[0].ServiceType)
				geminiMetrics.MoveKeyToHalfOpen(baseURL, "sk-active", serviceType)
			}

			r := gin.New()
			tt.register(r, sch, cfgManager)

			req := httptest.NewRequest(http.MethodPost, tt.path, nil)
			w := httptest.NewRecorder()
			r.ServeHTTP(w, req)

			if w.Code != http.StatusOK {
				t.Fatalf("status=%d, want=200, body=%s", w.Code, w.Body.String())
			}

			var resp struct {
				Success      bool   `json:"success"`
				Message      string `json:"message"`
				RestoredKeys int    `json:"restoredKeys"`
			}
			if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
				t.Fatalf("unmarshal response: %v", err)
			}
			if !resp.Success {
				t.Fatalf("success=%v, want=true", resp.Success)
			}
			if resp.RestoredKeys != 1 {
				t.Fatalf("restoredKeys=%d, want=1", resp.RestoredKeys)
			}

			tt.checkResult(t, sch, cfgManager.GetConfig())
		})
	}
}
