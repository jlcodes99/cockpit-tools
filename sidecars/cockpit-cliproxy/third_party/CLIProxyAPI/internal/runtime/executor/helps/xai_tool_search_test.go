package helps

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/tidwall/gjson"
)

func TestXAIClientToolSearchRoundTrip(t *testing.T) {
	body := []byte(`{"tools":[{"type":"tool_search","execution":"client","description":"Discover suitable tools","parameters":{"type":"object","properties":{"goal":{"type":"string"}},"required":["goal"]}},{"type":"function","name":"__codex_tool_search","parameters":{"type":"object"}},{"type":"namespace","name":"__codex","tools":[{"type":"function","name":"__codex_tool_search_1"}]}],"tool_choice":{"type":"allowed_tools","tools":[{"type":"tool_search"}]},"input":[{"type":"tool_search_call","id":"tsc_one","call_id":"call_a","execution":"client","arguments":{"goal":"Inspect repository","limit":9223372036854775807}},{"type":"tool_search_output","id":"tso_one","call_id":"call_a","status":"completed","execution":"client","tools":[{"type":"function","name":"inspect","defer_loading":true,"parameters":{"type":"object"}},{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"type":"object"}}]}]}]}`)
	out, mapping, err := AdaptXAIClientToolSearch(body)
	if err != nil {
		t.Fatal(err)
	}
	if mapping == nil || mapping.Alias != "__codex_tool_search_2" {
		t.Fatalf("collision not avoided: %+v", mapping)
	}
	if got := gjson.GetBytes(out, "tools.0.description").String(); got != "Discover suitable tools" {
		t.Fatal(got)
	}
	if got := gjson.GetBytes(out, "tools.0.parameters.required.0").String(); got != "goal" {
		t.Fatal(got)
	}
	if got := gjson.GetBytes(out, "tool_choice.tools.0.name").String(); got != mapping.Alias {
		t.Fatal(got)
	}
	if got := gjson.GetBytes(out, "input.0.id").String(); got != "fc_one" {
		t.Fatal(got)
	}
	if got := gjson.GetBytes(out, "input.0.arguments").String(); !strings.Contains(got, "9223372036854775807") {
		t.Fatal(got)
	}
	if got := gjson.GetBytes(out, "input.1"); got.Get("type").String() != "function_call_output" || got.Get("call_id").String() != "call_a" || got.Get("id").Exists() || got.Get("tools").Exists() || got.Get("execution").Exists() {
		t.Fatal(got)
	}
	if got := gjson.GetBytes(out, "tools.#").Int(); got != 5 {
		t.Fatalf("discoveries not promoted: %s", out)
	}
	if gjson.GetBytes(out, "tools.3.defer_loading").Exists() {
		t.Fatal("deferred tool requires native search")
	}
	payload, _ := json.Marshal(map[string]any{"output": []any{map[string]any{"id": "fc_b", "type": "function_call", "call_id": "call_b", "name": mapping.Alias, "arguments": `{"goal":"next"}`}, map[string]any{"id": "fc_real", "type": "function_call", "name": "__codex_tool_search", "arguments": "{}"}}})
	restored, err := mapping.RestorePayload(payload)
	if err != nil {
		t.Fatal(err)
	}
	item := gjson.GetBytes(restored, "output.0")
	if item.Get("type").String() != "tool_search_call" || item.Get("execution").String() != "client" || !item.Get("arguments").IsObject() || item.Get("name").Exists() || item.Get("id").String() != "tsc_b" || item.Get("call_id").String() != "call_b" {
		t.Fatal(item)
	}
	if gjson.GetBytes(restored, "output.1.type").String() != "function_call" {
		t.Fatal("ordinary function rewritten")
	}
}

