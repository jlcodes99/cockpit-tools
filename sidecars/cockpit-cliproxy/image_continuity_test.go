package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"

	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestContinuityClientResponsesImagePoolKeepsConversationHistory(t *testing.T) {
	body := json.RawMessage(`{"input":[{"role":"user","content":"old question"},{"type":"reasoning","encrypted_content":"old-image-conversation-cipher"},{"role":"user","content":"draw it"}],"tools":[{"type":"image_generation"}],"tool_choice":{"type":"image_generation"}}`)
	var calls atomic.Int32
	worker := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		var req coreexecutor.ContinuityPrepareRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil || !req.AllowRebuild || string(req.Body) != string(body) {
			t.Errorf("Responses image history was lost: %s err=%v", req.Body, err)
		}
		_ = json.NewEncoder(w).Encode(map[string]any{"ticket": "image-conversation", "self_contained": true, "body": req.Body, "headers": map[string][]string{}})
	}))
	defer worker.Close()
	m := continuityManifestFixture(t)
	client, err := newContinuityClient(worker.URL, strings.Repeat("a", 64), m)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	m.continuityClient = client
	ctx := context.WithValue(context.Background(), clientAPIKeyContextKey, &apiKeySpec{ID: "legacy", Enabled: true})
	ctx = context.WithValue(ctx, requestKindContextKey, "text")
	ctx = withRequestContinuity(ctx, m, http.Header{"Thread-Id": {"original-image-conversation"}}, false)
	ctx = context.WithValue(ctx, requestKindContextKey, "image_generation")
	prepared, err := client.Prepare(ctx, coreexecutor.ContinuityPrepareRequest{Body: body, AllowRebuild: true})
	if err != nil || prepared.Ticket == "" || calls.Load() != 1 {
		t.Fatal("OAuth Responses image source bypassed continuity", prepared, err, calls.Load())
	}
}

func TestImageAPIResponsesOpaqueHistoryIsNeverFlattened(t *testing.T) {
	var calls atomic.Int32
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		_, _ = io.WriteString(w, `{"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}]}`)
	}))
	defer upstream.Close()
	for _, tc := range []struct{ name, input, extra string }{
		{"reasoning", `[{"type":"reasoning","encrypted_content":"old-cipher"},{"role":"user","content":"draw"}]`, ""},
		{"tool-output", `[{"type":"function_call_output","call_id":"old-call","output":[{"type":"encrypted_content","encrypted_content":"old-output"}]},{"role":"user","content":"draw"}]`, ""},
		{"encrypted-arguments", `[{"type":"function_call","name":"tool","arguments":"{}","encrypted_function_args":["old-arguments"]},{"role":"user","content":"draw"}]`, ""},
		{"agent-message", `[{"type":"function_call","namespace":"collaboration","name":"send_message","arguments":"{\"message\":\"old-agent-cipher\"}"},{"role":"user","content":"draw"}]`, ""},
		{"item-reference", `[{"type":"item_reference","id":"old-item"},{"role":"user","content":"draw"}]`, ""},
		{"previous", `"draw"`, `,"previous_response_id":"old-response"`},
		{"conversation", `"draw"`, `,"conversation":"old-conversation"`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
			body := `{"model":"chat-model","input":` + tc.input + tc.extra + `,"tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation"}]}`
			response := imageAPITestRequest(s, "/v1/responses", body)
			if response.Code != http.StatusConflict || !strings.Contains(response.Body.String(), "continuation_not_supported") || calls.Load() != 0 {
				t.Fatalf("opaque history was flattened: status=%d calls=%d body=%s", response.Code, calls.Load(), response.Body.String())
			}
		})
	}
	// Explicit plaintext agent messages do not invent encrypted history.
	s := imageAPITestRelay(t, upstream.URL, []string{defaultImagesToolModel})
	response := imageAPITestRequest(s, "/v1/responses", `{"model":"chat-model","input":[{"type":"function_call","namespace":"collaboration","name":"send_message","encrypted_function_args":[],"arguments":"{\"message\":\"plain task\"}"},{"role":"user","content":"draw"}],"tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation"}]}`)
	if response.Code != http.StatusOK || calls.Load() != 1 {
		t.Fatalf("plaintext image conversion changed: status=%d calls=%d body=%s", response.Code, calls.Load(), response.Body.String())
	}
}
