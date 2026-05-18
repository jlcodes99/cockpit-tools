package converters

import (
	"encoding/json"
	"strings"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
)

// CodexCustomToolKind classifies the type of Codex custom tool.
type CodexCustomToolKind string

const (
	CodexCustomToolRaw        CodexCustomToolKind = "raw"
	CodexCustomToolApplyPatch CodexCustomToolKind = "apply_patch"
	CodexCustomToolExec       CodexCustomToolKind = "exec"
	CodexCustomToolBuiltIn    CodexCustomToolKind = "builtin"
)

// CodexCustomToolSpec describes a single Codex custom tool and its upstream proxy.
type CodexCustomToolSpec struct {
	OpenAIName        string
	GrammarDefinition string
	Kind              CodexCustomToolKind
	ProxyAction       string // "", add_file, delete_file, update_file, replace_file, batch
}

// CodexFunctionToolSpec describes a normal function tool for namespace tracking.
type CodexFunctionToolSpec struct {
	Namespace string
	Name      string
}

// CodexToolContext holds parsed information about all tools in a request.
type CodexToolContext struct {
	CustomTools       map[string]CodexCustomToolSpec
	FunctionTools     map[string]CodexFunctionToolSpec
	HasCustomTools    bool
	HasNamespaceTools bool
}

// BuildCodexToolContext inspects a Responses request's tools array and builds
// the tool context needed for request/response conversion.
func BuildCodexToolContext(tools []map[string]interface{}) CodexToolContext {
	rawTools := make([]interface{}, 0, len(tools))
	for _, tool := range tools {
		rawTools = append(rawTools, tool)
	}
	return BuildCodexToolContextFromRaw(rawTools)
}

func BuildCodexToolContextFromRaw(tools []interface{}) CodexToolContext {
	ctx := CodexToolContext{
		CustomTools:   make(map[string]CodexCustomToolSpec),
		FunctionTools: make(map[string]CodexFunctionToolSpec),
	}

	for _, rawTool := range tools {
		if name, ok := rawTool.(string); ok && name != "" {
			if action := proxyActionFromUpstreamName(name); strings.HasPrefix(name, "apply_patch_") && action != "" {
				ctx.CustomTools[name] = CodexCustomToolSpec{OpenAIName: "apply_patch", Kind: CodexCustomToolApplyPatch, ProxyAction: action}
			} else {
				ctx.CustomTools[name] = CodexCustomToolSpec{OpenAIName: name, Kind: CodexCustomToolRaw}
			}
			ctx.HasCustomTools = true
			continue
		}
		tool, ok := rawTool.(map[string]interface{})
		if !ok {
			continue
		}
		toolType, _ := tool["type"].(string)
		switch toolType {
		case "custom":
			name, _ := tool["name"].(string)
			if name == "" {
				continue
			}
			kind, grammarDef := detectCodexCustomToolKind(tool)
			spec := CodexCustomToolSpec{
				OpenAIName:        name,
				GrammarDefinition: grammarDef,
				Kind:              kind,
			}
			switch kind {
			case CodexCustomToolApplyPatch:
				ctx.CustomTools[name] = spec
				for _, suffix := range []string{"_add_file", "_delete_file", "_update_file", "_replace_file", "_batch"} {
					proxySpec := spec
					proxySpec.ProxyAction = strings.TrimPrefix(suffix, "_")
					ctx.CustomTools[name+suffix] = proxySpec
				}
			default:
				ctx.CustomTools[name] = spec
			}
			ctx.HasCustomTools = true
		case "function":
			name, _ := tool["name"].(string)
			if name == "" {
				continue
			}
			ctx.FunctionTools[name] = CodexFunctionToolSpec{Name: name}
		case "namespace":
			addNamespaceToolsToContext(&ctx, tool)
		case "web_search", "local_shell", "computer_use":
			name, _ := tool["name"].(string)
			if name == "" {
				name = toolType
			}
			ctx.CustomTools[name] = CodexCustomToolSpec{OpenAIName: name, Kind: CodexCustomToolBuiltIn}
			ctx.HasCustomTools = true
		}
	}

	return ctx
}

// flattenNamespaceToolName returns the flat function name for a namespace tool child.
// Expects namespace names to end with "__" per Codex convention.
func flattenNamespaceToolName(namespace, name string) string {
	if namespace == "" {
		return name
	}
	if name == "" {
		return namespace
	}
	if strings.HasSuffix(namespace, "__") || strings.HasPrefix(name, "__") {
		return namespace + name
	}
	return namespace + "__" + name
}

