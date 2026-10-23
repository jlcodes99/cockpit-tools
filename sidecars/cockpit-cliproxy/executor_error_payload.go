package main

import (
	"encoding/json"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
)

// Preserve structured upstream errors at the outer relay boundary too. The SDK
// has already classified EOF, missing completion and deadlines independently.
func publicExecutorError(message string) (string, bool) {
	var value map[string]any
	if len(message) <= 64<<10 {
		decoder := json.NewDecoder(strings.NewReader(message))
		decoder.UseNumber()
		if decoder.Decode(&value) == nil && json.Valid([]byte(message)) {
			var detail map[string]any
			if node, ok := value["error"].(map[string]any); ok {
				detail = node
			} else if response, ok := value["response"].(map[string]any); ok {
				detail, _ = response["error"].(map[string]any)
			}
			if detail != nil {
				payload, err := json.Marshal(map[string]any{"error": sanitizeExecutorErrorValue(detail, 0)})
				if err == nil {
					return string(payload), true
				}
			}
		}
	}
	return helps.RedactStreamTransportError(message), false
}

func sanitizeExecutorErrorValue(value any, depth int) any {
	if depth > 16 {
		return "[omitted: nesting limit]"
	}
	switch value := value.(type) {
	case map[string]any:
		for key, child := range value {
			if diagnosticSensitiveField(key) {
				value[key] = "[REDACTED]"
			} else {
				value[key] = sanitizeExecutorErrorValue(child, depth+1)
			}
		}
		return value
	case []any:
		for i := range value {
			value[i] = sanitizeExecutorErrorValue(value[i], depth+1)
		}
		return value
	case string:
		return helps.RedactStreamTransportError(value)
	default:
		return value
	}
}
