package handlers

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/metrics"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/scheduler"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/warmup"
	"github.com/gin-gonic/gin"
)

// TestGetChannelLogs_AfterChannelDeletion 验证删除渠道后，logs API 对剩余渠道仍返回正确数据
func TestGetChannelLogs_AfterChannelDeletion(t *testing.T) {
	gin.SetMode(gin.TestMode)

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{
			{Name: "ch-a", BaseURL: "https://shared.example.com", APIKeys: []string{"sk-a"}},
			{Name: "ch-b", BaseURL: "https://shared.example.com", APIKeys: []string{"sk-b"}},
		},
	}

	tmpDir := t.TempDir()
	configFile := filepath.Join(tmpDir, "config.json")
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		t.Fatalf("序列化配置失败: %v", err)
	}
	if err := os.WriteFile(configFile, data, 0644); err != nil {
		t.Fatalf("写入配置文件失败: %v", err)
	}

	cfgManager, err := config.NewConfigManager(configFile)
	if err != nil {
		t.Fatalf("创建配置管理器失败: %v", err)
	}
	t.Cleanup(func() { cfgManager.Close() })

	messagesMetrics := metrics.NewMetricsManager()
	responsesMetrics := metrics.NewMetricsManager()
	geminiMetrics := metrics.NewMetricsManager()
	chatMetrics := metrics.NewMetricsManager()
	imagesMetrics := metrics.NewMetricsManager()
	traceAffinity := session.NewTraceAffinityManager()
	urlManager := warmup.NewURLManager(30*0, 3)
	sch := scheduler.NewChannelScheduler(cfgManager, messagesMetrics, responsesMetrics, geminiMetrics, chatMetrics, imagesMetrics, traceAffinity, urlManager)
	t.Cleanup(func() {
		messagesMetrics.Stop()
		responsesMetrics.Stop()
		geminiMetrics.Stop()
		chatMetrics.Stop()
		traceAffinity.Stop()
	})

	logStore := sch.GetChannelLogStore(scheduler.ChannelKindMessages)

	// 为两个渠道记录日志
	logStore.Record(0, &metrics.ChannelLog{Model: "model-a", BaseURL: "https://shared.example.com", KeyMask: "***a"})
	logStore.Record(1, &metrics.ChannelLog{Model: "model-b", BaseURL: "https://shared.example.com", KeyMask: "***b"})

	// 验证删除前 channel 1 日志存在
	logsBefore := logStore.Get(1)
	if len(logsBefore) != 1 {
		t.Fatalf("删除前 channel 1 日志数 = %d, want 1", len(logsBefore))
	}

	// 执行 RemoveAndShift 模拟删除 channel 0
	logStore.RemoveAndShift(0)

	// 验证删除后原 channel 1 的日志现在在 index 0
	logs := logStore.Get(0)
	if len(logs) != 1 || logs[0].Model != "model-b" {
		t.Fatalf("删除后 channel 0 日志 = %#v, want model-b", logs)
	}
	if logStore.Get(1) != nil {
		t.Fatalf("删除后 channel 1 应为 nil")
	}

	// 验证通过 API 查询
	r := gin.New()
	r.GET("/messages/channels/:id/logs", GetChannelLogs(logStore))

	req := httptest.NewRequest(http.MethodGet, "/messages/channels/0/logs", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200", w.Code)
	}

	var resp struct {
		ChannelIndex int                   `json:"channelIndex"`
		Logs         []*metrics.ChannelLog `json:"logs"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if resp.ChannelIndex != 0 {
		t.Fatalf("channelIndex = %d, want 0", resp.ChannelIndex)
	}
	if len(resp.Logs) != 1 || resp.Logs[0].Model != "model-b" {
		t.Fatalf("logs = %#v, want model-b", resp.Logs)
	}
}

// TestGetChannelDashboard_AfterChannelDeletion 验证 dashboard 在渠道删除后 metrics 索引一致性
func TestGetChannelDashboard_AfterChannelDeletion(t *testing.T) {
	gin.SetMode(gin.TestMode)

	cfg := config.Config{
		Upstream: []config.UpstreamConfig{
			{Name: "ch-a", BaseURL: "https://api-a.example.com", APIKeys: []string{"sk-a"}},
			{Name: "ch-b", BaseURL: "https://api-b.example.com", APIKeys: []string{"sk-b"}},
		},
	}

	tmpDir := t.TempDir()
	configFile := filepath.Join(tmpDir, "config.json")
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		t.Fatalf("序列化配置失败: %v", err)
	}
	if err := os.WriteFile(configFile, data, 0644); err != nil {
		t.Fatalf("写入配置文件失败: %v", err)
	}

	cfgManager, err := config.NewConfigManager(configFile)
	if err != nil {
		t.Fatalf("创建配置管理器失败: %v", err)
	}
	t.Cleanup(func() { cfgManager.Close() })

	messagesMetrics := metrics.NewMetricsManager()
	responsesMetrics := metrics.NewMetricsManager()
	geminiMetrics := metrics.NewMetricsManager()
	chatMetrics := metrics.NewMetricsManager()
	imagesMetrics := metrics.NewMetricsManager()
	traceAffinity := session.NewTraceAffinityManager()
	urlManager := warmup.NewURLManager(0, 3)
	sch := scheduler.NewChannelScheduler(cfgManager, messagesMetrics, responsesMetrics, geminiMetrics, chatMetrics, imagesMetrics, traceAffinity, urlManager)
	t.Cleanup(func() {
		messagesMetrics.Stop()
		responsesMetrics.Stop()
		geminiMetrics.Stop()
		chatMetrics.Stop()
		traceAffinity.Stop()
	})

	// 为 channel 1 (ch-b) 记录 metrics
	messagesMetrics.RecordSuccess("https://api-b.example.com", "sk-b", "claude")

	// 模拟删除 channel 0，此时配置中只剩 channel-b
	removed, err := cfgManager.RemoveUpstream(0)
	if err != nil {
		t.Fatalf("删除渠道失败: %v", err)
	}
	sch.GetChannelLogStore(scheduler.ChannelKindMessages).RemoveAndShift(0)
	sch.DeleteChannelMetrics(removed, scheduler.ChannelKindMessages)

	// 请求 dashboard
	r := gin.New()
	r.GET("/messages/channels/dashboard", GetChannelDashboard(cfgManager, sch))

	req := httptest.NewRequest(http.MethodGet, "/messages/channels/dashboard?type=messages", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200, body = %s", w.Code, w.Body.String())
	}

	var resp struct {
		Channels []map[string]any `json:"channels"`
		Metrics  []map[string]any `json:"metrics"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}

	// 删除后只剩 1 个渠道
	if len(resp.Channels) != 1 {
		t.Fatalf("channels count = %d, want 1", len(resp.Channels))
	}
	if resp.Channels[0]["name"] != "ch-b" {
		t.Fatalf("channel name = %v, want ch-b", resp.Channels[0]["name"])
	}

	// metrics 索引应该和 channels 索引对齐
	if len(resp.Metrics) != 1 {
		t.Fatalf("metrics count = %d, want 1", len(resp.Metrics))
	}
	metricsIdx, ok := resp.Metrics[0]["channelIndex"].(float64)
	if !ok || metricsIdx != 0 {
		t.Fatalf("metrics channelIndex = %v, want 0", resp.Metrics[0]["channelIndex"])
	}
}
