package executor

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestIsCompleteJSONObject(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want bool
	}{
		{name: "complete object", in: `{"command":"echo hello"}`, want: true},
		{name: "complete object with whitespace", in: `  {"a":1}  `, want: true},
		{name: "nested object", in: `{"a":{"b":[1,2,3]}}`, want: true},
		{name: "partial fragment missing brace", in: `{"command":`, want: false},
		{name: "partial fragment leading quote", in: `"echo hello"}`, want: false},
		{name: "array", in: `[1,2,3]`, want: false},
		{name: "empty string", in: ``, want: false},
		{name: "whitespace only", in: `   `, want: false},
		{name: "null", in: `null`, want: false},
		{name: "scalar", in: `42`, want: false},
		{name: "two concatenated objects", in: `{"a":1}{"a":1}`, want: false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := isCompleteJSONObject(tc.in); got != tc.want {
				t.Fatalf("isCompleteJSONObject(%q) = %v, want %v", tc.in, got, tc.want)
			}
		})
	}
}

func TestMergeToolCallFragments_FullSnapshotDedupe(t *testing.T) {
	fragments := []gjson.Result{
		gjson.Parse(`{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{\"command\":\"echo hello\"}"}}`),
		gjson.Parse(`{"index":0,"function":{"arguments":"{\"command\":\"echo hello\"}"}}`),
	}
	out := mergeToolCallFragments(fragments)
	if len(out) != 1 {
		t.Fatalf("len(out) = %d, want 1", len(out))
	}
	fn, ok := out[0]["function"].(map[string]any)
	if !ok {
		t.Fatalf("function is %T, want map[string]any", out[0]["function"])
	}
	args, _ := fn["arguments"].(string)
	if args != `{"command":"echo hello"}` {
		t.Fatalf("arguments = %q, want a single complete object", args)
	}
}

func TestMergeToolCallFragments_FullSnapshotReplacesChangedValue(t *testing.T) {
	fragments := []gjson.Result{
		gjson.Parse(`{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{\"command\":\"echo\"}"}}`),
		gjson.Parse(`{"index":0,"function":{"arguments":"{\"command\":\"echo hello\"}"}}`),
	}
	out := mergeToolCallFragments(fragments)
	fn, ok := out[0]["function"].(map[string]any)
	if !ok {
		t.Fatalf("function is %T, want map[string]any", out[0]["function"])
	}
	args, _ := fn["arguments"].(string)
	if args != `{"command":"echo hello"}` {
		t.Fatalf("arguments = %q, want latest snapshot", args)
	}
}

func TestMergeToolCallFragments_IncrementalAppend(t *testing.T) {
	fragments := []gjson.Result{
		gjson.Parse(`{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{\"command\":"}}`),
		gjson.Parse(`{"index":0,"function":{"arguments":"\"echo hello\"}"}}`),
	}
	out := mergeToolCallFragments(fragments)
	fn, ok := out[0]["function"].(map[string]any)
	if !ok {
		t.Fatalf("function is %T, want map[string]any", out[0]["function"])
	}
	args, _ := fn["arguments"].(string)
	if args != `{"command":"echo hello"}` {
		t.Fatalf("arguments = %q, want concatenated fragments", args)
	}
}

func TestMergeToolCallFragments_EmptyArgumentsDefaultsToEmptyObject(t *testing.T) {
	fragments := []gjson.Result{
		gjson.Parse(`{"index":0,"id":"call_1","type":"function","function":{"name":"noop"}}`),
	}
	out := mergeToolCallFragments(fragments)
	fn, ok := out[0]["function"].(map[string]any)
	if !ok {
		t.Fatalf("function is %T, want map[string]any", out[0]["function"])
	}
	args, _ := fn["arguments"].(string)
	if args != `{}` {
		t.Fatalf("arguments = %q, want %q", args, "{}")
	}
}