func addNamespaceToolsToContext(ctx *CodexToolContext, namespaceTool map[string]interface{}) {
	namespace, _ := namespaceTool["name"].(string)
	// Silently yields nil when the key is absent; for range nil is safe in Go.
	children, _ := namespaceTool["tools"].([]interface{})
	for _, raw := range children {
		child, ok := raw.(map[string]interface{})
		if !ok {
			continue
		}
		childType, _ := child["type"].(string)
		switch childType {
		case "function":
			name, _ := child["name"].(string)
			if name == "" {
				continue
			}
			flat := flattenNamespaceToolName(namespace, name)
			if spec, exists := ctx.FunctionTools[flat]; exists && spec.Namespace == "" {
				continue
			}
			ctx.FunctionTools[flat] = CodexFunctionToolSpec{
				Namespace: namespace,
				Name:      name,
			}
			ctx.HasNamespaceTools = true
			// case "custom", "namespace": not implemented in this pass
		}
	}
}

// OpenAINameForFunctionTool returns the unflattened (name, namespace) for an upstream flat function name.
func (ctx CodexToolContext) OpenAINameForFunctionTool(upstreamName string) (name string, namespace string) {
	spec, ok := ctx.FunctionTools[upstreamName]
	if !ok {
		return upstreamName, ""
	}
	if spec.Name == "" {
		return upstreamName, spec.Namespace
	}
	return spec.Name, spec.Namespace
}

func detectCodexCustomToolKind(tool map[string]interface{}) (CodexCustomToolKind, string) {
	name, _ := tool["name"].(string)
	format, _ := tool["format"].(map[string]interface{})
	grammarDef := ""
	if format != nil {
		grammarDef, _ = format["definition"].(string)
	}
	if name == "apply_patch" {
		return CodexCustomToolApplyPatch, grammarDef
	}
	if grammarDef != "" {
		if strings.Contains(grammarDef, "begin_patch") &&
			strings.Contains(grammarDef, "end_patch") &&
			strings.Contains(grammarDef, "add_hunk") {
			return CodexCustomToolApplyPatch, grammarDef
		}
	}
	if name == "exec" {
		return CodexCustomToolExec, grammarDef
	}
	return CodexCustomToolRaw, grammarDef
}

func (ctx *CodexToolContext) IsCustomToolProxy(upstreamName string) bool {
	_, ok := ctx.CustomTools[upstreamName]
	return ok
}

func (ctx *CodexToolContext) OriginalCustomToolName(upstreamName string) string {
	if spec, ok := ctx.CustomTools[upstreamName]; ok {
		return spec.OpenAIName
	}
	return upstreamName
}

// ============== Request-Side Tool Conversion ==============

func responsesToolsToOpenAIWithContext(tools []map[string]interface{}, ctx CodexToolContext) []map[string]interface{} {
	rawTools := make([]interface{}, 0, len(tools))
	for _, tool := range tools {
		rawTools = append(rawTools, tool)
	}
	return responsesRawToolsToOpenAIWithContext(rawTools, ctx)
}

func responsesRawToolsToOpenAI(tools []interface{}) []map[string]interface{} {
	mappedTools := make([]map[string]interface{}, 0, len(tools))
	openaiTools := make([]map[string]interface{}, 0, len(tools))
	for _, raw := range tools {
		switch tool := raw.(type) {
		case string:
			if tool == "" {
				continue
			}
			openaiTools = append(openaiTools, genericCustomProxyTool(tool, ""))
		case map[string]interface{}:
			mappedTools = append(mappedTools, tool)
		}
	}
	openaiTools = append(openaiTools, responsesToolsToOpenAI(mappedTools)...)
	return openaiTools
}

// ConvertRawToolsToOpenAI converts Responses-format tools (including custom/namespace/web_search/local_shell/computer_use)
// into OpenAI function-call format. Used by the passthrough branch when codexToolCompat is enabled.
func ConvertRawToolsToOpenAI(tools []interface{}) []map[string]interface{} {
	ctx := BuildCodexToolContextFromRaw(tools)
	return responsesRawToolsToOpenAIWithContext(tools, ctx)
}

func responsesRawToolsToOpenAIWithContext(tools []interface{}, ctx CodexToolContext) []map[string]interface{} {
	if !ctx.HasCustomTools && !ctx.HasNamespaceTools {
		return responsesRawToolsToOpenAI(tools)
	}

	openaiTools := make([]map[string]interface{}, 0, len(tools)*2)
	seenApplyPatch := map[string]bool{}

	for _, rawTool := range tools {
		if name, ok := rawTool.(string); ok && name != "" {
			openaiTools = append(openaiTools, genericCustomProxyTool(name, ""))
			continue
		}
		tool, ok := rawTool.(map[string]interface{})
		if !ok {
			continue
		}
		toolType, _ := tool["type"].(string)
		switch toolType {
		case "function":
			name, description, parameters := extractResponsesToolFields(tool)
			if name == "" {
				continue
			}
			ot := map[string]interface{}{
				"type": "function",
				"function": map[string]interface{}{
					"name":       name,
					"parameters": parameters,
				},
			}
			if description != "" {
				ot["function"].(map[string]interface{})["description"] = description
			}
			openaiTools = append(openaiTools, ot)
		case "namespace":
			openaiTools = append(openaiTools, namespaceToolsToOpenAI(tool, ctx)...)
		case "custom", "web_search", "local_shell", "computer_use":
			name, _ := tool["name"].(string)
			if name == "" {
				name = toolType
			}
			spec, ok := ctx.CustomTools[name]
			if !ok {
				spec = CodexCustomToolSpec{OpenAIName: name, Kind: CodexCustomToolRaw}
			}
			description, _ := tool["description"].(string)
			switch spec.Kind {
			case CodexCustomToolApplyPatch:
				if seenApplyPatch[name] {
					continue
				}
				seenApplyPatch[name] = true
				openaiTools = append(openaiTools,
					applyPatchAddFileTool(name, description),
					applyPatchDeleteFileTool(name, description),
					applyPatchUpdateFileTool(name, description),
					applyPatchReplaceFileTool(name, description),
					applyPatchBatchTool(name, description),
				)
			default:
				openaiTools = append(openaiTools, genericCustomProxyTool(name, description))
			}
		}
	}

	return openaiTools
}

