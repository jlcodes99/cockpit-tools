package executor

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/tidwall/gjson"
	"github.com/tidwall/sjson"
)

const codexIncompleteStreamMessage = "stream closed before response.completed"

type codexIncompleteStreamError struct {
	statusErr
	cause error
}

type codexEmptyIncompleteStreamError struct {
	statusErr
}

func newCodexEmptyIncompleteStreamError() codexEmptyIncompleteStreamError {
	return codexEmptyIncompleteStreamError{statusErr: statusErr{
		code: http.StatusBadGateway,
		msg:  helps.CodexEmptyIncompleteStreamMessage,
	}}
}

func (codexEmptyIncompleteStreamError) IsRequestScoped() bool { return true }

func newCodexIncompleteStreamError(cause error) codexIncompleteStreamError {
	status, code, message := http.StatusBadGateway, "incomplete_upstream_stream", codexIncompleteStreamMessage
	if cause != nil && !errors.Is(cause, io.EOF) {
		code, message = "upstream_stream_error", helps.RedactStreamTransportError(cause.Error())
		var timeout net.Error
		if errors.Is(cause, context.DeadlineExceeded) || errors.As(cause, &timeout) && timeout.Timeout() {
			status, code = http.StatusRequestTimeout, "request_timeout"
		}
	}
	body := []byte(`{"error":{"type":"server_error"}}`)
	body, _ = sjson.SetBytes(body, "error.code", code)
	body, _ = sjson.SetBytes(body, "error.message", message)
	return codexIncompleteStreamError{statusErr: statusErr{
		code: status,
		msg:  string(body),
	}, cause: cause}
}

func (e codexIncompleteStreamError) Unwrap() error { return e.cause }

func (codexIncompleteStreamError) IsRequestScoped() bool {
	return true
}

// Preserve cancellation and structured WebSocket failures, while giving transport
// failures the same request-scoped classification as HTTP response-body reads.
func codexResponsesReadError(cause error) error {
	if errors.Is(cause, context.Canceled) {
		return cause
	}
	var status interface{ StatusCode() int }
	if errors.As(cause, &status) {
		return cause
	}
	return newCodexIncompleteStreamError(cause)
}

// Streamed Codex responses may emit response.output_item.done events while leaving
// response.completed.response.output empty. Keep the stream path aligned with the
// already-patched non-stream path by reconstructing response.output from those items.
func collectCodexOutputItemDone(eventData []byte, outputItemsByIndex map[int64][]byte, outputItemsFallback *[][]byte) {
	itemResult := gjson.GetBytes(eventData, "item")
	if !itemResult.Exists() || itemResult.Type != gjson.JSON {
		return
	}
	outputIndexResult := gjson.GetBytes(eventData, "output_index")
	if outputIndexResult.Exists() {
		outputItemsByIndex[outputIndexResult.Int()] = []byte(itemResult.Raw)
		return
	}
	*outputItemsFallback = append(*outputItemsFallback, []byte(itemResult.Raw))
}

func hydrateCodexCompletedOutputItemIDs(eventData []byte, outputItems []gjson.Result, outputItemsByIndex map[int64][]byte) []byte {
	patchedData := eventData
	for outputIndex, outputItem := range outputItems {
		itemData := []byte(outputItem.Raw)
		itemID := gjson.GetBytes(itemData, "id")
		if itemID.Exists() && itemID.Type != gjson.Null && (itemID.Type != gjson.String || strings.TrimSpace(itemID.String()) != "") {
			continue
		}

		completedItem, ok := outputItemsByIndex[int64(outputIndex)]
		if !ok {
			continue
		}
		completedID := gjson.GetBytes(completedItem, "id")
		if completedID.Type != gjson.String || strings.TrimSpace(completedID.String()) == "" {
			continue
		}

		updatedData, errSet := sjson.SetRawBytes(patchedData, "response.output."+strconv.Itoa(outputIndex)+".id", []byte(completedID.Raw))
		if errSet != nil {
			continue
		}
		patchedData = updatedData
	}
	return patchedData
}

