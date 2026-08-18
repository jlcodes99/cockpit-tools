package executor

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestNormalizeCodexResponsesLiteRequestFiltersUnsupportedToolsAndKeepsNamespaces(t *testing.T) {
	body := []byte(`{
		"parallel_tool_calls": true,
		"tools": [
			{"type":"function","name":"lookup"},
			{"type":"custom","name":"apply_patch"},
			{"type":"tool_search","execution":"client"},
			{"type":"tool_search","execution":"server"},
			{"type":"web_search"},
			{"type":"image_generation"},
			{"type":"namespace","name":"functions.collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"type":"object"}}]}
		],
		"tool_choice": {"type":"image_generation"}
	}`)
	headers := http.Header{codexResponsesLiteHeaderName: []string{"true"}}

	oauth := &cliproxyauth.Auth{Metadata: map[string]any{"access_token": "oauth-token"}}
	result, useFullResponses := normalizeCodexResponsesLiteRequest(body, headers, oauth, false)
	if useFullResponses {
		t.Fatal("compact-compatible filtering should not switch to full Responses")
	}

	if gjson.GetBytes(result, "parallel_tool_calls").Bool() {
		t.Fatalf("parallel_tool_calls = true, want false: %s", result)
	}
	tools := gjson.GetBytes(result, "tools").Array()
	if len(tools) != 4 {
		t.Fatalf("len(tools) = %d, want 4: %s", len(tools), result)
	}
	if got := tools[0].Get("type").String(); got != "function" {
		t.Fatalf("tools.0.type = %q, want function", got)
	}
	if got := tools[1].Get("type").String(); got != "custom" {
		t.Fatalf("tools.1.type = %q, want custom", got)
	}
	if got := tools[2].Get("type").String(); got != "tool_search" {
		t.Fatalf("tools.2.type = %q, want tool_search", got)
	}
	if got := tools[3].Get("name").String(); got != "functions.collaboration" {
		t.Fatalf("tools.3.name = %q, want functions.collaboration", got)
	}
	if got := tools[3].Get("tools.0.name").String(); got != "spawn_agent" {
		t.Fatalf("tools.3.tools.0.name = %q, want spawn_agent", got)
	}
	if gjson.GetBytes(result, "tool_choice").Exists() {
		t.Fatalf("unsupported tool_choice was not removed: %s", result)
	}
}

func TestNormalizeCodexResponsesLiteRequestRemovesEmptyTools(t *testing.T) {
	body := []byte(`{"tools":[{"type":"image_generation"}],"tool_choice":"image_generation"}`)
	headers := http.Header{codexResponsesLiteHeaderName: []string{"true"}}

	oauth := &cliproxyauth.Auth{Metadata: map[string]any{"access_token": "oauth-token"}}
	result, useFullResponses := normalizeCodexResponsesLiteRequest(body, headers, oauth, false)
	if useFullResponses {
		t.Fatal("compact-compatible filtering should not switch to full Responses")
	}

	if gjson.GetBytes(result, "tools").Exists() {
		t.Fatalf("empty tools should be removed: %s", result)
	}
	if gjson.GetBytes(result, "tool_choice").Exists() {
		t.Fatalf("unsupported tool_choice should be removed: %s", result)
	}
}

func TestNormalizeCodexResponsesLiteRequestLeavesRegularRequestUnchanged(t *testing.T) {
	body := []byte(`{"parallel_tool_calls":true,"tools":[{"type":"image_generation"}]}`)

	result, useFullResponses := normalizeCodexResponsesLiteRequest(body, nil, nil, true)
	if useFullResponses {
		t.Fatal("regular request should not switch Responses mode")
	}

	if string(result) != string(body) {
		t.Fatalf("regular request changed: %s", result)
	}
}

