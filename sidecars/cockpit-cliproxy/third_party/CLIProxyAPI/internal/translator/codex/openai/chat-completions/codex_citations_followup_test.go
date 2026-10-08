package chat_completions

import (
	"testing"

	"github.com/tidwall/gjson"
)

func TestCodexFollowupCitationOffsetsAndDeduplication(t *testing.T) {
	var state any
	send := func(event string) [][]byte {
		return ConvertCodexResponseToOpenAI(t.Context(), "test", nil, nil, []byte("data: "+event), &state)
	}
	send(`{"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"前文"}`)
	send(`{"type":"response.output_text.delta","output_index":0,"content_index":1,"delta":"引用"}`)
	annotation := `{"type":"response.output_text.annotation.added","output_index":0,"content_index":1,"annotation":{"type":"url_citation","url":"https://example.com","title":"Example","start_index":0,"end_index":2}}`
	out := send(annotation)
	if len(out) != 1 {
		t.Fatalf("expected citation, got %d chunks", len(out))
	}
	got := gjson.GetBytes(out[0], "choices.0.delta.annotations.0")
	if got.Get("type").String() != "url_citation" || got.Get("url_citation.url").String() != "https://example.com" || got.Get("url_citation.title").String() != "Example" || got.Get("url").Exists() {
		t.Fatalf("invalid Chat Completions citation shape: %s", out[0])
	}
	if got.Get("url_citation.start_index").Int() != 2 || got.Get("url_citation.end_index").Int() != 4 {
		t.Fatalf("citation outside its text span: %s", out[0])
	}
	if out := send(annotation); len(out) != 0 {
		t.Fatal("duplicate citation emitted")
	}
	out = send(`{"type":"response.output_text.done","output_index":0,"content_index":0,"text":"前文","annotations":[{"type":"url_citation","url":"https://example.com","title":"Example","start_index":0,"end_index":2}]}`)
	if len(out) != 1 || gjson.GetBytes(out[0], "choices.0.delta.annotations.0.url_citation.start_index").Int() != 0 {
		t.Fatal("completion citation offset or cross-part deduplication incorrect")
	}
}

func TestCodexFollowupNonStreamCitationsPreserveAllTextParts(t *testing.T) {
	response := []byte(`{"type":"response.completed","response":{"id":"r","output":[{"type":"message","content":[{"type":"output_text","text":"前文"},{"type":"output_text","text":"引用","annotations":[{"type":"url_citation","url":"https://example.com","title":"Example","start_index":0,"end_index":2}]}]}]}}`)
	out := ConvertCodexResponseToOpenAINonStream(t.Context(), "test", nil, nil, response, nil)
	if gjson.GetBytes(out, "choices.0.message.content").String() != "前文引用" {
		t.Fatal("text parts were dropped")
	}
	annotation := gjson.GetBytes(out, "choices.0.message.annotations.0")
	if annotation.Get("type").String() != "url_citation" || annotation.Get("url_citation.url").String() != "https://example.com" || annotation.Get("url_citation.title").String() != "Example" || annotation.Get("url").Exists() {
		t.Fatalf("invalid Chat Completions citation shape: %s", out)
	}
	if annotation.Get("url_citation.start_index").Int() != 2 || annotation.Get("url_citation.end_index").Int() != 4 {
		t.Fatalf("invalid citation: %s", out)
	}
}
