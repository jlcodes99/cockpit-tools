package executor

import (
	"net/http"
	"strings"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

func metadataBool(metadata map[string]any, key string) bool {
	if metadata == nil {
		return false
	}
	switch value := metadata[key].(type) {
	case bool:
		return value
	case string:
		return strings.EqualFold(strings.TrimSpace(value), "true")
	default:
		return false
	}
}

func isOfficialCodexClient(userAgent, originator string) bool {
	ua := strings.ToLower(strings.TrimSpace(userAgent))
	for _, prefix := range []string{
		"codex_cli_rs/", "codex-tui/", "codex_vscode/", "codex_vscode_copilot/",
		"codex_app/", "codex_chatgpt_desktop/", "codex_atlas/", "codex_exec/",
		"codex_sdk_ts/", "codex ",
	} {
		if strings.HasPrefix(ua, prefix) {
			return true
		}
	}
	originator = strings.ToLower(strings.TrimSpace(originator))
	if originator == "codex_cli_rs" || originator == "codex-tui" ||
		originator == "codex_vscode" || originator == "codex_vscode_copilot" ||
		originator == "codex_app" || originator == "codex_chatgpt_desktop" ||
		originator == "codex_atlas" || originator == "codex_exec" || originator == "codex_sdk_ts" {
		return true
	}
	return strings.HasPrefix(originator, "codex ")
}

func enforceCodexClientPolicy(auth *cliproxyauth.Auth, headers http.Header, payload []byte) error {
	if auth == nil || !metadataBool(auth.Metadata, "codex_cli_only") {
		return nil
	}
	userAgent, originator := "", ""
	if headers != nil {
		userAgent, originator = headers.Get("User-Agent"), headers.Get("Originator")
	}
	if isOfficialCodexClient(userAgent, originator) ||
		metadataBool(auth.Metadata, "codex_cli_only_allow_app_server") ||
		metadataBool(auth.Metadata, "codex_cli_only_allow_app_server_clients") {
		return nil
	}
	_ = payload
	return statusErr{code: http.StatusForbidden, msg: "This account only allows Codex official clients"}
}

func _codexPolicyCompileAnchor(_ cliproxyexecutor.Options) {}
