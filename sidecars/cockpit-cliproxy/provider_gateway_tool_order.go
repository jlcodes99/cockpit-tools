package main

import (
	"net/url"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// providerGatewayRepairsToolCallOrder reports whether the upstream verifies that every tool
// call's output directly follows that call.
func providerGatewayRepairsToolCallOrder(gateway *providerGatewaySpec) bool {
	if gateway == nil {
		return false
	}
	parsed, err := url.Parse(strings.TrimSpace(gateway.BaseURL))
	if err != nil {
		return false
	}
	return strings.EqualFold(parsed.Hostname(), "api.deepseek.com")
}

// providerGatewayOrderToolCallPairs restores the positional invariant DeepSeek verifies: every
// tool call must be immediately followed by its own output.
//
// Codex breaks that ordering on a normal path. A tool hook (PostToolUse) records its developer
// message as soon as the tool returns, which can be before the tool's output item reaches
// conversation history, so the transcript ends up as
//
//	function_call -> message -> function_call_output
//
// DeepSeek validates adjacency per request and answers `No tool output found for tool call ...`
// for the whole request, which wedges the thread until it is unloaded. The pairing repair above
// does not cover this: the items exist, they are merely ordered illegally.
//
// The rewrite is deterministic and lossless. Each live call is paired with its output, a run of
// consecutive calls keeps its relative order, and every other item keeps its position relative
// to those pairs. Only an output that is not already in place moves, and it moves forward to sit
// directly behind its call.
func providerGatewayOrderToolCallPairs(body []byte) ([]byte, int, bool) {
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() {
		return body, 0, true
	}
	items := input.Array()
	if len(items) < 2 {
		return body, 0, true
	}
	if !providerGatewayToolCallOrderNeedsRepair(input) {
		return body, 0, true
	}

	// The upstream pairs a call with the next item that carries the same call_id, so the rebuilt
	// transcript has to put that token directly behind its call. Two position facts decide the
	// whole rewrite, and both are computed up front so the rebuild stays a single pass:
	//
	//   firstCall[id] / firstOutput[id]  where the call and its answer first appear
	//   relocated                        answers that did not follow their call in the original
	//
	// An answer that is already in place stays where it is; an answer displaced by a later call
	// or an intervening message is emitted behind its call, and anything that sat in between
	// follows it. Items the model does not need to re-read in order keep their relative order.
	firstCall := make(map[string]int)
	firstOutput := make(map[string]int)
	relocated := 0
	for index, item := range items {
		itemType := item.Get("type").String()
		switch {
		case providerGatewayOrderIsToolCallItem(itemType):
			callID := strings.TrimSpace(item.Get("call_id").String())
			if callID == "" {
				return body, 0, false
			}
			if _, seen := firstCall[callID]; !seen {
				firstCall[callID] = index
			}
		case providerGatewayOrderIsToolCallOutputItem(itemType):
			callID := strings.TrimSpace(item.Get("call_id").String())
			if callID == "" {
				return body, 0, false
			}
			if _, seen := firstOutput[callID]; !seen {
				firstOutput[callID] = index
			}
		}
	}
	for callID, outputIndex := range firstOutput {
		if callIndex, ok := firstCall[callID]; !ok || callIndex != outputIndex-1 {
			relocated++
		}
	}

	ordered := make([]gjson.Result, 0, len(items))
	callIndex := make(map[string]int)     // position of each call's item inside ordered
	waiting := make(map[string]struct{})  // calls emitted so far whose answer has not been placed
	answered := make(map[string]struct{}) // calls whose answer is in place
	takeCall := func(callID string) {
		delete(answered, callID)
		waiting[callID] = struct{}{}
	}
	placeAnswer := func(callID string, item gjson.Result) {
		position, ok := callIndex[callID]
		if !ok {
			return
		}
		ordered = append(ordered, gjson.Result{})
		copy(ordered[position+2:], ordered[position+1:])
		ordered[position+1] = item
		delete(waiting, callID)
		answered[callID] = struct{}{}
		for id, slot := range callIndex {
			if slot > position {
				callIndex[id] = slot + 1
			}
		}
	}
	for index, item := range items {
		itemType := item.Get("type").String()
		if providerGatewayOrderIsToolCallItem(itemType) {
			callID := strings.TrimSpace(item.Get("call_id").String())
			callIndex[callID] = len(ordered)
			ordered = append(ordered, item)
			if outputIndex, ok := firstOutput[callID]; !ok || outputIndex > index {
				takeCall(callID)
			} else {
				answered[callID] = struct{}{}
			}
			continue
		}
		if providerGatewayOrderIsToolCallOutputItem(itemType) {
			callID := strings.TrimSpace(item.Get("call_id").String())
			if _, done := answered[callID]; done {
				// A duplicate answer cannot sit next to its call any more, so it is dropped: the
				// alternative is a transcript the upstream rejects outright.
				continue
			}
			if _, owed := waiting[callID]; owed {
				placeAnswer(callID, item)
				continue
			}
			if firstOutput[callID] != index {
				continue
			}
			if _, hasCall := firstCall[callID]; hasCall {
				continue
			}
			// An answer with no call in this request keeps its place; the pairing repair above
			// already gave it a matching call when the upstream requires one.
			ordered = append(ordered, item)
			continue
		}
		ordered = append(ordered, item)
	}

	rebuilt := make([]byte, 0, len(body))
	rebuilt = append(rebuilt, '[')
	for index, item := range ordered {
		if index > 0 {
			rebuilt = append(rebuilt, ',')
		}
		rebuilt = append(rebuilt, item.Raw...)
	}
	rebuilt = append(rebuilt, ']')

	updated, err := sjson.SetRawBytes(body, "input", rebuilt)
	if err != nil {
		return body, 0, false
	}
	if !providerGatewayToolCallPairsAdjacent(gjson.GetBytes(updated, "input")) {
		return body, 0, false
	}
	return updated, relocated, true
}

// providerGatewayToolCallOrderNeedsRepair reports whether any tool call is answered by
// something other than its own output, which is the positional rule DeepSeek enforces.
func providerGatewayToolCallOrderNeedsRepair(input gjson.Result) bool {
	if !input.IsArray() {
		return false
	}
	items := input.Array()
	for index, item := range items {
		if !providerGatewayOrderIsToolCallItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			continue
		}
		if index+1 >= len(items) {
			// No output recorded in this request; the pairing repair handles that case.
			continue
		}
		next := items[index+1]
		if !providerGatewayOrderIsToolCallOutputItem(next.Get("type").String()) ||
			strings.TrimSpace(next.Get("call_id").String()) != callID {
			return true
		}
	}
	return false
}