// namespaceToolsToOpenAI converts a namespace tool into OpenAI function tools.
// The ctx parameter is used to check for name collisions with top-level functions.
func namespaceToolsToOpenAI(namespaceTool map[string]interface{}, ctx CodexToolContext) []map[string]interface{} {
	namespace, _ := namespaceTool["name"].(string)
	namespaceDesc, _ := namespaceTool["description"].(string)
	children, _ := namespaceTool["tools"].([]interface{})

	out := make([]map[string]interface{}, 0, len(children))
	for _, raw := range children {
		child, ok := raw.(map[string]interface{})
		if !ok {
			continue
		}
		if childType, _ := child["type"].(string); childType != "function" {
			continue
		}
		name, description, parameters := extractResponsesToolFields(child)
		if name == "" {
			continue
		}
		flat := flattenNamespaceToolName(namespace, name)
		// Skip generating an OpenAI tool when the flat name is already occupied by a top-level function.
		// Only applies when namespace is non-empty, otherwise the FunctionTools entry may be from this same namespace tool.
		if namespace != "" {
			if spec, exists := ctx.FunctionTools[flat]; exists && spec.Namespace == "" {
				continue
			}
		}
		combinedDescription := combineNamespaceDescription(namespaceDesc, description)
		function := map[string]interface{}{
			"name":       flat,
			"parameters": parameters,
		}
		if combinedDescription != "" {
			function["description"] = combinedDescription
		}
		out = append(out, map[string]interface{}{
			"type":     "function",
			"function": function,
		})
	}
	return out
}

func combineNamespaceDescription(namespaceDesc, childDesc string) string {
	namespaceDesc = strings.TrimSpace(namespaceDesc)
	childDesc = strings.TrimSpace(childDesc)
	switch {
	case namespaceDesc == "":
		return childDesc
	case childDesc == "":
		return namespaceDesc
	default:
		return namespaceDesc + "\n\n" + childDesc
	}
}

func genericCustomProxyTool(name, description string) map[string]interface{} {
	desc := description
	if desc == "" {
		desc = "FREEFORM custom tool: " + name + ". Put only the tool input text here."
	} else {
		desc = description + "\n\nThis is a FREEFORM tool. Do not wrap the input in JSON or markdown."
	}
	return map[string]interface{}{
		"type": "function",
		"function": map[string]interface{}{
			"name":        name,
			"description": desc,
			"parameters": map[string]interface{}{
				"type":                 "object",
				"additionalProperties": false,
				"properties": map[string]interface{}{
					"input": map[string]interface{}{
						"type":        "string",
						"description": "Raw freeform input for this custom tool.",
					},
				},
				"required": []string{"input"},
			},
		},
	}
}

// ============== Apply Patch Proxy Tool Builders ==============

func applyPatchAddFileTool(name, description string) map[string]interface{} {
	return funcTool(name+"_add_file", patchProxyDesc(description, "add_file",
		"Create one new file by providing a target path and full file content."),
		applyPatchAddFileSchema())
}

func applyPatchDeleteFileTool(name, description string) map[string]interface{} {
	return funcTool(name+"_delete_file", patchProxyDesc(description, "delete_file",
		"Delete one file by providing a target path."),
		applyPatchDeleteFileSchema())
}

func applyPatchUpdateFileTool(name, description string) map[string]interface{} {
	return funcTool(name+"_update_file", patchProxyDesc(description, "update_file",
		"Edit one existing file with structured hunks."),
		applyPatchUpdateFileSchema())
}

func applyPatchReplaceFileTool(name, description string) map[string]interface{} {
	return funcTool(name+"_replace_file", patchProxyDesc(description, "replace_file",
		"Replace one existing file by providing a target path and full new file content."),
		applyPatchReplaceFileSchema())
}

func applyPatchBatchTool(name, description string) map[string]interface{} {
	return funcTool(name+"_batch", patchProxyDesc(description, "batch",
		"Edit files by providing structured JSON patch operations."),
		applyPatchBatchSchema())
}

