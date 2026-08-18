package executor

import (
	"fmt"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

func normalizeCodexCollaborationSpawnAgentModel(payload []byte) []byte {
	switch gjson.GetBytes(payload, "type").String() {
	case "response.output_item.done":
		return normalizeCodexSpawnAgentItemModel(payload, "item")
	case "response.completed", "response.done":
		output := gjson.GetBytes(payload, "response.output")
		if !output.Exists() || !output.IsArray() {
			return payload
		}
		for index := range output.Array() {
			payload = normalizeCodexSpawnAgentItemModel(payload, fmt.Sprintf("response.output.%d", index))
		}
	}
	return payload
}

func normalizeCodexSpawnAgentItemModel(payload []byte, itemPath string) []byte {
	item := gjson.GetBytes(payload, itemPath)
	if !isCodexCollaborationSpawnAgentCall(item) {
		return payload
	}

	arguments := item.Get("arguments")
	var argumentsJSON []byte
	switch arguments.Type {
	case gjson.String:
		argumentsJSON = []byte(arguments.String())
	case gjson.JSON:
		argumentsJSON = []byte(arguments.Raw)
	default:
		return payload
	}
	if !gjson.ValidBytes(argumentsJSON) {
		return payload
	}

	model := gjson.GetBytes(argumentsJSON, "model")
	if model.Type != gjson.String {
		return payload
	}
	modelID := codexSpawnAgentModelID(model.String())
	if modelID == "" || modelID == model.String() {
		return payload
	}

	normalizedArguments, err := sjson.SetBytes(argumentsJSON, "model", modelID)
	if err != nil {
		return payload
	}
	argumentsPath := itemPath + ".arguments"
	if arguments.Type == gjson.JSON {
		updated, err := sjson.SetRawBytes(payload, argumentsPath, normalizedArguments)
		if err == nil {
			return updated
		}
		return payload
	}
	updated, err := sjson.SetBytes(payload, argumentsPath, string(normalizedArguments))
	if err != nil {
		return payload
	}
	return updated
}

func isCodexCollaborationSpawnAgentCall(item gjson.Result) bool {
	if item.Get("type").String() != "function_call" {
		return false
	}
	name := strings.ToLower(strings.TrimSpace(item.Get("name").String()))
	if name == "spawn_agent" {
		return true
	}
	if !strings.HasSuffix(name, "__spawn_agent") && !strings.HasSuffix(name, ".spawn_agent") {
		return false
	}
	return strings.Contains(name, "collaboration") || strings.Contains(name, "multi_agent")
}

func codexSpawnAgentModelID(model string) string {
	switch strings.ToLower(strings.TrimSpace(model)) {
	case "sol":
		return "gpt-5.6-sol"
	case "terra":
		return "gpt-5.6-terra"
	case "luna":
		return "gpt-5.6-luna"
	default:
		return model
	}
}
