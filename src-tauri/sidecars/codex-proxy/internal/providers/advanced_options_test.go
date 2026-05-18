package providers

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/gin-gonic/gin"
)

func TestOpenAIProvider_InjectsModelLevelReasoningAndChannelLevelOptions(t *testing.T) {
	gin.SetMode(gin.TestMode)
	c := newGinContext(http.MethodPost, "/v1/messages", []byte(`{"model":"gpt-5.1-codex","messages":[{"role":"user","content":"hi"}]}`), context.Background())
	upstream := &config.UpstreamConfig{
		BaseURL:     "https://api.example.com",
		ServiceType: "openai",
		ModelMapping: map[string]string{
			"gpt-5.1-codex": "gpt-5.2-codex",
		},
		ReasoningMapping: map[string]string{
			"gpt-5.1-codex": "xhigh",
		},
		TextVerbosity: "high",
		FastMode:      true,
	}

	p := &OpenAIProvider{}
	req, _, err := p.ConvertToProviderRequest(c, upstream, "sk-test")
	if err != nil {
		t.Fatalf("ConvertToProviderRequest() err = %v", err)
	}

	var body map[string]interface{}
	if err := json.NewDecoder(req.Body).Decode(&body); err != nil {
		t.Fatalf("decode request body: %v", err)
	}

	if got := body["model"]; got != "gpt-5.2-codex" {
		t.Fatalf("model = %v, want gpt-5.2-codex", got)
	}

	reasoning, ok := body["reasoning"].(map[string]interface{})
	if !ok || reasoning["effort"] != "xhigh" {
		t.Fatalf("reasoning = %#v, want effort=xhigh", body["reasoning"])
	}

	text, ok := body["text"].(map[string]interface{})
	if !ok || text["verbosity"] != "high" {
		t.Fatalf("text = %#v, want verbosity=high", body["text"])
	}

	if got := body["service_tier"]; got != "priority" {
		t.Fatalf("service_tier = %v, want priority", got)
	}
}

func TestResponsesProvider_PassthroughInjectsModelLevelReasoningAndChannelLevelOptions(t *testing.T) {
	gin.SetMode(gin.TestMode)
	c := newGinContext(http.MethodPost, "/v1/responses", []byte(`{"model":"gpt-5","input":"hi"}`), context.Background())
	upstream := &config.UpstreamConfig{
		BaseURL:     "https://api.example.com",
		ServiceType: "responses",
		ModelMapping: map[string]string{
			"gpt-5": "gpt-5.2",
		},
		ReasoningMapping: map[string]string{
			"gpt-5": "high",
		},
		TextVerbosity: "medium",
		FastMode:      true,
	}

	p := &ResponsesProvider{}
	req, _, err := p.ConvertToProviderRequest(c, upstream, "sk-test")
	if err != nil {
		t.Fatalf("ConvertToProviderRequest() err = %v", err)
	}

	var body map[string]interface{}
	if err := json.NewDecoder(req.Body).Decode(&body); err != nil {
		t.Fatalf("decode request body: %v", err)
	}

	if got := body["model"]; got != "gpt-5.2" {
		t.Fatalf("model = %v, want gpt-5.2", got)
	}

	reasoning, ok := body["reasoning"].(map[string]interface{})
	if !ok || reasoning["effort"] != "high" {
		t.Fatalf("reasoning = %#v, want effort=high", body["reasoning"])
	}

	text, ok := body["text"].(map[string]interface{})
	if !ok || text["verbosity"] != "medium" {
		t.Fatalf("text = %#v, want verbosity=medium", body["text"])
	}

	if got := body["service_tier"]; got != "priority" {
		t.Fatalf("service_tier = %v, want priority", got)
	}
}