func funcTool(name, description string, params map[string]interface{}) map[string]interface{} {
	return map[string]interface{}{
		"type": "function",
		"function": map[string]interface{}{
			"name":        name,
			"description": description,
			"parameters":  params,
		},
	}
}

func patchProxyDesc(description, action, defaultDesc string) string {
	if description != "" {
		return description + " (proxy action: " + action + ")"
	}
	return defaultDesc
}

func applyPatchAddFileSchema() map[string]interface{} {
	return map[string]interface{}{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]interface{}{
			"path":    map[string]interface{}{"type": "string", "description": "Target file path."},
			"content": map[string]interface{}{"type": "string", "description": "Full file content without patch '+' prefixes."},
		},
		"required": []string{"path", "content"},
	}
}

func applyPatchDeleteFileSchema() map[string]interface{} {
	return map[string]interface{}{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]interface{}{
			"path": map[string]interface{}{"type": "string", "description": "Target file path."},
		},
		"required": []string{"path"},
	}
}

func applyPatchUpdateFileSchema() map[string]interface{} {
	return map[string]interface{}{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]interface{}{
			"path":    map[string]interface{}{"type": "string", "description": "Target file path."},
			"move_to": map[string]interface{}{"type": "string", "description": "Optional destination path for move operations."},
			"hunks":   applyPatchHunksSchema(),
		},
		"required": []string{"path", "hunks"},
	}
}

func applyPatchReplaceFileSchema() map[string]interface{} {
	return map[string]interface{}{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]interface{}{
			"path":    map[string]interface{}{"type": "string", "description": "Target file path."},
			"content": map[string]interface{}{"type": "string", "description": "Full replacement content."},
		},
		"required": []string{"path", "content"},
	}
}

func applyPatchBatchSchema() map[string]interface{} {
	return map[string]interface{}{
		"type":                 "object",
		"additionalProperties": false,
		"properties": map[string]interface{}{
			"operations": map[string]interface{}{
				"type":        "array",
				"description": "Ordered list of file patch operations.",
				"items": map[string]interface{}{
					"type":                 "object",
					"additionalProperties": false,
					"properties": map[string]interface{}{
						"type":    map[string]interface{}{"type": "string", "enum": []string{"add_file", "delete_file", "update_file", "replace_file"}},
						"path":    map[string]interface{}{"type": "string"},
						"move_to": map[string]interface{}{"type": "string", "description": "Optional destination path for move operations (update_file only)."},
						"content": map[string]interface{}{"type": "string", "description": "Full file content for add_file / replace_file."},
						"hunks":   applyPatchHunksSchema(),
					},
					"required": []string{"type", "path"},
				},
			},
		},
		"required": []string{"operations"},
	}
}

// applyPatchHunksSchema 返回 hunks 数组的完整 JSON Schema 定义。
// 抽取出来供 update_file 单文件工具与 batch 工具共用，确保两处 schema 一致，
// 避免某些严格校验的上游（如 OpenAI 官方）因 array 缺少 items 字段返回
// invalid_function_parameters 400 错误。
func applyPatchHunksSchema() map[string]interface{} {
	return map[string]interface{}{
		"type":        "array",
		"description": "Structured update hunks (required when type=update_file).",
		"items": map[string]interface{}{
			"type":                 "object",
			"additionalProperties": false,
			"properties": map[string]interface{}{
				"context": map[string]interface{}{"type": "string", "description": "Optional @@ context header text."},
				"lines": map[string]interface{}{
					"type": "array",
					"items": map[string]interface{}{
						"type":                 "object",
						"additionalProperties": false,
						"properties": map[string]interface{}{
							"op":   map[string]interface{}{"type": "string", "enum": []string{"context", "add", "remove"}},
							"text": map[string]interface{}{"type": "string"},
						},
						"required": []string{"op", "text"},
					},
				},
			},
			"required": []string{"lines"},
		},
	}
}

// ============== Patch Reconstruction Types ==============

type ApplyPatchProxyInput struct {
	Operations []ApplyPatchOperation `json:"operations"`
	RawPatch   string                `json:"raw_patch"`
	Patch      string                `json:"patch"`
	Input      string                `json:"input"`
}

type ApplyPatchOperation struct {
	Type    string             `json:"type"`
	Path    string             `json:"path"`
	MoveTo  string             `json:"move_to"`
	Content string             `json:"content"`
	Hunks   []ApplyPatchHunk   `json:"hunks"`
	Changes string             `json:"changes"`
	Lines   []ApplyPatchLineOp `json:"lines"`
}

type ApplyPatchHunk struct {
	Context string             `json:"context"`
	Lines   []ApplyPatchLineOp `json:"lines"`
}

type ApplyPatchLineOp struct {
	Op   string `json:"op"`
	Text string `json:"text"`
}

