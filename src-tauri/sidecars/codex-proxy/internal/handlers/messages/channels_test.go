package messages

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

func setupTestConfigManager(t *testing.T, upstream []config.UpstreamConfig) *config.ConfigManager {
	t.Helper()
	cfg := config.Config{Upstream: upstream}
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

func newModelsRouter(cfgManager *config.ConfigManager) *gin.Engine {
	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.POST("/messages/channels/:id/models", GetChannelModels(cfgManager))
	return r
}

func postModels(t *testing.T, router *gin.Engine, id string, body GetModelsRequest) *httptest.ResponseRecorder {
	t.Helper()
	raw, _ := json.Marshal(body)
	req := httptest.NewRequest(http.MethodPost, "/messages/channels/"+id+"/models", bytes.NewReader(raw))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)
	return w
}

// TestGetChannelModels_InvalidID 非法路径参数返回 400
func TestGetChannelModels_InvalidID(t *testing.T) {
	cm := setupTestConfigManager(t, nil)
	r := newModelsRouter(cm)

	w := postModels(t, r, "abc", GetModelsRequest{Key: "sk-test"})
	if w.Code != http.StatusBadRequest {
		t.Errorf("期望 400，实际 %d", w.Code)
	}
}

// TestGetChannelModels_MissingKey key 为空时返回 400
func TestGetChannelModels_MissingKey(t *testing.T) {
	upstream := []config.UpstreamConfig{{Name: "ch0", BaseURL: "http://example.com", APIKeys: []string{"sk-saved"}}}
	cm := setupTestConfigManager(t, upstream)
	r := newModelsRouter(cm)

	w := postModels(t, r, "0", GetModelsRequest{Key: ""})
	if w.Code != http.StatusBadRequest {
		t.Errorf("期望 400，实际 %d", w.Code)
	}
}

// TestGetChannelModels_ChannelNotFound ID 超出范围返回 404
func TestGetChannelModels_ChannelNotFound(t *testing.T) {
	cm := setupTestConfigManager(t, nil) // 没有任何 upstream
	r := newModelsRouter(cm)

	w := postModels(t, r, "0", GetModelsRequest{Key: "sk-test"})
	if w.Code != http.StatusNotFound {
		t.Errorf("期望 404，实际 %d", w.Code)
	}
}

// TestGetChannelModels_UpstreamReturns200 上游返回 200 时透传响应
func TestGetChannelModels_UpstreamReturns200(t *testing.T) {
	// 启动 mock 上游
	mockResp := `{"object":"list","data":[{"id":"gpt-4","object":"model"}]}`
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(mockResp))
	}))
	defer upstream.Close()

	cfgUpstream := []config.UpstreamConfig{{
		Name:    "test-ch",
		BaseURL: upstream.URL,
		APIKeys: []string{"sk-valid"},
	}}
	cm := setupTestConfigManager(t, cfgUpstream)
	r := newModelsRouter(cm)

	w := postModels(t, r, "0", GetModelsRequest{Key: "sk-valid"})
	if w.Code != http.StatusOK {
		t.Errorf("期望 200，实际 %d，body: %s", w.Code, w.Body.String())
	}
	if w.Body.String() != mockResp {
		t.Errorf("响应体不匹配，实际: %s", w.Body.String())
	}
}

// TestGetChannelModels_UpstreamReturns401 上游返回 401 时，包装为 400 避免前端误判为管理 API 认证失败
func TestGetChannelModels_UpstreamReturns401(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = w.Write([]byte(`{"error":"invalid key"}`))
	}))
	defer upstream.Close()

	cfgUpstream := []config.UpstreamConfig{{
		Name:    "test-ch",
		BaseURL: upstream.URL,
		APIKeys: []string{"sk-bad"},
	}}
	cm := setupTestConfigManager(t, cfgUpstream)
	r := newModelsRouter(cm)

	w := postModels(t, r, "0", GetModelsRequest{Key: "sk-bad"})
	if w.Code != http.StatusBadRequest {
		t.Errorf("期望 400（上游 401 包装），实际 %d", w.Code)
	}
}

// TestGetChannelModels_TempBaseURL 新增模式：使用请求体中的临时 baseUrl
func TestGetChannelModels_TempBaseURL(t *testing.T) {
	mockResp := `{"object":"list","data":[{"id":"claude-3","object":"model"}]}`
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer sk-temp" {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(mockResp))
	}))
	defer upstream.Close()

	// 配置中没有任何 upstream；新增模式通过 baseUrl 参数传入
	cm := setupTestConfigManager(t, nil)
	r := newModelsRouter(cm)

	w := postModels(t, r, "0", GetModelsRequest{
		Key:     "sk-temp",
		BaseURL: upstream.URL,
	})
	if w.Code != http.StatusOK {
		t.Errorf("期望 200，实际 %d，body: %s", w.Code, w.Body.String())
	}
	if w.Body.String() != mockResp {
		t.Errorf("响应体不匹配，实际: %s", w.Body.String())
	}
}

// TestGetChannelModels_InvalidBody 请求体非 JSON 时返回 400
func TestGetChannelModels_InvalidBody(t *testing.T) {
	cm := setupTestConfigManager(t, nil)
	gin.SetMode(gin.TestMode)
	r := gin.New()
	r.POST("/messages/channels/:id/models", GetChannelModels(cm))

	req := httptest.NewRequest(http.MethodPost, "/messages/channels/0/models",
		bytes.NewBufferString("not-json"))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	if w.Code != http.StatusBadRequest {
		t.Errorf("期望 400，实际 %d", w.Code)
	}
}