func patchCodexCompletedOutput(eventData []byte, outputItemsByIndex map[int64][]byte, outputItemsFallback [][]byte) []byte {
	outputResult := gjson.GetBytes(eventData, "response.output")
	if outputResult.Exists() && outputResult.IsArray() && len(outputResult.Array()) > 0 {
		return hydrateCodexCompletedOutputItemIDs(eventData, outputResult.Array(), outputItemsByIndex)
	}

	shouldPatchOutput := (!outputResult.Exists() || !outputResult.IsArray() || len(outputResult.Array()) == 0) && (len(outputItemsByIndex) > 0 || len(outputItemsFallback) > 0)
	if !shouldPatchOutput {
		return eventData
	}

	indexes := make([]int64, 0, len(outputItemsByIndex))
	for idx := range outputItemsByIndex {
		indexes = append(indexes, idx)
	}
	sort.Slice(indexes, func(i, j int) bool {
		return indexes[i] < indexes[j]
	})

	items := make([][]byte, 0, len(outputItemsByIndex)+len(outputItemsFallback))
	for _, idx := range indexes {
		items = append(items, outputItemsByIndex[idx])
	}
	items = append(items, outputItemsFallback...)

	outputArray := []byte("[]")
	if len(items) > 0 {
		var buf bytes.Buffer
		totalLen := 2
		for _, item := range items {
			totalLen += len(item)
		}
		if len(items) > 1 {
			totalLen += len(items) - 1
		}
		buf.Grow(totalLen)
		buf.WriteByte('[')
		for i, item := range items {
			if i > 0 {
				buf.WriteByte(',')
			}
			buf.Write(item)
		}
		buf.WriteByte(']')
		outputArray = buf.Bytes()
	}

	completedDataPatched, _ := sjson.SetRawBytes(eventData, "response.output", outputArray)
	return completedDataPatched
}

func codexTerminalStreamContextLengthErr(eventData []byte) (statusErr, bool) {
	streamErr, body, ok := codexTerminalStreamErr(eventData)
	if !ok || !codexTerminalErrorIsContextLength(body) {
		return statusErr{}, false
	}
	return streamErr, true
}

func codexTerminalStreamErr(eventData []byte) (statusErr, []byte, bool) {
	return codexTerminalStreamErrWithCooling(eventData, false)
}

func codexTerminalStreamErrWithCooling(eventData []byte, modelLevelCooling bool) (statusErr, []byte, bool) {
	body, ok := codexTerminalFailureBody(eventData)
	if !ok || !codexTerminalStreamErrShouldHandle(body) {
		return statusErr{}, nil, false
	}
	return newCodexStatusErrWithCooling(http.StatusBadRequest, body, modelLevelCooling), body, true
}

func codexTerminalFailureErr(eventData []byte) (statusErr, []byte, bool) {
	return codexTerminalFailureErrWithCooling(eventData, false)
}

func codexTerminalFailureErrWithCooling(eventData []byte, modelLevelCooling bool) (statusErr, []byte, bool) {
	if streamErr, body, ok := codexTerminalStreamErrWithCooling(eventData, modelLevelCooling); ok {
		return streamErr, body, true
	}
	body, ok := codexTerminalFailureBody(eventData)
	if !ok {
		return statusErr{}, nil, false
	}
	return newCodexStatusErrWithCooling(codexTerminalFailureStatus(body), body, modelLevelCooling), body, true
}

func codexTerminalFailureStatus(body []byte) int {
	for _, path := range []string{"error.status_code", "error.status"} {
		if status := int(gjson.GetBytes(body, path).Int()); status >= 400 && status <= 599 {
			return status
		}
	}

	errorType := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.type").String()))
	errorCode := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.code").String()))
	switch {
	case errorCode == "cyber_policy":
		return http.StatusBadRequest
	case errorType == "not_found_error", errorCode == "not_found", errorCode == "model_not_found":
		return http.StatusNotFound
	case errorType == "authentication_error", errorCode == "invalid_api_key", errorCode == "unauthorized":
		return http.StatusUnauthorized
	case errorType == "permission_error", errorCode == "forbidden", errorCode == "permission_denied":
		return http.StatusForbidden
	case errorType == "rate_limit_error", errorCode == "rate_limit_exceeded":
		return http.StatusTooManyRequests
	case errorType == "invalid_request_error", errorType == "bad_request_error":
		return http.StatusBadRequest
	default:
		return http.StatusBadGateway
	}
}

func codexTerminalFailureBody(eventData []byte) ([]byte, bool) {
	eventType := gjson.GetBytes(eventData, "type").String()
	var body []byte
	switch eventType {
	case "error":
		body = codexTerminalErrorBody(eventData, "error")
		if len(body) == 0 {
			body = codexTerminalTopLevelErrorBody(eventData)
		}
	case "response.failed":
		body = codexTerminalErrorBody(eventData, "response.error")
		if len(body) == 0 {
			body = codexTerminalErrorBody(eventData, "error")
		}
	default:
		return nil, false
	}
	if len(body) == 0 {
		body = []byte(`{"error":{"message":"upstream stream failed without error details"}}`)
	}
	// Transport envelopes may carry reset hints outside the nested error.
	for _, key := range []string{"headers", "retry_after", "retry_after_seconds", "resets_at", "resets_in_seconds"} {
		value := gjson.GetBytes(eventData, key)
		if !value.Exists() {
			value = gjson.GetBytes(eventData, "response."+key)
		}
		if value.Exists() {
			body, _ = sjson.SetRawBytes(body, key, []byte(value.Raw))
		}
	}
	if seq := gjson.GetBytes(eventData, "sequence_number"); seq.Exists() {
		body, _ = sjson.SetBytes(body, "sequence_number", seq.Int())
	}
	return body, true
}

