package main

import (
	"bytes"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/signature"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// 第三方 Responses 上游（DeepSeek 等）会在推理项里回放两类官方无法接受的字段：
//
//	{"type":"reasoning","content":[{"type":"reasoning_text","text":"..."}],"summary":[],"encrypted_content":"bf0ff0e2-...-1c-0"}
//
// 官方 Codex 后端只接受 content 为空数组的 reasoning 项，并且会校验 encrypted_content：
//
//	Invalid 'input[i].content': array too long. Expected an array with maximum length 0...
//	The encrypted content bf0f...1c-0 could not be verified. Reason: Encrypted content could not be decrypted or parsed.
//
// 客户端把这类项落盘后，同一会话切到官方账号或其它严格 Responses 上游时（普通回合与自动压缩
// 都会）整段请求被拒。因此在网关响应出口把可见推理正文迁移到合法的 summary、清空 content，
// 并丢弃非官方格式的 encrypted_content，让客户端从一开始就只落盘官方可回放的推理形状。

const reasoningTextPartType = "reasoning_text"
const reasoningEncryptedContentField = "encrypted_content"

var reasoningTextPartMarker = []byte(reasoningTextPartType)
var reasoningEncryptedContentMarker = []byte(reasoningEncryptedContentField)

// normalizeResponsesReasoningContentBody 清洗非流式 Responses 响应体里的推理项。
// 不包含 reasoning_text 的响应体原样返回。
func normalizeResponsesReasoningContentBody(body []byte) []byte {
	if len(body) == 0 || !hasReasoningSanitizeMarker(body) {
		return body
	}
	if !gjson.ValidBytes(body) {
		return body
	}
	return normalizeResponsesReasoningContentPayload(body)
}

// normalizeResponsesReasoningContentSSE 逐行清洗 Responses SSE 文本（可能含多帧）。
// 未命中 reasoning_text 的文本原样返回，保持网关当前字节级透传行为。
func normalizeResponsesReasoningContentSSE(chunk []byte) []byte {
	if len(chunk) == 0 || !hasReasoningSanitizeMarker(chunk) {
		return chunk
	}
	out := make([]byte, 0, len(chunk))
	rest := chunk
	for len(rest) > 0 {
		index := bytes.IndexByte(rest, '\n')
		var line []byte
		if index < 0 {
			line, rest = rest, nil
		} else {
			line, rest = rest[:index+1], rest[index+1:]
		}
		out = append(out, normalizeResponsesReasoningContentSSELine(line)...)
	}
	return out
}

// hasReasoningSanitizeMarker 让未命中相关字段的请求维持零改写的快速路径。
func hasReasoningSanitizeMarker(payload []byte) bool {
	return bytes.Contains(payload, reasoningTextPartMarker) ||
		bytes.Contains(payload, reasoningEncryptedContentMarker)
}

func normalizeResponsesReasoningContentSSELine(line []byte) []byte {
	content := line
	eol := []byte(nil)
	if trimmed := bytes.TrimRight(content, "\r\n"); len(trimmed) != len(content) {
		eol = content[len(trimmed):]
		content = trimmed
	}
	field := bytes.TrimLeft(content, " \t")
	indent := content[:len(content)-len(field)]
	if !bytes.HasPrefix(field, []byte("data:")) {
		return line
	}
	remainder := field[len("data:"):]
	spacing := remainder[:len(remainder)-len(bytes.TrimLeft(remainder, " \t"))]
	payload := bytes.TrimSpace(remainder)
	if len(payload) == 0 || !gjson.ValidBytes(payload) {
		return line
	}
	normalized := normalizeResponsesReasoningContentPayload(payload)
	if bytes.Equal(normalized, payload) {
		return line
	}
	out := make([]byte, 0, len(line)-len(payload)+len(normalized))
	out = append(out, indent...)
	out = append(out, "data:"...)
	out = append(out, spacing...)
	out = append(out, normalized...)
	out = append(out, eol...)
	return out
}

// normalizeResponsesReasoningContentPayload 清洗单个 Responses 事件或响应体 JSON。
func normalizeResponsesReasoningContentPayload(payload []byte) []byte {
	normalized := payload
	if item := gjson.GetBytes(normalized, "item"); item.IsObject() {
		if updated := normalizeResponsesReasoningItem([]byte(item.Raw)); !bytes.Equal(updated, []byte(item.Raw)) {
			if next, err := sjson.SetRawBytes(normalized, "item", updated); err == nil {
				normalized = next
			}
		}
	}
	if output := gjson.GetBytes(normalized, "response.output"); output.IsArray() {
		if updated, changed := normalizeResponsesReasoningItems(output.Array()); changed {
			if next, err := sjson.SetRawBytes(normalized, "response.output", updated); err == nil {
				normalized = next
			}
		}
	}
	if output := gjson.GetBytes(normalized, "output"); output.IsArray() {
		if updated, changed := normalizeResponsesReasoningItems(output.Array()); changed {
			if next, err := sjson.SetRawBytes(normalized, "output", updated); err == nil {
				normalized = next
			}
		}
	}
	return normalized
}

func normalizeResponsesReasoningItems(items []gjson.Result) ([]byte, bool) {
	var buffer bytes.Buffer
	buffer.WriteByte('[')
	changed := false
	for index, item := range items {
		if index > 0 {
			buffer.WriteByte(',')
		}
		raw := []byte(item.Raw)
		updated := normalizeResponsesReasoningItem(raw)
		if !bytes.Equal(updated, raw) {
			changed = true
		}
		buffer.Write(updated)
	}
	buffer.WriteByte(']')
	return buffer.Bytes(), changed
}

// normalizeResponsesReasoningItem 把推理项里的可见推理正文迁移到 summary、清空 content，
// 并丢弃官方无法校验的 encrypted_content（例如第三方返回的 "<response-id>-0"）。
// 已经是官方形状（content 为空数组且签名合法）的项原样返回。
func normalizeResponsesReasoningItem(item []byte) []byte {
	if len(item) == 0 || !hasReasoningSanitizeMarker(item) || !gjson.ValidBytes(item) {
		return item
	}
	parsed := gjson.ParseBytes(item)
	if strings.TrimSpace(parsed.Get("type").String()) != "reasoning" {
		return item
	}

	item = normalizeReasoningEncryptedContent(item, parsed)
	parsed = gjson.ParseBytes(item)

	content := parsed.Get("content")
	if !content.IsArray() || len(content.Array()) == 0 {
		return item
	}

	if summary := parsed.Get("summary"); !summary.Exists() || (summary.IsArray() && len(summary.Array()) == 0) {
		if parts, ok := reasoningSummaryParts(content.Array()); ok {
			if updated, err := sjson.SetRawBytes(item, "summary", parts); err == nil {
				item = updated
			}
		}
	}
	if updated, err := sjson.SetRawBytes(item, "content", []byte("[]")); err == nil {
		return updated
	}
	return item
}

// normalizeReasoningEncryptedContent 丢弃非官方格式的 encrypted_content。
// 官方签名（gAAAA 前缀的 Fernet 结构）保持原样，否则会触发上游
// `invalid_encrypted_content`（自动压缩与普通回合都会失败）。
func normalizeReasoningEncryptedContent(item []byte, parsed gjson.Result) []byte {
	encryptedContent := parsed.Get(reasoningEncryptedContentField)
	if !encryptedContent.Exists() || encryptedContent.Type == gjson.Null {
		return item
	}
	if encryptedContent.Type == gjson.String &&
		signature.IsValidGPTReasoningSignature(encryptedContent.String()) {
		return item
	}
	if updated, err := sjson.DeleteBytes(item, reasoningEncryptedContentField); err == nil {
		return updated
	}
	return item
}

// reasoningSummaryParts 把 reasoning_text 片段转写成 summary_text 片段。
func reasoningSummaryParts(parts []gjson.Result) ([]byte, bool) {
	var buffer bytes.Buffer
	buffer.WriteByte('[')
	written := 0
	for _, part := range parts {
		text := ""
		if part.Type == gjson.String {
			text = part.String()
		} else if strings.TrimSpace(part.Get("type").String()) == reasoningTextPartType {
			text = part.Get("text").String()
		}
		if strings.TrimSpace(text) == "" {
			continue
		}
		summaryPart, err := sjson.SetBytes([]byte(`{"type":"summary_text","text":""}`), "text", text)
		if err != nil {
			continue
		}
		if written > 0 {
			buffer.WriteByte(',')
		}
		buffer.Write(summaryPart)
		written++
	}
	buffer.WriteByte(']')
	return buffer.Bytes(), written > 0
}
