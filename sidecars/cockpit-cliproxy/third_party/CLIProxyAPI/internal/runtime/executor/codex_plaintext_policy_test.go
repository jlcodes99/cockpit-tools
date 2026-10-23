package executor

import (
	"testing"

	"github.com/gorilla/websocket"
	"github.com/tidwall/gjson"
)

func TestCodexWebsocketPlaintextPolicyIsIndependentOfNamespaceAndConnectionState(t *testing.T) {
	conn := &websocket.Conn{}
	session := &codexWebsocketSession{conn: conn}
	response := []byte(`{"type":"function_call","namespace":"collaboration","name":"spawn_agent","arguments":"{}"}`)
	incremental := []byte(`{"previous_response_id":"resp-1","input":[]}`)
	check := func(request []byte, want bool) {
		t.Helper()
		policy := session.plaintextPolicyForSentRequest(conn, request)
		if marked := gjson.GetBytes(policy.MarkCalls(response), "encrypted_function_args").Raw == "[]"; marked != want {
			t.Fatalf("request=%s marked=%t want=%t", request, marked, want)
		}
	}
	check(plaintextMarkerTestRequest("false", false), true)
	session.setMultiAgentV2Optimized(conn, false)
	check(incremental, true)
	check(plaintextMarkerTestRequest("true", true), false)
	check(incremental, false)
	check(plaintextMarkerTestRequest("false", true), true)
	check([]byte(`{"previous_response_id":"resp-1","tools":[],"input":[]}`), true)
	check([]byte(`{"previous_response_id":"resp-1","tools":[{"type":"image_generation"}],"input":[]}`), true)
	check([]byte(`{"previous_response_id":"resp-1","input":[{"type":"additional_tools","tools":[{"type":"function","name":"unrelated","parameters":{}}]}]}`), true)
	check([]byte(`{"previous_response_id":"resp-1","tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}]}],"input":[]}`), false)
	check([]byte(`{"previous_response_id":"resp-1","tools":[{"type":"image_generation"}],"input":[]}`), false)
	check(incremental, false)
	check(plaintextMarkerTestRequest("false", true), true)
	check([]byte(`{"previous_response_id":"resp-1","input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}]}]}]}`), false)
	check(incremental, false)
	check(plaintextMarkerTestRequest("false", true), true)
	check([]byte(`{"previous_response_id":"resp-1","input":[{"type":"additional_tools","tools":null}]}`), false)
	check(incremental, false)
	check(plaintextMarkerTestRequest("false", false), true)
	check([]byte(`{"previous_response_id":"resp-1","tools":null,"input":[]}`), false)
	check(incremental, false)
	check(plaintextMarkerTestRequest("false", false), true)
	check([]byte(`{"input":[]}`), false)
	check(incremental, false)
	check(plaintextMarkerTestRequest("false", false), true)
	conn = &websocket.Conn{}
	session.conn = conn
	check(incremental, false)
}
