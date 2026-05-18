package converters

import "github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"

func defaultResponsesToolParameters() map[string]interface{} {
	return map[string]interface{}{
		"type":       "object",
		"properties": map[string]interface{}{},
		"required":   []interface{}{},
	}
}

func sanitizeResponsesToolParameters(parameters interface{}) interface{} {
	paramMap, ok := parameters.(map[string]interface{})
	if !ok {
		return defaultResponsesToolParameters()
	}
	if paramType, ok := paramMap["type"].(string); !ok || paramType == "" {
		paramMap["type"] = "object"
	}
	if _, ok := paramMap["properties"].(map[string]interface{}); !ok {
		paramMap["properties"] = map[string]interface{}{}
	}
	// 补齐 required=[]：部分上游（如 duckcoding 的 OpenAI 严格 schema 校验）
	// 在 required 缺失时直接按 None 校验，抛出
	// "Invalid schema for function ...: None is not of type 'array'"。
	if _, exists := paramMap["required"]; !exists {
		paramMap["required"] = []interface{}{}
	}
	return paramMap
}

func extractResponsesToolFields(tool map[string]interface{}) (string, string, interface{}) {
	name, _ := tool["name"].(string)
	description, _ := tool["description"].(string)
	parameters := tool["parameters"]

	if function, ok := tool["function"].(map[string]interface{}); ok {
		if name == "" {
			name, _ = function["name"].(string)
		}
		if description == "" {
			description, _ = function["description"].(string)
		}
		if parameters == nil {
			parameters = function["parameters"]
		}
	}

	if parameters == nil {
		parameters = defaultResponsesToolParameters()
	} else {
		parameters = sanitizeResponsesToolParameters(parameters)
	}

	return name, description, parameters
}

// isChatCompatibleResponsesTool 判断 Responses tool 是否能安全映射到 Chat Completions。
// Chat Completions 只支持 function tool；Responses 扩展类型（custom/web_search/
// namespace 等）直接映射会让上游拒绝或产出无效参数，应在入口处过滤掉。
func isChatCompatibleResponsesTool(tool map[string]interface{}) bool {
	toolType, _ := tool["type"].(string)
	if toolType == "" || toolType == "function" {
		return true
	}
	return false
}

func responsesToolsToOpenAI(tools []map[string]interface{}) []map[string]interface{} {
	openaiTools := make([]map[string]interface{}, 0, len(tools))
	for _, tool := range tools {
		if !isChatCompatibleResponsesTool(tool) {
			continue
		}
		name, description, parameters := extractResponsesToolFields(tool)
		if name == "" {
			continue
		}
		openaiTool := map[string]interface{}{
			"type": "function",
			"function": map[string]interface{}{
				"name":       name,
				"parameters": parameters,
			},
		}
		if description != "" {
			openaiTool["function"].(map[string]interface{})["description"] = description
		}
		openaiTools = append(openaiTools, openaiTool)
	}
	return openaiTools
}

func responsesToolsToClaude(tools []map[string]interface{}) []map[string]interface{} {
	claudeTools := make([]map[string]interface{}, 0, len(tools))
	for _, tool := range tools {
		name, description, parameters := extractResponsesToolFields(tool)
		if name == "" {
			continue
		}
		claudeTool := map[string]interface{}{
			"name":         name,
			"input_schema": parameters,
		}
		if description != "" {
			claudeTool["description"] = description
		}
		claudeTools = append(claudeTools, claudeTool)
	}
	return claudeTools
}

func responsesToolsToGemini(tools []map[string]interface{}) []types.GeminiTool {
	declarations := make([]types.GeminiFunctionDeclaration, 0, len(tools))
	for _, tool := range tools {
		name, description, parameters := extractResponsesToolFields(tool)
		if name == "" {
			continue
		}
		declarations = append(declarations, types.GeminiFunctionDeclaration{
			Name:        name,
			Description: description,
			Parameters:  parameters,
		})
	}
	if len(declarations) == 0 {
		return nil
	}
	return []types.GeminiTool{{FunctionDeclarations: declarations}}
}
