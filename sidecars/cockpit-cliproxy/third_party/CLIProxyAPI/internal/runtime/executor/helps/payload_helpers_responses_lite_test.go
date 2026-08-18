package helps

import (
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/tidwall/gjson"
)

func TestIsCodexResponsesLiteRequestUsesCaseInsensitiveHeaderPresence(t *testing.T) {
	tests := []struct {
		name    string
		headers http.Header
		want    bool
	}{
		{
			name:    "canonical non-empty",
			headers: http.Header{CodexResponsesLiteHeader: {"true"}},
			want:    true,
		},
		{
			name:    "lowercase empty",
			headers: http.Header{"x-openai-internal-codex-responses-lite": {""}},
			want:    true,
		},
		{
			name:    "mixed case nil values",
			headers: http.Header{"x-OpEnAi-InTeRnAl-CoDeX-ReSpOnSeS-LiTe": nil},
			want:    true,
		},
		{
			name:    "absent",
			headers: http.Header{"X-Other": {"true"}},
			want:    false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := IsCodexResponsesLiteRequest(tt.headers); got != tt.want {
				t.Fatalf("IsCodexResponsesLiteRequest() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestShouldInjectImageGenerationToolResponsesLiteAlwaysBlocksInjection(t *testing.T) {
	headers := http.Header{"x-openai-internal-codex-responses-lite": {""}}
	if ShouldInjectImageGenerationTool(&config.Config{}, "/v1/responses", headers) {
		t.Fatal("Responses Lite must not inject image_generation")
	}
	if ShouldInjectImageGenerationToolForModel(&config.Config{}, "gpt-5.6-sol", "/v1/responses", nil) {
		t.Fatal("Responses Lite registry model must not inject image_generation without a header")
	}
	if !ShouldInjectImageGenerationTool(&config.Config{}, "/v1/responses", nil) {
		t.Fatal("regular Responses request unexpectedly blocked image_generation")
	}
}

func TestApplyPayloadConfigWithRequestResponsesLiteRegistryModelFiltersWithoutHeader(t *testing.T) {
	payload := []byte(`{"tools":[{"type":"function","name":"keep"},{"type":"web_search"},{"type":"image_generation"}]}`)

	out := ApplyPayloadConfigWithRequest(nil, "gpt-5.6-terra", "codex", "openai-response", "", payload, nil, "", "/v1/responses", nil)

	assertResponsesLiteToolTypes(t, out, "tools", []string{"function"})
}

func TestApplyPayloadConfigWithRequestResponsesLiteFiltersAllToolDeclarations(t *testing.T) {
	cfg := &config.Config{
		Payload: config.PayloadConfig{
			OverrideRaw: []config.PayloadRule{
				{
					Models: []config.PayloadModelRule{{Name: "gpt-5.6-sol", Protocol: "codex"}},
					Params: map[string]any{
						"tools": `[
							{"type":"function","name":"lookup"},
							{"type":"custom","name":"apply_patch"},
							{"type":"tool_search","execution":"client"},
							{"type":"tool_search","execution":"server"},
							{"type":"web_search"},
							{"type":"image_generation"},
							{"type":"namespace","name":"codex_app","tools":[{"type":"function","name":"list_threads"}]}
						]`,
						"tool_choice": `{"type":"allowed_tools","mode":"auto","tools":[
							{"type":"function","name":"lookup"},
							{"type":"web_search"},
							{"type":"custom","name":"apply_patch"}
						]}`,
					},
				},
			},
		},
	}
	payload := []byte(`{
		"model":"gpt-5.6-sol",
		"input":[
			{"type":"additional_tools","tools":[
				{"type":"function","name":"dynamic_fn"},
				{"type":"custom","name":"dynamic_custom"},
				{"type":"tool_search","execution":"client"},
				{"type":"tool_search","execution":"server"},
				{"type":"web_search"},
				{"type":"image_generation"},
				{"type":"namespace","name":"collaboration","tools":[
					{"type":"function","name":"spawn_agent","parameters":{"type":"object"}},
					{"type":"function","name":"wait_agent","parameters":{"type":"object"}}
				]}
			],"tool_choice":{"type":"web_search"}},
			{"type":"message","role":"user","tools":[{"type":"web_search"}]}
		],
		"response":{
			"tools":[
				{"type":"function","name":"response_fn"},
				{"type":"custom","name":"response_custom"},
				{"type":"tool_search","execution":"client"},
				{"type":"tool_search","execution":"server"},
				{"type":"web_search"}
			],
			"tool_choice":{"type":"web_search"}
		}
	}`)
	headers := http.Header{"x-openai-internal-codex-responses-lite": {""}}

	out := ApplyPayloadConfigWithRequest(
		cfg,
		"gpt-5.6-sol",
		"codex",
		"openai-response",
		"",
		payload,
		nil,
		"gpt-5.6-sol",
		"/v1/responses",
		headers,
	)

	assertResponsesLiteToolTypes(t, out, "tools", []string{"function", "custom", "tool_search", "namespace"})
	assertResponsesLiteToolTypes(t, out, "tool_choice.tools", []string{"function", "custom"})
	assertResponsesLiteToolTypes(t, out, "input.0.tools", []string{"function", "custom", "tool_search", "namespace"})
	assertResponsesLiteToolTypes(t, out, "response.tools", []string{"function", "custom", "tool_search"})
	if gjson.GetBytes(out, "input.0.tool_choice").Exists() {
		t.Fatalf("unsupported additional_tools tool_choice survived: %s", out)
	}
	if got := gjson.GetBytes(out, "input.#").Int(); got != 2 {
		t.Fatalf("input item count = %d, want 2 with collaboration additional_tools preserved: %s", got, out)
	}
	if got := gjson.GetBytes(out, "input.0.tools.3.name").String(); got != "collaboration" {
		t.Fatalf("collaboration namespace = %q, want collaboration: %s", got, out)
	}
	if got := gjson.GetBytes(out, "input.0.tools.3.tools.0.name").String(); got != "spawn_agent" {
		t.Fatalf("collaboration tool = %q, want spawn_agent: %s", got, out)
	}
	if got := gjson.GetBytes(out, "input.0.tools.3.tools.1.name").String(); got != "wait_agent" {
		t.Fatalf("collaboration tool = %q, want wait_agent: %s", got, out)
	}
	if got := gjson.GetBytes(out, "input.1.tools.0.type").String(); got != "web_search" {
		t.Fatalf("non-additional_tools history was changed, type=%q: %s", got, out)
	}
	if gjson.GetBytes(out, "response.tool_choice").Exists() {
		t.Fatalf("unsupported nested response.tool_choice survived: %s", out)
	}
}

func TestApplyPayloadConfigWithRequestResponsesLiteSupportsRootAndRemovesEmptyChoice(t *testing.T) {
	payload := []byte(`{"request":{"tools":[{"type":"function","name":"keep"},{"type":"web_search"}],"tool_choice":{"type":"allowed_tools","tools":[{"type":"web_search"}]},"response":{"tools":[{"type":"custom","name":"keep"},{"type":"image_generation"}]}}}`)
	headers := http.Header{CodexResponsesLiteHeader: {""}}

	out := ApplyPayloadConfigWithRequest(nil, "gpt-5.6-sol", "codex", "openai-response", "request", payload, nil, "", "/v1/responses", headers)

	assertResponsesLiteToolTypes(t, out, "request.tools", []string{"function"})
	assertResponsesLiteToolTypes(t, out, "request.response.tools", []string{"custom"})
	if gjson.GetBytes(out, "request.tool_choice").Exists() {
		t.Fatalf("empty allowed_tools choice survived: %s", out)
	}
}

func TestApplyPayloadConfigWithRequestRegularRequestLeavesToolsUnchanged(t *testing.T) {
	payload := []byte(`{"tools":[{"type":"function","name":"keep"},{"type":"web_search"},{"type":"image_generation"}],"tool_choice":{"type":"web_search"},"response":{"tools":[{"type":"namespace","name":"keep"}]}}`)

	out := ApplyPayloadConfigWithRequest(nil, "gpt-5.5", "codex", "openai-response", "", payload, nil, "", "/v1/responses", nil)
	if string(out) != string(payload) {
		t.Fatalf("regular request changed:\n got: %s\nwant: %s", out, payload)
	}
}

func TestApplyPayloadConfigWithRequestLiteHeaderPreservesImageToolsForNonLiteModel(t *testing.T) {
	// Non-catalog models can still carry the Lite feature header while using
	// image_gen / image_generation. Those tools must survive so the executor can
	// upgrade OAuth to full Responses or keep API-key image generation intact.
	headers := http.Header{CodexResponsesLiteHeader: {"true"}}

	imageGenPayload := []byte(`{"tools":[{"type":"image_generation","output_format":"png"},{"type":"function","name":"keep"}]}`)
	out := ApplyPayloadConfigWithRequest(nil, "gpt-5.4", "codex", "openai-response", "", imageGenPayload, nil, "", "/v1/responses", headers)
	if got := gjson.GetBytes(out, "tools.0.type").String(); got != "image_generation" {
		t.Fatalf("hosted image tool stripped for non-lite model: %s", out)
	}
	if got := gjson.GetBytes(out, "tools.1.name").String(); got != "keep" {
		t.Fatalf("function tool changed: %s", out)
	}

	namespacePayload := []byte(`{"tools":[{"type":"namespace","name":"image_gen","tools":[{"type":"function","name":"imagegen"}]},{"type":"web_search"}]}`)
	out = ApplyPayloadConfigWithRequest(nil, "gpt-5.4", "codex", "openai-response", "", namespacePayload, nil, "", "/v1/responses", headers)
	if got := gjson.GetBytes(out, "tools.0.name").String(); got != "image_gen" {
		t.Fatalf("image_gen namespace stripped for non-lite model: %s", out)
	}
	// web_search is also preserved because image tools short-circuit the Lite filter.
	if got := gjson.GetBytes(out, "tools.1.type").String(); got != "web_search" {
		t.Fatalf("tools unexpectedly rewritten: %s", out)
	}
}

func TestApplyPayloadConfigWithRequestLiteHeaderFiltersNonImageToolsForNonLiteModel(t *testing.T) {
	headers := http.Header{CodexResponsesLiteHeader: {"true"}}
	payload := []byte(`{"tools":[{"type":"function","name":"keep"},{"type":"web_search"},{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent"}]}]}`)

	out := ApplyPayloadConfigWithRequest(nil, "gpt-5.4", "codex", "openai-response", "", payload, nil, "", "/v1/responses", headers)
	assertResponsesLiteToolTypes(t, out, "tools", []string{"function", "namespace"})
	if got := gjson.GetBytes(out, "tools.1.tools.0.name").String(); got != "spawn_agent" {
		t.Fatalf("collaboration namespace child = %q, want spawn_agent: %s", got, out)
	}
}

func assertResponsesLiteToolTypes(t *testing.T, payload []byte, path string, want []string) {
	t.Helper()
	tools := gjson.GetBytes(payload, path)
	if !tools.IsArray() {
		t.Fatalf("%s is not an array: %s", path, payload)
	}
	items := tools.Array()
	if len(items) != len(want) {
		t.Fatalf("%s length = %d, want %d: %s", path, len(items), len(want), payload)
	}
	for index, wantType := range want {
		if got := items[index].Get("type").String(); got != wantType {
			t.Fatalf("%s.%d.type = %q, want %q: %s", path, index, got, wantType, payload)
		}
	}
}
