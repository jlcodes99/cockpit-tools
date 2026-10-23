package auth

import (
	"errors"
	"testing"

	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func TestCapabilityFallbackRequiresExplicitAdmissionAndPortablePairedHistory(t *testing.T) {
	plain := `{"input":[{"role":"user","content":"use tools"}],"tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]}`
	rejected := cliproxyexecutor.UpstreamRequestRejected(&Error{HTTPStatus: 400, Message: `{"error":{"code":"unsupported_tool"}}`})
	for _, tc := range []struct {
		name, body string
		opts       cliproxyexecutor.Options
		err        error
		want       bool
	}{
		{name: "explicit tool rejection", body: plain, err: rejected, want: true},
		{name: "duplicate history cannot hide private state", body: `{"input":[{"encrypted_content":"private"}],"input":"hi","tools":[{"type":"function"}]}`, err: rejected},
		{name: "mixed case histories are ambiguous", body: `{"INPUT":[{"encrypted_content":"private"}],"input":"hi","tools":[{"type":"function"}]}`, err: rejected},
		{name: "execution error has no admission marker", body: plain, err: &Error{HTTPStatus: 400, Message: `{"error":{"code":"unsupported_tool"}}`}},
		{name: "unstructured text is not evidence", body: plain, err: cliproxyexecutor.UpstreamRequestRejected(&Error{HTTPStatus: 400, Message: "unsupported tool"})},
		{name: "ordinary parameter error", body: plain, err: cliproxyexecutor.UpstreamRequestRejected(&Error{HTTPStatus: 400, Message: `{"error":{"code":"invalid_value","param":"temperature"}}`})},
		{name: "tool schema error", body: plain, err: cliproxyexecutor.UpstreamRequestRejected(&Error{HTTPStatus: 400, Message: `{"error":{"code":"unsupported_value","param":"tools[0].parameters"}}`})},
		{name: "tool type rejection", body: plain, err: cliproxyexecutor.UpstreamRequestRejected(&Error{HTTPStatus: 400, Message: `{"error":{"code":"unsupported_value","param":"tools[0].type"}}`}), want: true},
		{name: "no requested tool", body: `{"input":"hi"}`, err: rejected},
		{name: "hard pin", body: plain, opts: cliproxyexecutor.Options{Metadata: map[string]any{cliproxyexecutor.PinnedAuthMetadataKey: "a"}}, err: rejected},
		{name: "original history cannot be hidden", body: plain, opts: cliproxyexecutor.Options{OriginalRequest: []byte(`{"input":"hi","previous_response_id":"resp-private"}`)}, err: rejected},
		{name: "unresolved conversation", body: `{"input":"hi","conversation":{"id":"private"},"tools":[{"type":"function"}]}`, err: rejected},
		{name: "opaque reasoning", body: `{"input":[{"type":"reasoning","encrypted_content":"opaque"}],"tools":[{"type":"function"}]}`, err: rejected},
		{name: "uploaded resource", body: `{"input":[{"type":"input_file","file_id":"file-private"}],"tools":[{"type":"function"}]}`, err: rejected},
		{name: "unpaired result", body: `{"input":[{"type":"function_call_output","call_id":"missing","output":"result"}],"tools":[{"type":"function"}]}`, err: rejected},
		{name: "unpaired call", body: `{"input":[{"type":"function_call","call_id":"call","name":"lookup","arguments":"{}"}],"tools":[{"type":"function"}]}`, err: rejected},
		{name: "paired history", body: `{"input":[{"type":"function_call","call_id":"call","name":"lookup","arguments":"{}"},{"type":"function_call_output","call_id":"call","output":"result"}],"tools":[{"type":"function"}]}`, err: rejected, want: true},
		{name: "paired chat history", body: `{"messages":[{"role":"assistant","tool_calls":[{"id":"call","type":"function","function":{"name":"lookup","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call","content":"result"}],"tools":[{"type":"function"}]}`, err: rejected, want: true},
		{name: "nested opaque result", body: `{"input":[{"type":"function_call","call_id":"call"},{"type":"function_call_output","call_id":"call","output":{"encrypted_content":"opaque"}}],"tools":[{"type":"function"}]}`, err: rejected},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := canFallbackCapability(cliproxyexecutor.Request{Payload: []byte(tc.body)}, tc.opts, tc.err)
			if got != tc.want {
				t.Fatalf("fallback=%v want=%v", got, tc.want)
			}
		})
	}
	if canFallbackCapability(cliproxyexecutor.Request{Payload: []byte(plain)}, cliproxyexecutor.Options{}, errors.New("timeout")) {
		t.Fatal("ambiguous execution was replayable")
	}
}
