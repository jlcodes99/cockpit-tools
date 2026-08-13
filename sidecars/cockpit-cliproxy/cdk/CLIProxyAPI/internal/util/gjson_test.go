package util

import "testing"

func TestGetGJSONBytesNoCopy(t *testing.T) {
	input := []byte(`{"request":{"contents":[{"role":"user"}]}}`)
	contents := GetGJSONBytesNoCopy(input, "request.contents")
	if !contents.IsArray() || len(contents.Array()) != 1 {
		t.Fatalf("contents = %s, want array with one item", contents.Raw)
	}
}

func TestGetGJSONBytesNoCopyEmptyInput(t *testing.T) {
	if result := GetGJSONBytesNoCopy(nil, "contents"); result.Exists() {
		t.Fatalf("empty input returned result: %s", result.Raw)
	}
}