func TestNormalizeCodexResponsesLiteRequestKeepsAPIKeyImageGeneration(t *testing.T) {
	body := []byte(`{"parallel_tool_calls":true,"tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"}}`)
	headers := http.Header{codexResponsesLiteHeaderName: []string{"true"}}
	auth := &cliproxyauth.Auth{Attributes: map[string]string{
		"api_key":  "sk-test",
		"base_url": "https://api.example.com/v1",
	}}

	result, useFullResponses := normalizeCodexResponsesLiteRequest(body, headers, auth, true)
	if useFullResponses {
		t.Fatal("API key request should keep its existing Responses mode")
	}

	if string(result) != string(body) {
		t.Fatalf("API key request changed: %s", result)
	}
}

func TestNormalizeCodexResponsesLiteRequestUsesFullResponsesForOAuthImageGeneration(t *testing.T) {
	body := []byte(`{"parallel_tool_calls":true,"tools":[{"type":"image_generation","output_format":"png"}],"tool_choice":{"type":"image_generation"}}`)
	headers := http.Header{codexResponsesLiteHeaderName: []string{"true"}}
	oauth := &cliproxyauth.Auth{Metadata: map[string]any{"access_token": "oauth-token"}}

	result, useFullResponses := normalizeCodexResponsesLiteRequest(body, headers, oauth, true)

	if !useFullResponses {
		t.Fatal("OAuth image generation should switch to full Responses")
	}
	if gjson.GetBytes(result, "parallel_tool_calls").Bool() {
		t.Fatalf("parallel_tool_calls = true, want false: %s", result)
	}
	if got := gjson.GetBytes(result, "tools.0.type").String(); got != "image_generation" {
		t.Fatalf("tools.0.type = %q, want image_generation: %s", got, result)
	}
	if got := gjson.GetBytes(result, "tool_choice.type").String(); got != "image_generation" {
		t.Fatalf("tool_choice.type = %q, want image_generation: %s", got, result)
	}
}

func TestNormalizeCodexResponsesLiteRequestUsesFullResponsesForCodexImageGenTools(t *testing.T) {
	tests := []struct {
		name string
		body []byte
	}{
		{
			name: "qualified function",
			body: []byte(`{"tools":[{"type":"function","name":"image_gen.imagegen"}],"tool_choice":{"type":"function","name":"image_gen.imagegen"}}`),
		},
		{
			name: "namespace",
			body: []byte(`{"tools":[{"type":"namespace","name":"image_gen","tools":[{"type":"function","name":"imagegen"}]}]}`),
		},
	}
	headers := http.Header{codexResponsesLiteHeaderName: []string{"true"}}
	oauth := &cliproxyauth.Auth{Metadata: map[string]any{"access_token": "oauth-token"}}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result, useFullResponses := normalizeCodexResponsesLiteRequest(tt.body, headers, oauth, true)

			if !useFullResponses {
				t.Fatal("Codex imagegen tool should switch to full Responses")
			}
			if got := gjson.GetBytes(result, "tools.0.name").String(); got == "" {
				t.Fatalf("imagegen tool was removed: %s", result)
			}
		})
	}
}

