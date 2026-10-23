// Package collaboration handles Codex collaboration tool response metadata.
package collaboration

import (
	"strconv"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// MarkPlaintextCalls declares plaintext arguments from a known plaintext
// source, such as a Chat Completions conversion. Existing encryption metadata
// and signatures remain authoritative, including on incomplete stream items.
func MarkPlaintextCalls(payload []byte) []byte {
	return markPlaintextCalls(payload, nil, false)
}

// MarkPlaintextCallsForRequest uses only the declarations actually sent to the
// upstream. Missing, unknown or conflicting declarations never prove plaintext.
// The caller must separately enforce its optimization/client eligibility.
func MarkPlaintextCallsForRequest(payload, request []byte) []byte {
	return PlaintextPolicyForRequest(request).MarkCalls(payload)
}

// PlaintextPolicy is a snapshot of final transmitted tool declarations. Its
// zero value proves no plaintext tools and retains no request or argument text.
type PlaintextPolicy struct {
	tools, additional         map[string]toolRule
	topDeclared, invalidExtra bool
	blocked                   map[string]bool
}

type toolRule struct {
	plaintext, wireAlias bool
}

// InheritContinuation follows declaration sources: top-level tools replace that
// request parameter, while additional_tools blocks append to conversation input.
// Conflicting redeclarations cannot grant plaintext, and malformed blocks clear
// inherited additional-tool evidence. No request text is retained.
func (p PlaintextPolicy) InheritContinuation(previous PlaintextPolicy) PlaintextPolicy {
	blocked := make(map[string]bool)
	for name, denied := range previous.blocked {
		blocked[name] = denied
	}
	for _, rules := range []map[string]toolRule{p.tools, p.additional} {
		for name, rule := range rules {
			if !rule.plaintext {
				blocked[name] = true
			}
		}
	}
	for _, rules := range []map[string]toolRule{previous.tools, previous.additional} {
		for name, rule := range rules {
			if !rule.plaintext {
				blocked[name] = true
			}
		}
	}
	p.blocked = blocked
	if !p.topDeclared {
		p.tools = previous.tools
	}
	if !p.invalidExtra {
		merged := make(map[string]toolRule, len(previous.additional)+len(p.additional))
		for name, rule := range previous.additional {
			merged[name] = rule
		}
		for name, rule := range p.additional {
			mergeRule(merged, name, rule)
		}
		p.additional = merged
	}
	return p
}

func (p PlaintextPolicy) MarkCalls(payload []byte) []byte {
	return markPlaintextCalls(payload, p.rules(), false)
}

// MarkWireCalls also recognizes only aliases authorized by an actual optimizer
// rename and present in the final sent declarations; it changes no wire identity.
func (p PlaintextPolicy) MarkWireCalls(payload []byte) []byte {
	return markPlaintextCalls(payload, p.rules(), true)
}

func (p PlaintextPolicy) rules() map[string]toolRule {
	combined := make(map[string]toolRule, len(p.tools)+len(p.additional))
	for name, rule := range p.tools {
		combined[name] = rule
	}
	for name, rule := range p.additional {
		mergeRule(combined, name, rule)
	}
	for name, denied := range p.blocked {
		if denied {
			combined[name] = toolRule{}
		}
	}
	return combined
}

func mergeRule(rules map[string]toolRule, name string, rule toolRule) {
	if previous, exists := rules[name]; exists {
		rule.plaintext = previous.plaintext && rule.plaintext
		rule.wireAlias = previous.wireAlias || rule.wireAlias
	}
	rules[name] = rule
}

// PlaintextPolicyForRequest parses a final request once rather than rescanning
// its potentially large input on every streaming response event.
// optimizedNamespace only authorizes the reserved wire alias; plaintext still
// requires an explicit matching final declaration without encryption.
func PlaintextPolicyForRequest(request []byte, optimizedNamespace ...bool) PlaintextPolicy {
	if !gjson.ValidBytes(request) {
		return PlaintextPolicy{topDeclared: true, invalidExtra: true}
	}
	policy := PlaintextPolicy{tools: make(map[string]toolRule), additional: make(map[string]toolRule)}
	allowAlias := len(optimizedNamespace) > 0 && optimizedNamespace[0]
	var scan func(gjson.Result, string, map[string]toolRule)
	scan = func(tools gjson.Result, namespace string, rules map[string]toolRule) {
		if !tools.IsArray() {
			return
		}
		for _, tool := range tools.Array() {
			if tool.Get("type").String() == "namespace" {
				scan(tool.Get("tools"), tool.Get("name").String(), rules)
				continue
			}
			name, ok := messageToolName(tool.Get("name").String(), namespace)
			if !ok {
				continue
			}
			message := tool.Get("parameters.properties.message")
			encrypted := message.Get("encrypted")
			plaintext := tool.Get("type").String() == "function" && message.IsObject() && message.Get("type").String() == "string" && (!encrypted.Exists() || encrypted.Type == gjson.False)
			alias := namespace == "collaboration-optimize" || strings.HasPrefix(tool.Get("name").String(), "collaboration-optimize")
			mergeRule(rules, name, toolRule{plaintext: plaintext && (!alias || allowAlias), wireAlias: alias && allowAlias})
		}
	}
	root := gjson.ParseBytes(request)
	policy.topDeclared = root.Get("tools").Exists()
	scan(root.Get("tools"), "", policy.tools)
	for _, item := range root.Get("input").Array() {
		if item.Get("type").String() == "additional_tools" {
			if !item.Get("tools").IsArray() {
				policy.invalidExtra = true
			}
			scan(item.Get("tools"), "", policy.additional)
		}
	}
	if policy.invalidExtra {
		policy.additional = nil
	}
	return policy
}

func markPlaintextCalls(payload []byte, policy map[string]toolRule, wire bool) []byte {
	if !gjson.ValidBytes(payload) {
		return payload
	}
	root := gjson.ParseBytes(payload)
	paths := []string{""}
	switch root.Get("type").String() {
	case "response.output_item.added", "response.output_item.done":
		paths = append(paths, "item")
	}
	for _, prefix := range []string{"output", "response.output"} {
		for index := range root.Get(prefix).Array() {
			paths = append(paths, prefix+"."+strconv.Itoa(index))
		}
	}
	for _, path := range paths {
		item := root
		if path != "" {
			item = gjson.GetBytes(payload, path)
		}
		if item.Get("type").String() != "function_call" {
			continue
		}
		// A user-defined reserved namespace is not a registered collaboration
		// call. Actual optimizer aliases have already been restored by the caller.
		alias := item.Get("namespace").String() == "collaboration-optimize" || strings.HasPrefix(item.Get("name").String(), "collaboration-optimize")
		if alias && !wire {
			continue
		}
		name, ok := messageToolName(item.Get("name").String(), item.Get("namespace").String())
		if !ok || policy != nil && (!policy[name].plaintext || alias && !policy[name].wireAlias) {
			continue
		}
		if item.Get("encrypted_content").Exists() || item.Get("signature").Exists() || item.Get("encrypted_function_args").Type != gjson.Null {
			continue
		}
		field := "encrypted_function_args"
		if path != "" {
			field = path + "." + field
		}
		if updated, err := sjson.SetRawBytes(payload, field, []byte("[]")); err == nil {
			payload = updated
		}
	}
	return payload
}

func messageToolName(name, namespace string) (string, bool) {
	if namespace != "" && namespace != "collaboration" && namespace != "collaboration-optimize" {
		return "", false
	}
	for _, prefix := range []string{"collaboration__", "collaboration.", "collaboration::", "collaboration-optimize__", "collaboration-optimize.", "collaboration-optimize::"} {
		if strings.HasPrefix(name, prefix) {
			namespace, name = "collaboration", strings.TrimPrefix(name, prefix)
			break
		}
	}
	if namespace == "" {
		return "", false
	}
	switch name {
	case "spawn_agent", "send_message", "followup_task":
		return name, true
	}
	return "", false
}
