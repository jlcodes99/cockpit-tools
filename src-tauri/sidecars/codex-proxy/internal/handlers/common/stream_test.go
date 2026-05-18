package common

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/utils"
	"github.com/gin-gonic/gin"
)

func TestPatchUsageFieldsWithLog_NilInputTokens(t *testing.T) {
	tests := []struct {
		name           string
		usage          map[string]interface{}
		estimatedInput int
		hasCacheTokens bool
		wantPatched    bool
		wantValue      int
	}{
		{
			name:           "nil input_tokens without cache - should patch",
			usage:          map[string]interface{}{"input_tokens": nil, "output_tokens": float64(100)},
			estimatedInput: 10920,
			hasCacheTokens: false,
			wantPatched:    true,
			wantValue:      10920,
		},
		{
			name:           "nil input_tokens with cache - should also patch",
			usage:          map[string]interface{}{"input_tokens": nil, "output_tokens": float64(100)},
			estimatedInput: 10920,
			hasCacheTokens: true,
			wantPatched:    true,
			wantValue:      10920,
		},
		{
			name:           "valid input_tokens - should not patch",
			usage:          map[string]interface{}{"input_tokens": float64(5000), "output_tokens": float64(100)},
			estimatedInput: 10920,
			hasCacheTokens: true,
			wantPatched:    false,
			wantValue:      5000,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			patchUsageFieldsWithLog(tt.usage, tt.estimatedInput, 100, tt.hasCacheTokens, false, "test", false)

			if tt.wantPatched {
				if v, ok := tt.usage["input_tokens"].(int); !ok || v != tt.wantValue {
					t.Errorf("expected input_tokens=%d, got %v", tt.wantValue, tt.usage["input_tokens"])
				}
			} else if tt.usage["input_tokens"] == nil {
				// nil case - expected to remain nil
			} else if v, ok := tt.usage["input_tokens"].(float64); ok && int(v) != tt.wantValue {
				t.Errorf("expected input_tokens=%d, got %v", tt.wantValue, tt.usage["input_tokens"])
			}
		})
	}
}

