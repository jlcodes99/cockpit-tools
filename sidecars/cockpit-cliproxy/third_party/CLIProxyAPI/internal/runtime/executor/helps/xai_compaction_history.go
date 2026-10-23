package helps

import (
	"encoding/json"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/signature"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// ProjectXAICompactionHistory retains the complete caller payload outside this
// wire copy. A valid compaction replaces its summarized prefix, including when
// the outer gateway expanded a cached previous_response_id into full history.
func ProjectXAICompactionHistory(payload []byte) []byte {
	items := gjson.GetBytes(payload, "input")
	if !items.IsArray() {
		return payload
	}
	last := 0
	changed := false
	projected := make([]json.RawMessage, 0, len(items.Array()))
	for index, item := range items.Array() {
		raw := []byte(item.Raw)
		kind := item.Get("type").String()
		if kind == "compaction_summary" {
			if updated, err := sjson.SetBytes(raw, "type", "compaction"); err == nil {
				raw, kind, changed = updated, "compaction", true
			}
		}
		if kind == "compaction" && signature.IsValidGrokEncryptedContent(item.Get("encrypted_content").String()) {
			last = index
		}
		projected = append(projected, raw)
	}
	if !changed && last == 0 {
		return payload
	}
	encoded, err := json.Marshal(projected[last:])
	if err != nil {
		return payload
	}
	if updated, err := sjson.SetRawBytes(payload, "input", encoded); err == nil {
		return updated
	}
	return payload
}