// BuildApplyPatchInput reconstructs the raw apply_patch grammar input from operations.
func BuildApplyPatchInput(ops []ApplyPatchOperation) string {
	var sb strings.Builder
	sb.WriteString("*** Begin Patch\n")
	for _, op := range ops {
		switch op.Type {
		case "add_file":
			sb.WriteString("*** Add File: ")
			sb.WriteString(op.Path)
			sb.WriteString("\n")
			writeApplyPatchAddedContent(&sb, op.Content)
		case "delete_file":
			sb.WriteString("*** Delete File: ")
			sb.WriteString(op.Path)
			sb.WriteString("\n")
		case "update_file":
			sb.WriteString("*** Update File: ")
			sb.WriteString(op.Path)
			sb.WriteString("\n")
			if op.MoveTo != "" {
				sb.WriteString("*** Move to: ")
				sb.WriteString(op.MoveTo)
				sb.WriteString("\n")
			}
			for _, hunk := range op.Hunks {
				if hunk.Context != "" {
					sb.WriteString("@@ ")
					sb.WriteString(hunk.Context)
					sb.WriteString("\n")
				} else {
					sb.WriteString("@@\n")
				}
				for _, l := range hunk.Lines {
					sb.WriteString(lineOpPrefix(l.Op))
					sb.WriteString(l.Text)
					sb.WriteString("\n")
				}
			}
		case "replace_file":
			sb.WriteString("*** Delete File: ")
			sb.WriteString(op.Path)
			sb.WriteString("\n")
			sb.WriteString("*** Add File: ")
			sb.WriteString(op.Path)
			sb.WriteString("\n")
			writeApplyPatchAddedContent(&sb, op.Content)
		}
	}
	sb.WriteString("*** End Patch")
	return sb.String()
}

func writeApplyPatchAddedContent(sb *strings.Builder, content string) {
	if content == "" {
		return
	}
	content = strings.TrimSuffix(content, "\n")
	for _, line := range strings.Split(content, "\n") {
		sb.WriteString("+")
		sb.WriteString(line)
		sb.WriteString("\n")
	}
}

// ApplyPatchInputFromProxyArguments converts upstream proxy tool arguments
// into raw apply_patch grammar text.
func ApplyPatchInputFromProxyArguments(rawArguments string, action string) string {
	var parsed map[string]interface{}
	if err := json.Unmarshal([]byte(rawArguments), &parsed); err != nil {
		return rawArguments
	}
	return applyPatchInputFromParsedArgs(parsed, action, rawArguments)
}

func applyPatchInputFromParsedArgs(args map[string]interface{}, action, rawArguments string) string {
	if input, ok := args["input"].(string); ok && action != "" {
		var nested map[string]interface{}
		if err := json.Unmarshal([]byte(input), &nested); err == nil {
			for key, value := range nested {
				if _, exists := args[key]; !exists {
					args[key] = value
				}
			}
		}
	}

	var ops []ApplyPatchOperation

	switch action {
	case "add_file":
		path, _ := args["path"].(string)
		content, _ := args["content"].(string)
		ops = append(ops, ApplyPatchOperation{Type: "add_file", Path: path, Content: content})
	case "delete_file":
		path, _ := args["path"].(string)
		ops = append(ops, ApplyPatchOperation{Type: "delete_file", Path: path})
	case "update_file":
		path, _ := args["path"].(string)
		moveTo, _ := args["move_to"].(string)
		ops = append(ops, ApplyPatchOperation{Type: "update_file", Path: path, MoveTo: moveTo, Hunks: parseHunksFromRaw(args["hunks"])})
	case "replace_file":
		path, _ := args["path"].(string)
		content, _ := args["content"].(string)
		ops = append(ops, ApplyPatchOperation{Type: "replace_file", Path: path, Content: content})
	case "batch":
		if rawOps, _ := args["operations"].([]interface{}); rawOps != nil {
			for _, rawOp := range rawOps {
				opMap, ok := rawOp.(map[string]interface{})
				if !ok {
					continue
				}
				opType, _ := opMap["type"].(string)
				path, _ := opMap["path"].(string)
				ops = append(ops, ApplyPatchOperation{
					Type:    opType,
					Path:    path,
					MoveTo:  mapString(opMap, "move_to"),
					Content: mapString(opMap, "content"),
					Hunks:   parseHunksFromRaw(opMap["hunks"]),
				})
			}
		}
	default:
		if input, ok := args["input"].(string); ok {
			return input
		}
		return rawArguments
	}

	if len(ops) == 0 {
		return rawArguments
	}
	return BuildApplyPatchInput(ops)
}

func mapString(m map[string]interface{}, key string) string {
	v, _ := m[key].(string)
	return v
}

func parseHunksFromRaw(raw interface{}) []ApplyPatchHunk {
	arr, ok := raw.([]interface{})
	if !ok {
		return nil
	}
	hunks := make([]ApplyPatchHunk, 0, len(arr))
	for _, rawHunk := range arr {
		hunkMap, ok := rawHunk.(map[string]interface{})
		if !ok {
			continue
		}
		hunks = append(hunks, ApplyPatchHunk{
			Context: mapString(hunkMap, "context"),
			Lines:   parseLineOpsFromRaw(hunkMap["lines"]),
		})
	}
	return hunks
}