// providerGatewayToolCallPairsAdjacent is the post-condition of providerGatewayOrderToolCallPairs.
func providerGatewayToolCallPairsAdjacent(input gjson.Result) bool {
	if !input.IsArray() {
		return false
	}
	items := input.Array()
	answered := make(map[string]struct{})
	for _, item := range items {
		if !providerGatewayOrderIsToolCallOutputItem(item.Get("type").String()) {
			continue
		}
		if callID := strings.TrimSpace(item.Get("call_id").String()); callID != "" {
			answered[callID] = struct{}{}
		}
	}
	for index, item := range items {
		if !providerGatewayOrderIsToolCallItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			return false
		}
		// A call whose result is genuinely absent from the request cannot be paired here; the
		// pairing repair decides what to do with it, so adjacency does not apply.
		if _, hasAnswer := answered[callID]; !hasAnswer {
			continue
		}
		if index+1 >= len(items) {
			return false
		}
		next := items[index+1]
		if !providerGatewayOrderIsToolCallOutputItem(next.Get("type").String()) ||
			strings.TrimSpace(next.Get("call_id").String()) != callID {
			return false
		}
	}
	return true
}

func providerGatewayOrderIsToolCallItem(itemType string) bool {
	switch strings.ToLower(strings.TrimSpace(itemType)) {
	case "function_call", "custom_tool_call":
		return true
	default:
		return false
	}
}

func providerGatewayOrderIsToolCallOutputItem(itemType string) bool {
	switch strings.ToLower(strings.TrimSpace(itemType)) {
	case "function_call_output", "custom_tool_call_output":
		return true
	default:
		return false
	}
}
