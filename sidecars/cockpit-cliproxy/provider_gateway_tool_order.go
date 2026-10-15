package main

import (
	"fmt"
	"sort"
	"strings"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// 严格 Responses 上游（DeepSeek）按「位置」校验工具调用：同一条 assistant 回合里的调用必须
// 保持相邻，且这些调用的输出要紧跟在该批次之后，否则整轮请求被拒，而且被拒的这一轮会留在
// 客户端历史里，线程此后无法继续。
//
// 两种破坏方式对应两条不同的上游报错，见到的都是误导性文案：
//
//	调用 → 消息（钩子）→ 输出
//	  → 400 `No tool output found for tool call ...`
//	调用A → 输出A → 调用B → 输出B（同一回合的两个调用被输出拆开）
//	  → 400 `The `reasoning_text` in the thinking mode must be passed back to the API`
//
// 第二条最容易被误判成推理回放问题：上游把「输出夹在中间」理解成「新起了一批调用却没有回放
// 推理」，于是报了 reasoning_text。实测同一份历史保持「调用A → 调用B → 输出A → 输出B」时
// 稳定 200，改成「调用A → 输出A → 调用B → 输出B」稳定 400。
//
// Codex 在正常路径上就会破坏第一种顺序：工具执行完的 PostToolUse 钩子会立刻把开发消息写进历史，
// 可能早于工具输出项落盘，于是历史里出现
//
//	function_call -> message -> function_call_output
//
// 官方上游只按 `call_id` 配对、不校验位置，所以一直没暴露；DeepSeek 会校验。
//
// 这里在请求出口把顺序还原：连续的调用视为同一批次整体保留，批次结束后再按原有相对顺序放这批
// 调用的输出；其余项保持相对顺序。已经在位的项不动，因此正常历史（官方账号、以及本来就合法的
// DeepSeek 历史）是逐字节 no-op。

const (
	providerToolOrderCallType       = "function_call"
	providerToolOrderCustomCallType = "custom_tool_call"
	providerToolOrderSearchCallType = "tool_search_call"
	providerToolOrderOutputType     = "function_call_output"
	providerToolOrderCustomOutput   = "custom_tool_call_output"
	providerToolOrderSearchOutput   = "tool_search_output"
)

// providerGatewayRepairsToolCallOrder 报告该上游是否要求「输出紧跟调用」。
//
// 只对 DeepSeek 官方上游启用：官方上游不校验位置，不需要为它重排历史（重排只会白白改变
// 提示词前缀、影响缓存命中），其它第三方上游也未观察到该要求。
func providerGatewayRepairsToolCallOrder(gateway *providerGatewaySpec) bool {
	return isDeepSeekResponsesGateway(gatewayBaseURL(gateway))
}

func gatewayBaseURL(gateway *providerGatewaySpec) string {
	if gateway == nil {
		return ""
	}
	return gateway.BaseURL
}

func providerToolOrderIsCallItem(itemType string) bool {
	switch strings.ToLower(strings.TrimSpace(itemType)) {
	case providerToolOrderCallType, providerToolOrderCustomCallType, providerToolOrderSearchCallType, "tool_call", "mcp_tool_call":
		return true
	default:
		return false
	}
}

func providerToolOrderIsOutputItem(itemType string) bool {
	switch strings.ToLower(strings.TrimSpace(itemType)) {
	case providerToolOrderOutputType, providerToolOrderCustomOutput, providerToolOrderSearchOutput, "tool_call_output", "mcp_tool_call_output":
		return true
	default:
		return false
	}
}

// providerGatewayRepairsToolCallOrderBody 还原「批次内调用相邻、输出紧跟批次」的顺序。
//
// 返回重建后的请求体与被搬动的输出项数量；`ok` 为 false 表示当前请求无法安全重排
// （例如调用项缺少 `call_id`），调用方应当放弃重排、保持原样转发。
func providerGatewayRepairsToolCallOrderBody(body []byte) ([]byte, int, bool) {
	input := gjson.GetBytes(body, "input")
	if !input.IsArray() {
		return body, 0, true
	}
	items := input.Array()
	if len(items) < 3 {
		// 少于三项不可能出现「调用与输出之间夹了东西」。
		return body, 0, true
	}

	outputIndexesByID := make(map[string][]int)
	for index, item := range items {
		itemType := item.Get("type").String()
		callID := strings.TrimSpace(item.Get("call_id").String())
		switch {
		case providerToolOrderIsCallItem(itemType):
			if callID == "" {
				// 没有 call_id 就没法判断归属，交给配对修复处理，这里不猜。
				return body, 0, false
			}
		case providerToolOrderIsOutputItem(itemType):
			if callID == "" {
				return body, 0, false
			}
			outputIndexesByID[callID] = append(outputIndexesByID[callID], index)
		}
	}
	if len(outputIndexesByID) == 0 {
		return body, 0, true
	}

	rebuilt := make([]string, 0, len(items))
	newPosition := make([]int, len(items))
	emitted := make([]bool, len(items))
	for index := 0; index < len(items); index++ {
		item := items[index]
		if emitted[index] {
			continue
		}
		itemType := item.Get("type").String()
		if !providerToolOrderIsCallItem(itemType) {
			newPosition[index] = len(rebuilt)
			emitted[index] = true
			rebuilt = append(rebuilt, item.Raw)
			continue
		}

		// 连续的调用属于同一条 assistant 回合：整体保留相邻，输出放到批次之后。
		batchEnd := index
		for batchEnd+1 < len(items) && providerToolOrderIsCallItem(items[batchEnd+1].Get("type").String()) {
			batchEnd++
		}
		outputs := make([]int, 0)
		collected := make(map[int]bool)
		for cursor := index; cursor <= batchEnd; cursor++ {
			callID := strings.TrimSpace(items[cursor].Get("call_id").String())
			for _, outputIndex := range outputIndexesByID[callID] {
				// 排在调用之前的输出属于历史损坏，不在这里搬动。
				if outputIndex <= cursor || emitted[outputIndex] || collected[outputIndex] {
					continue
				}
				collected[outputIndex] = true
				outputs = append(outputs, outputIndex)
			}
		}
		// 保持输出原有的相对顺序，尽量少改字节。
		sort.Ints(outputs)
		for cursor := index; cursor <= batchEnd; cursor++ {
			newPosition[cursor] = len(rebuilt)
			emitted[cursor] = true
			rebuilt = append(rebuilt, items[cursor].Raw)
		}
		for _, outputIndex := range outputs {
			newPosition[outputIndex] = len(rebuilt)
			emitted[outputIndex] = true
			rebuilt = append(rebuilt, items[outputIndex].Raw)
		}
		index = batchEnd
	}

	relocated := 0
	for index, item := range items {
		if !providerToolOrderIsOutputItem(item.Get("type").String()) {
			continue
		}
		if newPosition[index] != index {
			relocated++
		}
	}
	if relocated == 0 {
		// 已经是合法形状：逐字节透传，避免同一份历史因为重排丢掉提示词缓存。
		return body, 0, true
	}

	updated, err := sjson.SetRawBytes(body, "input", []byte("["+strings.Join(rebuilt, ",")+"]"))
	if err != nil {
		return body, 0, false
	}
	if !providerToolOrderBatchesAreAdjacent(gjson.GetBytes(updated, "input")) {
		// 重排后仍不满足「批次内调用相邻、输出紧跟批次」，放弃改动，避免把请求改坏。
		return body, 0, false
	}
	return updated, relocated, true
}

// providerToolOrderBatchesAreAdjacent 校验每个连续调用批次后面紧跟这批调用的输出。
func providerToolOrderBatchesAreAdjacent(input gjson.Result) bool {
	if !input.IsArray() {
		return true
	}
	items := input.Array()
	outputCountByID := make(map[string]int)
	for _, item := range items {
		if !providerToolOrderIsOutputItem(item.Get("type").String()) {
			continue
		}
		callID := strings.TrimSpace(item.Get("call_id").String())
		if callID == "" {
			continue
		}
		outputCountByID[callID]++
	}
	for index := 0; index < len(items); index++ {
		item := items[index]
		if !providerToolOrderIsCallItem(item.Get("type").String()) {
			continue
		}
		batchEnd := index
		for batchEnd+1 < len(items) && providerToolOrderIsCallItem(items[batchEnd+1].Get("type").String()) {
			batchEnd++
		}
		expected := make(map[string]int)
		for cursor := index; cursor <= batchEnd; cursor++ {
			callID := strings.TrimSpace(items[cursor].Get("call_id").String())
			if callID == "" {
				return false
			}
			if count := outputCountByID[callID]; count > 0 {
				expected[callID] += count
			}
		}
		actual := make(map[string]int)
		position := batchEnd + 1
		for ; position < len(items) && providerToolOrderIsOutputItem(items[position].Get("type").String()); position++ {
			callID := strings.TrimSpace(items[position].Get("call_id").String())
			actual[callID]++
		}
		if len(actual) != len(expected) {
			return false
		}
		for callID, count := range expected {
			if actual[callID] != count {
				return false
			}
		}
		index = batchEnd
	}
	return true
}

// providerGatewayToolOrderDiagnostic 供请求诊断日志使用。
func providerGatewayToolOrderDiagnostic(relocated int) string {
	return fmt.Sprintf("relocated %d displaced tool output item(s)", relocated)
}

// providerGatewaySerializeToolCalls 关闭上游的并行工具调用。
//
// 顺序与配对问题都源自并行批次：Codex 的 app-server 可能在下一次采样请求里抢先带上还没落盘的
// 调用，形成「有 call 无 output」或错位的历史。让上游一次只返回一个调用可以从源头避开这个竞态，
// 顺序还原与配对补齐则作为兜底处理已经落盘的历史。
func providerGatewaySerializeToolCalls(body []byte) []byte {
	updated, err := sjson.SetBytes(body, "parallel_tool_calls", false)
	if err != nil {
		return body
	}
	return updated
}
