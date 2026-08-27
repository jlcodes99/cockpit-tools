package executor

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	log "github.com/sirupsen/logrus"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// isCompleteJSONObject reports whether s is exactly one complete JSON object
// (not a partial fragment). Used to distinguish upstream "full-snapshot"
// argument deltas (must be replaced, not appended) from OpenAI-style
// incremental fragments (must be appended).
func isCompleteJSONObject(s string) bool {
	trimmed := strings.TrimSpace(s)
	if len(trimmed) < 2 || trimmed[0] != '{' || trimmed[len(trimmed)-1] != '}' {
		return false
	}
	if !gjson.Valid(trimmed) {
		return false
	}
	return gjson.Parse(trimmed).IsObject()
}

// isEmptyToolArguments reports whether a full-snapshot arguments value carries
// no real content (empty object "{}", empty string, or whitespace). Such
// snapshots must not overwrite real arguments accumulated from earlier deltas:
// some backends re-send a trailing "{}" after the real arguments, and a naive
// "overwrite with the last non-empty value" would clobber the real payload.
func isEmptyToolArguments(s string) bool {
	trimmed := strings.TrimSpace(s)
	if trimmed == "" {
		return true
	}
	if !gjson.Valid(trimmed) {
		return false
	}
	res := gjson.Parse(trimmed)
	if res.IsObject() {
		return len(res.Map()) == 0
	}
	return false
}

// codebuddyStreamToolCall holds the accumulated state of a single in-flight
// tool call, keyed by its streaming `index`.
type codebuddyStreamToolCall struct {
	index     int64
	id        string // first non-empty id, normalized (tooluse_ -> call_)
	typ       string
	name      string
	arguments string // full snapshot -> overwrite; incremental fragment -> append
}

// codebuddyStreamToolCallBuffer buffers tool_calls across an OpenAI-compatible
// SSE stream so they can be emitted exactly once, as a single consolidated
// delta, right before finish_reason.
//
// The CodeBuddy CN backend (falling back to DeepSeek etc.) re-sends the
// *complete* accumulated arguments JSON on every delta. Cursor appends every
// `function.arguments` fragment it receives, so naive forwarding produces
// `{...}{...}` (multiple top-level values) and the tool call fails to parse.
// Buffering and emitting once sidesteps that entirely: full snapshots overwrite
// (last non-empty wins) while true incremental fragments are concatenated.
type codebuddyStreamToolCallBuffer struct {
	calls   map[int64]*codebuddyStreamToolCall
	order   []int64
	id      string
	model   string
	created int64
}

func newCodebuddyStreamToolCallBuffer() *codebuddyStreamToolCallBuffer {
	return &codebuddyStreamToolCallBuffer{
		calls: make(map[int64]*codebuddyStreamToolCall),
	}
}

// HasToolCalls reports whether at least one tool call has been accumulated.
func (b *codebuddyStreamToolCallBuffer) HasToolCalls() bool {
	return len(b.order) > 0
}

