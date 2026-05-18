package gemini

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

func setupGeminiConfigManager(t *testing.T, upstream []config.UpstreamConfig) *config.ConfigManager {
	t.Helper()
	cfg := config.Config{GeminiUpstream: upstream}
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		t.Fatalf("序列化配置失败: %v", err)
	}
	tmpFile := filepath.Join(t.TempDir(), "config.json")
	if err := os.WriteFile(tmpFile, data, 0644); err != nil {
		t.Fatalf("写入配置文件失败: %v", err)
	}
	cm, err := config.NewConfigManager(tmpFile)
	if err != nil {
		t.Fatalf("创建配置管理器失败: %v", err)
	}
	t.Cleanup(func() { cm.Close() })
	return cm
}

func TestBuildHealthCheckURLs_UseExistingVersionSuffix(t *testing.T) {
	if got := buildModelsURL("https://api.example.com/codex/v1beta"); got != "https://api.example.com/codex/v1beta/models" {
		t.Fatalf("buildModelsURL() = %s", got)
	}
	if got := buildModelsURL("https://api.example.com/codex/v1beta#"); got != "https://api.example.com/codex/v1beta/models" {
		t.Fatalf("buildModelsURL() with marker = %s", got)
	}
}

func TestPingChannel_WithoutBaseURLReturnsError(t *testing.T) {
	cm := setupGeminiConfigManager(t, []config.UpstreamConfig{{
		Name:        "gemini-ch",
		ServiceType: "gemini",
	}})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.GET("/gemini/ping/:id", PingChannel(cm))

	req := httptest.NewRequest(http.MethodGet, "/gemini/ping/0", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK && w.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 200 or 400, body=%s", w.Code, w.Body.String())
	}

	var resp map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if got := resp["error"]; got == nil {
		t.Fatalf("error = %v, want non-nil", got)
	}
}

func TestPingAllChannels_WithoutBaseURLMarksChannelError(t *testing.T) {
	cm := setupGeminiConfigManager(t, []config.UpstreamConfig{{
		Name:        "gemini-ch",
		ServiceType: "gemini",
	}})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.GET("/gemini/ping", PingAllChannels(cm))

	req := httptest.NewRequest(http.MethodGet, "/gemini/ping", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200, body=%s", w.Code, w.Body.String())
	}

	var resp struct {
		Channels []map[string]any `json:"channels"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if len(resp.Channels) != 1 {
		t.Fatalf("len(channels) = %d, want 1", len(resp.Channels))
	}
	if got := resp.Channels[0]["error"]; got == nil {
		t.Fatalf("error = %v, want non-nil", got)
	}
}
