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

type claudeToResponsesState struct {
	Seq                int
	ResponseID         string
	CreatedAt          int64
	FirstChunk         bool
	NextOutputIndex    int
	ActiveItemType     string
	CurrentMsgID       string
	CurrentReasoningID string
	CurrentToolItemID  string
	CurrentToolCallID  string
	CurrentToolName    string
	ReasoningIndex     int
	TextIndex          int
	ToolIndex          int
	TextBuf            strings.Builder
	ReasoningBuf       strings.Builder
	ToolArgsBuf        strings.Builder
	CompletedOutput    []interface{}
	InputTokens        int64
	OutputTokens       int64
}

func ConvertClaudeMessagesToResponses(ctx context.Context, modelName string, originalRequestRawJSON, requestRawJSON, rawJSON []byte, param *any) []string {
	_ = ctx
	_ = requestRawJSON
	if *param == nil {
		*param = &claudeToResponsesState{FirstChunk: true}
	}
	st := (*param).(*claudeToResponsesState)

	if !bytes.HasPrefix(rawJSON, chatDataTag) {
		return nil
	}
	rawJSON = bytes.TrimSpace(rawJSON[len(chatDataTag):])
	if string(rawJSON) == "[DONE]" {
		return nil
	}

	root := gjson.ParseBytes(rawJSON)
	eventType := root.Get("type").String()
	if eventType == "" {
		return nil
	}

	var out []string
	nextSeq := func() int { st.Seq++; return st.Seq }

	if st.FirstChunk {
		st.FirstChunk = false
		st.ResponseID = root.Get("message.id").String()
		if st.ResponseID == "" {
			st.ResponseID = fmt.Sprintf("resp_%d", time.Now().UnixNano())
		}
		st.CreatedAt = time.Now().Unix()
		out = append(out, st.emitCreatedAndInProgress(nextSeq)...)
	}

	switch eventType {
	case "message_start":
		if id := root.Get("message.id").String(); id != "" {
			st.ResponseID = id
		}
		if inputTokens := root.Get("message.usage.input_tokens"); inputTokens.Exists() {
			st.InputTokens = inputTokens.Int()
		}
	case "content_block_start":
		blockType := root.Get("content_block.type").String()
		switch blockType {
		case "thinking":
			out = append(out, st.startReasoning(nextSeq)...)
			if thinking := root.Get("content_block.thinking").String(); thinking != "" {
				out = append(out, st.appendReasoningDelta(thinking, nextSeq)...)
			}
		case "text":
			out = append(out, st.startText(nextSeq)...)
			if text := root.Get("content_block.text").String(); text != "" {
				out = append(out, st.appendTextDelta(text, nextSeq)...)
			}
		case "tool_use":
			out = append(out, st.startToolUse(
				root.Get("content_block.id").String(),
				root.Get("content_block.name").String(),
				nextSeq,
			)...)
		}
	case "content_block_delta":
		deltaType := root.Get("delta.type").String()
		switch deltaType {
		case "thinking_delta":
			if thinking := root.Get("delta.thinking").String(); thinking != "" {
				if st.ActiveItemType != "reasoning" {
					out = append(out, st.startReasoning(nextSeq)...)
				}
				out = append(out, st.appendReasoningDelta(thinking, nextSeq)...)
			}
		case "text_delta":
			if text := root.Get("delta.text").String(); text != "" {
				if st.ActiveItemType != "text" {
					out = append(out, st.startText(nextSeq)...)
				}
				out = append(out, st.appendTextDelta(text, nextSeq)...)
			}
		case "input_json_delta":
			if partialJSON := root.Get("delta.partial_json").String(); partialJSON != "" {
				if st.ActiveItemType == "tool_use" {
					out = append(out, st.appendToolUseDelta(partialJSON, nextSeq)...)
				}
			}
		}
	case "content_block_stop":
		switch st.ActiveItemType {
		case "reasoning":
			out = append(out, st.closeReasoning(nextSeq)...)
		case "text":
			out = append(out, st.closeText(nextSeq)...)
		case "tool_use":
			out = append(out, st.closeToolUse(nextSeq)...)
		}
	case "message_delta":
		if usage := root.Get("usage"); usage.Exists() {
			if inputTokens := usage.Get("input_tokens"); inputTokens.Exists() {
				st.InputTokens = inputTokens.Int()
			}
			if outputTokens := usage.Get("output_tokens"); outputTokens.Exists() {
				st.OutputTokens = outputTokens.Int()
			}
		}
	case "message_stop":
		if st.ActiveItemType == "reasoning" {
			out = append(out, st.closeReasoning(nextSeq)...)
		}
		if st.ActiveItemType == "text" {
			out = append(out, st.closeText(nextSeq)...)
		}
		if st.ActiveItemType == "tool_use" {
			out = append(out, st.closeToolUse(nextSeq)...)
		}
		out = append(out, st.completedEvent(modelName, originalRequestRawJSON, nextSeq))
	}

	return out
}

