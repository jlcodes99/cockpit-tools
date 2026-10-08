package main

import (
	"bytes"
	"compress/gzip"
	"compress/zlib"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"regexp"
	"strings"
	"unicode/utf8"
)

// This is deliberately an allow-list. Unknown vendor headers can contain API
// credentials even if their names do not resemble Authorization or Cookie.
func safeDiagnosticHeaders(headers http.Header) map[string]string {
	result := make(map[string]string)
	for _, name := range []string{"Content-Type", "Content-Encoding", "Accept", "User-Agent", "X-Request-Id", "OpenAI-Beta"} {
		if value := headers.Get(name); value != "" {
			result[strings.ToLower(name)] = truncateDiagnosticUTF8(value, 512)
		}
	}
	return result
}

func diagnosticSensitiveField(name string) bool {
	name = strings.ToLower(strings.Map(func(r rune) rune {
		switch r {
		case '_', '-', ' ', '.':
			return -1
		default:
			return r
		}
	}, name))
	switch name {
	case "authorization", "proxyauthorization", "cookie", "setcookie", "password", "passwd", "pwd", "apikey", "apiaccesskey", "accesstoken", "refreshtoken", "idtoken", "token", "secret", "clientsecret", "credential", "credentials", "privatekey", "twofactorsecret", "twofasecret", "totpsecret", "otpsecret", "twofacode", "totpcode", "otp", "sessiontoken", "sessionkey":
		return true
	}
	return strings.HasSuffix(name, "password") || strings.HasSuffix(name, "secret") || strings.HasSuffix(name, "apikey") || strings.HasSuffix(name, "accesstoken") || strings.HasSuffix(name, "refreshtoken") || strings.HasSuffix(name, "privatekey")
}

func redactDiagnosticJSON(value any, depth int) any {
	if depth > 64 {
		return "[omitted: nesting limit]"
	}
	switch typed := value.(type) {
	case map[string]any:
		for key, child := range typed {
			if diagnosticSensitiveField(key) {
				typed[key] = "[REDACTED]"
			} else {
				typed[key] = redactDiagnosticJSON(child, depth+1)
			}
		}
	case []any:
		for i, child := range typed {
			typed[i] = redactDiagnosticJSON(child, depth+1)
		}
	case string:
		// Tool arguments and vendor metadata sometimes embed JSON as a string.
		trimmed := strings.TrimSpace(typed)
		if len(trimmed) > 1 && (trimmed[0] == '{' || trimmed[0] == '[') && json.Valid([]byte(trimmed)) {
			var nested any
			decoder := json.NewDecoder(strings.NewReader(trimmed))
			decoder.UseNumber()
			if decoder.Decode(&nested) == nil {
				if encoded, err := json.Marshal(redactDiagnosticJSON(nested, depth+1)); err == nil {
					return string(encoded)
				}
			}
		}
	}
	return value
}

func decodeDiagnosticBody(snapshot rawDiagnosticSnapshot) ([]byte, bool) {
	if snapshot.Truncated {
		return []byte("[body omitted: capture limit exceeded]"), true
	}
	body := snapshot.body
	encoding := strings.ToLower(strings.TrimSpace(snapshot.Headers["content-encoding"]))
	var reader io.ReadCloser
	var err error
	switch encoding {
	case "", "identity":
	case "gzip":
		reader, err = gzip.NewReader(bytes.NewReader(body))
	case "deflate":
		reader, err = zlib.NewReader(bytes.NewReader(body))
	default:
		return []byte("[body omitted: unsupported content encoding]"), true
	}
	if err != nil {
		return []byte("[body omitted: invalid compressed body]"), true
	}
	if reader != nil {
		defer reader.Close()
		body, err = io.ReadAll(io.LimitReader(reader, maxDiagnosticRawBodyBytes+1))
		if err != nil || len(body) > maxDiagnosticRawBodyBytes {
			return []byte("[body omitted: decompression limit exceeded]"), true
		}
	}
	if len(body) == 0 {
		return body, false
	}
	// No opaque multipart/image/binary/authentication body is logged. Only a
	// complete JSON object/array can be recursively redacted safely.
	trimmed := bytes.TrimSpace(body)
	if len(trimmed) == 0 || (trimmed[0] != '{' && trimmed[0] != '[') || !json.Valid(trimmed) {
		return []byte("[body omitted: not complete JSON]"), true
	}
	var decoded any
	decoder := json.NewDecoder(bytes.NewReader(trimmed))
	decoder.UseNumber()
	if decoder.Decode(&decoded) != nil {
		return []byte("[body omitted: invalid JSON]"), true
	}
	redacted, err := json.Marshal(redactDiagnosticJSON(decoded, 0))
	if err != nil {
		return []byte("[body omitted: redaction failed]"), true
	}
	return redacted, false
}