// Consume processes a single SSE line (with a `data:` prefix). It accumulates
// any tool_calls deltas and strips `delta.tool_calls` and
// `delta.reasoning_content` from the returned line so they are not forwarded to
// the client (which would append them and corrupt the arguments JSON). It also
// captures id/model/created for the consolidated chunk and reports the chunk's
// finish_reason (if any). Non-data lines, non-JSON payloads and `[DONE]` are
// returned unchanged with an empty finish reason (zero impact on pure-text
// streams).
func (b *codebuddyStreamToolCallBuffer) Consume(line []byte) (stripped []byte, finishReason string) {
	trimmed := bytes.TrimSpace(line)
	if len(trimmed) == 0 || !bytes.HasPrefix(trimmed, []byte("data:")) {
		return line, ""
	}
	payload := bytes.TrimSpace(trimmed[len("data:"):])
	if len(payload) == 0 || bytes.Equal(payload, []byte("[DONE]")) || !gjson.ValidBytes(payload) {
		return line, ""
	}

	if b.id == "" {
		b.id = gjson.GetBytes(payload, "id").String()
	}
	if b.model == "" {
		b.model = gjson.GetBytes(payload, "model").String()
	}
	if b.created == 0 {
		b.created = gjson.GetBytes(payload, "created").Int()
	}

	choices := gjson.GetBytes(payload, "choices")
	if !choices.IsArray() {
		return line, ""
	}

	out := payload
	modified := false
	var finish string
	for ci, choice := range choices.Array() {
		if fr := choice.Get("finish_reason"); fr.Exists() && fr.String() != "" {
			finish = fr.String()
		}

		if delta := choice.Get("delta"); delta.IsObject() {
			if toolCalls := delta.Get("tool_calls"); toolCalls.IsArray() {
				// Dump the raw upstream tool_calls delta (redacted) so the exact
				// shape (full-snapshot vs incremental fragment) can be inspected.
				helps.DumpCodebuddyDebugBody("toolcall-delta", payload)
				for _, tc := range toolCalls.Array() {
					b.accumulate(tc)
				}
				if next, err := sjson.DeleteBytes(out, fmt.Sprintf("choices.%d.delta.tool_calls", ci)); err == nil {
					out = next
					modified = true
				}
			}

			// Strip the legacy (deprecated) `function_call` field. Upstream
			// deepseek backends emit a placeholder {"name":"","arguments":""} on
			// the finish chunk; clients like Cursor may JSON-parse the empty
			// arguments and conflate it with the modern `tool_calls` we emit.
			// Only strip when it is a real object (not the ubiquitous null) so
			// pure-text chunks are not re-serialized on every line.
			if fc := delta.Get("function_call"); fc.Exists() && fc.Type != gjson.Null {
				if next, err := sjson.DeleteBytes(out, fmt.Sprintf("choices.%d.delta.function_call", ci)); err == nil {
					out = next
					modified = true
				}
			}

			if delta.Get("reasoning_content").Exists() {
				if next, err := sjson.DeleteBytes(out, fmt.Sprintf("choices.%d.delta.reasoning_content", ci)); err == nil {
					out = next
					modified = true
				}
			}
		}

		// Some backends echo the whole message (non-streaming shape) under
		// `message` in the final chunk, duplicating the delta stream we already
		// accumulated. Strip those echoes so they don't reach the client and get
		// concatenated with our consolidated tool_calls.
		if msg := choice.Get("message"); msg.IsObject() {
			if msg.Get("tool_calls").Exists() {
				helps.DumpCodebuddyDebugBody("toolcall-message-echo", payload)
				if next, err := sjson.DeleteBytes(out, fmt.Sprintf("choices.%d.message.tool_calls", ci)); err == nil {
					out = next
					modified = true
				}
			}
			if msg.Get("function_call").Exists() {
				if next, err := sjson.DeleteBytes(out, fmt.Sprintf("choices.%d.message.function_call", ci)); err == nil {
					out = next
					modified = true
				}
			}
		}
	}

	if !modified {
		return line, finish
	}
	return append([]byte("data: "), out...), finish
}

