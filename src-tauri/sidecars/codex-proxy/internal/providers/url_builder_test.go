package providers

import (
	"regexp"
	"strings"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

// buildOpenAIURL 模拟 openai.go 中的 URL 构建逻辑
func buildOpenAIURL(baseURL string) string {
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	if skipVersionPrefix {
		baseURL = strings.TrimSuffix(baseURL, "#")
	}
	baseURL = strings.TrimSuffix(baseURL, "/")

	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
	hasVersionSuffix := versionPattern.MatchString(baseURL)

	endpoint := "/chat/completions"
	if !hasVersionSuffix && !skipVersionPrefix {
		endpoint = "/v1" + endpoint
	}
	return baseURL + endpoint
}

// buildClaudeURL 模拟 claude.go 中的 URL 构建逻辑
func buildClaudeURL(baseURL, requestPath string) string {
	endpoint := strings.TrimPrefix(requestPath, "/v1")
	skipVersionPrefix := strings.HasSuffix(baseURL, "#")
	if skipVersionPrefix {
		baseURL = strings.TrimSuffix(baseURL, "#")
	}
	baseURL = strings.TrimSuffix(baseURL, "/")

	versionPattern := regexp.MustCompile(`/v\d+[a-z]*$`)
	if versionPattern.MatchString(baseURL) || skipVersionPrefix {
		return baseURL + endpoint
	}
	return baseURL + "/v1" + endpoint
}

func TestOpenAIURL_SkipVersionWithHash(t *testing.T) {
	tests := []struct {
		name    string
		baseURL string
		want    string
	}{
		{"normal", "https://api.openai.com", "https://api.openai.com/v1/chat/completions"},
		{"with_v1", "https://api.openai.com/v1", "https://api.openai.com/v1/chat/completions"},
		{"hash_skip", "https://api.example.com#", "https://api.example.com/chat/completions"},
		{"hash_with_slash", "https://api.example.com/#", "https://api.example.com/chat/completions"},
		{"hash_with_path", "https://core.blink.new/api/v1/ai#", "https://core.blink.new/api/v1/ai/chat/completions"},
		{"trailing_slash", "https://api.example.com/", "https://api.example.com/v1/chat/completions"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := buildOpenAIURL(tt.baseURL)
			if got != tt.want {
				t.Errorf("buildOpenAIURL(%q) = %q, want %q", tt.baseURL, got, tt.want)
			}
		})
	}
}

func TestClaudeURL_SkipVersionWithHash(t *testing.T) {
	tests := []struct {
		name        string
		baseURL     string
		requestPath string
		want        string
	}{
		{"normal", "https://api.anthropic.com", "/v1/messages", "https://api.anthropic.com/v1/messages"},
		{"with_v1", "https://api.anthropic.com/v1", "/v1/messages", "https://api.anthropic.com/v1/messages"},
		{"hash_skip", "https://api.example.com#", "/v1/messages", "https://api.example.com/messages"},
		{"hash_with_slash", "https://api.example.com/#", "/v1/messages", "https://api.example.com/messages"},
		{"trailing_slash", "https://api.example.com/", "/v1/messages", "https://api.example.com/v1/messages"},
		{"count_tokens", "https://api.example.com#", "/v1/messages/count_tokens", "https://api.example.com/messages/count_tokens"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := buildClaudeURL(tt.baseURL, tt.requestPath)
			if got != tt.want {
				t.Errorf("buildClaudeURL(%q, %q) = %q, want %q", tt.baseURL, tt.requestPath, got, tt.want)
			}
		})
	}
}

func TestBuildTargetURL_SkipVersionWithHash(t *testing.T) {
	p := &ResponsesProvider{}

	tests := []struct {
		name        string
		baseURL     string
		serviceType string
		want        string
	}{
		// 正常情况：自动添加 /v1
		{"normal_responses", "https://api.example.com", "responses", "https://api.example.com/v1/responses"},
		{"normal_claude", "https://api.example.com", "claude", "https://api.example.com/v1/messages"},
		{"normal_openai", "https://api.example.com", "openai", "https://api.example.com/v1/chat/completions"},

		// 已有版本号：不添加 /v1
		{"with_version", "https://api.example.com/v1", "responses", "https://api.example.com/v1/responses"},
		{"with_v2", "https://api.example.com/v2", "openai", "https://api.example.com/v2/chat/completions"},

		// # 结尾：跳过 /v1
		{"hash_skip", "https://api.example.com#", "responses", "https://api.example.com/responses"},
		{"hash_skip_claude", "https://api.example.com#", "claude", "https://api.example.com/messages"},
		{"hash_skip_openai", "https://api.example.com#", "openai", "https://api.example.com/chat/completions"},
		{"hash_with_path_openai", "https://core.blink.new/api/v1/ai#", "openai", "https://core.blink.new/api/v1/ai/chat/completions"},

		// # 结尾 + 末尾斜杠：正确处理
		{"hash_with_slash", "https://api.example.com/#", "responses", "https://api.example.com/responses"},
		{"hash_with_slash_openai", "https://api.example.com/#", "openai", "https://api.example.com/chat/completions"},

		// 末尾斜杠：正确移除
		{"trailing_slash", "https://api.example.com/", "responses", "https://api.example.com/v1/responses"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			upstream := &config.UpstreamConfig{
				BaseURL:     tt.baseURL,
				ServiceType: tt.serviceType,
			}
			got := p.buildTargetURL(upstream)
			if got != tt.want {
				t.Errorf("buildTargetURL() = %q, want %q", got, tt.want)
			}
		})
	}
}
