package chat

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
	"github.com/gin-gonic/gin"
)

func TestDeleteUpstream_PreservesRemainingChannelLogs(t *testing.T) {
	tmpDir := t.TempDir()
	configPath := filepath.Join(tmpDir, "config.json")
	cfg := config.Config{ChatUpstream: []config.UpstreamConfig{
		{Name: "channel-a", BaseURL: "https://shared.example.com", APIKeys: []string{"sk-a"}},
		{Name: "channel-b", BaseURL: "https://shared.example.com", APIKeys: []string{"sk-b"}},
	}}
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		t.Fatalf("序列化配置失败: %v", err)
	}
	if err := os.WriteFile(configPath, data, 0644); err != nil {
		t.Fatalf("写入配置文件失败: %v", err)
	}

	cm, err := config.NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("创建配置管理器失败: %v", err)
	}
	t.Cleanup(func() { cm.Close() })

	messagesMetrics := metrics.NewMetricsManager()
	responsesMetrics := metrics.NewMetricsManager()
	geminiMetrics := metrics.NewMetricsManager()
	chatMetrics := metrics.NewMetricsManager()
	imagesMetrics := metrics.NewMetricsManager()
	traceAffinity := session.NewTraceAffinityManager()
	t.Cleanup(func() {
		messagesMetrics.Stop()
		responsesMetrics.Stop()
		geminiMetrics.Stop()
		chatMetrics.Stop()
		traceAffinity.Stop()
	})

	sch := scheduler.NewChannelScheduler(cm, messagesMetrics, responsesMetrics, geminiMetrics, chatMetrics, imagesMetrics, traceAffinity, nil)
	logStore := sch.GetChannelLogStore(scheduler.ChannelKindChat)
	logStore.Record(0, &metrics.ChannelLog{Model: "deleted-channel"})
	logStore.Record(1, &metrics.ChannelLog{Model: "remaining-channel"})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.DELETE("/chat/channels/:id", DeleteUpstream(cm, sch))

	req := httptest.NewRequest(http.MethodDelete, "/chat/channels/0", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200, body = %s", w.Code, w.Body.String())
	}

	remainingLogs := logStore.Get(0)
	if len(remainingLogs) != 1 || remainingLogs[0].Model != "remaining-channel" {
		t.Fatalf("remaining logs = %#v, want remaining-channel", remainingLogs)
	}
	if got := logStore.Get(1); got != nil {
		t.Fatalf("channel 1 logs = %#v, want nil", got)
	}
}