func parseLineOpsFromRaw(raw interface{}) []ApplyPatchLineOp {
	arr, ok := raw.([]interface{})
	if !ok {
		return nil
	}
	lines := make([]ApplyPatchLineOp, 0, len(arr))
	for _, rawLine := range arr {
		lineMap, ok := rawLine.(map[string]interface{})
		if !ok {
			continue
		}
		lines = append(lines, ApplyPatchLineOp{
			Op:   mapString(lineMap, "op"),
			Text: mapString(lineMap, "text"),
		})
	}
	return lines
}

// ParseApplyPatchOperations attempts to parse raw apply_patch grammar text into structured operations.
func ParseApplyPatchOperations(input string) ([]ApplyPatchOperation, bool) {
	if input == "" || !strings.HasPrefix(input, "*** Begin Patch") {
		return nil, false
	}

	var ops []ApplyPatchOperation
	lines := strings.Split(input, "\n")

	var currentOp *ApplyPatchOperation

	for _, line := range lines {
		line = strings.TrimRight(line, "\r")

		if line == "*** Begin Patch" || line == "*** End Patch" {
			continue
		}

		if after, ok := strings.CutPrefix(line, "*** Add File: "); ok {
			if currentOp != nil {
				ops = append(ops, *currentOp)
			}
			currentOp = &ApplyPatchOperation{Type: "add_file", Path: after}
			continue
		}

		if after, ok := strings.CutPrefix(line, "*** Delete File: "); ok {
			if currentOp != nil {
				ops = append(ops, *currentOp)
			}
			currentOp = &ApplyPatchOperation{Type: "delete_file", Path: after}
			continue
		}

		if after, ok := strings.CutPrefix(line, "*** Update File: "); ok {
			if currentOp != nil {
				ops = append(ops, *currentOp)
			}
			currentOp = &ApplyPatchOperation{Type: "update_file", Path: after}
			continue
		}

		if after, ok := strings.CutPrefix(line, "*** Move to: "); ok && currentOp != nil && currentOp.Type == "update_file" {
			currentOp.MoveTo = after
			continue
		}

		if strings.HasPrefix(line, "@@") && currentOp != nil && currentOp.Type == "update_file" {
			currentOp.Hunks = append(currentOp.Hunks, ApplyPatchHunk{
				Context: strings.TrimSpace(strings.TrimPrefix(line, "@@")),
			})
			continue
		}

		if currentOp != nil {
			switch currentOp.Type {
			case "add_file":
				if after, ok := strings.CutPrefix(line, "+"); ok {
					currentOp.Content += after + "\n"
				}
			case "update_file":
				if len(currentOp.Hunks) > 0 {
					hunkIdx := len(currentOp.Hunks) - 1
					if prefix, op := lineOpFromPrefix(line); op != "" {
						currentOp.Hunks[hunkIdx].Lines = append(currentOp.Hunks[hunkIdx].Lines, ApplyPatchLineOp{
							Op:   op,
							Text: strings.TrimPrefix(line, prefix),
						})
					}
				}
			}
		}
	}

	if currentOp != nil {
		ops = append(ops, *currentOp)
	}

	if len(ops) == 0 {
		return nil, false
	}
	return ops, true
}

// NormalizeApplyPatchInput cleans up common model errors in patch grammar output.
func NormalizeApplyPatchInput(input string) string {
	input = strings.ReplaceAll(input, "+*** End of File", "*** End of File")
	input = strings.ReplaceAll(input, "+*** End Patch", "*** End Patch")
	if !strings.HasSuffix(input, "\n*** End Patch") {
		input = strings.TrimSuffix(input, "*** End Patch")
		input = strings.TrimRight(input, "\n") + "\n*** End Patch"
	}
	return input
}

// ============== Response-Side Custom Tool Reconstruction ==============

// ReconstructCustomToolCallInput rebuilds the raw custom tool input from upstream proxy function call arguments.
func ReconstructCustomToolCallInput(ctx CodexToolContext, upstreamName, rawArguments string) string {
	spec, ok := ctx.CustomTools[upstreamName]
	if !ok {
		return rawArguments
	}

	switch spec.Kind {
	case CodexCustomToolApplyPatch:
		action := spec.ProxyAction
		if action == "" {
			action = proxyActionFromUpstreamName(upstreamName)
		}
		return ApplyPatchInputFromProxyArguments(rawArguments, action)
	default:
		var parsed map[string]interface{}
		if err := json.Unmarshal([]byte(rawArguments), &parsed); err != nil {
			return rawArguments
		}
		if input, ok := parsed["input"].(string); ok {
			return input
		}
		return rawArguments
	}
}

