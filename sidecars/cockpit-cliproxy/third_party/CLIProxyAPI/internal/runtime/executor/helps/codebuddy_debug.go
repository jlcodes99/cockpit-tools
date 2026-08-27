package helps

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sync"
)

// codebuddyDebugBodyEnv is the environment variable that enables request/response
// body dumps for the CodeBuddy executor. Set to "1" to enable. The dump is
// written to a log file (see codebuddyDebugBodyDirEnv); if the file cannot be
// opened it falls back to stdout as a JSON event. It is disabled by default and
// has no effect on normal requests.
const codebuddyDebugBodyEnv = "CODEBUDDY_DEBUG_BODY"

// codebuddyDebugBodyDirEnv optionally redirects the debug body dump to a file
// under this directory. When unset, dumps are written to docs/log relative to
// the sidecar working directory (the repo root in dev mode).
const codebuddyDebugBodyDirEnv = "CODEBUDDY_DEBUG_BODY_DIR"

// codebuddyDebugBodyMaxLen caps the redacted dump length so a single request with
// large payloads (e.g. images) cannot flood the log stream. The value is large
// enough to capture the full tools array and complete messages (Cursor sends 40+
// tool definitions) so the exact invalid parameter behind 11133 can be inspected;
// inline image base64 is already truncated separately by codebuddyDebugImageDataRe.
const codebuddyDebugBodyMaxLen = 300000

var (
	// codebuddyDebugAccessTokenRe masks access_token / refresh_token values.
	codebuddyDebugAccessTokenRe = regexp.MustCompile(`(?i)("(?:access_token|refresh_token)"\s*:\s*")[^"]*(")`)
	// codebuddyDebugImageDataRe truncates inline base64 image data to a short
	// prefix so image blobs do not blow up the log. Only `data:image/...` URLs
	// are affected; remote image URLs are left intact.
	codebuddyDebugImageDataRe = regexp.MustCompile(`(data:image/[a-zA-Z0-9.+-]+;base64,)([A-Za-z0-9+/=]{1,32})[A-Za-z0-9+/=]*`)

	codebuddyDebugFileOnce    sync.Once
	codebuddyDebugFilePath    string
	codebuddyDebugFilePathErr error
	codebuddyDebugFileMu      sync.Mutex
)

// CodebuddyDebugBodyEnabled reports whether CODEBUDDY_DEBUG_BODY is set to "1".
func CodebuddyDebugBodyEnabled() bool {
	return os.Getenv(codebuddyDebugBodyEnv) == "1"
}

// codebuddyDebugBodyLogFile resolves the debug body log file path (once),
// creating the target directory if needed. A non-nil error means the file
// cannot be resolved and callers should fall back to stdout.
func codebuddyDebugBodyLogFile() (string, error) {
	codebuddyDebugFileOnce.Do(func() {
		dir := os.Getenv(codebuddyDebugBodyDirEnv)
		if dir == "" {
			dir = filepath.Join("docs", "log")
		}
		if err := os.MkdirAll(dir, 0o755); err != nil {
			codebuddyDebugFilePathErr = err
			return
		}
		codebuddyDebugFilePath = filepath.Join(dir, "codebuddy_debug.log")
	})
	return codebuddyDebugFilePath, codebuddyDebugFilePathErr
}

// redactCodebuddyDebugBody returns a redacted string representation of the given
// JSON body, masking credentials and truncating inline base64 image data.
func redactCodebuddyDebugBody(body []byte) string {
	if len(body) == 0 {
		return ""
	}
	s := string(body)
	s = codebuddyDebugAccessTokenRe.ReplaceAllString(s, `${1}***REDACTED***${2}`)
	s = codebuddyDebugImageDataRe.ReplaceAllString(s, `${1}${2}...[image-data-truncated]`)
	return s
}

// DumpCodebuddyDebugBody emits a redacted body dump when CODEBUDDY_DEBUG_BODY=1.
// It appends to docs/log/codebuddy_debug.log (or CODEBUDDY_DEBUG_BODY_DIR when
// set) so the stream does not flood stdout; the file can then be tailed/grepped
// at leisure. If the file cannot be opened it falls back to stdout as a JSON
// event.
func DumpCodebuddyDebugBody(phase string, body []byte) {
	if !CodebuddyDebugBodyEnabled() {
		return
	}
	redacted := redactCodebuddyDebugBody(body)
	if len(redacted) > codebuddyDebugBodyMaxLen {
		redacted = redacted[:codebuddyDebugBodyMaxLen] + "...[truncated]"
	}
	event, err := json.Marshal(map[string]string{
		"type":  "codebuddy_debug_body",
		"phase": phase,
		"body":  redacted,
	})
	if err != nil {
		return
	}
	line := append(event, '\n')

	if path, err := codebuddyDebugBodyLogFile(); err == nil && path != "" {
		codebuddyDebugFileMu.Lock()
		f, openErr := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
		if openErr == nil {
			_, _ = f.Write(line)
			_ = f.Close()
		}
		codebuddyDebugFileMu.Unlock()
		if openErr == nil {
			return
		}
	}
	fmt.Println(string(event))
}
