package responses

import (
	"bytes"
	"testing"

	"github.com/tidwall/gjson"
)

func TestEnsureCodexWebSearchHistoryTool(t *testing.T) {
	const search = `{"type":"web_search_call","id":"ws_1","status":"completed","action":{"type":"search","query":"test"}}`
	const function = `{"type":"function","name":"echo","parameters":{"type":"object"}}`
	const custom = `{"type":"custom","name":"apply_patch"}`
	const trigger = `{"type":"compaction_trigger"}`
	tests := []struct {
		name, body, toolsPath, choice string
		lite                          bool
		toolCount                     int
		unchanged                     bool
	}{
		{name: "absent tools", body: `{"input":[` + search + `]}`, toolsPath: "tools", choice: "none", toolCount: 1},
		{name: "empty tools auto", body: `{"input":[` + search + `],"tools":[],"tool_choice":"auto"}`, toolsPath: "tools", choice: "none", toolCount: 1},
		{name: "null tools choice", body: `{"input":[` + search + `],"tools":null,"tool_choice":null}`, toolsPath: "tools", choice: "none", toolCount: 1},
		{name: "explicit required", body: `{"input":[` + search + `],"tool_choice":"required"}`, toolsPath: "tools", choice: "required", toolCount: 1},
		{name: "function tools auto", body: `{"input":[` + search + `],"tools":[` + function + `],"tool_choice":"auto"}`, toolsPath: "tools", choice: "auto", toolCount: 2},
		{name: "custom tools auto", body: `{"input":[` + search + `],"tools":[` + custom + `],"tool_choice":"auto"}`, toolsPath: "tools", choice: "auto", toolCount: 2},
		{name: "function tools no choice", body: `{"input":[` + search + `],"tools":[` + function + `]}`, toolsPath: "tools", toolCount: 2},
		{name: "explicit choice preserved", body: `{"input":[` + search + `],"tool_choice":{"type":"web_search"}}`, toolsPath: "tools", choice: `{"type":"web_search"}`, toolCount: 1},
		{name: "already declared", body: `{"input":[` + search + `],"tools":[{"type":"web_search","external_web_access":true}],"tool_choice":"auto"}`, unchanged: true},
		{name: "preview declaration", body: `{"input":[` + search + `],"tools":[{"type":"web_search_preview_2025_03_11"}]}`, unchanged: true},
		{name: "additional declaration", body: `{"input":[` + search + `,{"type":"additional_tools","tools":[{"type":"web_search"}]}]}`, unchanged: true},
		{name: "no search history", body: `{"input":[{"type":"function_call","call_id":"call_1","name":"web_search","arguments":"{}"},{"type":"function_call_output","call_id":"call_1","output":"web_search_call"}],"tools":[]}`, unchanged: true},
		{name: "message mentions search", body: `{"input":[{"type":"message","role":"user","content":"web_search_call"}]}`, unchanged: true},
		{name: "string input", body: `{"input":"web_search_call"}`, unchanged: true},
		{name: "nested response ignored", body: `{"response":{"input":[` + search + `]}}`, unchanged: true},
		{name: "lite inserts before trigger", body: `{"input":[` + search + `,` + trigger + `]}`, lite: true, toolsPath: "input.1.tools", choice: "none", toolCount: 1},
		{name: "lite appends", body: `{"input":[` + search + `]}`, lite: true, toolsPath: "input.1.tools", choice: "none", toolCount: 1},
		{name: "lite existing functions", body: `{"input":[{"type":"additional_tools","role":"developer","tools":[` + function + `]},` + search + `,` + trigger + `],"tool_choice":"auto"}`, lite: true, toolsPath: "input.0.tools", choice: "auto", toolCount: 2},
		{name: "lite existing custom", body: `{"input":[{"type":"additional_tools","tools":[` + custom + `]},` + search + `],"tool_choice":"required"}`, lite: true, toolsPath: "input.0.tools", choice: "required", toolCount: 2},
		{name: "lite existing null tools", body: `{"input":[{"type":"additional_tools","tools":null},` + search + `]}`, lite: true, toolsPath: "input.0.tools", choice: "none", toolCount: 1},
		{name: "lite top-level function preserved", body: `{"input":[` + search + `],"tools":[` + function + `],"tool_choice":"auto"}`, lite: true, toolsPath: "input.1.tools", choice: "auto", toolCount: 1},
		{name: "invalid body", body: `{"input":[` + search + `]`, unchanged: true},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			body := []byte(tc.body)
			before := append([]byte(nil), body...)
			got := EnsureCodexWebSearchHistoryTool(body, tc.lite)
			if !bytes.Equal(body, before) {
				t.Fatal("mutated caller input")
			}
			if tc.unchanged {
				if !bytes.Equal(got, before) {
					t.Fatalf("unexpected change: %s", got)
				}
				return
			}
			if !gjson.ValidBytes(got) {
				t.Fatalf("invalid output: %s", got)
			}
			tools := gjson.GetBytes(got, tc.toolsPath).Array()
			if len(tools) != tc.toolCount {
				t.Fatalf("tools count %d, want %d: %s", len(tools), tc.toolCount, got)
			}
			last := tools[len(tools)-1]
			if last.Get("type").String() != "web_search" || last.Get("external_web_access").Type != gjson.False {
				t.Fatalf("must only declare cached-only search: %s", last.Raw)
			}
			if choice := gjson.GetBytes(got, "tool_choice").String(); choice != tc.choice {
				t.Fatalf("tool_choice %q, want %q", choice, tc.choice)
			}
			if tc.lite && codexToolsDeclareWebSearch(gjson.GetBytes(got, "tools")) {
				t.Fatalf("Lite hosted tool at top level: %s", got)
			}
			originalItems := gjson.GetBytes(before, "input").Array()
			if len(originalItems) > 0 && originalItems[len(originalItems)-1].Get("type").String() == "compaction_trigger" {
				items := gjson.GetBytes(got, "input").Array()
				if items[len(items)-1].Get("type").String() != "compaction_trigger" {
					t.Fatalf("compaction trigger must remain last: %s", got)
				}
			}
			if second := EnsureCodexWebSearchHistoryTool(got, tc.lite); !bytes.Equal(got, second) {
				t.Fatalf("not idempotent: %s", second)
			}
		})
	}
}

func TestEnsureCodexWebSearchHistoryToolEscapedType(t *testing.T) {
	for _, lite := range []bool{false, true} {
		body := []byte(`{"input":[{"type":"web_\u0073earch_call","id":"ws_search","action":{"type":"search","query":"q"}}],"tools":[]}`)
		out := EnsureCodexWebSearchHistoryTool(body, lite)
		path := "tools.0.type"
		if lite {
			path = "input.1.tools.0.type"
		}
		if gjson.GetBytes(out, path).String() != "web_search" || gjson.GetBytes(out, "tool_choice").String() != "none" {
			t.Fatalf("escaped enum must use decoded semantics lite=%v: %s", lite, out)
		}
		if !bytes.Equal(EnsureCodexWebSearchHistoryTool(out, lite), out) {
			t.Fatal("escaped declaration must remain idempotent")
		}
	}
}
