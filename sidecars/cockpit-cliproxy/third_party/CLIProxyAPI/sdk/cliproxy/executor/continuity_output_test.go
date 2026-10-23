package executor

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"testing"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

type continuityOutputHook struct {
	continuityTestHook
	observeError error
}

func (h *continuityOutputHook) Observe(_ context.Context, request ContinuityObserveRequest) (ContinuityObserveResponse, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.observed = append(h.observed, request)
	terminal := false
	for _, event := range request.Events {
		switch gjson.GetBytes(event, "type").String() {
		case "response.completed", "response.done", "response.failed", "response.incomplete", "response.cancelled", "response.error", "error":
			terminal = true
		}
	}
	return ContinuityObserveResponse{NextSequence: request.Sequence + 1, Terminal: terminal}, h.observeError
}

func newContinuityOutputAttempt(t *testing.T, hook *continuityOutputHook) *ContinuityAttempt {
	t.Helper()
	ctx, _, err := PrepareContinuity(WithContinuityHook(context.Background(), hook), ContinuityPrepareRequest{Body: []byte(`{"input":[]}`), AllowRebuild: true})
	if err != nil {
		t.Fatal(err)
	}
	attempt, _, _, err := BindContinuity(ctx, "account", http.Header{}, []byte(`{"input":[]}`))
	if err != nil {
		t.Fatal(err)
	}
	return attempt
}

func continuityOutputDone(index int, item string) []byte {
	return []byte(fmt.Sprintf(`{"type":"response.output_item.done","output_index":%d,"item":%s}`, index, item))
}

const continuityCompleteMessage = `{"id":"message-1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"synthetic answer","annotations":[]}],"phase":"final_answer","unknown":{"keep":[1,true,"原样"]}}`

func observeContinuityOutput(t *testing.T, attempt *ContinuityAttempt, event []byte) {
	t.Helper()
	before := bytes.Clone(event)
	if err := attempt.Observe(context.Background(), nil, event); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(event, before) {
		t.Fatal("observation changed upstream event bytes")
	}
}

func TestContinuityOutputNormalizesOnlySuccessfulEmptyTerminal(t *testing.T) {
	for _, kind := range []string{"response.completed", "response.done"} {
		for _, output := range []string{"", `,"output":[]`} {
			t.Run(kind+output, func(t *testing.T) {
				hook := &continuityOutputHook{}
				attempt := newContinuityOutputAttempt(t, hook)
				items := []string{
					continuityCompleteMessage,
					`{"id":"reasoning-1","type":"reasoning","encrypted_content":"synthetic-cipher-unchanged","summary":[],"signature":"synthetic-signature"}`,
					`{"id":"call-1","type":"function_call","status":"completed","call_id":"call-id","namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"synthetic task\"}","encrypted_function_args":["synthetic-real-cipher"],"unknown":[2]}`,
				}
				for _, index := range []int{2, 0, 1} {
					observeContinuityOutput(t, attempt, continuityOutputDone(index, items[index]))
				}
				terminal := []byte(fmt.Sprintf(`{"type":%q,"sequence_number":17,"unknown":{"terminal":true},"response":{"id":"response-1","status":"completed","error":null%s,"unknown":{"preserve":"yes"}}}`, kind, output))
				observeContinuityOutput(t, attempt, terminal)
				actual := hook.observed[len(hook.observed)-1].Events[0]
				outputItems := gjson.GetBytes(actual, "response.output").Array()
				if len(outputItems) != len(items) {
					t.Fatalf("normalized items=%d, want %d", len(outputItems), len(items))
				}
				for i, item := range outputItems {
					if item.Raw != items[i] {
						t.Fatalf("item %d changed or lost opaque fields", i)
					}
				}
				if gjson.GetBytes(actual, "unknown.terminal").Bool() != true || gjson.GetBytes(actual, "response.unknown.preserve").String() != "yes" || gjson.GetBytes(actual, "sequence_number").Int() != 17 {
					t.Fatal("terminal metadata changed")
				}
				if len(attempt.output.items) != 0 {
					t.Fatal("terminal retained aggregation bytes")
				}
				if err := attempt.Finish(context.Background(), nil); err != nil {
					t.Fatal(err)
				}
				if got := hook.finished[0].Status; got != "succeeded" {
					t.Fatalf("finish status=%s", got)
				}
			})
		}
	}
}

