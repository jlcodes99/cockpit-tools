package executor

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestNormalizeCodexCollaborationSpawnAgentModelOutputItemDone(t *testing.T) {
	payload := []byte(`{"type":"response.output_item.done","item":{"type":"function_call","name":"spawn_agent","namespace":"collaboration","arguments":"{\"model\":\"luna\",\"reasoning_effort\":\"max\"}"}}`)

	got := normalizeCodexCollaborationSpawnAgentModel(payload)
	arguments := gjson.GetBytes(got, "item.arguments").String()
	if model := gjson.Get(arguments, "model").String(); model != "gpt-5.6-luna" {
		t.Fatalf("model = %q, want gpt-5.6-luna: %s", model, got)
	}
	if effort := gjson.Get(arguments, "reasoning_effort").String(); effort != "max" {
		t.Fatalf("reasoning_effort = %q, want max: %s", effort, got)
	}
}

func TestNormalizeCodexCollaborationSpawnAgentModelCompletedOutput(t *testing.T) {
	payload := []byte(`{"type":"response.completed","response":{"output":[{"type":"function_call","name":"multi_agent_v1__spawn_agent","arguments":{"model":"sol","reasoning_effort":"high"}},{"type":"function_call","name":"collaboration__spawn_agent","arguments":"{\"model\":\"terra\"}"}]}}`)

	got := normalizeCodexCollaborationSpawnAgentModel(payload)
	if model := gjson.GetBytes(got, "response.output.0.arguments.model").String(); model != "gpt-5.6-sol" {
		t.Fatalf("first model = %q, want gpt-5.6-sol: %s", model, got)
	}
	secondArguments := gjson.GetBytes(got, "response.output.1.arguments").String()
	if model := gjson.Get(secondArguments, "model").String(); model != "gpt-5.6-terra" {
		t.Fatalf("second model = %q, want gpt-5.6-terra: %s", model, got)
	}
}

func TestNormalizeCodexCollaborationSpawnAgentModelLeavesOtherCallsUnchanged(t *testing.T) {
	payload := []byte(`{"type":"response.output_item.done","item":{"type":"function_call","name":"other_tool","arguments":"{\"model\":\"luna\"}"}}`)

	got := normalizeCodexCollaborationSpawnAgentModel(payload)
	if string(got) != string(payload) {
		t.Fatalf("unrelated tool call changed: got=%s want=%s", got, payload)
	}
}

func TestNormalizeCodexCollaborationSpawnAgentModelLeavesFullModelIDUnchanged(t *testing.T) {
	payload := []byte(`{"type":"response.output_item.done","item":{"type":"function_call","name":"spawn_agent","namespace":"collaboration","arguments":"{\"model\":\"gpt-5.6-luna\",\"reasoning_effort\":\"max\"}"}}`)

	got := normalizeCodexCollaborationSpawnAgentModel(payload)
	if string(got) != string(payload) {
		t.Fatalf("full model ID changed: got=%s want=%s", got, payload)
	}
}
