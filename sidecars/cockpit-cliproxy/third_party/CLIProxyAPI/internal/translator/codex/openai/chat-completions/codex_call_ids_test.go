package chat_completions

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/tidwall/gjson"
)

func TestConvertOpenAIRequestToCodexCallIDLength(t *testing.T) {
	for _, tc := range []struct {
		name, id string
	}{
		{"short", "call_1"},
		{"64 bytes", strings.Repeat("a", 64)},
		{"65 bytes", strings.Repeat("a", 65)},
		{"85 bytes", "call_" + strings.Repeat("x", 80)},
		{"UTF8 at boundary", strings.Repeat("a", 46) + strings.Repeat("界", 7)},
		{"UTF8 long", strings.Repeat("界", 30)},
	} {
		for _, typ := range []string{"function", "custom"} {
			t.Run(tc.name+"/"+typ, func(t *testing.T) {
				payload := codexCallIDTestPayload(t, []string{tc.id}, typ, false)
				got := ConvertOpenAIRequestToCodex("gpt-test", payload, true)
				calls := gjson.GetBytes(got, "input").Array()
				if len(calls) != 2 {
					t.Fatalf("expected call and result: %s", got)
				}
				callID := calls[0].Get("call_id").String()
				if len(callID) > 64 || !utf8.ValidString(callID) {
					t.Fatalf("invalid wire ID %q, bytes=%d", callID, len(callID))
				}
				if len(tc.id) <= 64 && callID != tc.id {
					t.Fatalf("short ID changed: %q", callID)
				}
				if len(tc.id) > 64 && callID == tc.id {
					t.Fatal("long ID not shortened")
				}
				if calls[1].Get("call_id").String() != callID {
					t.Fatalf("call/result mismatch: %s", got)
				}
				wantType, wantOutput := "function_call", "function_call_output"
				if typ == "custom" {
					wantType, wantOutput = "custom_tool_call", "custom_tool_call_output"
				}
				if calls[0].Get("type").String() != wantType || calls[1].Get("type").String() != wantOutput {
					t.Fatalf("tool kinds changed: %s", got)
				}
				if second := ConvertOpenAIRequestToCodex("gpt-test", payload, true); !bytes.Equal(got, second) {
					t.Fatal("conversion must be deterministic")
				}
			})
		}
	}
}

func TestConvertOpenAIRequestToCodexCallIDCollision(t *testing.T) {
	longID := "call_" + strings.Repeat("z", 100)
	shortCollision := shortenCodexCallIDIfNeeded(longID)
	secondLong := "call_" + strings.Repeat("z", 99) + "x"
	for _, separateTurns := range []bool{false, true} {
		payload := codexCallIDTestPayload(t, []string{longID, shortCollision, secondLong}, "function", separateTurns)
		got := ConvertOpenAIRequestToCodex("gpt-test", payload, true)
		calls := gjson.GetBytes(got, "input").Array()
		seen := map[string]bool{}
		outputs := map[string]bool{}
		for _, item := range calls {
			id := item.Get("call_id").String()
			if len(id) > 64 {
				t.Fatalf("ID too long: %q", id)
			}
			switch item.Get("type").String() {
			case "function_call":
				if seen[id] {
					t.Fatalf("distinct source IDs collided: %s", got)
				}
				seen[id] = true
			case "function_call_output":
				outputs[id] = true
			}
		}
		if len(seen) != 3 || len(outputs) != 3 || !seen[shortCollision] {
			t.Fatalf("caller short ID must remain reserved: %s", got)
		}
		for id := range seen {
			if !outputs[id] {
				t.Fatalf("output not matched for %q: %s", id, got)
			}
		}
	}
}

func TestBuildCodexCallIDMapStableAcrossOrderAndReusedTurns(t *testing.T) {
	id := "call_" + strings.Repeat("q", 80)
	collision := shortenCodexCallIDIfNeeded(id)
	first, _ := buildCodexCallIDMap(gjson.GetBytes(codexCallIDTestPayload(t, []string{id, collision, id}, "function", true), "messages"))
	second, _ := buildCodexCallIDMap(gjson.GetBytes(codexCallIDTestPayload(t, []string{collision, id}, "function", false), "messages"))
	if first[id] != second[id] || first[collision] != collision {
		t.Fatalf("mapping depends on order: %#v vs %#v", first, second)
	}
	got := ConvertOpenAIRequestToCodex("gpt-test", codexCallIDTestPayload(t, []string{id, id}, "function", true), true)
	for _, item := range gjson.GetBytes(got, "input").Array() {
		if item.Get("call_id").String() != shortenCodexCallIDIfNeeded(id) {
			t.Fatalf("reused source ID changed across turns: %s", got)
		}
	}
}