func TestContinuityOutputCollectsAfterAuthoritativeEventTransform(t *testing.T) {
	hook := &continuityOutputHook{}
	attempt := newContinuityOutputAttempt(t, hook)
	attempt.SetEventTransform(func(event []byte) []byte {
		if gjson.GetBytes(event, "type").String() == "response.output_item.done" {
			marked, err := sjson.SetRawBytes(event, "item.encrypted_function_args", []byte(`[]`))
			if err != nil {
				t.Fatal(err)
			}
			return marked
		}
		return event
	})
	item := `{"id":"call","type":"function_call","call_id":"call","name":"send_message","namespace":"collaboration","arguments":"{\"message\":\"synthetic plaintext\"}","extra":"preserve"}`
	observeContinuityOutput(t, attempt, continuityOutputDone(0, item))
	observeContinuityOutput(t, attempt, []byte(`{"type":"response.completed","response":{"id":"response","status":"completed","output":[]}}`))
	done := gjson.GetBytes(hook.observed[0].Events[0], "item")
	terminalItem := gjson.GetBytes(hook.observed[1].Events[0], "response.output.0")
	if terminalItem.Raw != done.Raw || terminalItem.Get("encrypted_function_args").Raw != "[]" || terminalItem.Get("extra").String() != "preserve" {
		t.Fatal("terminal did not retain the authoritative observed done item")
	}
}

func TestContinuityOutputPreservesCompleteProtocolItems(t *testing.T) {
	items := []string{
		`{"id":"agent","type":"agent_message","author":"child","recipient":"parent","content":[{"type":"encrypted_content","encrypted_content":"synthetic-agent-cipher","signature":"synthetic-signature"}],"metadata":{"keep":true}}`,
		`{"id":"image","type":"image_generation_call","status":"completed","result":"synthetic-image-data","revised_prompt":"synthetic prompt","unknown":true}`,
		`{"id":"custom","type":"custom_tool_call","call_id":"call","name":"tool","input":"synthetic freeform input"}`,
		`{"id":"function-output","type":"function_call_output","call_id":"call","output":"synthetic result"}`,
		`{"id":"custom-output","type":"custom_tool_call_output","call_id":"call","output":[{"type":"output_text","text":"synthetic result"}]}`,
		`{"id":"additional","type":"additional_tools","role":"developer","tools":[{"type":"function","name":"synthetic"}]}`,
		`{"id":"search-output","type":"tool_search_output","status":"completed","execution":"server","tools":[{"type":"function","name":"synthetic"}]}`,
		`{"id":"search-call","type":"tool_search_call","status":"completed","execution":"server","arguments":{"query":"synthetic"}}`,
	}
	hook := &continuityOutputHook{}
	attempt := newContinuityOutputAttempt(t, hook)
	for index, item := range items {
		observeContinuityOutput(t, attempt, continuityOutputDone(index, item))
	}
	observeContinuityOutput(t, attempt, []byte(`{"type":"response.completed","response":{"id":"response","status":"completed","output":[]}}`))
	actual := gjson.GetBytes(hook.observed[len(hook.observed)-1].Events[0], "response.output").Array()
	if len(actual) != len(items) {
		t.Fatalf("preserved protocol items=%d, want %d", len(actual), len(items))
	}
	for index, item := range actual {
		if item.Raw != items[index] {
			t.Fatalf("protocol item %d changed", index)
		}
	}
}

