package executor

import (
	"context"
	"fmt"
	"net/http"
	"strings"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/util"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

var (
	xaiDataTag  = []byte("data:")
	xaiEventTag = []byte("event:")
	// Codex 的 apply_patch 是 freeform(custom) 工具，声明里带 lark grammar 而不是
	// JSON parameters，xAI 的 Responses 端点只接受 function 形态。请求侧降级为
	// 单字段 JSON function，并把官方信封写进每轮 tool description；响应侧再还原
	// 成客户端期望的 custom_tool_call，同时把独立成行的错误头尾改回官方信封。
	xaiApplyPatchDescription = strings.Join([]string{
		"Use apply_patch to edit local files. The input is freeform patch text, not JSON.",
		"",
		"The first line MUST be exactly:",
		xaiApplyPatchBeginMarker,
		"The last line MUST be exactly:",
		xaiApplyPatchEndMarker,
		"",
		"Do not wrap the markers with extra asterisks. These are invalid and will fail:",
		xaiApplyPatchBeginWrong,
		xaiApplyPatchEndWrong,
		"",
		"Example:",
		xaiApplyPatchBeginMarker,
		"*** Add File: hello.txt",
		"+hello",
		xaiApplyPatchEndMarker,
	}, "\n")
	xaiApplyPatchParameters          = `{"type":"object","properties":{"input":{"type":"string","description":"The complete apply_patch patch text. First line must be exactly *** Begin Patch. Last line must be exactly *** End Patch. Do not write *** Begin Patch *** or *** End Patch ***."}},"required":["input"],"additionalProperties":false}`
	xaiApplyPatchInstructionReminder = strings.Join([]string{
		"When calling apply_patch, the first line MUST be exactly:",
		xaiApplyPatchBeginMarker,
		"The last line MUST be exactly:",
		xaiApplyPatchEndMarker,
		"Do not write " + xaiApplyPatchBeginWrong + " or " + xaiApplyPatchEndWrong + ".",
	}, "\n")
)

const (
	xaiImageHandlerType        = "openai-image"
	xaiVideoHandlerType        = "openai-video"
	xaiCustomToolType          = "custom"
	xaiFunctionToolType        = "function"
	xaiApplyPatchToolName      = "apply_patch"
	xaiImageGenerationToolType = "image_generation"
	xaiNamespaceToolType       = "namespace"
	xaiToolSearchType          = "tool_search"
	xaiWebSearchToolType       = "web_search"
	xaiXSearchToolType         = "x_search"
	xaiMaxTools                = 200
	// Codex Desktop injects codex_app.automation_update with a large oneOf+$ref
	// schema. xAI's free/build Responses path accepts the HTTP request but never
	// emits SSE when that schema is present, so Desktop hangs on "thinking".
	xaiCodexAppNamespaceName    = "codex_app"
	xaiAutomationUpdateToolName = "automation_update"
	// Permissive placeholder schema: keeps the tool callable without the hang.
	xaiSafeFunctionParameters   = `{"type":"object","properties":{},"additionalProperties":true}`
	xaiApplyPatchBeginMarker    = "*** Begin Patch"
	xaiApplyPatchEndMarker      = "*** End Patch"
	xaiApplyPatchBeginWrong     = "*** Begin Patch ***"
	xaiApplyPatchEndWrong       = "*** End Patch ***"
	xaiImagesGenerationsPath    = "/images/generations"
	xaiImagesEditsPath          = "/images/edits"
	xaiDefaultImageEndpointPath = xaiImagesGenerationsPath
	xaiVideosGenerationsPath    = "/videos/generations"
	xaiVideosEditsPath          = "/videos/edits"
	xaiVideosExtensionsPath     = "/videos/extensions"
	xaiVideosPath               = "/videos"
	xaiIdempotencyKeyMetaKey    = "idempotency_key"
	xaiComposerModelPrefix      = "grok-composer-"
	xaiTokenAuthHeader          = "X-XAI-Token-Auth"
	xaiTokenAuthValue           = "xai-grok-cli"
	xaiClientVersionHeader      = "x-grok-client-version"
	// Keep in sync with the current Grok CLI client version that chat-proxy expects.
	xaiClientVersionValue         = "0.2.120"
	xaiClientIdentifierHeader     = "x-grok-client-identifier"
	xaiClientIdentifierValue      = "grok-shell"
	xaiAuthenticateResponseHeader = "x-authenticateresponse"
	xaiAuthenticateResponseValue  = "authenticate-response"
	// xaiUsingAPIAttr enables the official API path for HTTP chat and media.
	xaiUsingAPIAttr = "using_api"
)

// xaiXSearchToolJSON is the native X Search tool injected when enabled by config.
// Internal subtool traces are still filtered downstream when this tool is present.
var xaiXSearchToolJSON = []byte(`{"type":"x_search"}`)

// XAIExecutor is a stateless executor for xAI Grok's Responses API.
type XAIExecutor struct {
	cfg *config.Config
}

// NewXAIExecutor creates a new xAI executor.
func NewXAIExecutor(cfg *config.Config) *XAIExecutor {
	return &XAIExecutor{cfg: cfg}
}

// Identifier returns the provider identifier.
func (e *XAIExecutor) Identifier() string {
	return "xai"
}

// PrepareRequest injects xAI credentials into the outgoing HTTP request.
func (e *XAIExecutor) PrepareRequest(req *http.Request, auth *cliproxyauth.Auth) error {
	if req == nil {
		return nil
	}
	token, _ := xaiCreds(auth)
	if strings.TrimSpace(token) != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	} else {
		req.Header.Del("Authorization")
	}
	var attrs map[string]string
	if auth != nil {
		attrs = auth.Attributes
	}
	util.ApplyCustomHeadersFromAttrs(req, attrs)
	return nil
}

// HttpRequest injects xAI credentials into the request and executes it.
func (e *XAIExecutor) HttpRequest(ctx context.Context, auth *cliproxyauth.Auth, req *http.Request) (*http.Response, error) {
	if req == nil {
		return nil, fmt.Errorf("xai executor: request is nil")
	}
	if ctx == nil {
		ctx = req.Context()
	}
	httpReq := req.WithContext(ctx)
	if errPrepare := e.PrepareRequest(httpReq, auth); errPrepare != nil {
		return nil, errPrepare
	}
	httpClient := helps.NewProxyAwareHTTPClient(ctx, e.cfg, auth, 0)
	return httpClient.Do(httpReq)
}
