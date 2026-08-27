package executor

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestNormalizeCodebuddyToolMessages_NullContentBecomesString(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"run","arguments":"{}"}}]},
			{"role":"tool","tool_call_id":"call_1","content":null}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	got := gjson.GetBytes(out, "messages.1.content")
	if !got.Exists() || got.Type != gjson.String {
		t.Fatalf("content should be coerced to string, got %s", got.Raw)
	}
}

func TestNormalizeCodebuddyToolMessages_ArrayContentJoined(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"run","arguments":"{}"}}]},
			{"role":"tool","tool_call_id":"call_1","content":[{"type":"text","text":"hello"},{"type":"text","text":"world"}]}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	got := gjson.GetBytes(out, "messages.1.content").String()
	if got != "hello\nworld" {
		t.Fatalf("content = %q, want %q", got, "hello\nworld")
	}
}

func TestNormalizeCodebuddyToolMessages_EmptyArgumentsBecomeEmptyObject(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"run","arguments":""}}]},
			{"role":"tool","tool_call_id":"call_1","content":"ok"}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	got := gjson.GetBytes(out, "messages.0.tool_calls.0.function.arguments").String()
	if got != "{}" {
		t.Fatalf("arguments = %q, want %q", got, "{}")
	}
}

func TestNormalizeCodebuddyToolMessages_NonJSONArgumentsBecomeEmptyObject(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"run","arguments":"` + "`" + `"}}]},
			{"role":"tool","tool_call_id":"call_1","content":"ok"}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	got := gjson.GetBytes(out, "messages.0.tool_calls.0.function.arguments").String()
	if got != "{}" {
		t.Fatalf("arguments = %q, want %q", got, "{}")
	}
}

func TestNormalizeCodebuddyToolMessages_DropsEmptyAssistantMessage(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"user","content":"hi"},
			{"role":"assistant","content":null},
			{"role":"user","content":"again"}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	if n := len(gjson.GetBytes(out, "messages").Array()); n != 2 {
		t.Fatalf("messages length = %d, want 2; body=%s", n, string(out))
	}
}

func TestNormalizeCodebuddyToolMessages_KeepsToolCallAssistantMessage(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"run","arguments":"{}"}}]}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	if n := len(gjson.GetBytes(out, "messages").Array()); n != 1 {
		t.Fatalf("messages length = %d, want 1; body=%s", n, string(out))
	}
}

func TestNormalizeCodebuddyToolMessages_CallIDAlias(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"assistant","tool_calls":[{"id":"call_9","type":"function","function":{"name":"run","arguments":"{}"}}]},
			{"role":"tool","call_id":"call_9","content":"ok"}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	got := gjson.GetBytes(out, "messages.1.tool_call_id").String()
	if got != "call_9" {
		t.Fatalf("tool_call_id = %q, want %q", got, "call_9")
	}
}

func TestNormalizeCodebuddyToolMessages_AppendsUserAfterTrailingTool(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"run","arguments":"{}"}}]},
			{"role":"tool","tool_call_id":"call_1","content":"ok"}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	msgs := gjson.GetBytes(out, "messages").Array()
	if len(msgs) != 3 {
		t.Fatalf("messages length = %d, want 3; body=%s", len(msgs), string(out))
	}
	if got := msgs[len(msgs)-1].Get("role").String(); got != "user" {
		t.Fatalf("last role = %q, want user; body=%s", got, string(out))
	}
}

func TestNormalizeCodebuddyToolMessages_KeepsUserTrailingUnchanged(t *testing.T) {
	body := []byte(`{
		"messages":[
			{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"run","arguments":"{}"}}]},
			{"role":"tool","tool_call_id":"call_1","content":"ok"},
			{"role":"user","content":"next"}
		]
	}`)
	out, err := normalizeCodebuddyToolMessages(body)
	if err != nil {
		t.Fatalf("normalizeCodebuddyToolMessages() error = %v", err)
	}
	msgs := gjson.GetBytes(out, "messages").Array()
	if len(msgs) != 3 {
		t.Fatalf("messages length = %d, want 3; body=%s", len(msgs), string(out))
	}
	if got := msgs[len(msgs)-1].Get("role").String(); got != "user" {
		t.Fatalf("last role = %q, want user; body=%s", got, string(out))
	}
}
