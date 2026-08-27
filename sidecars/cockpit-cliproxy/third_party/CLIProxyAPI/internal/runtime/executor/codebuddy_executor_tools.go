package executor

import (
	"fmt"
	"strings"

	log "github.com/sirupsen/logrus"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// normalizeCodebuddyToolMessages repairs tool-related message fields that the
// strict Tencent CodeBuddy backend rejects with `400 invalid_parameter_value`
// (param left empty). Cursor and other OpenAI-compatible clients send tool
// messages whose shape differs slightly from what the backend accepts. This
// pass only repairs clearly-invalid shapes; valid messages pass through
// unchanged.
//
// Repairs performed:
//  1. Drop assistant messages that are entirely empty (no content, no
//     tool_calls, no function_call).
//  2. Normalize `role=tool` messages: resolve `tool_call_id` (standard name,
//     then `call_id` alias, then infer from the single pending assistant tool
//     call), and coerce `content` to a string (null/missing -> "", array ->
//     joined text).
//  3. Normalize `function.arguments` on assistant tool_calls: empty, null or
//     non-JSON arguments are replaced with "{}" so the backend can parse them.
func normalizeCodebuddyToolMessages(body []byte) ([]byte, error) {
	if len(body) == 0 || !gjson.ValidBytes(body) {
		return body, nil
	}
	messages := gjson.GetBytes(body, "messages")
	if !messages.Exists() || !messages.IsArray() {
		return body, nil
	}

	out, dropped, err := filterCodebuddyEmptyAssistantMessages(body, messages.Array())
	if err != nil {
		return body, err
	}
	if dropped > 0 {
		log.WithField("dropped_assistant_messages", dropped).
			Debug("codebuddy executor: dropped empty assistant messages")
	}

	messages = gjson.GetBytes(out, "messages")
	msgs := messages.Array()
	pending := make([]string, 0)
	patched := 0

	removePending := func(id string) {
		for i := range pending {
			if pending[i] == id {
				pending = append(pending[:i], pending[i+1:]...)
				return
			}
		}
	}

	for msgIdx := range msgs {
		msg := msgs[msgIdx]
		role := strings.TrimSpace(msg.Get("role").String())
		switch role {
		case "assistant":
			toolCalls := msg.Get("tool_calls")
			if !toolCalls.IsArray() {
				continue
			}
			for tcIdx, tc := range toolCalls.Array() {
				if id := strings.TrimSpace(tc.Get("id").String()); id != "" {
					pending = append(pending, id)
				}
				args := tc.Get("function.arguments")
				argsStr := args.String()
				if !args.Exists() || args.Type == gjson.Null || !gjson.Valid(argsStr) {
					path := fmt.Sprintf("messages.%d.tool_calls.%d.function.arguments", msgIdx, tcIdx)
					next, errSet := sjson.SetBytes(out, path, "{}")
					if errSet != nil {
						return body, errSet
					}
					out = next
					patched++
				}
			}
		case "tool":
			toolCallID := strings.TrimSpace(msg.Get("tool_call_id").String())
			if toolCallID == "" {
				if alias := strings.TrimSpace(msg.Get("call_id").String()); alias != "" {
					toolCallID = alias
					next, errSet := sjson.SetBytes(out, fmt.Sprintf("messages.%d.tool_call_id", msgIdx), alias)
					if errSet != nil {
						return body, errSet
					}
					out = next
					patched++
				}
			}
			if toolCallID == "" && len(pending) == 1 {
				toolCallID = pending[0]
				next, errSet := sjson.SetBytes(out, fmt.Sprintf("messages.%d.tool_call_id", msgIdx), toolCallID)
				if errSet != nil {
					return body, errSet
				}
				out = next
				patched++
			}
			if toolCallID != "" {
				removePending(toolCallID)
			}

			content := msg.Get("content")
			switch {
			case !content.Exists() || content.Type == gjson.Null:
				next, errSet := sjson.SetBytes(out, fmt.Sprintf("messages.%d.content", msgIdx), "")
				if errSet != nil {
					return body, errSet
				}
				out = next
				patched++
			case content.IsArray():
				next, errSet := sjson.SetBytes(out, fmt.Sprintf("messages.%d.content", msgIdx), joinCodebuddyToolContent(content))
				if errSet != nil {
					return body, errSet
				}
				out = next
				patched++
			}
		}
	}

	// The backend rejects a messages array that ends with a tool or assistant
	// message when the request carries both `tools` and `reasoning_effort`
	// (400 invalid_parameter_value, param empty). Cursor's tool-result
	// follow-ups end on a tool message, so append a user continuation to make
	// the trailing role legal — mirroring Cursor's own `<system_reminder>`
	// follow-up that the backend accepts. This runs after
	// filterCodebuddyEmptyAssistantMessages so it is never dropped.
	lastMsgs := gjson.GetBytes(out, "messages").Array()
	if len(lastMsgs) > 0 && strings.TrimSpace(lastMsgs[len(lastMsgs)-1].Get("role").String()) == "tool" {
		next, errSet := sjson.SetRawBytes(out, "messages.-1", []byte(`{"role":"user","content":"<system_reminder>The tool call completed. Continue based on the tool result.</system_reminder>"}`))
		if errSet != nil {
			return body, errSet
		}
		out = next
		patched++
		log.Debug("codebuddy executor: appended user continuation after trailing tool message")
	}

	if patched > 0 {
		log.WithField("patched_tool_messages", patched).
			Debug("codebuddy executor: normalized tool message fields")
	}
	return out, nil
}

// filterCodebuddyEmptyAssistantMessages drops assistant messages that carry no
// text content and no tool calls, which the backend rejects.
func filterCodebuddyEmptyAssistantMessages(body []byte, msgs []gjson.Result) ([]byte, int, error) {
	kept := make([]string, 0, len(msgs))
	dropped := 0
	for _, msg := range msgs {
		if strings.TrimSpace(msg.Get("role").String()) != "assistant" {
			kept = append(kept, msg.Raw)
			continue
		}
		if isEmptyCodebuddyAssistantMessage(msg) {
			dropped++
			continue
		}
		kept = append(kept, msg.Raw)
	}
	if dropped == 0 {
		return body, 0, nil
	}
	rawMessages := []byte("[" + strings.Join(kept, ",") + "]")
	out, err := sjson.SetRawBytes(body, "messages", rawMessages)
	if err != nil {
		return body, 0, fmt.Errorf("codebuddy executor: failed to drop empty assistant messages: %w", err)
	}
	return out, dropped, nil
}

// isEmptyCodebuddyAssistantMessage reports whether an assistant message carries
// no meaningful content (no text, no tool_calls, no legacy function_call).
func isEmptyCodebuddyAssistantMessage(msg gjson.Result) bool {
	if toolCalls := msg.Get("tool_calls"); toolCalls.Exists() && toolCalls.IsArray() && len(toolCalls.Array()) > 0 {
		return false
	}
	if fc := msg.Get("function_call"); fc.Exists() && fc.Type != gjson.Null && strings.TrimSpace(fc.Raw) != "{}" {
		return false
	}
	if rc := msg.Get("reasoning_content"); rc.Exists() && strings.TrimSpace(rc.String()) != "" {
		return false
	}
	content := msg.Get("content")
	if !content.Exists() || content.Type == gjson.Null {
		return true
	}
	if content.Type == gjson.String {
		return strings.TrimSpace(content.String()) == ""
	}
	if content.IsArray() {
		for _, part := range content.Array() {
			if text := strings.TrimSpace(part.Get("text").String()); text != "" {
				return false
			}
		}
		return true
	}
	return false
}

// joinCodebuddyToolContent flattens an array-shaped tool content into a single
// string by concatenating the text parts, mirroring the OpenAI convention.
func joinCodebuddyToolContent(content gjson.Result) string {
	parts := make([]string, 0, len(content.Array()))
	for _, part := range content.Array() {
		if text := strings.TrimSpace(part.Get("text").String()); text != "" {
			parts = append(parts, text)
		}
	}
	return strings.Join(parts, "\n")
}
