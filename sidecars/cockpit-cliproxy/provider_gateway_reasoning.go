package main

import (
	"encoding/json"
	"fmt"
	"net/url"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// providerGatewayRestoresReasoningText reports whether the upstream verifies that every replayed
// reasoning item carries `reasoning_text`.
func providerGatewayRestoresReasoningText(gateway *providerGatewaySpec) bool {
	if gateway == nil {
		return false
	}
	parsed, err := url.Parse(strings.TrimSpace(gateway.BaseURL))
	if err != nil {
		return false
	}
	return strings.EqualFold(parsed.Hostname(), "api.deepseek.com")
}

// providerGatewayRestoreReasoningText 把“只有 summary、没有 content”的推理项补回
// reasoning_text 再发往严格上游。
//
// 背景：两个上游对同一个字段的要求相反。
//   - DeepSeek 的思考模式要求每次回放都带上 reasoning_text，否则整段请求被拒：
//     `The reasoning_text in the thinking mode must be passed back to the API.`
//   - 官方 Codex 后端只接受 content 为空数组的推理项（见 responses_reasoning_sanitize.go）。
//
// 网关在响应出口为了让历史能切回官方账号，会把 DeepSeek 的 content 迁移进 summary 并清空
// content。于是同一份历史再发回 DeepSeek 时必然被拒，线程从此无法继续。本函数在请求出口做
// 反向补全：content 为空而 summary 有文本时，把 summary 文本写回 reasoning_text。两个方向各自
// 得到自己需要的形状，且不新增任何信息——只是把原本来自该模型自己的思考文本放回原字段。
func providerGatewayRestoreReasoningText(gateway *providerGatewaySpec, body []byte) ([]byte, int) {
	if !providerGatewayRestoresReasoningText(gateway) {
		return body, 0
	}
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() || len(input.Array()) == 0 {
		return body, 0
	}
	restored := 0
	for index, item := range input.Array() {
		if !strings.EqualFold(strings.TrimSpace(item.Get("type").String()), "reasoning") {
			continue
		}
		content := item.Get("content")
		if content.IsArray() && len(content.Array()) > 0 {
			continue
		}
		summary := item.Get("summary")
		if !summary.IsArray() || len(summary.Array()) == 0 {
			continue
		}
		texts := make([]string, 0, len(summary.Array()))
		for _, part := range summary.Array() {
			if text := part.Get("text").String(); text != "" {
				texts = append(texts, text)
			}
		}
		if len(texts) == 0 {
			continue
		}
		payload, err := json.Marshal([]map[string]string{{"type": "reasoning_text", "text": strings.Join(texts, "\n")}})
		if err != nil {
			continue
		}
		updated, err := sjson.SetRawBytes(body, fmt.Sprintf("input.%d.content", index), payload)
		if err != nil {
			continue
		}
		body = updated
		restored++
	}
	return body, restored
}
