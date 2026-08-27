package executor

import (
	"strings"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/usage"
	"github.com/tidwall/gjson"
)

// --- image input detection -------------------------------------------------

func TestCodebuddyChatHasImageInput(t *testing.T) {
	tests := []struct {
		name string
		in   string
		want bool
	}{
		{
			name: "image_url part detected",
			in:   `{"messages":[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}]}`,
			want: true,
		},
		{
			name: "input_image part detected",
			in:   `{"messages":[{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,BBBB"}]}]}`,
			want: true,
		},
		{
			name: "text only",
			in:   `{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}`,
			want: false,
		},
		{
			name: "string content",
			in:   `{"messages":[{"role":"user","content":"hello"}]}`,
			want: false,
		},
		{
			name: "no messages",
			in:   `{"model":"auto"}`,
			want: false,
		},
		{
			name: "invalid json",
			in:   `not-json`,
			want: false,
		},
		{
			name: "historical image ignored when last user message is text-only",
			in:   `{"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]},{"role":"assistant","content":"ok"},{"role":"user","content":[{"type":"text","text":"继续"}]}]}`,
			want: false,
		},
		{
			name: "image in last user message detected despite text history",
			in:   `{"messages":[{"role":"user","content":[{"type":"text","text":"之前"}]},{"role":"assistant","content":"ok"},{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}]}`,
			want: true,
		},
		{
			name: "image in tool message detected (Read tool result)",
			in:   `{"messages":[{"role":"user","content":[{"type":"text","text":"读一下这张图"}]},{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"a.png\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}]}`,
			want: true,
		},
		{
			name: "historical tool image ignored when last user message is text-only",
			in:   `{"messages":[{"role":"user","content":[{"type":"text","text":"读图"}]},{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"a.png\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,HIST"}}]},{"role":"assistant","content":"看完了"},{"role":"user","content":[{"type":"text","text":"继续"}]}]}`,
			want: false,
		},
		{
			name: "no user message",
			in:   `{"messages":[{"role":"assistant","content":"ok"}]}`,
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := codebuddyChatHasImageInput([]byte(tt.in)); got != tt.want {
				t.Fatalf("codebuddyChatHasImageInput() = %v, want %v", got, tt.want)
			}
		})
	}
}

// --- model rewrite (routing mode) ------------------------------------------

func TestRewriteCodebuddyModel(t *testing.T) {
	tests := []struct {
		name  string
		in    string
		model string
		want  string
	}{
		{
			name:  "existing model replaced",
			in:    `{"model":"deepseek-v4-flash","messages":[]}`,
			model: "hy3-preview",
			want:  "hy3-preview",
		},
		{
			name:  "missing model added",
			in:    `{"messages":[]}`,
			model: "hy3-preview",
			want:  "hy3-preview",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			out := rewriteCodebuddyModel([]byte(tt.in), tt.model)
			got := gjson.GetBytes(out, "model").String()
			if got != tt.want {
				t.Fatalf("model = %q, want %q; out=%s", got, tt.want, out)
			}
			// messages must survive untouched
			if !gjson.GetBytes(out, "messages").Exists() {
				t.Fatalf("messages lost; out=%s", out)
			}
		})
	}
}

// --- image -> text replacement (preprocess mode) ---------------------------

func TestReplaceCodebuddyImagesWithText(t *testing.T) {
	in := `{"messages":[{"role":"user","content":[{"type":"text","text":"这是什么？"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}]}`
	out := replaceCodebuddyImagesWithText([]byte(in), "一张红色方块")

	parts := gjson.GetBytes(out, "messages.0.content").Array()
	if len(parts) != 2 {
		t.Fatalf("expected 2 parts, got %d; out=%s", len(parts), out)
	}
	if parts[0].Get("type").String() != "text" || parts[0].Get("text").String() != "这是什么？" {
		t.Fatalf("text part corrupted: %s", parts[0].Raw)
	}
	if parts[1].Get("type").String() != "text" {
		t.Fatalf("image part not replaced by text: %s", parts[1].Raw)
	}
	if got := parts[1].Get("text").String(); got != "一张红色方块" {
		t.Fatalf("replacement text = %q, want %q", got, "一张红色方块")
	}
}