func TestPatchMessageStartInputTokensIfNeeded(t *testing.T) {
	requestBody := []byte(`{"messages":[{"role":"user","content":[{"type":"text","text":"hello world hello world hello world"}]}]}`)
	estimated := utils.EstimateRequestTokens(requestBody)
	if estimated <= 0 {
		t.Fatalf("expected estimated input tokens > 0, got %d", estimated)
	}

	extractInputTokens := func(t *testing.T, event string) float64 {
		t.Helper()
		for _, line := range strings.Split(event, "\n") {
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			var data map[string]interface{}
			if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &data); err != nil {
				t.Fatalf("failed to unmarshal data: %v", err)
			}
			msg, ok := data["message"].(map[string]interface{})
			if !ok {
				t.Fatalf("missing message field")
			}
			usage, ok := msg["usage"].(map[string]interface{})
			if !ok {
				t.Fatalf("missing message.usage field")
			}
			v, ok := usage["input_tokens"].(float64)
			if !ok {
				t.Fatalf("missing input_tokens field")
			}
			return v
		}
		t.Fatalf("no data line found")
		return 0
	}

	t.Run("input_tokens=0 should patch in message_start", func(t *testing.T) {
		event := "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}}\n\n"
		hasUsage, needInputPatch, _, usageData := CheckEventUsageStatus(event, false)
		if !hasUsage {
			t.Fatalf("expected hasUsage=true")
		}
		if !needInputPatch {
			t.Fatalf("expected needInputPatch=true")
		}

		patched := PatchMessageStartInputTokensIfNeeded(event, requestBody, needInputPatch, usageData, false, false)
		got := extractInputTokens(t, patched)
		if got != float64(estimated) {
			t.Fatalf("expected input_tokens=%d, got %v", estimated, got)
		}
	})

	t.Run("input_tokens<10 should patch in message_start", func(t *testing.T) {
		event := "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n"
		hasUsage, needInputPatch, _, usageData := CheckEventUsageStatus(event, false)
		if !hasUsage {
			t.Fatalf("expected hasUsage=true")
		}
		if needInputPatch {
			t.Fatalf("expected needInputPatch=false")
		}

		patched := PatchMessageStartInputTokensIfNeeded(event, requestBody, needInputPatch, usageData, false, false)
		got := extractInputTokens(t, patched)
		if got != float64(estimated) {
			t.Fatalf("expected input_tokens=%d, got %v", estimated, got)
		}
	})

	t.Run("cache hit should not patch input_tokens", func(t *testing.T) {
		event := "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"cache_read_input_tokens\":100}}}\n\n"
		hasUsage, needInputPatch, _, usageData := CheckEventUsageStatus(event, false)
		if !hasUsage {
			t.Fatalf("expected hasUsage=true")
		}
		if needInputPatch {
			t.Fatalf("expected needInputPatch=false")
		}

		patched := PatchMessageStartInputTokensIfNeeded(event, requestBody, needInputPatch, usageData, false, false)
		got := extractInputTokens(t, patched)
		if got != 0 {
			t.Fatalf("expected input_tokens=0, got %v", got)
		}
	})

	t.Run("valid input_tokens should not patch", func(t *testing.T) {
		event := "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":50,\"output_tokens\":0}}}\n\n"
		hasUsage, needInputPatch, _, usageData := CheckEventUsageStatus(event, false)
		if !hasUsage {
			t.Fatalf("expected hasUsage=true")
		}
		if needInputPatch {
			t.Fatalf("expected needInputPatch=false")
		}

		patched := PatchMessageStartInputTokensIfNeeded(event, requestBody, needInputPatch, usageData, false, false)
		got := extractInputTokens(t, patched)
		if got != 50 {
			t.Fatalf("expected input_tokens=50, got %v", got)
		}
	})
}

// TestInferImplicitCacheRead 测试隐式缓存推断逻辑
func TestInferImplicitCacheRead(t *testing.T) {
	tests := []struct {
		name                    string
		messageStartInputTokens int
		collectedInputTokens    int
		existingCacheRead       int
		wantCacheRead           int
	}{
		{
			name:                    "large diff ratio (>10%) should infer cache",
			messageStartInputTokens: 100000,
			collectedInputTokens:    20000,
			existingCacheRead:       0,
			wantCacheRead:           80000,
		},
		{
			name:                    "large diff value (>10k) should infer cache",
			messageStartInputTokens: 50000,
			collectedInputTokens:    38000,
			existingCacheRead:       0,
			wantCacheRead:           12000,
		},
		{
			name:                    "small diff should not infer cache",
			messageStartInputTokens: 10000,
			collectedInputTokens:    9500,
			existingCacheRead:       0,
			wantCacheRead:           0,
		},
		{
			name:                    "existing cache_read should not be overwritten",
			messageStartInputTokens: 100000,
			collectedInputTokens:    20000,
			existingCacheRead:       50000,
			wantCacheRead:           50000,
		},
		{
			name:                    "zero message_start should not infer",
			messageStartInputTokens: 0,
			collectedInputTokens:    20000,
			existingCacheRead:       0,
			wantCacheRead:           0,
		},
		{
			name:                    "zero collected should not infer",
			messageStartInputTokens: 100000,
			collectedInputTokens:    0,
			existingCacheRead:       0,
			wantCacheRead:           0,
		},
		{
			name:                    "negative diff should not infer",
			messageStartInputTokens: 10000,
			collectedInputTokens:    15000,
			existingCacheRead:       0,
			wantCacheRead:           0,
		},
		{
			name:                    "exactly 10% diff should not infer",
			messageStartInputTokens: 10000,
			collectedInputTokens:    9000,
			existingCacheRead:       0,
			wantCacheRead:           0,
		},
		{
			name:                    "just over 10% diff should infer",
			messageStartInputTokens: 10000,
			collectedInputTokens:    8900,
			existingCacheRead:       0,
			wantCacheRead:           1100,
		},
		{
			name:                    "10k diff but ratio <10% should infer (diff > 10k takes precedence)",
			messageStartInputTokens: 150000,
			collectedInputTokens:    139000,
			existingCacheRead:       0,
			wantCacheRead:           11000,
		},
		{
			name:                    "diff exactly 10k with ratio <10% should not infer",
			messageStartInputTokens: 150000,
			collectedInputTokens:    140000,
			existingCacheRead:       0,
			wantCacheRead:           0,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ctx := &StreamContext{
				MessageStartInputTokens: tt.messageStartInputTokens,
				CollectedUsage: CollectedUsageData{
					InputTokens:          tt.collectedInputTokens,
					CacheReadInputTokens: tt.existingCacheRead,
				},
			}

			inferImplicitCacheRead(ctx, false)

			if ctx.CollectedUsage.CacheReadInputTokens != tt.wantCacheRead {
				t.Errorf("CacheReadInputTokens = %d, want %d",
					ctx.CollectedUsage.CacheReadInputTokens, tt.wantCacheRead)
			}
		})
	}
}