func codexTerminalStreamErrShouldHandle(body []byte) bool {
	if codexTerminalErrorIsContextLength(body) {
		return true
	}
	if isCodexUsageLimitError(body) || isCodexModelCapacityError(body) {
		return true
	}
	code, _, ok := codexStatusErrorClassification(http.StatusBadRequest, body)
	return ok && code == "thinking_signature_invalid"
}

func codexTerminalErrorBody(eventData []byte, path string) []byte {
	errorResult := gjson.GetBytes(eventData, path)
	if !errorResult.Exists() {
		return nil
	}
	body := []byte(`{"error":{}}`)
	if errorResult.Type == gjson.JSON {
		body, _ = sjson.SetRawBytes(body, "error", []byte(errorResult.Raw))
	} else if message := strings.TrimSpace(errorResult.String()); message != "" {
		body, _ = sjson.SetBytes(body, "error.message", message)
	}
	if strings.TrimSpace(gjson.GetBytes(body, "error.message").String()) == "" {
		if message := strings.TrimSpace(gjson.GetBytes(eventData, "response.error.message").String()); message != "" {
			body, _ = sjson.SetBytes(body, "error.message", message)
		}
	}
	if strings.TrimSpace(gjson.GetBytes(body, "error.message").String()) == "" {
		if code := strings.TrimSpace(gjson.GetBytes(body, "error.code").String()); code != "" {
			body, _ = sjson.SetBytes(body, "error.message", code)
		}
	}
	if strings.TrimSpace(gjson.GetBytes(body, "error.message").String()) == "" {
		if errorType := strings.TrimSpace(gjson.GetBytes(body, "error.type").String()); errorType != "" {
			body, _ = sjson.SetBytes(body, "error.message", errorType)
		}
	}
	return body
}

func codexTerminalTopLevelErrorBody(eventData []byte) []byte {
	message := strings.TrimSpace(gjson.GetBytes(eventData, "message").String())
	code := strings.TrimSpace(gjson.GetBytes(eventData, "code").String())
	errorType := strings.TrimSpace(gjson.GetBytes(eventData, "error_type").String())
	param := strings.TrimSpace(gjson.GetBytes(eventData, "param").String())
	if message == "" && code == "" && errorType == "" && param == "" {
		return nil
	}

	body := []byte(`{"error":{}}`)
	if message != "" {
		body, _ = sjson.SetBytes(body, "error.message", message)
	}
	if code != "" {
		body, _ = sjson.SetBytes(body, "error.code", code)
	}
	if errorType != "" {
		body, _ = sjson.SetBytes(body, "error.type", errorType)
	}
	if param != "" {
		body, _ = sjson.SetBytes(body, "error.param", param)
	}
	if strings.TrimSpace(gjson.GetBytes(body, "error.message").String()) == "" {
		if code != "" {
			body, _ = sjson.SetBytes(body, "error.message", code)
		} else if errorType != "" {
			body, _ = sjson.SetBytes(body, "error.message", errorType)
		}
	}
	return body
}

func codexTerminalErrorIsContextLength(body []byte) bool {
	errorCode := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.code").String()))
	message := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.message").String()))
	return errorCode == "context_length_exceeded" ||
		errorCode == "context_too_large" ||
		strings.Contains(message, "context window") ||
		strings.Contains(message, "context length") ||
		strings.Contains(message, "too many tokens")
}

func newCodexStatusErr(statusCode int, body []byte) statusErr {
	return newCodexStatusErrWithCooling(statusCode, body, false)
}

func newCodexStatusErrWithCooling(statusCode int, body []byte, modelLevelCooling bool) statusErr {
	errCode := statusCode
	isUsageLimit := isCodexUsageLimitError(body)
	credentialScoped := isUsageLimit && !modelLevelCooling
	if isCodexModelCapacityError(body) || isUsageLimit {
		errCode = http.StatusTooManyRequests
	}
	body = classifyCodexStatusError(errCode, body)
	err := statusErr{code: errCode, msg: string(body), credentialScoped: credentialScoped}
	if retryAfter := parseCodexRetryAfter(errCode, body, time.Now()); retryAfter != nil {
		err.retryAfter = retryAfter
	}
	return err
}

