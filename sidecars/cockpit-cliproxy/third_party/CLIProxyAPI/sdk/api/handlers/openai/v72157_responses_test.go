package openai

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestV72157PrewarmFollowupPreservesWarmupInput(t *testing.T) {
	warmup := []byte(`{"type":"response.create","model":"gpt-test","generate":false,"input":[{"type":"message","role":"developer","content":"base"}]}`)
	followup := []byte(`{"type":"response.create","previous_response_id":"resp_prewarm","input":[{"type":"function_call_output","name":"heartbeat","output":"next"}]}`)
	normalized, saved, errMsg := normalizeResponsesWebsocketPrewarmFollowup(followup, warmup)
	if errMsg != nil {
		t.Fatalf("normalize: %v", errMsg.Error)
	}
	if got := len(gjson.GetBytes(normalized, "input").Array()); got != 2 {
		t.Fatalf("input count = %d, want 2; payload=%s", got, normalized)
	}
	if string(saved) != string(normalized) {
		t.Fatalf("saved transcript differs from normalized request")
	}
}

func TestV72157NamedStandaloneToolOutputIsPreserved(t *testing.T) {
	item, err := parseResponsesWebsocketInputItem([]byte(`{"type":"function_call_output","name":"heartbeat","output":"ok"}`))
	if err != nil {
		t.Fatalf("parse item: %v", err)
	}
	filtered, err := repairResponsesToolCallItems(nil, nil, "session", []responsesWebsocketInputItem{item}, false, false, nil, true)
	if err != nil {
		t.Fatalf("repair: %v", err)
	}
	if len(filtered) != 1 {
		t.Fatalf("filtered count = %d, want 1", len(filtered))
	}
}

func TestV72157CreateRejectsNonArrayInput(t *testing.T) {
	_, _, errMsg := normalizeResponseCreateRequest([]byte(`{"type":"response.create","input":"invalid"}`))
	if errMsg == nil {
		t.Fatal("non-array input was accepted")
	}
}
