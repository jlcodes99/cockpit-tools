package main

import (
	"bytes"
	"encoding/base64"
	"testing"

	"github.com/tidwall/gjson"
)

const thirdPartyReasoningItem = `{"type":"reasoning","id":"755eea76-ad25-4cb6-aec3-279d11c25848","status":"completed","content":[{"type":"reasoning_text","text":"先分析问题"}],"summary":[],"encrypted_content":"sig-0"}`

// validGPTReasoningTestSignature 生成符合 internal/signature 校验的官方格式签名。
func validGPTReasoningTestSignature() string {
	payload := make([]byte, 1+8+16+16+32)
	payload[0] = 0x80
	for i := 9; i < len(payload); i++ {
		payload[i] = byte(i)
	}
	return base64.RawURLEncoding.EncodeToString(payload)
}

func framePayload(t *testing.T, frame []byte, path string) gjson.Result {
	t.Helper()
	payload, ok := responsesSSEDataPayload(frame)
	if !ok {
		t.Fatalf("frame has no data payload: %q", string(frame))
	}
	return gjson.GetBytes(payload, path)
}

func TestNormalizeResponsesReasoningContentFrameMovesTextToSummary(t *testing.T) {
	frame := []byte("event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"item\":" + thirdPartyReasoningItem + ",\"output_index\":0}\n\n")

	got := normalizeResponsesReasoningContentSSE(frame)

	item := framePayload(t, got, "item")
	if item.Get("content").Raw != "[]" {
		t.Fatalf("content = %s, want []", item.Get("content").Raw)
	}
	summary := item.Get("summary")
	if !summary.IsArray() || len(summary.Array()) != 1 {
		t.Fatalf("summary = %s, want single summary part", summary.Raw)
	}
	if got := summary.Array()[0].Get("type").String(); got != "summary_text" {
		t.Fatalf("summary part type = %q, want summary_text", got)
	}
	if got := summary.Array()[0].Get("text").String(); got != "先分析问题" {
		t.Fatalf("summary text = %q, want moved reasoning text", got)
	}
	if !bytes.HasSuffix(got, []byte("\n\n")) {
		t.Fatalf("frame lost trailing delimiter: %q", string(got))
	}
}

func TestNormalizeResponsesReasoningContentFrameKeepsExistingSummary(t *testing.T) {
	item := `{"type":"reasoning","id":"rs_1","content":[{"type":"reasoning_text","text":"细节"}],"summary":[{"type":"summary_text","text":"结论"}]}`
	frame := []byte("data: {\"type\":\"response.output_item.done\",\"item\":" + item + "}\n\n")

	got := normalizeResponsesReasoningContentSSE(frame)

	summary := framePayload(t, got, "item.summary")
	if len(summary.Array()) != 1 || summary.Array()[0].Get("text").String() != "结论" {
		t.Fatalf("summary = %s, want original summary kept", summary.Raw)
	}
	if content := framePayload(t, got, "item.content").Raw; content != "[]" {
		t.Fatalf("content = %s, want []", content)
	}
}

func TestNormalizeResponsesReasoningContentForCompletedResponse(t *testing.T) {
	payload := []byte(`{"type":"response.completed","response":{"id":"resp_1","output":[` + thirdPartyReasoningItem + `,{"type":"message","role":"assistant","content":[{"type":"output_text","text":"2"}]}]}}`)

	got := normalizeResponsesReasoningContentBody(payload)

	output := gjson.GetBytes(got, "response.output")
	if len(output.Array()) != 2 {
		t.Fatalf("output = %s, want untouched item count", output.Raw)
	}
	if content := output.Array()[0].Get("content").Raw; content != "[]" {
		t.Fatalf("reasoning content = %s, want []", content)
	}
	if text := output.Array()[0].Get("summary.0.text").String(); text != "先分析问题" {
		t.Fatalf("reasoning summary text = %q, want moved text", text)
	}
	if text := output.Array()[1].Get("content.0.text").String(); text != "2" {
		t.Fatalf("message content = %q, want untouched assistant text", text)
	}
}

