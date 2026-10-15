package openai

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/registry"
	"github.com/router-for-me/CLIProxyAPI/v7/sdk/api/handlers"
	coreauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	sdkconfig "github.com/router-for-me/CLIProxyAPI/v7/sdk/config"
	"github.com/tidwall/gjson"
)

type liteBoundaryCaptureExecutor struct {
	compactCaptureExecutor
	payload []byte
}

func (e *liteBoundaryCaptureExecutor) Execute(_ context.Context, _ *coreauth.Auth, req coreexecutor.Request, _ coreexecutor.Options) (coreexecutor.Response, error) {
	e.payload = bytes.Clone(req.Payload)
	return coreexecutor.Response{Payload: []byte(`{"id":"resp-lite","output":[]}`)}, nil
}

// Provider-specific Lite policy belongs in the executor, not in a second
// handler rewrite based solely on a model name or a header.
func TestResponsesLiteBoundaryLeavesPolicyToExecutor(t *testing.T) {
	for _, model := range []string{"gpt-5.5", "gpt-5.6-luna"} {
		for _, header := range []string{"", "true", "false"} {
			t.Run(model+"/"+header, func(t *testing.T) {
				executor := &liteBoundaryCaptureExecutor{}
				manager := coreauth.NewManager(nil, nil, nil)
				manager.RegisterExecutor(executor)
				auth := &coreauth.Auth{ID: "lite-boundary", Provider: executor.Identifier(), Status: coreauth.StatusActive}
				if _, err := manager.Register(context.Background(), auth); err != nil {
					t.Fatal(err)
				}
				registry.GetGlobalRegistry().RegisterClient(auth.ID, auth.Provider, []*registry.ModelInfo{{ID: model}})
				defer registry.GetGlobalRegistry().UnregisterClient(auth.ID)
				h := NewOpenAIResponsesAPIHandler(handlers.NewBaseAPIHandlers(&sdkconfig.SDKConfig{}, manager))
				router := gin.New()
				router.POST("/v1/responses", h.Responses)
				body := `{"model":"` + model + `","parallel_tool_calls":true,"input":"hello"}`
				req := httptest.NewRequest(http.MethodPost, "/v1/responses", strings.NewReader(body))
				req.Header.Set("Content-Type", "application/json")
				if header != "" {
					req.Header.Set("X-OpenAI-Internal-Codex-Responses-Lite", header)
				}
				response := httptest.NewRecorder()
				router.ServeHTTP(response, req)
				if response.Code != http.StatusOK {
					t.Fatalf("request failed: %d %s", response.Code, response.Body.String())
				}
				if !gjson.GetBytes(executor.payload, "parallel_tool_calls").Bool() {
					t.Fatalf("handler rewrote executor policy: %s", executor.payload)
				}
			})
		}
	}
}