func classifyCodexStatusError(statusCode int, body []byte) []byte {
	code, errType, ok := codexStatusErrorClassification(statusCode, body)
	if !ok {
		return body
	}
	message := gjson.GetBytes(body, "error.message").String()
	if message == "" {
		message = gjson.GetBytes(body, "message").String()
	}
	if message == "" {
		message = strings.TrimSpace(string(body))
	}
	if message == "" {
		message = http.StatusText(statusCode)
	}
	out := []byte(`{"error":{}}`)
	out, _ = sjson.SetBytes(out, "error.message", message)
	out, _ = sjson.SetBytes(out, "error.type", errType)
	out, _ = sjson.SetBytes(out, "error.code", code)
	return out
}

func codexStatusErrorClassification(statusCode int, body []byte) (code string, errType string, ok bool) {
	errorMessage := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.message").String()))
	if errorMessage == "" {
		errorMessage = strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "message").String()))
	}
	lower := strings.ToLower(strings.TrimSpace(string(body)))
	upstreamCode := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.code").String()))
	upstreamType := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.type").String()))
	isInvalidRequest := upstreamType == "" || upstreamType == "invalid_request_error"

	switch {
	case statusCode == http.StatusRequestEntityTooLarge || upstreamCode == "context_length_exceeded" || upstreamCode == "context_too_large" || isInvalidRequest && (strings.Contains(errorMessage, "context length") || strings.Contains(errorMessage, "context_length") || strings.Contains(errorMessage, "maximum context") || strings.Contains(errorMessage, "too many tokens")):
		// Codex only recognizes this wire code as ContextWindowExceeded.
		// A private alias turns context recovery into repeated stream retries.
		return "context_length_exceeded", "invalid_request_error", true
	case strings.Contains(lower, "invalid signature in thinking block") || strings.Contains(lower, "invalid_encrypted_content"):
		return "thinking_signature_invalid", "invalid_request_error", true
	case upstreamCode == "previous_response_not_found" || strings.Contains(lower, "previous_response_not_found") || strings.Contains(lower, "previous_response_id") && strings.Contains(lower, "not found"):
		return "previous_response_not_found", "invalid_request_error", true
	case statusCode == http.StatusUnauthorized || upstreamType == "authentication_error" || upstreamCode == "invalid_api_key" || strings.Contains(lower, "invalid or expired token") || strings.Contains(lower, "refresh_token_reused"):
		return "auth_unavailable", "authentication_error", true
	default:
		return "", "", false
	}
}

func isCodexModelCapacityError(errorBody []byte) bool {
	if len(errorBody) == 0 {
		return false
	}
	candidates := []string{
		gjson.GetBytes(errorBody, "error.message").String(),
		gjson.GetBytes(errorBody, "message").String(),
		string(errorBody),
	}
	for _, candidate := range candidates {
		lower := strings.ToLower(strings.TrimSpace(candidate))
		if lower == "" {
			continue
		}
		if strings.Contains(lower, "model is at capacity") ||
			strings.Contains(lower, "model_at_capacity") ||
			strings.Contains(lower, "model_is_at_capacity") ||
			(strings.Contains(lower, "model") && strings.Contains(lower, "at capacity")) {
			return true
		}
	}
	return false
}

// isCodexUsageLimitError recognizes structured account quota failures without
// treating transient request-rate limits or generated text as quota exhaustion.
func isCodexUsageLimitError(errorBody []byte) bool {
	return helps.CodexQuotaExhaustion(errorBody)
}

