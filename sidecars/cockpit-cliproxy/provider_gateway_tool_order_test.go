package main

import (
	"strings"
	"testing"

	"github.com/tidwall/gjson"
)

func toolOrderBody(items ...string) []byte {
	return []byte(`{"model":"deepseek-v4-flash","input":[` + strings.Join(items, ",") + `]}`)
}

func toolOrderTypes(body []byte) []string {
	types := make([]string, 0)
	for _, item := range gjson.GetBytes(body, "input").Array() {
		types = append(types, item.Get("type").String()+":"+item.Get("call_id").String())
	}
	return types
}

func TestProviderGatewaySerializeToolCallsDisablesParallelCalls(t *testing.T) {
	body := toolOrderBody(`{"type":"function_call","call_id":"call_a","name":"read","arguments":"{}"}`)
	got := providerGatewaySerializeToolCalls(body)
	if v := gjson.GetBytes(got, "parallel_tool_calls"); !v.Exists() || v.Bool() {
		t.Fatalf("parallel_tool_calls was not disabled: %s", got)
	}
	// 原有内容不能丢。
	if gjson.GetBytes(got, "input.0.call_id").String() != "call_a" {
		t.Fatalf("input was damaged: %s", got)
	}

	// 已经有该字段时同样被改成 false。
	existing := []byte(`{"model":"m","parallel_tool_calls":true,"input":[]}`)
	if v := gjson.GetBytes(providerGatewaySerializeToolCalls(existing), "parallel_tool_calls"); v.Bool() {
		t.Fatalf("existing parallel_tool_calls=true was not overridden")
	}
}

func TestProviderGatewayToolOrderIsGatedToDeepSeek(t *testing.T) {
	if !providerGatewayRepairsToolCallOrder(&providerGatewaySpec{BaseURL: "https://api.deepseek.com"}) {
		t.Fatal("DeepSeek gateway must repair tool call order")
	}
	for _, baseURL := range []string{
		"https://api.openai.com",
		"https://api.example.com",
		"https://api.deepseek.com.evil.test",
		"",
	} {
		if providerGatewayRepairsToolCallOrder(&providerGatewaySpec{BaseURL: baseURL}) {
			t.Fatalf("gateway %q must not repair tool call order", baseURL)
		}
	}
	if providerGatewayRepairsToolCallOrder(nil) {
		t.Fatal("nil gateway must not repair tool call order")
	}
}

func TestProviderGatewayToolOrderMovesMessageOutOfThePair(t *testing.T) {
	// Codex 的 PostToolUse 钩子会把开发消息写在工具输出之前，
	// DeepSeek 因此判定调用「没有输出」并拒绝整轮。
	body := toolOrderBody(
		`{"type":"message","role":"user","content":"run pwd"}`,
		`{"type":"function_call","call_id":"call_a","name":"exec_command","arguments":"{}"}`,
		`{"type":"message","role":"developer","content":"hook note"}`,
		`{"type":"function_call_output","call_id":"call_a","output":"/tmp"}`,
	)

	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
	if !ok {
		t.Fatal("reorder should be allowed")
	}
	if relocated != 1 {
		t.Fatalf("relocated = %d, want 1: %s", relocated, got)
	}
	want := []string{
		"message:",
		"function_call:call_a",
		"function_call_output:call_a",
		"message:",
	}
	if strings.Join(toolOrderTypes(got), "|") != strings.Join(want, "|") {
		t.Fatalf("order = %v, want %v: %s", toolOrderTypes(got), want, got)
	}
	// 内容不能丢：开发消息仍然在请求里。
	if !strings.Contains(string(got), "hook note") {
		t.Fatalf("hook message was dropped: %s", got)
	}
}

