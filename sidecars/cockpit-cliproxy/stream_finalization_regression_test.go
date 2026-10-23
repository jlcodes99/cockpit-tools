package main

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
)

func TestChatStreamTerminatesOnceAndOnlyAfterAllChoices(t *testing.T) {
	for _, tc := range []struct {
		name   string
		chunks []string
		done   int
	}{
		{"complete", []string{`{"choices":[{"index":0,"delta":{"content":"hello"}}]}`, `{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}`}, 1},
		{"tool", []string{`{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}`}, 1},
		{"already_done", []string{`{"choices":[{"index":0,"finish_reason":"stop"}]}`, "data: [DONE]\n\n", "[DONE]"}, 1},
		{"unfinished_choice", []string{`{"choices":[{"index":0,"finish_reason":"stop"},{"index":1,"delta":{"content":"partial"}}]}`}, 0},
		{"failed", []string{`{"choices":[{"index":0,"finish_reason":"stop"}]}`, `{"error":{"message":"failed"}}`}, 0},
		{"empty", nil, 0},
	} {
		t.Run(tc.name, func(t *testing.T) {
			f := newRelayStreamFramer(sdktranslator.FormatOpenAI, "/v1/chat/completions")
			var output bytes.Buffer
			for _, chunk := range tc.chunks {
				if err := f.Write(&output, []byte(chunk)); err != nil {
					t.Fatal(err)
				}
			}
			if err := f.Close(&output); err != nil {
				t.Fatal(err)
			}
			if err := f.Close(&output); err != nil {
				t.Fatal(err)
			}
			if got := strings.Count(output.String(), "data: [DONE]"); got != tc.done {
				t.Fatalf("done=%d want=%d: %s", got, tc.done, output.String())
			}
		})
	}
	for _, format := range []sdktranslator.Format{sdktranslator.FormatGemini, sdktranslator.FormatOpenAIResponse} {
		f := newRelayStreamFramer(format, "/v1/responses")
		var output bytes.Buffer
		_ = f.Write(&output, []byte(`{"choices":[{"index":0,"finish_reason":"stop"}]}`))
		_ = f.Close(&output)
		if strings.Contains(output.String(), "[DONE]") {
			t.Fatal("injected Chat marker into another protocol")
		}
	}
}

func TestOllamaFinalFrameDoesNotRepeatTextOrThinking(t *testing.T) {
	s := newOllamaStreamState("test")
	var text, thinking string
	for _, chunk := range []string{
		`{"choices":[{"delta":{"content":"hello","reasoning_content":"think"}}]}`,
		`{"choices":[{"delta":{"content":"世界","reasoning_content":"more","tool_calls":[{"index":0,"id":"call","function":{"name":"run","arguments":"{\"x\":"}}]}}]}`,
		`{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":4,"completion_tokens":6}}`,
	} {
		for _, event := range s.applyOpenAIChunk([]byte(chunk)) {
			m := event["message"].(gin.H)
			if v, ok := m["content"].(string); ok {
				text += v
			}
			if v, ok := m["thinking"].(string); ok {
				thinking += v
			}
		}
	}
	end := s.finalChunk()
	message := end["message"].(gin.H)
	if text != "hello世界" || thinking != "thinkmore" || message["content"] != "" || message["thinking"] != nil {
		t.Fatalf("duplicate/missing content: %q %q %+v", text, thinking, message)
	}
	encoded, err := json.Marshal(end)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(encoded), `"arguments":{"x":1}`) || end["done"] != true || end["eval_count"] != int64(6) {
		t.Fatalf("lost tool/usage completion: %s", encoded)
	}
}

func TestCanceledHTTP200IsRecordedAs499UnlessOutputCompleted(t *testing.T) {
	gin.SetMode(gin.TestMode)
	for _, complete := range []bool{false, true} {
		ctx, cancel := context.WithCancel(context.Background())
		c, _ := gin.CreateTestContext(httptest.NewRecorder())
		c.Request = httptest.NewRequest("POST", "/v1/responses", nil).WithContext(ctx)
		p := &requestPolicy{manifest: &manifest{}, emitter: &eventEmitter{}, tracker: newRequestUsageTracker()}
		observer := newRequestDiagnosticsObserver(p.manifest, "cancel", time.Now(), false, nil)
		attempt := coreexecutor.WithUpstreamAttemptTracker(ctx)
		observer.ExecutionStarted(attempt, coreexecutor.DiagnosticExecution{AuthID: "a", Model: "m"})
		observer.UpstreamRequest(attempt, coreexecutor.DiagnosticRequest{AuthID: "a", Body: []byte(`{"model":"m"}`)})
		if complete {
			observer.UpstreamCompletion(attempt, true)
			observer.recordResult(attempt, coreauth.Result{Success: true})
		}
		c.Request = c.Request.WithContext(coreexecutor.WithRequestDiagnosticsObserver(ctx, observer))
		c.Status(200)
		c.Writer.WriteHeaderNow()
		cancel()
		out := captureStdout(t, func() { p.emitRequestCompleted(c, "cancel", nil, "text", "m", time.Now()) })
		want := 499
		if complete {
			want = 200
		}
		var count int
		for _, line := range strings.Split(strings.TrimSpace(out), "\n") {
			var event map[string]any
			if json.Unmarshal([]byte(line), &event) != nil {
				continue
			}
			if event["type"] == "request_completed" || event["type"] == "usage" {
				count++
				if event["status"] != float64(want) {
					t.Fatalf("status: %s", line)
				}
				if event["type"] == "usage" && event["success"] != complete {
					t.Fatalf("success: %s", line)
				}
			}
		}
		if count != 2 {
			t.Fatalf("missing completion and usage: %s", out)
		}
	}
}

func TestCancellationAfterFailedAttemptPreservesTokensAndFinalCause(t *testing.T) {
	tracker := newRequestUsageTracker()
	tracker.record(usagePayload{RequestID: "retry-cancel", ErrorCategory: "server_is_overloaded", ErrorMessage: "earlier attempt", Usage: usageDetails{InputTokens: 4, OutputTokens: 6, TotalTokens: 10}})
	payload, ok := tracker.finalize("retry-cancel", usageFinalizeInput{status: 499, errorMessage: context.Canceled.Error()})
	if !ok || payload.Success || payload.Status != 499 || payload.ErrorCategory != "client_canceled" || payload.ErrorMessage != context.Canceled.Error() || payload.Usage.TotalTokens != 10 {
		t.Fatalf("wrong final cause/usage: %+v", payload)
	}
}

func TestWebsocketTransportCloseDoesNotInventCanceledInference(t *testing.T) {
	ctx, cancel := context.WithCancel(context.WithValue(context.Background(), websocketUsageContextKey, &websocketUsageSink{}))
	c, _ := gin.CreateTestContext(httptest.NewRecorder())
	c.Request = httptest.NewRequest("GET", "/v1/responses", nil).WithContext(ctx)
	c.Status(200)
	cancel()
	status, _ := requestCompletionStatus(c, "transport", newRequestUsageTracker())
	if status != 200 {
		t.Fatalf("websocket transport created extra failure: %d", status)
	}
}
