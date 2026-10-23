package helps

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
)

const xaiToolSearchAlias = "__codex_tool_search"
const xaiToolSearchParameters = `{"type":"object","properties":{"query":{"type":"string","description":"Search query for tools or connectors to load."},"limit":{"type":"integer","description":"Maximum number of tool groups to return."}},"required":["query"]}`

// XAIClientToolSearch is request-local: tool identities never cross accounts or
// sessions. Explicit server-side searches are outside this adapter's scope.
type XAIClientToolSearch struct {
	Alias                string
	declarations         []byte
	explicitDeclarations []byte
}

// AdaptXAIClientToolSearch lowers client searches before namespace expansion.
// Missing execution is the legacy Codex client declaration; explicit server
// declarations and their history retain native upstream semantics.
func AdaptXAIClientToolSearch(body []byte) ([]byte, *XAIClientToolSearch, error) {
	var req map[string]any
	if err := xaiSearchDecode(body, &req); err != nil {
		return body, nil, err
	}
	tools, _ := req["tools"].([]any)
	input, _ := req["input"].([]any)
	if err := xaiValidateSearchToolTrees(tools, input); err != nil {
		return body, nil, err
	}
	var declaration map[string]any
	var findDeclaration func([]any)
	findDeclaration = func(items []any) {
		for _, raw := range items {
			tool, _ := raw.(map[string]any)
			if xaiSearchString(tool, "type") == "tool_search" && xaiSearchClient(tool) {
				if declaration == nil {
					declaration = tool
				}
			}
			if children, ok := tool["tools"].([]any); ok {
				findDeclaration(children)
			}
		}
	}
	findDeclaration(tools)
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		if xaiSearchString(item, "type") == "additional_tools" {
			children, _ := item["tools"].([]any)
			findDeclaration(children)
		}
	}
	declared := declaration != nil
	if declaration == nil {
		serverCalls := make(map[string]bool)
		for _, raw := range input {
			item, _ := raw.(map[string]any)
			if xaiSearchString(item, "type") == "tool_search_call" && xaiSearchString(item, "execution") == "server" {
				serverCalls[xaiSearchString(item, "call_id")] = true
			}
		}
		for _, raw := range input {
			item, _ := raw.(map[string]any)
			if typ := xaiSearchString(item, "type"); (typ == "tool_search_call" || typ == "tool_search_output") && xaiSearchClient(item) && !serverCalls[xaiSearchString(item, "call_id")] && (!xaiHasNativeToolSearch(tools) || xaiSearchString(item, "execution") == "client") {
				declaration = map[string]any{"type": "tool_search"}
				break
			}
		}
	}
	if declaration == nil {
		return body, nil, nil
	}

	var err error
	var explicitDeclarations []byte
	if declared {
		explicitDeclarations, err = json.Marshal(tools)
		if err != nil {
			return body, nil, err
		}
		tools, err = xaiPromoteSearchDiscoveries(tools, input)
		if err != nil {
			return body, nil, err
		}
	}
	if err := xaiValidateSearchToolTrees(tools, nil); err != nil {
		return body, nil, err
	}
	occupied := make(map[string]bool)
	var collectNames func([]any, string)
	collectNames = func(items []any, namespace string) {
		for _, raw := range items {
			tool, _ := raw.(map[string]any)
			name := xaiSearchString(tool, "name")
			if name != "" {
				occupied[name] = true
				occupied[xaiSearchQualifiedName(namespace, name)] = true
			}
			if children, ok := tool["tools"].([]any); ok {
				collectNames(children, name)
			}
		}
	}
	collectNames(tools, "")
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		if xaiSearchString(item, "type") == "additional_tools" {
			children, _ := item["tools"].([]any)
			collectNames(children, "")
		}
	}
	alias := xaiToolSearchAlias
	for suffix := 1; occupied[alias]; suffix++ {
		alias = fmt.Sprintf("%s_%d", xaiToolSearchAlias, suffix)
	}
	mapping := &XAIClientToolSearch{Alias: alias, explicitDeclarations: explicitDeclarations}
	if declared {
		mapping.declarations, err = json.Marshal(tools)
		if err != nil {
			return body, nil, err
		}
		if len(mapping.declarations) > xaiSearchMaxDeclarations {
			return body, nil, fmt.Errorf("tool_search declarations exceed session limit")
		}
	}
	var removeSearch func([]any) []any
	removeSearch = func(items []any) []any {
		filtered := make([]any, 0, len(items))
		for _, raw := range items {
			tool, _ := raw.(map[string]any)
			if xaiSearchString(tool, "type") == "tool_search" && xaiSearchClient(tool) {
				continue
			}
			if children, ok := tool["tools"].([]any); ok {
				tool["tools"] = removeSearch(children)
			}
			filtered = append(filtered, raw)
		}
		return filtered
	}
	tools = removeSearch(tools)
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		if xaiSearchString(item, "type") == "additional_tools" {
			children, _ := item["tools"].([]any)
			item["tools"] = removeSearch(children)
		}
	}
	parameters := declaration["parameters"]
	if _, ok := parameters.(map[string]any); !ok {
		_ = xaiSearchDecode([]byte(xaiToolSearchParameters), &parameters)
	}
	description, _ := declaration["description"].(string)
	if strings.TrimSpace(description) == "" {
		description = "Search and load tools, plugins, connectors, and MCP namespaces for the current task."
	}
	proxy := map[string]any{"type": "function", "name": alias, "description": description, "parameters": parameters}
	// Keep discovery callable even when the upstream tool cap trims later tools.
	if declared {
		req["tools"] = append([]any{proxy}, tools...)
	}
	if err := mapping.rewriteInput(input); err != nil {
		return body, nil, err
	}
	mapping.rewriteChoice(req["tool_choice"])
	if !xaiHasNativeToolSearch(req["tools"]) {
		xaiStripSearchDeferredFlags(req["tools"])
	}
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		if xaiSearchString(item, "type") == "additional_tools" && !xaiHasNativeToolSearch(req["tools"]) {
			xaiStripSearchDeferredFlags(item["tools"])
		}
	}
	out, err := json.Marshal(req)
	return out, mapping, err
}

