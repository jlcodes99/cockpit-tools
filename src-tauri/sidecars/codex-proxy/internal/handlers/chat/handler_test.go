package chat

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
)

func TestBuildProviderRequest_InjectsReasoningBeforeModelRedirect(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil).WithContext(context.Background())

	bodyBytes := []byte(`{"model":"gpt-5.1-codex","messages":[{"role":"user","content":"hi"}]}`)
	upstream := &config.UpstreamConfig{
		ServiceType: "openai",
		ModelMapping: map[string]string{
			"gpt-5.1-codex": "gpt-5.2-codex",
		},
		ReasoningMapping: map[string]string{
			"gpt-5.1-codex": "xhigh",
		},
		TextVerbosity: "low",
		FastMode:      true,
	}

	req, err := buildProviderRequest(c, upstream, "https://api.example.com", "sk-test", bodyBytes, "gpt-5.1-codex", false)
	if err != nil {
		t.Fatalf("buildProviderRequest() err = %v", err)
	}

	var got map[string]interface{}
	if err := json.NewDecoder(req.Body).Decode(&got); err != nil {
		t.Fatalf("decode request body: %v", err)
	}

	if got["model"] != "gpt-5.2-codex" {
		t.Fatalf("model = %v, want gpt-5.2-codex", got["model"])
	}

	reasoning, ok := got["reasoning"].(map[string]interface{})
	if !ok || reasoning["effort"] != "xhigh" {
		t.Fatalf("reasoning = %#v, want effort=xhigh", got["reasoning"])
	}

	text, ok := got["text"].(map[string]interface{})
	if !ok || text["verbosity"] != "low" {
		t.Fatalf("text = %#v, want verbosity=low", got["text"])
	}

	if got["service_tier"] != "priority" {
		t.Fatalf("service_tier = %v, want priority", got["service_tier"])
	}
}

func TestBuildProviderRequest_InjectsReasoningEffortStyle(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil).WithContext(context.Background())

	bodyBytes := []byte(`{"model":"gpt-5.1-codex","messages":[{"role":"user","content":"hi"}]}`)
	upstream := &config.UpstreamConfig{
		ServiceType:         "openai",
		ReasoningParamStyle: "reasoning_effort",
		ReasoningMapping: map[string]string{
			"gpt-5.1-codex": "xhigh",
		},
	}

	req, err := buildProviderRequest(c, upstream, "https://api.example.com", "sk-test", bodyBytes, "gpt-5.1-codex", false)
	if err != nil {
		t.Fatalf("buildProviderRequest() err = %v", err)
	}

	var got map[string]interface{}
	if err := json.NewDecoder(req.Body).Decode(&got); err != nil {
		t.Fatalf("decode request body: %v", err)
	}

	if got["reasoning_effort"] != "xhigh" {
		t.Fatalf("reasoning_effort = %v, want xhigh", got["reasoning_effort"])
	}
	if _, ok := got["reasoning"]; ok {
		t.Fatalf("reasoning should not be set when reasoningParamStyle=reasoning_effort: %#v", got["reasoning"])
	}
}

func TestBuildProviderRequest_NormalizeNonstandardChatRolesDefaultOff(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil).WithContext(context.Background())

	bodyBytes := []byte(`{"model":"gpt-5","messages":[{"role":"developer","content":"dev"},{"role":"user","content":"hi"}]}`)
	upstream := &config.UpstreamConfig{ServiceType: "openai"}

	req, err := buildProviderRequest(c, upstream, "https://api.example.com", "sk-test", bodyBytes, "gpt-5", false)
	if err != nil {
		t.Fatalf("buildProviderRequest() err = %v", err)
	}

	var got map[string]interface{}
	if err := json.NewDecoder(req.Body).Decode(&got); err != nil {
		t.Fatalf("decode request body: %v", err)
	}

	messages := got["messages"].([]interface{})
	first := messages[0].(map[string]interface{})
	if first["role"] != "developer" {
		t.Fatalf("role = %v, want developer when switch is off", first["role"])
	}
}

