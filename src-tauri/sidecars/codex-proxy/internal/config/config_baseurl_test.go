package config

import (
	"os"
	"path/filepath"
	"testing"
)

// TestUpdateUpstream_BaseURLConsistency 测试更新 baseUrl 时的一致性
// 覆盖场景：
// 1. 只更新 baseUrl 时，baseUrls 应被清空
// 2. 只更新 baseUrls 时，baseUrl 应保持不变
// 3. 同时更新 baseUrl 和 baseUrls 时，两者应独立更新
// 4. 都不更新时，两者应保持原值
func TestUpdateUpstream_BaseURLConsistency(t *testing.T) {
	// 创建临时配置文件
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{
		"upstream": [{
			"name": "test-channel",
			"baseUrl": "https://old.example.com",
			"baseUrls": ["https://old-1.example.com", "https://old-2.example.com"],
			"apiKeys": ["test-key"],
			"serviceType": "claude"
		}]
	}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	// 初始化配置管理器
	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	tests := []struct {
		name            string
		updates         UpstreamUpdate
		wantBaseURL     string
		wantBaseURLs    []string
		wantBaseURLsNil bool // 期望 baseUrls 为 nil（而非空切片）
	}{
		{
			name: "只更新 baseUrl 时 baseUrls 应被清空",
			updates: UpstreamUpdate{
				BaseURL: strPtr("https://new.example.com"),
			},
			wantBaseURL:     "https://new.example.com",
			wantBaseURLsNil: true,
		},
		{
			name: "只更新 baseUrls 时 baseUrl 应保持不变",
			updates: UpstreamUpdate{
				BaseURLs: []string{"https://urls-1.example.com", "https://urls-2.example.com"},
			},
			wantBaseURL:  "https://new.example.com", // 保持上个测试的值
			wantBaseURLs: []string{"https://urls-1.example.com", "https://urls-2.example.com"},
		},
		{
			name: "同时更新 baseUrl 和 baseUrls 时两者独立更新",
			updates: UpstreamUpdate{
				BaseURL:  strPtr("https://both-base.example.com"),
				BaseURLs: []string{"https://both-urls-1.example.com", "https://both-urls-2.example.com"},
			},
			wantBaseURL:  "https://both-base.example.com",
			wantBaseURLs: []string{"https://both-urls-1.example.com", "https://both-urls-2.example.com"},
		},
		{
			name:         "都不更新时保持原值",
			updates:      UpstreamUpdate{Name: strPtr("renamed-channel")},
			wantBaseURL:  "https://both-base.example.com",
			wantBaseURLs: []string{"https://both-urls-1.example.com", "https://both-urls-2.example.com"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := cm.UpdateUpstream(0, tt.updates)
			if err != nil {
				t.Fatalf("UpdateUpstream 失败: %v", err)
			}

			cfg := cm.GetConfig()
			upstream := cfg.Upstream[0]

			if upstream.BaseURL != tt.wantBaseURL {
				t.Errorf("BaseURL = %q, want %q", upstream.BaseURL, tt.wantBaseURL)
			}

			if tt.wantBaseURLsNil {
				if upstream.BaseURLs != nil {
					t.Errorf("BaseURLs = %v, want nil", upstream.BaseURLs)
				}
			} else {
				if len(upstream.BaseURLs) != len(tt.wantBaseURLs) {
					t.Errorf("BaseURLs length = %d, want %d", len(upstream.BaseURLs), len(tt.wantBaseURLs))
				} else {
					for i, url := range upstream.BaseURLs {
						if url != tt.wantBaseURLs[i] {
							t.Errorf("BaseURLs[%d] = %q, want %q", i, url, tt.wantBaseURLs[i])
						}
					}
				}
			}
		})
	}
}

