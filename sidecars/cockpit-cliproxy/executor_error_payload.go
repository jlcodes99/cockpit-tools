package main

import (
	"encoding/json"
	"errors"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

// Selection failures happen locally, before an upstream attempt exists. Keep
// their machine-readable code rather than flattening them to upstream_error.
func publicExecutorFailure(err error) (string, bool) {
	var selection *coreauth.Error
	if errors.As(err, &selection) && selection != nil {
		switch selection.Code {
		case "auth_not_found", "auth_unavailable":
			payload, marshalErr := json.Marshal(map[string]any{"error": map[string]any{
				"code": selection.Code, "type": "server_error", "message": selection.Message,
			}})
			if marshalErr == nil {
				return publicExecutorError(string(payload))
			}
		}
	}
	return publicExecutorError(errorMessage(err))
}

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
