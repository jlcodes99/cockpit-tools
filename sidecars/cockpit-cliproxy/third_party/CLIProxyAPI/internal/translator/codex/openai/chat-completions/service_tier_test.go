package chat_completions

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestConvertOpenAIRequestToCodexServiceTier(t *testing.T) {
	for _, tc := range []struct {
		name, input, want string
	}{
		{"priority alias", `{"service_tier":"fast"}`, "priority"},
		{"ultrafast", `{"service_tier":" ultrafast "}`, "ultrafast"},
		{"unsupported", `{"service_tier":"economy"}`, ""},
		{"non string", `{"service_tier":true}`, ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := gjson.GetBytes(ConvertOpenAIRequestToCodex("gpt-4o", []byte(tc.input), false), "service_tier").String()
			if got != tc.want {
				t.Fatalf("service_tier = %q, want %q", got, tc.want)
			}
		})
	}
}