func TestReplaceCodebuddyImagesWithText_NoImages(t *testing.T) {
	in := `{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}`
	out := replaceCodebuddyImagesWithText([]byte(in), "ignored")
	if string(out) != in {
		t.Fatalf("expected unchanged, got %s", out)
	}
}

// --- vision proxy plan (decision) -------------------------------------------

func TestCodebuddyVisionPlan(t *testing.T) {
	tests := []struct {
		name         string
		mode         string
		visionModel  string
		currentModel string
		hasImage     bool
		supportsImg  bool
		want         codebuddyVisionAction
	}{
		{
			name:         "off always passes through",
			mode:         config.CodebuddyVisionModeOff,
			currentModel: "deepseek-v4-flash",
			hasImage:     true,
			want:         codebuddyVisionPassThrough,
		},
		{
			name:         "no image passes through",
			mode:         config.CodebuddyVisionModeRouting,
			currentModel: "deepseek-v4-flash",
			hasImage:     false,
			want:         codebuddyVisionPassThrough,
		},
		{
			name:         "vision model itself passes through (no recursion)",
			mode:         config.CodebuddyVisionModeRouting,
			visionModel:  "hy3-preview",
			currentModel: "hy3-preview",
			hasImage:     true,
			want:         codebuddyVisionPassThrough,
		},
		{
			name:         "native vision model passes through",
			mode:         config.CodebuddyVisionModeRouting,
			visionModel:  "hy3-preview",
			currentModel: "glm-4.6v",
			hasImage:     true,
			supportsImg:  true,
			want:         codebuddyVisionPassThrough,
		},
		{
			name:         "routing swaps text-only model",
			mode:         config.CodebuddyVisionModeRouting,
			visionModel:  "hy3-preview",
			currentModel: "deepseek-v4-flash",
			hasImage:     true,
			want:         codebuddyVisionRoute,
		},
		{
			name:         "preprocess describes then keeps model",
			mode:         config.CodebuddyVisionModePreprocess,
			visionModel:  "hy3-preview",
			currentModel: "deepseek-v4-flash",
			hasImage:     true,
			want:         codebuddyVisionPreprocess,
		},
		{
			name:         "unknown mode falls back to pass-through",
			mode:         "bogus",
			visionModel:  "hy3-preview",
			currentModel: "deepseek-v4-flash",
			hasImage:     true,
			want:         codebuddyVisionPassThrough,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := codebuddyVisionPlan(tt.mode, tt.visionModel, tt.currentModel, tt.hasImage, tt.supportsImg)
			if got != tt.want {
				t.Fatalf("codebuddyVisionPlan() = %v, want %v", got, tt.want)
			}
		})
	}
}

// --- agentic vision: image extraction & tool injection ---------------------

func TestExtractCodebuddyImagesForAgentic(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-pro","messages":[
		{"role":"user","content":[
			{"type":"text","text":"看这两张图"},
			{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}},
			{"type":"image_url","image_url":{"url":"data:image/png;base64,BBBB"}}
		]}
	]}`)

	out, images, err := extractCodebuddyImagesForAgentic(body)
	if err != nil {
		t.Fatalf("extractCodebuddyImagesForAgentic() error: %v", err)
	}
	if len(images) != 2 {
		t.Fatalf("expected 2 images, got %d", len(images))
	}
	if images[0].id != 1 || images[1].id != 2 {
		t.Fatalf("expected sequential ids 1,2, got %d,%d", images[0].id, images[1].id)
	}
	// First image part must be replaced by a text hint, second likewise.
	if gjson.GetBytes(out, "messages.0.content.1.type").String() != "text" {
		t.Fatalf("image part 1 should be replaced with text, got %s", gjson.GetBytes(out, "messages.0.content.1").Raw)
	}
	if !strings.Contains(gjson.GetBytes(out, "messages.0.content.1.text").String(), "inspect_image") {
		t.Fatalf("replacement should reference inspect_image tool")
	}
	// Text part untouched.
	if gjson.GetBytes(out, "messages.0.content.0.text").String() != "看这两张图" {
		t.Fatalf("text part should remain untouched")
	}
}

func TestExtractCodebuddyImagesForAgentic_OnlyLastUserMessage(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-pro","messages":[
		{"role":"user","content":[
			{"type":"image_url","image_url":{"url":"data:image/png;base64,HIST"}}
		]},
		{"role":"assistant","content":"ok"},
		{"role":"user","content":[
			{"type":"text","text":"再看这张"},
			{"type":"image_url","image_url":{"url":"data:image/png;base64,NEW"}}
		]}
	]}`)

	out, images, err := extractCodebuddyImagesForAgentic(body)
	if err != nil {
		t.Fatalf("extractCodebuddyImagesForAgentic() error: %v", err)
	}
	if len(images) != 1 {
		t.Fatalf("expected 1 image (only last user message), got %d", len(images))
	}
	// The historical image (messages.0.content.0) must be replaced with a
	// placeholder so it never reaches the text-only model.
	if gjson.GetBytes(out, "messages.0.content.0.type").String() != "text" {
		t.Fatalf("historical image should be replaced with text, got %s", gjson.GetBytes(out, "messages.0.content.0").Raw)
	}
	// The last user message's image (messages.2.content.1) must be replaced.
	if gjson.GetBytes(out, "messages.2.content.1.type").String() != "text" {
		t.Fatalf("last user image should be replaced with text, got %s", gjson.GetBytes(out, "messages.2.content.1").Raw)
	}
	if !strings.Contains(gjson.GetBytes(out, "messages.2.content.1.text").String(), "inspect_image") {
		t.Fatalf("replacement should reference inspect_image tool")
	}
}

