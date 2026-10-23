package chat_completions

import (
	"crypto/sha256"
	"encoding/hex"
	"sort"
	"strconv"
	"unicode/utf8"

	"github.com/tidwall/gjson"
)

// buildCodexCallIDMap reserves the caller's short IDs before assigning long
// IDs, including IDs in later turns. Sorting makes collision handling independent
// of message order. A repeated source ID always receives the same wire ID.
func buildCodexCallIDMap(messages gjson.Result) (map[string]string, map[string]struct{}) {
	ids := map[string]struct{}{}
	for _, message := range messages.Array() {
		if message.Get("role").String() != "assistant" {
			continue
		}
		for _, call := range message.Get("tool_calls").Array() {
			if typ := call.Get("type").String(); typ != "function" && typ != "custom" {
				continue
			}
			if id := call.Get("id").String(); id != "" {
				ids[id] = struct{}{}
			}
		}
	}
	result := make(map[string]string, len(ids))
	used := make(map[string]struct{}, len(ids))
	longIDs := make([]string, 0)
	for id := range ids {
		if len(id) <= 64 {
			result[id] = id
			used[id] = struct{}{}
		} else {
			longIDs = append(longIDs, id)
		}
	}
	sort.Strings(longIDs)
	for _, id := range longIDs {
		shortID := shortenCodexCallIDIfNeeded(id)
		for attempt := 1; ; attempt++ {
			if _, exists := used[shortID]; !exists {
				break
			}
			shortID = shortenCodexCallIDIfNeeded(id + "_" + strconv.Itoa(attempt))
		}
		result[id] = shortID
		used[shortID] = struct{}{}
	}
	return result, used
}

// Codex bounds call_id by bytes. Preserve a UTF-8 prefix and hash the complete
// source ID so IDs with identical prefixes still remain distinct.
func shortenCodexCallIDIfNeeded(id string) string {
	const limit = 64
	if len(id) <= limit {
		return id
	}
	sum := sha256.Sum256([]byte(id))
	suffix := "_" + hex.EncodeToString(sum[:8])
	prefixEnd := limit - len(suffix)
	for prefixEnd > 0 && !utf8.RuneStart(id[prefixEnd]) {
		prefixEnd--
	}
	return id[:prefixEnd] + suffix
}
