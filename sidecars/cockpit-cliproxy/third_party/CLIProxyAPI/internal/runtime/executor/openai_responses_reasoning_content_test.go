package executor

import (
	"context"
	"testing"

	"github.com/tidwall/gjson"
)

func TestNormalizeNonOfficialCodexReasoningItemsDropsThirdPartyReasoningItem(t *testing.T) {
	body := []byte(`{"input":[` +
		`{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]},` +
		`{"id":"ee770f0e-7df1-4928-92e5-511cadbc145d","type":"reasoning","summary":[],"content":[{"type":"reasoning_text","text":"thinking"}],"encrypted_content":"ee770f0e-7df1-4928-92e5-511cadbc145d-0"},` +
		`{"type":"function_call","id":"fc_1","call_id":"call_1","name":"exec_command","arguments":"{}"}` +
		`]}`)

	got := normalizeNonOfficialCodexReasoningItems(context.Background(), "codex executor", body)

	if count := gjson.GetBytes(got, "input.#").Int(); count != 2 {
		t.Fatalf("input item count = %d, want 2: %s", count, got)
	}
	if itemType := gjson.GetBytes(got, "input.0.type").String(); itemType != "message" {
		t.Fatalf("input.0.type = %q, want message", itemType)
	}
	if itemType := gjson.GetBytes(got, "input.1.type").String(); itemType != "function_call" {
		t.Fatalf("input.1.type = %q, want function_call", itemType)
	}
	if gjson.GetBytes(got, "input.1.call_id").String() != "call_1" {
		t.Fatalf("function_call item was not preserved: %s", got)
	}
}

func TestNormalizeNonOfficialCodexReasoningItemsClearsContentWithOfficialSignature(t *testing.T) {
	encryptedContent := validCodexReasoningEncryptedContentForTest()
	body := []byte(`{"input":[` +
		`{"id":"rs_hybrid","type":"reasoning","summary":[{"type":"summary_text","text":"done"}],"content":[{"type":"reasoning_text","text":"thinking"}],"encrypted_content":"` + encryptedContent + `"}` +
		`]}`)

	got := normalizeNonOfficialCodexReasoningItems(context.Background(), "codex executor", body)

	if count := gjson.GetBytes(got, "input.#").Int(); count != 1 {
		t.Fatalf("input item count = %d, want 1: %s", count, got)
	}
	if count := gjson.GetBytes(got, "input.0.content.#").Int(); count != 0 {
		t.Fatalf("reasoning content = %s, want empty array", gjson.GetBytes(got, "input.0.content").Raw)
	}
	if got := gjson.GetBytes(got, "input.0.encrypted_content").String(); got != encryptedContent {
		t.Fatalf("encrypted_content = %q, want preserved", got)
	}
	if got := gjson.GetBytes(got, "input.0.summary.#").Int(); got != 1 {
		t.Fatalf("summary item count = %d, want 1", got)
	}
}

func TestNormalizeNonOfficialCodexReasoningItemsClearsContentWithSummary(t *testing.T) {
	body := []byte(`{"input":[` +
		`{"type":"reasoning","summary":[{"type":"summary_text","text":"kept"}],"content":[{"type":"reasoning_text","text":"thinking"}]}` +
		`]}`)

	got := normalizeNonOfficialCodexReasoningItems(context.Background(), "codex executor", body)

	if count := gjson.GetBytes(got, "input.#").Int(); count != 1 {
		t.Fatalf("input item count = %d, want 1: %s", count, got)
	}
	if count := gjson.GetBytes(got, "input.0.content.#").Int(); count != 0 {
		t.Fatalf("reasoning content = %s, want empty array", gjson.GetBytes(got, "input.0.content").Raw)
	}
}

func TestNormalizeNonOfficialCodexReasoningItemsLeavesOfficialHistoryUntouched(t *testing.T) {
	body := []byte(`{"store":false,"input":[` +
		`{"id":"rs_official","type":"reasoning","summary":[],"content":[],"encrypted_content":"` + validCodexReasoningEncryptedContentForTest() + `"},` +
		`{"id":"rs_no_content","type":"reasoning","summary":[{"type":"summary_text","text":"s"}]},` +
		`{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}` +
		`]}`)

	got := normalizeNonOfficialCodexReasoningItems(context.Background(), "codex executor", body)

	if string(got) != string(body) {
		t.Fatalf("official history changed:\nwant %s\n got %s", body, got)
	}
}

func TestNormalizeNonOfficialCodexReasoningItemsLeavesUnsupportedPayloadsUnchanged(t *testing.T) {
	cases := [][]byte{
		[]byte(`{}`),
		[]byte(`{"input":"not-an-array"}`),
		[]byte(`{"input":[]}`),
	}
	for _, body := range cases {
		got := normalizeNonOfficialCodexReasoningItems(context.Background(), "codex executor", body)
		if string(got) != string(body) {
			t.Fatalf("body changed:\nwant %s\n got %s", body, got)
		}
	}
}
