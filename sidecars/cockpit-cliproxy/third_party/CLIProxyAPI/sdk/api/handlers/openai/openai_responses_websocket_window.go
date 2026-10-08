package openai

import (
	"encoding/json"
	"strings"

	"github.com/tidwall/gjson"
)

func responsesWebsocketWindowID(payload []byte) string {
	if id := strings.TrimSpace(gjson.GetBytes(payload, "client_metadata.x-codex-window-id").String()); id != "" {
		return id
	}
	metadata := gjson.GetBytes(payload, "client_metadata.x-codex-turn-metadata").String()
	return strings.TrimSpace(gjson.Get(metadata, "window_id").String())
}

// A new local context window starts a new upstream response chain.
func normalizeResponsesWebsocketContextWindow(payload []byte, previousWindowID string) ([]byte, string, bool, error) {
	windowID := responsesWebsocketWindowID(payload)
	if previousWindowID == "" || windowID == "" || previousWindowID == windowID {
		return payload, windowID, false, nil
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(payload, &object); err != nil {
		return nil, windowID, false, err
	}
	delete(object, "previous_response_id")
	updated, err := json.Marshal(object)
	return updated, windowID, true, err
}
