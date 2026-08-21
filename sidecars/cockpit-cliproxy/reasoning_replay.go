package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const (
	reasoningReplayCacheTTL        = 2 * time.Hour
	reasoningReplayCacheMaxEntries = 4096
	reasoningReplayCacheEvictBatch = 128
	reasoningReplayPlaceholder     = "tool call"
	reasoningReplayUnavailable     = "[reasoning unavailable]"
)

type reasoningReplayEntry struct {
	Reasoning string
	Model     string
	Timestamp time.Time
}

var (
	reasoningReplayMu      sync.Mutex
	reasoningReplayEntries = make(map[string]reasoningReplayEntry)
)

func clearReasoningReplayCache() {
	reasoningReplayMu.Lock()
	reasoningReplayEntries = make(map[string]reasoningReplayEntry)
	reasoningReplayMu.Unlock()
}

func providerGatewayShouldDisableThinking(gateway *providerGatewaySpec) bool {
	return gateway != nil && gateway.DisableThinking
}

func providerGatewayShouldReplayReasoning(gateway *providerGatewaySpec, model string) bool {
	if providerGatewayShouldDisableThinking(gateway) {
		return false
	}
	haystack := strings.ToLower(strings.TrimSpace(model))
	if gateway != nil {
		haystack += " " + strings.ToLower(gateway.BaseURL)
		haystack += " " + strings.ToLower(gateway.UpstreamModel)
		for _, upstream := range gateway.UpstreamModels {
			haystack += " " + strings.ToLower(upstream)
		}
	}
	if haystack == "" {
		return false
	}
	if strings.Contains(haystack, "deepseek-reasoner") || strings.Contains(haystack, "deepseek-r1") {
		return false
	}
	return strings.Contains(haystack, "deepseek")
}

func prepareProviderGatewayChatCompletionsReasoning(gateway *providerGatewaySpec, model string, body []byte) []byte {
	if len(body) == 0 {
		return body
	}
	if providerGatewayShouldDisableThinking(gateway) {
		return disableChatCompletionsThinking(body)
	}
	if !providerGatewayShouldReplayReasoning(gateway, model) {
		return body
	}
	cacheReasoningFromChatCompletionsMessages(model, body)
	return replayReasoningIntoChatCompletions(model, body)
}

func disableChatCompletionsThinking(body []byte) []byte {
	out, err := sjson.SetRawBytes(body, "thinking", []byte(`{"type":"disabled"}`))
	if err != nil {
		return body
	}
	out, _ = sjson.DeleteBytes(out, "reasoning_effort")
	out, _ = sjson.DeleteBytes(out, "reasoning")
	messages := gjson.GetBytes(out, "messages")
	if !messages.IsArray() {
		return out
	}
	for index, message := range messages.Array() {
		if !message.Get("reasoning_content").Exists() && !message.Get("reasoning").Exists() {
			continue
		}
		path := fmt.Sprintf("messages.%d", index)
		out, _ = sjson.DeleteBytes(out, path+".reasoning_content")
		out, _ = sjson.DeleteBytes(out, path+".reasoning")
	}
	return out
}

func cacheReasoningFromChatCompletionsResponse(model string, payload []byte) {
	if len(payload) == 0 {
		return
	}
	root := gjson.ParseBytes(payload)
	choices := root.Get("choices")
	if !choices.IsArray() {
		cacheReasoningFromChatCompletionsMessages(model, payload)
		return
	}
	for _, choice := range choices.Array() {
		message := choice.Get("message")
		if !message.Exists() {
			continue
		}
		cacheReasoningFromAssistantMessage(model, message)
	}
}

func cacheReasoningFromChatCompletionsMessages(model string, body []byte) {
	messages := gjson.GetBytes(body, "messages")
	if !messages.IsArray() {
		return
	}
	for _, message := range messages.Array() {
		cacheReasoningFromAssistantMessage(model, message)
	}
}

func cacheReasoningFromAssistantMessage(model string, message gjson.Result) {
	if strings.TrimSpace(message.Get("role").String()) != "assistant" {
		return
	}
	reasoning := assistantReasoningText(message)
	if reasoning == "" || isReasoningReplayPlaceholder(reasoning) {
		return
	}
	for _, id := range assistantToolCallIDs(message) {
		putReasoningReplay(reasoningReplayToolCallKey(id), model, reasoning)
	}
	if fingerprint := assistantReasoningFingerprint(message); fingerprint != "" {
		putReasoningReplay(reasoningReplayFingerprintKey(fingerprint), model, reasoning)
	}
}

func replayReasoningIntoChatCompletions(model string, body []byte) []byte {
	messages := gjson.GetBytes(body, "messages")
	if !messages.IsArray() {
		return body
	}
	out := body
	for index, message := range messages.Array() {
		if strings.TrimSpace(message.Get("role").String()) != "assistant" {
			continue
		}
		existing := assistantReasoningText(message)
		if existing != "" && !isReasoningReplayPlaceholder(existing) {
			continue
		}
		reasoning, ok := lookupReasoningForAssistantMessage(model, message)
		if !ok {
			continue
		}
		path := fmt.Sprintf("messages.%d.reasoning_content", index)
		next, err := sjson.SetBytes(out, path, reasoning)
		if err != nil {
			continue
		}
		out = next
	}
	return out
}