func TestXAIClientToolSearchNativeServer(t *testing.T) {
	for _, raw := range []string{
		`{"tools":[{"type":"tool_search","execution":"server"}],"input":[{"type":"tool_search_call","execution":"server","call_id":"s","arguments":{}},{"type":"tool_search_output","call_id":"s","tools":[{"type":"function","name":"server_tool"}]}]}`,
		`{"tools":[{"type":"function","name":"lookup"}],"input":[{"type":"tool_search_call","execution":"server","call_id":"s","arguments":{}},{"type":"tool_search_output","call_id":"s","tools":[]}]}`,
	} {
		out, mapping, err := AdaptXAIClientToolSearch([]byte(raw))
		if err != nil {
			t.Fatal(err)
		}
		if mapping != nil || string(out) != raw {
			t.Fatalf("server search mutated: %s", out)
		}
	}
	out, mapping, err := AdaptXAIClientToolSearch([]byte(`{"tools":[{"type":"tool_search","execution":"server"},{"type":"tool_search","execution":"client"},{"type":"function","name":"deferred","defer_loading":true}],"input":[{"type":"tool_search_call","execution":"server","call_id":"s","arguments":{}},{"type":"tool_search_output","call_id":"s","tools":[]},{"type":"tool_search_call","execution":"client","call_id":"c","arguments":{}},{"type":"tool_search_output","call_id":"c","tools":[]}]}`))
	if err != nil || mapping == nil {
		t.Fatalf("%s %v", out, err)
	}
	if gjson.GetBytes(out, "tools.1.execution").String() != "server" || !gjson.GetBytes(out, "tools.2.defer_loading").Bool() {
		t.Fatal(string(out))
	}
	if gjson.GetBytes(out, "input.0.type").String() != "tool_search_call" || gjson.GetBytes(out, "input.1.type").String() != "tool_search_output" || gjson.GetBytes(out, "input.2.type").String() != "function_call" {
		t.Fatal(string(out))
	}
}

func TestXAIClientToolSearchDiscoveryMergeAndConflict(t *testing.T) {
	body := []byte(`{"tools":[{"type":"tool_search"},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"one","defer_loading":true}]}],"input":[{"type":"tool_search_output","call_id":"c","tools":[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"one"},{"type":"function","name":"two"}]}]},{"type":"tool_search_output","call_id":"d","output":"{\"tools\":[{\"type\":\"function\",\"name\":\"three\"}]}"}]}`)
	out, _, err := AdaptXAIClientToolSearch(body)
	if err != nil {
		t.Fatal(err)
	}
	if gjson.GetBytes(out, "tools.#").Int() != 3 || gjson.GetBytes(out, "tools.1.tools.#").Int() != 2 || gjson.GetBytes(out, "tools.2.name").String() != "three" {
		t.Fatal(string(out))
	}
	for _, conflict := range []string{
		`{"tools":[{"type":"tool_search"},{"type":"function","name":"same"}],"input":[{"type":"tool_search_output","call_id":"c","tools":[{"type":"custom","name":"same"}]}]}`,
		`{"tools":[{"type":"tool_search"},{"type":"function","name":"ns__same"}],"input":[{"type":"tool_search_output","call_id":"c","tools":[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"same"}]}]}]}`,
	} {
		if _, _, err := AdaptXAIClientToolSearch([]byte(conflict)); err == nil {
			t.Fatal("conflicting discovery admitted")
		}
	}
}

func TestXAIClientToolSearchValidatesHistory(t *testing.T) {
	for _, item := range []string{
		`{"type":"tool_search_call","arguments":"not json"}`,
		`{"type":"tool_search_call","arguments":[]}`,
		`{"type":"tool_search_output","tools":[]}`,
		`{"type":"tool_search_output","call_id":"c"}`,
	} {
		if _, _, err := AdaptXAIClientToolSearch([]byte(`{"tools":[{"type":"tool_search"}],"input":[` + item + `]}`)); err == nil {
			t.Fatal(item)
		}
	}
}

