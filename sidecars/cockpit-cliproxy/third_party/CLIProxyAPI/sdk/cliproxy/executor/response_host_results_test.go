package executor

import (
	"bytes"
	"fmt"
	"testing"
)

func TestHostResultsNeverConsumePendingCallsOrRewriteSavedIDs(t *testing.T) {
	for _, name := range []string{"create_thread", "send_message_to_thread"} {
		for _, id := range []string{"", `,"call_id":""`, `,"call_id":null`, `,"call_id":" "`, `,"call_id":123`, `,"call_id":"stale-call"`} {
			for _, output := range []string{`"Original host result\n  exact text  "`, `[{"type":"input_text","text":"Host payload:"},{"type":"encrypted_content","encrypted_content":"host-cipher"}]`} {
				body := []byte(fmt.Sprintf(`{"input":[{"type":"reasoning","encrypted_content":"original-cipher","summary":[]},{"type":"function_call","name":"read_file","call_id":"real-call","arguments":"{}"},{"type":"function_call_output","namespace":"codex_app","name":%q%s,"output":%s},{"type":"function_call_output","call_id":"real-call","output":"real result"}]}`, name, id, output))
				for _, preserveOpaque := range []bool{false, true} {
					if got := NormalizeResponsesCallIDs(body, preserveOpaque); !bytes.Equal(got, body) {
						t.Fatalf("host result changed or consumed unrelated call (name=%s id=%s continuity=%t): %s", name, id, preserveOpaque, got)
					}
				}
			}
		}
	}
}
