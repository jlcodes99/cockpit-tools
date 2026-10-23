package helps

import (
	"fmt"
	"testing"

	"github.com/tidwall/gjson"
)

func TestXAIAgentMessagesDoNotDependOnOptimizationOrUserAgent(t *testing.T) {
	original := []byte(`{"tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}]}],"input":[{"type":"agent_message","author":"/root","recipient":"/root/child","content":[{"type":"input_text","text":"continue"},{"type":"encrypted_content","encrypted_content":"legacy literal"}]}]}`)
	got := RewriteXAIAgentMessages(original)
	if gjson.GetBytes(got, "input.0.type").String() != "message" || gjson.GetBytes(got, "input.0.role").String() != "user" {
		t.Fatalf("private agent item was not adapted: %s", got)
	}
	if gjson.GetBytes(got, "input.0.content.1.text").String() != "legacy literal" || gjson.GetBytes(original, "input.0.type").String() != "agent_message" {
		t.Fatal("message bytes or source history changed")
	}
	if gjson.GetBytes(got, "tools.0.tools.0.parameters.properties.message.encrypted").Exists() {
		t.Fatal("unsupported encrypted schema option survived")
	}
}

func TestXAIPlaintextMessageMarkersPreserveActualEncryptedEvidence(t *testing.T) {
	for _, name := range []string{"spawn_agent", "send_message", "followup_task"} {
		for _, envelope := range []string{
			`{"type":"response.output_item.done","item":%s}`,
			`{"type":"response.completed","response":{"output":[%s]}}`,
			`{"output":[%s]}`,
		} {
			item := `{"type":"function_call","namespace":"collaboration","name":"` + name + `","arguments":"{\"message\":\"continue\"}"}`
			payload := []byte(fmt.Sprintf(envelope, item))
			got := NormalizeXAICollaborationToolCalls(payload)
			path := "item"
			if gjson.GetBytes(got, "response.output").Exists() {
				path = "response.output.0"
			}
			if gjson.GetBytes(got, "output").Exists() {
				path = "output.0"
			}
			if gjson.GetBytes(got, path+".encrypted_function_args").Raw != "[]" {
				t.Fatalf("plaintext marker missing: %s", got)
			}
		}
	}
	for _, evidence := range []string{`"encrypted_content":"real cipher"`, `"encrypted_function_args":["cipher"]`} {
		payload := []byte(`{"output":[{"type":"function_call","namespace":"collaboration","name":"spawn_agent","arguments":"{}",` + evidence + `}]}`)
		if got := NormalizeXAICollaborationToolCalls(payload); string(got) != string(payload) {
			t.Fatalf("explicit encrypted evidence changed: %s", got)
		}
	}
	partial := []byte(`{"type":"response.output_item.added","item":{"type":"function_call","namespace":"collaboration","name":"spawn_agent","arguments":""}}`)
	if got := NormalizeXAICollaborationToolCalls(partial); string(got) != string(partial) {
		t.Fatal("partial item changed")
	}
}
