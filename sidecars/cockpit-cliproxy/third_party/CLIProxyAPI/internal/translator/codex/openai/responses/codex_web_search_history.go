package responses

import (
	"bytes"
	"fmt"
	"strings"

	translatorcommon "github.com/router-for-me/CLIProxyAPI/v7/internal/translator/common"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// EnsureCodexWebSearchHistoryTool declares the cached-only hosted tool required
// to replay web_search_call history. Callers must restrict this to official
// Codex OAuth /responses requests, after Lite filtering and history replay;
// API-key gateways and /responses/compact have different wire contracts.
// Lite declarations belong in input additional_tools before a trailing trigger.
func EnsureCodexWebSearchHistoryTool(body []byte, lite bool) []byte {
	// JSON strings may spell this ASCII enum using Unicode escapes. Only
	// skip parsing when neither representation can occur.
	if (!bytes.Contains(body, []byte(`"web_search_call"`)) && !bytes.Contains(body, []byte(`\u`))) || !gjson.ValidBytes(body) {
		return body
	}
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() {
		return body
	}
	items := input.Array()
	hasSearchHistory := false
	callerDeclaredTools := false
	additionalIndex := -1
	for index, item := range items {
		switch strings.TrimSpace(item.Get("type").String()) {
		case "web_search_call":
			hasSearchHistory = true
		case "additional_tools":
			tools := item.Get("tools")
			if codexToolsDeclareWebSearch(tools) {
				return body
			}
			callerDeclaredTools = callerDeclaredTools || tools.IsArray() && len(tools.Array()) > 0
			if additionalIndex < 0 {
				additionalIndex = index
			}
		}
	}
	tools := gjson.GetBytes(body, "tools")
	if !hasSearchHistory || codexToolsDeclareWebSearch(tools) {
		return body
	}
	topLevelTools := tools.IsArray() && len(tools.Array()) > 0
	callerDeclaredTools = callerDeclaredTools || topLevelTools
	// Never turn a historical declaration into a new network search capability.
	tool := []byte(`{"type":"web_search","external_web_access":false}`)
	var next []byte
	var errSet error
	switch {
	case !lite && topLevelTools:
		next, errSet = sjson.SetRawBytes(body, "tools.-1", tool)
	case !lite:
		next, errSet = sjson.SetRawBytes(body, "tools", translatorcommon.JoinRawArray([][]byte{tool}))
	case additionalIndex >= 0:
		path := fmt.Sprintf("input.%d.tools", additionalIndex)
		if items[additionalIndex].Get("tools").IsArray() {
			next, errSet = sjson.SetRawBytes(body, path+".-1", tool)
		} else {
			next, errSet = sjson.SetRawBytes(body, path, translatorcommon.JoinRawArray([][]byte{tool}))
		}
	default:
		insertAt := len(items)
		if insertAt > 0 && items[insertAt-1].Get("type").String() == "compaction_trigger" {
			insertAt--
		}
		additional := []byte(`{"type":"additional_tools","role":"developer","tools":[{"type":"web_search","external_web_access":false}]}`)
		rebuilt := make([][]byte, 0, len(items)+1)
		for index, item := range items {
			if index == insertAt {
				rebuilt = append(rebuilt, additional)
			}
			rebuilt = append(rebuilt, []byte(item.Raw))
		}
		if insertAt == len(items) {
			rebuilt = append(rebuilt, additional)
		}
		next, errSet = sjson.SetRawBytes(body, "input", translatorcommon.JoinRawArray(rebuilt))
	}
	if errSet != nil {
		return body
	}
	if !callerDeclaredTools {
		choice := gjson.GetBytes(next, "tool_choice")
		pinChoice := !choice.Exists() || choice.Type == gjson.Null
		if choice.Type == gjson.String {
			switch strings.ToLower(strings.TrimSpace(choice.String())) {
			case "", "auto", "none":
				pinChoice = true
			}
		}
		// Keep required and explicit tool selections, as well as all caller
		// choices when other tools are present.
		if pinChoice {
			next, errSet = sjson.SetBytes(next, "tool_choice", "none")
			if errSet != nil {
				return body
			}
		}
	}
	return next
}

func codexToolsDeclareWebSearch(tools gjson.Result) bool {
	if !tools.IsArray() {
		return false
	}
	for _, tool := range tools.Array() {
		if strings.HasPrefix(strings.TrimSpace(tool.Get("type").String()), "web_search") {
			return true
		}
	}
	return false
}
