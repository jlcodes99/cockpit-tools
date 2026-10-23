package helps

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"sync"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const xaiSearchMaxDeclarations = 8 * 1024 * 1024

// XAIClientToolSearchSession is owned by an existing websocket session. It
// retains only declarations, never outputs or credentials, and resets when
// account scope changes. Explicit tools (including []) replace the cache.
// Each session serializes its Inherit/Remember pair through the executor's
// request lock, preserving which declarations came from the client.
type XAIClientToolSearchSession struct {
	mu                   sync.Mutex
	scope                [32]byte
	declarations         []byte
	explicitDeclarations []byte
	retainingExplicit    bool
	pendingExplicit      []byte
}

func (s *XAIClientToolSearchSession) Inherit(body []byte, accountScope string, continuation bool) ([]byte, error) {
	if s == nil {
		return body, nil
	}
	scope := sha256.Sum256([]byte(accountScope))
	s.mu.Lock()
	if s.scope != scope {
		s.scope, s.declarations, s.explicitDeclarations = scope, nil, nil
	}
	declarations := bytes.Clone(s.declarations)
	explicitDeclarations := bytes.Clone(s.explicitDeclarations)
	s.pendingExplicit = nil
	s.retainingExplicit = continuation && !gjson.GetBytes(body, "tools").Exists() && len(declarations) > 0
	inherit := s.retainingExplicit
	s.mu.Unlock()
	if !inherit {
		return body, nil
	}
	var explicit, fallback []any
	if err := xaiSearchDecode(explicitDeclarations, &explicit); err != nil {
		return nil, err
	}
	if err := xaiSearchDecode(declarations, &fallback); err != nil {
		return nil, err
	}
	var request map[string]any
	if err := xaiSearchDecode(body, &request); err != nil {
		return nil, err
	}
	input, _ := request["input"].([]any)
	if err := xaiValidateSearchToolTrees(explicit, input); err != nil {
		return nil, err
	}
	// Responses Lite adds explicit declarations through input rather than tools.
	// Merge these before history and remove the input wrappers already consumed.
	var additions []any
	remainingInput := make([]any, 0, len(input))
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		if xaiSearchString(item, "type") == "additional_tools" {
			if children, ok := item["tools"].([]any); ok {
				additions = append(additions, children...)
				continue
			}
		}
		remainingInput = append(remainingInput, raw)
	}
	if len(additions) > 0 {
		explicit = xaiMergeExplicitSearchTools(explicit, additions)
	}
	pendingExplicit, err := json.Marshal(explicit)
	if err != nil {
		return nil, err
	}
	tools, err := xaiPromoteSearchDiscoveries(explicit, remainingInput, fallback)
	if err != nil {
		return nil, err
	}
	if err := xaiValidateSearchToolTrees(tools, nil); err != nil {
		return nil, err
	}
	merged, err := json.Marshal(tools)
	if err != nil {
		return nil, err
	}
	if len(merged) > xaiSearchMaxDeclarations {
		return nil, fmt.Errorf("tool_search declarations exceed session limit")
	}
	if len(remainingInput) != len(input) {
		body, err = sjson.SetBytes(body, "input", remainingInput)
		if err != nil {
			return nil, err
		}
	}
	s.mu.Lock()
	s.pendingExplicit = pendingExplicit
	s.mu.Unlock()
	return sjson.SetRawBytes(body, "tools", merged)
}

func (s *XAIClientToolSearchSession) Remember(accountScope string, mapping *XAIClientToolSearch) {
	if s == nil {
		return
	}
	var declarations, explicitDeclarations []byte
	if mapping != nil {
		declarations = bytes.Clone(mapping.declarations)
		explicitDeclarations = bytes.Clone(mapping.explicitDeclarations)
	}
	scope := sha256.Sum256([]byte(accountScope))
	s.mu.Lock()
	if !s.retainingExplicit || s.scope != scope || len(declarations) == 0 {
		s.explicitDeclarations = explicitDeclarations
	} else if len(s.pendingExplicit) > 0 {
		s.explicitDeclarations = s.pendingExplicit
	}
	s.retainingExplicit = false
	s.pendingExplicit = nil
	s.scope, s.declarations = scope, declarations
	s.mu.Unlock()
}

