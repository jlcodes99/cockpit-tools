package helps

import "fmt"

// Discovery merges run on client-owned protocol fields only. Current explicit
// declarations take precedence over history; among history, the newest usable
// output wins. Only distinct identities sharing a wire name are rejected.
func xaiPromoteSearchDiscoveries(tools, input []any, fallbackTools ...[]any) ([]any, error) {
	type toolIdentity struct{ kind, namespace, name string }
	known := make(map[string]toolIdentity)
	namespaces := make(map[string]map[string]any)
	register := func(tool map[string]any, namespace string) error {
		name := xaiSearchString(tool, "name")
		if name == "" {
			return nil
		}
		key := xaiSearchQualifiedName(namespace, name)
		identity := toolIdentity{kind: xaiSearchString(tool, "type"), namespace: namespace, name: name}
		if previous, exists := known[key]; exists && previous != identity {
			return fmt.Errorf("discovered tool %q conflicts with a different tool identity", key)
		}
		known[key] = identity
		return nil
	}
	for _, raw := range tools {
		tool, _ := raw.(map[string]any)
		if xaiSearchString(tool, "type") == "namespace" {
			namespace := xaiSearchString(tool, "name")
			namespaces[namespace] = tool
			children, _ := tool["tools"].([]any)
			for _, rawChild := range children {
				child, _ := rawChild.(map[string]any)
				if err := register(child, namespace); err != nil {
					return nil, err
				}
			}
		} else if err := register(tool, ""); err != nil {
			return nil, err
		}
	}
	admit := func(tool map[string]any, namespace string) (bool, error) {
		typ := xaiSearchString(tool, "type")
		if (typ != "function" && typ != "custom") || xaiSearchString(tool, "name") == "" {
			return false, nil
		}
		key := xaiSearchQualifiedName(namespace, xaiSearchString(tool, "name"))
		_, exists := known[key]
		if err := register(tool, namespace); err != nil {
			return false, err
		}
		return !exists, nil
	}
	mergeDiscoveries := func(discoveries []any) error {
		for _, rawDiscovery := range discoveries {
			discovery, _ := rawDiscovery.(map[string]any)
			if xaiSearchString(discovery, "type") != "namespace" {
				added, err := admit(discovery, "")
				if err != nil {
					return err
				}
				if added {
					tools = append(tools, discovery)
				}
				continue
			}
			name := xaiSearchString(discovery, "name")
			if name == "" {
				continue
			}
			children, _ := discovery["tools"].([]any)
			if children == nil {
				children, _ = discovery["children"].([]any)
			}
			var admitted []any
			for _, rawChild := range children {
				child, _ := rawChild.(map[string]any)
				added, err := admit(child, name)
				if err != nil {
					return err
				}
				if added {
					admitted = append(admitted, child)
				}
			}
			if len(admitted) == 0 {
				continue
			}
			if existing := namespaces[name]; existing != nil {
				previous, _ := existing["tools"].([]any)
				existing["tools"] = append(previous, admitted...)
			} else {
				discovery["tools"] = admitted
				delete(discovery, "children")
				namespaces[name] = discovery
				tools = append(tools, discovery)
			}
		}
		return nil
	}
	serverCalls := make(map[string]bool)
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		if xaiSearchString(item, "type") == "tool_search_call" && !xaiSearchClient(item) {
			serverCalls[xaiSearchString(item, "call_id")] = true
		}
	}
	for index := len(input) - 1; index >= 0; index-- {
		item, _ := input[index].(map[string]any)
		if xaiSearchString(item, "type") != "tool_search_output" || !xaiSearchClient(item) || serverCalls[xaiSearchString(item, "call_id")] {
			continue
		}
		if status := xaiSearchString(item, "status"); status != "" && status != "completed" {
			continue
		}
		discoveries, _ := item["tools"].([]any)
		// Legacy clients place definitions under output, either as JSON or as an object.
		if discoveries == nil {
			output := item["output"]
			if encoded, ok := output.(string); ok {
				_ = xaiSearchDecode([]byte(encoded), &output)
			}
			switch value := output.(type) {
			case []any:
				discoveries = value
			case map[string]any:
				discoveries, _ = value["tools"].([]any)
			}
		}
		if err := mergeDiscoveries(discoveries); err != nil {
			return nil, err
		}
	}
	for _, fallback := range fallbackTools {
		if err := mergeDiscoveries(fallback); err != nil {
			return nil, err
		}
	}
	return tools, nil
}
