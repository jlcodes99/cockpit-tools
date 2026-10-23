package multiagentv2

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/tidwall/gjson"
)

func TestCPAMultiAgentCompatibilityIsIndependentOfOptimizerAndClient(t *testing.T) {
	payload := []byte(`{"input":[{"type":"agent_message","id":"keep-id","author":"/root","recipient":"/root/child","internal_chat_message_metadata_passthrough":{"keep":1},"content":[{"type":"encrypted_content","encrypted_content":"literal payload"}]},{"type":"message","role":"user","author":"private","recipient":"private","internal_chat_message_metadata_passthrough":{},"content":"keep text"}]}`)
	for _, enabled := range []bool{false, true} {
		for _, userAgent := range []string{"", "curl/synthetic", "codex_exec/synthetic", "Codex Desktop/synthetic", "codex_cli_rs", "codex_cli_rs/synthetic", "codex-tui/synthetic"} {
			cfg := &config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: enabled}}
			headers := http.Header{"User-Agent": {userAgent}}
			got := RewriteCodexMultiAgentV2Input(context.Background(), headers, payload, cfg, true)
			if gjson.GetBytes(got, "input.0.type").String() != "message" || gjson.GetBytes(got, "input.0.content.0.text").String() != "literal payload" || gjson.GetBytes(got, "input.0.id").String() != "keep-id" {
				t.Fatalf("compatibility depends on optional client flags: %s", got)
			}
			for _, item := range gjson.GetBytes(got, "input").Array() {
				for _, key := range []string{"author", "recipient", "internal_chat_message_metadata_passthrough"} {
					if item.Get(key).Exists() {
						t.Fatalf("internal metadata %s survived: %s", key, got)
					}
				}
			}
			withoutCompat := RewriteCodexMultiAgentV2Input(context.Background(), headers, payload, cfg)
			if enabled && IsCodexClientUserAgent(userAgent) {
				if gjson.GetBytes(withoutCompat, "input.0.type").String() != "message" || gjson.GetBytes(withoutCompat, "input.0.author").String() != "/root" {
					t.Fatalf("optimization without compat should retain routing metadata: %s", withoutCompat)
				}
			} else if string(withoutCompat) != string(payload) {
				t.Fatalf("ineligible request changed: %s", withoutCompat)
			}
		}
	}
	ginContext, _ := gin.CreateTestContext(httptest.NewRecorder())
	ginContext.Request = httptest.NewRequest(http.MethodPost, "/v1/responses", nil)
	ginContext.Request.Header.Set("User-Agent", "curl/synthetic")
	ctx := context.WithValue(context.Background(), "gin", ginContext)
	got := RewriteCodexMultiAgentV2Input(ctx, http.Header{"User-Agent": {"codex_exec/synthetic"}}, payload, &config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: true}})
	if string(got) != string(payload) {
		t.Fatal("Gin's actual client identity must override forwarded headers")
	}
}
