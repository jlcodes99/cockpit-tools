package main

import (
	"bytes"
	"encoding/json"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// Codex 0.156 起把工具声明从顶层 `tools` 挪进了 `input` 数组里的 `additional_tools` 项，
// 内部是嵌套的 namespace 结构，freeform 工具用 `custom` 类型声明：
//
//	{"type":"additional_tools","role":"developer","tools":[
//	   {"type":"namespace","name":"functions","tools":[
//	      {"type":"custom","name":"exec","description":"...","format":{...}},
//	      {"type":"function","name":"wait","parameters":{...}}]}]}
//
// 第三方 Responses 上游只认顶层 `tools`，不认识这个输入项，原样转发时工具被静默忽略；而请求里
// 的 `tool_choice` 仍然在位，模型看不到任何工具定义，于是退回各家的原生文本工具调用格式
// （doubao 的 `<seed:tool_call>`、其它上游的 `<tool_call>`）。客户端把它当普通文本渲染，
// 工具永远不会被执行。
//
// 这里在请求出口把 `additional_tools` 提升到顶层 `tools`：
//
//   - namespace 递归摊平，工具名保持原样（客户端按裸名匹配）
//   - `function` 原样搬到顶层
//   - `custom`（freeform）在上游没有对应类型，声明成只有一个 `input` 字符串参数的 function，
//     响应出口再把这些调用的 `function_call` 还原回 `custom_tool_call`，客户端仍按自定义工具
//     处理。`includeCustom` 为 false 时直接跳过 custom 工具——目标上游的响应转换链路还没有
//     对应的还原逻辑时，宁可保持现状也不要造出客户端无法识别的调用。
//
// 请求里没有 `additional_tools` 项时逐字节透传，避免因为多余的重编码丢掉提示词缓存。

const (
	providerAdditionalToolsInputType      = "additional_tools"
	providerAdditionalToolsNamespaceType  = "namespace"
	providerAdditionalToolsFunctionType   = "function"
	providerAdditionalToolsCustomType     = "custom"
	providerAdditionalToolsFreeformArg    = "input"
	providerAdditionalToolsFreeformArgDoc = "Raw input payload for this tool."
)

// providerGatewayHoistAdditionalTools 把 `input[].additional_tools` 提升为顶层 `tools`。
//
// 返回改写后的请求体，以及由 `custom` 工具提升而来的工具名集合——响应出口需要用这个集合把
// `function_call` 还原成 `custom_tool_call`。第二个返回值恒非 nil。
func providerGatewayHoistAdditionalTools(body []byte, includeCustom bool) ([]byte, map[string]bool) {
	freeform := make(map[string]bool)
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() {
		return body, freeform
	}

	// 顶层与 `additional_tools` 里同名的工具只保留先出现的那一个，避免上游收到重复声明。
	seen := make(map[string]bool)
	existingTools := gjson.GetBytes(body, "tools")
	if existingTools.IsArray() {
		for _, tool := range existingTools.Array() {
			if name := strings.TrimSpace(tool.Get("name").String()); name != "" {
				seen[name] = true
			}
		}
	}

	items := input.Array()
	kept := make([]string, 0, len(items))
	hoisted := make([]string, 0)
	removed := false
	for _, item := range items {
		if strings.TrimSpace(item.Get("type").String()) != providerAdditionalToolsInputType {
			kept = append(kept, item.Raw)
			continue
		}
		// 工具已经提升到顶层，这个输入项对上游没有意义，必须一并去掉：严格的上游会按字段
		// 反序列化整个 input，留着未知类型反而可能整包被拒。
		removed = true
		collectProviderGatewayAdditionalTools(item.Get("tools"), includeCustom, &hoisted, seen, freeform)
	}
	if !removed {
		return body, freeform
	}

	updated, err := sjson.SetRawBytes(body, "input", []byte("["+strings.Join(kept, ",")+"]"))
	if err != nil {
		return body, make(map[string]bool)
	}
	if len(hoisted) == 0 {
		return updated, freeform
	}

	merged := make([]string, 0, len(hoisted)+len(existingTools.Array()))
	for _, tool := range existingTools.Array() {
		merged = append(merged, tool.Raw)
	}
	merged = append(merged, hoisted...)
	updated, err = sjson.SetRawBytes(updated, "tools", []byte("["+strings.Join(merged, ",")+"]"))
	if err != nil {
		return body, make(map[string]bool)
	}
	return updated, freeform
}

// collectProviderGatewayAdditionalTools 摊平一层 namespace，把可用的工具追加到 out。
func collectProviderGatewayAdditionalTools(tools gjson.Result, includeCustom bool, out *[]string, seen map[string]bool, freeform map[string]bool) {
	if !tools.IsArray() {
		return
	}
	for _, tool := range tools.Array() {
		switch strings.TrimSpace(tool.Get("type").String()) {
		case providerAdditionalToolsNamespaceType:
			collectProviderGatewayAdditionalTools(tool.Get("tools"), includeCustom, out, seen, freeform)
		case providerAdditionalToolsFunctionType:
			name := strings.TrimSpace(tool.Get("name").String())
			if name == "" || seen[name] {
				continue
			}
			seen[name] = true
			*out = append(*out, tool.Raw)
		case providerAdditionalToolsCustomType:
			if !includeCustom {
				continue
			}
			name := strings.TrimSpace(tool.Get("name").String())
			if name == "" || seen[name] {
				continue
			}
			seen[name] = true
			freeform[name] = true
			*out = append(*out, providerGatewayAdditionalToolsFreeformTool(name, tool.Get("description").String()))
		}
	}
}

// providerGatewayAdditionalToolsFreeformTool 把 freeform 工具声明成等价的 function：
// 上游不理解 `custom`，但能表达「一个字符串入参」。
func providerGatewayAdditionalToolsFreeformTool(name, description string) string {
	declaration := map[string]any{
		"type": "function",
		"name": name,
		"parameters": map[string]any{
			"type": "object",
			"properties": map[string]any{
				providerAdditionalToolsFreeformArg: map[string]any{
					"type":        "string",
					"description": providerAdditionalToolsFreeformArgDoc,
				},
			},
			"required": []string{providerAdditionalToolsFreeformArg},
		},
	}
	if strings.TrimSpace(description) != "" {
		declaration["description"] = description
	}
	encoded, err := json.Marshal(declaration)
	if err != nil {
		return ""
	}
	return string(encoded)
}

// providerGatewayAdditionalToolsResponsePayload 在非流式响应上还原 freeform 工具调用。
func providerGatewayAdditionalToolsResponsePayload(payload []byte, freeform map[string]bool) []byte {
	if len(freeform) == 0 || len(payload) == 0 {
		return payload
	}
	updated, changed := providerGatewayAdditionalToolsRewritePayload(payload, freeform)
	if !changed {
		return payload
	}
	return updated
}

// providerGatewayAdditionalToolsRewritePayload 把 payload 里所有 freeform 工具的
// `function_call` 项还原成 `custom_tool_call`，覆盖 `item`、`output[]` 与 `response.output[]`
// 三种承载位置。
func providerGatewayAdditionalToolsRewritePayload(payload []byte, freeform map[string]bool) ([]byte, bool) {
	if len(freeform) == 0 || !gjson.ValidBytes(payload) {
		return payload, false
	}
	updated := payload
	changed := false

	if gjson.GetBytes(updated, "item.type").Exists() {
		if rewritten, ok := providerGatewayAdditionalToolsRewriteItem(gjson.GetBytes(updated, "item"), freeform); ok {
			if next, err := sjson.SetRawBytes(updated, "item", []byte(rewritten)); err == nil {
				updated, changed = next, true
			}
		}
	}
	for _, container := range []string{"output", "response.output"} {
		items := gjson.GetBytes(updated, container)
		if !items.IsArray() {
			continue
		}
		rebuilt := make([]string, 0, len(items.Array()))
		containerChanged := false
		for _, item := range items.Array() {
			if rewritten, ok := providerGatewayAdditionalToolsRewriteItem(item, freeform); ok {
				rebuilt = append(rebuilt, rewritten)
				containerChanged = true
				continue
			}
			rebuilt = append(rebuilt, item.Raw)
		}
		if !containerChanged {
			continue
		}
		if next, err := sjson.SetRawBytes(updated, container, []byte("["+strings.Join(rebuilt, ",")+"]")); err == nil {
			updated, changed = next, true
		}
	}
	return updated, changed
}

// providerGatewayAdditionalToolsRewriteItem 转换单个工具调用项。第二个返回值表示是否发生改写。
func providerGatewayAdditionalToolsRewriteItem(item gjson.Result, freeform map[string]bool) (string, bool) {
	if strings.TrimSpace(item.Get("type").String()) != "function_call" {
		return "", false
	}
	if !freeform[strings.TrimSpace(item.Get("name").String())] {
		return "", false
	}
	updated, err := sjson.Set(item.Raw, "type", "custom_tool_call")
	if err != nil {
		return "", false
	}
	updated, err = sjson.Set(updated, providerAdditionalToolsFreeformArg, providerGatewayAdditionalToolsFreeformInput(item.Get("arguments").String()))
	if err != nil {
		return "", false
	}
	// `custom_tool_call` 用 `input` 而不是 `arguments`，留着会让客户端按错误的类型反序列化。
	updated, err = sjson.Delete(updated, "arguments")
	if err != nil {
		return "", false
	}
	return updated, true
}

// providerGatewayAdditionalToolsFreeformInput 从 function 参数里取出原始文本。
// 模型没有按约定只给一个 `input` 字段时原样保留参数 JSON，至少不丢信息。
func providerGatewayAdditionalToolsFreeformInput(arguments string) string {
	if strings.TrimSpace(arguments) == "" {
		return ""
	}
	var decoded map[string]any
	if err := json.Unmarshal([]byte(arguments), &decoded); err != nil || len(decoded) != 1 {
		return arguments
	}
	value, ok := decoded[providerAdditionalToolsFreeformArg]
	if !ok {
		return arguments
	}
	if text, isString := value.(string); isString {
		return text
	}
	encoded, err := json.Marshal(value)
	if err != nil {
		return arguments
	}
	return string(encoded)
}

// providerGatewayAdditionalToolsRewriter 在流式响应上还原 freeform 工具调用。
//
// 与 item id 改写器一样按行处理：`event:`、空行与 `[DONE]` 原样返回，只动 `data:` 行里的
// JSON。参数增量事件对 `custom_tool_call` 没有对应形态，直接丢弃——完整入参已经随
// `response.output_item.done` 一起发出，客户端据此还原调用。
type providerGatewayAdditionalToolsRewriter struct {
	freeform map[string]bool
	names    map[string]string
}

func newProviderGatewayAdditionalToolsRewriter(freeform map[string]bool) *providerGatewayAdditionalToolsRewriter {
	return &providerGatewayAdditionalToolsRewriter{
		freeform: freeform,
		names:    make(map[string]string),
	}
}

// RewriteSSELine 返回改写后的行；返回 nil 表示该行应当被丢弃。
func (r *providerGatewayAdditionalToolsRewriter) RewriteSSELine(line []byte) []byte {
	if r == nil || len(r.freeform) == 0 || len(line) == 0 {
		return line
	}
	trimmed := bytes.TrimSpace(line)
	if !bytes.HasPrefix(trimmed, []byte("data:")) {
		return line
	}
	payload := bytes.TrimSpace(trimmed[len("data:"):])
	if len(payload) == 0 || bytes.Equal(payload, []byte("[DONE]")) {
		return line
	}

	switch strings.TrimSpace(gjson.GetBytes(payload, "type").String()) {
	case "response.output_item.added":
		item := gjson.GetBytes(payload, "item")
		if strings.TrimSpace(item.Get("type").String()) == "function_call" {
			r.remember(item.Get("id").String(), item.Get("name").String())
		}
	case "response.output_item.done":
		delete(r.names, strings.TrimSpace(gjson.GetBytes(payload, "item.id").String()))
	case "response.function_call_arguments.delta", "response.function_call_arguments.done":
		if r.freeform[r.names[strings.TrimSpace(gjson.GetBytes(payload, "item_id").String())]] {
			return nil
		}
	}

	rewritten, changed := providerGatewayAdditionalToolsRewritePayload(payload, r.freeform)
	if !changed {
		return line
	}
	out := make([]byte, 0, len(rewritten)+len("data: ")+2)
	out = append(out, "data: "...)
	out = append(out, rewritten...)
	if bytes.HasSuffix(line, []byte("\r\n")) {
		out = append(out, '\r', '\n')
	} else if bytes.HasSuffix(line, []byte("\n")) {
		out = append(out, '\n')
	}
	return out
}

func (r *providerGatewayAdditionalToolsRewriter) remember(id, name string) {
	id = strings.TrimSpace(id)
	name = strings.TrimSpace(name)
	if id == "" || name == "" {
		return
	}
	r.names[id] = name
}