func TestMergeToolCallFragments_EmptySnapshotDoesNotClobberRealArgs(t *testing.T) {
	fragments := []gjson.Result{
		gjson.Parse(`{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{\"command\":\"ls\"}"}}`),
		gjson.Parse(`{"index":0,"function":{"arguments":"{}"}}`),
	}
	out := mergeToolCallFragments(fragments)
	fn, ok := out[0]["function"].(map[string]any)
	if !ok {
		t.Fatalf("function is %T, want map[string]any", out[0]["function"])
	}
	args, _ := fn["arguments"].(string)
	if args != `{"command":"ls"}` {
		t.Fatalf("arguments = %q, want real args (empty snapshot must not clobber)", args)
	}
}

func TestMergeToolCallFragments_EmptySnapshotThenRealArgs(t *testing.T) {
	fragments := []gjson.Result{
		gjson.Parse(`{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{}"}}`),
		gjson.Parse(`{"index":0,"function":{"arguments":"{\"command\":\"ls\"}"}}`),
	}
	out := mergeToolCallFragments(fragments)
	fn, ok := out[0]["function"].(map[string]any)
	if !ok {
		t.Fatalf("function is %T, want map[string]any", out[0]["function"])
	}
	args, _ := fn["arguments"].(string)
	if args != `{"command":"ls"}` {
		t.Fatalf("arguments = %q, want real args (leading empty snapshot must be skipped)", args)
	}
}

func TestMergeToolCallFragments_IDKeepsFirstNonEmpty(t *testing.T) {
	fragments := []gjson.Result{
		gjson.Parse(`{"index":0,"id":"tooluse_abc","type":"function","function":{"name":"shell","arguments":"{}"}}`),
		gjson.Parse(`{"index":0,"id":"tooluse_xyz","function":{"arguments":"{}"}}`),
	}
	out := mergeToolCallFragments(fragments)
	id, _ := out[0]["id"].(string)
	if id != "call_abc" {
		t.Fatalf("id = %q, want %q (first non-empty, normalized)", id, "call_abc")
	}
}

func TestStreamToolCallBuffer_FullSnapshotOverwrite(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	// Growing full snapshots: the upstream re-sends the complete accumulated
	// arguments on each delta. The buffer must overwrite, not concatenate.
	l1 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"tooluse_abc","type":"function","function":{"name":"shell","arguments":"{\"command\":\"echo\"}"}}]},"finish_reason":null}]}`)
	l2 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\":\"echo hello\"}"}}]},"finish_reason":null}]}`)

	buf.Consume(l1)
	buf.Consume(l2)

	chunk := buf.BuildToolCallsChunk()
	if chunk == nil {
		t.Fatal("BuildToolCallsChunk() = nil, want a chunk")
	}
	got := gjson.GetBytes(chunk, "choices.0.delta.tool_calls.0.function.arguments").String()
	if got != `{"command":"echo hello"}` {
		t.Fatalf("arguments = %q, want the last (overwritten) snapshot, not a concatenation", got)
	}
	if n := len(gjson.GetBytes(chunk, "choices.0.delta.tool_calls").Array()); n != 1 {
		t.Fatalf("tool_calls length = %d, want 1", n)
	}
}

func TestStreamToolCallBuffer_IncrementalAppend(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	l1 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{\"command\":"}}]},"finish_reason":null}]}`)
	l2 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"echo hello\"}"}}]},"finish_reason":null}]}`)

	buf.Consume(l1)
	buf.Consume(l2)

	got := gjson.GetBytes(buf.BuildToolCallsChunk(), "choices.0.delta.tool_calls.0.function.arguments").String()
	if got != `{"command":"echo hello"}` {
		t.Fatalf("arguments = %q, want concatenated fragments", got)
	}
}

func TestStreamToolCallBuffer_IDStableAndNormalized(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	l1 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"tooluse_abc","type":"function","function":{"name":"shell","arguments":"{}"}}]},"finish_reason":null}]}`)
	// A later chunk sends a different id; the buffer must keep the first one.
	l2 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"tooluse_xyz","function":{"arguments":"{}"}}]},"finish_reason":null}]}`)

	buf.Consume(l1)
	buf.Consume(l2)

	got := gjson.GetBytes(buf.BuildToolCallsChunk(), "choices.0.delta.tool_calls.0.id").String()
	if got != "call_abc" {
		t.Fatalf("id = %q, want %q (first non-empty, normalized)", got, "call_abc")
	}
}

