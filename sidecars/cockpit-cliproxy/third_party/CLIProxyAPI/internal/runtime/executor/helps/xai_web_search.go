package helps

import (
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// OpenAI-only options are rejected by xAI; accepted search options stay intact.
var xaiUnsupportedWebSearchFields = [...]string{"external_web_access", "search_context_size"}

func StripUnsupportedXAIWebSearchFields(raw []byte) ([]byte, []string, bool) {
	var removed []string
	for _, field := range xaiUnsupportedWebSearchFields {
		if !gjson.GetBytes(raw, field).Exists() {
			continue
		}
		updated, errDel := sjson.DeleteBytes(raw, field)
		if errDel != nil {
			return nil, nil, false
		}
		raw = updated
		removed = append(removed, field)
	}
	return raw, removed, true
}