func TestXAIClientToolSearchStreamLifecycle(t *testing.T) {
	mapping := &XAIClientToolSearch{Alias: xaiToolSearchAlias}
	restorer := NewXAIClientToolSearchRestorer(mapping)
	apply := func(event string) []byte {
		t.Helper()
		out, err := restorer.Apply([]byte(event))
		if err != nil {
			t.Fatal(err)
		}
		return out
	}
	added := apply(`{"type":"response.output_item.added","output_index":1,"item":{"id":"fc_s","call_id":"call_s","type":"function_call","name":"__codex_tool_search","arguments":""}}`)
	if gjson.GetBytes(added, "item.id").String() != "tsc_s" || !gjson.GetBytes(added, "item.arguments").IsObject() {
		t.Fatal(string(added))
	}
	if got := apply(`{"type":"response.function_call_arguments.delta","item_id":"fc_s","delta":"{\"query\":"}`); len(got) != 0 {
		t.Fatal(string(got))
	}
	if got := apply(`{"type":"response.function_call_arguments.delta","output_index":1,"delta":"\"test\"}"}`); len(got) != 0 {
		t.Fatal(string(got))
	}
	ordinary := `{"type":"response.function_call_arguments.delta","item_id":"fc_other","output_index":2,"delta":"ordinary"}`
	if string(apply(ordinary)) != ordinary {
		t.Fatal("ordinary call changed")
	}
	if got := apply(`{"type":"response.function_call_arguments.done","item_id":"fc_s","arguments":"{\"query\":\"test\"}"}`); len(got) != 0 {
		t.Fatal(string(got))
	}
	done := apply(`{"type":"response.output_item.done","output_index":1,"item":{"id":"fc_s","call_id":"call_s","type":"function_call","name":"__codex_tool_search"}}`)
	if gjson.GetBytes(done, "item.arguments.query").String() != "test" || gjson.GetBytes(done, "item.call_id").String() != "call_s" {
		t.Fatal(string(done))
	}
	if len(restorer.calls) != 0 || len(restorer.byIndex) != 0 {
		t.Fatal("completed call retained")
	}
	completed := apply(`{"type":"response.completed","response":{"output":[{"id":"fc_s","type":"function_call","call_id":"call_s","name":"__codex_tool_search","arguments":"{\"query\":\"test\"}"}]}}`)
	if gjson.GetBytes(completed, "response.output.0.type").String() != "tool_search_call" {
		t.Fatal(string(completed))
	}
}

func TestXAIClientToolSearchStreamLimitsAndIsolation(t *testing.T) {
	mapping := &XAIClientToolSearch{Alias: xaiToolSearchAlias}
	a, b := NewXAIClientToolSearchRestorer(mapping), NewXAIClientToolSearchRestorer(nil)
	added := []byte(`{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_s","type":"function_call","name":"__codex_tool_search"}}`)
	if _, err := a.Apply(added); err != nil {
		t.Fatal(err)
	}
	if got, err := b.Apply(added); err != nil || string(got) != string(added) {
		t.Fatal("mapping escaped request scope")
	}
	delta, _ := json.Marshal(map[string]any{"type": "response.function_call_arguments.delta", "item_id": "fc_s", "delta": strings.Repeat("x", xaiSearchMaxArguments+1)})
	if _, err := a.Apply(delta); err == nil {
		t.Fatal("unbounded arguments accepted")
	}
	if _, err := a.Apply([]byte(`{"type":"response.failed","response":{"output":[]}}`)); err != nil {
		t.Fatal(err)
	}
	if len(a.calls) != 0 || len(a.byIndex) != 0 {
		t.Fatal("failed response retained call buffers")
	}
	if _, err := mapping.RestorePayload([]byte(`{"output":[{"type":"function_call","name":"__codex_tool_search","arguments":"invalid"}]}`)); err == nil {
		t.Fatal("invalid JSON arguments accepted")
	}
}