func TestAddImagesUpstream_RejectsUnsupportedServiceType(t *testing.T) {
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{"upstream":[],"imagesUpstream":[]}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	err = cm.AddImagesUpstream(UpstreamConfig{
		Name:        "images-gemini",
		ServiceType: "gemini",
		BaseURL:     "https://example.com",
		APIKeys:     []string{"test-key"},
	})
	if err == nil {
		t.Fatal("expected AddImagesUpstream to reject unsupported serviceType")
	}
	if err.Error() != "Images 渠道仅支持 openai serviceType，当前为 gemini" {
		t.Fatalf("unexpected error: %v", err)
	}
}

// TestUpdateResponsesUpstream_BaseURLConsistency 测试 Responses 渠道的 baseUrl 一致性
func TestUpdateResponsesUpstream_BaseURLConsistency(t *testing.T) {
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{
		"upstream": [],
		"responsesUpstream": [{
			"name": "responses-channel",
			"baseUrl": "https://old.responses.com",
			"baseUrls": ["https://old-1.responses.com", "https://old-2.responses.com"],
			"apiKeys": ["test-key"],
			"serviceType": "claude"
		}]
	}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	// 测试：只更新 baseUrl 时 baseUrls 应被清空
	t.Run("只更新 baseUrl 时 baseUrls 应被清空", func(t *testing.T) {
		_, err := cm.UpdateResponsesUpstream(0, UpstreamUpdate{
			BaseURL: strPtr("https://new.responses.com"),
		})
		if err != nil {
			t.Fatalf("UpdateResponsesUpstream 失败: %v", err)
		}

		cfg := cm.GetConfig()
		upstream := cfg.ResponsesUpstream[0]

		if upstream.BaseURL != "https://new.responses.com" {
			t.Errorf("BaseURL = %q, want %q", upstream.BaseURL, "https://new.responses.com")
		}
		if upstream.BaseURLs != nil {
			t.Errorf("BaseURLs = %v, want nil", upstream.BaseURLs)
		}
	})
}

func TestUpdateResponsesUpstream_ReasoningParamStyle(t *testing.T) {
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{
		"upstream": [],
		"responsesUpstream": [{
			"name": "responses-channel",
			"baseUrl": "https://responses.example.com",
			"apiKeys": ["test-key"],
			"serviceType": "responses"
		}]
	}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	style := "reasoning_effort"
	_, err = cm.UpdateResponsesUpstream(0, UpstreamUpdate{
		ReasoningParamStyle: &style,
	})
	if err != nil {
		t.Fatalf("UpdateResponsesUpstream 失败: %v", err)
	}

	cfg := cm.GetConfig()
	if got := cfg.ResponsesUpstream[0].ReasoningParamStyle; got != style {
		t.Fatalf("ReasoningParamStyle = %q, want %q", got, style)
	}
}

// TestUpdateGeminiUpstream_BaseURLConsistency 测试 Gemini 渠道的 baseUrl 一致性
func TestUpdateGeminiUpstream_BaseURLConsistency(t *testing.T) {
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{
		"upstream": [],
		"geminiUpstream": [{
			"name": "gemini-channel",
			"baseUrl": "https://old.gemini.com",
			"baseUrls": ["https://old-1.gemini.com", "https://old-2.gemini.com"],
			"apiKeys": ["test-key"],
			"serviceType": "gemini"
		}]
	}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	// 测试：只更新 baseUrl 时 baseUrls 应被清空
	t.Run("只更新 baseUrl 时 baseUrls 应被清空", func(t *testing.T) {
		_, err := cm.UpdateGeminiUpstream(0, UpstreamUpdate{
			BaseURL: strPtr("https://new.gemini.com"),
		})
		if err != nil {
			t.Fatalf("UpdateGeminiUpstream 失败: %v", err)
		}

		cfg := cm.GetConfig()
		upstream := cfg.GeminiUpstream[0]

		if upstream.BaseURL != "https://new.gemini.com" {
			t.Errorf("BaseURL = %q, want %q", upstream.BaseURL, "https://new.gemini.com")
		}
		if upstream.BaseURLs != nil {
			t.Errorf("BaseURLs = %v, want nil", upstream.BaseURLs)
		}
	})
}

