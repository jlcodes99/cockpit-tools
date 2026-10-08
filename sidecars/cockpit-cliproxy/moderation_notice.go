package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	sdktranslator "github.com/router-for-me/CLIProxyAPI/v7/sdk/translator"
	"github.com/tidwall/gjson"
)

const cockpitRelayGinKey = "cockpitRelay"

func (s *relayServer) bindRelayContext(c *gin.Context) {
	if s == nil || c == nil {
		return
	}
	c.Set(cockpitRelayGinKey, s)
}

func relayServerFromContext(c *gin.Context) *relayServer {
	if c == nil {
		return nil
	}
	value, ok := c.Get(cockpitRelayGinKey)
	if !ok {
		return nil
	}
	server, _ := value.(*relayServer)
	return server
}

func isModerationHTTPStatus(status int) bool {
	switch status {
	case http.StatusBadRequest, http.StatusForbidden, http.StatusUnprocessableEntity:
		return true
	default:
		return false
	}
}

func isContentModerationError(status int, body string) bool {
	if !isModerationHTTPStatus(status) {
		return false
	}
	if contentModerationNeedle(body) || contentModerationCode(body) {
		return true
	}
	trimmed := strings.TrimSpace(body)
	if trimmed == "" || !gjson.Valid(trimmed) {
		return false
	}
	parsed := gjson.Parse(trimmed)
	if contentModerationNeedle(parsed.Get("error.message").String()) ||
		contentModerationNeedle(parsed.Get("message").String()) ||
		contentModerationNeedle(parsed.Get("error.msg").String()) {
		return true
	}
	if contentModerationCode(parsed.Get("error.code").String()) ||
		contentModerationCode(parsed.Get("error.type").String()) ||
		contentModerationCode(parsed.Get("code").String()) ||
		contentModerationCode(parsed.Get("choices.0.finish_reason").String()) {
		return true
	}
	return false
}

func contentModerationCode(code string) bool {
	lower := strings.ToLower(strings.TrimSpace(code))
	if lower == "" {
		return false
	}
	switch lower {
	case "content_filter", "content_policy", "content_policy_violation",
		"content_moderation", "responsible_ai", "responsible_ai_policy":
		return true
	default:
		return strings.Contains(lower, "content_filter") ||
			strings.Contains(lower, "content_policy") ||
			strings.Contains(lower, "content exists risk")
	}
}

func contentModerationNeedle(text string) bool {
	lower := strings.ToLower(text)
	if lower == "" {
		return false
	}
	for _, needle := range []string{
		"content exists risk",
		"content_filter",
		"content filter",
		"content_policy",
		"content policy",
		"content moderation",
		"prompt flagged",
		"output flagged",
		"blocked by our content",
		"violates our content",
		"responsibleai",
		"responsible ai policy",
	} {
		if strings.Contains(lower, needle) {
			return true
		}
	}
	return false
}

func moderationNoticeText(locale string) string {
	if strings.HasPrefix(strings.ToLower(strings.TrimSpace(locale)), "zh") {
		return "本轮内容被上游内容审核拦截，请切换模型后重试。"
	}
	return "This turn was blocked by upstream content moderation. Switch models and try again."
}

func sourceFormatFromRequest(c *gin.Context) sdktranslator.Format {
	if c == nil || c.Request == nil {
		return sdktranslator.FormatOpenAIResponse
	}
	path := strings.ToLower(requestPath(c.Request))
	switch {
	case strings.Contains(path, "/chat/completions"):
		return sdktranslator.FormatOpenAI
	case strings.Contains(path, "/messages"):
		return sdktranslator.FormatClaude
	default:
		return sdktranslator.FormatOpenAIResponse
	}
}

func requestPrefersStream(c *gin.Context) bool {
	if c == nil {
		return false
	}
	if c.Writer != nil && c.Writer.Written() {
		contentType := strings.ToLower(c.Writer.Header().Get("Content-Type"))
		if strings.Contains(contentType, "text/event-stream") {
			return true
		}
	}
	if c.Request != nil {
		accept := strings.ToLower(c.Request.Header.Get("Accept"))
		if strings.Contains(accept, "text/event-stream") {
			return true
		}
		body, err := readAndRestoreBody(c.Request)
		if err == nil && requestBodyStream(body) {
			return true
		}
	}
	return false
}