func xaiSearchClient(item map[string]any) bool {
	execution := xaiSearchString(item, "execution")
	return execution == "" || execution == "client"
}

func xaiSearchString(item map[string]any, key string) string {
	value, _ := item[key].(string)
	return strings.TrimSpace(value)
}

func xaiSearchDecode(data []byte, value any) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	return decoder.Decode(value)
}

func xaiStripSearchDeferredFlags(raw any) {
	items, _ := raw.([]any)
	for _, value := range items {
		tool, _ := value.(map[string]any)
		delete(tool, "defer_loading")
		xaiStripSearchDeferredFlags(tool["tools"])
	}
}

func (m *XAIClientToolSearch) rewriteChoice(raw any) {
	choice, _ := raw.(map[string]any)
	if xaiSearchString(choice, "type") == "tool_search" && xaiSearchClient(choice) {
		for key := range choice {
			delete(choice, key)
		}
		choice["type"], choice["name"] = "function", m.Alias
	}
	if children, ok := choice["tools"].([]any); ok {
		for _, child := range children {
			m.rewriteChoice(child)
		}
	}
}

// RewriteHistory also handles transcript replay inserted after preparation.
func (m *XAIClientToolSearch) RewriteHistory(body []byte) ([]byte, error) {
	if m == nil {
		return body, nil
	}
	var req map[string]any
	if err := xaiSearchDecode(body, &req); err != nil {
		return body, err
	}
	input, _ := req["input"].([]any)
	if err := m.rewriteInput(input); err != nil {
		return body, err
	}
	return json.Marshal(req)
}

func (m *XAIClientToolSearch) rewriteInput(input []any) error {
	serverCalls := make(map[string]bool)
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		if xaiSearchString(item, "type") == "tool_search_call" && !xaiSearchClient(item) {
			serverCalls[xaiSearchString(item, "call_id")] = true
		}
	}
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		typ := xaiSearchString(item, "type")
		if !xaiSearchClient(item) || serverCalls[xaiSearchString(item, "call_id")] {
			continue
		}
		switch typ {
		case "tool_search_call":
			if xaiSearchString(item, "call_id") == "" {
				return fmt.Errorf("tool_search_call requires a non-empty call_id")
			}
			args := item["arguments"]
			if text, ok := args.(string); ok {
				if err := xaiSearchDecode([]byte(text), &args); err != nil {
					return fmt.Errorf("tool_search_call arguments must be a JSON object")
				}
			}
			if args == nil {
				args = map[string]any{}
			}
			if _, ok := args.(map[string]any); !ok {
				return fmt.Errorf("tool_search_call arguments must be a JSON object")
			}
			encoded, err := json.Marshal(args)
			if err != nil {
				return err
			}
			item["type"], item["name"], item["arguments"] = "function_call", m.Alias, string(encoded)
			delete(item, "execution")
			delete(item, "namespace")
			xaiSearchLowerItemID(item)
		case "tool_search_output":
			if xaiSearchString(item, "call_id") == "" {
				return fmt.Errorf("tool_search_output requires a non-empty call_id")
			}
			output, exists := item["output"]
			if !exists {
				output, exists = item["tools"]
			}
			if !exists {
				return fmt.Errorf("tool_search_output requires output or tools")
			}
			if output == nil {
				output = ""
			}
			if _, ok := output.(string); !ok {
				encoded, err := json.Marshal(output)
				if err != nil {
					return err
				}
				output = string(encoded)
			}
			item["type"], item["output"] = "function_call_output", output
			for _, key := range []string{"tools", "status", "execution"} {
				delete(item, key)
			}
			xaiSearchLowerItemID(item)
		}
	}
	return nil
}

func xaiSearchLowerItemID(item map[string]any) {
	id := xaiSearchString(item, "id")
	if strings.HasPrefix(id, "tsc_") {
		item["id"] = "fc_" + strings.TrimPrefix(id, "tsc_")
	} else if id != "" && !strings.HasPrefix(id, "fc_") {
		delete(item, "id")
	}
}

func xaiSearchQualifiedName(namespace, name string) string {
	if namespace == "" || strings.HasPrefix(name, "mcp__") {
		return name
	}
	prefix := namespace
	if !strings.HasSuffix(prefix, "__") {
		prefix += "__"
	}
	if strings.HasPrefix(name, prefix) {
		return name
	}
	return prefix + name
}

func xaiHasNativeToolSearch(raw any) bool {
	items, _ := raw.([]any)
	for _, value := range items {
		tool, _ := value.(map[string]any)
		if xaiSearchString(tool, "type") == "tool_search" && !xaiSearchClient(tool) {
			return true
		}
		if xaiHasNativeToolSearch(tool["tools"]) {
			return true
		}
	}
	return false
}
