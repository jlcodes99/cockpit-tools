package responses

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

func setupResponsesConfigManager(t *testing.T, upstream []config.UpstreamConfig) *config.ConfigManager {
	t.Helper()
	cfg := config.Config{ResponsesUpstream: upstream}
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
	if got := buildMessagesURL("https://api.example.com/codex/v1"); got != "https://api.example.com/codex/v1/messages" {
		t.Fatalf("buildMessagesURL() = %s", got)
	}
	if got := buildModelsURL("https://api.example.com/codex/v1"); got != "https://api.example.com/codex/v1/models" {
		t.Fatalf("buildModelsURL() = %s", got)
	}
	if got := buildGeminiModelsURL("https://api.example.com/codex/v1beta"); got != "https://api.example.com/codex/v1beta/models" {
		t.Fatalf("buildGeminiModelsURL() = %s", got)
	}
	if got := buildModelsURL("https://api.example.com/codex/v1#"); got != "https://api.example.com/codex/v1/models" {
		t.Fatalf("buildModelsURL() with marker = %s", got)
	}
}

func TestGetUpstreams_IncludesUnifiedStateFields(t *testing.T) {
	cm := setupResponsesConfigManager(t, []config.UpstreamConfig{{
		Name:        "resp-ch",
		ServiceType: "responses",
		BaseURL:     "https://api.example.com",
		APIKeys:     []string{"sk-1"},
		Status:      "suspended",
		DisabledAPIKeys: []config.DisabledKeyInfo{{
			Key:        "sk-disabled",
			Reason:     "insufficient_balance",
			Message:    "no balance",
			DisabledAt: "2026-04-11T00:00:00Z",
		}},
	}})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.GET("/responses/channels", GetUpstreams(cm))

	req := httptest.NewRequest(http.MethodGet, "/responses/channels", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200", w.Code)
	}

	var resp struct {
		Channels []map[string]interface{} `json:"channels"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if len(resp.Channels) != 1 {
		t.Fatalf("len(channels) = %d, want 1", len(resp.Channels))
	}
	if got := resp.Channels[0]["status"]; got != "suspended" {
		t.Fatalf("status = %v, want suspended", got)
	}
	if got := resp.Channels[0]["adminState"]; got != "suspended" {
		t.Fatalf("adminState = %v, want suspended", got)
	}
	if got := resp.Channels[0]["effectiveState"]; got != "suspended" {
		t.Fatalf("effectiveState = %v, want suspended", got)
	}
	if got := resp.Channels[0]["runtimeState"]; got != "disabled_keys_present" {
		t.Fatalf("runtimeState = %v, want disabled_keys_present", got)
	}
	disabledKeys, ok := resp.Channels[0]["disabledApiKeys"].([]interface{})
	if !ok || len(disabledKeys) != 1 {
		t.Fatalf("disabledApiKeys = %#v, want len 1", resp.Channels[0]["disabledApiKeys"])
	}
}

func TestPingChannel_WithoutBaseURLReturnsError(t *testing.T) {
	cm := setupResponsesConfigManager(t, []config.UpstreamConfig{{
		Name:        "resp-ch",
		ServiceType: "responses",
	}})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.GET("/responses/ping/:id", PingChannel(cm))

	req := httptest.NewRequest(http.MethodGet, "/responses/ping/0", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200, body=%s", w.Code, w.Body.String())
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
	cm := setupResponsesConfigManager(t, []config.UpstreamConfig{{
		Name:        "resp-ch",
		ServiceType: "responses",
	}})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.GET("/responses/ping", PingAllChannels(cm))

	req := httptest.NewRequest(http.MethodGet, "/responses/ping", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200, body=%s", w.Code, w.Body.String())
	}

	var resp []map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("unmarshal response: %v", err)
	}
	if len(resp) != 1 {
		t.Fatalf("len(results) = %d, want 1", len(resp))
	}
	if got := resp[0]["error"]; got == nil {
		t.Fatalf("error = %v, want non-nil", got)
	}
}
