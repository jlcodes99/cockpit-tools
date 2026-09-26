package main

import (
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/tidwall/gjson"
)

// Check identity and content together: a terminal snapshot must update the
// streamed message, while legitimate repeated text must remain untouched.
func assertProviderGatewayMessageIdentity(t *testing.T, body, wantText string) {
	t.Helper()
	var addedID, deltaText, doneText, completedText string
	added, done, completed := 0, 0, 0
	for _, line := range strings.Split(body, "\n") {
		if !strings.HasPrefix(line, "data:") {
			continue
		}
		event := gjson.Parse(strings.TrimSpace(strings.TrimPrefix(line, "data:")))
		var id string
		switch event.Get("type").String() {
		case "response.output_item.added":
			added++
			addedID = event.Get("item.id").String()
			id = addedID
		case "response.output_text.delta":
			id = event.Get("item_id").String()
			deltaText += event.Get("delta").String()
		case "response.output_item.done":
			done++
			id = event.Get("item.id").String()
			doneText = event.Get("item.content.0.text").String()
		case "response.completed":
			completed++
			id = event.Get("response.output.0.id").String()
			completedText = event.Get("response.output.0.content.0.text").String()
		default:
			continue
		}
		if addedID == "" || id != addedID {
			t.Errorf("stream item identity changed: added=%q, current=%q; event=%s", addedID, id, event.Raw)
		}
	}
	if added != 1 || done != 1 || completed != 1 {
		t.Errorf("expected one message lifecycle, got added=%d done=%d completed=%d", added, done, completed)
	}
	if deltaText != wantText || doneText != wantText || completedText != wantText {
		t.Errorf("message content changed: delta=%q done=%q completed=%q; want %q", deltaText, doneText, completedText, wantText)
	}
}

func TestProviderGatewayResponsesStreamKeepsMessageIdentity(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	body := strings.Join([]string{
		`data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","content":[]}}`,
		`data: {"type":"response.output_text.delta","item_id":"msg_1","delta":"hello "}`,
		`data: {"type":"response.output_text.delta","item_id":"msg_1","delta":"hello "}`,
		`data: {"type":"response.output_item.done","output_index":0,"item":{"type":"message","id":"msg_1","content":[{"type":"output_text","text":"hello hello "}]}}`,
		`data: {"type":"response.completed","response":{"id":"resp_1","output":[{"type":"message","id":"msg_1","content":[{"type":"output_text","text":"hello hello "}]}]}}`,
	}, "\n\n") + "\n\n"
	(&relayServer{}).writeProviderGatewayResponsesStream(c, strings.NewReader(body), false)
	assertProviderGatewayMessageIdentity(t, w.Body.String(), "hello hello ")
}
