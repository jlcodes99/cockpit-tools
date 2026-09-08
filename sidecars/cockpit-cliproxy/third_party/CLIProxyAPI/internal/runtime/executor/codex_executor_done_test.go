package executor

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	_ "github.com/router-for-me/CLIProxyAPI/v7/internal/translator"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

func TestCodexExecutorExecuteDoneTerminalStatus(t *testing.T) {
	for _, test := range []struct {
		name, eventType, state string
		wantHTTP               int
	}{
		{"done_completed", "response.done", "completed", 200},
		{"done_incomplete", "response.done", "incomplete", 200},
		{"done_failed", "response.done", "failed", 502},
		{"done_cancelled", "response.done", "cancelled", 408},
		{"done_missing_status", "response.done", "", 408},
		{"existing_completed", "response.completed", "completed", 200},
		{"existing_incomplete", "response.incomplete", "incomplete", 200},
		{"existing_failed", "response.failed", "failed", 502},
	} {
		t.Run(test.name, func(t *testing.T) {
			response := map[string]any{
				"id": "resp_synthetic", "model": "gpt-5.4", "output": []any{},
				"usage": map[string]int{"input_tokens": 7, "output_tokens": 3, "total_tokens": 10},
			}
			if test.state != "" {
				response["status"] = test.state
			}
			if test.state == "failed" {
				response["error"] = map[string]string{"type": "upstream_error", "code": "synthetic_failure", "message": "Synthetic failure."}
			}
			if test.state == "incomplete" {
				response["incomplete_details"] = map[string]string{"reason": "max_output_tokens"}
			}
			event, err := json.Marshal(map[string]any{"type": test.eventType, "response": response})
			if err != nil {
				t.Fatal(err)
			}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.Header().Set("Content-Type", "text/event-stream")
				_, _ = io.WriteString(w, "data: "+string(event)+"\n\n")
			}))
			defer server.Close()
			executor := NewCodexExecutor(&config.Config{})
			auth := &cliproxyauth.Auth{Attributes: map[string]string{"base_url": server.URL, "api_key": "synthetic-test-only"}}
			result, err := executor.Execute(context.Background(), auth, cliproxyexecutor.Request{
				Model: "gpt-5.4", Payload: []byte(`{"model":"gpt-5.4","input":"Synthetic test."}`),
			}, cliproxyexecutor.Options{SourceFormat: sdktranslator.FormatOpenAIResponse, Stream: false})
			if test.wantHTTP >= 400 {
				var status interface{ StatusCode() int }
				if err == nil || !errors.As(err, &status) || status.StatusCode() != test.wantHTTP {
					t.Fatalf("error status mismatch: want %d; err=%v", test.wantHTTP, err)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if got := gjson.GetBytes(result.Payload, "status").String(); got != test.state {
				t.Fatalf("status=%q, want %q", got, test.state)
			}
			if gjson.GetBytes(result.Payload, "id").String() != "resp_synthetic" || gjson.GetBytes(result.Payload, "usage.output_tokens").Int() != 3 {
				t.Fatal("terminal response identity or usage changed")
			}
			if test.state == "incomplete" && gjson.GetBytes(result.Payload, "incomplete_details.reason").String() != "max_output_tokens" {
				t.Fatal("incomplete reason changed")
			}
		})
	}
}