func codexCallIDTestPayload(t *testing.T, ids []string, typ string, separateTurns bool) []byte {
	t.Helper()
	messages := make([]any, 0)
	calls := make([]any, 0, len(ids))
	outputs := make([]any, 0, len(ids))
	for _, id := range ids {
		call := map[string]any{"id": id, "type": typ}
		if typ == "custom" {
			call["custom"] = map[string]any{"name": "apply_patch", "input": "patch"}
		} else {
			call["function"] = map[string]any{"name": "echo", "arguments": "{}"}
		}
		output := map[string]any{"role": "tool", "tool_call_id": id, "content": "result"}
		if separateTurns {
			messages = append(messages, map[string]any{"role": "assistant", "tool_calls": []any{call}}, output)
		} else {
			calls = append(calls, call)
			outputs = append(outputs, output)
		}
	}
	if !separateTurns {
		messages = append(messages, map[string]any{"role": "assistant", "tool_calls": calls})
		messages = append(messages, outputs...)
	}
	body, err := json.Marshal(map[string]any{"model": "gpt-test", "messages": messages})
	if err != nil {
		t.Fatal(err)
	}
	return body
}

// A wire alias is private to this conversion. An unmatched caller result must
// not consume the source call whose long ID happens to hash to that alias.
func TestConvertOpenAIRequestToCodexIgnoresOrphanWireAlias(t *testing.T) {
	for _, kind := range []string{"function", "custom"} {
		t.Run(kind, func(t *testing.T) {
			sourceID := "call_" + strings.Repeat("z", 100)
			alias := shortenCodexCallIDIfNeeded(sourceID)
			var payload map[string]any
			if err := json.Unmarshal(codexCallIDTestPayload(t, []string{sourceID}, kind, false), &payload); err != nil {
				t.Fatal(err)
			}
			messages := payload["messages"].([]any)
			orphan := map[string]any{"role": "tool", "tool_call_id": alias, "content": "wrong orphan result"}
			payload["messages"] = []any{messages[0], orphan, messages[1]}
			raw, err := json.Marshal(payload)
			if err != nil {
				t.Fatal(err)
			}
			converted := ConvertOpenAIRequestToCodex("gpt-test", raw, true)
			items := gjson.GetBytes(converted, "input").Array()
			if len(items) != 2 || items[1].Get("output").String() != "result" || items[0].Get("call_id").String() != items[1].Get("call_id").String() {
				t.Fatalf("orphan consumed correct result: %s", converted)
			}
			// A real source ID equal to the alias remains independently reserved.
			raw = codexCallIDTestPayload(t, []string{sourceID, alias}, kind, false)
			converted = ConvertOpenAIRequestToCodex("gpt-test", raw, true)
			items = gjson.GetBytes(converted, "input").Array()
			if len(items) != 4 || items[0].Get("call_id").String() == items[1].Get("call_id").String() || items[2].Get("call_id").String() != items[0].Get("call_id").String() || items[3].Get("call_id").String() != items[1].Get("call_id").String() {
				t.Fatalf("declared collision lost pairing: %s", converted)
			}
		})
	}
}

func TestConvertOpenAIRequestToCodexMissingIDKeepsResultFallback(t *testing.T) {
	for _, kind := range []string{"function", "custom"} {
		for _, resultID := range []string{"", "call_missing_0_0"} {
			var payload map[string]any
			if err := json.Unmarshal(codexCallIDTestPayload(t, []string{""}, kind, false), &payload); err != nil {
				t.Fatal(err)
			}
			payload["messages"].([]any)[1].(map[string]any)["tool_call_id"] = resultID
			raw, err := json.Marshal(payload)
			if err != nil {
				t.Fatal(err)
			}
			converted := ConvertOpenAIRequestToCodex("gpt-test", raw, true)
			items := gjson.GetBytes(converted, "input").Array()
			if len(items) != 2 || items[0].Get("call_id").String() != "call_missing_0_0" || items[1].Get("call_id").String() != items[0].Get("call_id").String() {
				t.Fatalf("missing-ID fallback lost: %s", converted)
			}
		}
	}
}