func TestContinuityOutputRejectsInvalidOrIncompleteDoneEvidence(t *testing.T) {
	valid := continuityOutputDone(0, continuityCompleteMessage)
	cases := []struct {
		name   string
		events [][]byte
	}{
		{"missing-index", [][]byte{[]byte(`{"type":"response.output_item.done","item":` + continuityCompleteMessage + `}`)}},
		{"negative-index", [][]byte{continuityOutputDone(-1, continuityCompleteMessage)}},
		{"out-of-range-index", [][]byte{continuityOutputDone(4096, continuityCompleteMessage)}},
		{"fractional-index", [][]byte{[]byte(`{"type":"response.output_item.done","output_index":0.5,"item":` + continuityCompleteMessage + `}`)}},
		{"integer-as-float", [][]byte{[]byte(`{"type":"response.output_item.done","output_index":0.0,"item":` + continuityCompleteMessage + `}`)}},
		{"string-index", [][]byte{[]byte(`{"type":"response.output_item.done","output_index":"0","item":` + continuityCompleteMessage + `}`)}},
		{"overflow-index", [][]byte{[]byte(`{"type":"response.output_item.done","output_index":18446744073709551616,"item":` + continuityCompleteMessage + `}`)}},
		{"missing-zero", [][]byte{continuityOutputDone(1, continuityCompleteMessage)}},
		{"index-gap", [][]byte{valid, continuityOutputDone(2, continuityCompleteMessage)}},
		{"conflicting-duplicate", [][]byte{valid, continuityOutputDone(0, `{"id":"different","type":"message","role":"assistant","content":"different"}`)}},
		{"null-item", [][]byte{continuityOutputDone(0, `null`)}},
		{"empty-item", [][]byte{continuityOutputDone(0, `{}`)}},
		{"missing-id", [][]byte{continuityOutputDone(0, `{"type":"message","role":"assistant","content":"answer"}`)}},
		{"missing-type", [][]byte{continuityOutputDone(0, `{"id":"message","role":"assistant","content":"answer"}`)}},
		{"missing-message-content", [][]byte{continuityOutputDone(0, `{"id":"message","type":"message","role":"assistant"}`)}},
		{"empty-message-content", [][]byte{continuityOutputDone(0, `{"id":"message","type":"message","role":"assistant","content":[]}`)}},
		{"incomplete-message-part", [][]byte{continuityOutputDone(0, `{"id":"message","type":"message","role":"assistant","content":[{"type":"output_text"}]}`)}},
		{"invalid-call-arguments", [][]byte{continuityOutputDone(0, `{"id":"call","type":"function_call","call_id":"call","name":"tool","arguments":"{"}`)}},
		{"item-in-progress", [][]byte{continuityOutputDone(0, `{"id":"message","type":"message","status":"in_progress","role":"assistant","content":"unfinished"}`)}},
		{"unknown-item-type", [][]byte{continuityOutputDone(0, `{"id":"item","type":"unknown","content":"unknown"}`)}},
		{"prior-error", [][]byte{valid, []byte(`{"type":"response.error","error":{"code":"synthetic_failure"}}`)}},
		{"no-done-only-added", [][]byte{[]byte(`{"type":"response.output_item.added","output_index":0,"item":` + continuityCompleteMessage + `}`)}},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			hook := &continuityOutputHook{}
			attempt := newContinuityOutputAttempt(t, hook)
			for _, event := range test.events {
				observeContinuityOutput(t, attempt, event)
			}
			terminal := []byte(`{"type":"response.completed","response":{"id":"response","status":"completed","output":[]}}`)
			observeContinuityOutput(t, attempt, terminal)
			actual := hook.observed[len(hook.observed)-1].Events[0]
			if !bytes.Equal(actual, terminal) {
				t.Fatal("invalid evidence synthesized a terminal transcript")
			}
			if len(attempt.output.items) != 0 {
				t.Fatal("terminal retained aggregation bytes")
			}
		})
	}
}

func TestContinuityOutputNeverOverridesAuthoritativeOrFailedTerminal(t *testing.T) {
	cases := []struct {
		name, terminal string
	}{
		{"nonempty-output", `{"type":"response.completed","response":{"id":"response","status":"completed","output":[{"id":"other","type":"message","role":"assistant","content":"different"}]}}`},
		{"missing-status", `{"type":"response.completed","response":{"id":"response","output":[]}}`},
		{"missing-id", `{"type":"response.completed","response":{"status":"completed","output":[]}}`},
		{"blank-id", `{"type":"response.completed","response":{"id":"  ","status":"completed","output":[]}}`},
		{"nonstring-id", `{"type":"response.completed","response":{"id":42,"status":"completed","output":[]}}`},
		{"error", `{"type":"response.completed","error":{"code":"synthetic_failure"},"response":{"id":"response","status":"completed","output":[]}}`},
		{"response-error", `{"type":"response.completed","response":{"id":"response","status":"completed","error":{},"output":[]}}`},
		{"failed", `{"type":"response.failed","response":{"id":"response","status":"failed","output":[]}}`},
		{"incomplete", `{"type":"response.incomplete","response":{"id":"response","status":"incomplete","output":[]}}`},
		{"cancelled", `{"type":"response.cancelled","response":{"id":"response","status":"cancelled","output":[]}}`},
		{"false-success-status", `{"type":"response.completed","response":{"id":"response","status":"incomplete","output":[]}}`},
		{"null-output", `{"type":"response.completed","response":{"id":"response","status":"completed","output":null}}`},
		{"object-output", `{"type":"response.completed","response":{"id":"response","status":"completed","output":{}}}`},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			hook := &continuityOutputHook{}
			attempt := newContinuityOutputAttempt(t, hook)
			observeContinuityOutput(t, attempt, continuityOutputDone(0, continuityCompleteMessage))
			terminal := []byte(test.terminal)
			observeContinuityOutput(t, attempt, terminal)
			actual := hook.observed[len(hook.observed)-1].Events[0]
			if !bytes.Equal(actual, terminal) {
				t.Fatal("authoritative terminal was changed")
			}
			if len(attempt.output.items) != 0 {
				t.Fatal("terminal retained aggregation bytes")
			}
		})
	}
}

