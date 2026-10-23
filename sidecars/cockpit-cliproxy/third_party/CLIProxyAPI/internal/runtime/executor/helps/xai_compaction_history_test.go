package helps

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"github.com/tidwall/gjson"
	"testing"
)

func TestXAICompactionReplacesPrefixWithoutEditingCallerHistory(t *testing.T) {
	noise := make([]byte, 96)
	for index := range noise {
		noise[index] = byte(index)
	}
	cipher := base64.RawStdEncoding.EncodeToString(noise)
	for _, kind := range []string{"compaction", "compaction_summary"} {
		original := []byte(fmt.Sprintf(`{"input":[{"type":"message","role":"user","content":"old"},{"type":%q,"encrypted_content":%q},{"type":"message","role":"user","content":"new"}]}`, kind, cipher))
		copy := bytes.Clone(original)
		got := ProjectXAICompactionHistory(original)
		if len(gjson.GetBytes(got, "input").Array()) != 2 || gjson.GetBytes(got, "input.0.type").String() != "compaction" || gjson.GetBytes(got, "input.0.encrypted_content").String() != cipher {
			t.Fatalf("compaction prefix or state was not preserved: %s", got)
		}
		if !bytes.Equal(copy, original) {
			t.Fatal("caller history was modified")
		}
	}
	invalid := []byte(`{"input":[{"type":"message","role":"user","content":"old"},{"type":"compaction","encrypted_content":"invalid"}]}`)
	if !bytes.Equal(invalid, ProjectXAICompactionHistory(invalid)) {
		t.Fatal("invalid compaction discarded recoverable history")
	}
}