func TestXAIClientToolSearchSessionInheritance(t *testing.T) {
	var session XAIClientToolSearchSession
	first := []byte(`{"tools":[{"type":"tool_search","execution":"client","description":"Keep schema","parameters":{"type":"object","properties":{"goal":{"type":"string"}},"required":["goal"]}}],"input":[]}`)
	_, mapping, err := AdaptXAIClientToolSearch(first)
	if err != nil {
		t.Fatal(err)
	}
	session.Remember("account-a", mapping)
	next, err := session.Inherit([]byte(`{"input":[{"type":"tool_search_output","execution":"client","call_id":"c","tools":[{"type":"function","name":"discovered"}]}]}`), "account-a", true)
	if err != nil {
		t.Fatal(err)
	}
	out, mapping, err := AdaptXAIClientToolSearch(next)
	if err != nil {
		t.Fatal(err)
	}
	if gjson.GetBytes(out, "tools.0.parameters.required.0").String() != "goal" || gjson.GetBytes(out, "tools.1.name").String() != "discovered" {
		t.Fatal(string(out))
	}
	session.Remember("account-a", mapping)
	next, err = session.Inherit([]byte(`{"input":[]}`), "account-a", true)
	if err != nil || gjson.GetBytes(next, "tools.#").Int() != 2 {
		t.Fatalf("discoveries not inherited: %s %v", next, err)
	}
	clearRequest := []byte(`{"tools":[],"input":[{"type":"tool_search_call","execution":"client","call_id":"c","arguments":{}},{"type":"tool_search_output","execution":"client","call_id":"c","tools":[]}]}`)
	next, err = session.Inherit(clearRequest, "account-a", true)
	if err != nil || string(next) != string(clearRequest) {
		t.Fatal("explicit tools changed")
	}
	out, mapping, err = AdaptXAIClientToolSearch(next)
	if err != nil || gjson.GetBytes(out, "tools.#").Int() != 0 || gjson.GetBytes(out, "input.0.type").String() != "function_call" {
		t.Fatalf("cleared search reenabled or history lost: %s %v", out, err)
	}
	session.Remember("account-a", mapping)
	next, err = session.Inherit([]byte(`{"input":[]}`), "account-a", true)
	if err != nil || gjson.GetBytes(next, "tools").Exists() {
		t.Fatal("cleared tools reappeared")
	}
	_, mapping, _ = AdaptXAIClientToolSearch(first)
	session.Remember("account-a", mapping)
	next, err = session.Inherit([]byte(`{"input":[]}`), "account-b", true)
	if err != nil || gjson.GetBytes(next, "tools").Exists() {
		t.Fatal("tools crossed account scope")
	}
}

func TestXAIClientToolSearchReplayUsesFreshAlias(t *testing.T) {
	first := []byte(`{"tools":[{"type":"tool_search"}],"input":[{"type":"tool_search_call","id":"tsc_old","call_id":"old","arguments":{"query":"load"}},{"type":"tool_search_output","call_id":"old","tools":[]}]}`)
	upstream, mapping, err := AdaptXAIClientToolSearch(first)
	if err != nil {
		t.Fatal(err)
	}
	transcript, err := mapping.RestoreHistory(upstream)
	if err != nil {
		t.Fatal(err)
	}
	if gjson.GetBytes(transcript, "input.0.type").String() != "tool_search_call" || gjson.GetBytes(transcript, "input.1.type").String() != "tool_search_output" {
		t.Fatal(string(transcript))
	}
	var decoded map[string]any
	if err := json.Unmarshal(transcript, &decoded); err != nil {
		t.Fatal(err)
	}
	decoded["tools"] = []any{map[string]any{"type": "tool_search"}, map[string]any{"type": "function", "name": xaiToolSearchAlias}}
	replayed, _ := json.Marshal(decoded)
	replayed, mapping, err = AdaptXAIClientToolSearch(replayed)
	if err != nil || mapping.Alias != xaiToolSearchAlias+"_1" || gjson.GetBytes(replayed, "input.0.name").String() != mapping.Alias {
		t.Fatalf("stale alias in replay: %s %v", replayed, err)
	}
}

func TestXAIClientToolSearchNativeServerWithoutCall(t *testing.T) {
	body := []byte(`{"tools":[{"type":"tool_search","execution":"server"}],"input":[{"type":"tool_search_output","call_id":"server","tools":[]}]}`)
	out, mapping, err := AdaptXAIClientToolSearch(body)
	if err != nil || mapping != nil || string(out) != string(body) {
		t.Fatalf("native server output lowered: %s %v", out, err)
	}
}

func TestXAIClientToolSearchSessionRejectsNewConversation(t *testing.T) {
	_, mapping, err := AdaptXAIClientToolSearch([]byte(`{"tools":[{"type":"tool_search"}],"input":[]}`))
	if err != nil {
		t.Fatal(err)
	}
	var session XAIClientToolSearchSession
	session.Remember("account", mapping)
	body := []byte(`{"type":"response.create","model":"grok-4.6","input":[]}`)
	got, err := session.Inherit(body, "account", false)
	if err != nil || string(got) != string(body) {
		t.Fatalf("new conversation inherited tools: %s %v", got, err)
	}
}

