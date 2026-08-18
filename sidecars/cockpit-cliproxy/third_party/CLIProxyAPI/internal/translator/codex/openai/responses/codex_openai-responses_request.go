package responses

import (
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	log "github.com/sirupsen/logrus"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

func ConvertOpenAIResponsesRequestToCodex(modelName string, inputRawJSON []byte, _ bool) []byte {
	rawJSON := inputRawJSON

	inputResult := gjson.GetBytes(rawJSON, "input")
	if inputResult.Type == gjson.String {
		input, _ := sjson.SetBytes([]byte(`[{"type":"message","role":"user","content":[{"type":"input_text","text":""}]}]`), "0.content.0.text", inputResult.String())
		rawJSON, _ = sjson.SetRawBytes(rawJSON, "input", input)
	}

	rawJSON, _ = sjson.SetBytes(rawJSON, "stream", true)
	rawJSON, _ = sjson.SetBytes(rawJSON, "store", false)
	parallelToolCalls := true
	if requested := gjson.GetBytes(rawJSON, "parallel_tool_calls"); requested.Exists() {
		parallelToolCalls = requested.Bool()
	}
	if registry.CodexClientModelUsesResponsesLite(modelName) {
		parallelToolCalls = false
	}
	rawJSON, _ = sjson.SetBytes(rawJSON, "parallel_tool_calls", parallelToolCalls)
	rawJSON, _ = sjson.SetBytes(rawJSON, "include", []string{"reasoning.encrypted_content"})
	// Codex Responses rejects token limit fields, so strip them out before forwarding.
	rawJSON, _ = sjson.DeleteBytes(rawJSON, "max_output_tokens")
	rawJSON, _ = sjson.DeleteBytes(rawJSON, "max_completion_tokens")
	rawJSON, _ = sjson.DeleteBytes(rawJSON, "temperature")
	rawJSON, _ = sjson.DeleteBytes(rawJSON, "top_p")
	if v := gjson.GetBytes(rawJSON, "service_tier"); v.Exists() {
		if v.String() != "priority" {
			rawJSON, _ = sjson.DeleteBytes(rawJSON, "service_tier")
		}
	}

	rawJSON, _ = sjson.DeleteBytes(rawJSON, "truncation")
	rawJSON = applyResponsesCompactionCompatibility(rawJSON)

	// Delete the user field as it is not supported by the Codex upstream.
	rawJSON, _ = sjson.DeleteBytes(rawJSON, "user")

	// Normalize replayed input metadata and roles in one pass.
	rawJSON = normalizeCodexInputItems(rawJSON)
	rawJSON = normalizeCodexBuiltinTools(rawJSON)

	return rawJSON
}

// normalizeCodexInputItems strips unsupported metadata while preserving tool-call
// namespaces needed by Codex OAuth, and converts unsupported system roles.
func normalizeCodexInputItems(rawJSON []byte) []byte {
	inputResult := gjson.GetBytes(rawJSON, "input")
	if !inputResult.IsArray() {
		return rawJSON
	}

	inputItems := inputResult.Array()
	hasChanges := false
	for _, item := range inputItems {
		if shouldStripCodexInputNamespace(item) || item.Get("role").String() == "system" {
			hasChanges = true
			break
		}
	}
	if !hasChanges {
		return rawJSON
	}

	normalizedInput := make([]byte, 0, len(inputResult.Raw)+16)
	normalizedInput = append(normalizedInput, '[')
	for i, item := range inputItems {
		if i > 0 {
			normalizedInput = append(normalizedInput, ',')
		}

		hasNamespace := shouldStripCodexInputNamespace(item)
		hasSystemRole := item.Get("role").String() == "system"
		if !hasNamespace && !hasSystemRole {
			normalizedInput = append(normalizedInput, item.Raw...)
			continue
		}

		itemJSON := []byte(item.Raw)
		if hasNamespace {
			if updated, err := sjson.DeleteBytes(itemJSON, "namespace"); err == nil {
				itemJSON = updated
			}
		}
		if hasSystemRole {
			if updated, err := sjson.SetBytes(itemJSON, "role", "developer"); err == nil {
				itemJSON = updated
			}
		}
		normalizedInput = append(normalizedInput, itemJSON...)
	}
	normalizedInput = append(normalizedInput, ']')

	updated, err := sjson.SetRawBytes(rawJSON, "input", normalizedInput)
	if err != nil {
		return rawJSON
	}
	return updated
}

func shouldStripCodexInputNamespace(item gjson.Result) bool {
	if !item.Get("namespace").Exists() {
		return false
	}
	switch strings.TrimSpace(item.Get("type").String()) {
	case "function_call", "custom_tool_call", "tool_call", "mcp_tool_call":
		return false
	default:
		return true
	}
}

// applyResponsesCompactionCompatibility handles OpenAI Responses context_management.compaction
// for Codex upstream compatibility.
//
// Codex /responses currently rejects context_management with:
// {"detail":"Unsupported parameter: context_management"}.
//
// Compatibility strategy:
// 1) Remove context_management before forwarding to Codex upstream.
func applyResponsesCompactionCompatibility(rawJSON []byte) []byte {
	if !gjson.GetBytes(rawJSON, "context_management").Exists() {
		return rawJSON
	}

	rawJSON, _ = sjson.DeleteBytes(rawJSON, "context_management")
	return rawJSON
}

// normalizeCodexBuiltinTools rewrites legacy/preview built-in tool variants to the
// stable names expected by the current Codex upstream.
func normalizeCodexBuiltinTools(rawJSON []byte) []byte {
	result := normalizeCodexBuiltinToolArray(rawJSON, "tools")

	toolChoiceType := gjson.GetBytes(result, "tool_choice.type").String()
	result = normalizeCodexBuiltinToolAtPath(result, "tool_choice.type", toolChoiceType)
	result = normalizeCodexBuiltinToolArray(result, "tool_choice.tools")

	return result
}

func normalizeCodexBuiltinToolArray(rawJSON []byte, path string) []byte {
	toolResult := gjson.GetBytes(rawJSON, path)
	if !toolResult.IsArray() {
		return rawJSON
	}

	tools := toolResult.Array()
	hasChanges := false
	for _, tool := range tools {
		if normalizeCodexBuiltinToolType(tool.Get("type").String()) != "" {
			hasChanges = true
			break
		}
	}
	if !hasChanges {
		return rawJSON
	}

	normalizedTools := make([]byte, 0, len(toolResult.Raw))
	normalizedTools = append(normalizedTools, '[')
	for i, tool := range tools {
		if i > 0 {
			normalizedTools = append(normalizedTools, ',')
		}

		currentType := tool.Get("type").String()
		normalizedType := normalizeCodexBuiltinToolType(currentType)
		if normalizedType == "" {
			normalizedTools = append(normalizedTools, tool.Raw...)
			continue
		}

		toolJSON := []byte(tool.Raw)
		if updated, err := sjson.SetBytes(toolJSON, "type", normalizedType); err == nil {
			toolJSON = updated
			log.Debugf("codex responses: normalized builtin tool type at %s.%d.type from %q to %q", path, i, currentType, normalizedType)
		}
		normalizedTools = append(normalizedTools, toolJSON...)
	}
	normalizedTools = append(normalizedTools, ']')

	updated, err := sjson.SetRawBytes(rawJSON, path, normalizedTools)
	if err != nil {
		return rawJSON
	}
	return updated
}

func normalizeCodexBuiltinToolAtPath(rawJSON []byte, path, currentType string) []byte {
	normalizedType := normalizeCodexBuiltinToolType(currentType)
	if normalizedType == "" {
		return rawJSON
	}

	updated, err := sjson.SetBytes(rawJSON, path, normalizedType)
	if err != nil {
		return rawJSON
	}

	log.Debugf("codex responses: normalized builtin tool type at %s from %q to %q", path, currentType, normalizedType)
	return updated
}

// normalizeCodexBuiltinToolType centralizes the current known Codex Responses
// built-in tool alias compatibility. If Codex introduces more legacy aliases,
// extend this helper instead of adding path-specific rewrite logic elsewhere.
func normalizeCodexBuiltinToolType(toolType string) string {
	switch toolType {
	case "web_search_preview", "web_search_preview_2025_03_11":
		return "web_search"
	default:
		return ""
	}
}
