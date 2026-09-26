package main

import (
	"fmt"
	"strings"
	"testing"

	"github.com/tidwall/gjson"
)

func TestProviderGatewayItemIDRewriterKeepsIDAcrossStreamEvents(t *testing.T) {
	for _, tc := range []struct {
		name, itemType, id, prefix string
	}{
		{"valid message", "message", "msg_1", "msg_"},
		{"valid reasoning", "reasoning", "rs_1", "rs_"},
		{"valid function", "function_call", "fc_1", "fc_"},
		{"valid custom tool", "custom_tool_call", "ctc_1", "ctc_"},
		{"missing prefix", "message", "upstream_1", "msg_"},
		{"wrong prefix", "custom_tool_call", "fc_1", "ctc_"},
		{"long ID", "message", "msg_" + strings.Repeat("x", 80), "msg_"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			r := newProviderGatewayItemIDRewriter()
			item := fmt.Sprintf(`{"type":%q,"id":%q}`, tc.itemType, tc.id)
			added := r.RewritePayload([]byte(`{"type":"response.output_item.added","output_index":0,"item":` + item + `}`))
			want := gjson.GetBytes(added, "item.id").String()
			if !strings.HasPrefix(want, tc.prefix) || len([]rune(want)) > providerGatewayItemIDLimit {
				t.Fatalf("invalid normalized ID: %q", want)
			}
			if strings.HasPrefix(tc.id, tc.prefix) && len([]rune(tc.id)) <= providerGatewayItemIDLimit && want != tc.id {
				t.Fatalf("valid ID changed: %q -> %q", tc.id, want)
			}
			for _, event := range []struct{ payload, path string }{
				{fmt.Sprintf(`{"type":"response.output_text.delta","item_id":%q,"delta":"hello"}`, tc.id), "item_id"},
				{`{"type":"response.output_item.done","output_index":0,"item":` + item + `}`, "item.id"},
				{fmt.Sprintf(`{"type":"response.output_text.done","item_id":%q,"text":"hello"}`, tc.id), "item_id"},
				{`{"type":"response.completed","response":{"id":"resp_1","output":[` + item + `]}}`, "response.output.0.id"},
			} {
				got := r.RewritePayload([]byte(event.payload))
				if id := gjson.GetBytes(got, event.path).String(); id != want {
					t.Errorf("same item changed ID: want %q, got %q; event=%s", want, id, got)
				}
			}
		})
	}
}

func TestProviderGatewayItemIDRewriterKeepsCollidingIDsDistinct(t *testing.T) {
	r := newProviderGatewayItemIDRewriter()
	ids := map[string]string{}
	for _, eventType := range []string{"response.output_item.added", "response.output_item.done"} {
		for index, rawID := range []string{"msg_1", "1"} {
			payload := fmt.Sprintf(`{"type":%q,"output_index":%d,"item":{"type":"message","id":%q}}`, eventType, index, rawID)
			got := gjson.GetBytes(r.RewritePayload([]byte(payload)), "item.id").String()
			if previous, ok := ids[rawID]; ok && previous != got {
				t.Fatalf("ID changed for %q: %q -> %q", rawID, previous, got)
			}
			ids[rawID] = got
		}
	}
	if ids["msg_1"] == ids["1"] {
		t.Fatal("distinct upstream IDs must not be merged")
	}
}

func TestProviderGatewayItemIDRewriterRepairsWrongPrefix(t *testing.T) {
	rewriter := newProviderGatewayItemIDRewriter()
	added := []byte("event: response.output_item.added\n" +
		`data: {"type":"response.output_item.added","output_index":0,"item":{"id":"fc_de28d1b9-9d75-9d1f-a581-a32e81c8494a_0","type":"custom_tool_call","status":"completed","call_id":"call-a9610ddc","name":"apply_patch","input":"patch"}}`)

	got := rewriter.RewriteSSEFrame(added)
	if id := framePayload(t, got, "item.id").String(); id != "ctc_fc_de28d1b9-9d75-9d1f-a581-a32e81c8494a_0" {
		t.Fatalf("custom_tool_call id = %q, want ctc-prefixed id; frame=%s", id, got)
	}
	if !strings.Contains(string(got), "event: response.output_item.added") {
		t.Fatalf("event name dropped from frame: %s", got)
	}

	delta := []byte("event: response.custom_tool_call_input.done\n" +
		`data: {"type":"response.custom_tool_call_input.done","item_id":"fc_de28d1b9-9d75-9d1f-a581-a32e81c8494a_0","input":"patch"}`)
	gotDelta := rewriter.RewriteSSEFrame(delta)
	if id := framePayload(t, gotDelta, "item_id").String(); id != "ctc_fc_de28d1b9-9d75-9d1f-a581-a32e81c8494a_0" {
		t.Fatalf("item_id = %q, want the rewritten item id; frame=%s", id, gotDelta)
	}
}