func TestStreamToolCallBuffer_EmptyArgumentsDefaultsToEmptyObject(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	line := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"noop"}}]},"finish_reason":null}]}`)
	buf.Consume(line)

	got := gjson.GetBytes(buf.BuildToolCallsChunk(), "choices.0.delta.tool_calls.0.function.arguments").String()
	if got != "{}" {
		t.Fatalf("arguments = %q, want %q", got, "{}")
	}
}

func TestStreamToolCallBuffer_ContentNullInConsolidatedChunk(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	line := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{}"}}]},"finish_reason":null}]}`)
	buf.Consume(line)

	content := gjson.GetBytes(buf.BuildToolCallsChunk(), "choices.0.delta.content")
	if content.Type != gjson.Null {
		t.Fatalf("delta.content = %s, want null", content.Raw)
	}
}

func TestStreamToolCallBuffer_StripsToolCallsAndReasoning(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	line := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"reasoning_content":"thinking...","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{}"}}]},"finish_reason":null}]}`)
	stripped, finish := buf.Consume(line)

	if finish != "" {
		t.Fatalf("finishReason = %q, want empty for non-finish chunk", finish)
	}
	if gjson.GetBytes(stripped, "choices.0.delta.tool_calls").Exists() {
		t.Fatalf("forwarded line still has tool_calls: %s", stripped)
	}
	if gjson.GetBytes(stripped, "choices.0.delta.reasoning_content").Exists() {
		t.Fatalf("forwarded line still has reasoning_content: %s", stripped)
	}
}

func TestStreamToolCallBuffer_PureTextPassthrough(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	line := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}]}`)
	stripped, finish := buf.Consume(line)

	if string(stripped) != string(line) {
		t.Fatalf("pure-text chunk modified: %s", stripped)
	}
	if finish != "" {
		t.Fatalf("finishReason = %q, want empty", finish)
	}
	if buf.HasToolCalls() {
		t.Fatal("HasToolCalls() = true for pure-text stream")
	}
}

func TestStreamToolCallBuffer_FinishReasonDetected(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	line := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}`)
	_, finish := buf.Consume(line)
	if finish != "tool_calls" {
		t.Fatalf("finishReason = %q, want %q", finish, "tool_calls")
	}
}

func TestStreamToolCallBuffer_ParallelToolCallsIndependent(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	l0 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_0","type":"function","function":{"name":"a","arguments":"{\"a\":1}"}}]},"finish_reason":null}]}`)
	l1 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_1","type":"function","function":{"name":"b","arguments":"{\"b\":2}"}}]},"finish_reason":null}]}`)
	// Repeating index 0 must not clobber index 1.
	l0dup := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"a\":1}"}}]},"finish_reason":null}]}`)

	buf.Consume(l0)
	buf.Consume(l1)
	buf.Consume(l0dup)

	toolCalls := gjson.GetBytes(buf.BuildToolCallsChunk(), "choices.0.delta.tool_calls")
	if n := len(toolCalls.Array()); n != 2 {
		t.Fatalf("tool_calls length = %d, want 2", n)
	}
	if got := toolCalls.Get("0.function.arguments").String(); got != `{"a":1}` {
		t.Fatalf("index0 arguments = %q, want %q", got, `{"a":1}`)
	}
	if got := toolCalls.Get("1.function.arguments").String(); got != `{"b":2}` {
		t.Fatalf("index1 arguments = %q, want %q", got, `{"b":2}`)
	}
}

