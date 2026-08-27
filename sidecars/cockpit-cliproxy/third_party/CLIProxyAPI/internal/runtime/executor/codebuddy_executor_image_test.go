package executor

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestNormalizeCodebuddyChatImageContent(t *testing.T) {
	tests := []struct {
		name    string
		in      string
		wantURL string // empty means the part should remain untouched
	}{
		{
			name:    "standard object url kept",
			in:      `{"messages":[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}]}`,
			wantURL: "data:image/png;base64,AAAA",
		},
		{
			name:    "string image_url wrapped",
			in:      `{"messages":[{"role":"user","content":[{"type":"image_url","image_url":"data:image/png;base64,BBBB"}]}]}`,
			wantURL: "data:image/png;base64,BBBB",
		},
		{
			name:    "nested url object flattened (extra braces)",
			in:      `{"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":{"url":"data:image/png;base64,CCCC"}}}]}]}`,
			wantURL: "data:image/png;base64,CCCC",
		},
		{
			name:    "double-encoded JSON string",
			in:      `{"messages":[{"role":"user","content":[{"type":"image_url","image_url":"{\"url\":\"data:image/png;base64,DDDD\"}"}]}]}`,
			wantURL: "data:image/png;base64,DDDD",
		},
		{
			name:    "nested image_url field",
			in:      `{"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"image_url":"data:image/png;base64,EEEE"}}]}]}`,
			wantURL: "data:image/png;base64,EEEE",
		},
		{
			name:    "input_image part converted",
			in:      `{"messages":[{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,FFFF"}]}]}`,
			wantURL: "data:image/png;base64,FFFF",
		},
		{
			name:    "detail preserved",
			in:      `{"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,GGGG","detail":"high"}}]}]}`,
			wantURL: "data:image/png;base64,GGGG",
		},
		{
			name:    "non-image content untouched",
			in:      `{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}`,
			wantURL: "",
		},
		{
			name:    "string content untouched",
			in:      `{"messages":[{"role":"user","content":"hello"}]}`,
			wantURL: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			out := normalizeCodebuddyChatImageContent([]byte(tt.in))
			if tt.wantURL == "" {
				// Must be byte-identical when nothing changed.
				if string(out) != tt.in {
					t.Fatalf("expected untouched input, got %s", out)
				}
				return
			}
			// First image part must carry the normalized url as a string.
			parts := gjson.GetBytes(out, "messages.0.content").Array()
			if len(parts) == 0 {
				t.Fatalf("no content parts; out=%s", out)
			}
			var imgPart gjson.Result
			for _, p := range parts {
				if p.Get("type").String() == "image_url" {
					imgPart = p
					break
				}
			}
			if !imgPart.Exists() {
				t.Fatalf("no image_url part; out=%s", out)
			}
			gotURL := imgPart.Get("image_url.url")
			if gotURL.Type != gjson.String || gotURL.String() != tt.wantURL {
				t.Fatalf("image_url.url = %v (%s), want %q; out=%s", gotURL.Type, gotURL.String(), tt.wantURL, out)
			}
			if tt.name == "detail preserved" {
				if got := imgPart.Get("image_url.detail").String(); got != "high" {
					t.Fatalf("detail = %q, want high; out=%s", got, out)
				}
			}
		})
	}
}

func TestNormalizeCodebuddyChatImageContent_NoMessages(t *testing.T) {
	in := `{"model":"auto"}`
	out := normalizeCodebuddyChatImageContent([]byte(in))
	if string(out) != in {
		t.Fatalf("expected unchanged, got %s", out)
	}
}

func TestNormalizeCodebuddyChatImageContent_InvalidJSON(t *testing.T) {
	in := `not-json`
	out := normalizeCodebuddyChatImageContent([]byte(in))
	if string(out) != in {
		t.Fatalf("expected unchanged, got %s", out)
	}
}
