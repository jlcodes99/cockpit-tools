package gemini

import (
	"context"
	"testing"

	"github.com/tidwall/gjson"
)

func TestCompletedStreamEmitsStopAndUsageWithoutRepeatingText(t *testing.T) {
	var state any
	ConvertCodexResponseToGemini(context.Background(), "model", nil, nil, []byte(`data: {"type":"response.output_text.delta","delta":"hello"}`), &state)
	frames := ConvertCodexResponseToGemini(context.Background(), "model", nil, nil, []byte(`data: {"type":"response.completed","response":{"id":"r","usage":{"input_tokens":4,"output_tokens":6},"output":[{"type":"message","content":[{"type":"output_text","text":"hello"}]}]}}`), &state)
	if len(frames) != 1 {
		t.Fatalf("frames=%q", frames)
	}
	frame := frames[0]
	if gjson.GetBytes(frame, "candidates.0.finishReason").String() != "STOP" || gjson.GetBytes(frame, "usageMetadata.totalTokenCount").Int() != 10 {
		t.Fatalf("missing terminal metadata: %s", frame)
	}
	if gjson.GetBytes(frame, "candidates.0.content.parts.#").Int() != 0 {
		t.Fatalf("repeated content: %s", frame)
	}
}
