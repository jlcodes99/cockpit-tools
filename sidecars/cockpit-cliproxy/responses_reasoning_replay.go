package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/url"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// 第三方 Responses 上游返回的推理正文在响应出口被改写成官方形状（见
// responses_reasoning_sanitize.go）：`content` 清空、正文搬到 `summary`，这样 Codex 客户端
// 落盘的历史才能同时喂给官方账号。
//
// 但 DeepSeek 的思考模式反过来要求回放时必须带上 `reasoning_text`：
//
//	400 {"error":{"message":"The `reasoning_text` in the thinking mode must be passed back to the API",...}}
//
// 于是同一个历史在两边的要求正好相反——官方只接受空 `content`，DeepSeek 要求 `content` 里有
// `reasoning_text`。这里在「发往 DeepSeek」的方向上把正文还原回去，让两边各拿到自己认的形状。
//
// 只在确实是 DeepSeek 官方 Responses 上游时才还原：其它第三方上游未见过该要求，而官方上游
// 明确拒绝这种形状，多写一步反而会把能用的链路弄坏。

// summaryTextPartType 是官方形状里承载推理摘要的 part 类型（正文被搬到那里）。
const summaryTextPartType = "summary_text"

var summaryTextPartMarker = []byte(summaryTextPartType)

// isDeepSeekResponsesGateway 判断网关地址是否为 DeepSeek 官方上游。
func isDeepSeekResponsesGateway(rawURL string) bool {
	trimmed := strings.TrimSpace(rawURL)
	if trimmed == "" {
		return false
	}
	parsed, err := url.Parse(trimmed)
	if err != nil {
		return false
	}
	host := strings.ToLower(parsed.Hostname())
	if host == "" {
		// 允许只配置主机名（如 `api.deepseek.com`）而没有 scheme。
		host = strings.ToLower(strings.TrimSuffix(strings.SplitN(trimmed, "/", 2)[0], "."))
	}
	return host == "deepseek.com" || strings.HasSuffix(host, ".deepseek.com")
}

// restoreResponsesReasoningTextForReplay 把官方形状的推理项还原成 DeepSeek 要求的形状。
//
// 只处理「`content` 为空、`summary` 里有正文」的推理项，其它项原样保留；不含 `summary_text`
// 的请求体直接返回，保持网关当前的字节级透传行为。
func restoreResponsesReasoningTextForReplay(body []byte) []byte {
	if len(body) == 0 || !bytes.Contains(body, summaryTextPartMarker) {
		return body
	}
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() {
		return body
	}

	changed := false
	input.ForEach(func(key, item gjson.Result) bool {
		index := int(key.Int())
		if strings.TrimSpace(item.Get("type").String()) != "reasoning" {
			return true
		}
		if content := item.Get("content"); content.IsArray() && len(content.Array()) > 0 {
			// 已经有正文（例如官方账号的加密推理项），不动。
			return true
		}
		summary := item.Get("summary")
		if !summary.IsArray() {
			return true
		}
		texts := make([]string, 0, len(summary.Array()))
		for _, part := range summary.Array() {
			text := part.Get("text").String()
			if strings.TrimSpace(text) == "" {
				continue
			}
			texts = append(texts, text)
		}
		if len(texts) == 0 {
			return true
		}
		// 合并成单个 reasoning_text：上游只校验正文是否存在，片段边界没有语义。
		payload, err := json.Marshal([]map[string]string{{
			"type": reasoningTextPartType,
			"text": strings.Join(texts, "\n"),
		}})
		if err != nil {
			return true
		}
		updated, err := sjson.SetRawBytes(
			body,
			fmt.Sprintf("input.%d.content", index),
			payload,
		)
		if err != nil {
			return true
		}
		body = updated
		changed = true
		return true
	})
	if !changed {
		return body
	}
	return body
}
