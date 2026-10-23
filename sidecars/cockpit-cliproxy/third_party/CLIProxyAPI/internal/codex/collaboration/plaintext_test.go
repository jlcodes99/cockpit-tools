package collaboration

import (
	"fmt"
	"testing"

	"github.com/tidwall/gjson"
)

func TestPlaintextPolicyUsesEachFinalToolDeclaration(t *testing.T) {
	for _, name := range []string{"spawn_agent", "send_message", "followup_task"} {
		for _, tc := range []struct {
			name, schema string
			want         bool
		}{
			{"plaintext", `{"type":"string"}`, true},
			{"explicit plaintext", `{"type":"string","encrypted":false}`, true},
			{"encrypted", `{"type":"string","encrypted":true}`, false},
			{"unknown flag", `{"type":"string","encrypted":"false"}`, false},
			{"null flag", `{"type":"string","encrypted":null}`, false},
			{"unknown type", `{"type":"object"}`, false},
			{"missing type", `{}`, false},
		} {
			tool := fmt.Sprintf(`{"type":"function","name":%q,"parameters":{"properties":{"message":%s}}}`, name, tc.schema)
			for _, additional := range []bool{false, true} {
				request := []byte(fmt.Sprintf(`{"tools":[{"type":"namespace","name":"collaboration","tools":[%s]}]}`, tool))
				if additional {
					request = []byte(fmt.Sprintf(`{"input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"collaboration","tools":[%s]}]}]}`, tool))
				}
				for _, event := range []string{"json", "response.output_item.added", "response.output_item.done", "response.completed"} {
					item := fmt.Sprintf(`{"type":"function_call","namespace":"collaboration","name":%q,"arguments":"{\"message\":\"task\"}"}`, name)
					payload := []byte(fmt.Sprintf(`{"type":%q,"item":%s}`, event, item))
					path := "item"
					if event == "json" {
						payload, path = []byte(`{"output":[`+item+`]}`), "output.0"
					} else if event == "response.completed" {
						payload, path = []byte(`{"type":"response.completed","response":{"output":[`+item+`]}}`), "response.output.0"
					}
					got := MarkPlaintextCallsForRequest(payload, request)
					if marked := gjson.GetBytes(got, path+".encrypted_function_args").Raw == "[]"; marked != tc.want {
						t.Fatalf("%s/%s/additional=%t/%s: %s", name, tc.name, additional, event, got)
					}
					if !tc.want && string(got) != string(payload) {
						t.Fatal("unknown/encrypted declaration changed response")
					}
				}
			}
		}
	}
}

func TestPlaintextPolicyRejectsConflictsMissingDeclarationsAndOpaqueEvidence(t *testing.T) {
	plain := `{"type":"function","name":"send_message","parameters":{"properties":{"message":{"type":"string"}}}}`
	cipher := `{"type":"function","name":"send_message","parameters":{"properties":{"message":{"type":"string","encrypted":true}}}}`
	payload := []byte(`{"type":"function_call","namespace":"collaboration","name":"send_message","arguments":"{}"}`)
	for _, request := range []string{`{}`, `{"tools":[]}`, `{"tools":null}`, `{"input":[{"type":"additional_tools","tools":{}}]}`, `{"tools":`,
		fmt.Sprintf(`{"tools":[{"type":"namespace","name":"collaboration","tools":[%s,%s]}]}`, plain, cipher),
		fmt.Sprintf(`{"tools":[{"type":"namespace","name":"collaboration","tools":[%s,%s]}]}`, cipher, plain),
		fmt.Sprintf(`{"tools":[{"type":"namespace","name":"collaboration","tools":[%s]}],"input":[{"type":"additional_tools","tools":[{"type":"namespace","name":"collaboration","tools":[%s]}]}]}`, plain, cipher),
		fmt.Sprintf(`{"tools":[{"type":"namespace","name":"other","tools":[%s]}]}`, plain),
	} {
		if got := MarkPlaintextCallsForRequest(payload, []byte(request)); string(got) != string(payload) {
			t.Fatalf("missing/ambiguous declaration marked plaintext: %s", request)
		}
	}
	for _, evidence := range []string{`"encrypted_function_args":["cipher"]`, `"encrypted_function_args":[]`, `"encrypted_function_args":"unknown"`, `"encrypted_content":"gAAAA-real-cipher"`, `"signature":"signed"`} {
		item := []byte(fmt.Sprintf(`{"type":"function_call","namespace":"collaboration","name":"send_message","arguments":"{}",%s}`, evidence))
		if got := MarkPlaintextCalls(item); string(got) != string(item) {
			t.Fatalf("opaque evidence changed: %s", got)
		}
	}
	if got := MarkPlaintextCalls([]byte(`{"type":"function_call","namespace":"collaboration-optimize","name":"send_message","arguments":"{}"}`)); gjson.GetBytes(got, "encrypted_function_args").Exists() {
		t.Fatal("user-defined reserved namespace marked plaintext")
	}
}

func TestPlaintextPolicyMixedToolsAreIndependent(t *testing.T) {
	request := []byte(`{"tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","parameters":{"properties":{"message":{"type":"string"}}}},{"type":"function","name":"send_message","parameters":{"properties":{"message":{"type":"string","encrypted":true}}}},{"type":"function","name":"followup_task","parameters":{"properties":{"message":{"type":"string","encrypted":"unknown"}}}}]}]}`)
	payload := []byte(`{"output":[{"type":"function_call","namespace":"collaboration","name":"spawn_agent","arguments":"{}"},{"type":"function_call","namespace":"collaboration","name":"send_message","arguments":"{}"},{"type":"function_call","namespace":"collaboration","name":"followup_task","arguments":"{}"}]}`)
	got := MarkPlaintextCallsForRequest(payload, request)
	if gjson.GetBytes(got, "output.0.encrypted_function_args").Raw != "[]" || gjson.GetBytes(got, "output.1.encrypted_function_args").Exists() || gjson.GetBytes(got, "output.2.encrypted_function_args").Exists() {
		t.Fatalf("mixed declarations shared a global plaintext flag: %s", got)
	}
}

func TestWireAliasesRequireFinalDeclarationAndActualRename(t *testing.T) {
	response := []byte(`{"type":"response.output_item.done","item":{"type":"function_call","namespace":"collaboration-optimize","name":"spawn_agent","arguments":"{\"message\":\"task\"}","id":"fc-1","call_id":"call-1"}}`)
	for _, tc := range []struct {
		encrypted     string
		renamed, want bool
	}{
		{`false`, false, false}, {`false`, true, true}, {`true`, true, false}, {`"unknown"`, true, false},
	} {
		request := []byte(fmt.Sprintf(`{"tools":[{"type":"namespace","name":"collaboration-optimize","tools":[{"type":"function","name":"spawn_agent","parameters":{"properties":{"message":{"type":"string","encrypted":%s}}}}]}]}`, tc.encrypted))
		policy := PlaintextPolicyForRequest(request, tc.renamed)
		got := policy.MarkWireCalls(response)
		if marked := gjson.GetBytes(got, "item.encrypted_function_args").Raw == "[]"; marked != tc.want {
			t.Fatalf("alias=%t encrypted=%s: %s", tc.renamed, tc.encrypted, got)
		}
		for _, field := range []string{"type", "item.namespace", "item.name", "item.arguments", "item.id", "item.call_id"} {
			if gjson.GetBytes(got, field).Raw != gjson.GetBytes(response, field).Raw {
				t.Fatalf("wire projection changed %s", field)
			}
		}
	}
}