func (st *claudeToResponsesState) emitCreatedAndInProgress(nextSeq func() int) []string {
	created := `{"type":"response.created","sequence_number":0,"response":{"id":"","object":"response","created_at":0,"status":"in_progress","background":false,"error":null,"instructions":""}}`
	created, _ = sjson.Set(created, "sequence_number", nextSeq())
	created, _ = sjson.Set(created, "response.id", st.ResponseID)
	created, _ = sjson.Set(created, "response.created_at", st.CreatedAt)

	inProgress := `{"type":"response.in_progress","sequence_number":0,"response":{"id":"","object":"response","created_at":0,"status":"in_progress"}}`
	inProgress, _ = sjson.Set(inProgress, "sequence_number", nextSeq())
	inProgress, _ = sjson.Set(inProgress, "response.id", st.ResponseID)
	inProgress, _ = sjson.Set(inProgress, "response.created_at", st.CreatedAt)

	return []string{
		emitResponsesEvent("response.created", created),
		emitResponsesEvent("response.in_progress", inProgress),
	}
}

func (st *claudeToResponsesState) startReasoning(nextSeq func() int) []string {
	st.ActiveItemType = "reasoning"
	st.ReasoningIndex = st.NextOutputIndex
	st.NextOutputIndex++
	st.CurrentReasoningID = fmt.Sprintf("rs_%s_%d", st.ResponseID, st.ReasoningIndex)
	st.ReasoningBuf.Reset()

	item := `{"type":"response.output_item.added","sequence_number":0,"output_index":0,"item":{"id":"","type":"reasoning","status":"in_progress","summary":[]}}`
	item, _ = sjson.Set(item, "sequence_number", nextSeq())
	item, _ = sjson.Set(item, "output_index", st.ReasoningIndex)
	item, _ = sjson.Set(item, "item.id", st.CurrentReasoningID)

	part := `{"type":"response.reasoning_summary_part.added","sequence_number":0,"item_id":"","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}`
	part, _ = sjson.Set(part, "sequence_number", nextSeq())
	part, _ = sjson.Set(part, "item_id", st.CurrentReasoningID)
	part, _ = sjson.Set(part, "output_index", st.ReasoningIndex)

	return []string{
		emitResponsesEvent("response.output_item.added", item),
		emitResponsesEvent("response.reasoning_summary_part.added", part),
	}
}

func (st *claudeToResponsesState) appendReasoningDelta(text string, nextSeq func() int) []string {
	st.ReasoningBuf.WriteString(text)
	delta := `{"type":"response.reasoning_summary_text.delta","sequence_number":0,"item_id":"","output_index":0,"summary_index":0,"text":""}`
	delta, _ = sjson.Set(delta, "sequence_number", nextSeq())
	delta, _ = sjson.Set(delta, "item_id", st.CurrentReasoningID)
	delta, _ = sjson.Set(delta, "output_index", st.ReasoningIndex)
	delta, _ = sjson.Set(delta, "text", text)
	return []string{emitResponsesEvent("response.reasoning_summary_text.delta", delta)}
}

