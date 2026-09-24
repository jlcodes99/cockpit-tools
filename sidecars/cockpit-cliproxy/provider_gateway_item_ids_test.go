package main

import (
	"strings"
	"testing"

	"github.com/tidwall/gjson"
)

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

func TestProviderGatewayItemIDRewriterPreservesPrePrefixedIDAcrossStreamEvents(t *testing.T) {
	rewriter := newProviderGatewayItemIDRewriter()
	added := []byte(`data: {"type":"response.output_item.added","output_index":0,"item":{"id":"msg_openrouter_123","type":"message","role":"assistant","content":[]}}`)
	done := []byte(`data: {"type":"response.output_item.done","output_index":0,"item":{"id":"msg_openrouter_123","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}]}}`)

	gotAdded := rewriter.RewriteSSEFrame(added)
	idAdded := framePayload(t, gotAdded, "item.id").String()
	if idAdded != "msg_openrouter_123" {
		t.Fatalf("added id = %q, want %q", idAdded, "msg_openrouter_123")
	}

	gotDone := rewriter.RewriteSSEFrame(done)
	idDone := framePayload(t, gotDone, "item.id").String()
	if idDone != "msg_openrouter_123" {
		t.Fatalf("done id = %q, want same id %q; got frame=%s", idDone, "msg_openrouter_123", gotDone)
	}
}