func TestProviderGatewayToolOrderKeepsParallelBatchAdjacent(t *testing.T) {
	// 同一回合的两个调用必须保持相邻：拆成「调用A → 输出A → 调用B → 输出B」会被 DeepSeek
	// 判成「新的一批调用没有回放推理」，并以 reasoning_text 缺失拒绝整轮。
	body := toolOrderBody(
		`{"type":"function_call","call_id":"call_a","name":"read","arguments":"{}"}`,
		`{"type":"function_call","call_id":"call_b","name":"read","arguments":"{}"}`,
		`{"type":"function_call_output","call_id":"call_a","output":"a"}`,
		`{"type":"function_call_output","call_id":"call_b","output":"b"}`,
	)

	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
	if !ok {
		t.Fatal("reorder should be allowed")
	}
	if relocated != 0 {
		t.Fatalf("batched parallel calls must not be relocated: relocated=%d: %s", relocated, got)
	}
	if string(got) != string(body) {
		t.Fatalf("batched parallel calls changed: %s", got)
	}
}

func TestProviderGatewayToolOrderMovesBatchOutputsBehindBatch(t *testing.T) {
	// 钩子消息插在批次与输出之间时，两个调用仍要相邻，输出整体后移到批次之后。
	body := toolOrderBody(
		`{"type":"message","role":"user","content":"run both"}`,
		`{"type":"function_call","call_id":"call_a","name":"exec_command","arguments":"{}"}`,
		`{"type":"function_call","call_id":"call_b","name":"exec_command","arguments":"{}"}`,
		`{"type":"message","role":"developer","content":"hook note"}`,
		`{"type":"function_call_output","call_id":"call_a","output":"a"}`,
		`{"type":"function_call_output","call_id":"call_b","output":"b"}`,
	)

	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
	if !ok {
		t.Fatal("reorder should be allowed")
	}
	if relocated != 2 {
		t.Fatalf("relocated = %d, want 2: %s", relocated, got)
	}
	want := []string{
		"message:",
		"function_call:call_a",
		"function_call:call_b",
		"function_call_output:call_a",
		"function_call_output:call_b",
		"message:",
	}
	if strings.Join(toolOrderTypes(got), "|") != strings.Join(want, "|") {
		t.Fatalf("order = %v, want %v: %s", toolOrderTypes(got), want, got)
	}
	if !strings.Contains(string(got), "hook note") {
		t.Fatalf("hook message was dropped: %s", got)
	}
}

func TestProviderGatewayToolOrderPreservesOutputOrderWithinBatch(t *testing.T) {
	// 输出的先后顺序不影响上游校验，保持原有相对顺序、逐字节透传即可。
	body := toolOrderBody(
		`{"type":"function_call","call_id":"call_a","name":"read","arguments":"{}"}`,
		`{"type":"function_call","call_id":"call_b","name":"read","arguments":"{}"}`,
		`{"type":"function_call_output","call_id":"call_b","output":"b"}`,
		`{"type":"function_call_output","call_id":"call_a","output":"a"}`,
	)

	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
	if !ok || relocated != 0 {
		t.Fatalf("reversed outputs must stay untouched: ok=%v relocated=%d", ok, relocated)
	}
	if string(got) != string(body) {
		t.Fatalf("reversed outputs changed the body: %s", got)
	}
}

func TestProviderGatewayToolOrderKeepsLongBatchAdjacent(t *testing.T) {
	body := toolOrderBody(
		`{"type":"function_call","call_id":"call_a","name":"read","arguments":"{}"}`,
		`{"type":"function_call","call_id":"call_b","name":"read","arguments":"{}"}`,
		`{"type":"function_call","call_id":"call_c","name":"read","arguments":"{}"}`,
		`{"type":"function_call_output","call_id":"call_a","output":"a"}`,
		`{"type":"function_call_output","call_id":"call_b","output":"b"}`,
		`{"type":"function_call_output","call_id":"call_c","output":"c"}`,
	)

	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
	if !ok || relocated != 0 {
		t.Fatalf("three-call batch must stay untouched: ok=%v relocated=%d", ok, relocated)
	}
	if string(got) != string(body) {
		t.Fatalf("three-call batch changed: %s", got)
	}
}