func lookupReasoningForAssistantMessage(model string, message gjson.Result) (string, bool) {
	for _, id := range assistantToolCallIDs(message) {
		if reasoning, ok := getReasoningReplay(reasoningReplayToolCallKey(id), model); ok {
			return reasoning, true
		}
	}
	if fingerprint := assistantReasoningFingerprint(message); fingerprint != "" {
		if reasoning, ok := getReasoningReplay(reasoningReplayFingerprintKey(fingerprint), model); ok {
			return reasoning, true
		}
	}
	return "", false
}

func assistantReasoningText(message gjson.Result) string {
	for _, key := range []string{"reasoning_content", "reasoning"} {
		if text := strings.TrimSpace(message.Get(key).String()); text != "" {
			return text
		}
	}
	return ""
}

func assistantToolCallIDs(message gjson.Result) []string {
	toolCalls := message.Get("tool_calls")
	if !toolCalls.IsArray() {
		return nil
	}
	ids := make([]string, 0, len(toolCalls.Array()))
	seen := make(map[string]struct{}, len(toolCalls.Array()))
	for _, toolCall := range toolCalls.Array() {
		id := strings.TrimSpace(toolCall.Get("id").String())
		if id == "" {
			continue
		}
		if _, ok := seen[id]; ok {
			continue
		}
		seen[id] = struct{}{}
		ids = append(ids, id)
	}
	return ids
}

func assistantReasoningFingerprint(message gjson.Result) string {
	payload := map[string]any{
		"role":    "assistant",
		"content": canonicalizeAssistantContent(message.Get("content")),
	}
	if toolCalls := canonicalizeAssistantToolCalls(message.Get("tool_calls")); len(toolCalls) > 0 {
		payload["tool_calls"] = toolCalls
	}
	raw, err := json.Marshal(payload)
	if err != nil {
		return ""
	}
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:])
}

func canonicalizeAssistantContent(content gjson.Result) any {
	if !content.Exists() || content.Type == gjson.Null {
		return ""
	}
	if content.Type == gjson.String {
		return content.String()
	}
	if !content.IsArray() {
		return content.Raw
	}
	parts := make([]string, 0, len(content.Array()))
	for _, part := range content.Array() {
		if part.Type == gjson.String {
			if text := part.String(); text != "" {
				parts = append(parts, text)
			}
			continue
		}
		text := part.Get("text").String()
		if text == "" {
			text = part.Get("content").String()
		}
		if text != "" {
			parts = append(parts, text)
		}
	}
	return strings.Join(parts, "")
}

func canonicalizeAssistantToolCalls(toolCalls gjson.Result) []map[string]any {
	if !toolCalls.IsArray() {
		return nil
	}
	out := make([]map[string]any, 0, len(toolCalls.Array()))
	for _, toolCall := range toolCalls.Array() {
		out = append(out, map[string]any{
			"type": strings.TrimSpace(toolCall.Get("type").String()),
			"function": map[string]any{
				"name":      strings.TrimSpace(toolCall.Get("function.name").String()),
				"arguments": toolCall.Get("function.arguments").String(),
			},
		})
	}
	return out
}

func isReasoningReplayPlaceholder(value string) bool {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return true
	}
	switch strings.ToLower(trimmed) {
	case reasoningReplayPlaceholder, reasoningReplayUnavailable:
		return true
	default:
		return false
	}
}

func reasoningReplayToolCallKey(id string) string {
	id = strings.TrimSpace(id)
	if id == "" {
		return ""
	}
	return "tc:" + id
}

func reasoningReplayFingerprintKey(fingerprint string) string {
	fingerprint = strings.TrimSpace(fingerprint)
	if fingerprint == "" {
		return ""
	}
	return "fp:" + fingerprint
}

func putReasoningReplay(key, model, reasoning string) bool {
	key = strings.TrimSpace(key)
	reasoning = strings.TrimSpace(reasoning)
	if key == "" || reasoning == "" || isReasoningReplayPlaceholder(reasoning) {
		return false
	}
	now := time.Now()
	reasoningReplayMu.Lock()
	defer reasoningReplayMu.Unlock()
	reasoningReplayEntries[key] = reasoningReplayEntry{
		Reasoning: reasoning,
		Model:     strings.TrimSpace(model),
		Timestamp: now,
	}
	if len(reasoningReplayEntries) > reasoningReplayCacheMaxEntries {
		evictOldestReasoningReplayEntries(reasoningReplayCacheEvictBatch)
	}
	return true
}

