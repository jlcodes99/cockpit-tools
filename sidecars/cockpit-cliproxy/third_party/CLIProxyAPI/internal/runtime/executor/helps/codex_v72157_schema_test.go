package helps

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestCodexV72157StripsUnicodePropertyPatterns(t *testing.T) {
	raw := []byte(`{"type":"object","properties":{"name":{"type":"string","pattern":"^[\\p{L}]+$"}},"patternProperties":{"^[\\p{N}]+$":{"type":"string"},"^safe$":{"type":"string","pattern":"ok"}}}`)
	got, changed := stripIncompatiblePatternsFromJSON(raw)
	if !changed {
		t.Fatal("schema was not normalized")
	}
	if gjson.GetBytes(got, "properties.name.pattern").Exists() {
		t.Fatalf("unsupported property pattern remained: %s", got)
	}
	if gjson.GetBytes(got, `patternProperties.#[%"^[\\p{N}]+$"]`).Exists() {
		t.Fatalf("unsupported patternProperties key remained: %s", got)
	}
	if gotSafe := gjson.GetBytes(got, `patternProperties.^safe$.pattern`).String(); gotSafe != "ok" {
		t.Fatalf("safe pattern = %q, want ok; schema=%s", gotSafe, got)
	}
}