func parseCodexRetryAfter(statusCode int, errorBody []byte, now time.Time) *time.Duration {
	if statusCode != http.StatusTooManyRequests || !isCodexUsageLimitError(errorBody) {
		return nil
	}
	// Prefer an explicit reset instant over relative hints. Inspect all envelope
	// positions because SSE and websocket peers may attach timing to the wrapper.
	nodes := []gjson.Result{gjson.GetBytes(errorBody, "error"), gjson.ParseBytes(errorBody), gjson.GetBytes(errorBody, "body.error"), gjson.GetBytes(errorBody, "body")}
	for _, quota := range nodes {
		if resetsAt := quota.Get("resets_at").Int(); resetsAt > 0 {
			if delay := time.Unix(resetsAt, 0).Sub(now); delay > 0 {
				return &delay
			}
		}
	}
	for _, quota := range nodes {
		for _, field := range []string{"resets_in_seconds", "retry_after_seconds", "retry_after"} {
			seconds := quota.Get(field).Float()
			if seconds > 0 && seconds <= float64((1<<63-1)/int64(time.Second)) {
				delay := time.Duration(seconds * float64(time.Second))
				return &delay
			}
		}
	}
	for _, quota := range nodes {
		var delay *time.Duration
		quota.Get("headers").ForEach(func(key, value gjson.Result) bool {
			if !strings.EqualFold(key.String(), "Retry-After") {
				return true
			}
			if seconds, errParse := strconv.ParseFloat(value.String(), 64); errParse == nil && seconds > 0 && seconds <= float64((1<<63-1)/int64(time.Second)) {
				parsed := time.Duration(seconds * float64(time.Second))
				delay = &parsed
			} else if resetAt, errParse := http.ParseTime(value.String()); errParse == nil && resetAt.After(now) {
				parsed := resetAt.Sub(now)
				delay = &parsed
			}
			return delay == nil
		})
		if delay != nil {
			return delay
		}
	}
	return nil
}

// codexBootstrapMaxBufferedEvents bounds how many handshake metadata events may be held
// back while probing for an upstream rejection embedded in an HTTP 200 stream. The websocket
// transport prefixes response events with codex.response.metadata and codex.rate_limits frames,
// so the limit must comfortably exceed the four handshake frames observed in practice. Once the
// limit is reached the stream is released and the original unbuffered semantics apply.
const codexBootstrapMaxBufferedEvents = 16

// observeCodexTokenEvent inspects a stream payload and marks TTFT on the first substantive token event.
func observeCodexTokenEvent(reporter *helps.UsageReporter, payload []byte) {
	helps.ObserveResponsesTokenEvent(reporter, payload)
}

// newCodexBootstrapOverloadErr reports a buffered overload rejection with its upstream-derived status.
//
// The status is deliberately produced here instead of in codexTerminalFailureStatus: that mapping
// is shared with the unbuffered path, where the rejection is delivered in-stream and a status
// change would alter cooldown classification and retry-after parsing for everyone. Keeping the
// bootstrap remapping scoped to this path means disabling the feature restores the previous
// behaviour exactly.
func newCodexBootstrapOverloadErr(body []byte) statusErr {
	return newCodexStatusErr(http.StatusServiceUnavailable, body)
}

// isCodexOverloadBootstrapFailure reports whether a terminal failure delivered inside an HTTP 200
// stream is a transient capacity rejection that a different credential may be able to serve.
// Only these failures justify replacing the whole attempt during bootstrap; every other terminal
// failure keeps the original in-stream delivery semantics so downstream behaviour is unchanged.
func isCodexOverloadBootstrapFailure(body []byte) bool {
	if isCodexModelCapacityError(body) {
		return true
	}
	errorType := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.type").String()))
	errorCode := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.code").String()))
	errorMessage := strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "error.message").String()))
	if errorMessage == "" {
		errorMessage = strings.ToLower(strings.TrimSpace(gjson.GetBytes(body, "message").String()))
	}
	switch {
	case errorType == "service_unavailable_error", errorCode == "server_is_overloaded":
		return true
	case errorType == "rate_limit_error", errorCode == "rate_limit_exceeded":
		return true
	case (errorType == "server_error" || errorCode == "server_error") && strings.Contains(errorMessage, "you can retry your request"):
		return true
	default:
		return false
	}
}

// Account-scoped quota and disabled-account errors can rotate before output,
// while retaining the shared unbuffered error mapping and Retry-After parsing.
func isCodexRetryableBootstrapFailure(body []byte) bool {
	if helps.CodexBootstrapInputError(body) {
		return false
	}
	if _, ok := helps.CodexBootstrapAccountFailureStatus(body); ok {
		return true
	}
	if codexTerminalErrorIsContextLength(body) {
		return false
	}
	return isCodexOverloadBootstrapFailure(body)
}

func newCodexBootstrapFailureErr(body []byte, modelLevelCooling bool) statusErr {
	if status, ok := helps.CodexBootstrapAccountFailureStatus(body); ok {
		if status == http.StatusForbidden {
			// Some upstreams pair disabled-account codes with invalid_request_error.
			// Normalize that contradictory type only on the pre-output path, keeping
			// the original structured code and message for credential diagnostics.
			body, _ = sjson.SetBytes(body, "error.type", "permission_error")
		}
		err := newCodexStatusErrWithCooling(status, body, modelLevelCooling)
		err.credentialScoped = true
		return err
	}
	return newCodexBootstrapOverloadErr(body)
}

const codexBootstrapMaxBufferedBytes = 64 << 10