// TestGetAllBaseURLs_Priority 测试 GetAllBaseURLs 的优先级逻辑
func TestGetAllBaseURLs_Priority(t *testing.T) {
	tests := []struct {
		name     string
		upstream UpstreamConfig
		want     []string
	}{
		{
			name: "baseUrls 非空时优先返回 baseUrls",
			upstream: UpstreamConfig{
				BaseURL:  "https://single.example.com",
				BaseURLs: []string{"https://multi-1.example.com", "https://multi-2.example.com"},
			},
			want: []string{"https://multi-1.example.com", "https://multi-2.example.com"},
		},
		{
			name: "baseUrls 为空时回退到 baseUrl",
			upstream: UpstreamConfig{
				BaseURL:  "https://single.example.com",
				BaseURLs: nil,
			},
			want: []string{"https://single.example.com"},
		},
		{
			name: "两者都为空时返回 nil",
			upstream: UpstreamConfig{
				BaseURL:  "",
				BaseURLs: nil,
			},
			want: nil,
		},
		{
			name: "baseUrls 为空切片时回退到 baseUrl",
			upstream: UpstreamConfig{
				BaseURL:  "https://single.example.com",
				BaseURLs: []string{},
			},
			want: []string{"https://single.example.com"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := tt.upstream.GetAllBaseURLs()

			if len(got) != len(tt.want) {
				t.Errorf("GetAllBaseURLs() length = %d, want %d", len(got), len(tt.want))
				return
			}
			for i := range got {
				if got[i] != tt.want[i] {
					t.Errorf("GetAllBaseURLs()[%d] = %q, want %q", i, got[i], tt.want[i])
				}
			}
		})
	}
}

// TestGetEffectiveBaseURL_Priority 测试 GetEffectiveBaseURL 的优先级逻辑
func TestGetEffectiveBaseURL_Priority(t *testing.T) {
	tests := []struct {
		name     string
		upstream UpstreamConfig
		want     string
	}{
		{
			name: "baseUrl 非空时优先返回 baseUrl",
			upstream: UpstreamConfig{
				BaseURL:  "https://single.example.com",
				BaseURLs: []string{"https://multi-1.example.com", "https://multi-2.example.com"},
			},
			want: "https://single.example.com",
		},
		{
			name: "baseUrl 为空时回退到 baseUrls[0]",
			upstream: UpstreamConfig{
				BaseURL:  "",
				BaseURLs: []string{"https://multi-1.example.com", "https://multi-2.example.com"},
			},
			want: "https://multi-1.example.com",
		},
		{
			name: "两者都为空时返回空字符串",
			upstream: UpstreamConfig{
				BaseURL:  "",
				BaseURLs: nil,
			},
			want: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := tt.upstream.GetEffectiveBaseURL()
			if got != tt.want {
				t.Errorf("GetEffectiveBaseURL() = %q, want %q", got, tt.want)
			}
		})
	}
}

