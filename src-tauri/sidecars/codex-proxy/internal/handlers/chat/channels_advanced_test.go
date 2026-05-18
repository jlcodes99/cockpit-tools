package chat

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

func setupChatConfigManager(t *testing.T, upstream []config.UpstreamConfig) *config.ConfigManager {
	t.Helper()
	cfg := config.Config{ChatUpstream: upstream}
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
	if got := buildModelsURL("https://api.example.com/codex/v1#"); got != "https://api.example.com/codex/v1/models" {
		t.Fatalf("buildModelsURL() with marker = %s", got)
	}
}

func TestGetUpstreams_IncludesUnifiedStateFields(t *testing.T) {
	cm := setupChatConfigManager(t, []config.UpstreamConfig{{
		Name:                          "chat-ch",
		ServiceType:                   "openai",
		BaseURL:                       "https://api.example.com",
		APIKeys:                       []string{"sk-1"},
		Status:                        "suspended",
		NormalizeNonstandardChatRoles: true,
		DisabledAPIKeys: []config.DisabledKeyInfo{{
			Key:        "sk-disabled",
			Reason:     "insufficient_balance",
			Message:    "no balance",
			DisabledAt: "2026-04-11T00:00:00Z",
		}},
	}})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.GET("/chat/channels", GetUpstreams(cm))

	req := httptest.NewRequest(http.MethodGet, "/chat/channels", nil)
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
	if got := resp.Channels[0]["normalizeNonstandardChatRoles"]; got != true {
		t.Fatalf("normalizeNonstandardChatRoles = %v, want true", got)
	}
}

func TestPingChannel_WithoutBaseURLReturnsError(t *testing.T) {
	cm := setupChatConfigManager(t, []config.UpstreamConfig{{
		Name:        "chat-ch",
		ServiceType: "openai",
	}})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.GET("/chat/ping/:id", PingChannel(cm))

	req := httptest.NewRequest(http.MethodGet, "/chat/ping/0", nil)
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
	cm := setupChatConfigManager(t, []config.UpstreamConfig{{
		Name:        "chat-ch",
		ServiceType: "openai",
	}})

	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.GET("/chat/ping", PingAllChannels(cm))

	req := httptest.NewRequest(http.MethodGet, "/chat/ping", nil)
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