func TestExtractCodebuddyImagesForAgentic_HistoricalImageIgnored(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-pro","messages":[
		{"role":"user","content":[
			{"type":"image_url","image_url":{"url":"data:image/png;base64,HIST"}}
		]},
		{"role":"assistant","content":"ok"},
		{"role":"user","content":[
			{"type":"text","text":"继续"}
		]}
	]}`)

	out, images, err := extractCodebuddyImagesForAgentic(body)
	if err != nil {
		t.Fatalf("extractCodebuddyImagesForAgentic() error: %v", err)
	}
	if len(images) != 0 {
		t.Fatalf("expected 0 images (last user message is text-only), got %d", len(images))
	}
	// The historical image must be replaced with a placeholder so it never
	// reaches the text-only model.
	if gjson.GetBytes(out, "messages.0.content.0.type").String() != "text" {
		t.Fatalf("historical image should be replaced with text, got %s", gjson.GetBytes(out, "messages.0.content.0").Raw)
	}
}

func TestExtractCodebuddyImagesForAgentic_ToolMessageImage(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-pro","messages":[
		{"role":"user","content":[{"type":"text","text":"读一下这张图"}]},
		{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"Read","arguments":"{\"file_path\":\"a.png\"}"}}]},
		{"role":"tool","tool_call_id":"call_1","content":[
			{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}
		]}
	]}`)

	out, images, err := extractCodebuddyImagesForAgentic(body)
	if err != nil {
		t.Fatalf("extractCodebuddyImagesForAgentic() error: %v", err)
	}
	if len(images) != 1 {
		t.Fatalf("expected 1 image from tool message, got %d", len(images))
	}
	if images[0].id != 1 {
		t.Fatalf("expected image id 1, got %d", images[0].id)
	}
	// The tool message's image must be replaced with a text hint referencing
	// inspect_image, so the text-only model can query it via the vision model.
	if gjson.GetBytes(out, "messages.2.content.0.type").String() != "text" {
		t.Fatalf("tool image should be replaced with text, got %s", gjson.GetBytes(out, "messages.2.content.0").Raw)
	}
	if !strings.Contains(gjson.GetBytes(out, "messages.2.content.0.text").String(), "inspect_image") {
		t.Fatalf("replacement should reference inspect_image tool")
	}
}

func TestInjectCodebuddyInspectTool(t *testing.T) {
	body := []byte(`{"model":"deepseek-v4-pro","messages":[{"role":"user","content":"hi"}]}`)
	out := injectCodebuddyInspectTool(body, 1)

	// tools array injected.
	tools := gjson.GetBytes(out, "tools")
	if !tools.IsArray() || len(tools.Array()) != 1 {
		t.Fatalf("expected 1 tool injected, got %s", tools.Raw)
	}
	if tools.Get("0.function.name").String() != inspectImageToolName {
		t.Fatalf("expected inspect_image tool, got %s", tools.Get("0.function.name").String())
	}

	// System message prepended.
	sysContent := gjson.GetBytes(out, "messages.0.content").String()
	if gjson.GetBytes(out, "messages.0.role").String() != "system" || !strings.Contains(sysContent, "inspect_image") {
		t.Fatalf("expected system guidance message, got role=%s content=%s",
			gjson.GetBytes(out, "messages.0.role").String(), sysContent)
	}
}