func TestXAIClientToolSearchStreamTotalBufferLimit(t *testing.T) {
	restorer := NewXAIClientToolSearchRestorer(&XAIClientToolSearch{Alias: xaiToolSearchAlias})
	for index := 0; index < 3; index++ {
		added, _ := json.Marshal(map[string]any{"type": "response.output_item.added", "output_index": index, "item": map[string]any{"id": string(rune('a' + index)), "type": "function_call", "name": xaiToolSearchAlias}})
		if _, err := restorer.Apply(added); err != nil {
			t.Fatal(err)
		}
		delta, _ := json.Marshal(map[string]any{"type": "response.function_call_arguments.delta", "output_index": index, "delta": strings.Repeat("x", xaiSearchMaxArguments)})
		_, err := restorer.Apply(delta)
		if index < 2 && err != nil {
			t.Fatal(err)
		}
		if index == 2 && err == nil {
			t.Fatal("unbounded total stream buffer")
		}
	}
}

func TestXAIClientToolSearchBoundsToolTree(t *testing.T) {
	var nested any = map[string]any{"type": "tool_search", "execution": "client"}
	for depth := 0; depth < xaiSearchMaxToolDepth; depth++ {
		nested = map[string]any{"type": "namespace", "name": "n", "tools": []any{nested}}
	}
	body, _ := json.Marshal(map[string]any{"tools": []any{nested}})
	if _, _, err := AdaptXAIClientToolSearch(body); err == nil || !strings.Contains(err.Error(), "nesting") {
		t.Fatalf("unbounded nesting: %v", err)
	}
	tools := make([]any, xaiSearchMaxToolNodes+1)
	for i := range tools {
		tools[i] = map[string]any{"type": "function", "name": "same"}
	}
	body, _ = json.Marshal(map[string]any{"tools": tools})
	if _, _, err := AdaptXAIClientToolSearch(body); err == nil || !strings.Contains(err.Error(), "nodes") {
		t.Fatalf("unbounded tool count: %v", err)
	}
}

func TestXAIClientToolSearchMissingCallIDAndNullOutput(t *testing.T) {
	for _, call := range []string{
		`{"type":"tool_search_call","arguments":{}}`,
		`{"type":"tool_search_call","id":"tsc_a","call_id":" ","arguments":{}}`,
	} {
		if _, _, err := AdaptXAIClientToolSearch([]byte(`{"tools":[{"type":"tool_search"}],"input":[` + call + `]}`)); err == nil || !strings.Contains(err.Error(), "call_id") {
			t.Fatalf("missing call_id accepted: %v", err)
		}
	}
	out, _, err := AdaptXAIClientToolSearch([]byte(`{"tools":[{"type":"tool_search"}],"input":[{"type":"tool_search_output","call_id":"a","output":null}]}`))
	if err != nil || gjson.GetBytes(out, "input.0.output").Type != gjson.String || gjson.GetBytes(out, "input.0.output").String() != "" {
		t.Fatalf("null output not normalized to empty string: %s %v", out, err)
	}
}

func TestXAIClientToolSearchRejectsTruncatedArguments(t *testing.T) {
	mapping := &XAIClientToolSearch{Alias: xaiToolSearchAlias}
	added := []byte(`{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_s","call_id":"s","type":"function_call","name":"__codex_tool_search"}}`)
	for _, terminal := range []string{
		`{"type":"response.output_item.done","output_index":0,"item":{"id":"fc_s","call_id":"s","type":"function_call","name":"__codex_tool_search"}}`,
		`{"type":"response.completed","response":{"output":[{"id":"fc_s","call_id":"s","type":"function_call","name":"__codex_tool_search","arguments":""}]}}`,
	} {
		restorer := NewXAIClientToolSearchRestorer(mapping)
		if got, err := restorer.Apply(added); err != nil || !gjson.GetBytes(got, "item.arguments").IsObject() {
			t.Fatalf("added must allow provisional empty arguments: %s %v", got, err)
		}
		if got, err := restorer.Apply([]byte(terminal)); err == nil {
			t.Fatalf("truncated call became executable: %s", got)
		}
	}
}

