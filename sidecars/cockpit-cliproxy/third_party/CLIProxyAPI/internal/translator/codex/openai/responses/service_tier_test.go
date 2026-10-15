package responses

import (
	"github.com/tidwall/gjson"
	"testing"
)

func TestResponsesServiceTierCompatibility(t *testing.T) {
	for _, tc := range []struct{ input, want string }{
		{`{"service_tier":" FAST "}`, "priority"},
		{`{"service_tier":" ULTRAFAST "}`, "ultrafast"},
		{`{"service_tier":"default"}`, ""},
		{`{"service_tier":123}`, ""},
		{`{}`, ""},
	} {
		got := gjson.GetBytes(ConvertOpenAIResponsesRequestToCodex("gpt-4o", []byte(tc.input), false), "service_tier")
		if got.String() != tc.want || tc.want == "" && got.Exists() {
			t.Fatalf("input %s: service_tier=%s, want %q", tc.input, got.Raw, tc.want)
		}
	}
}