func proxyActionFromUpstreamName(name string) string {
	switch {
	case strings.HasSuffix(name, "_add_file"):
		return "add_file"
	case strings.HasSuffix(name, "_delete_file"):
		return "delete_file"
	case strings.HasSuffix(name, "_update_file"):
		return "update_file"
	case strings.HasSuffix(name, "_replace_file"):
		return "replace_file"
	case strings.HasSuffix(name, "_batch"):
		return "batch"
	default:
		return ""
	}
}

// BuildCustomToolCallHistoryArguments converts a Codex custom_tool_call input
// into upstream Chat Completions function call arguments for history replay.
func BuildCustomToolCallHistoryArguments(ctx CodexToolContext, originalName, input string) (string, string) {
	spec, ok := ctx.CustomTools[originalName]
	if !ok {
		argsJSON, _ := json.Marshal(map[string]interface{}{"input": input})
		return originalName, string(argsJSON)
	}

	switch spec.Kind {
	case CodexCustomToolApplyPatch:
		ops, ok := ParseApplyPatchOperations(input)
		if !ok || len(ops) == 0 {
			argsJSON, _ := json.Marshal(map[string]interface{}{
				"operations": []interface{}{},
				"raw_patch":  input,
			})
			return spec.OpenAIName + "_batch", string(argsJSON)
		}
		if len(ops) == 1 {
			action := chooseSingleProxyAction(ops[0].Type)
			return spec.OpenAIName + "_" + action, buildSingleOpArgsJSON(ops[0])
		}
		return spec.OpenAIName + "_batch", buildBatchOpsJSON(ops)
	default:
		argsJSON, _ := json.Marshal(map[string]interface{}{"input": input})
		return spec.OpenAIName, string(argsJSON)
	}
}

func chooseSingleProxyAction(opType string) string {
	switch opType {
	case "add_file", "delete_file", "update_file", "replace_file":
		return opType
	default:
		return "batch"
	}
}

func buildSingleOpArgsJSON(op ApplyPatchOperation) string {
	var args map[string]interface{}
	switch op.Type {
	case "add_file", "replace_file":
		args = map[string]interface{}{"path": op.Path, "content": op.Content}
	case "delete_file":
		args = map[string]interface{}{"path": op.Path}
	case "update_file":
		hunks := make([]map[string]interface{}, len(op.Hunks))
		for i, h := range op.Hunks {
			lines := make([]map[string]interface{}, len(h.Lines))
			for j, l := range h.Lines {
				lines[j] = map[string]interface{}{"op": l.Op, "text": l.Text}
			}
			hunks[i] = map[string]interface{}{"context": h.Context, "lines": lines}
		}
		args = map[string]interface{}{"path": op.Path, "hunks": hunks}
		if op.MoveTo != "" {
			args["move_to"] = op.MoveTo
		}
	default:
		args = map[string]interface{}{"path": op.Path}
	}
	b, _ := json.Marshal(args)
	return string(b)
}

func buildBatchOpsJSON(ops []ApplyPatchOperation) string {
	batchOps := make([]map[string]interface{}, len(ops))
	for i, op := range ops {
		item := map[string]interface{}{"type": op.Type, "path": op.Path}
		if op.MoveTo != "" {
			item["move_to"] = op.MoveTo
		}
		if op.Content != "" {
			item["content"] = op.Content
		}
		if len(op.Hunks) > 0 {
			hunks := make([]map[string]interface{}, len(op.Hunks))
			for j, h := range op.Hunks {
				lines := make([]map[string]interface{}, len(h.Lines))
				for k, l := range h.Lines {
					lines[k] = map[string]interface{}{"op": l.Op, "text": l.Text}
				}
				hunks[j] = map[string]interface{}{"context": h.Context, "lines": lines}
			}
			item["hunks"] = hunks
		}
		batchOps[i] = item
	}
	b, _ := json.Marshal(map[string]interface{}{"operations": batchOps})
	return string(b)
}

func lineOpPrefix(op string) string {
	switch op {
	case "context":
		return " "
	case "add":
		return "+"
	case "remove", "delete":
		return "-"
	default:
		return " "
	}
}

func lineOpFromPrefix(line string) (string, string) {
	if len(line) == 0 {
		return "", ""
	}
	switch line[0] {
	case ' ':
		return " ", "context"
	case '+':
		return "+", "add"
	case '-':
		return "-", "remove"
	default:
		return "", ""
	}
}

// ============== Tool Choice ==============

