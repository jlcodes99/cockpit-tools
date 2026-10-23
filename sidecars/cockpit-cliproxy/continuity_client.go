package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/url"
	"time"

	coreexecutor "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/executor"
)

type continuityServiceSpec struct {
	Version    int    `json:"version"`
	Executable string `json:"executable"`
	ConfigPath string `json:"configPath"`
	Token      string `json:"token"`
}

type continuityClient struct {
	endpoint, token string
	client          *http.Client
	manifest        *manifest
	closeWorker     func()
}

type continuityClientHeadersKey struct{}
type continuityWebsocketKey struct{}
type continuityConversationKey struct{}

type continuityControlError struct {
	status        int
	code, message string
}

func (e continuityControlError) Error() string {
	body, _ := json.Marshal(map[string]any{"error": map[string]any{"type": "server_error", "code": e.code, "message": e.message}})
	return string(body)
}
func (e continuityControlError) StatusCode() int       { return e.status }
func (e continuityControlError) IsRequestScoped() bool { return true }

func newContinuityClient(endpoint, token string, m *manifest) (*continuityClient, error) {
	parsed, err := url.Parse(endpoint)
	if err != nil || parsed.Scheme != "http" || parsed.Hostname() != "127.0.0.1" || parsed.Port() == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" || parsed.Path != "" || len(token) != 64 {
		return nil, errors.New("invalid local continuity endpoint")
	}
	if _, err := hex.DecodeString(token); err != nil {
		return nil, errors.New("invalid local continuity token")
	}
	return &continuityClient{endpoint: endpoint, token: token, manifest: m, client: &http.Client{Transport: &http.Transport{Proxy: nil, MaxIdleConnsPerHost: 16}, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }, Timeout: 10 * time.Second}}, nil
}

func (c *continuityClient) call(ctx context.Context, method string, input, output any) error {
	var buffer bytes.Buffer
	encoder := json.NewEncoder(&buffer)
	encoder.SetEscapeHTML(false)
	err := encoder.Encode(input)
	if err != nil {
		return continuityControlError{status: 400, code: "continuity_invalid_request", message: "Continuity request could not be encoded"}
	}
	encoded := buffer.Bytes()
	// Control operations use stable request/attempt/sequence IDs. Retry only a
	// lost local transport response; never create a second upstream attempt.
	for attempt := 0; attempt < 2; attempt++ {
		request, errRequest := http.NewRequestWithContext(ctx, http.MethodPost, c.endpoint+"/v1/continuity/"+method, bytes.NewReader(encoded))
		if errRequest != nil {
			return errRequest
		}
		request.Header.Set("authorization", "Bearer "+c.token)
		request.Header.Set("content-type", "application/json")
		response, errSend := c.client.Do(request)
		if errSend != nil {
			if attempt == 0 && ctx.Err() == nil {
				continue
			}
			return continuityControlError{status: 503, code: "continuity_unavailable", message: "The local continuity worker is unavailable"}
		}
		body, errRead := io.ReadAll(response.Body)
		_ = response.Body.Close()
		if errRead != nil && attempt == 0 && ctx.Err() == nil {
			continue
		}
		if errRead != nil {
			return continuityControlError{status: 503, code: "continuity_invalid_response", message: "The local continuity response is incomplete"}
		}
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			var envelope struct {
				Error struct {
					Code    string `json:"code"`
					Message string `json:"message"`
				} `json:"error"`
			}
			if json.Unmarshal(body, &envelope) != nil || envelope.Error.Code == "" {
				return continuityControlError{status: 503, code: "continuity_invalid_response", message: "The local continuity worker returned an invalid error"}
			}
			return continuityControlError{status: response.StatusCode, code: envelope.Error.Code, message: envelope.Error.Message}
		}
		if output != nil && json.Unmarshal(body, output) != nil {
			return continuityControlError{status: 503, code: "continuity_invalid_response", message: "The local continuity worker returned an invalid response"}
		}
		return nil
	}
	return errors.New("continuity control failed")
}

func canonicalContinuityCaller(spec *apiKeySpec) string {
	if spec == nil {
		return ""
	}
	switch spec.ID {
	case "legacy":
		return "native:primary"
	case "__cockpit_internal__":
		return "native:internal"
	default:
		return spec.ID
	}
}