func requestModelFromContext(c *gin.Context) string {
	if c == nil || c.Request == nil {
		return ""
	}
	if model, _ := c.Request.Context().Value(requestModelContextKey).(string); strings.TrimSpace(model) != "" {
		return strings.TrimSpace(model)
	}
	body, err := readAndRestoreBody(c.Request)
	if err != nil {
		return ""
	}
	return requestBodyModel(body)
}

func clearCopiedUpstreamBodyHeaders(c *gin.Context) {
	if c == nil {
		return
	}
	header := c.Writer.Header()
	header.Del("Content-Length")
	header.Del("Content-Encoding")
	header.Del("Transfer-Encoding")
}

func (s *relayServer) tryWriteModerationNoticeFromError(c *gin.Context, err error, sourceFormat sdktranslator.Format, stream bool) bool {
	if err == nil {
		return false
	}
	return s.tryWriteModerationNotice(c, statusCodeFromError(err), errorMessage(err), sourceFormat, stream)
}

func (s *relayServer) tryWriteModerationNotice(c *gin.Context, status int, body string, sourceFormat sdktranslator.Format, stream bool) bool {
	if c == nil || !isContentModerationError(status, body) {
		return false
	}
	locale := ""
	if s != nil && s.manifest != nil {
		locale = s.manifest.Locale
	}
	model := requestModelFromContext(c)
	notice := moderationNoticeText(locale)
	clearCopiedUpstreamBodyHeaders(c)
	if stream {
		return writeModerationNoticeStream(c, sourceFormat, model, notice)
	}
	return writeModerationNoticeJSON(c, sourceFormat, model, notice)
}

func writeModerationNoticeJSON(c *gin.Context, sourceFormat sdktranslator.Format, model, notice string) bool {
	payload, err := moderationNoticeJSON(sourceFormat, model, notice)
	if err != nil {
		return false
	}
	c.Data(http.StatusOK, "application/json", payload)
	return true
}

func writeModerationNoticeStream(c *gin.Context, sourceFormat sdktranslator.Format, model, notice string) bool {
	frames, err := moderationNoticeStreamFrames(sourceFormat, model, notice)
	if err != nil || len(frames) == 0 {
		return false
	}
	if !c.Writer.Written() {
		setEventStreamHeaders(c.Writer.Header())
		c.Status(http.StatusOK)
	}
	for _, frame := range frames {
		if _, writeErr := c.Writer.Write(frame); writeErr != nil {
			return true
		}
	}
	if flusher, ok := c.Writer.(http.Flusher); ok {
		flusher.Flush()
	}
	return true
}

func moderationNoticeJSON(sourceFormat sdktranslator.Format, model, notice string) ([]byte, error) {
	now := time.Now().Unix()
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI) {
		return json.Marshal(map[string]any{
			"id":      fmt.Sprintf("chatcmpl_moderation_%d", now),
			"object":  "chat.completion",
			"created": now,
			"model":   model,
			"choices": []map[string]any{
				{
					"index": 0,
					"message": map[string]any{
						"role":    "assistant",
						"content": notice,
					},
					"finish_reason": "stop",
				},
			},
			"usage": map[string]any{
				"prompt_tokens":     0,
				"completion_tokens": 0,
				"total_tokens":      0,
			},
		})
	}
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatClaude) {
		return json.Marshal(map[string]any{
			"id":          fmt.Sprintf("msg_moderation_%d", now),
			"type":        "message",
			"role":        "assistant",
			"model":       model,
			"content":     []map[string]any{{"type": "text", "text": notice}},
			"stop_reason": "end_turn",
			"usage":       map[string]any{"input_tokens": 0, "output_tokens": 0},
		})
	}
	response := moderationResponsesObject(now, model, notice, "completed")
	return json.Marshal(response)
}