func TestStreamToolCallBuffer_StripsLegacyFunctionCall(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	// The upstream finish chunk carries a deprecated empty function_call
	// placeholder alongside finish_reason=tool_calls; it must be stripped.
	line := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"","function_call":{"name":"","arguments":""},"refusal":"","extra_fields":null},"logprobs":null,"finish_reason":"tool_calls"}]}`)
	stripped, finish := buf.Consume(line)

	if finish != "tool_calls" {
		t.Fatalf("finishReason = %q, want %q", finish, "tool_calls")
	}
	if gjson.GetBytes(stripped, "choices.0.delta.function_call").Exists() {
		t.Fatalf("forwarded line still has function_call: %s", stripped)
	}
	if gjson.GetBytes(stripped, "choices.0.delta.tool_calls").Exists() {
		t.Fatalf("forwarded line still has tool_calls: %s", stripped)
	}
	if gjson.GetBytes(stripped, "choices.0.delta.reasoning_content").Exists() {
		t.Fatalf("forwarded line still has reasoning_content: %s", stripped)
	}
}

func TestStreamToolCallBuffer_StripsMessageToolCallEcho(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	// The upstream final chunk echoes the full message (non-streaming shape)
	// under `message.tool_calls`; it duplicates the delta stream and must be
	// stripped so the client does not concatenate it with the merged chunk.
	line := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"ls","arguments":"{}"}}],"function_call":null},"finish_reason":"tool_calls"}]}`)
	stripped, finish := buf.Consume(line)

	if finish != "tool_calls" {
		t.Fatalf("finishReason = %q, want %q", finish, "tool_calls")
	}
	if gjson.GetBytes(stripped, "choices.0.message.tool_calls").Exists() {
		t.Fatalf("forwarded line still has message.tool_calls: %s", stripped)
	}
	if gjson.GetBytes(stripped, "choices.0.message.function_call").Exists() {
		t.Fatalf("forwarded line still has message.function_call: %s", stripped)
	}
}

func TestStreamToolCallBuffer_EmptySnapshotDoesNotClobberRealArgs(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	// Real args first, then a trailing empty "{}" snapshot that must be ignored.
	l1 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{\"command\":\"ls\"}"}}]},"finish_reason":null}]}`)
	l2 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]},"finish_reason":null}]}`)

	buf.Consume(l1)
	buf.Consume(l2)

	got := gjson.GetBytes(buf.BuildToolCallsChunk(), "choices.0.delta.tool_calls.0.function.arguments").String()
	if got != `{"command":"ls"}` {
		t.Fatalf("arguments = %q, want real args (empty snapshot must not clobber)", got)
	}
}

func TestStreamToolCallBuffer_EmptySnapshotThenRealArgs(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	// Empty "{}" first, then real args; the empty snapshot must be skipped.
	l1 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"shell","arguments":"{}"}}]},"finish_reason":null}]}`)
	l2 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\":\"ls\"}"}}]},"finish_reason":null}]}`)

	buf.Consume(l1)
	buf.Consume(l2)

	got := gjson.GetBytes(buf.BuildToolCallsChunk(), "choices.0.delta.tool_calls.0.function.arguments").String()
	if got != `{"command":"ls"}` {
		t.Fatalf("arguments = %q, want real args", got)
	}
}

func TestStreamToolCallBuffer_PreservesWhitespaceOnlyFragment(t *testing.T) {
	buf := newCodebuddyStreamToolCallBuffer()
	// 片段1：不完整 JSON 前缀（无结尾 }），走增量 append
	l1 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"replace_in_file","arguments":"{\"old_str\": \"OpenAI Chat Completions"}}]},"finish_reason":null}]}`)
	// 片段2：纯空格（本次 bug 关键：必须被保留为增量 arguments 的一部分，而非被 TrimSpace 丢弃）
	l2 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" "}}]},"finish_reason":null}]}`)
	// 片段3：剩余增量
	l3 := []byte(`data: {"id":"cmpl-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"协议\"}"}}]},"finish_reason":null}]}`)

	buf.Consume(l1)
	buf.Consume(l2)
	buf.Consume(l3)

	got := gjson.GetBytes(buf.BuildToolCallsChunk(), "choices.0.delta.tool_calls.0.function.arguments").String()
	want := `{"old_str": "OpenAI Chat Completions 协议"}`
	if got != want {
		t.Fatalf("whitespace-only fragment was dropped; arguments = %q, want %q (space preserved)", got, want)
	}
}