func TestXAIClientToolSearchPreservesUnknownResponseFields(t *testing.T) {
	mapping := &XAIClientToolSearch{Alias: xaiToolSearchAlias}
	out, err := mapping.RestorePayload([]byte(`{"output":[{"id":"fc_s","call_id":"s","type":"function_call","name":"__codex_tool_search","arguments":"{\"query\":\"q\"}","encrypted_function_args":"opaque","vendor":{"value":1}}]}`))
	if err != nil || gjson.GetBytes(out, "output.0.encrypted_function_args").String() != "opaque" || gjson.GetBytes(out, "output.0.vendor.value").Int() != 1 {
		t.Fatalf("unknown response fields removed: %s %v", out, err)
	}
}

func TestXAIClientToolSearchDiscoveryPrefersExplicitAndLatestSchemas(t *testing.T) {
	body := []byte(`{"tools":[{"type":"tool_search"},{"type":"function","name":"explicit","description":"Current declaration","parameters":{"type":"object","properties":{"current":{"type":"string"}}}},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"explicit","parameters":{"type":"object","properties":{"current":{"type":"string"}}}}]}],"input":[{"type":"tool_search_output","call_id":"old","status":"completed","tools":[{"type":"function","name":"explicit","parameters":{"type":"object","properties":{"old":{"type":"string"}}}},{"type":"function","name":"discovered","parameters":{"type":"object","properties":{"old":{"type":"string"}}}},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"explicit","parameters":{"type":"object","properties":{"old":{"type":"string"}}}},{"type":"function","name":"discovered","parameters":{"type":"object","properties":{"old":{"type":"string"}}}}]}]},{"type":"tool_search_output","call_id":"new","status":"completed","tools":[{"type":"function","name":"discovered","parameters":{"type":"object","properties":{"latest":{"type":"string"}}}},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"discovered","parameters":{"type":"object","properties":{"latest":{"type":"string"}}}}]}]},{"type":"tool_search_output","call_id":"pending","status":"in_progress","tools":[{"type":"function","name":"discovered","parameters":{"type":"object","properties":{"pending":{"type":"string"}}}}]}]}`)
	out, _, err := AdaptXAIClientToolSearch(body)
	if err != nil {
		t.Fatal(err)
	}
	if gjson.GetBytes(out, "tools.#").Int() != 4 {
		t.Fatalf("unexpected declaration count: %s", out)
	}
	if gjson.GetBytes(out, "tools.1.description").String() != "Current declaration" || !gjson.GetBytes(out, "tools.1.parameters.properties.current").Exists() {
		t.Fatalf("explicit declaration replaced by old history: %s", out)
	}
	if !gjson.GetBytes(out, "tools.2.tools.0.parameters.properties.current").Exists() {
		t.Fatalf("explicit namespace declaration replaced: %s", out)
	}
	if !gjson.GetBytes(out, "tools.2.tools.1.parameters.properties.latest").Exists() || !gjson.GetBytes(out, "tools.3.parameters.properties.latest").Exists() {
		t.Fatalf("newest completed discovery did not win: %s", out)
	}
}