// RestoreHistory keeps replay transcripts in the client protocol so a later
// discovery that occupies an old alias can safely choose a different alias.
func (m *XAIClientToolSearch) RestoreHistory(body []byte) ([]byte, error) {
	if m == nil {
		return body, nil
	}
	searchCalls := make(map[string]bool)
	for index, item := range gjson.GetBytes(body, "input").Array() {
		if item.Get("type").String() != "function_call" || item.Get("name").String() != m.Alias {
			continue
		}
		searchCalls[item.Get("call_id").String()] = true
		var err error
		body, err = m.restoreItem(body, fmt.Sprintf("input.%d", index), item.Get("arguments").String())
		if err != nil {
			return nil, err
		}
	}
	for index, item := range gjson.GetBytes(body, "input").Array() {
		if item.Get("type").String() == "function_call_output" && searchCalls[item.Get("call_id").String()] {
			body, _ = sjson.SetBytes(body, fmt.Sprintf("input.%d.type", index), "tool_search_output")
			body, _ = sjson.SetBytes(body, fmt.Sprintf("input.%d.execution", index), "client")
		}
	}
	return body, nil
}

// New explicit declarations replace the same identity while untouched explicit
// tools remain active. A different identity sharing a wire name is left for the
// ordinary conflict check to reject.
func xaiMergeExplicitSearchTools(current, additions []any) []any {
	updates := make(map[string]string)
	var collect func([]any, string)
	collect = func(tools []any, namespace string) {
		for _, raw := range tools {
			tool, _ := raw.(map[string]any)
			typ, name := xaiSearchString(tool, "type"), xaiSearchString(tool, "name")
			if typ == "namespace" {
				children, _ := tool["tools"].([]any)
				collect(children, name)
				continue
			}
			key := xaiSearchQualifiedName(namespace, name)
			identity := typ + "\x00" + namespace + "\x00" + name
			if name == "" {
				key = typ + "\x00" + xaiSearchString(tool, "execution")
				identity = key
			}
			updates[key] = identity
		}
	}
	collect(additions, "")
	var filter func([]any, string) []any
	filter = func(tools []any, namespace string) []any {
		kept := make([]any, 0, len(tools))
		for _, raw := range tools {
			tool, _ := raw.(map[string]any)
			typ, name := xaiSearchString(tool, "type"), xaiSearchString(tool, "name")
			if typ == "namespace" {
				children, _ := tool["tools"].([]any)
				children = filter(children, name)
				if len(children) == 0 {
					continue
				}
				copy := make(map[string]any, len(tool))
				for key, value := range tool {
					copy[key] = value
				}
				copy["tools"] = children
				kept = append(kept, copy)
				continue
			}
			key := xaiSearchQualifiedName(namespace, name)
			identity := typ + "\x00" + namespace + "\x00" + name
			if name == "" {
				key = typ + "\x00" + xaiSearchString(tool, "execution")
				identity = key
			}
			if replacement, ok := updates[key]; !ok || replacement != identity {
				kept = append(kept, raw)
			}
		}
		return kept
	}
	merged := filter(current, "")
	for _, raw := range additions {
		tool, _ := raw.(map[string]any)
		if xaiSearchString(tool, "type") != "namespace" {
			merged = append(merged, raw)
			continue
		}
		name := xaiSearchString(tool, "name")
		found := false
		for index, previousRaw := range merged {
			previous, _ := previousRaw.(map[string]any)
			if xaiSearchString(previous, "type") != "namespace" || xaiSearchString(previous, "name") != name {
				continue
			}
			copy := make(map[string]any, len(tool))
			for key, value := range previous {
				copy[key] = value
			}
			for key, value := range tool {
				copy[key] = value
			}
			oldChildren, _ := previous["tools"].([]any)
			newChildren, _ := tool["tools"].([]any)
			copy["tools"] = append(oldChildren, newChildren...)
			merged[index] = copy
			found = true
			break
		}
		if !found {
			merged = append(merged, raw)
		}
	}
	return merged
}
