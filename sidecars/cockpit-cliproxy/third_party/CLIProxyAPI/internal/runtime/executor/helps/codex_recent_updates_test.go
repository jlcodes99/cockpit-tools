package helps

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestNormalizeCodexToolSchemasFlattensLargeConstantUnion(t *testing.T) {
	body := []byte(`{"tools":[{"type":"function","name":"lookup","parameters":{"type":"object","properties":{"kind":{"oneOf":[{"const":"a"},{"const":"b"},{"const":"c"},{"const":"d"},{"const":"e"},{"const":"f"},{"const":"g"},{"const":"h"}]}}}}]}`)
	got := NormalizeCodexToolSchemas(body)
	if gjson.GetBytes(got, "tools.0.parameters.properties.kind.enum.#").Int() != 8 {
		t.Fatalf("normalized enum = %s, want 8 values: %s", gjson.GetBytes(got, "tools.0.parameters.properties.kind.enum.#").Raw, got)
	}
}

func TestIsCodexTerminalEmptyIncompleteRequiresExplicitZeroTokens(t *testing.T) {
	empty := []byte(`{"type":"response.incomplete","response":{"output":[],"usage":{"output_tokens":0}}}`)
	if !IsCodexTerminalEmptyIncomplete(empty, 0, false) {
		t.Fatal("expected zero-token empty response to be detected")
	}
	nonEmpty := []byte(`{"type":"response.incomplete","response":{"output":[],"usage":{"output_tokens":1}}}`)
	if IsCodexTerminalEmptyIncomplete(nonEmpty, 0, false) {
		t.Fatal("non-zero output token response must not be classified as empty")
	}
}