func getReasoningReplay(key, model string) (string, bool) {
	key = strings.TrimSpace(key)
	if key == "" {
		return "", false
	}
	now := time.Now()
	reasoningReplayMu.Lock()
	defer reasoningReplayMu.Unlock()
	entry, ok := reasoningReplayEntries[key]
	if !ok {
		return "", false
	}
	if now.Sub(entry.Timestamp) > reasoningReplayCacheTTL {
		delete(reasoningReplayEntries, key)
		return "", false
	}
	if model != "" && entry.Model != "" && !strings.EqualFold(entry.Model, model) {
		return "", false
	}
	if isReasoningReplayPlaceholder(entry.Reasoning) {
		delete(reasoningReplayEntries, key)
		return "", false
	}
	entry.Timestamp = now
	reasoningReplayEntries[key] = entry
	return entry.Reasoning, true
}

func evictOldestReasoningReplayEntries(count int) {
	if count <= 0 || len(reasoningReplayEntries) == 0 {
		return
	}
	type candidate struct {
		key       string
		timestamp time.Time
	}
	candidates := make([]candidate, 0, len(reasoningReplayEntries))
	for key, entry := range reasoningReplayEntries {
		candidates = append(candidates, candidate{key: key, timestamp: entry.Timestamp})
	}
	sort.Slice(candidates, func(i, j int) bool {
		return candidates[i].timestamp.Before(candidates[j].timestamp)
	})
	if count > len(candidates) {
		count = len(candidates)
	}
	for i := 0; i < count; i++ {
		delete(reasoningReplayEntries, candidates[i].key)
	}
}

type chatStreamReasoningAccumulator struct {
	reasoning     strings.Builder
	content       strings.Builder
	toolCallIDs   []string
	idByIndex     map[int]string
	seenToolCalls map[string]struct{}
}

func newChatStreamReasoningAccumulator() *chatStreamReasoningAccumulator {
	return &chatStreamReasoningAccumulator{
		idByIndex:     make(map[int]string),
		seenToolCalls: make(map[string]struct{}),
	}
}

func (a *chatStreamReasoningAccumulator) consume(line []byte) {
	if a == nil {
		return
	}
	payload := chatCompletionsStreamPayload(line)
	if len(payload) == 0 {
		return
	}
	delta := gjson.GetBytes(payload, "choices.0.delta")
	if !delta.Exists() {
		message := gjson.GetBytes(payload, "choices.0.message")
		if message.Exists() {
			if text := assistantReasoningText(message); text != "" {
				a.reasoning.WriteString(text)
			}
			if content := message.Get("content"); content.Type == gjson.String {
				a.content.WriteString(content.String())
			}
			for _, id := range assistantToolCallIDs(message) {
				a.rememberToolCallID(id)
			}
		}
		return
	}
	if text := strings.TrimSpace(delta.Get("reasoning_content").String()); text != "" {
		a.reasoning.WriteString(delta.Get("reasoning_content").String())
	} else if text := strings.TrimSpace(delta.Get("reasoning").String()); text != "" {
		a.reasoning.WriteString(delta.Get("reasoning").String())
	}
	if content := delta.Get("content"); content.Type == gjson.String {
		a.content.WriteString(content.String())
	}
	toolCalls := delta.Get("tool_calls")
	if !toolCalls.IsArray() {
		return
	}
	for _, toolCall := range toolCalls.Array() {
		index := int(toolCall.Get("index").Int())
		id := strings.TrimSpace(toolCall.Get("id").String())
		if id == "" {
			id = a.idByIndex[index]
		} else {
			a.idByIndex[index] = id
		}
		a.rememberToolCallID(id)
	}
}

func (a *chatStreamReasoningAccumulator) rememberToolCallID(id string) {
	id = strings.TrimSpace(id)
	if id == "" {
		return
	}
	if _, ok := a.seenToolCalls[id]; ok {
		return
	}
	a.seenToolCalls[id] = struct{}{}
	a.toolCallIDs = append(a.toolCallIDs, id)
}

func (a *chatStreamReasoningAccumulator) cache(model string) {
	if a == nil {
		return
	}
	reasoning := strings.TrimSpace(a.reasoning.String())
	if reasoning == "" || isReasoningReplayPlaceholder(reasoning) {
		return
	}
	if len(a.toolCallIDs) > 0 {
		for _, id := range a.toolCallIDs {
			putReasoningReplay(reasoningReplayToolCallKey(id), model, reasoning)
		}
		return
	}
	messageJSON := []byte(`{"role":"assistant"}`)
	messageJSON, _ = sjson.SetBytes(messageJSON, "content", a.content.String())
	messageJSON, _ = sjson.SetBytes(messageJSON, "reasoning_content", reasoning)
	cacheReasoningFromAssistantMessage(model, gjson.ParseBytes(messageJSON))
}

func chatCompletionsStreamPayload(line []byte) []byte {
	line = bytes.TrimSpace(line)
	if bytes.HasPrefix(line, []byte("data:")) {
		line = bytes.TrimSpace(line[len("data:"):])
	}
	if len(line) == 0 || bytes.Equal(line, []byte("[DONE]")) {
		return nil
	}
	if !gjson.ValidBytes(line) {
		return nil
	}
	return line
}