func TestBuildProviderRequest_NormalizeNonstandardChatRoles(t *testing.T) {
	gin.SetMode(gin.TestMode)
	tests := []struct {
		name        string
		serviceType string
	}{
		{name: "openai", serviceType: "openai"},
		{name: "gemini_without_model_redirect", serviceType: "gemini"},
	}

	bodyBytes := []byte(`{"model":"gpt-5","messages":[{"role":"system","content":"sys"},{"role":"developer","content":"dev"},{"role":"user","content":"hi"},{"role":"assistant","content":"ok"},{"role":"tool","tool_call_id":"call_1","content":"{}"},{"role":"function","content":"legacy"},{"content":"missing"},{"role":123,"content":"number"}]}`)
	wantRoles := []string{"system", "user", "user", "assistant", "tool", "user", "user", "user"}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil).WithContext(context.Background())

			upstream := &config.UpstreamConfig{
				ServiceType:                   tt.serviceType,
				NormalizeNonstandardChatRoles: true,
			}
			req, err := buildProviderRequest(c, upstream, "https://api.example.com", "sk-test", bodyBytes, "gpt-5", false)
			if err != nil {
				t.Fatalf("buildProviderRequest() err = %v", err)
			}

			var got map[string]interface{}
			if err := json.NewDecoder(req.Body).Decode(&got); err != nil {
				t.Fatalf("decode request body: %v", err)
			}

			messages, ok := got["messages"].([]interface{})
			if !ok || len(messages) != len(wantRoles) {
				t.Fatalf("messages = %#v, want %d items", got["messages"], len(wantRoles))
			}

			for i, want := range wantRoles {
				msg, ok := messages[i].(map[string]interface{})
				if !ok {
					t.Fatalf("message[%d] = %#v, want object", i, messages[i])
				}
				if gotRole := msg["role"]; gotRole != want {
					t.Fatalf("message[%d].role = %v, want %s", i, gotRole, want)
				}
			}
		})
	}
}

func TestBuildProviderRequest_FunctionWithToolCallIDMapsToTool(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil).WithContext(context.Background())

	bodyBytes := []byte(`{"model":"gpt-5","messages":[{"role":"assistant","content":"ok","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"function","name":"f","content":"result","tool_call_id":"call_1"}]}`)
	upstream := &config.UpstreamConfig{
		ServiceType:                   "openai",
		NormalizeNonstandardChatRoles: true,
	}
	req, err := buildProviderRequest(c, upstream, "https://api.example.com", "sk-test", bodyBytes, "gpt-5", false)
	if err != nil {
		t.Fatalf("buildProviderRequest() err = %v", err)
	}

	var got map[string]interface{}
	if err := json.NewDecoder(req.Body).Decode(&got); err != nil {
		t.Fatalf("decode request body: %v", err)
	}

	messages := got["messages"].([]interface{})
	if len(messages) != 2 {
		t.Fatalf("messages length = %d, want 2", len(messages))
	}
	second := messages[1].(map[string]interface{})
	if second["role"] != "tool" {
		t.Fatalf("function message with tool_call_id role = %v, want tool", second["role"])
	}
}