func prepareDiagnosticSnapshots(pending *pendingRequestDiagnostics) {
	if pending.payload.Attempts == nil {
		pending.payload.Attempts = []requestAttemptDetail{}
	}
	if pending.payload.Payloads == nil {
		pending.payload.Payloads = []requestPayloadSnapshot{}
	}
	remaining := maxDiagnosticTotalBodyBytes
	for _, snapshot := range pending.raw {
		body, omitted := decodeDiagnosticBody(snapshot)
		limit := maxDiagnosticSnapshotBytes
		if limit > remaining {
			limit = remaining
		}
		if limit <= 0 {
			pending.payload.Truncated = true
			break
		}
		snapshot.Truncated = snapshot.Truncated || omitted || len(body) > limit
		snapshot.Body = truncateDiagnosticUTF8(string(body), limit)
		remaining -= len(snapshot.Body)
		// Hash the readable, redacted snapshot, never the original credentials.
		hash := sha256.Sum256([]byte(snapshot.Body))
		snapshot.SHA256 = hex.EncodeToString(hash[:])
		pending.payload.Payloads = append(pending.payload.Payloads, snapshot.requestPayloadSnapshot)
		pending.payload.Truncated = pending.payload.Truncated || snapshot.Truncated
	}
	pending.raw = nil
}

// The HTTP IPC batches have a strict encoded-size bound too: JSON escaping,
// attempt errors and headers count toward it, not only readable body bytes.
func limitDiagnosticEvent(payload *requestDiagnosticsPayload) {
	for {
		encoded, err := json.Marshal(payload)
		if err != nil || len(encoded) <= maxDiagnosticEventBytes {
			return
		}
		payload.Truncated = true
		excess := len(encoded) - maxDiagnosticEventBytes + 256
		changed := false
		for i := len(payload.Payloads) - 1; i >= 0; i-- {
			snapshot := &payload.Payloads[i]
			if len(snapshot.Body) == 0 {
				continue
			}
			limit := len(snapshot.Body) - excess
			if limit < 0 {
				limit = 0
			}
			snapshot.Body = truncateDiagnosticUTF8(snapshot.Body, limit)
			snapshot.Truncated = true
			hash := sha256.Sum256([]byte(snapshot.Body))
			snapshot.SHA256 = hex.EncodeToString(hash[:])
			changed = true
			break
		}
		if changed {
			continue
		}
		for i := len(payload.Attempts) - 1; i >= 0; i-- {
			attempt := &payload.Attempts[i]
			if len(attempt.ErrorMessage) > 0 {
				limit := len(attempt.ErrorMessage) - excess
				if limit < 0 {
					limit = 0
				}
				attempt.ErrorMessage = truncateDiagnosticUTF8(attempt.ErrorMessage, limit)
				changed = true
				break
			}
		}
		if changed {
			continue
		}
		// Extreme user-defined identifiers/header values must not evade the IPC
		// limit; omit tail records after preserving the earlier attempt chain.
		if len(payload.Payloads) > 0 {
			payload.Payloads = payload.Payloads[:len(payload.Payloads)-1]
		} else if len(payload.Attempts) > 0 {
			payload.Attempts = payload.Attempts[:len(payload.Attempts)-1]
		} else {
			payload.RequestID = truncateDiagnosticUTF8(payload.RequestID, 512)
		}
	}
}

func truncateDiagnosticUTF8(value string, limit int) string {
	value = strings.ToValidUTF8(value, "�")
	if len(value) <= limit {
		return value
	}
	value = value[:limit]
	for len(value) > 0 && !utf8.ValidString(value) {
		value = value[:len(value)-1]
	}
	return value
}

var diagnosticCredentialPatterns = []*regexp.Regexp{
	regexp.MustCompile(`(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]+`),
	regexp.MustCompile(`(?i)([?&](?:api[_-]?key|access[_-]?token|refresh[_-]?token|token|key|secret|password)=)[^&\s"']+`),
	regexp.MustCompile(`\bsk-[A-Za-z0-9_-]{8,}\b`),
}

func safeDiagnosticError(value string) string {
	if len(value) > maxDiagnosticRawBodyBytes {
		return "[error omitted: size limit exceeded]"
	}
	trimmed := strings.TrimSpace(value)
	if json.Valid([]byte(trimmed)) {
		var decoded any
		decoder := json.NewDecoder(strings.NewReader(trimmed))
		decoder.UseNumber()
		if decoder.Decode(&decoded) == nil {
			if data, err := json.Marshal(redactDiagnosticJSON(decoded, 0)); err == nil {
				trimmed = string(data)
			}
		}
	}
	// Error diagnostics carry messages only. Payloads are never attached here.
	trimmed = diagnosticCredentialPatterns[0].ReplaceAllString(trimmed, "$1 [REDACTED]")
	trimmed = diagnosticCredentialPatterns[1].ReplaceAllString(trimmed, "$1[REDACTED]")
	trimmed = diagnosticCredentialPatterns[2].ReplaceAllString(trimmed, "[REDACTED]")
	return truncateDiagnosticUTF8(trimmed, 2048)
}
