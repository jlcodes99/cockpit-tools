package config

import "testing"

func TestSupportsModel(t *testing.T) {
	tests := []struct {
		name            string
		supportedModels []string
		model           string
		want            bool
	}{
		{"空列表匹配所有", nil, "gpt-4o", true},
		{"空列表匹配空模型", nil, "", true},
		{"精确匹配", []string{"gpt-4o"}, "gpt-4o", true},
		{"精确不匹配", []string{"gpt-4o"}, "gpt-4-turbo", false},
		{"前缀匹配", []string{"gpt-4*"}, "gpt-4o", true},
		{"后缀匹配", []string{"*image"}, "gpt-image", true},
		{"包含匹配", []string{"*image*"}, "gpt-4-image-preview", true},
		{"通配符不匹配", []string{"gpt-4*"}, "o3", false},
		{"多模式匹配第二个", []string{"gpt-4*", "claude-*"}, "claude-3-opus", true},
		{"精确和通配符混合", []string{"o3", "gpt-4*"}, "o3", true},
		{"通配符星号本身", []string{"*"}, "anything", true},
		{"精确排除命中", []string{"gpt-4*", "!gpt-4-image-preview"}, "gpt-4-image-preview", false},
		{"包含排除命中", []string{"gpt-4*", "!*image*"}, "gpt-4-image-preview", false},
		{"后缀排除命中", []string{"*", "!*image"}, "gpt-image", false},
		{"仅排除且未命中时放行", []string{"!*image*"}, "gpt-4o", true},
		{"排除优先于包含", []string{"*image*", "!*image*"}, "gpt-image", false},
		{"非法中间通配被跳过且不影响合法规则", []string{"foo*bar", "gpt-4*"}, "gpt-4o", true},
		{"仅非法中间通配时等价于无有效规则", []string{"foo*bar"}, "foobar", true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			u := &UpstreamConfig{SupportedModels: tt.supportedModels}
			if got := u.SupportsModel(tt.model); got != tt.want {
				t.Errorf("SupportsModel(%q) = %v, want %v", tt.model, got, tt.want)
			}
		})
	}
}

func TestExplainModelSupport(t *testing.T) {
	tests := []struct {
		name            string
		supportedModels []string
		model           string
		wantSupported   bool
		wantReason      string
	}{
		{"空列表匹配所有", nil, "gpt-5.5", true, ""},
		{"命中排除规则", []string{"*", "!gpt-5.5"}, "gpt-5.5", false, "命中排除规则 !gpt-5.5"},
		{"未命中包含规则", []string{"claude-*"}, "gpt-5.5", false, "未命中包含规则"},
		{"命中包含规则", []string{"gpt-*"}, "gpt-5.5", true, ""},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			u := &UpstreamConfig{SupportedModels: tt.supportedModels}
			gotSupported, gotReason := u.ExplainModelSupport(tt.model)
			if gotSupported != tt.wantSupported || gotReason != tt.wantReason {
				t.Fatalf("ExplainModelSupport(%q) = (%v, %q), want (%v, %q)", tt.model, gotSupported, gotReason, tt.wantSupported, tt.wantReason)
			}
		})
	}
}

func TestIsValidSupportedModelPattern(t *testing.T) {
	tests := []struct {
		name    string
		pattern string
		want    bool
	}{
		{"精确匹配合法", "gpt-4o", true},
		{"前缀匹配合法", "gpt-4*", true},
		{"后缀匹配合法", "*image", true},
		{"包含匹配合法", "*image*", true},
		{"全通配合法", "*", true},
		{"空字符串非法", "", false},
		{"仅空白非法", "   ", false},
		{"空 contains 非法", "**", false},
		{"多重排除前缀非法", "!!gpt-4*", false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := isValidSupportedModelPattern(tt.pattern); got != tt.want {
				t.Errorf("isValidSupportedModelPattern(%q) = %v, want %v", tt.pattern, got, tt.want)
			}
		})
	}
}

func TestResolveReasoningEffort(t *testing.T) {
	upstream := &UpstreamConfig{
		ReasoningMapping: map[string]string{
			"gpt-5":         "high",
			"gpt-5.1-codex": "xhigh",
			"o3":            "medium",
		},
	}

	tests := []struct {
		name  string
		model string
		want  string
	}{
		{"精确匹配", "o3", "medium"},
		{"最长匹配优先", "gpt-5.1-codex", "xhigh"},
		{"模糊匹配回退", "gpt-5.1", "high"},
		{"未匹配返回空", "claude-3-7-sonnet", ""},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := ResolveReasoningEffort(tt.model, upstream); got != tt.want {
				t.Fatalf("ResolveReasoningEffort(%q) = %q, want %q", tt.model, got, tt.want)
			}
		})
	}
}