func TestCodexExecutorExecutePreservesCollaborationNamespaceInResponsesLiteAdditionalTools(t *testing.T) {
	type capturedRequest struct {
		header http.Header
		body   []byte
	}
	captured := make(chan capturedRequest, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		captured <- capturedRequest{header: r.Header.Clone(), body: body}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = w.Write([]byte("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"created_at\":0,\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"total_tokens\":0}}}\n\n"))
	}))
	defer server.Close()

	ginContext, _ := gin.CreateTestContext(httptest.NewRecorder())
	ginContext.Request = httptest.NewRequest(http.MethodPost, "http://localhost/v1/responses", nil)
	ginContext.Request.Header.Set(codexResponsesLiteHeaderName, "true")
	ctx := context.WithValue(context.Background(), "gin", ginContext)

	executor := NewCodexExecutor(&config.Config{})
	auth := &cliproxyauth.Auth{
		Attributes: map[string]string{"base_url": server.URL},
		Metadata:   map[string]any{"access_token": "oauth-token"},
	}
	payload := []byte(`{
		"model":"gpt-5.6-sol",
		"instructions":"",
		"parallel_tool_calls":true,
		"tool_choice":"required",
		"input":[
			{"type":"additional_tools","role":"developer","tools":[
				{"type":"namespace","name":"collaboration","tools":[
					{"type":"function","name":"spawn_agent","parameters":{"type":"object"}},
					{"type":"function","name":"wait_agent","parameters":{"type":"object"}},
					{"type":"function","name":"send_message","parameters":{"type":"object"}},
					{"type":"function","name":"followup_task","parameters":{"type":"object"}},
					{"type":"function","name":"interrupt_agent","parameters":{"type":"object"}},
					{"type":"function","name":"list_agents","parameters":{"type":"object"}}
				]}
			]},
			{"type":"message","role":"user","content":[{"type":"input_text","text":"delegate this task"}]}
		]
	}`)

	_, err := executor.Execute(ctx, auth, cliproxyexecutor.Request{
		Model:   "gpt-5.6-sol",
		Payload: payload,
	}, cliproxyexecutor.Options{
		SourceFormat: sdktranslator.FromString("openai-response"),
		Headers:      http.Header{codexResponsesLiteHeaderName: []string{"true"}},
	})
	if err != nil {
		t.Fatalf("Execute error: %v", err)
	}

	got := <-captured
	if !codexResponsesLiteEnabled(got.header) {
		t.Fatalf("upstream request lost Responses Lite header: %v", got.header)
	}
	if gjson.GetBytes(got.body, "parallel_tool_calls").Bool() {
		t.Fatalf("parallel_tool_calls = true, want false: %s", got.body)
	}
	if choice := gjson.GetBytes(got.body, "tool_choice").String(); choice != "required" {
		t.Fatalf("tool_choice = %q, want required: %s", choice, got.body)
	}
	if name := gjson.GetBytes(got.body, "input.0.tools.0.name").String(); name != "collaboration" {
		t.Fatalf("upstream namespace = %q, want collaboration: %s", name, got.body)
	}
	wantTools := []string{"spawn_agent", "wait_agent", "send_message", "followup_task", "interrupt_agent", "list_agents"}
	tools := gjson.GetBytes(got.body, "input.0.tools.0.tools").Array()
	if len(tools) != len(wantTools) {
		t.Fatalf("upstream collaboration tool count = %d, want %d: %s", len(tools), len(wantTools), got.body)
	}
	for index, want := range wantTools {
		if name := tools[index].Get("name").String(); name != want {
			t.Fatalf("upstream collaboration tool %d = %q, want %q: %s", index, name, want, got.body)
		}
	}
}

func TestCodexExecutorExecutePreservesImageGenNamespaceOutsideResponsesLite(t *testing.T) {
	type capturedRequest struct {
		header http.Header
		body   []byte
	}
	captured := make(chan capturedRequest, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		captured <- capturedRequest{header: r.Header.Clone(), body: body}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = w.Write([]byte("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"created_at\":0,\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"total_tokens\":0}}}\n\n"))
	}))
	defer server.Close()

	ginContext, _ := gin.CreateTestContext(httptest.NewRecorder())
	ginContext.Request = httptest.NewRequest(http.MethodPost, "http://localhost/v1/responses", nil)
	ginContext.Request.Header.Set(codexResponsesLiteHeaderName, "true")
	ctx := context.WithValue(context.Background(), "gin", ginContext)

	executor := NewCodexExecutor(&config.Config{})
	auth := &cliproxyauth.Auth{
		Attributes: map[string]string{"base_url": server.URL},
		Metadata:   map[string]any{"access_token": "oauth-token"},
	}
	payload := []byte(`{
		"model":"gpt-5.4",
		"instructions":"",
		"input":"draw a test image",
		"tools":[{"type":"namespace","name":"image_gen","tools":[{"type":"function","name":"imagegen"}]}]
	}`)

	_, err := executor.Execute(ctx, auth, cliproxyexecutor.Request{
		Model:   "gpt-5.4",
		Payload: payload,
	}, cliproxyexecutor.Options{
		SourceFormat: sdktranslator.FromString("codex"),
		Headers:      http.Header{codexResponsesLiteHeaderName: []string{"true"}},
	})
	if err != nil {
		t.Fatalf("Execute error: %v", err)
	}

	got := <-captured
	if codexResponsesLiteEnabled(got.header) {
		t.Fatalf("upstream request retained Responses Lite header: %v", got.header)
	}
	if name := gjson.GetBytes(got.body, "tools.0.name").String(); name != "image_gen" {
		t.Fatalf("upstream imagegen namespace = %q, want image_gen: %s", name, got.body)
	}
	if name := gjson.GetBytes(got.body, "tools.0.tools.0.name").String(); name != "imagegen" {
		t.Fatalf("upstream imagegen function = %q, want imagegen: %s", name, got.body)
	}
}