// TestDeduplicateBaseURLs 测试 BaseURLs 去重逻辑
func TestDeduplicateBaseURLs(t *testing.T) {
	tests := []struct {
		name        string
		serviceType string
		input       []string
		want        []string
	}{
		{
			name:        "精确重复应去重",
			serviceType: "openai",
			input:       []string{"https://a.com", "https://b.com", "https://a.com"},
			want:        []string{"https://a.com", "https://b.com"},
		},
		{
			name:        "末尾斜杠差异应视为相同",
			serviceType: "openai",
			input:       []string{"https://a.com", "https://a.com/"},
			want:        []string{"https://a.com"},
		},
		{
			name:        "末尾井号差异应保留独立语义",
			serviceType: "openai",
			input:       []string{"https://a.com", "https://a.com#"},
			want:        []string{"https://a.com", "https://a.com#"},
		},
		{
			name:        "根域名与 /v1 应视为相同",
			serviceType: "openai",
			input:       []string{"https://a.com", "https://a.com/v1"},
			want:        []string{"https://a.com"},
		},
		{
			name:        "Gemini 根域名与 /v1beta 应视为相同",
			serviceType: "gemini",
			input:       []string{"https://gemini.example.com", "https://gemini.example.com/v1beta"},
			want:        []string{"https://gemini.example.com"},
		},
		{
			name:        "多个不同 URL 保持原始顺序",
			serviceType: "openai",
			input:       []string{"https://c.com", "https://a.com", "https://b.com"},
			want:        []string{"https://c.com", "https://a.com", "https://b.com"},
		},
		{
			name:        "单个元素不变",
			serviceType: "openai",
			input:       []string{"https://only.com"},
			want:        []string{"https://only.com"},
		},
		{
			name:        "单个元素也执行 canonical 规范化",
			serviceType: "openai",
			input:       []string{"https://only.com/v1"},
			want:        []string{"https://only.com"},
		},
		{
			name:        "空切片返回空切片",
			serviceType: "openai",
			input:       []string{},
			want:        []string{},
		},
		{
			name:        "nil 返回 nil",
			serviceType: "openai",
			input:       nil,
			want:        nil,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := deduplicateBaseURLs(tt.input, tt.serviceType)

			if len(got) != len(tt.want) {
				t.Errorf("deduplicateBaseURLs() length = %d, want %d", len(got), len(tt.want))
				return
			}
			for i := range got {
				if got[i] != tt.want[i] {
					t.Errorf("deduplicateBaseURLs()[%d] = %q, want %q", i, got[i], tt.want[i])
				}
			}
		})
	}
}

// TestAddUpstream_BaseURLDeduplication 测试添加渠道时的 BaseURLs 去重
func TestAddUpstream_BaseURLDeduplication(t *testing.T) {
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{"upstream": []}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	// 添加包含重复 URL 的渠道
	err = cm.AddUpstream(UpstreamConfig{
		Name:        "dedup-test",
		BaseURL:     "https://main.example.com",
		BaseURLs:    []string{"https://a.com", "https://b.com", "https://a.com/", "https://c.com"},
		APIKeys:     []string{"key1"},
		ServiceType: "claude",
	})
	if err != nil {
		t.Fatalf("AddUpstream 失败: %v", err)
	}

	cfg := cm.GetConfig()
	upstream := cfg.Upstream[0]

	// 期望去重后只有 3 个 URL
	expectedURLs := []string{"https://a.com", "https://b.com", "https://c.com"}
	if len(upstream.BaseURLs) != len(expectedURLs) {
		t.Errorf("BaseURLs length = %d, want %d", len(upstream.BaseURLs), len(expectedURLs))
	}
	for i, url := range upstream.BaseURLs {
		if url != expectedURLs[i] {
			t.Errorf("BaseURLs[%d] = %q, want %q", i, url, expectedURLs[i])
		}
	}
}

func TestLoadConfig_BackfillsLegacyServiceTypeDefaults(t *testing.T) {
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{
		"upstream": [{
			"name": "messages-legacy",
			"baseUrl": "https://messages.example.com/v1",
			"apiKeys": ["sk-m"]
		}],
		"responsesUpstream": [{
			"name": "responses-legacy",
			"baseUrl": "https://responses.example.com/v1",
			"apiKeys": ["sk-r"]
		}],
		"geminiUpstream": [{
			"name": "gemini-legacy",
			"baseUrl": "https://generativelanguage.googleapis.com/v1beta",
			"baseUrls": [
				"https://generativelanguage.googleapis.com/v1beta",
				"https://generativelanguage.googleapis.com"
			],
			"apiKeys": ["sk-g"]
		}],
		"chatUpstream": [{
			"name": "chat-legacy",
			"baseUrl": "https://chat.example.com/v1",
			"apiKeys": ["sk-c"]
		}]
	}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	cfg := cm.GetConfig()
	if got := cfg.Upstream[0].ServiceType; got != "claude" {
		t.Fatalf("messages ServiceType = %q, want claude", got)
	}
	if got := cfg.ResponsesUpstream[0].ServiceType; got != "responses" {
		t.Fatalf("responses ServiceType = %q, want responses", got)
	}
	if got := cfg.GeminiUpstream[0].ServiceType; got != "gemini" {
		t.Fatalf("gemini ServiceType = %q, want gemini", got)
	}
	if got := cfg.ChatUpstream[0].ServiceType; got != "openai" {
		t.Fatalf("chat ServiceType = %q, want openai", got)
	}

	// 回填后 Gemini 的 root 与 /v1beta 应按等价规则折叠为最短形式。
	if urls := cfg.GeminiUpstream[0].GetAllBaseURLs(); len(urls) != 1 || urls[0] != "https://generativelanguage.googleapis.com" {
		t.Fatalf("Gemini GetAllBaseURLs() = %#v, want [https://generativelanguage.googleapis.com]", urls)
	}
}

