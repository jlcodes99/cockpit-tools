package helps

import "fmt"

const xaiSearchMaxToolDepth = 32
const xaiSearchMaxToolNodes = 10000

// Validate protocol-owned tool trees before recursive declaration rewrites.
// Schemas and ordinary tool result payloads are not traversed as tool trees.
func xaiValidateSearchToolTrees(tools, input []any) error {
	type level struct {
		tools []any
		depth int
	}
	pending := []level{{tools: tools, depth: 1}}
	for _, raw := range input {
		item, _ := raw.(map[string]any)
		switch xaiSearchString(item, "type") {
		case "additional_tools", "tool_search_output":
			if items, ok := item["tools"].([]any); ok {
				pending = append(pending, level{tools: items, depth: 1})
			}
		}
	}
	count := 0
	for len(pending) > 0 {
		current := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		if len(current.tools) > 0 && current.depth > xaiSearchMaxToolDepth {
			return fmt.Errorf("tool_search tool nesting exceeds %d levels", xaiSearchMaxToolDepth)
		}
		count += len(current.tools)
		if count > xaiSearchMaxToolNodes {
			return fmt.Errorf("tool_search tool declarations exceed %d nodes", xaiSearchMaxToolNodes)
		}
		for _, raw := range current.tools {
			tool, _ := raw.(map[string]any)
			for _, key := range []string{"tools", "children"} {
				if children, ok := tool[key].([]any); ok {
					pending = append(pending, level{tools: children, depth: current.depth + 1})
				}
			}
		}
	}
	return nil
}
