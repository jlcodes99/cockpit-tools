package helps

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"

	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
	cliproxyexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
	"github.com/tidwall/gjson"
)

type httpPreviousHook struct {
	bound         cliproxyexecutor.ContinuityBindRequest
	selfContained bool
}

func (h *httpPreviousHook) Prepare(_ context.Context, r cliproxyexecutor.ContinuityPrepareRequest) (cliproxyexecutor.ContinuityPrepareResponse, error) {
	return cliproxyexecutor.ContinuityPrepareResponse{Ticket: "ticket", Body: r.Body, Headers: r.Headers, SelfContained: h.selfContained}, nil
}
func (h *httpPreviousHook) Bind(_ context.Context, r cliproxyexecutor.ContinuityBindRequest) (cliproxyexecutor.ContinuityBindResponse, error) {
	h.bound = r
	return cliproxyexecutor.ContinuityBindResponse{Ticket: r.Ticket, AttemptID: r.AttemptID, Body: r.Body, Headers: r.Headers}, nil
}
func (*httpPreviousHook) Observe(context.Context, cliproxyexecutor.ContinuityObserveRequest) (cliproxyexecutor.ContinuityObserveResponse, error) {
	return cliproxyexecutor.ContinuityObserveResponse{}, nil
}
func (*httpPreviousHook) Finish(context.Context, cliproxyexecutor.ContinuityFinishRequest) error {
	return nil
}

func TestHTTPPreviousExpansionRequiresVerifiedHistoryOrOAuthHTTP(t *testing.T) {
	for _, tc := range []struct {
		name, path                            string
		host                                  string
		selfContained                         bool
		apiKey, websocket, tokenMissing, want bool
	}{
		{name: "oauth HTTP", path: "/backend-api/codex/responses", want: true},
		{name: "API key", path: "/responses", apiKey: true},
		{name: "API key with legacy OAuth metadata", path: "/responses", apiKey: true},
		{name: "compatible API key with verified history", path: "/responses", apiKey: true, selfContained: true, want: true},
		{name: "official OpenAI preserves previous ID", host: "api.openai.com", path: "/v1/responses", apiKey: true, selfContained: true},
		{name: "compatible websocket preserves previous ID", path: "/responses", apiKey: true, selfContained: true, websocket: true},
		{name: "compatible compact preserves previous ID", path: "/responses/compact", apiKey: true, selfContained: true},
		{name: "OAuth websocket", path: "/responses", websocket: true},
		{name: "compact", path: "/responses/compact"},
		{name: "missing OAuth token", path: "/responses", tokenMissing: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			auth := &cliproxyauth.Auth{ID: "account", Provider: "codex", Metadata: map[string]any{"access_token": "synthetic-token"}}
			if tc.apiKey {
				auth.Attributes = map[string]string{"api_key": "synthetic-key"}
			}
			if tc.tokenMissing {
				delete(auth.Metadata, "access_token")
			}
			hook := &httpPreviousHook{selfContained: tc.selfContained}
			canonical := []byte(`{"model":"test","previous_response_id":"previous","input":[{"role":"user","content":"delta"}]}`)
			ctx, _, err := cliproxyexecutor.PrepareContinuity(cliproxyexecutor.WithContinuityHook(context.Background(), hook), cliproxyexecutor.ContinuityPrepareRequest{Body: canonical})
			if err != nil {
				t.Fatal(err)
			}
			if tc.websocket {
				_, _, _, err = BindCodexContinuity(ctx, auth, http.Header{}, canonical)
			} else {
				host := tc.host
				if host == "" {
					host = "synthetic.invalid"
				}
				req, requestErr := http.NewRequestWithContext(ctx, http.MethodPost, "https://"+host+tc.path, nil)
				if requestErr != nil {
					t.Fatal(requestErr)
				}
				_, _, err = BindContinuityHTTPRequest(ctx, auth, req, canonical)
			}
			if err != nil {
				t.Fatal(err)
			}
			if hook.bound.ExpandHTTPPrevious != tc.want {
				t.Fatalf("expand_http_previous=%v, want %v", hook.bound.ExpandHTTPPrevious, tc.want)
			}
			wire, err := json.Marshal(hook.bound)
			if err != nil {
				t.Fatal(err)
			}
			if gjson.GetBytes(wire, "expand_http_previous").Bool() != tc.want {
				t.Fatal("capability was not encoded for the Rust bridge")
			}
			if gjson.GetBytes(hook.bound.Body, "previous_response_id").String() != "previous" || gjson.GetBytes(hook.bound.Body, "input").Raw != gjson.GetBytes(canonical, "input").Raw {
				t.Fatal("capability transfer modified canonical continuation history")
			}
		})
	}
}