func (st *claudeToResponsesState) closeReasoning(nextSeq func() int) []string {
	full := st.ReasoningBuf.String()
	textDone := `{"type":"response.reasoning_summary_text.done","sequence_number":0,"item_id":"","output_index":0,"summary_index":0,"text":""}`
	textDone, _ = sjson.Set(textDone, "sequence_number", nextSeq())
	textDone, _ = sjson.Set(textDone, "item_id", st.CurrentReasoningID)
	textDone, _ = sjson.Set(textDone, "output_index", st.ReasoningIndex)
	textDone, _ = sjson.Set(textDone, "text", full)

	partDone := `{"type":"response.reasoning_summary_part.done","sequence_number":0,"item_id":"","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}`
	partDone, _ = sjson.Set(partDone, "sequence_number", nextSeq())
	partDone, _ = sjson.Set(partDone, "item_id", st.CurrentReasoningID)
	partDone, _ = sjson.Set(partDone, "output_index", st.ReasoningIndex)
	partDone, _ = sjson.Set(partDone, "part.text", full)

	itemDone := `{"type":"response.output_item.done","sequence_number":0,"output_index":0,"item":{"id":"","type":"reasoning","status":"completed","summary":[]}}`
	itemDone, _ = sjson.Set(itemDone, "sequence_number", nextSeq())
	itemDone, _ = sjson.Set(itemDone, "output_index", st.ReasoningIndex)
	itemDone, _ = sjson.Set(itemDone, "item.id", st.CurrentReasoningID)
	itemDone, _ = sjson.Set(itemDone, "item.summary", []interface{}{map[string]interface{}{"type": "summary_text", "text": full}})

	if full != "" {
		st.CompletedOutput = append(st.CompletedOutput, map[string]interface{}{
			"id":     st.CurrentReasoningID,
			"type":   "reasoning",
			"status": "completed",
			"summary": []interface{}{map[string]interface{}{
				"type": "summary_text",
				"text": full,
			}},
		})
	}

	st.ActiveItemType = ""
	return []string{
		emitResponsesEvent("response.reasoning_summary_text.done", textDone),
		emitResponsesEvent("response.reasoning_summary_part.done", partDone),
		emitResponsesEvent("response.output_item.done", itemDone),
	}
}

func (st *claudeToResponsesState) startText(nextSeq func() int) []string {
	st.ActiveItemType = "text"
	st.TextIndex = st.NextOutputIndex
	st.NextOutputIndex++
	st.CurrentMsgID = fmt.Sprintf("msg_%s_%d", st.ResponseID, st.TextIndex)
	st.TextBuf.Reset()

	item := `{"type":"response.output_item.added","sequence_number":0,"output_index":0,"item":{"id":"","type":"message","status":"in_progress","content":[],"role":"assistant"}}`
	item, _ = sjson.Set(item, "sequence_number", nextSeq())
	item, _ = sjson.Set(item, "output_index", st.TextIndex)
	item, _ = sjson.Set(item, "item.id", st.CurrentMsgID)

	part := `{"type":"response.content_part.added","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":""}}`
	part, _ = sjson.Set(part, "sequence_number", nextSeq())
	part, _ = sjson.Set(part, "item_id", st.CurrentMsgID)
	part, _ = sjson.Set(part, "output_index", st.TextIndex)

	return []string{
		emitResponsesEvent("response.output_item.added", item),
		emitResponsesEvent("response.content_part.added", part),
	}
}