func TestProviderGatewayToolOrderKeepsReasoningTurnBatched(t *testing.T) {
	// 线上失败请求的最小复刻：推理 + 助手消息 + 两个并行调用 + 两个输出。
	body := toolOrderBody(
		`{"type":"reasoning","summary":[{"type":"summary_text","text":"think"}],"encrypted_content":null}`,
		`{"type":"message","role":"assistant","content":"先核对一下状态"}`,
		`{"type":"function_call","call_id":"call_a","name":"exec_command","arguments":"{}"}`,
		`{"type":"function_call","call_id":"call_b","name":"exec_command","arguments":"{}"}`,
		`{"type":"function_call_output","call_id":"call_a","output":"a"}`,
		`{"type":"function_call_output","call_id":"call_b","output":"b"}`,
	)

	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
	if !ok || relocated != 0 {
		t.Fatalf("replayed thinking turn must stay untouched: ok=%v relocated=%d", ok, relocated)
	}
	if string(got) != string(body) {
		t.Fatalf("replayed thinking turn changed the body: %s", got)
	}
}

func TestProviderGatewayToolOrderIsNoOpForOrderedHistory(t *testing.T) {
	body := toolOrderBody(
		`{"type":"message","role":"user","content":"hi"}`,
		`{"type":"function_call","call_id":"call_a","name":"read","arguments":"{}"}`,
		`{"type":"function_call_output","call_id":"call_a","output":"a"}`,
		`{"type":"message","role":"assistant","content":"done"}`,
	)

	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
	if !ok || relocated != 0 {
		t.Fatalf("ordered history must not be touched: relocated=%d ok=%v", relocated, ok)
	}
	if string(got) != string(body) {
		t.Fatalf("ordered history changed: %s", got)
	}
}

func TestProviderGatewayToolOrderLeavesUnpairedAndPrecedingOutputsAlone(t *testing.T) {
	// 没有 call_id 的调用项无法判断归属：放弃重排（保持透传），不能瞎猜。
	mixed := toolOrderBody(
		`{"type":"function_call","name":"read","arguments":"{}"}`,
		`{"type":"message","role":"developer","content":"note"}`,
		`{"type":"function_call_output","output":"a"}`,
	)
	if _, relocated, ok := providerGatewayRepairsToolCallOrderBody(mixed); ok || relocated != 0 {
		t.Fatalf("missing call_id must abort reorder: ok=%v relocated=%d", ok, relocated)
	}

	// 输出排在调用之前属于历史损坏，交给配对修复处理，本函数保持原样。
	preceding := toolOrderBody(
		`{"type":"function_call_output","call_id":"call_a","output":"a"}`,
		`{"type":"function_call","call_id":"call_a","name":"read","arguments":"{}"}`,
		`{"type":"message","role":"assistant","content":"done"}`,
	)
	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(preceding)
	if !ok || relocated != 0 {
		t.Fatalf("preceding output scenario: ok=%v relocated=%d", ok, relocated)
	}
	if string(got) != string(preceding) {
		t.Fatalf("preceding output scenario changed the body: %s", got)
	}
}

func TestProviderGatewayToolOrderHandlesCustomToolCalls(t *testing.T) {
	body := toolOrderBody(
		`{"type":"custom_tool_call","call_id":"call_c","name":"apply_patch","input":"patch"}`,
		`{"type":"message","role":"developer","content":"note"}`,
		`{"type":"custom_tool_call_output","call_id":"call_c","output":"ok"}`,
	)

	got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
	if !ok || relocated != 1 {
		t.Fatalf("custom tool call pair was not repaired: relocated=%d ok=%v", relocated, ok)
	}
	want := []string{"custom_tool_call:call_c", "custom_tool_call_output:call_c", "message:"}
	if strings.Join(toolOrderTypes(got), "|") != strings.Join(want, "|") {
		t.Fatalf("order = %v, want %v: %s", toolOrderTypes(got), want, got)
	}
}

