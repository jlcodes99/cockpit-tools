package historyprojection

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestProjectFillsWebSearchQueryFields(t *testing.T) {
	profile := ProfileFor(UpstreamDeepSeek)

	singular := []byte(`{"model":"deepseek-flash","input":[{"type":"web_search_call","id":"ws_1","status":"completed","action":{"type":"search","query":"codex apply_patch"}}]}`)
	projected := Project(singular, profile)
	if got := gjson.GetBytes(projected, "input.0.action.queries.0").String(); got != "codex apply_patch" {
		t.Fatalf("queries[0] = %q, want the original query; payload=%s", got, projected)
	}

	plural := []byte(`{"input":[{"type":"web_search_call","id":"ws_1","status":"completed","action":{"type":"search","queries":["first","second"]}}]}`)
	projected = Project(plural, profile)
	if got := gjson.GetBytes(projected, "input.0.action.query").String(); got != "first" {
		t.Fatalf("query = %q, want the first query; payload=%s", got, projected)
	}
}

func TestProjectNormalizesIntegralArguments(t *testing.T) {
	profile := ProfileFor(UpstreamGeneric)

	payload := []byte(`{"input":[{"type":"function_call","name":"wait_agent","arguments":"{\"timeout_ms\":180000.0}"}]}`)
	projected := Project(payload, profile)
	if got := gjson.GetBytes(projected, "input.0.arguments").String(); got != `{"timeout_ms":180000}` {
		t.Fatalf("arguments = %q, want integral timeout_ms; payload=%s", got, projected)
	}

	nonIntegral := []byte(`{"input":[{"type":"function_call","name":"wait_agent","arguments":"{\"timeout_ms\":1.5}"}]}`)
	if got := Project(nonIntegral, profile); string(got) != string(nonIntegral) {
		t.Fatalf("non-integral float must stay unchanged: %s", got)
	}
}

func TestProjectKeepsUnrelatedPayloadBytes(t *testing.T) {
	profile := ProfileFor(UpstreamGeneric)
	payload := []byte(`{"input":[{"type":"message","role":"user","content":"hello"}]}`)
	if got := Project(payload, profile); string(got) != string(payload) {
		t.Fatalf("unrelated payload mutated: %s", got)
	}
}
