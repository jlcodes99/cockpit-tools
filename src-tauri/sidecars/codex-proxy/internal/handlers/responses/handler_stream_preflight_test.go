package responses

import (
	"testing"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/handlers/common"
)

func TestHasResponsesSemanticContent(t *testing.T) {
	t.Run("function call arguments delta", func(t *testing.T) {
		event := "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"\"}\n\n"
		if !common.HasResponsesSemanticContent(event) {
			t.Fatal("expected function_call_arguments.delta to be treated as non-text content")
		}
	})

	t.Run("output item added function call", func(t *testing.T) {
		event := "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"name\":\"Read\",\"call_id\":\"call_1\"}}\n\n"
		if !common.HasResponsesSemanticContent(event) {
			t.Fatal("expected function_call output_item to be treated as non-text content")
		}
	})

	t.Run("completed event with function call output", func(t *testing.T) {
		event := "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"function_call\",\"name\":\"Read\",\"call_id\":\"call_1\",\"arguments\":\"{}\"}]}}\n\n"
		if !common.HasResponsesSemanticContent(event) {
			t.Fatal("expected completed event with function_call output to be treated as non-text content")
		}
	})

	t.Run("reasoning item added", func(t *testing.T) {
		event := "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"id\":\"rs_1\",\"status\":\"in_progress\",\"summary\":[]}}\n\n"
		if !common.HasResponsesSemanticContent(event) {
			t.Fatal("expected reasoning output_item to be treated as semantic content")
		}
	})

	t.Run("plain empty completed", func(t *testing.T) {
		event := "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[]}}\n\n"
		if common.HasResponsesSemanticContent(event) {
			t.Fatal("did not expect empty completed event to be treated as non-text content")
		}
	})
}

func TestIsResponsesEmptyContent(t *testing.T) {
	cases := []struct {
		name  string
		text  string
		empty bool
	}{
		{name: "empty string", text: "", empty: true},
		{name: "opening brace only", text: "{", empty: true},
		{name: "whitespace brace", text: "  {  ", empty: true},
		{name: "json body", text: "{\"path\":\"/tmp/x\"}", empty: false},
		{name: "plain text", text: "hello", empty: false},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := common.IsEffectivelyEmptyStreamText(tc.text); got != tc.empty {
				t.Fatalf("IsEffectivelyEmptyStreamText(%q) = %v, want %v", tc.text, got, tc.empty)
			}
		})
	}
}

func TestBuildResponsesPreflightDiagnostic(t *testing.T) {
	if got := buildResponsesPreflightDiagnostic(false, false, false, false, "", ""); got == "" {
		t.Fatal("expected diagnostic for no-event case")
	}
	if got := buildResponsesPreflightDiagnostic(true, true, false, false, "", ""); got == "" {
		t.Fatal("expected diagnostic for completed-empty case")
	}
	if got := buildResponsesPreflightDiagnostic(true, false, false, true, "response.custom.delta", ""); got == "" || got == "收到了未识别的 Responses 事件类型，但没有文本或语义内容" {
		t.Fatal("expected diagnostic to mention unknown responses event type")
	}
}
