package openai

import (
	"bytes"
	"encoding/json"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/interfaces"
)

func isInvalidEncryptedContentError(errMsg *interfaces.ErrorMessage) bool {
	if errMsg == nil || errMsg.Error == nil {
		return false
	}
	return strings.Contains(strings.ToLower(errMsg.Error.Error()), "invalid_encrypted_content")
}

// sanitizeInvalidEncryptedContentRetryBody derives a one-shot recovery request
// without mutating the canonical body. json.Number preserves large integer
// lexemes that would otherwise lose precision through float64 decoding.
func sanitizeInvalidEncryptedContentRetryBody(body []byte) ([]byte, bool) {
	if len(bytes.TrimSpace(body)) == 0 {
		return body, false
	}

	var request map[string]any
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.UseNumber()
	if err := decoder.Decode(&request); err != nil {
		return body, false
	}

	input, ok := request["input"]
	if !ok {
		return body, false
	}

	changed := false
	switch value := input.(type) {
	case []any:
		filtered := make([]any, 0, len(value))
		for _, item := range value {
			next, itemChanged, keep := sanitizeInvalidEncryptedContentInputItem(item)
			changed = changed || itemChanged
			if keep {
				filtered = append(filtered, next)
			}
		}
		if !changed {
			return body, false
		}
		if len(filtered) == 0 {
			delete(request, "input")
		} else {
			request["input"] = filtered
		}
	case map[string]any:
		next, itemChanged, keep := sanitizeInvalidEncryptedContentInputItem(value)
		if !itemChanged {
			return body, false
		}
		changed = true
		if keep {
			request["input"] = next
		} else {
			delete(request, "input")
		}
	default:
		return body, false
	}

	if !changed {
		return body, false
	}
	updated, err := json.Marshal(request)
	if err != nil {
		return body, false
	}
	return updated, true
}

func sanitizeInvalidEncryptedContentInputItem(item any) (any, bool, bool) {
	inputItem, ok := item.(map[string]any)
	if !ok {
		return item, false, true
	}

	itemType, _ := inputItem["type"].(string)
	switch strings.TrimSpace(itemType) {
	case "compaction", "compaction_summary":
		if _, encrypted := inputItem["encrypted_content"]; encrypted {
			return nil, true, false
		}
		return item, false, true
	case "reasoning":
	default:
		return item, false, true
	}

	changed := false
	if _, encrypted := inputItem["encrypted_content"]; encrypted {
		delete(inputItem, "encrypted_content")
		changed = true
	}
	if content, exists := inputItem["content"]; exists && content == nil {
		delete(inputItem, "content")
		changed = true
	}
	if !changed {
		return item, false, true
	}
	if len(inputItem) == 1 {
		return nil, true, false
	}
	return inputItem, true, true
}