func TestProviderGatewayItemIDRewriterAddsMissingPrefixes(t *testing.T) {
	rewriter := newProviderGatewayItemIDRewriter()
	frame := []byte(`data: {"type":"response.output_item.added","output_index":2,"item":{"id":"b45f0d9b-159f-4517-b432-7adf31be8189","type":"function_call","call_id":"call_00_CJdan5OoaUbJIts4Tf1y9990","name":"exec_command"}}`)

	got := rewriter.RewriteSSEFrame(frame)
	if id := framePayload(t, got, "item.id").String(); id != "fc_b45f0d9b-159f-4517-b432-7adf31be8189" {
		t.Fatalf("function_call id = %q, want fc-prefixed id; frame=%s", id, got)
	}
}

func TestProviderGatewayItemIDRewriterFallsBackToCallID(t *testing.T) {
	rewriter := newProviderGatewayItemIDRewriter()
	frame := []byte(`data: {"type":"response.output_item.done","output_index":0,"item":{"id":"","type":"custom_tool_call_output","call_id":"call-1","output":"ok"}}`)

	got := rewriter.RewriteSSEFrame(frame)
	if id := framePayload(t, got, "item.id").String(); id != "ctco_call-1" {
		t.Fatalf("custom_tool_call_output id = %q, want ctco_call-1; frame=%s", id, got)
	}
}

func TestProviderGatewayItemIDRewriterNormalizesCompletedOutput(t *testing.T) {
	rewriter := newProviderGatewayItemIDRewriter()
	payload := []byte(`{"type":"response.completed","response":{"id":"resp_1","output":[` +
		`{"id":"","type":"reasoning","summary":[]},` +
		`{"id":"fc_old","type":"custom_tool_call","call_id":"call_b","input":"patch"},` +
		`{"id":"same","type":"function_call","call_id":"call_a","arguments":"{}"},` +
		`{"id":"same","type":"function_call","call_id":"call_c","arguments":"{}"}` +
		`]}}`)

	got := rewriter.RewritePayload(payload)
	items := gjson.GetBytes(got, "response.output").Array()
	if len(items) != 4 {
		t.Fatalf("output length = %d, want 4; payload=%s", len(items), got)
	}
	if id := items[0].Get("id").String(); !strings.HasPrefix(id, "rs_") || id == "rs_" {
		t.Fatalf("reasoning id = %q, want synthesized rs-prefixed id", id)
	}
	if id := items[1].Get("id").String(); id != "ctc_fc_old" {
		t.Fatalf("custom_tool_call id = %q, want ctc_fc_old", id)
	}
	first := items[2].Get("id").String()
	second := items[3].Get("id").String()
	if !strings.HasPrefix(first, "fc_") || !strings.HasPrefix(second, "fc_") {
		t.Fatalf("function_call ids = %q/%q, want fc-prefixed ids", first, second)
	}
	if first != second {
		t.Fatalf("identical upstream ids must keep one shared mapping: %q vs %q", first, second)
	}
}

func TestProviderGatewayItemIDRewriterIsIdempotent(t *testing.T) {
	payload := []byte(`{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_legacy_0","type":"custom_tool_call","call_id":"call-1","input":"patch"}}`)
	first := newProviderGatewayItemIDRewriter().RewritePayload(payload)
	second := newProviderGatewayItemIDRewriter().RewritePayload(first)
	if string(first) != string(second) {
		t.Fatalf("rewrite is not idempotent: first=%s second=%s", first, second)
	}
}

func TestProviderGatewayItemIDRewriterLeavesOtherPayloadsUntouched(t *testing.T) {
	rewriter := newProviderGatewayItemIDRewriter()
	chunk := []byte(`data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"delta":{"content":"hi"}}]}`)
	if got := rewriter.RewriteSSEFrame(chunk); string(got) != string(chunk) {
		t.Fatalf("chat completion chunk changed: %s", got)
	}
	done := []byte("data: [DONE]\n\n")
	if got := rewriter.RewriteSSEFrame(done); string(got) != string(done) {
		t.Fatalf("[DONE] frame changed: %s", got)
	}
}