// TestPatchTokensInEventWithCache 测试带缓存推断的事件修补
func TestPatchTokensInEventWithCache(t *testing.T) {
	extractCacheRead := func(t *testing.T, event string) float64 {
		t.Helper()
		for _, line := range strings.Split(event, "\n") {
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			var data map[string]interface{}
			if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &data); err != nil {
				continue
			}
			if usage, ok := data["usage"].(map[string]interface{}); ok {
				if v, ok := usage["cache_read_input_tokens"].(float64); ok {
					return v
				}
			}
		}
		return 0
	}

	t.Run("should write inferred cache_read when not present", func(t *testing.T) {
		event := "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":20000,\"output_tokens\":100}}\n\n"
		patched := PatchTokensInEventWithCache(event, 20000, 100, 80000, true, false, false)
		got := extractCacheRead(t, patched)
		if got != 80000 {
			t.Errorf("expected cache_read_input_tokens=80000, got %v", got)
		}
	})

	t.Run("should not overwrite existing cache_read", func(t *testing.T) {
		event := "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":20000,\"output_tokens\":100,\"cache_read_input_tokens\":50000}}\n\n"
		patched := PatchTokensInEventWithCache(event, 20000, 100, 80000, true, false, false)
		got := extractCacheRead(t, patched)
		if got != 50000 {
			t.Errorf("expected cache_read_input_tokens=50000 (unchanged), got %v", got)
		}
	})

	t.Run("should not write when inferredCacheRead is 0", func(t *testing.T) {
		event := "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":20000,\"output_tokens\":100}}\n\n"
		patched := PatchTokensInEventWithCache(event, 20000, 100, 0, false, false, false)
		got := extractCacheRead(t, patched)
		if got != 0 {
			t.Errorf("expected cache_read_input_tokens=0, got %v", got)
		}
	})

	t.Run("should not overwrite explicit zero from upstream", func(t *testing.T) {
		// 上游显式返回 cache_read_input_tokens: 0 表示"明确无缓存"，不应被推断值覆盖
		event := "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":20000,\"output_tokens\":100,\"cache_read_input_tokens\":0}}\n\n"
		patched := PatchTokensInEventWithCache(event, 20000, 100, 80000, true, false, false)
		got := extractCacheRead(t, patched)
		if got != 0 {
			t.Errorf("expected cache_read_input_tokens=0 (explicit zero preserved), got %v", got)
		}
	})
}

