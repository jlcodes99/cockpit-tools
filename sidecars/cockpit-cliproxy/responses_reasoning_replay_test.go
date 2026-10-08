package main

import (
	"strings"
	"testing"

	"github.com/tidwall/gjson"
)

func TestIsDeepSeekResponsesGateway(t *testing.T) {
	cases := map[string]bool{
		"https://api.deepseek.com":            true,
		"https://api.deepseek.com/v1":         true,
		"http://api.deepseek.com":             true,
		"API.DEEPSEEK.COM":                    true,
		"https://gateway.deepseek.com/openai": true,
		"api.deepseek.com":                    true,
		"https://api.openai.com":              false,
		"https://api.example.com/deepseek":    false,
		"https://notdeepseek.com":             false,
		"https://api.deepseek.com.evil.test":  false,
		"":                                    false,
	}
	for rawURL, want := range cases {
		if got := isDeepSeekResponsesGateway(rawURL); got != want {
			t.Fatalf("isDeepSeekResponsesGateway(%q) = %v, want %v", rawURL, got, want)
		}
	}
}

func TestRestoreResponsesReasoningTextForReplayRestoresSanitizedItem(t *testing.T) {
	// 响应出口把正文搬到 summary 并清空 content；回放给 DeepSeek 时要还原回去，
	// 否则上游以 "The reasoning_text in the thinking mode must be passed back to the API" 拒绝。
	body := []byte(`{"model":"deepseek-v4-flash","input":[
		{"type":"message","role":"user","content":"继续"},
		{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"先分析问题"}],"content":[]},
		{"type":"message","role":"assistant","content":"好的"}
	]}`)

	got := restoreResponsesReasoningTextForReplay(body)
	content := gjson.GetBytes(got, "input.1.content")
	if !content.IsArray() || len(content.Array()) != 1 {
		t.Fatalf("reasoning content was not restored: %s", got)
	}
	if content.Array()[0].Get("type").String() != reasoningTextPartType {
		t.Fatalf("restored part type = %q, want %q: %s", content.Array()[0].Get("type").String(), reasoningTextPartType, got)
	}
	if content.Array()[0].Get("text").String() != "先分析问题" {
		t.Fatalf("restored text = %q: %s", content.Array()[0].Get("text").String(), got)
	}
	// 其余项保持不变。
	if gjson.GetBytes(got, "input.0.content").String() != "继续" || gjson.GetBytes(got, "input.2.content").String() != "好的" {
		t.Fatalf("unrelated items changed: %s", got)
	}
	// summary 保留，官方方向仍然可用。
	if gjson.GetBytes(got, "input.1.summary.0.text").String() != "先分析问题" {
		t.Fatalf("summary was dropped: %s", got)
	}
}

func TestRestoreResponsesReasoningTextForReplayJoinsMultipleSummaryParts(t *testing.T) {
	body := []byte(`{"input":[
		{"type":"reasoning","content":[],"summary":[{"type":"summary_text","text":"第一段"},{"type":"summary_text","text":"第二段"}]},
		{"type":"message","role":"assistant","content":"done"}
	]}`)

	got := restoreResponsesReasoningTextForReplay(body)
	content := gjson.GetBytes(got, "input.0.content").Array()
	// 合并成单个 reasoning_text part，片段之间用换行连接。
	if len(content) != 1 {
		t.Fatalf("content parts = %d, want 1: %s", len(content), got)
	}
	if content[0].Get("text").String() != "第一段\n第二段" {
		t.Fatalf("summary parts were not joined in order: %q", content[0].Get("text").String())
	}
}

func TestRestoreResponsesReasoningTextForReplayIsNoOpWithoutSummary(t *testing.T) {
	for _, body := range [][]byte{
		// 没有 summary 的官方加密推理项：原样透传。
		[]byte(`{"input":[{"type":"reasoning","content":[],"encrypted_content":"sig-0"}]}`),
		// 已经有正文的推理项：不覆盖。
		[]byte(`{"input":[{"type":"reasoning","content":[{"type":"reasoning_text","text":"已有"}],"summary":[{"type":"summary_text","text":"概要"}]}]}`),
		// 没有 input 的请求体。
		[]byte(`{"model":"deepseek-v4-flash","instructions":""}`),
	} {
		if got := restoreResponsesReasoningTextForReplay(body); string(got) != string(body) {
			t.Fatalf("body changed unexpectedly: %s", got)
		}
	}
}

func TestRestoreResponsesReasoningTextForReplayKeepsRequestBodyValid(t *testing.T) {
	body := []byte(`{"input":[{"type":"reasoning","content":[],"summary":[{"type":"summary_text","text":"含\"引号\"与\n换行"}]}]}`)
	got := restoreResponsesReasoningTextForReplay(body)
	if !gjson.ValidBytes(got) {
		t.Fatalf("result is not valid JSON: %s", got)
	}
	text := gjson.GetBytes(got, "input.0.content.0.text").String()
	if !strings.Contains(text, `"引号"`) || !strings.Contains(text, "\n") {
		t.Fatalf("text was not escaped correctly: %q", text)
	}
}
