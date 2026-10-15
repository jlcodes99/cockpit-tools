package executor

import (
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
	"testing"
)

func TestCodexFollowupCompatKeepsUnknownEncryptedContent(t *testing.T) {
	for _, compat := range []bool{false, true} {
		for _, value := range []string{"vendor-reasoning-token", "", " vendor-token "} {
			body, _ := sjson.Set(`{"input":[{"type":"reasoning","id":"rs1"}]}`, "input.0.encrypted_content", value)
			got := sanitizeOpenAIResponsesReasoningEncryptedContentWithCompat(t.Context(), "test", []byte(body), compat)
			want := compat && value == "vendor-reasoning-token"
			if gjson.GetBytes(got, "input.0.encrypted_content").Exists() != want {
				t.Fatalf("compat=%v value=%q output=%s", compat, value, got)
			}
		}
	}
}
