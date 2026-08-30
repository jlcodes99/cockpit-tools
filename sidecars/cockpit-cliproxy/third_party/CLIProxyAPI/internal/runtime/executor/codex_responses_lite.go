package executor

import (
	"fmt"
	"net/http"
	"strings"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const codexResponsesLiteHeaderName = "X-OpenAI-Internal-Codex-Responses-Lite"

func codexResponsesLiteEnabled(headers http.Header) bool {
	for name := range headers {
		if strings.EqualFold(name, codexResponsesLiteHeaderName) {
			return true
		}
	}
	return false
}

func removeCodexResponsesLiteHeaderForFullResponse(headers http.Header, full bool) {
	if !full || headers == nil {
		return
	}
	for name := range headers {
		if strings.EqualFold(name, codexResponsesLiteHeaderName) {
			delete(headers, name)
		}
	}
}

func normalizeCodexResponsesLiteRequest(body []byte, headers http.Header, auth *cliproxyauth.Auth, allowFullResponsesForImage bool) ([]byte, bool) {
	if (!codexResponsesLiteEnabled(headers) && !isCodexResponsesLiteRequest(body, headers)) || auth == nil || codexAuthUsesAPIKey(auth) {
		return body, false
	}
	body, _ = sjson.SetBytes(body, "parallel_tool_calls", false)
	if allowFullResponsesForImage && codexRequestUsesImageGeneration(body) {
		return body, true
	}

	// Responses Lite metadata can appear at the root, in additional_tools input
	// items, or under response. Filter every such object; the old monolithic
	// executor did this recursively and the proxy split accidentally lost it.
	paths := codexToolObjectPaths(body)
	for index := len(paths) - 1; index >= 0; index-- {
		objectPath := paths[index]
		body = filterCodexResponsesLiteTools(body, codexObjectFieldPath(objectPath, "tools"))
		body = filterCodexResponsesLiteToolChoice(body, codexObjectFieldPath(objectPath, "tool_choice"))
	}
	// Empty additional_tools items are invalid and were removed by the legacy
	// recursive filter. Delete them from deepest to shallowest paths.
	for index := len(paths) - 1; index >= 0; index-- {
		path := paths[index]
		if path == "" || !strings.Contains(path, ".input.") && !strings.HasPrefix(path, "input.") {
			continue
		}
		if !strings.EqualFold(strings.TrimSpace(gjson.GetBytes(body, path+".type").String()), "additional_tools") {
			continue
		}
		tools := gjson.GetBytes(body, path+".tools")
		if !tools.IsArray() || len(tools.Array()) == 0 {
			body, _ = sjson.DeleteBytes(body, path)
		}
	}
	return body, false
}

func filterCodexResponsesLiteTools(body []byte, toolsPath string) []byte {
	tools := gjson.GetBytes(body, toolsPath)
	if !tools.IsArray() {
		return body
	}
	filtered := make([][]byte, 0, len(tools.Array()))
	for _, tool := range tools.Array() {
		if codexResponsesLiteToolSupported(tool) {
			filtered = append(filtered, []byte(tool.Raw))
		}
	}
	if len(filtered) == 0 {
		body, _ = sjson.DeleteBytes(body, toolsPath)
		return body
	}
	body, _ = sjson.SetRawBytes(body, toolsPath, joinJSONArray(filtered))
	return body
}

func filterCodexResponsesLiteToolChoice(body []byte, choicePath string) []byte {
	choice := gjson.GetBytes(body, choicePath)
	if !choice.Exists() {
		return body
	}
	if choice.Type == gjson.String {
		switch strings.ToLower(strings.TrimSpace(choice.String())) {
		case "auto", "none", "required":
			return body
		default:
			body, _ = sjson.DeleteBytes(body, choicePath)
			return body
		}
	}
	if !choice.IsObject() {
		body, _ = sjson.DeleteBytes(body, choicePath)
		return body
	}
	switch strings.ToLower(strings.TrimSpace(choice.Get("type").String())) {
	case "function", "custom", "namespace":
		return body
	case "tool_search":
		if codexResponsesLiteToolSupported(choice) {
			return body
		}
	case "allowed_tools":
		hasTools := false
		for _, relativePath := range []string{"tools", "allowed_tools", "allowed_tools.tools"} {
			path := choicePath + "." + relativePath
			before := gjson.GetBytes(body, path)
			body = filterCodexResponsesLiteTools(body, path)
			remaining := gjson.GetBytes(body, path)
			if before.IsArray() && remaining.IsArray() && len(remaining.Array()) > 0 {
				hasTools = true
			}
		}
		if hasTools {
			return body
		}
	}
	body, _ = sjson.DeleteBytes(body, choicePath)
	return body
}

func codexRequestUsesImageGeneration(body []byte) bool {
	for _, objectPath := range codexToolObjectPaths(body) {
		tools := gjson.GetBytes(body, codexObjectFieldPath(objectPath, "tools"))
		if containsCodexImageTool(tools) {
			return true
		}
		if codexImageToolReference(gjson.GetBytes(body, codexObjectFieldPath(objectPath, "tool_choice"))) {
			return true
		}
	}
	return false
}

func joinJSONArray(items [][]byte) []byte {
	if len(items) == 0 {
		return []byte("[]")
	}
	out := []byte("[")
	for index, item := range items {
		if index > 0 {
			out = append(out, ',')
		}
		out = append(out, item...)
	}
	return append(out, ']')
}

func codexResponsesLiteToolSupported(tool gjson.Result) bool {
	switch strings.TrimSpace(tool.Get("type").String()) {
	case "function", "custom", "namespace":
		return true
	case "tool_search":
		return strings.EqualFold(strings.TrimSpace(tool.Get("execution").String()), "client")
	default:
		return false
	}
}

func containsCodexImageTool(tools gjson.Result) bool {
	for _, tool := range tools.Array() {
		if codexImageToolReference(tool) {
			return true
		}
	}
	return false
}

func codexImageToolReference(tool gjson.Result) bool {
	if !tool.Exists() {
		return false
	}
	if tool.Type == gjson.String {
		name := strings.TrimSpace(tool.String())
		return strings.EqualFold(name, "image_generation") || strings.EqualFold(name, "image_gen.imagegen")
	}
	if strings.EqualFold(strings.TrimSpace(tool.Get("type").String()), "image_generation") {
		return true
	}
	if strings.EqualFold(strings.TrimSpace(tool.Get("type").String()), "tool") &&
		strings.EqualFold(strings.TrimSpace(tool.Get("name").String()), "image_generation") {
		return true
	}
	return isCodexImageFunction(tool)
}

func isCodexImageFunction(tool gjson.Result) bool {
	return isImageGenerationFunctionTool(tool)
}

var _ = fmt.Sprintf