func TestBuildProviderRequest_PreservesMultimodalContentArray(t *testing.T) {
	gin.SetMode(gin.TestMode)

	tests := []struct {
		name        string
		serviceType string
		upstream    *config.UpstreamConfig
		model       string
		wantModel   string
	}{
		{
			name:        "openai_passthrough_keeps_image_url",
			serviceType: "openai",
			upstream: &config.UpstreamConfig{
				ServiceType: "openai",
			},
			model:     "gpt-4o-image",
			wantModel: "gpt-4o-image",
		},
		{
			name:        "responses_passthrough_keeps_image_url",
			serviceType: "responses",
			upstream: &config.UpstreamConfig{
				ServiceType: "responses",
			},
			model:     "gpt-4o-image",
			wantModel: "gpt-4o-image",
		},
		{
			name:        "gemini_passthrough_keeps_image_url_without_remarshal",
			serviceType: "gemini",
			upstream: &config.UpstreamConfig{
				ServiceType: "gemini",
			},
			model:     "gpt-4o-image",
			wantModel: "gpt-4o-image",
		},
		{
			name:        "gemini_passthrough_keeps_image_url_with_remarshal",
			serviceType: "gemini",
			upstream: &config.UpstreamConfig{
				ServiceType: "gemini",
				ModelMapping: map[string]string{
					"gpt-4o-image": "gemini-2.5-flash-image-preview",
				},
			},
			model:     "gpt-4o-image",
			wantModel: "gemini-2.5-flash-image-preview",
		},
	}

	bodyBytes := []byte(`{"model":"gpt-4o-image","messages":[{"role":"user","content":[{"type":"text","text":"修改这个图片"},{"type":"image_url","image_url":{"url":"https://example.com/image.png"}}]}]}`)

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			w := httptest.NewRecorder()
			c, _ := gin.CreateTestContext(w)
			c.Request = httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil).WithContext(context.Background())

			req, err := buildProviderRequest(c, tt.upstream, "https://api.example.com", "sk-test", bodyBytes, tt.model, false)
			if err != nil {
				t.Fatalf("buildProviderRequest() err = %v", err)
			}

			var got map[string]interface{}
			if err := json.NewDecoder(req.Body).Decode(&got); err != nil {
				t.Fatalf("decode request body: %v", err)
			}

			if got["model"] != tt.wantModel {
				t.Fatalf("model = %v, want %v", got["model"], tt.wantModel)
			}

			messages, ok := got["messages"].([]interface{})
			if !ok || len(messages) != 1 {
				t.Fatalf("messages = %#v, want single message", got["messages"])
			}

			msg, ok := messages[0].(map[string]interface{})
			if !ok {
				t.Fatalf("message[0] = %#v, want object", messages[0])
			}

			content, ok := msg["content"].([]interface{})
			if !ok || len(content) != 2 {
				t.Fatalf("content = %#v, want 2-part array", msg["content"])
			}

			textPart, ok := content[0].(map[string]interface{})
			if !ok || textPart["type"] != "text" || textPart["text"] != "修改这个图片" {
				t.Fatalf("text part = %#v, want text block", content[0])
			}

			imagePart, ok := content[1].(map[string]interface{})
			if !ok || imagePart["type"] != "image_url" {
				t.Fatalf("image part = %#v, want image_url block", content[1])
			}

			imageURL, ok := imagePart["image_url"].(map[string]interface{})
			if !ok || imageURL["url"] != "https://example.com/image.png" {
				t.Fatalf("image_url = %#v, want original url", imagePart["image_url"])
			}
		})
	}
}

func TestConvertChatToClaudeRequest_ConvertsImageURLContent(t *testing.T) {
	bodyBytes := []byte(`{"model":"gpt-4o-image","messages":[{"role":"user","content":[{"type":"text","text":"看这张图"},{"type":"image_url","image_url":{"url":"data:image/png;base64,abc123"}}]}]}`)

	claudeReq, err := convertChatToClaudeRequest(bodyBytes, "claude-3-5-sonnet-20241022", false)
	if err != nil {
		t.Fatalf("convertChatToClaudeRequest() err = %v", err)
	}

	messages, ok := claudeReq["messages"].([]map[string]interface{})
	if !ok || len(messages) != 1 {
		t.Fatalf("messages = %#v, want single Claude message", claudeReq["messages"])
	}
	content, ok := messages[0]["content"].([]map[string]interface{})
	if !ok || len(content) != 2 {
		t.Fatalf("content = %#v, want text and image blocks", messages[0]["content"])
	}
	if content[0]["type"] != "text" || content[0]["text"] != "看这张图" {
		t.Fatalf("text block = %#v, want Claude text block", content[0])
	}
	if content[1]["type"] != "image" {
		t.Fatalf("image block type = %#v, want image", content[1]["type"])
	}
	source, ok := content[1]["source"].(map[string]interface{})
	if !ok {
		t.Fatalf("source = %#v, want object", content[1]["source"])
	}
	if source["type"] != "base64" || source["media_type"] != "image/png" || source["data"] != "abc123" {
		t.Fatalf("source = %#v, want Claude base64 image source", source)
	}
}

