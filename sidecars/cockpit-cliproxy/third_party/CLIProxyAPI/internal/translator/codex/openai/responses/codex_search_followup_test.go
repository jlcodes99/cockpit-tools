package responses

import (
	"github.com/tidwall/gjson"
	"testing"
)

func TestCodexFollowupSearchSourcesIncludePreserved(t *testing.T) {
	input := []byte(`{"model":"test","include":["web_search_call.action.sources","reasoning.encrypted_content","unsupported"],"input":[]}`)
	out := ConvertOpenAIResponsesRequestToCodex("test", input, true)
	include := gjson.GetBytes(out, "include").Array()
	if len(include) != 2 {
		t.Fatalf("include=%s", out)
	}
	seen := map[string]bool{}
	for _, value := range include {
		seen[value.String()] = true
	}
	if !seen["web_search_call.action.sources"] || !seen["reasoning.encrypted_content"] {
		t.Fatal("requested sources or reasoning replay were dropped")
	}
}
