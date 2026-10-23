package helps

import (
	"regexp"
	"strings"
)

var (
	streamTransportSecretPattern = regexp.MustCompile(`(?i)((?:["']?(?:api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|token|authorization|client[_-]?secret|secret|password|passwd)["']?)\s*[=:]\s*)(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\s"'&,;}]+)`)
	streamTransportBearerPattern = regexp.MustCompile(`(?i)\b(?:Bearer|Basic)\s+[^\s"'&,;}]+`)
	streamTransportURLPattern    = regexp.MustCompile(`(?i)(?:https?|wss?)://[^\s"'<>]+`)
)

// RedactStreamTransportError retains the transport cause without exposing URL or
// credential values when the error is serialized outside the HTTP SSE handler.
func RedactStreamTransportError(message string) string {
	message = streamTransportURLPattern.ReplaceAllString(strings.TrimSpace(message), "[REDACTED URL]")
	message = streamTransportBearerPattern.ReplaceAllString(message, "[REDACTED]")
	message = streamTransportSecretPattern.ReplaceAllString(message, `${1}[REDACTED]`)
	message = strings.Map(func(r rune) rune {
		if r < ' ' || r == '\u007f' {
			return ' '
		}
		return r
	}, message)
	const limit = 2048
	if runes := []rune(message); len(runes) > limit {
		return string(runes[:limit]) + "…"
	}
	return message
}