func (st *claudeToResponsesState) appendTextDelta(text string, nextSeq func() int) []string {
	st.TextBuf.WriteString(text)
	delta := `{"type":"response.output_text.delta","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"delta":"","logprobs":[]}`
	delta, _ = sjson.Set(delta, "sequence_number", nextSeq())
	delta, _ = sjson.Set(delta, "item_id", st.CurrentMsgID)
	delta, _ = sjson.Set(delta, "output_index", st.TextIndex)
	delta, _ = sjson.Set(delta, "delta", text)
	return []string{emitResponsesEvent("response.output_text.delta", delta)}
}

func (st *claudeToResponsesState) closeText(nextSeq func() int) []string {
	full := st.TextBuf.String()

	textDone := `{"type":"response.output_text.done","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"text":"","logprobs":[]}`
	textDone, _ = sjson.Set(textDone, "sequence_number", nextSeq())
	textDone, _ = sjson.Set(textDone, "item_id", st.CurrentMsgID)
	textDone, _ = sjson.Set(textDone, "output_index", st.TextIndex)
	textDone, _ = sjson.Set(textDone, "text", full)

	partDone := `{"type":"response.content_part.done","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"part":{"type":"output_text","annotations":[],"logprobs":[],"text":""}}`
	partDone, _ = sjson.Set(partDone, "sequence_number", nextSeq())
	partDone, _ = sjson.Set(partDone, "item_id", st.CurrentMsgID)
	partDone, _ = sjson.Set(partDone, "output_index", st.TextIndex)
	partDone, _ = sjson.Set(partDone, "part.text", full)

	itemDone := `{"type":"response.output_item.done","sequence_number":0,"output_index":0,"item":{"id":"","type":"message","status":"completed","content":[{"type":"output_text","annotations":[],"logprobs":[],"text":""}],"role":"assistant"}}`
	itemDone, _ = sjson.Set(itemDone, "sequence_number", nextSeq())
	itemDone, _ = sjson.Set(itemDone, "output_index", st.TextIndex)
	itemDone, _ = sjson.Set(itemDone, "item.id", st.CurrentMsgID)
	itemDone, _ = sjson.Set(itemDone, "item.content.0.text", full)

	if full != "" {
		st.CompletedOutput = append(st.CompletedOutput, map[string]interface{}{
			"id":     st.CurrentMsgID,
			"type":   "message",
			"status": "completed",
			"role":   "assistant",
			"content": []interface{}{map[string]interface{}{
				"type":        "output_text",
				"annotations": []interface{}{},
				"logprobs":    []interface{}{},
				"text":        full,
			}},
		})
	}

	st.ActiveItemType = ""
	return []string{
		emitResponsesEvent("response.output_text.done", textDone),
		emitResponsesEvent("response.content_part.done", partDone),
		emitResponsesEvent("response.output_item.done", itemDone),
	}
}

func (st *claudeToResponsesState) startToolUse(callID, name string, nextSeq func() int) []string {
	st.ActiveItemType = "tool_use"
	st.ToolIndex = st.NextOutputIndex
	st.NextOutputIndex++
	st.CurrentToolCallID = callID
	st.CurrentToolName = name
	st.CurrentToolItemID = fmt.Sprintf("fc_%s", callID)
	if callID == "" {
		st.CurrentToolItemID = fmt.Sprintf("fc_%s_%d", st.ResponseID, st.ToolIndex)
	}
	st.ToolArgsBuf.Reset()

	item := `{"type":"response.output_item.added","sequence_number":0,"output_index":0,"item":{"id":"","type":"function_call","status":"in_progress","arguments":"","call_id":"","name":""}}`
	item, _ = sjson.Set(item, "sequence_number", nextSeq())
	item, _ = sjson.Set(item, "output_index", st.ToolIndex)
	item, _ = sjson.Set(item, "item.id", st.CurrentToolItemID)
	item, _ = sjson.Set(item, "item.call_id", st.CurrentToolCallID)
	item, _ = sjson.Set(item, "item.name", st.CurrentToolName)

	return []string{emitResponsesEvent("response.output_item.added", item)}
}

