package multiagentv2

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestAgentRoutingMetadataBecomesPortableContent(t *testing.T) {
	for _, author := range []string{`"/root/config_docs"`, `{"role":"user"}`} {
		payload := []byte(`{"input":[{"type":"agent_message","author":` + author + `,"recipient":"/root","content":[{"type":"input_text","text":"original task"}]},{"type":"function_call_output","call_id":"call_1","output":"unchanged"}]}`)
		got := rewriteCodexAgentMessageInput(payload)
		item := gjson.GetBytes(got, "input.0")
		if item.Get("type").String() != "message" || item.Get("role").String() != "user" || item.Get("author").Exists() || item.Get("recipient").Exists() {
			t.Fatalf("nonportable message: %s", got)
		}
		if item.Get("content.0.text").String() != "original task" || item.Get("content.1.text").String() != `Agent routing metadata: {"author":`+author+`,"recipient":"/root"}` {
			t.Fatalf("lost content or routing metadata: %s", got)
		}
		if gjson.GetBytes(got, "input.1").Raw != gjson.GetBytes(payload, "input.1").Raw {
			t.Fatalf("unrelated tool history changed: %s", got)
		}
		if again := rewriteCodexAgentMessageInput(got); string(again) != string(got) {
			t.Fatalf("conversion is not idempotent: %s", again)
		}
	}
}
