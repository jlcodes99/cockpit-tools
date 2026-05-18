package converters

import (
	"bytes"
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

// chatToResponsesState 流式转换状态
type chatToResponsesState struct {
	Seq           int
	ResponseID    string
	CreatedAt     int64
	CurrentMsgID  string
	CurrentFCID   string
	InTextBlock   bool
	InFuncBlock   bool
	FuncArgsBuf   map[int]*strings.Builder // index -> args
	FuncNames     map[int]string           // index -> function name
	FuncCallIDs   map[int]string           // index -> call id
	FuncItemAdded map[int]bool             // index -> whether output_item.added has been emitted
	TextBuf       strings.Builder
	// reasoning state
	ReasoningActive    bool
	ReasoningItemID    string
	ReasoningBuf       strings.Builder
	ReasoningPartAdded bool
	ReasoningIndex     int
	// usage（完整支持详细字段，参考 claude-code-hub）
	InputTokens             int64
	InputTokensIncludeCache bool // OpenAI prompt_tokens 口径，已包含 cached tokens
	OutputTokens            int64
	TotalTokens             int64
	CachedTokens            int64 // input_tokens_details.cached_tokens / cache_read_input_tokens
	ReasoningTokens         int64 // output_tokens_details.reasoning_tokens
	UsageSeen               bool
	// Claude 缓存 TTL 细分
	CacheCreationTokens   int64  // cache_creation_input_tokens
	CacheCreation5mTokens int64  // cache_creation_5m_input_tokens
	CacheCreation1hTokens int64  // cache_creation_1h_input_tokens
	CacheTTL              string // "5m" | "1h" | "mixed"
	HasClaudeCacheFields  bool
	HasCacheDetails       bool
	// 首次消息标记
	FirstChunk             bool
	CodexToolCompatEnabled bool
	CodexCtx               CodexToolContext
	CodexCtxInitialized    bool
}

// isCustomProxy returns true if the tool call at the given index is a Codex custom tool proxy.
func (st *chatToResponsesState) isCustomProxy(idx int) bool {
	name := st.FuncNames[idx]
	if name == "" || !st.CodexCtxInitialized {
		return false
	}
	return st.CodexCtx.IsCustomToolProxy(name)
}

func (st *chatToResponsesState) addToolCallItemIfNeeded(idx int, nextSeq func() int) []string {
	if st.FuncItemAdded[idx] || st.FuncCallIDs[idx] == "" || st.FuncNames[idx] == "" {
		return nil
	}

	callID := st.FuncCallIDs[idx]
	name := st.FuncNames[idx]
	outputIndex := st.customToolOutputIndex(idx)

	var item string
	if st.isCustomProxy(idx) {
		itemID := fmt.Sprintf("ctc_%s", callID)
		originalName := st.CodexCtx.OriginalCustomToolName(name)
		item = `{"type":"response.output_item.added","sequence_number":0,"output_index":0,"item":{"id":"","type":"custom_tool_call","status":"in_progress","call_id":"","name":"","input":""}}`
		item, _ = sjson.Set(item, "item.id", itemID)
		item, _ = sjson.Set(item, "item.name", originalName)
	} else {
		itemID := fmt.Sprintf("fc_%s", callID)
		item = `{"type":"response.output_item.added","sequence_number":0,"output_index":0,"item":{"id":"","type":"function_call","status":"in_progress","arguments":"","call_id":"","name":""}}`
		item, _ = sjson.Set(item, "item.id", itemID)
		displayName, namespace := st.CodexCtx.OpenAINameForFunctionTool(name)
		item, _ = sjson.Set(item, "item.name", displayName)
		if namespace != "" {
			item, _ = sjson.Set(item, "item.namespace", namespace)
		}
	}
	item, _ = sjson.Set(item, "sequence_number", nextSeq())
	item, _ = sjson.Set(item, "output_index", outputIndex)
	item, _ = sjson.Set(item, "item.call_id", callID)
	st.FuncItemAdded[idx] = true
	return []string{emitResponsesEvent("response.output_item.added", item)}
}

var chatDataTag = []byte("data:")

func emitResponsesEvent(event string, payload string) string {
	return fmt.Sprintf("event: %s\ndata: %s\n\n", event, payload)
}

func effectiveCacheCreationTokens(cacheCreation, cacheCreation5m, cacheCreation1h int64) int64 {
	if cacheCreation > 0 {
		return cacheCreation
	}
	return cacheCreation5m + cacheCreation1h
}

func calculateClaudeTotalTokens(inputTokens, outputTokens, cacheReadTokens, cacheCreation, cacheCreation5m, cacheCreation1h int64) int64 {
	return inputTokens + outputTokens + cacheReadTokens + effectiveCacheCreationTokens(cacheCreation, cacheCreation5m, cacheCreation1h)
}

func normalizeInputTokensWithCache(inputTokens, cacheReadTokens, cacheCreation, cacheCreation5m, cacheCreation1h int64) int64 {
	cacheTokens := cacheReadTokens + effectiveCacheCreationTokens(cacheCreation, cacheCreation5m, cacheCreation1h)
	if cacheTokens <= 0 {
		return inputTokens
	}
	normalized := inputTokens - cacheTokens
	if normalized < 0 {
		return 0
	}
	return normalized
}

// ConvertOpenAIChatToResponses 将 OpenAI Chat Completions SSE 转换为 Responses SSE 事件
// ctx: 上下文
// modelName: 模型名称
// originalRequestRawJSON: 原始的 Responses API 请求 JSON（用于回显字段）
// requestRawJSON: 转换后的 Chat Completions 请求 JSON
// rawJSON: OpenAI Chat Completions SSE 行
// param: 状态指针（在多次调用间保持状态）
func ConvertOpenAIChatToResponses(ctx context.Context, modelName string, originalRequestRawJSON, requestRawJSON, rawJSON []byte, param *any) []string {
	if *param == nil {
		*param = &chatToResponsesState{
			FuncArgsBuf:   make(map[int]*strings.Builder),
			FuncNames:     make(map[int]string),
			FuncCallIDs:   make(map[int]string),
			FuncItemAdded: make(map[int]bool),
			FirstChunk:    true,
		}
	}
	st := (*param).(*chatToResponsesState)

	// 期望 `data: {..}` 格式
	if !bytes.HasPrefix(rawJSON, chatDataTag) {
		return []string{}
	}
	rawJSON = bytes.TrimSpace(rawJSON[5:])

	// 检查 [DONE] 标记
	if string(rawJSON) == "[DONE]" {
		// 生成完成事件
		return st.generateCompletedEvents(originalRequestRawJSON)
	}

	root := gjson.ParseBytes(rawJSON)
	var out []string

	nextSeq := func() int { st.Seq++; return st.Seq }

	// 处理首次 chunk - 初始化并生成 response.created 和 response.in_progress
	if st.FirstChunk {
		st.FirstChunk = false
		// 从 chunk 中提取 id
		if id := root.Get("id"); id.Exists() {
			st.ResponseID = id.String()
		} else {
			st.ResponseID = fmt.Sprintf("resp_%d", time.Now().UnixNano())
		}
		st.CreatedAt = time.Now().Unix()

		// 重置状态
		st.TextBuf.Reset()
		st.ReasoningBuf.Reset()
		st.ReasoningActive = false
		st.InTextBlock = false
		st.InFuncBlock = false
		st.CurrentMsgID = ""
		st.CurrentFCID = ""
		st.ReasoningItemID = ""
		st.ReasoningIndex = 0
		st.ReasoningPartAdded = false
		st.FuncArgsBuf = make(map[int]*strings.Builder)
		st.FuncNames = make(map[int]string)
		st.FuncCallIDs = make(map[int]string)
		st.FuncItemAdded = make(map[int]bool)
		st.InputTokens = 0
		st.OutputTokens = 0
		st.CachedTokens = 0
		st.ReasoningTokens = 0
		st.CacheCreationTokens = 0
		st.CacheCreation5mTokens = 0
		st.CacheCreation1hTokens = 0
		st.CacheTTL = ""
		st.UsageSeen = false

		st.ensureCodexToolContext(originalRequestRawJSON)

		// 发送 response.created
		created := `{"type":"response.created","sequence_number":0,"response":{"id":"","object":"response","created_at":0,"status":"in_progress","background":false,"error":null,"instructions":""}}`
		created, _ = sjson.Set(created, "sequence_number", nextSeq())
		created, _ = sjson.Set(created, "response.id", st.ResponseID)
		created, _ = sjson.Set(created, "response.created_at", st.CreatedAt)
		out = append(out, emitResponsesEvent("response.created", created))

		// 发送 response.in_progress
		inprog := `{"type":"response.in_progress","sequence_number":0,"response":{"id":"","object":"response","created_at":0,"status":"in_progress"}}`
		inprog, _ = sjson.Set(inprog, "sequence_number", nextSeq())
		inprog, _ = sjson.Set(inprog, "response.id", st.ResponseID)
		inprog, _ = sjson.Set(inprog, "response.created_at", st.CreatedAt)
		out = append(out, emitResponsesEvent("response.in_progress", inprog))
	}

	// 解析 choices
	choices := root.Get("choices")
	if !choices.Exists() || !choices.IsArray() {
		return out
	}

	for _, choice := range choices.Array() {
		delta := choice.Get("delta")
		if !delta.Exists() {
			continue
		}

		finishReason := choice.Get("finish_reason").String()

		// 处理 reasoning_content（OpenAI o1 模型的 reasoning）
		if reasoning := delta.Get("reasoning_content"); reasoning.Exists() && reasoning.String() != "" {
			reasoningText := reasoning.String()

			// 开始 reasoning block
			if !st.ReasoningActive {
				st.ReasoningActive = true
				st.ReasoningIndex = 0
				st.ReasoningBuf.Reset()
				st.ReasoningItemID = fmt.Sprintf("rs_%s_0", st.ResponseID)

				// response.output_item.added for reasoning
				item := `{"type":"response.output_item.added","sequence_number":0,"output_index":0,"item":{"id":"","type":"reasoning","status":"in_progress","summary":[]}}`
				item, _ = sjson.Set(item, "sequence_number", nextSeq())
				item, _ = sjson.Set(item, "output_index", st.ReasoningIndex)
				item, _ = sjson.Set(item, "item.id", st.ReasoningItemID)
				out = append(out, emitResponsesEvent("response.output_item.added", item))

				// response.reasoning_summary_part.added
				part := `{"type":"response.reasoning_summary_part.added","sequence_number":0,"item_id":"","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}`
				part, _ = sjson.Set(part, "sequence_number", nextSeq())
				part, _ = sjson.Set(part, "item_id", st.ReasoningItemID)
				part, _ = sjson.Set(part, "output_index", st.ReasoningIndex)
				out = append(out, emitResponsesEvent("response.reasoning_summary_part.added", part))
				st.ReasoningPartAdded = true
			}

			// 发送 reasoning delta
			st.ReasoningBuf.WriteString(reasoningText)
			msg := `{"type":"response.reasoning_summary_text.delta","sequence_number":0,"item_id":"","output_index":0,"summary_index":0,"text":""}`
			msg, _ = sjson.Set(msg, "sequence_number", nextSeq())
			msg, _ = sjson.Set(msg, "item_id", st.ReasoningItemID)
			msg, _ = sjson.Set(msg, "output_index", st.ReasoningIndex)
			msg, _ = sjson.Set(msg, "text", reasoningText)
			out = append(out, emitResponsesEvent("response.reasoning_summary_text.delta", msg))
		}

		// 处理 content（文本内容）
		if content := delta.Get("content"); content.Exists() && content.String() != "" {
			contentText := content.String()

			// 如果 reasoning 还在活跃状态，先关闭它
			if st.ReasoningActive {
				out = append(out, st.closeReasoningBlock(nextSeq)...)
			}

			// 开始 text block
			if !st.InTextBlock {
				st.InTextBlock = true
				// 计算 output_index：如果有 reasoning 则为 1，否则为 0
				outputIndex := 0
				if st.ReasoningPartAdded {
					outputIndex = 1
				}
				st.CurrentMsgID = fmt.Sprintf("msg_%s_%d", st.ResponseID, outputIndex)

				// response.output_item.added for message
				item := `{"type":"response.output_item.added","sequence_number":0,"output_index":0,"item":{"id":"","type":"message","status":"in_progress","content":[],"role":"assistant"}}`
				item, _ = sjson.Set(item, "sequence_number", nextSeq())
				item, _ = sjson.Set(item, "output_index", outputIndex)
				item, _ = sjson.Set(item, "item.id", st.CurrentMsgID)
				out = append(out, emitResponsesEvent("response.output_item.added", item))

				// response.content_part.added
				part := `{"type":"response.content_part.added","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":""}}`
				part, _ = sjson.Set(part, "sequence_number", nextSeq())
				part, _ = sjson.Set(part, "item_id", st.CurrentMsgID)
				part, _ = sjson.Set(part, "output_index", outputIndex)
				out = append(out, emitResponsesEvent("response.content_part.added", part))
			}

			// 发送 text delta
			st.TextBuf.WriteString(contentText)
			outputIndex := 0
			if st.ReasoningPartAdded {
				outputIndex = 1
			}
			msg := `{"type":"response.output_text.delta","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"delta":"","logprobs":[]}`
			msg, _ = sjson.Set(msg, "sequence_number", nextSeq())
			msg, _ = sjson.Set(msg, "item_id", st.CurrentMsgID)
			msg, _ = sjson.Set(msg, "output_index", outputIndex)
			msg, _ = sjson.Set(msg, "delta", contentText)
			out = append(out, emitResponsesEvent("response.output_text.delta", msg))
		}

		// 处理 tool_calls
		if toolCalls := delta.Get("tool_calls"); toolCalls.Exists() && toolCalls.IsArray() {
			for _, tc := range toolCalls.Array() {
				idx := int(tc.Get("index").Int())

				// 如果 reasoning 还在活跃状态，先关闭它
				if st.ReasoningActive {
					out = append(out, st.closeReasoningBlock(nextSeq)...)
				}

				// 如果 text block 还在活跃状态，先关闭它
				if st.InTextBlock {
					out = append(out, st.closeTextBlock(nextSeq)...)
				}

				// 初始化 tool call 状态
				if st.FuncArgsBuf[idx] == nil {
					st.FuncArgsBuf[idx] = &strings.Builder{}
				}

				// 处理 tool call ID
				if tcID := tc.Get("id"); tcID.Exists() && tcID.String() != "" {
					st.FuncCallIDs[idx] = tcID.String()
					st.CurrentFCID = tcID.String()

					// 开始新的 tool call item. The concrete item type is emitted
					// after the function name is known.
					st.InFuncBlock = true

					// When codexToolCompat is disabled, emit function_call output_item.added
					// immediately upon receiving tool_call.id, preserving the original event timing.
					if !st.CodexToolCompatEnabled {
						outputIndex := st.customToolOutputIndex(idx)
						item := `{"type":"response.output_item.added","sequence_number":0,"output_index":0,"item":{"id":"","type":"function_call","status":"in_progress","arguments":"","call_id":"","name":""}}`
						item, _ = sjson.Set(item, "sequence_number", nextSeq())
						item, _ = sjson.Set(item, "output_index", outputIndex)
						item, _ = sjson.Set(item, "item.id", fmt.Sprintf("fc_%s", tcID.String()))
						item, _ = sjson.Set(item, "item.call_id", tcID.String())
						st.FuncItemAdded[idx] = true
						out = append(out, emitResponsesEvent("response.output_item.added", item))
					}
				}

				// 处理 function
				if function := tc.Get("function"); function.Exists() {
					// 处理函数名
					if name := function.Get("name"); name.Exists() && name.String() != "" {
						st.FuncNames[idx] = name.String()
					}
					out = append(out, st.addToolCallItemIfNeeded(idx, nextSeq)...)

					// 处理参数
					if args := function.Get("arguments"); args.Exists() && args.String() != "" {
						st.FuncArgsBuf[idx].WriteString(args.String())
						out = append(out, st.addToolCallItemIfNeeded(idx, nextSeq)...)

						if st.isCustomProxy(idx) {
							continue
						}

						// 计算 output_index
						outputIndex := idx
						if st.ReasoningPartAdded {
							outputIndex += 1
						}
						if st.CurrentMsgID != "" {
							outputIndex += 1
						}

						msg := `{"type":"response.function_call_arguments.delta","sequence_number":0,"item_id":"","output_index":0,"delta":""}`
						msg, _ = sjson.Set(msg, "sequence_number", nextSeq())
						msg, _ = sjson.Set(msg, "item_id", fmt.Sprintf("fc_%s", st.FuncCallIDs[idx]))
						msg, _ = sjson.Set(msg, "output_index", outputIndex)
						msg, _ = sjson.Set(msg, "delta", args.String())
						out = append(out, emitResponsesEvent("response.function_call_arguments.delta", msg))
					}
				}
			}
		}

		// 处理 finish_reason
		if finishReason != "" && finishReason != "null" {
			// 关闭所有打开的 blocks
			if st.ReasoningActive {
				out = append(out, st.closeReasoningBlock(nextSeq)...)
			}
			if st.InTextBlock {
				out = append(out, st.closeTextBlock(nextSeq)...)
			}
			if st.InFuncBlock {
				out = append(out, st.closeFuncBlocks(nextSeq)...)
			}
		}
	}

	// 处理 usage（完整支持多格式详细字段，参考 claude-code-hub）
	if usage := root.Get("usage"); usage.Exists() {
		st.UsageSeen = true

		// OpenAI 格式基础字段
		if v := usage.Get("prompt_tokens"); v.Exists() {
			st.InputTokens = v.Int()
			st.InputTokensIncludeCache = true
		}
		if v := usage.Get("completion_tokens"); v.Exists() {
			st.OutputTokens = v.Int()
		}
		if v := usage.Get("total_tokens"); v.Exists() {
			st.TotalTokens = v.Int()
		}

		// OpenAI 格式详细字段
		if v := usage.Get("prompt_tokens_details.cached_tokens"); v.Exists() {
			st.CachedTokens = v.Int()
			st.HasCacheDetails = true
		}
		if v := usage.Get("completion_tokens_details.reasoning_tokens"); v.Exists() {
			st.ReasoningTokens = v.Int()
		}

		// Claude 格式基础字段（优先级高于 OpenAI）
		if v := usage.Get("input_tokens"); v.Exists() {
			st.InputTokens = v.Int()
			st.InputTokensIncludeCache = false
		}
		if v := usage.Get("output_tokens"); v.Exists() {
			st.OutputTokens = v.Int()
		}

		// Claude 格式缓存字段
		if v := usage.Get("cache_read_input_tokens"); v.Exists() {
			st.CachedTokens = v.Int()
			st.HasClaudeCacheFields = true
			st.HasCacheDetails = true
		}
		if v := usage.Get("cache_creation_input_tokens"); v.Exists() {
			st.CacheCreationTokens = v.Int()
			st.HasClaudeCacheFields = true
		}
		if v := usage.Get("cache_creation_5m_input_tokens"); v.Exists() {
			st.CacheCreation5mTokens = v.Int()
			st.HasClaudeCacheFields = true
		}
		if v := usage.Get("cache_creation_1h_input_tokens"); v.Exists() {
			st.CacheCreation1hTokens = v.Int()
			st.HasClaudeCacheFields = true
		}

		// 设置缓存 TTL 标识
		has5m := st.CacheCreation5mTokens > 0
		has1h := st.CacheCreation1hTokens > 0
		if has5m && has1h {
			st.CacheTTL = "mixed"
		} else if has1h {
			st.CacheTTL = "1h"
		} else if has5m {
			st.CacheTTL = "5m"
		}

		// Gemini 格式（自动去重）
		if v := usage.Get("promptTokenCount"); v.Exists() {
			promptTokens := v.Int()
			cachedTokens := usage.Get("cachedContentTokenCount").Int()
			// Gemini 的 promptTokenCount 已包含 cachedContentTokenCount，需要扣除
			actualInput := promptTokens - cachedTokens
			if actualInput < 0 {
				actualInput = 0
			}
			st.InputTokens = actualInput
			st.InputTokensIncludeCache = false
			st.CachedTokens = cachedTokens
			st.HasCacheDetails = cachedTokens > 0
		}
		if v := usage.Get("candidatesTokenCount"); v.Exists() {
			st.OutputTokens = v.Int()
		}
	}

	return out
}

// closeReasoningBlock 关闭 reasoning block
func (st *chatToResponsesState) closeReasoningBlock(nextSeq func() int) []string {
	if !st.ReasoningActive {
		return nil
	}

	var out []string
	full := st.ReasoningBuf.String()

	// response.reasoning_summary_text.done
	textDone := `{"type":"response.reasoning_summary_text.done","sequence_number":0,"item_id":"","output_index":0,"summary_index":0,"text":""}`
	textDone, _ = sjson.Set(textDone, "sequence_number", nextSeq())
	textDone, _ = sjson.Set(textDone, "item_id", st.ReasoningItemID)
	textDone, _ = sjson.Set(textDone, "output_index", st.ReasoningIndex)
	textDone, _ = sjson.Set(textDone, "text", full)
	out = append(out, emitResponsesEvent("response.reasoning_summary_text.done", textDone))

	// response.reasoning_summary_part.done
	partDone := `{"type":"response.reasoning_summary_part.done","sequence_number":0,"item_id":"","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}`
	partDone, _ = sjson.Set(partDone, "sequence_number", nextSeq())
	partDone, _ = sjson.Set(partDone, "item_id", st.ReasoningItemID)
	partDone, _ = sjson.Set(partDone, "output_index", st.ReasoningIndex)
	partDone, _ = sjson.Set(partDone, "part.text", full)
	out = append(out, emitResponsesEvent("response.reasoning_summary_part.done", partDone))

	// response.output_item.done for reasoning
	itemDone := `{"type":"response.output_item.done","sequence_number":0,"output_index":0,"item":{"id":"","type":"reasoning","status":"completed","summary":[]}}`
	itemDone, _ = sjson.Set(itemDone, "sequence_number", nextSeq())
	itemDone, _ = sjson.Set(itemDone, "output_index", st.ReasoningIndex)
	itemDone, _ = sjson.Set(itemDone, "item.id", st.ReasoningItemID)
	itemDone, _ = sjson.Set(itemDone, "item.summary", []interface{}{map[string]interface{}{"type": "summary_text", "text": full}})
	out = append(out, emitResponsesEvent("response.output_item.done", itemDone))

	st.ReasoningActive = false
	return out
}

// closeTextBlock 关闭 text block
func (st *chatToResponsesState) closeTextBlock(nextSeq func() int) []string {
	if !st.InTextBlock {
		return nil
	}

	var out []string
	outputIndex := 0
	if st.ReasoningPartAdded {
		outputIndex = 1
	}

	// response.output_text.done
	done := `{"type":"response.output_text.done","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"text":"","logprobs":[]}`
	done, _ = sjson.Set(done, "sequence_number", nextSeq())
	done, _ = sjson.Set(done, "item_id", st.CurrentMsgID)
	done, _ = sjson.Set(done, "output_index", outputIndex)
	done, _ = sjson.Set(done, "text", st.TextBuf.String())
	out = append(out, emitResponsesEvent("response.output_text.done", done))

	// response.content_part.done
	partDone := `{"type":"response.content_part.done","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":""}}`
	partDone, _ = sjson.Set(partDone, "sequence_number", nextSeq())
	partDone, _ = sjson.Set(partDone, "item_id", st.CurrentMsgID)
	partDone, _ = sjson.Set(partDone, "output_index", outputIndex)
	partDone, _ = sjson.Set(partDone, "part.text", st.TextBuf.String())
	out = append(out, emitResponsesEvent("response.content_part.done", partDone))

	// response.output_item.done for message
	final := `{"type":"response.output_item.done","sequence_number":0,"output_index":0,"item":{"id":"","type":"message","status":"completed","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":""}],"role":"assistant"}}`
	final, _ = sjson.Set(final, "sequence_number", nextSeq())
	final, _ = sjson.Set(final, "output_index", outputIndex)
	final, _ = sjson.Set(final, "item.id", st.CurrentMsgID)
	final, _ = sjson.Set(final, "item.content.0.text", st.TextBuf.String())
	out = append(out, emitResponsesEvent("response.output_item.done", final))

	st.InTextBlock = false
	return out
}

// closeFuncBlocks 关闭所有 function call blocks
func (st *chatToResponsesState) closeFuncBlocks(nextSeq func() int) []string {
	if !st.InFuncBlock || len(st.FuncArgsBuf) == 0 {
		return nil
	}

	var out []string

	// 收集并排序索引
	idxs := make([]int, 0, len(st.FuncArgsBuf))
	for idx := range st.FuncArgsBuf {
		idxs = append(idxs, idx)
	}
	// 简单排序
	for i := 0; i < len(idxs); i++ {
		for j := i + 1; j < len(idxs); j++ {
			if idxs[j] < idxs[i] {
				idxs[i], idxs[j] = idxs[j], idxs[i]
			}
		}
	}

	for _, idx := range idxs {
		args := "{}"
		if buf := st.FuncArgsBuf[idx]; buf != nil && buf.Len() > 0 {
			args = buf.String()
		}
		callID := st.FuncCallIDs[idx]
		name := st.FuncNames[idx]

		// 计算 output_index
		outputIndex := st.customToolOutputIndex(idx)

		if st.isCustomProxy(idx) {
			customInput := ReconstructCustomToolCallInput(st.CodexCtx, name, args)
			originalName := st.CodexCtx.OriginalCustomToolName(name)
			itemID := fmt.Sprintf("ctc_%s", callID)

			ctcDelta := `{"type":"response.custom_tool_call_input.delta","sequence_number":0,"item_id":"","call_id":"","output_index":0,"delta":""}`
			ctcDelta, _ = sjson.Set(ctcDelta, "sequence_number", nextSeq())
			ctcDelta, _ = sjson.Set(ctcDelta, "item_id", itemID)
			ctcDelta, _ = sjson.Set(ctcDelta, "call_id", callID)
			ctcDelta, _ = sjson.Set(ctcDelta, "output_index", outputIndex)
			ctcDelta, _ = sjson.Set(ctcDelta, "delta", customInput)
			out = append(out, emitResponsesEvent("response.custom_tool_call_input.delta", ctcDelta))

			itemDone := `{"type":"response.output_item.done","sequence_number":0,"output_index":0,"item":{"id":"","type":"custom_tool_call","status":"completed","call_id":"","name":"","input":""}}`
			itemDone, _ = sjson.Set(itemDone, "sequence_number", nextSeq())
			itemDone, _ = sjson.Set(itemDone, "output_index", outputIndex)
			itemDone, _ = sjson.Set(itemDone, "item.id", itemID)
			itemDone, _ = sjson.Set(itemDone, "item.call_id", callID)
			itemDone, _ = sjson.Set(itemDone, "item.name", originalName)
			itemDone, _ = sjson.Set(itemDone, "item.input", customInput)
			out = append(out, emitResponsesEvent("response.output_item.done", itemDone))
			continue
		}

		// response.function_call_arguments.done
		fcDone := `{"type":"response.function_call_arguments.done","sequence_number":0,"item_id":"","output_index":0,"arguments":""}`
		fcDone, _ = sjson.Set(fcDone, "sequence_number", nextSeq())
		fcDone, _ = sjson.Set(fcDone, "item_id", fmt.Sprintf("fc_%s", callID))
		fcDone, _ = sjson.Set(fcDone, "output_index", outputIndex)
		fcDone, _ = sjson.Set(fcDone, "arguments", args)
		out = append(out, emitResponsesEvent("response.function_call_arguments.done", fcDone))

		// response.output_item.done for function_call
		itemDone := `{"type":"response.output_item.done","sequence_number":0,"output_index":0,"item":{"id":"","type":"function_call","status":"completed","arguments":"","call_id":"","name":""}}`
		itemDone, _ = sjson.Set(itemDone, "sequence_number", nextSeq())
		itemDone, _ = sjson.Set(itemDone, "output_index", outputIndex)
		itemDone, _ = sjson.Set(itemDone, "item.id", fmt.Sprintf("fc_%s", callID))
		itemDone, _ = sjson.Set(itemDone, "item.arguments", args)
		itemDone, _ = sjson.Set(itemDone, "item.call_id", callID)
		displayName, namespace := st.CodexCtx.OpenAINameForFunctionTool(name)
		itemDone, _ = sjson.Set(itemDone, "item.name", displayName)
		if namespace != "" {
			itemDone, _ = sjson.Set(itemDone, "item.namespace", namespace)
		}
		out = append(out, emitResponsesEvent("response.output_item.done", itemDone))
	}

	st.InFuncBlock = false
	return out
}

// generateCompletedEvents 生成完成事件
func (st *chatToResponsesState) generateCompletedEvents(originalRequestRawJSON []byte) []string {
	var out []string
	nextSeq := func() int { st.Seq++; return st.Seq }

	// 先关闭所有打开的 blocks
	if st.ReasoningActive {
		out = append(out, st.closeReasoningBlock(nextSeq)...)
	}
	if st.InTextBlock {
		out = append(out, st.closeTextBlock(nextSeq)...)
	}
	if st.InFuncBlock {
		out = append(out, st.closeFuncBlocks(nextSeq)...)
	}

	// 构建 response.completed
	completed := `{"type":"response.completed","sequence_number":0,"response":{"id":"","object":"response","created_at":0,"status":"completed","background":false,"error":null}}`
	completed, _ = sjson.Set(completed, "sequence_number", nextSeq())
	completed, _ = sjson.Set(completed, "response.id", st.ResponseID)
	completed, _ = sjson.Set(completed, "response.created_at", st.CreatedAt)

	// 注入原始请求字段
	if originalRequestRawJSON != nil {
		req := gjson.ParseBytes(originalRequestRawJSON)
		if v := req.Get("instructions"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.instructions", v.String())
		}
		if v := req.Get("max_output_tokens"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.max_output_tokens", v.Int())
		}
		if v := req.Get("model"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.model", v.String())
		}
		if v := req.Get("parallel_tool_calls"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.parallel_tool_calls", v.Bool())
		}
		if v := req.Get("previous_response_id"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.previous_response_id", v.String())
		}
		if v := req.Get("reasoning"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.reasoning", v.Value())
		}
		if v := req.Get("temperature"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.temperature", v.Float())
		}
		if v := req.Get("tool_choice"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.tool_choice", v.Value())
		}
		if v := req.Get("tools"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.tools", v.Value())
		}
		if v := req.Get("top_p"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.top_p", v.Float())
		}
		if v := req.Get("metadata"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.metadata", v.Value())
		}
	}

	// 构建 output 数组
	var outputs []interface{}

	// reasoning item（如果有）
	if st.ReasoningBuf.Len() > 0 || st.ReasoningPartAdded {
		r := map[string]interface{}{
			"id":     st.ReasoningItemID,
			"type":   "reasoning",
			"status": "completed",
			"summary": []interface{}{map[string]interface{}{
				"type": "summary_text",
				"text": st.ReasoningBuf.String(),
			}},
		}
		outputs = append(outputs, r)
	}

	// message item（如果有文本）
	if st.TextBuf.Len() > 0 || st.CurrentMsgID != "" {
		m := map[string]interface{}{
			"id":     st.CurrentMsgID,
			"type":   "message",
			"status": "completed",
			"content": []interface{}{map[string]interface{}{
				"type":        "output_text",
				"annotations": []interface{}{},
				"logprobs":    []interface{}{},
				"text":        st.TextBuf.String(),
			}},
			"role": "assistant",
		}
		outputs = append(outputs, m)
	}

	// function_call items
	if len(st.FuncArgsBuf) > 0 {
		idxs := make([]int, 0, len(st.FuncArgsBuf))
		for idx := range st.FuncArgsBuf {
			idxs = append(idxs, idx)
		}
		for i := 0; i < len(idxs); i++ {
			for j := i + 1; j < len(idxs); j++ {
				if idxs[j] < idxs[i] {
					idxs[i], idxs[j] = idxs[j], idxs[i]
				}
			}
		}
		for _, idx := range idxs {
			args := ""
			if b := st.FuncArgsBuf[idx]; b != nil {
				args = b.String()
			}
			if args == "" {
				args = "{}"
			}
			callID := st.FuncCallIDs[idx]
			name := st.FuncNames[idx]
			if st.isCustomProxy(idx) {
				customInput := ReconstructCustomToolCallInput(st.CodexCtx, name, args)
				originalName := st.CodexCtx.OriginalCustomToolName(name)
				item := map[string]interface{}{
					"id":      fmt.Sprintf("ctc_%s", callID),
					"type":    "custom_tool_call",
					"status":  "completed",
					"call_id": callID,
					"name":    originalName,
					"input":   customInput,
				}
				outputs = append(outputs, item)
				continue
			}
			displayName, namespace := st.CodexCtx.OpenAINameForFunctionTool(name)
			item := map[string]interface{}{
				"id":        fmt.Sprintf("fc_%s", callID),
				"type":      "function_call",
				"status":    "completed",
				"arguments": args,
				"call_id":   callID,
				"name":      displayName,
			}
			if namespace != "" {
				item["namespace"] = namespace
			}
			outputs = append(outputs, item)
		}
	}

	if len(outputs) > 0 {
		completed, _ = sjson.Set(completed, "response.output", outputs)
	}

	// 添加 usage（完整支持多格式详细字段，参考 claude-code-hub）
	reasoningTokens := st.ReasoningTokens
	if reasoningTokens == 0 && st.ReasoningBuf.Len() > 0 {
		reasoningTokens = int64(st.ReasoningBuf.Len() / 4)
	}

	inputTokens := st.InputTokens
	if st.InputTokensIncludeCache {
		inputTokens = normalizeInputTokensWithCache(
			st.InputTokens,
			st.CachedTokens,
			st.CacheCreationTokens,
			st.CacheCreation5mTokens,
			st.CacheCreation1hTokens,
		)
	}

	// 始终添加基础 usage 字段，即使值为 0
	// 这样 handler 可以检测到 usage 存在，并在需要时用本地估算值替换 0 值
	// 参见 handler.go 中的 patchResponsesCompletedEventUsage 和 injectResponsesUsageToCompletedEvent
	completed, _ = sjson.Set(completed, "response.usage.input_tokens", inputTokens)
	completed, _ = sjson.Set(completed, "response.usage.output_tokens", st.OutputTokens)
	total := st.TotalTokens
	if total == 0 || st.CachedTokens > 0 || effectiveCacheCreationTokens(st.CacheCreationTokens, st.CacheCreation5mTokens, st.CacheCreation1hTokens) > 0 {
		total = calculateClaudeTotalTokens(
			inputTokens,
			st.OutputTokens,
			st.CachedTokens,
			st.CacheCreationTokens,
			st.CacheCreation5mTokens,
			st.CacheCreation1hTokens,
		)
	}
	completed, _ = sjson.Set(completed, "response.usage.total_tokens", total)

	// 可选的详情字段，仅在有值时添加
	// input_tokens_details
	if !st.HasClaudeCacheFields && st.HasCacheDetails && st.CachedTokens > 0 {
		completed, _ = sjson.Set(completed, "response.usage.input_tokens_details.cached_tokens", st.CachedTokens)
	}

	// output_tokens_details
	if reasoningTokens > 0 {
		completed, _ = sjson.Set(completed, "response.usage.output_tokens_details.reasoning_tokens", reasoningTokens)
	}

	// Claude 缓存 TTL 细分字段
	if st.CacheCreationTokens > 0 {
		completed, _ = sjson.Set(completed, "response.usage.cache_creation_input_tokens", st.CacheCreationTokens)
	}
	if st.CacheCreation5mTokens > 0 {
		completed, _ = sjson.Set(completed, "response.usage.cache_creation_5m_input_tokens", st.CacheCreation5mTokens)
	}
	if st.CacheCreation1hTokens > 0 {
		completed, _ = sjson.Set(completed, "response.usage.cache_creation_1h_input_tokens", st.CacheCreation1hTokens)
	}
	if st.HasClaudeCacheFields && st.CachedTokens > 0 {
		completed, _ = sjson.Set(completed, "response.usage.cache_read_input_tokens", st.CachedTokens)
	}
	if st.CacheTTL != "" {
		completed, _ = sjson.Set(completed, "response.usage.cache_ttl", st.CacheTTL)
	}

	out = append(out, emitResponsesEvent("response.completed", completed))
	return out
}

// SynthesizeResponsesCompleted 在流异常结束（上游未发送终止符）时合成 response.completed 事件。
// 根据 upstreamType 从 converterState 中提取已累积的状态来构建完整的完成事件。
// lastSeq 用于在无法从 converter state 获取序列号时，确保生成的事件序列号单调递增。
func SynthesizeResponsesCompleted(originalRequestRawJSON []byte, converterState *any, upstreamType string, lastSeq int) []string {
	if converterState != nil && *converterState != nil {
		switch upstreamType {
		case "claude":
			if st, ok := (*converterState).(*claudeToResponsesState); ok {
				nextSeq := func() int { st.Seq++; return st.Seq }
				var out []string
				if st.ActiveItemType == "reasoning" {
					out = append(out, st.closeReasoning(nextSeq)...)
				}
				if st.ActiveItemType == "text" {
					out = append(out, st.closeText(nextSeq)...)
				}
				if st.ActiveItemType == "tool_use" {
					out = append(out, st.closeToolUse(nextSeq)...)
				}
				out = append(out, st.completedEvent("", originalRequestRawJSON, nextSeq))
				return out
			}
		case "gemini":
			if st, ok := (*converterState).(*geminiToResponsesStreamState); ok {
				nextSeq := func() int { st.Seq++; return st.Seq }
				var out []string
				if st.InReasoningBlock {
					out = append(out, st.closeGeminiReasoningBlock(nextSeq)...)
				}
				if st.InTextBlock {
					out = append(out, st.closeGeminiTextBlock(nextSeq)...)
				}
				out = append(out, st.generateCompletedEvent(originalRequestRawJSON, "STOP")...)
				return out
			}
		default:
			if st, ok := (*converterState).(*chatToResponsesState); ok {
				return st.generateCompletedEvents(originalRequestRawJSON)
			}
		}
	}

	// 最终兜底：合成最小 response.completed，使用 lastSeq+1 确保序列号单调递增
	seq := lastSeq + 1
	completed := fmt.Sprintf(`{"type":"response.completed","sequence_number":%d,"response":{"id":"","object":"response","created_at":0,"status":"completed","background":false,"error":null}}`, seq)
	return []string{emitResponsesEvent("response.completed", completed)}
}

// ConvertOpenAIChatToResponsesNonStream 将 OpenAI Chat Completions 响应转换为 Responses 格式（非流式）
func ConvertOpenAIChatToResponsesNonStream(_ context.Context, _ string, originalRequestRawJSON, requestRawJSON, rawJSON []byte, _ *any) string {
	root := gjson.ParseBytes(rawJSON)

	// 基础响应模板
	out := `{"id":"","object":"response","created_at":0,"status":"completed","background":false,"error":null,"incomplete_details":null,"output":[],"usage":{"input_tokens":0,"input_tokens_details":{"cached_tokens":0},"output_tokens":0,"output_tokens_details":{},"total_tokens":0}}`

	// 提取基本字段
	responseID := root.Get("id").String()
	if responseID == "" {
		responseID = fmt.Sprintf("resp_%d", time.Now().UnixNano())
	}
	createdAt := root.Get("created").Int()
	if createdAt == 0 {
		createdAt = time.Now().Unix()
	}

	out, _ = sjson.Set(out, "id", responseID)
	out, _ = sjson.Set(out, "created_at", createdAt)

	// 注入原始请求字段
	if originalRequestRawJSON != nil {
		req := gjson.ParseBytes(originalRequestRawJSON)
		if v := req.Get("instructions"); v.Exists() {
			out, _ = sjson.Set(out, "instructions", v.String())
		}
		if v := req.Get("max_output_tokens"); v.Exists() {
			out, _ = sjson.Set(out, "max_output_tokens", v.Int())
		}
		if v := req.Get("model"); v.Exists() {
			out, _ = sjson.Set(out, "model", v.String())
		}
		if v := req.Get("parallel_tool_calls"); v.Exists() {
			out, _ = sjson.Set(out, "parallel_tool_calls", v.Bool())
		}
		if v := req.Get("previous_response_id"); v.Exists() {
			out, _ = sjson.Set(out, "previous_response_id", v.String())
		}
		if v := req.Get("reasoning"); v.Exists() {
			out, _ = sjson.Set(out, "reasoning", v.Value())
		}
		if v := req.Get("temperature"); v.Exists() {
			out, _ = sjson.Set(out, "temperature", v.Float())
		}
		if v := req.Get("tool_choice"); v.Exists() {
			out, _ = sjson.Set(out, "tool_choice", v.Value())
		}
		if v := req.Get("tools"); v.Exists() {
			out, _ = sjson.Set(out, "tools", v.Value())
		}
		if v := req.Get("top_p"); v.Exists() {
			out, _ = sjson.Set(out, "top_p", v.Float())
		}
		if v := req.Get("metadata"); v.Exists() {
			out, _ = sjson.Set(out, "metadata", v.Value())
		}
	}

	// 解析 choices
	choices := root.Get("choices")
	if !choices.Exists() || !choices.IsArray() || len(choices.Array()) == 0 {
		return out
	}

	codexCtx := buildCodexToolContextFromRequest(originalRequestRawJSON)
	var outputs []interface{}
	var textBuf strings.Builder
	var reasoningBuf strings.Builder
	currentMsgID := fmt.Sprintf("msg_%s_0", responseID)

	for _, choice := range choices.Array() {
		message := choice.Get("message")
		if !message.Exists() {
			continue
		}

		// 处理 reasoning_content
		if reasoning := message.Get("reasoning_content"); reasoning.Exists() && reasoning.String() != "" {
			reasoningBuf.WriteString(reasoning.String())
		}

		// 处理 content
		if content := message.Get("content"); content.Exists() && content.String() != "" {
			textBuf.WriteString(content.String())
		}

		// 处理 tool_calls
		if toolCalls := message.Get("tool_calls"); toolCalls.Exists() && toolCalls.IsArray() {
			for _, tc := range toolCalls.Array() {
				callID := tc.Get("id").String()
				funcName := tc.Get("function.name").String()
				funcArgs := tc.Get("function.arguments").String()
				if funcArgs == "" {
					funcArgs = "{}"
				}

				if codexCtx.IsCustomToolProxy(funcName) {
					customInput := ReconstructCustomToolCallInput(codexCtx, funcName, funcArgs)
					originalName := codexCtx.OriginalCustomToolName(funcName)
					item := map[string]interface{}{
						"id":      fmt.Sprintf("ctc_%s", callID),
						"type":    "custom_tool_call",
						"status":  "completed",
						"call_id": callID,
						"name":    originalName,
						"input":   customInput,
					}
					outputs = append(outputs, item)
					continue
				}

				displayName, namespace := codexCtx.OpenAINameForFunctionTool(funcName)
				item := map[string]interface{}{
					"id":        fmt.Sprintf("fc_%s", callID),
					"type":      "function_call",
					"status":    "completed",
					"arguments": funcArgs,
					"call_id":   callID,
					"name":      displayName,
				}
				if namespace != "" {
					item["namespace"] = namespace
				}
				outputs = append(outputs, item)
			}
		}
	}

	// 构建 output 数组
	outputIndex := 0

	// reasoning item（如果有）
	if reasoningBuf.Len() > 0 {
		reasoningItemID := fmt.Sprintf("rs_%s_0", responseID)
		r := map[string]interface{}{
			"id":     reasoningItemID,
			"type":   "reasoning",
			"status": "completed",
			"summary": []interface{}{map[string]interface{}{
				"type": "summary_text",
				"text": reasoningBuf.String(),
			}},
		}
		// 在开头插入
		outputs = append([]interface{}{r}, outputs...)
		outputIndex = 1
		currentMsgID = fmt.Sprintf("msg_%s_%d", responseID, outputIndex)
	}

	// message item（如果有文本）
	if textBuf.Len() > 0 {
		m := map[string]interface{}{
			"id":     currentMsgID,
			"type":   "message",
			"status": "completed",
			"content": []interface{}{map[string]interface{}{
				"type":        "output_text",
				"annotations": []interface{}{},
				"logprobs":    []interface{}{},
				"text":        textBuf.String(),
			}},
			"role": "assistant",
		}
		// 在 reasoning 之后，tool_calls 之前插入
		if outputIndex > 0 {
			// 有 reasoning，插入到位置 1
			newOutputs := make([]interface{}, 0, len(outputs)+1)
			newOutputs = append(newOutputs, outputs[0]) // reasoning
			newOutputs = append(newOutputs, m)          // message
			newOutputs = append(newOutputs, outputs[1:]...)
			outputs = newOutputs
		} else {
			// 没有 reasoning，插入到开头
			outputs = append([]interface{}{m}, outputs...)
		}
	}

	if len(outputs) > 0 {
		out, _ = sjson.Set(out, "output", outputs)
	}

	// 处理 usage（完整支持多格式详细字段，参考 claude-code-hub）
	if usage := root.Get("usage"); usage.Exists() {
		var inputTokens, outputTokens, totalTokens, cachedTokens int64
		var cacheCreation, cacheCreation5m, cacheCreation1h int64
		var cacheTTL string
		hasClaudeCacheFields := false
		hasCacheDetails := false
		inputTokensIncludeCache := false

		// OpenAI 格式
		if v := usage.Get("prompt_tokens"); v.Exists() {
			inputTokens = v.Int()
			inputTokensIncludeCache = true
		}
		if v := usage.Get("completion_tokens"); v.Exists() {
			outputTokens = v.Int()
		}
		if v := usage.Get("total_tokens"); v.Exists() {
			totalTokens = v.Int()
		}
		if v := usage.Get("prompt_tokens_details.cached_tokens"); v.Exists() {
			cachedTokens = v.Int()
			hasCacheDetails = true
		}
		reasoningTokensFromUsage := usage.Get("completion_tokens_details.reasoning_tokens").Int()

		// Claude 格式（优先级高于 OpenAI）
		if v := usage.Get("input_tokens"); v.Exists() {
			inputTokens = v.Int()
			inputTokensIncludeCache = false
		}
		if v := usage.Get("output_tokens"); v.Exists() {
			outputTokens = v.Int()
		}
		if v := usage.Get("cache_read_input_tokens"); v.Exists() {
			hasClaudeCacheFields = true
			cachedTokens = v.Int()
			hasCacheDetails = true
		}
		if v := usage.Get("cache_creation_input_tokens"); v.Exists() {
			hasClaudeCacheFields = true
			cacheCreation = v.Int()
		}
		if v := usage.Get("cache_creation_5m_input_tokens"); v.Exists() {
			hasClaudeCacheFields = true
			cacheCreation5m = v.Int()
		}
		if v := usage.Get("cache_creation_1h_input_tokens"); v.Exists() {
			hasClaudeCacheFields = true
			cacheCreation1h = v.Int()
		}

		// 设置缓存 TTL 标识
		if cacheCreation5m > 0 && cacheCreation1h > 0 {
			cacheTTL = "mixed"
		} else if cacheCreation1h > 0 {
			cacheTTL = "1h"
		} else if cacheCreation5m > 0 {
			cacheTTL = "5m"
		}

		// Gemini 格式（自动去重）
		if v := usage.Get("promptTokenCount"); v.Exists() {
			promptTokens := v.Int()
			geminiCached := usage.Get("cachedContentTokenCount").Int()
			// Gemini 的 promptTokenCount 已包含 cachedContentTokenCount，需要扣除
			actualInput := promptTokens - geminiCached
			if actualInput < 0 {
				actualInput = 0
			}
			inputTokens = actualInput
			inputTokensIncludeCache = false
			cachedTokens = geminiCached
			hasCacheDetails = geminiCached > 0
		}
		if v := usage.Get("candidatesTokenCount"); v.Exists() {
			outputTokens = v.Int()
		}

		usageInputTokens := inputTokens
		if inputTokensIncludeCache {
			usageInputTokens = normalizeInputTokensWithCache(
				inputTokens,
				cachedTokens,
				cacheCreation,
				cacheCreation5m,
				cacheCreation1h,
			)
		}

		// 计算总量
		if totalTokens == 0 || cachedTokens > 0 || effectiveCacheCreationTokens(cacheCreation, cacheCreation5m, cacheCreation1h) > 0 {
			totalTokens = calculateClaudeTotalTokens(
				usageInputTokens,
				outputTokens,
				cachedTokens,
				cacheCreation,
				cacheCreation5m,
				cacheCreation1h,
			)
		}

		// 写入基础字段
		out, _ = sjson.Set(out, "usage.input_tokens", usageInputTokens)
		out, _ = sjson.Set(out, "usage.output_tokens", outputTokens)
		out, _ = sjson.Set(out, "usage.total_tokens", totalTokens)

		// input_tokens_details
		if !hasClaudeCacheFields && hasCacheDetails && cachedTokens > 0 {
			out, _ = sjson.Set(out, "usage.input_tokens_details.cached_tokens", cachedTokens)
		}

		// output_tokens_details
		reasoningTokens := reasoningTokensFromUsage
		if reasoningTokens == 0 && reasoningBuf.Len() > 0 {
			reasoningTokens = int64(reasoningBuf.Len() / 4)
		}
		if reasoningTokens > 0 {
			out, _ = sjson.Set(out, "usage.output_tokens_details.reasoning_tokens", reasoningTokens)
		}

		// Claude 缓存 TTL 细分字段
		if cacheCreation > 0 {
			out, _ = sjson.Set(out, "usage.cache_creation_input_tokens", cacheCreation)
		}
		if cacheCreation5m > 0 {
			out, _ = sjson.Set(out, "usage.cache_creation_5m_input_tokens", cacheCreation5m)
		}
		if cacheCreation1h > 0 {
			out, _ = sjson.Set(out, "usage.cache_creation_1h_input_tokens", cacheCreation1h)
		}
		if hasClaudeCacheFields && cachedTokens > 0 {
			out, _ = sjson.Set(out, "usage.cache_read_input_tokens", cachedTokens)
		}
		if cacheTTL != "" {
			out, _ = sjson.Set(out, "usage.cache_ttl", cacheTTL)
		}
	}

	return out
}
