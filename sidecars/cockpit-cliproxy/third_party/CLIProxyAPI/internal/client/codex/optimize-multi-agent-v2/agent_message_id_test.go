package multiagentv2

import (
	"context"
	"net/http"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/tidwall/gjson"
)

func TestCompatibilityAgentMessagePrivateID(t *testing.T) {
	const privateID = "amsg_01a15d7a-297f-7ac2-9b3b-e4d2c36571da"
	payload := []byte(`{"input":[{"type":"agent_message","id":"` + privateID + `","author":"/root/child","recipient":"/root","content":[{"type":"input_text","text":"actual child receipt"}]},{"type":"message","id":"msg_existing","role":"user","content":"continue"},{"type":"function_call","id":"fc_existing","call_id":"call_existing","name":"test","arguments":"{}"},{"type":"function_call_output","call_id":"call_existing","output":"keep output"}]}`)
	headers := http.Header{"User-Agent": {"Codex Desktop/0.163.0"}}
	for _, tc := range []struct {
		name    string
		rewrite func([]byte) []byte
	}{
		{"compatibility", func(body []byte) []byte {
			return RewriteCodexMultiAgentV2Input(context.Background(), headers, body, nil, true)
		}},
		{"optimization enabled", func(body []byte) []byte {
			return RewriteCodexMultiAgentV2Input(context.Background(), headers, body, &config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: true}}, true)
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := tc.rewrite(payload)
			if gjson.GetBytes(got, "input.0.type").String() != "message" || gjson.GetBytes(got, "input.0.role").String() != "user" {
				t.Fatalf("child receipt not projected: %s", got)
			}
			if gjson.GetBytes(got, "input.0.id").Exists() {
				t.Fatalf("private agent message ID reached standard Responses input: %s", got)
			}
			if gjson.GetBytes(got, "input.0.content.0.text").String() != "actual child receipt" {
				t.Fatalf("child receipt changed: %s", got)
			}
			for _, index := range []string{"1", "2", "3"} {
				if gjson.GetBytes(got, "input."+index).Raw != gjson.GetBytes(payload, "input."+index).Raw {
					t.Fatalf("unrelated input %s changed: %s", index, got)
				}
			}
			if string(tc.rewrite(got)) != string(got) {
				t.Fatal("projection is not idempotent")
			}
			if gjson.GetBytes(payload, "input.0.id").String() != privateID {
				t.Fatal("canonical input mutated")
			}
		})
	}
	for _, enabled := range []bool{false, true} {
		cfg := &config.Config{Codex: config.CodexConfig{OptimizeMultiAgentV2: enabled}}
		got := RewriteCodexMultiAgentV2Input(context.Background(), headers, payload, cfg)
		if gjson.GetBytes(got, "input.0.id").String() != privateID {
			t.Fatalf("non-compatibility item identity changed: %s", got)
		}
	}
	valid := []byte(`{"input":[{"type":"agent_message","id":"msg_valid","content":[{"type":"input_text","text":"keep"}]}]}`)
	if got := RewriteCodexMultiAgentV2Input(context.Background(), headers, valid, nil, true); gjson.GetBytes(got, "input.0.id").String() != "msg_valid" {
		t.Fatalf("valid message ID changed: %s", got)
	}
}