// ConvertToolChoiceForCodex converts a Codex custom tool choice to upstream format.
func ConvertToolChoiceForCodex(toolChoice interface{}, ctx CodexToolContext) interface{} {
	tcMap, ok := toolChoice.(map[string]interface{})
	if !ok {
		return toolChoice
	}
	tcType, _ := tcMap["type"].(string)
	switch tcType {
	case "function":
		// Check for namespace tool_choice: {"type":"function","namespace":"...","name":"..."}
		if namespace, _ := tcMap["namespace"].(string); namespace != "" {
			name, _ := tcMap["name"].(string)
			flat := flattenNamespaceToolName(namespace, name)
			return map[string]interface{}{
				"type":     "function",
				"function": map[string]interface{}{"name": flat},
			}
		}
		// Check for nested namespace tool_choice: {"type":"function","function":{"namespace":"...","name":"..."}}
		if fnMap, ok := tcMap["function"].(map[string]interface{}); ok {
			if ns, _ := fnMap["namespace"].(string); ns != "" {
				name, _ := fnMap["name"].(string)
				flat := flattenNamespaceToolName(ns, name)
				return map[string]interface{}{
					"type":     "function",
					"function": map[string]interface{}{"name": flat},
				}
			}
		}
		return toolChoice
	case "custom":
		name, _ := tcMap["name"].(string)
		if name == "" {
			return toolChoice
		}
		spec, ok := ctx.CustomTools[name]
		if !ok {
			return nil
		}
		switch spec.Kind {
		case CodexCustomToolApplyPatch:
			return map[string]interface{}{
				"type": "function",
				"function": map[string]interface{}{
					"name": spec.OpenAIName + "_batch",
				},
			}
		default:
			return map[string]interface{}{
				"type": "function",
				"function": map[string]interface{}{
					"name": spec.OpenAIName,
				},
			}
		}
	default:
		return toolChoice
	}
}

// ============== Response Wrappers ==============

// WrapOpenAIChatResponseToResponsesWithContext converts a Chat Completions response
// and remaps proxy function_call items back into custom_tool_call items,
// and unflattens namespace function calls back into namespace/name format.
func WrapOpenAIChatResponseToResponsesWithContext(
	openaiResp map[string]interface{},
	sessionID string,
	ctx CodexToolContext,
) (*types.ResponsesResponse, error) {
	resp, err := OpenAIChatResponseToResponses(openaiResp, sessionID)
	if err != nil {
		return nil, err
	}

	if resp == nil {
		return resp, nil
	}

	if ctx.HasCustomTools {
		ctx.RemapCustomToolCallsInResponse(resp)
	}

	if ctx.HasNamespaceTools {
		ctx.RemapNamespaceFunctionCallsInResponse(resp)
	}

	return resp, nil
}

// RemapCustomToolCallsInResponse remaps proxy function_call items to custom_tool_call in a ResponsesResponse.
func (ctx *CodexToolContext) RemapCustomToolCallsInResponse(resp *types.ResponsesResponse) {
	if resp == nil || !ctx.HasCustomTools {
		return
	}
	for i, item := range resp.Output {
		if item.Type != "function_call" {
			continue
		}
		spec, ok := ctx.CustomTools[item.Name]
		if !ok {
			continue
		}
		customInput := ReconstructCustomToolCallInput(*ctx, item.Name, item.Arguments)
		if customInput != "" {
			resp.Output[i] = types.ResponsesItem{
				Type:   "custom_tool_call",
				CallID: item.CallID,
				Name:   spec.OpenAIName,
				Status: "completed",
				Input:  customInput,
			}
		}
	}
}

// RemapNamespaceFunctionCallsInResponse unflattens namespace function calls in a ResponsesResponse.
func (ctx *CodexToolContext) RemapNamespaceFunctionCallsInResponse(resp *types.ResponsesResponse) {
	if resp == nil || len(ctx.FunctionTools) == 0 {
		return
	}
	for i := range resp.Output {
		item := &resp.Output[i]
		if item.Type != "function_call" {
			continue
		}
		name, namespace := ctx.OpenAINameForFunctionTool(item.Name)
		if namespace == "" {
			continue
		}
		item.Name = name
		item.Namespace = namespace
	}
}

func customToolInputFromItem(item types.ResponsesItem) string {
	if item.Input != "" {
		return item.Input
	}
	if item.Arguments != "" {
		return item.Arguments
	}
	if s, ok := item.Output.(string); ok {
		return s
	}
	return ""
}

// replayCustomToolCall converts a custom_tool_call input for history replay without CodexToolContext.
// It detects apply_patch by name and content, and returns (upstreamName, argsJSON).
func replayCustomToolCall(name, input string) (string, string) {
	if name == "apply_patch" || (strings.HasPrefix(input, "*** Begin Patch") && strings.Contains(input, "*** End Patch")) {
		ops, ok := ParseApplyPatchOperations(input)
		if !ok || len(ops) == 0 {
			argsJSON, _ := json.Marshal(map[string]interface{}{
				"operations": []interface{}{},
				"raw_patch":  input,
			})
			return name + "_batch", string(argsJSON)
		}
		if len(ops) == 1 {
			action := chooseSingleProxyAction(ops[0].Type)
			return name + "_" + action, buildSingleOpArgsJSON(ops[0])
		}
		return name + "_batch", buildBatchOpsJSON(ops)
	}
	// Generic custom tool: {"input": "..."}
	argsJSON, _ := json.Marshal(map[string]interface{}{"input": input})
	return name, string(argsJSON)
}