func TestBuildProviderRequest_OmitsImageURLForDeepSeekTextChannel(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/chat/completions", nil).WithContext(context.Background())

	bodyBytes := []byte(`{"model":"deepseek-v3.2","messages":[{"role":"user","content":[{"type":"text","text":"继续分析"},{"type":"image_url","image_url":{"url":"https://example.com/image.png"}}]}]}`)
	upstream := &config.UpstreamConfig{
		Name:        "DeepSeek",
		ServiceType: "openai",
	}

	req, err := buildProviderRequest(c, upstream, "https://api.deepseek.com", "sk-test", bodyBytes, "deepseek-v3.2", false)
	if err != nil {
		t.Fatalf("buildProviderRequest() err = %v", err)
	}

	var got map[string]interface{}
	if err := json.NewDecoder(req.Body).Decode(&got); err != nil {
		t.Fatalf("decode request body: %v", err)
	}

	messages := got["messages"].([]interface{})
	content := messages[0].(map[string]interface{})["content"].([]interface{})
	for _, raw := range content {
		part := raw.(map[string]interface{})
		if part["type"] == "image_url" {
			t.Fatalf("content = %#v, should omit image_url for DeepSeek text channel", content)
		}
	}
	last := content[len(content)-1].(map[string]interface{})
	if last["type"] != "text" {
		t.Fatalf("last content part = %#v, want text placeholder", last)
	}
}

func TestInjectGeminiThoughtSignatures(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		wantSig  bool
		wantKeep string // 如果非空，期望保留原始 signature
	}{
		{
			name: "注入 dummy signature 到缺失的 tool_calls",
			input: `{"model":"gemini-3-pro","messages":[
				{"role":"user","content":"hi"},
				{"role":"assistant","tool_calls":[
					{"id":"call_1","type":"function","function":{"name":"test","arguments":"{}"}}
				]},
				{"role":"tool","tool_call_id":"call_1","content":"ok"}
			]}`,
			wantSig: true,
		},
		{
			name: "保留已有的 thought_signature",
			input: `{"model":"gemini-3-pro","messages":[
				{"role":"user","content":"hi"},
				{"role":"assistant","tool_calls":[
					{"id":"call_1","type":"function","function":{"name":"test","arguments":"{}"},
					 "extra_content":{"google":{"thought_signature":"real_sig_abc"}}}
				]},
				{"role":"tool","tool_call_id":"call_1","content":"ok"}
			]}`,
			wantSig:  true,
			wantKeep: "real_sig_abc",
		},
		{
			name:    "无 tool_calls 不修改",
			input:   `{"model":"gemini-3-pro","messages":[{"role":"user","content":"hi"}]}`,
			wantSig: false,
		},
		{
			name: "保留已有 extra_content 中的其他字段",
			input: `{"model":"gemini-3-pro","messages":[
				{"role":"user","content":"hi"},
				{"role":"assistant","tool_calls":[
					{"id":"call_1","type":"function","function":{"name":"test","arguments":"{}"},
					 "extra_content":{"custom_key":"custom_value","google":{"other_field":"keep_me"}}}
				]},
				{"role":"tool","tool_call_id":"call_1","content":"ok"}
			]}`,
			wantSig: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := injectGeminiThoughtSignatures([]byte(tt.input))

			var reqMap map[string]interface{}
			if err := json.Unmarshal(result, &reqMap); err != nil {
				t.Fatalf("unmarshal result: %v", err)
			}

			messages := reqMap["messages"].([]interface{})
			for _, msg := range messages {
				msgMap := msg.(map[string]interface{})
				if msgMap["role"] != "assistant" {
					continue
				}
				toolCalls, ok := msgMap["tool_calls"].([]interface{})
				if !ok || len(toolCalls) == 0 {
					continue
				}

				firstTC := toolCalls[0].(map[string]interface{})
				extraContent, hasEC := firstTC["extra_content"].(map[string]interface{})

				if tt.wantSig && !hasEC {
					t.Fatal("expected extra_content but not found")
				}
				if !tt.wantSig {
					return
				}

				google := extraContent["google"].(map[string]interface{})
				sig := google["thought_signature"].(string)

				if tt.wantKeep != "" {
					if sig != tt.wantKeep {
						t.Fatalf("expected signature %q, got %q", tt.wantKeep, sig)
					}
				} else {
					if sig == "" {
						t.Fatal("expected non-empty signature")
					}
				}

				// 验证 merge 行为：已有的 extra_content 字段应被保留
				if tt.name == "保留已有 extra_content 中的其他字段" {
					if _, ok := extraContent["custom_key"]; !ok {
						t.Fatal("extra_content.custom_key was lost during merge")
					}
					if otherField, ok := google["other_field"].(string); !ok || otherField != "keep_me" {
						t.Fatal("extra_content.google.other_field was lost during merge")
					}
				}
			}
		})
	}
}
