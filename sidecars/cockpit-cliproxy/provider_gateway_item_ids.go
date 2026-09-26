package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"strconv"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const providerGatewayItemIDLimit = 64

// providerGatewayItemIDPrefix returns the item id prefix the official Responses
// API validates. An empty result means the item type must stay untouched.
func providerGatewayItemIDPrefix(itemType string) string {
	switch strings.TrimSpace(itemType) {
	case "message":
		return "msg"
	case "reasoning":
		return "rs"
	case "function_call":
		return "fc"
	case "function_call_output":
		return "fco"
	case "custom_tool_call":
		return "ctc"
	case "custom_tool_call_output":
		return "ctco"
	default:
		return ""
	}
}

// providerGatewayItemIDRewriter normalizes item ids on the way out to the Codex
// client. Third-party upstreams return ids without the official prefixes (or
// without ids at all), and the client persists whatever it receives. Once such a
// conversation is replayed against an official account, the strict official
// validator rejects the whole request with invalid_id_prefix. Rewriting the ids
// here keeps the persisted history valid for every provider.
type providerGatewayItemIDRewriter struct {
	mapped map[string]string
	used   map[string]bool
}

func newProviderGatewayItemIDRewriter() *providerGatewayItemIDRewriter {
	return &providerGatewayItemIDRewriter{
		mapped: make(map[string]string),
		used:   make(map[string]bool),
	}
}

// RewriteSSEFrame rewrites item ids inside one SSE frame. Frames that carry a
// bare JSON payload are rewritten directly.
func (r *providerGatewayItemIDRewriter) RewriteSSEFrame(frame []byte) []byte {
	if len(frame) == 0 {
		return frame
	}
	if !bytes.Contains(frame, []byte(`"id"`)) && !bytes.Contains(frame, []byte(`"item_id"`)) {
		return frame
	}
	index := bytes.Index(frame, []byte("data:"))
	if index < 0 {
		return r.RewritePayload(frame)
	}
	prefixEnd := index + len("data:")
	rest := frame[prefixEnd:]
	lineEnd := bytes.IndexByte(rest, '\n')
	payload := rest
	var suffix []byte
	if lineEnd >= 0 {
		payload = rest[:lineEnd]
		suffix = rest[lineEnd:]
	}
	trimmed := bytes.TrimSpace(payload)
	if len(trimmed) == 0 || bytes.Equal(trimmed, []byte("[DONE]")) {
		return frame
	}
	rewritten := r.RewritePayload(trimmed)
	if bytes.Equal(rewritten, trimmed) {
		return frame
	}
	out := make([]byte, 0, len(frame)+16)
	out = append(out, frame[:prefixEnd]...)
	out = append(out, rewritten...)
	out = append(out, suffix...)
	return out
}

// RewritePayload rewrites every item id and item_id reference it can resolve.
func (r *providerGatewayItemIDRewriter) RewritePayload(payload []byte) []byte {
	trimmed := bytes.TrimSpace(payload)
	if len(trimmed) == 0 || trimmed[0] != '{' {
		return payload
	}
	updated := payload
	responseID := strings.TrimSpace(gjson.GetBytes(updated, "response.id").String())
	if responseID == "" {
		responseID = strings.TrimSpace(gjson.GetBytes(updated, "response_id").String())
	}
	if responseID == "" {
		responseID = strings.TrimSpace(gjson.GetBytes(updated, "id").String())
	}
	if gjson.GetBytes(updated, "item.type").Exists() {
		index := gjson.GetBytes(updated, "output_index").Int()
		updated = r.rewriteItemAtPath(updated, "item", fmt.Sprintf("%s_%d", responseID, index))
	}
	for _, container := range []string{"output", "response.output"} {
		items := gjson.GetBytes(updated, container)
		if !items.IsArray() {
			continue
		}
		for index := range items.Array() {
			updated = r.rewriteItemAtPath(
				updated,
				fmt.Sprintf("%s.%d", container, index),
				fmt.Sprintf("%s_%d", responseID, index),
			)
		}
	}
	if itemID := gjson.GetBytes(updated, "item_id"); itemID.Type == gjson.String {
		if mapped, ok := r.mapped[itemID.String()]; ok && mapped != itemID.String() {
			if next, err := sjson.SetBytes(updated, "item_id", mapped); err == nil {
				updated = next
			}
		}
	}
	return updated
}

func (r *providerGatewayItemIDRewriter) rewriteItemAtPath(payload []byte, path, fallback string) []byte {
	item := gjson.GetBytes(payload, path)
	if !item.IsObject() {
		return payload
	}
	prefix := providerGatewayItemIDPrefix(item.Get("type").String())
	if prefix == "" {
		return payload
	}
	originalID := strings.TrimSpace(item.Get("id").String())
	callID := strings.TrimSpace(item.Get("call_id").String())
	normalized := r.normalizeID(prefix, originalID, callID, fallback)
	if normalized == "" {
		return payload
	}
	// Even an unchanged ID must be remembered: later events for this item
	// must reuse it instead of treating its reserved ID as a collision.
	r.remember(originalID, normalized)
	if normalized == originalID {
		return payload
	}
	updated, err := sjson.SetBytes(payload, path+".id", normalized)
	if err != nil {
		return payload
	}
	return updated
}

func (r *providerGatewayItemIDRewriter) normalizeID(prefix, rawID, callID, fallback string) string {
	if rawID != "" {
		if mapped, ok := r.mapped[rawID]; ok {
			return mapped
		}
	}
	base := rawID
	if base == "" {
		base = callID
	}
	if base == "" {
		base = fallback
	}
	if base == "" {
		return ""
	}
	normalized := base
	if !strings.HasPrefix(normalized, prefix+"_") {
		normalized = prefix + "_" + normalized
	}
	normalized = providerGatewayLimitItemID(normalized, 0)
	if r.used[normalized] {
		for attempt := 1; ; attempt++ {
			candidate := providerGatewayLimitItemID(normalized, attempt)
			if !r.used[candidate] {
				normalized = candidate
				break
			}
		}
	}
	r.used[normalized] = true
	return normalized
}

func (r *providerGatewayItemIDRewriter) remember(rawID, normalized string) {
	if rawID == "" {
		return
	}
	r.mapped[rawID] = normalized
}

func providerGatewayLimitItemID(id string, attempt int) string {
	runes := []rune(id)
	// 只有「首次归一化 + 未超长」才允许原样返回。否则短 ID 在 attempt>0 时
	// 会一直返回同一个值，normalizeID 的重名去重循环永远找不到空闲 ID（死循环）。
	if attempt <= 0 && len(runes) <= providerGatewayItemIDLimit {
		return id
	}
	hashInput := id
	if attempt > 0 {
		hashInput += "\x00" + strconv.Itoa(attempt)
	}
	sum := sha256.Sum256([]byte(hashInput))
	suffix := "_" + hex.EncodeToString(sum[:8])
	prefixLength := providerGatewayItemIDLimit - len(suffix)
	if prefixLength < 0 {
		prefixLength = 0
	}
	if len(runes) < prefixLength {
		prefixLength = len(runes)
	}
	return string(runes[:prefixLength]) + suffix
}