func TestContinuityOutputDuplicateAndAttemptIsolation(t *testing.T) {
	hook := &continuityOutputHook{}
	first := newContinuityOutputAttempt(t, hook)
	second := newContinuityOutputAttempt(t, hook)
	observeContinuityOutput(t, first, continuityOutputDone(0, continuityCompleteMessage))
	var reordered map[string]json.RawMessage
	if err := json.Unmarshal([]byte(continuityCompleteMessage), &reordered); err != nil {
		t.Fatal(err)
	}
	duplicate, err := json.Marshal(reordered)
	if err != nil {
		t.Fatal(err)
	}
	observeContinuityOutput(t, first, continuityOutputDone(0, string(duplicate)))
	terminal := []byte(`{"type":"response.completed","response":{"id":"response","status":"completed","output":[]}}`)
	observeContinuityOutput(t, second, terminal)
	if !bytes.Equal(hook.observed[len(hook.observed)-1].Events[0], terminal) {
		t.Fatal("independent attempt borrowed another send's output")
	}
	observeContinuityOutput(t, first, terminal)
	if got := gjson.GetBytes(hook.observed[len(hook.observed)-1].Events[0], "response.output.0").Raw; got != continuityCompleteMessage {
		t.Fatal("identical duplicate lost or changed the original item")
	}
	observeContinuityOutput(t, first, continuityOutputDone(0, continuityCompleteMessage))
	observeContinuityOutput(t, first, terminal)
	if !bytes.Equal(hook.observed[len(hook.observed)-1].Events[0], terminal) {
		t.Fatal("late terminal reused released aggregation")
	}
}

func TestContinuityOutputCancellationEOFAndObserverFailureReleaseItems(t *testing.T) {
	for _, mode := range []string{"eof", "upstream-error", "cancelled", "observer-error", "invalid-event"} {
		t.Run(mode, func(t *testing.T) {
			hook := &continuityOutputHook{}
			attempt := newContinuityOutputAttempt(t, hook)
			observeContinuityOutput(t, attempt, continuityOutputDone(0, continuityCompleteMessage))
			ctx := context.Background()
			var cause error
			switch mode {
			case "upstream-error":
				cause = errors.New("synthetic upstream failure")
			case "cancelled":
				var cancel context.CancelFunc
				ctx, cancel = context.WithCancel(ctx)
				cancel()
				terminal := []byte(`{"type":"response.completed","response":{"id":"response","status":"completed","output":[]}}`)
				if err := attempt.Observe(ctx, nil, terminal); err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(hook.observed[len(hook.observed)-1].Events[0], terminal) {
					t.Fatal("cancelled attempt synthesized success")
				}
			case "observer-error":
				hook.observeError = errors.New("synthetic observer failure")
				cause = attempt.Observe(ctx, nil, []byte(`{"type":"response.output_text.delta","delta":"synthetic"}`))
				if cause == nil {
					t.Fatal("observer error was lost")
				}
			case "invalid-event":
				cause = attempt.Observe(ctx, nil, []byte(`{"type":`))
				if cause == nil {
					t.Fatal("invalid event was accepted")
				}
			}
			if err := attempt.Finish(ctx, cause); err != nil {
				t.Fatal(err)
			}
			if len(attempt.output.items) != 0 {
				t.Fatal("finished attempt retained aggregation bytes")
			}
			if hook.finished[0].Status == "succeeded" {
				t.Fatal("EOF, cancellation or failure claimed success")
			}
		})
	}
}
