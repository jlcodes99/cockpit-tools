package main

import (
	"encoding/json"
	"fmt"
	"net/url"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// providerGatewayRepairsToolCallPairing reports whether the upstream verifies tool call and
// output pairing per request and therefore needs the pairing repair below.
func providerGatewayRepairsToolCallPairing(gateway *providerGatewaySpec) bool {
	if gateway == nil {
		return false
	}
	parsed, err := url.Parse(strings.TrimSpace(gateway.BaseURL))
	if err != nil {
		return false
	}
	return strings.EqualFold(parsed.Hostname(), "api.deepseek.com")
}

// providerGatewayToolCallOutputPlaceholder is the explicit failure result used when the
// Codex app-server serializes a tool call before its output was committed. The upstream
// requires a matching output; the placeholder keeps the transcript valid without claiming
// the tool succeeded, so the model can decide to retry.
const providerGatewayToolCallOutputPlaceholder = "tool result unavailable: the local agent did not commit an output for this call"

// providerGatewayNormalizeToolCallPairing enforces the "every tool call has a tool output"
// invariant that strict Responses upstreams verify per request. The Codex app-server can
// serialize the next sampling request before the last function_call_output of a parallel
// batch reaches the conversation history, which makes the upstream reject the whole turn
// with `No tool output found ...` and leaves the in-memory thread unusable. Injecting the
// missing output (or the call that an injected standalone output never had) keeps the
// request valid and stops the thread from wedging.
func providerGatewayNormalizeToolCallPairing(gateway *providerGatewaySpec, body []byte) ([]byte, int, int) {
	if !providerGatewayRepairsToolCallPairing(gateway) {
		return body, 0, 0
	}
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() || len(input.Array()) == 0 {
		return body, 0, 0
	}
	type callSlot struct {
		index int
		item  gjson.Result
	}
	calls := make(map[string]callSlot)
	order := make([]string, 0)
	for index, item := range input.Array() {
		if !providerGatewayIsToolCallItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			continue
		}
		if _, exists := calls[callID]; !exists {
			order = append(order, callID)
		}
		calls[callID] = callSlot{index: index, item: item}
	}

	answers := make(map[string]struct{})
	for _, item := range input.Array() {
		if !providerGatewayIsToolCallOutputItem(item.Get("type").String()) {
			continue
		}
		if callID := strings.TrimSpace(item.Get("call_id").String()); callID != "" {
			answers[callID] = struct{}{}
		}
	}

	// Exhaustive pairing means every live call already has an output, so there is nothing to
	// repair and the request is forwarded byte-for-byte. Requests without any call still fall
	// through so standalone injected outputs get their missing call.
	if len(answers) > 0 && len(calls) == len(answers) {
		return body, 0, 0
	}

	orphanAnswers := make(map[int]struct{})
	for index, item := range input.Array() {
		if !providerGatewayIsToolCallOutputItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			orphanAnswers[index] = struct{}{}
			continue
		}
		if _, exists := calls[callID]; !exists {
			orphanAnswers[index] = struct{}{}
		}
	}

	insertedAt := make(map[int][]string)
	placeholderCount := 0
	for _, callID := range order {
		if _, answered := answers[callID]; answered {
			continue
		}
		insertedAt[calls[callID].index] = append(insertedAt[calls[callID].index], callID)
		placeholderCount++
	}

	rebuilt := make([]byte, 0, len(body)+256*placeholderCount)
	rebuilt = append(rebuilt, '[')
	written := 0
	writeItem := func(raw string) {
		if written > 0 {
			rebuilt = append(rebuilt, ',')
		}
		rebuilt = append(rebuilt, raw...)
		written++
	}
	for index, item := range input.Array() {
		raw := item.Raw
		if _, orphan := orphanAnswers[index]; orphan {
			// A standalone output without a call cannot exist on a strict upstream. The
			// app server injects this shape for automations and cross-thread messages, so
			// keep the content and add the call the output belongs to.
			callID := providerGatewayStandaloneCallID(item)
			writeItem(providerGatewaySynthesizeCallForOutput(item, callID))
			raw = providerGatewayOutputWithCallID(item, callID)
		}
		writeItem(raw)
		for _, callID := range insertedAt[index] {
			writeItem(providerGatewayPlaceholderOutputJSON(item.Get("type").String(), callID))
		}
	}
	rebuilt = append(rebuilt, ']')

	updated, err := sjson.SetRawBytes(body, "input", rebuilt)
	if err != nil {
		return body, 0, 0
	}
	return updated, placeholderCount, len(orphanAnswers)
}