func TestProviderGatewayToolOrderHandlesToolSearch(t *testing.T) {
	const (
		searchCall   = `{"type":"tool_search_call","id":"tsc_1","call_id":"search_1","execution":"client","arguments":{"query":"lookup"},"status":"completed"}`
		searchOutput = `{"type":"tool_search_output","id":"tso_1","call_id":"search_1","execution":"client","tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}],"status":"completed"}`
		functionCall = `{"type":"function_call","call_id":"call_1","name":"exec_command","arguments":"{\"cmd\":\"pwd\"}"}`
		functionOut  = `{"type":"function_call_output","call_id":"call_1","output":"/tmp"}`
		hook         = `{"type":"message","role":"developer","content":"hook note"}`
	)
	tests := []struct {
		name      string
		items     []string
		wantOrder []int
		relocated int
	}{
		{
			name:      "search with hook",
			items:     []string{searchCall, hook, searchOutput},
			wantOrder: []int{0, 2, 1},
			relocated: 1,
		},
		{
			name:      "function and search batch preserves reverse output order",
			items:     []string{functionCall, searchCall, hook, searchOutput, functionOut},
			wantOrder: []int{0, 1, 3, 4, 2},
			relocated: 2,
		},
		{
			name:      "search and function batch",
			items:     []string{searchCall, functionCall, hook, functionOut, searchOutput},
			wantOrder: []int{0, 1, 3, 4, 2},
			relocated: 2,
		},
		{
			name:      "separate tool turns",
			items:     []string{searchCall, hook, searchOutput, functionCall, functionOut},
			wantOrder: []int{0, 2, 1, 3, 4},
			relocated: 1,
		},
		{
			name:      "ordered search is unchanged",
			items:     []string{searchCall, searchOutput, hook},
			wantOrder: []int{0, 1, 2},
		},
		{
			name:      "ordered mixed batch is unchanged",
			items:     []string{functionCall, searchCall, searchOutput, functionOut, hook},
			wantOrder: []int{0, 1, 2, 3, 4},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			body := toolOrderBody(tt.items...)
			if tt.relocated > 0 && providerToolOrderBatchesAreAdjacent(gjson.GetBytes(body, "input")) {
				t.Error("validator accepted a displaced tool search output")
			}
			got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
			if !ok || relocated != tt.relocated {
				t.Fatalf("ok=%v relocated=%d, want true/%d: %s", ok, relocated, tt.relocated, got)
			}
			items := gjson.GetBytes(got, "input").Array()
			if len(items) != len(tt.items) {
				t.Fatalf("input length=%d, want %d: %s", len(items), len(tt.items), got)
			}
			for index, originalIndex := range tt.wantOrder {
				if items[index].Raw != tt.items[originalIndex] {
					t.Fatalf("item %d changed or misplaced: got=%s want=%s", index, items[index].Raw, tt.items[originalIndex])
				}
			}
			if tt.relocated == 0 && string(got) != string(body) {
				t.Fatalf("ordered history must remain byte-for-byte unchanged: %s", got)
			}
			if !providerToolOrderBatchesAreAdjacent(gjson.GetBytes(got, "input")) {
				t.Fatalf("repaired history failed validation: %s", got)
			}
			again, movedAgain, okAgain := providerGatewayRepairsToolCallOrderBody(got)
			if !okAgain || movedAgain != 0 || string(again) != string(got) {
				t.Fatalf("repair is not idempotent: ok=%v relocated=%d body=%s", okAgain, movedAgain, again)
			}
		})
	}
}

func TestProviderGatewayToolOrderLeavesUnidentifiableToolSearchUntouched(t *testing.T) {
	tests := []struct {
		name string
		call string
		out  string
	}{
		{
			name: "missing call ID",
			call: `{"type":"tool_search_call","execution":"client","arguments":{"query":"lookup"}}`,
			out:  `{"type":"tool_search_output","call_id":"search_1","execution":"client","tools":[]}`,
		},
		{
			name: "missing output call ID",
			call: `{"type":"tool_search_call","call_id":"search_1","execution":"client","arguments":{"query":"lookup"}}`,
			out:  `{"type":"tool_search_output","execution":"client","tools":[]}`,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			body := toolOrderBody(tt.call, `{"type":"message","role":"developer","content":"hook note"}`, tt.out)
			got, relocated, ok := providerGatewayRepairsToolCallOrderBody(body)
			if ok || relocated != 0 || string(got) != string(body) {
				t.Fatalf("unidentifiable search pair must abort repair unchanged: ok=%v relocated=%d body=%s", ok, relocated, got)
			}
		})
	}
}