func moderationNoticeStreamFrames(sourceFormat sdktranslator.Format, model, notice string) ([][]byte, error) {
	now := time.Now().Unix()
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatOpenAI) {
		chunk := map[string]any{
			"id":      fmt.Sprintf("chatcmpl_moderation_%d", now),
			"object":  "chat.completion.chunk",
			"created": now,
			"model":   model,
			"choices": []map[string]any{
				{
					"index": 0,
					"delta": map[string]any{
						"role":    "assistant",
						"content": notice,
					},
					"finish_reason": "stop",
				},
			},
		}
		payload, err := json.Marshal(chunk)
		if err != nil {
			return nil, err
		}
		return [][]byte{
			frameOpenAIStreamChunk(payload),
			[]byte("data: [DONE]\n\n"),
		}, nil
	}
	if sourceFormatEqual(sourceFormat, sdktranslator.FormatClaude) {
		messageID := fmt.Sprintf("msg_moderation_%d", now)
		frames := make([][]byte, 0, 6)
		for _, item := range []struct {
			event string
			data  map[string]any
		}{
			{"message_start", map[string]any{
				"type": "message_start",
				"message": map[string]any{
					"id":      messageID,
					"type":    "message",
					"role":    "assistant",
					"model":   model,
					"content": []any{},
				},
			}},
			{"content_block_start", map[string]any{
				"type":          "content_block_start",
				"index":         0,
				"content_block": map[string]any{"type": "text", "text": ""},
			}},
			{"content_block_delta", map[string]any{
				"type":  "content_block_delta",
				"index": 0,
				"delta": map[string]any{"type": "text_delta", "text": notice},
			}},
			{"content_block_stop", map[string]any{"type": "content_block_stop", "index": 0}},
			{"message_delta", map[string]any{
				"type":  "message_delta",
				"delta": map[string]any{"stop_reason": "end_turn"},
			}},
			{"message_stop", map[string]any{"type": "message_stop"}},
		} {
			payload, err := json.Marshal(item.data)
			if err != nil {
				return nil, err
			}
			frames = append(frames, sseFrame(item.event, payload))
		}
		return frames, nil
	}

	responseID := fmt.Sprintf("resp_moderation_%d", now)
	messageID := fmt.Sprintf("msg_moderation_%d", now)
	created := moderationResponsesObject(now, model, "", "in_progress")
	created["id"] = responseID
	completed := moderationResponsesObject(now, model, notice, "completed")
	completed["id"] = responseID
	outputItem := moderationResponsesOutputItem(messageID, notice)
	frames := make([][]byte, 0, 6)
	sequence := []struct {
		event string
		data  map[string]any
	}{
		{"response.created", map[string]any{"type": "response.created", "sequence_number": 0, "response": created}},
		{"response.in_progress", map[string]any{"type": "response.in_progress", "sequence_number": 1, "response": created}},
		{"response.output_item.added", map[string]any{
			"type":            "response.output_item.added",
			"sequence_number": 2,
			"output_index":    0,
			"item":            outputItem,
		}},
		{"response.output_text.delta", map[string]any{
			"type":            "response.output_text.delta",
			"sequence_number": 3,
			"item_id":         messageID,
			"output_index":    0,
			"content_index":   0,
			"delta":           notice,
		}},
		{"response.output_item.done", map[string]any{
			"type":            "response.output_item.done",
			"sequence_number": 4,
			"output_index":    0,
			"item":            outputItem,
		}},
		{"response.completed", map[string]any{"type": "response.completed", "sequence_number": 5, "response": completed}},
	}
	for _, item := range sequence {
		payload, err := json.Marshal(item.data)
		if err != nil {
			return nil, err
		}
		frames = append(frames, sseFrame(item.event, payload))
	}
	return frames, nil
}

func moderationResponsesObject(createdAt int64, model, notice, status string) map[string]any {
	response := map[string]any{
		"id":                 fmt.Sprintf("resp_moderation_%d", createdAt),
		"object":             "response",
		"created_at":         createdAt,
		"status":             status,
		"background":         false,
		"error":              nil,
		"incomplete_details": nil,
		"output":             []any{},
		"usage": map[string]any{
			"input_tokens":          0,
			"input_tokens_details":  map[string]any{"cached_tokens": 0},
			"output_tokens":         0,
			"output_tokens_details": map[string]any{"reasoning_tokens": 0},
			"total_tokens":          0,
		},
	}
	if strings.TrimSpace(model) != "" {
		response["model"] = model
	}
	if status == "completed" && strings.TrimSpace(notice) != "" {
		response["output"] = []any{moderationResponsesOutputItem(fmt.Sprintf("msg_moderation_%d", createdAt), notice)}
	}
	return response
}

func moderationResponsesOutputItem(messageID, notice string) map[string]any {
	return map[string]any{
		"id":     messageID,
		"type":   "message",
		"status": "completed",
		"role":   "assistant",
		"content": []map[string]any{
			{"type": "output_text", "text": notice},
		},
	}
}

func sseFrame(event string, data []byte) []byte {
	var builder strings.Builder
	if event != "" {
		builder.WriteString("event: ")
		builder.WriteString(event)
		builder.WriteByte('\n')
	}
	builder.WriteString("data: ")
	builder.Write(data)
	builder.WriteString("\n\n")
	return []byte(builder.String())
}