func TestNormalizeResponsesReasoningContentForNonStreamBody(t *testing.T) {
	payload := []byte(`{"id":"resp_1","output":[` + thirdPartyReasoningItem + `]}`)

	got := normalizeResponsesReasoningContentBody(payload)

	if content := gjson.GetBytes(got, "output.0.content").Raw; content != "[]" {
		t.Fatalf("content = %s, want []", content)
	}
	if text := gjson.GetBytes(got, "output.0.summary.0.text").String(); text != "先分析问题" {
		t.Fatalf("summary text = %q, want moved text", text)
	}
}

func TestNormalizeResponsesReasoningContentKeepsOfficialShapesUntouched(t *testing.T) {
	frame := []byte("data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"id\":\"rs_1\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"结论\"}],\"encrypted_content\":\"" + validGPTReasoningTestSignature() + "\"}}\n\n")
	body := []byte(`{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"reasoning_text is not a part type"}]}]}`)

	if got := normalizeResponsesReasoningContentSSE(frame); !bytes.Equal(got, frame) {
		t.Fatalf("official frame changed: %q", string(got))
	}
	if got := normalizeResponsesReasoningContentBody(body); !bytes.Equal(got, body) {
		t.Fatalf("body without reasoning part changed: %q", string(got))
	}
}

func TestNormalizeResponsesReasoningContentDropsThirdPartyEncryptedContent(t *testing.T) {
	frame := []byte("data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"id\":\"c2512c77-5683-49f3-90f1-b27543f0cc1c\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"你好\"}],\"content\":[],\"encrypted_content\":\"bf0ff0e2-ffac-4bf0-a39a-80b340896e1c-0\"}}\n\n")

	got := normalizeResponsesReasoningContentSSE(frame)

	item := framePayload(t, got, "item")
	if item.Get("encrypted_content").Exists() {
		t.Fatalf("encrypted_content = %q, want dropped", item.Get("encrypted_content").Raw)
	}
	if text := item.Get("summary.0.text").String(); text != "你好" {
		t.Fatalf("summary text = %q, want kept", text)
	}
}

func TestNormalizeResponsesReasoningContentKeepsOfficialEncryptedContent(t *testing.T) {
	signature := validGPTReasoningTestSignature()
	payload := []byte(`{"type":"response.completed","response":{"id":"resp_1","output":[{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"结论"}],"content":[],"encrypted_content":"` + signature + `"}]}}`)

	got := normalizeResponsesReasoningContentBody(payload)

	if got := gjson.GetBytes(got, "response.output.0.encrypted_content").String(); got != signature {
		t.Fatalf("encrypted_content = %q, want official signature kept", got)
	}
}

func TestNormalizeResponsesReasoningContentDropsEncryptedContentAlongsideReasoningText(t *testing.T) {
	payload := []byte(`{"output":[` + thirdPartyReasoningItem + `]}`)

	got := normalizeResponsesReasoningContentBody(payload)

	item := gjson.GetBytes(got, "output.0")
	if item.Get("encrypted_content").Exists() {
		t.Fatalf("encrypted_content = %q, want dropped", item.Get("encrypted_content").Raw)
	}
	if content := item.Get("content").Raw; content != "[]" {
		t.Fatalf("content = %s, want []", content)
	}
	if text := item.Get("summary.0.text").String(); text != "先分析问题" {
		t.Fatalf("summary text = %q, want moved text", text)
	}
}

func TestNormalizeResponsesReasoningContentHandlesPlainStringContent(t *testing.T) {
	payload := []byte(`{"type":"reasoning","id":"r1","content":[{"type":"reasoning_text","text":"历史推理正文"}],"summary":[]}`)

	got := normalizeResponsesReasoningContentBody([]byte(`{"item":` + string(payload) + `}`))

	if content := gjson.GetBytes(got, "item.content").Raw; content != "[]" {
		t.Fatalf("content = %s, want []", content)
	}
	if text := gjson.GetBytes(got, "item.summary.0.text").String(); text != "历史推理正文" {
		t.Fatalf("summary text = %q, want moved text", text)
	}
}