func TestPreflightStreamEvents_ToolUseNotEmpty(t *testing.T) {
	// 模拟纯 tool_use 响应：message_start → content_block_start(tool_use) → ... → message_stop
	// 这类响应没有 delta.text，但不应被判定为空响应
	events := []string{
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":478,\"output_tokens\":1}}}\n\n",
		"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_test\",\"name\":\"Bash\",\"input\":{}}}\n\n",
		"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\\\"ls\\\"}\"}}\n\n",
		"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
		"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"input_tokens\":2559,\"output_tokens\":23}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	}

	eventChan := make(chan string, len(events))
	errChan := make(chan error)
	for _, e := range events {
		eventChan <- e
	}
	close(eventChan)
	close(errChan)

	result := PreflightStreamEvents(eventChan, errChan)
	if result.IsEmpty {
		t.Errorf("tool_use response should NOT be detected as empty, got IsEmpty=true (buffered %d events)", len(result.BufferedEvents))
	}
}

func TestPreflightStreamEvents_ThinkingStartOnlyIsEmpty(t *testing.T) {
	// 模拟仅有 thinking start 但没有 thinking delta 的响应
	events := []string{
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":100,\"output_tokens\":1}}}\n\n",
		"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	}

	eventChan := make(chan string, len(events))
	errChan := make(chan error)
	for _, e := range events {
		eventChan <- e
	}
	close(eventChan)
	close(errChan)

	result := PreflightStreamEvents(eventChan, errChan)
	if !result.IsEmpty {
		t.Errorf("thinking start-only response should be detected as empty, got IsEmpty=false")
	}
}

func TestPreflightStreamEvents_ThinkingDeltaNotEmpty(t *testing.T) {
	events := []string{
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":100,\"output_tokens\":1}}}\n\n",
		"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
		"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"analysis\"}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	}

	eventChan := make(chan string, len(events))
	errChan := make(chan error)
	for _, e := range events {
		eventChan <- e
	}
	close(eventChan)
	close(errChan)

	result := PreflightStreamEvents(eventChan, errChan)
	if result.IsEmpty {
		t.Errorf("thinking delta response should NOT be detected as empty, got IsEmpty=true")
	}
}

func TestPreflightStreamEvents_TrueEmptyStillDetected(t *testing.T) {
	// 真正的空响应：有 message_start 和 message_stop 但没有任何 content block
	events := []string{
		"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"usage\":{\"input_tokens\":100,\"output_tokens\":0}}}\n\n",
		"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
	}

	eventChan := make(chan string, len(events))
	errChan := make(chan error)
	for _, e := range events {
		eventChan <- e
	}
	close(eventChan)
	close(errChan)

	result := PreflightStreamEvents(eventChan, errChan)
	if !result.IsEmpty {
		t.Errorf("truly empty response should be detected as empty, got IsEmpty=false")
	}
	if result.Diagnostic == "" {
		t.Fatal("expected diagnostic for empty preflight result")
	}
}

func TestPreflightStreamEvents_UnknownEventTypeRecordedInDiagnostic(t *testing.T) {
	eventChan := make(chan string, 2)
	errChan := make(chan error, 1)

	eventChan <- "event: weird\ndata: {\"type\":\"custom.semantic.delta\",\"foo\":\"bar\"}\n\n"
	close(eventChan)
	close(errChan)

	result := PreflightStreamEvents(eventChan, errChan)
	if !result.IsEmpty {
		t.Fatal("expected stream with only unknown event and no semantic content to be empty")
	}
	if result.UnknownEventType != "custom.semantic.delta" {
		t.Fatalf("UnknownEventType = %q, want %q", result.UnknownEventType, "custom.semantic.delta")
	}
	if !strings.Contains(result.Diagnostic, "custom.semantic.delta") {
		t.Fatalf("Diagnostic = %q, want it to mention unknown event type", result.Diagnostic)
	}
}

func TestPreflightStreamEvents_ToolUseStopReasonWithoutContentBlockStillNotEmpty(t *testing.T) {
	eventChan := make(chan string, 4)
	errChan := make(chan error, 1)

	events := []string{
		"event: message_start\ndata:{\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n",
		"event: message_delta\ndata:{\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}\n\n",
		"event: message_stop\ndata:{\"type\":\"message_stop\"}\n\n",
	}
	for _, e := range events {
		eventChan <- e
	}
	close(eventChan)
	close(errChan)

	result := PreflightStreamEvents(eventChan, errChan)
	if result.IsEmpty {
		t.Fatalf("tool_use stop_reason should NOT be detected as empty")
	}
}

func TestHasClaudeSemanticContent(t *testing.T) {
	tests := []struct {
		name  string
		event string
		want  bool
	}{
		{
			name:  "tool_use content block",
			event: "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_test\",\"name\":\"Bash\",\"input\":{}}}\n\n",
			want:  true,
		},
		{
			name:  "thinking content block",
			event: "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
			want:  false,
		},
		{
			name:  "redacted_thinking content block",
			event: "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"redacted_thinking\",\"thinking\":\"\"}}\n\n",
			want:  false,
		},
		{
			name:  "server_tool_use content block",
			event: "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"srvtoolu_test\",\"name\":\"web_search\"}}\n\n",
			want:  true,
		},
		{
			name:  "text content block - not non-text",
			event: "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
			want:  false,
		},
		{
			name:  "message_start - not content block",
			event: "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\"}}\n\n",
			want:  false,
		},
		{
			name:  "content_block_delta input_json_delta counts as non-text",
			event: "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
			want:  true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := HasClaudeSemanticContent(tt.event)
			if got != tt.want {
				t.Errorf("HasClaudeSemanticContent() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestEnsureMessageDeltaUsage(t *testing.T) {
	extractUsage := func(t *testing.T, event string) (int, int) {
		t.Helper()
		for _, line := range strings.Split(event, "\n") {
			if !strings.HasPrefix(line, "data: ") {
				continue
			}

			var data map[string]interface{}
			if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &data); err != nil {
				t.Fatalf("failed to unmarshal event data: %v", err)
			}
			if data["type"] != "message_delta" {
				continue
			}

			usage, ok := data["usage"].(map[string]interface{})
			if !ok {
				t.Fatalf("usage field missing in message_delta")
			}

			inputTokens, _ := usage["input_tokens"].(float64)
			outputTokens, _ := usage["output_tokens"].(float64)
			return int(inputTokens), int(outputTokens)
		}

		t.Fatalf("message_delta event not found")
		return 0, 0
	}

	t.Run("add usage when missing", func(t *testing.T) {
		event := "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n"
		patched := EnsureMessageDeltaUsage(event, 123, 7)

		in, out := extractUsage(t, patched)
		if in != 123 || out != 7 {
			t.Fatalf("expected usage input=123 output=7, got input=%d output=%d", in, out)
		}
	})

	t.Run("keep existing usage", func(t *testing.T) {
		event := "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}\n\n"
		patched := EnsureMessageDeltaUsage(event, 999, 888)

		in, out := extractUsage(t, patched)
		if in != 10 || out != 2 {
			t.Fatalf("expected existing usage to be kept, got input=%d output=%d", in, out)
		}
	})
}

func TestProcessStreamEvent_MessageStopInjectsUsageWhenMessageDeltaMissing(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	c.Request = httptest.NewRequest(http.MethodPost, "/v1/messages", strings.NewReader(`{}`))

	flusher, ok := c.Writer.(http.Flusher)
	if !ok {
		t.Fatalf("response writer does not implement http.Flusher")
	}

	ctx := &StreamContext{
		ContentBlockTypes: make(map[int]string),
		HasUsage:          true, // message_start 已经有 usage
		LogBuffer:         NewLimitedLogBuffer(MaxUpstreamResponseLogBytes),
	}
	envCfg := &config.EnvConfig{LogLevel: "info"}

	messageStopEvent := "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
	ProcessStreamEvent(c, c.Writer, flusher, messageStopEvent, ctx, envCfg, []byte(`{"messages":[{"role":"user","content":"hello"}]}`))

	body := w.Body.String()
	if !strings.Contains(body, "event: message_delta") {
		t.Fatalf("expected injected message_delta usage event before message_stop, body=%s", body)
	}
	if !strings.Contains(body, "\"usage\"") {
		t.Fatalf("expected injected usage field, body=%s", body)
	}
	if !strings.Contains(body, "event: message_stop") {
		t.Fatalf("expected message_stop event to be forwarded, body=%s", body)
	}
}

func TestDetectStreamBlacklistError_BalanceMessages(t *testing.T) {
	tests := []struct {
		name        string
		event       string
		wantReason  string
		wantMessage string
	}{
		{
			name: "nested error message semantic balance",
			event: `event: error
data: {"type":"error","error":{"type":"new_api_error","message":"预扣费额度失败, 用户剩余额度: ¥0.053950, 需要预扣费额度: ¥0.191160"}}

`,
			wantReason:  "insufficient_balance",
			wantMessage: "预扣费额度失败, 用户剩余额度: ¥0.053950, 需要预扣费额度: ¥0.191160",
		},
		{
			name: "string error field semantic balance",
			event: `event: error
data: {"type":"error","error":"API Key额度不足，请访问https://right.codes查看详情"}

`,
			wantReason:  "insufficient_balance",
			wantMessage: "API Key额度不足，请访问https://right.codes查看详情",
		},
		{
			name: "top level message semantic balance",
			event: `event: error
data: {"type":"error","message":"API Key额度不足，请访问https://right.codes查看详情"}

`,
			wantReason:  "insufficient_balance",
			wantMessage: "API Key额度不足，请访问https://right.codes查看详情",
		},
		{
			name: "401 nested token status exhausted balance",
			event: `event: error
data: {"type":"error","error":{"type":"new_api_error","message":"该令牌额度已用尽 TokenStatusExhausted[sk-duK***qqX]"}}

`,
			wantReason:  "insufficient_balance",
			wantMessage: "该令牌额度已用尽 TokenStatusExhausted[sk-duK***qqX]",
		},
		{
			name: "401 top level balance exhausted message",
			event: `event: error
data: {"type":"error","message":"账户余额已用尽，请充值"}

`,
			wantReason:  "insufficient_balance",
			wantMessage: "账户余额已用尽，请充值",
		},
		{
			name: "nested daily limit exceeded code",
			event: `event: error
data: {"type":"error","error":{"code":"DAILY_LIMIT_EXCEEDED","message":"daily usage limit exceeded"}}

`,
			wantReason:  "insufficient_balance",
			wantMessage: "daily usage limit exceeded",
		},
		{
			name: "top level usage limit exceeded code",
			event: `event: error
data: {"type":"error","code":"USAGE_LIMIT_EXCEEDED","message":"error: code=429 reason=\"DAILY_LIMIT_EXCEEDED\" message=\"daily usage limit exceeded\" metadata=map[]"}

`,
			wantReason:  "insufficient_balance",
			wantMessage: "error: code=429 reason=\"DAILY_LIMIT_EXCEEDED\" message=\"daily usage limit exceeded\" metadata=map[]",
		},
		{
			name: "string error field invalid api key",
			event: `event: error
data: {"type":"error","error":"无效的API Key"}

`,
			wantReason:  "authentication_error",
			wantMessage: "无效的API Key",
		},
		{
			name: "nested permission message without type",
			event: `event: error
data: {"type":"error","error":{"message":"permission denied for this resource"}}

`,
			wantReason:  "permission_error",
			wantMessage: "permission denied for this resource",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotReason, gotMessage := DetectStreamBlacklistError(tt.event)
			if gotReason != tt.wantReason || gotMessage != tt.wantMessage {
				t.Fatalf("DetectStreamBlacklistError() = (%q, %q), want (%q, %q)", gotReason, gotMessage, tt.wantReason, tt.wantMessage)
			}
		})
	}
}