func (st *claudeToResponsesState) appendToolUseDelta(partialJSON string, nextSeq func() int) []string {
	st.ToolArgsBuf.WriteString(partialJSON)
	delta := `{"type":"response.function_call_arguments.delta","sequence_number":0,"item_id":"","output_index":0,"delta":""}`
	delta, _ = sjson.Set(delta, "sequence_number", nextSeq())
	delta, _ = sjson.Set(delta, "item_id", st.CurrentToolItemID)
	delta, _ = sjson.Set(delta, "output_index", st.ToolIndex)
	delta, _ = sjson.Set(delta, "delta", partialJSON)
	return []string{emitResponsesEvent("response.function_call_arguments.delta", delta)}
}

func (st *claudeToResponsesState) closeToolUse(nextSeq func() int) []string {
	args := st.ToolArgsBuf.String()
	if strings.TrimSpace(args) == "" {
		args = "{}"
	}

	argsDone := `{"type":"response.function_call_arguments.done","sequence_number":0,"item_id":"","output_index":0,"arguments":""}`
	argsDone, _ = sjson.Set(argsDone, "sequence_number", nextSeq())
	argsDone, _ = sjson.Set(argsDone, "item_id", st.CurrentToolItemID)
	argsDone, _ = sjson.Set(argsDone, "output_index", st.ToolIndex)
	argsDone, _ = sjson.Set(argsDone, "arguments", args)

	itemDone := `{"type":"response.output_item.done","sequence_number":0,"output_index":0,"item":{"id":"","type":"function_call","status":"completed","arguments":"","call_id":"","name":""}}`
	itemDone, _ = sjson.Set(itemDone, "sequence_number", nextSeq())
	itemDone, _ = sjson.Set(itemDone, "output_index", st.ToolIndex)
	itemDone, _ = sjson.Set(itemDone, "item.id", st.CurrentToolItemID)
	itemDone, _ = sjson.Set(itemDone, "item.arguments", args)
	itemDone, _ = sjson.Set(itemDone, "item.call_id", st.CurrentToolCallID)
	itemDone, _ = sjson.Set(itemDone, "item.name", st.CurrentToolName)

	st.CompletedOutput = append(st.CompletedOutput, map[string]interface{}{
		"id":        st.CurrentToolItemID,
		"type":      "function_call",
		"status":    "completed",
		"arguments": args,
		"call_id":   st.CurrentToolCallID,
		"name":      st.CurrentToolName,
	})

	st.ActiveItemType = ""
	return []string{
		emitResponsesEvent("response.function_call_arguments.done", argsDone),
		emitResponsesEvent("response.output_item.done", itemDone),
	}
}

func (st *claudeToResponsesState) completedEvent(modelName string, originalRequestRawJSON []byte, nextSeq func() int) string {
	completed := `{"type":"response.completed","sequence_number":0,"response":{"id":"","object":"response","created_at":0,"status":"completed","background":false,"error":null,"output":[],"usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}}}`
	completed, _ = sjson.Set(completed, "sequence_number", nextSeq())
	completed, _ = sjson.Set(completed, "response.id", st.ResponseID)
	completed, _ = sjson.Set(completed, "response.created_at", st.CreatedAt)
	if modelName != "" {
		completed, _ = sjson.Set(completed, "response.model", modelName)
	}
	if originalRequestRawJSON != nil {
		req := gjson.ParseBytes(originalRequestRawJSON)
		if v := req.Get("instructions"); v.Exists() {
			completed, _ = sjson.Set(completed, "response.instructions", v.String())
		}
	}

	completed, _ = sjson.Set(completed, "response.output", st.CompletedOutput)
	completed, _ = sjson.Set(completed, "response.usage.input_tokens", st.InputTokens)
	completed, _ = sjson.Set(completed, "response.usage.output_tokens", st.OutputTokens)
	completed, _ = sjson.Set(completed, "response.usage.total_tokens", st.InputTokens+st.OutputTokens)

	return emitResponsesEvent("response.completed", completed)
}