func providerGatewayIsToolCallItem(itemType string) bool {
	switch strings.ToLower(strings.TrimSpace(itemType)) {
	case "function_call", "custom_tool_call":
		return true
	default:
		return false
	}
}

func providerGatewayIsToolCallOutputItem(itemType string) bool {
	switch strings.ToLower(strings.TrimSpace(itemType)) {
	case "function_call_output", "custom_tool_call_output":
		return true
	default:
		return false
	}
}

// providerGatewayPlaceholderOutputJSON builds the stand-in output for a call whose result
// never reached the transcript. It mirrors the call flavor so a custom tool call does not
// receive a function-shaped output.
func providerGatewayPlaceholderOutputJSON(callItemType string, callID string) string {
	outputType := "function_call_output"
	if strings.EqualFold(strings.TrimSpace(callItemType), "custom_tool_call") {
		outputType = "custom_tool_call_output"
	}
	return fmt.Sprintf(
		`{"type":%q,"call_id":%q,"output":%q}`,
		outputType,
		callID,
		providerGatewayToolCallOutputPlaceholder,
	)
}

// providerGatewaySynthesizeCallForOutput converts an injected standalone output into the
// call/output pair a strict upstream accepts. Adding only a call_id is not enough: without a
// matching function_call the upstream answers `No tool call found for tool output ...`.
func providerGatewaySynthesizeCallForOutput(item gjson.Result, callID string) string {
	itemType := strings.ToLower(strings.TrimSpace(item.Get("type").String()))
	callType := "function_call"
	argumentsKey := "arguments"
	if itemType == "custom_tool_call_output" {
		callType = "custom_tool_call"
		argumentsKey = "input"
	}
	name := strings.TrimSpace(item.Get("name").String())
	if name == "" {
		name = "external_tool"
	}
	call := map[string]any{
		"type":    callType,
		"call_id": callID,
		"name":    name,
	}
	if argumentsKey == "arguments" {
		call["arguments"] = "{}"
	} else {
		call["input"] = ""
	}
	if namespace := strings.TrimSpace(item.Get("namespace").String()); namespace != "" {
		call["namespace"] = namespace
	}
	encoded, err := json.Marshal(call)
	if err != nil {
		return item.Raw
	}
	return string(encoded)
}

// providerGatewayStandaloneCallID derives the shared call_id for a synthesized pair. The
// source item id is used when the app server provided one so the mapping stays explainable
// in logs and rollouts; the result is sanitized because it travels as an opaque token.
func providerGatewayStandaloneCallID(item gjson.Result) string {
	if callID := strings.TrimSpace(item.Get("call_id").String()); callID != "" {
		return callID
	}
	return "call_standalone_" + providerGatewayShortToken(item.Get("id").String())
}

// providerGatewayOutputWithCallID rewrites the output item so it references the caller's
// call_id. The original item is returned unchanged if the rewrite fails.
func providerGatewayOutputWithCallID(item gjson.Result, callID string) string {
	updated, err := sjson.SetBytes([]byte(item.Raw), "call_id", callID)
	if err != nil {
		return item.Raw
	}
	return string(updated)
}

func providerGatewayShortToken(raw string) string {
	token := strings.TrimSpace(raw)
	if token == "" {
		return "orphan"
	}
	if len(token) > 24 {
		token = token[len(token)-24:]
	}
	token = strings.NewReplacer("\"", "", "\\", "", " ", "").Replace(token)
	return token
}