func TestUpdateChatUpstreamCanSetNormalizeNonstandardChatRoles(t *testing.T) {
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{
		"chatUpstream": [{
			"name": "chat-channel",
			"baseUrl": "https://chat.example.com/v1",
			"apiKeys": ["sk-c"],
			"serviceType": "openai"
		}]
	}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	cfg := cm.GetConfig()
	if cfg.ChatUpstream[0].NormalizeNonstandardChatRoles {
		t.Fatal("NormalizeNonstandardChatRoles = true, want default false")
	}

	enabled := true
	if _, err := cm.UpdateChatUpstream(0, UpstreamUpdate{NormalizeNonstandardChatRoles: &enabled}); err != nil {
		t.Fatalf("UpdateChatUpstream(enable) 失败: %v", err)
	}
	cfg = cm.GetConfig()
	if !cfg.ChatUpstream[0].NormalizeNonstandardChatRoles {
		t.Fatal("NormalizeNonstandardChatRoles = false, want true")
	}

	disabled := false
	if _, err := cm.UpdateChatUpstream(0, UpstreamUpdate{NormalizeNonstandardChatRoles: &disabled}); err != nil {
		t.Fatalf("UpdateChatUpstream(disable) 失败: %v", err)
	}
	cfg = cm.GetConfig()
	if cfg.ChatUpstream[0].NormalizeNonstandardChatRoles {
		t.Fatal("NormalizeNonstandardChatRoles = true, want false")
	}
}

// strPtr 辅助函数：返回字符串指针
func strPtr(s string) *string {
	return &s
}

func TestUpdateGeminiUpstream_AdvancedOptions(t *testing.T) {
	tempDir := t.TempDir()
	configPath := filepath.Join(tempDir, "config.json")
	initialConfig := `{
		"upstream": [],
		"geminiUpstream": [{
			"name": "gemini-channel",
			"baseUrl": "https://old.gemini.com",
			"apiKeys": ["test-key"],
			"serviceType": "gemini",
			"reasoningMapping": {"gemini-2.5-pro": "low"},
			"textVerbosity": "low",
			"fastMode": false
		}]
	}`
	if err := os.WriteFile(configPath, []byte(initialConfig), 0644); err != nil {
		t.Fatalf("写入初始配置失败: %v", err)
	}

	cm, err := NewConfigManager(configPath)
	if err != nil {
		t.Fatalf("初始化配置管理器失败: %v", err)
	}
	defer cm.Close()

	_, err = cm.UpdateGeminiUpstream(0, UpstreamUpdate{
		ReasoningMapping: map[string]string{"gemini-2.5-pro": "high"},
		TextVerbosity:    strPtr("medium"),
		FastMode:         boolPtr(true),
	})
	if err != nil {
		t.Fatalf("UpdateGeminiUpstream 失败: %v", err)
	}

	cfg := cm.GetConfig()
	upstream := cfg.GeminiUpstream[0]

	if got := upstream.ReasoningMapping["gemini-2.5-pro"]; got != "high" {
		t.Fatalf("ReasoningMapping[gemini-2.5-pro] = %q, want high", got)
	}
	if upstream.TextVerbosity != "medium" {
		t.Fatalf("TextVerbosity = %q, want medium", upstream.TextVerbosity)
	}
	if !upstream.FastMode {
		t.Fatal("FastMode = false, want true")
	}
}

func boolPtr(v bool) *bool {
	return &v
}