func TestXAIClientToolSearchSessionEvolvesDiscoverySchemas(t *testing.T) {
	var session XAIClientToolSearchSession
	for round := 1; round <= 4; round++ {
		request := map[string]any{"input": []any{}}
		if round == 1 {
			request["tools"] = []any{
				map[string]any{"type": "tool_search", "execution": "client"},
				map[string]any{"type": "function", "name": "explicit", "parameters": map[string]any{"version": "fixed"}},
				map[string]any{"type": "namespace", "name": "ns", "tools": []any{map[string]any{"type": "function", "name": "explicit", "parameters": map[string]any{"version": "fixed"}}}},
			}
		}
		if round <= 3 {
			request["input"] = []any{map[string]any{
				"type": "tool_search_output", "execution": "client", "status": "completed", "call_id": "search",
				"tools": []any{
					map[string]any{"type": "function", "name": "explicit", "parameters": map[string]any{"version": round}},
					map[string]any{"type": "function", "name": "discovered", "parameters": map[string]any{"version": round}},
					map[string]any{"type": "namespace", "name": "ns", "tools": []any{
						map[string]any{"type": "function", "name": "explicit", "parameters": map[string]any{"version": round}},
						map[string]any{"type": "function", "name": "discovered", "parameters": map[string]any{"version": round}},
					}},
				},
			}}
		}
		body, _ := json.Marshal(request)
		body, err := session.Inherit(body, "account", round > 1)
		if err != nil {
			t.Fatalf("round %d inherit: %v", round, err)
		}
		body, mapping, err := AdaptXAIClientToolSearch(body)
		if err != nil {
			t.Fatalf("round %d adapt: %v", round, err)
		}
		wantVersion := round
		if wantVersion > 3 {
			wantVersion = 3
		}
		foundDirect, foundNamespace := false, false
		for _, tool := range gjson.GetBytes(body, "tools").Array() {
			if tool.Get("name").String() == "explicit" && tool.Get("parameters.version").String() != "fixed" {
				t.Fatalf("round %d changed explicit tool: %s", round, tool)
			}
			if tool.Get("name").String() == "discovered" {
				foundDirect = true
				if tool.Get("parameters.version").Int() != int64(wantVersion) {
					t.Fatalf("round %d retained stale discovered schema: %s", round, tool)
				}
			}
			if tool.Get("type").String() == "namespace" {
				for _, child := range tool.Get("tools").Array() {
					switch child.Get("name").String() {
					case "explicit":
						if child.Get("parameters.version").String() != "fixed" {
							t.Fatalf("round %d changed explicit namespace child: %s", round, child)
						}
					case "discovered":
						foundNamespace = true
						if child.Get("parameters.version").Int() != int64(wantVersion) {
							t.Fatalf("round %d retained stale namespace schema: %s", round, child)
						}
					}
				}
			}
		}
		if !foundDirect || !foundNamespace {
			t.Fatalf("round %d lost tools without repeated history: %s", round, body)
		}
		session.Remember("account", mapping)
	}
}

func TestXAIClientToolSearchSessionRetainsExplicitAdditionalTools(t *testing.T) {
	var session XAIClientToolSearchSession
	first := []byte(`{"tools":[{"type":"tool_search"},{"type":"namespace","name":"ns","tools":[{"type":"function","name":"update","parameters":{"version":1}},{"type":"function","name":"keep"}]}],"input":[]}`)
	_, mapping, err := AdaptXAIClientToolSearch(first)
	if err != nil {
		t.Fatal(err)
	}
	session.Remember("account", mapping)
	second := []byte(`{"input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"update","parameters":{"version":2}}]}]},{"type":"tool_search_output","call_id":"c","tools":[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"update","parameters":{"version":99}}]}]}]}`)
	second, err = session.Inherit(second, "account", true)
	if err != nil {
		t.Fatal(err)
	}
	second, mapping, err = AdaptXAIClientToolSearch(second)
	if err != nil {
		t.Fatal(err)
	}
	if gjson.GetBytes(second, "tools.#").Int() != 2 || gjson.GetBytes(second, "tools.1.tools.#").Int() != 2 || gjson.GetBytes(second, "input.0.type").String() != "function_call_output" {
		t.Fatalf("explicit additions duplicated namespace or input: %s", second)
	}
	session.Remember("account", mapping)
	third := []byte(`{"input":[{"type":"tool_search_output","call_id":"c","tools":[{"type":"namespace","name":"ns","tools":[{"type":"function","name":"update","parameters":{"version":3}}]}]}]}`)
	third, err = session.Inherit(third, "account", true)
	if err != nil {
		t.Fatal(err)
	}
	third, _, err = AdaptXAIClientToolSearch(third)
	if err != nil {
		t.Fatal(err)
	}
	for _, child := range gjson.GetBytes(third, "tools.1.tools").Array() {
		if child.Get("name").String() == "update" && child.Get("parameters.version").Int() != 2 {
			t.Fatalf("new explicit declaration lost provenance: %s", third)
		}
	}
}
