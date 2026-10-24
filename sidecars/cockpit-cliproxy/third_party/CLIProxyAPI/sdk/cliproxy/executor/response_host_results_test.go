package executor

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/tidwall/gjson"
)

func TestHostResultsNeverConsumePendingCallsOrRewriteSavedIDs(t *testing.T) {
	for _, namespace := range []string{"codex_app", "codex_tui"} {
		for _, name := range []string{"create_thread", "send_message_to_thread"} {
			t.Run(namespace+"/"+name, func(t *testing.T) {
				for _, id := range []string{"", `,"call_id":""`, `,"call_id":null`, `,"call_id":" "`, `,"call_id":123`, `,"call_id":"stale-call"`} {
					for _, output := range []string{`"Original host result\n  exact text  "`, `[{"type":"input_text","text":"Host payload:"},{"type":"encrypted_content","encrypted_content":"host-cipher"}]`} {
						body := []byte(fmt.Sprintf(`{"input":[{"type":"reasoning","encrypted_content":"original-cipher","summary":[]},{"type":"function_call","name":"read_file","call_id":"real-call","arguments":"{}"},{"type":"function_call_output","namespace":%q,"name":%q%s,"output":%s},{"type":"function_call_output","call_id":"real-call","output":"real result"}]}`, namespace, name, id, output))
						for _, preserveOpaque := range []bool{false, true} {
							if got := NormalizeResponsesCallIDs(body, preserveOpaque); !bytes.Equal(got, body) {
								t.Fatalf("host result changed or consumed unrelated call (namespace=%s name=%s id=%s continuity=%t): %s", namespace, name, id, preserveOpaque, got)
							}
						}
					}
				}
			})
		}
	}
}

func TestHostResultsOnlyMatchExactEnvelopes(t *testing.T) {
	for _, namespace := range []string{"codex_app", "codex_tui"} {
		for _, name := range []string{"create_thread", "send_message_to_thread"} {
			item := gjson.Parse(fmt.Sprintf(`{"type":"function_call_output","namespace":%q,"name":%q,"output":"host result"}`, namespace, name))
			if !codexHostResult(item) {
				t.Errorf("known host result was not recognized: %s", item.Raw)
			}
		}
	}
	for _, item := range []string{
		`{"type":"function_call_output","name":"create_thread"}`,
		`{"type":"function_call_output","namespace":"other","name":"create_thread"}`,
		`{"type":"function_call_output","namespace":" codex_tui ","name":"create_thread"}`,
		`{"type":"function_call_output","namespace":"codex_tui.extra","name":"create_thread"}`,
		`{"type":"function_call_output","namespace":"CODEX_TUI","name":"create_thread"}`,
		`{"type":"function_call_output","namespace":"codex_tui","name":"unknown"}`,
		`{"type":"function_call_output","namespace":"codex_tui","name":"codex_tui__create_thread"}`,
		`{"type":"function_call_output","namespace":"codex_tui","name":" create_thread"}`,
		`{"type":"custom_tool_call_output","namespace":"codex_tui","name":"create_thread"}`,
		`{"type":" function_call_output ","namespace":"codex_tui","name":"create_thread"}`,
	} {
		if codexHostResult(gjson.Parse(item)) {
			t.Errorf("unexpected host-result exemption: %s", item)
		}
	}
}
