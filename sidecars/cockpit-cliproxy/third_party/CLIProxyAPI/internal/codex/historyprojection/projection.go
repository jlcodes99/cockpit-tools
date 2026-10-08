// Package historyprojection projects Codex conversation history onto the
// capabilities of a third-party upstream.
//
// Codex clients replay their own protocol extensions verbatim: custom tools,
// namespace tools, encrypted reasoning items, web_search_call items, agent
// messages and Responses Lite declarations. Third-party Responses endpoints
// implement only a subset of that protocol and validate strictly, so replaying
// raw history produces 422s that differ per upstream. This package owns the
// single projection pipeline shared by every third-party route, so a new
// upstream quirk is handled once instead of per executor.
package historyprojection

import (
	"bytes"
	"encoding/json"
	"math"
	"strconv"
	"strings"

	"github.com/tidwall/gjson"
)

// UpstreamKind identifies the third-party upstream a request is projected for.
type UpstreamKind string

const (
	UpstreamGeneric  UpstreamKind = "generic"
	UpstreamDeepSeek UpstreamKind = "deepseek"
	UpstreamXAI      UpstreamKind = "xai"
)

// Profile describes the history shapes an upstream accepts.
type Profile struct {
	Kind UpstreamKind
	// FillWebSearchQueries fills `queries` from `query` when the upstream's
	// strict schema requires the plural field.
	FillWebSearchQueries bool
	// FillWebSearchQuery fills `query` from `queries[0]` when required.
	FillWebSearchQuery bool
	// NormalizeIntegralArguments rewrites integer-valued floating point
	// arguments (for example 180000.0) into integers before the upstream
	// replays them.
	NormalizeIntegralArguments bool
}

// ProfileFor returns the projection profile for a known upstream kind.
func ProfileFor(kind UpstreamKind) Profile {
	return Profile{
		Kind:                       kind,
		FillWebSearchQueries:       true,
		FillWebSearchQuery:         true,
		NormalizeIntegralArguments: true,
	}
}

// Project rewrites a Responses payload (a full body or a single SSE event
// payload) so it can be replayed by the upstream described by profile. It
// returns the original bytes when nothing needed to change.
func Project(payload []byte, profile Profile) []byte {
	if len(payload) == 0 || !gjson.ValidBytes(payload) {
		return payload
	}
	decoder := json.NewDecoder(bytes.NewReader(payload))
	decoder.UseNumber()
	var value any
	if errDecode := decoder.Decode(&value); errDecode != nil {
		return payload
	}
	if !projectValue(value, profile) {
		return payload
	}
	projected, errMarshal := json.Marshal(value)
	if errMarshal != nil {
		return payload
	}
	return projected
}

func projectValue(value any, profile Profile) bool {
	changed := false
	switch typed := value.(type) {
	case []any:
		for _, item := range typed {
			if projectValue(item, profile) {
				changed = true
			}
		}
	case map[string]any:
		itemType := strings.TrimSpace(stringValue(typed, "type"))
		isToolCall := itemType == "function_call" || itemType == "custom_tool_call"
		isToolOutput := itemType == "function_call_output" || itemType == "custom_tool_call_output"
		switch itemType {
		case "web_search_call":
			if projectWebSearchAction(typed, profile) {
				changed = true
			}
		case "function_call":
			if profile.NormalizeIntegralArguments {
				if raw, ok := typed["arguments"].(string); ok {
					if normalized := NormalizeIntegralNumbersInArguments(raw); normalized != raw {
						typed["arguments"] = normalized
						changed = true
					}
				}
			}
		}
		for key, child := range typed {
			if isToolCall && (key == "arguments" || key == "input") {
				continue
			}
			if isToolOutput && key == "output" {
				continue
			}
			if projectValue(child, profile) {
				changed = true
			}
		}
	}
	return changed
}

func projectWebSearchAction(item map[string]any, profile Profile) bool {
	action, ok := item["action"].(map[string]any)
	if !ok {
		return false
	}
	if strings.TrimSpace(stringValue(action, "type")) != "search" {
		return false
	}
	changed := false
	query := strings.TrimSpace(stringValue(action, "query"))
	queries := stringSlice(action["queries"])
	if profile.FillWebSearchQueries && len(queries) == 0 && query != "" {
		action["queries"] = []any{query}
		changed = true
	}
	if profile.FillWebSearchQuery && query == "" && len(queries) > 0 {
		action["query"] = queries[0]
		changed = true
	}
	return changed
}

func stringValue(value map[string]any, key string) string {
	text, _ := value[key].(string)
	return text
}

func stringSlice(value any) []string {
	items, ok := value.([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(items))
	for _, item := range items {
		text, ok := item.(string)
		if !ok {
			continue
		}
		if text = strings.TrimSpace(text); text != "" {
			out = append(out, text)
		}
	}
	return out
}

// NormalizeIntegralNumbersInArguments rewrites integer-valued floating point
// numbers (for example {"timeout_ms":180000.0}) inside a JSON arguments object
// into integers. Strictly typed Codex clients deserialize these arguments as
// i64 and reject float payloads.
func NormalizeIntegralNumbersInArguments(raw string) string {
	trimmed := strings.TrimSpace(raw)
	if trimmed == "" || !gjson.Valid(trimmed) {
		return raw
	}
	decoder := json.NewDecoder(strings.NewReader(trimmed))
	decoder.UseNumber()
	var value any
	if errDecode := decoder.Decode(&value); errDecode != nil {
		return raw
	}
	if !normalizeIntegralNumbers(value) {
		return raw
	}
	encoded, errMarshal := json.Marshal(value)
	if errMarshal != nil {
		return raw
	}
	return string(encoded)
}

func normalizeIntegralNumbers(value any) bool {
	changed := false
	switch typed := value.(type) {
	case []any:
		for index, item := range typed {
			if number, ok := item.(json.Number); ok {
				if normalized, okNumber := integralJSONNumber(number); okNumber {
					typed[index] = normalized
					changed = true
				}
				continue
			}
			if normalizeIntegralNumbers(item) {
				changed = true
			}
		}
	case map[string]any:
		for key, child := range typed {
			if number, ok := child.(json.Number); ok {
				if normalized, okNumber := integralJSONNumber(number); okNumber {
					typed[key] = normalized
					changed = true
				}
				continue
			}
			if normalizeIntegralNumbers(child) {
				changed = true
			}
		}
	}
	return changed
}

func integralJSONNumber(number json.Number) (json.Number, bool) {
	text := number.String()
	if !strings.ContainsAny(text, ".eE") {
		return number, false
	}
	value, errParse := number.Float64()
	if errParse != nil || math.IsNaN(value) || math.IsInf(value, 0) || value != math.Trunc(value) {
		return number, false
	}
	if value < math.MinInt64 || value > math.MaxInt64 {
		return number, false
	}
	return json.Number(strconv.FormatInt(int64(value), 10)), true
}