func TestCodexExecutorExecuteKeepsAPIKeyImageGenerationPathUnchanged(t *testing.T) {
	type capturedRequest struct {
		header http.Header
		body   []byte
	}
	captured := make(chan capturedRequest, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		captured <- capturedRequest{header: r.Header.Clone(), body: body}
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = w.Write([]byte("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"created_at\":0,\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"total_tokens\":0}}}\n\n"))
	}))
	defer server.Close()

	ginContext, _ := gin.CreateTestContext(httptest.NewRecorder())
	ginContext.Request = httptest.NewRequest(http.MethodPost, "http://localhost/v1/responses", nil)
	ginContext.Request.Header.Set(codexResponsesLiteHeaderName, "true")
	ctx := context.WithValue(context.Background(), "gin", ginContext)

	executor := NewCodexExecutor(&config.Config{})
	auth := &cliproxyauth.Auth{Attributes: map[string]string{
		"api_key":  "sk-test",
		"base_url": server.URL,
	}}
	payload := []byte(`{
		"model":"gpt-5.4",
		"instructions":"",
		"input":"draw a test image",
		"parallel_tool_calls":true,
		"tools":[{"type":"image_generation","output_format":"png"}]
	}`)

	_, err := executor.Execute(ctx, auth, cliproxyexecutor.Request{
		Model:   "gpt-5.4",
		Payload: payload,
	}, cliproxyexecutor.Options{
		SourceFormat: sdktranslator.FromString("codex"),
		Headers:      http.Header{codexResponsesLiteHeaderName: []string{"true"}},
	})
	if err != nil {
		t.Fatalf("Execute error: %v", err)
	}

	got := <-captured
	if !codexResponsesLiteEnabled(got.header) {
		t.Fatalf("API key request lost its existing Responses Lite header: %v", got.header)
	}
	if toolType := gjson.GetBytes(got.body, "tools.0.type").String(); toolType != "image_generation" {
		t.Fatalf("API key image tool = %q, want image_generation: %s", toolType, got.body)
	}
	if !gjson.GetBytes(got.body, "parallel_tool_calls").Bool() {
		t.Fatalf("API key parallel_tool_calls changed: %s", got.body)
	}
}

func TestRemoveCodexResponsesLiteHeaderForFullResponse(t *testing.T) {
	headers := http.Header{
		"x-openai-internal-codex-responses-lite": []string{"true"},
		"X-Custom":                               []string{"keep-me"},
	}

	removeCodexResponsesLiteHeaderForFullResponse(headers, true)

	if codexResponsesLiteEnabled(headers) {
		t.Fatal("Responses Lite header should be removed for full Responses")
	}
	if got := headers.Get("X-Custom"); got != "keep-me" {
		t.Fatalf("X-Custom = %q, want keep-me", got)
	}
}