// accumulate merges a single tool_calls delta into the buffer.
func (b *codebuddyStreamToolCallBuffer) accumulate(tc gjson.Result) {
	idx := tc.Get("index").Int()
	call, ok := b.calls[idx]
	if !ok {
		call = &codebuddyStreamToolCall{index: idx}
		b.calls[idx] = call
		b.order = append(b.order, idx)
	}

	// id must stay stable for the whole stream; keep the first non-empty value.
	if call.id == "" {
		if v := tc.Get("id"); v.Exists() && strings.TrimSpace(v.String()) != "" {
			call.id = normalizeToolCallID(v.String())
		}
	}
	if call.typ == "" {
		if v := tc.Get("type"); v.Exists() && strings.TrimSpace(v.String()) != "" {
			call.typ = v.String()
		}
	}

	fn := tc.Get("function")
	if !fn.Exists() {
		return
	}
	if call.name == "" {
		if v := fn.Get("name"); v.Exists() && strings.TrimSpace(v.String()) != "" {
			call.name = v.String()
		}
	}
	argRes := fn.Get("arguments")
	if !argRes.Exists() || argRes.Type == gjson.Null {
		return
	}
	arg := argRes.String()
	if arg == "" { // 只跳过完全空的片段，保留 " " 等纯空格（它们是增量 arguments 的一部分）
		return
	}
	switch {
	case call.arguments == "":
		if isCompleteJSONObject(arg) {
			if isEmptyToolArguments(arg) {
				// First snapshot is an empty "{}": skip it and wait for the real
				// arguments instead of seeding an empty object.
				log.Debugf("codebuddy stream toolcall: index=%d FIRST snapshot is empty (ignored)", idx)
				break
			}
			call.arguments = strings.TrimSpace(arg)
			log.Debugf("codebuddy stream toolcall: index=%d FIRST full-snapshot arguments=%q", idx, call.arguments)
		} else {
			call.arguments = arg
			log.Debugf("codebuddy stream toolcall: index=%d FIRST fragment arguments=%q", idx, call.arguments)
		}
	case isCompleteJSONObject(arg):
		trimmed := strings.TrimSpace(arg)
		if isEmptyToolArguments(trimmed) {
			// Empty "{}" snapshot: never clobber real arguments with an empty
			// object (upstream may re-send "{}" after the real payload).
			log.Debugf("codebuddy stream toolcall: index=%d empty snapshot %q ignored (keeping %q)", idx, trimmed, call.arguments)
		} else if trimmed != call.arguments {
			// Full snapshot: overwrite with the last non-empty value.
			log.Debugf("codebuddy stream toolcall: index=%d OVERWRITE %q -> %q", idx, call.arguments, trimmed)
			call.arguments = trimmed
		} else {
			log.Debugf("codebuddy stream toolcall: index=%d full-snapshot unchanged (dedupe) %q", idx, trimmed)
		}
	default:
		// Incremental fragment: concatenate.
		log.Debugf("codebuddy stream toolcall: index=%d APPEND %q + %q", idx, call.arguments, arg)
		call.arguments += arg
	}
}

// BuildToolCallsChunk renders the accumulated tool calls as a single OpenAI
// stream chunk. `function.arguments` is always a JSON string (empty -> "{}")
// and `delta.content` is null so the client associates the tool calls with an
// empty message. Returns nil if there is nothing to emit.
func (b *codebuddyStreamToolCallBuffer) BuildToolCallsChunk() []byte {
	if len(b.order) == 0 {
		return nil
	}
	toolCalls := make([]map[string]any, 0, len(b.order))
	for _, idx := range b.order {
		call := b.calls[idx]
		args := call.arguments
		if strings.TrimSpace(args) == "" {
			args = "{}"
		}
		id := call.id
		if id == "" {
			id = fmt.Sprintf("call_%d", idx)
		}
		item := map[string]any{
			"index": call.index,
			"id":    id,
			"type":  "function",
			"function": map[string]any{
				"name":      call.name,
				"arguments": args,
			},
		}
		if call.typ != "" {
			item["type"] = call.typ
		}
		toolCalls = append(toolCalls, item)
	}

	obj := map[string]any{
		"id":      b.id,
		"object":  "chat.completion.chunk",
		"created": b.created,
		"model":   b.model,
		"choices": []map[string]any{
			{
				"index": 0,
				"delta": map[string]any{
					"content":    nil,
					"tool_calls": toolCalls,
				},
				"finish_reason": nil,
			},
		},
	}
	out, err := json.Marshal(obj)
	if err != nil {
		return nil
	}
	result := append([]byte("data: "), out...)
	log.Debugf("codebuddy stream toolcall: emitting consolidated chunk: %s", result)
	return result
}