func TestAppendAgenticMessage(t *testing.T) {
	body := []byte(`{"messages":[{"role":"user","content":"hi"}]}`)
	out, err := appendAgenticMessage(body, []byte(`{"role":"assistant","content":"ok"}`))
	if err != nil {
		t.Fatalf("appendAgenticMessage() error: %v", err)
	}
	arr := gjson.GetBytes(out, "messages")
	if !arr.IsArray() || len(arr.Array()) != 2 {
		t.Fatalf("expected 2 messages, got %s", arr.Raw)
	}
	if gjson.GetBytes(out, "messages.1.role").String() != "assistant" {
		t.Fatalf("expected assistant message appended")
	}
}

// TestAddCodebuddyAgenticUsageAccumulatesTokenBreakdown guards the regression
// where addCodebuddyAgenticUsage only summed TokenBreakdown.TotalTokens, leaving
// the Input/Output sub-fields at zero and making the aggregated breakdown
// invalid (which in turn caused the request log's input/output columns to show 0).
func TestAddCodebuddyAgenticUsageAccumulatesTokenBreakdown(t *testing.T) {
	total := usage.Detail{}
	add1 := usage.Detail{
		InputTokens:  100,
		OutputTokens: 50,
		TokenBreakdown: usage.TokenBreakdown{
			TotalTokens: 150,
			Input: usage.TokenInputBreakdown{
				TotalTokens:      100,
				UncachedTokens:   80,
				CacheReadTokens:  15,
				CacheWriteTokens: 5,
			},
			Output: usage.TokenOutputBreakdown{
				TotalTokens:        50,
				NonReasoningTokens: 40,
				ReasoningTokens:    10,
			},
		},
	}
	add2 := usage.Detail{
		InputTokens:  20,
		OutputTokens: 30,
		TokenBreakdown: usage.TokenBreakdown{
			TotalTokens: 50,
			Input: usage.TokenInputBreakdown{
				TotalTokens:      20,
				UncachedTokens:   12,
				CacheReadTokens:  8,
				CacheWriteTokens: 0,
			},
			Output: usage.TokenOutputBreakdown{
				TotalTokens:        30,
				NonReasoningTokens: 30,
				ReasoningTokens:    0,
			},
		},
	}

	addCodebuddyAgenticUsage(&total, add1)
	addCodebuddyAgenticUsage(&total, add2)

	if total.InputTokens != 120 || total.OutputTokens != 80 {
		t.Fatalf("top-level tokens = %d/%d, want 120/80", total.InputTokens, total.OutputTokens)
	}
	if total.TokenBreakdown.TotalTokens != 200 {
		t.Fatalf("breakdown total = %d, want 200", total.TokenBreakdown.TotalTokens)
	}
	if total.TokenBreakdown.Input.TotalTokens != 120 ||
		total.TokenBreakdown.Input.UncachedTokens != 92 ||
		total.TokenBreakdown.Input.CacheReadTokens != 23 ||
		total.TokenBreakdown.Input.CacheWriteTokens != 5 {
		t.Fatalf("breakdown input = %+v", total.TokenBreakdown.Input)
	}
	if total.TokenBreakdown.Output.TotalTokens != 80 ||
		total.TokenBreakdown.Output.NonReasoningTokens != 70 ||
		total.TokenBreakdown.Output.ReasoningTokens != 10 {
		t.Fatalf("breakdown output = %+v", total.TokenBreakdown.Output)
	}
}

func TestCodebuddyVisionAgenticEnabled(t *testing.T) {
	off := &CodebuddyExecutor{cfg: &config.Config{SDKConfig: config.SDKConfig{CodebuddyVision: config.CodebuddyVisionConfig{Mode: "off"}}}}
	if off.codebuddyVisionAgenticEnabled() {
		t.Fatal("off mode should not report agentic enabled")
	}
	agentic := &CodebuddyExecutor{cfg: &config.Config{SDKConfig: config.SDKConfig{CodebuddyVision: config.CodebuddyVisionConfig{Mode: "agentic"}}}}
	if !agentic.codebuddyVisionAgenticEnabled() {
		t.Fatal("agentic mode should report enabled")
	}
}