func (c *continuityClient) Prepare(ctx context.Context, request coreexecutor.ContinuityPrepareRequest) (coreexecutor.ContinuityPrepareResponse, error) {
	var result coreexecutor.ContinuityPrepareResponse
	// Standalone Images/audio calls must not claim a text conversation. A real
	// Responses image tool retains its conversation when pool selection changes
	// the request kind for image admission and account selection.
	if kind, ok := ctx.Value(requestKindContextKey).(string); ok && kind != "text" {
		conversation, _ := ctx.Value(continuityConversationKey{}).(bool)
		if kind != "image_generation" || !conversation {
			return result, nil
		}
	}
	spec, _ := ctx.Value(clientAPIKeyContextKey).(*apiKeySpec)
	if spec == nil || !spec.Enabled {
		return result, continuityControlError{status: 401, code: "invalid_api_key", message: "The API key is no longer available"}
	}
	request.Caller = canonicalContinuityCaller(spec)
	if original, ok := ctx.Value(continuityClientHeadersKey{}).(http.Header); ok {
		request.Headers = request.Headers.Clone()
		if request.Headers == nil {
			request.Headers = make(http.Header)
		}
		websocket, _ := ctx.Value(continuityWebsocketKey{}).(bool)
		for key, values := range original {
			if websocket && len(request.Headers.Values(key)) > 0 {
				continue
			}
			request.Headers[key] = append([]string(nil), values...)
		}
	}
	if err := c.call(ctx, "prepare", request, &result); err != nil {
		return result, err
	}
	if result.PreferredAccountID != "" {
		result.RequireCodex = true
		mapped := ""
		if c.manifest.authManager != nil {
			for _, auth := range c.manifest.authManager.List() {
				if account := accountForAuthInManifest(c.manifest, auth); account != nil && account.ID == result.PreferredAccountID {
					mapped = auth.ID
					break
				}
			}
		}
		if mapped == "" && !result.SelfContained {
			_ = c.Finish(context.WithoutCancel(ctx), coreexecutor.ContinuityFinishRequest{Ticket: result.Ticket, Sequence: 1, Status: "failed", ErrorCode: "continuity_account_unavailable"})
			_ = c.Release(context.WithoutCancel(ctx), coreexecutor.ContinuityReleaseRequest{Ticket: result.Ticket})
			return result, continuityControlError{status: 409, code: "account_binding_unavailable", message: "The original account is not available for this protected continuation"}
		}
		result.PreferredAccountID = mapped
	}
	result.Headers = canonicalContinuityHeaders(result.Headers)
	return result, nil
}

func (c *continuityClient) Bind(ctx context.Context, request coreexecutor.ContinuityBindRequest) (coreexecutor.ContinuityBindResponse, error) {
	var result coreexecutor.ContinuityBindResponse
	if c.manifest.authManager == nil {
		return result, continuityControlError{status: 503, code: "continuity_account_unavailable", message: "The selected account is unavailable"}
	}
	auth, found := c.manifest.authManager.GetByID(request.AccountID)
	if !found {
		return result, continuityControlError{status: 503, code: "continuity_account_unavailable", message: "The selected account is unavailable"}
	}
	account := accountForAuthInManifest(c.manifest, auth)
	if account == nil || account.ContinuityOwnerID == "" {
		return result, continuityControlError{status: 503, code: "continuity_account_unavailable", message: "The selected account has no valid continuity owner"}
	}
	request.AccountID, request.OwnerID = account.ID, account.ContinuityOwnerID
	err := c.call(ctx, "bind", request, &result)
	result.Headers = canonicalContinuityHeaders(result.Headers)
	return result, err
}

// Rust/HTTP normalize names to lowercase; Go Header.Get requires canonical map
// keys. Rebuild the map before executors consume Session/Thread/turn headers.
func canonicalContinuityHeaders(headers http.Header) http.Header {
	result := make(http.Header, len(headers))
	for name, values := range headers {
		for _, value := range values {
			result.Add(name, value)
		}
	}
	return result
}
func (c *continuityClient) Observe(ctx context.Context, request coreexecutor.ContinuityObserveRequest) (coreexecutor.ContinuityObserveResponse, error) {
	var result coreexecutor.ContinuityObserveResponse
	err := c.call(ctx, "observe", request, &result)
	return result, err
}
func (c *continuityClient) Finish(ctx context.Context, request coreexecutor.ContinuityFinishRequest) error {
	return c.call(ctx, "finish", request, nil)
}

func (c *continuityClient) Release(ctx context.Context, request coreexecutor.ContinuityReleaseRequest) error {
	return c.call(ctx, "release", request, nil)
}

func withRequestContinuity(ctx context.Context, m *manifest, headers http.Header, websocket bool) context.Context {
	if m == nil || m.continuityClient == nil {
		return ctx
	}
	ctx = coreexecutor.WithContinuityHook(ctx, m.continuityClient)
	kind, _ := ctx.Value(requestKindContextKey).(string)
	ctx = context.WithValue(ctx, continuityConversationKey{}, kind == "text")
	ctx = context.WithValue(ctx, continuityClientHeadersKey{}, headers.Clone())
	ctx = context.WithValue(ctx, continuityWebsocketKey{}, websocket)
	return ctx
}

func (c *continuityClient) Close() {
	if c != nil {
		if c.closeWorker != nil {
			c.closeWorker()
		}
		c.client.CloseIdleConnections()
	}
}

var _ coreexecutor.ContinuityHook = (*continuityClient)(nil)
var _ coreexecutor.ContinuityFinalizer = (*continuityClient)(nil)
