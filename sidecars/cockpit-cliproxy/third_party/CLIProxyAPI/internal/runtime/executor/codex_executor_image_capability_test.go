package executor

import (
	"net/http"
	"strings"
	"testing"

	"github.com/router-for-me/CLIProxyAPI/v7/internal/config"
	"github.com/router-for-me/CLIProxyAPI/v7/internal/runtime/executor/helps"
	cliproxyauth "github.com/router-for-me/CLIProxyAPI/v7/sdk/cliproxy/auth"
)

func TestIsImageGenerationPermissionError(t *testing.T) {
	tests := []struct {
		name       string
		statusCode int
		body       string
		want       bool
	}{
		{
			name:       "sub2api nested permission error",
			statusCode: http.StatusForbidden,
			body:       `{"error":{"code":"auth_failed","message":"{\"error\":{\"message\":\"Image generation is not enabled for this group\",\"type\":\"permission_error\"}}","type":"invalid_request_error"}}`,
			want:       true,
		},
		{
			name:       "direct permission error",
			statusCode: http.StatusForbidden,
			body:       `{"error":{"message":"Image generation is not enabled for this group","type":"permission_error"}}`,
			want:       true,
		},
		{
			name:       "underscore variant",
			statusCode: http.StatusBadRequest,
			body:       `{"error":{"message":"image_generation is disabled for this plan"}}`,
			want:       true,
		},
		{
			name:       "unrelated permission error",
			statusCode: http.StatusForbidden,
			body:       `{"error":{"message":"Search is not enabled for this group","type":"permission_error"}}`,
			want:       false,
		},
		{
			name:       "quota 403",
			statusCode: http.StatusForbidden,
			body:       `{"error":{"message":"usage limit exceeded","type":"insufficient_quota"}}`,
			want:       false,
		},
		{
			name:       "auth 401",
			statusCode: http.StatusUnauthorized,
			body:       `{"error":{"message":"Image generation is not enabled for this group"}}`,
			want:       false,
		},
		{
			name:       "server error with same message",
			statusCode: http.StatusInternalServerError,
			body:       `{"error":{"message":"Image generation is not enabled for this group"}}`,
			want:       false,
		},
		{
			name:       "model not available 403",
			statusCode: http.StatusForbidden,
			body:       `{"error":{"code":"model_not_available","message":"The requested model is not available"}}`,
			want:       false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := isImageGenerationPermissionError(tt.statusCode, []byte(tt.body)); got != tt.want {
				t.Fatalf("isImageGenerationPermissionError(%d, %s) = %v, want %v", tt.statusCode, tt.body, got, tt.want)
			}
		})
	}
}

func TestNewCodexStatusErrClassifiesImagePermission(t *testing.T) {
	body := []byte(`{"error":{"code":"auth_failed","message":"{\"error\":{\"message\":\"Image generation is not enabled for this group\",\"type\":\"permission_error\"}}","type":"invalid_request_error"}}`)

	err := newCodexStatusErr(http.StatusForbidden, body)
	if got := err.StatusCode(); got != http.StatusForbidden {
		t.Fatalf("status code = %d, want 403", got)
	}
	if !strings.Contains(err.Error(), "image_generation_not_enabled") {
		t.Fatalf("classified message = %s, want it to contain image_generation_not_enabled", err.Error())
	}

	code, errType, ok := codexStatusErrorClassification(http.StatusForbidden, []byte(err.Error()))
	if !ok {
		t.Fatalf("expected classification, msg=%s", err.Error())
	}
	if code != "image_generation_not_enabled" || errType != "permission_error" {
		t.Fatalf("code=%s type=%s, want image_generation_not_enabled/permission_error", code, errType)
	}
}

func TestMaybeInjectImageGenerationToolSkipsMarkedAuthAndFlagsDegraded(t *testing.T) {
	baseBody := []byte(`{"model":"gpt-5.4","input":"draw an icon"}`)

	marked := &cliproxyauth.Auth{ID: "a", Provider: "codex", ImageGenerationUnavailable: true}
	body, degraded := maybeInjectImageGenerationTool(&config.Config{}, baseBody, "gpt-5.4", "/v1/responses", nil, marked)
	if degraded != true {
		t.Fatalf("degraded = %v, want true", degraded)
	}
	if string(body) != string(baseBody) {
		t.Fatalf("body changed for marked auth: %s", body)
	}

	fresh := &cliproxyauth.Auth{ID: "b", Provider: "codex"}
	body, degraded = maybeInjectImageGenerationTool(&config.Config{}, baseBody, "gpt-5.4", "/v1/responses", nil, fresh)
	if degraded != false {
		t.Fatalf("degraded = %v, want false", degraded)
	}
	if !strings.Contains(string(body), `"type":"image_generation"`) {
		t.Fatalf("image tool not injected for fresh auth: %s", body)
	}
}

func TestMaybeInjectImageGenerationToolRespectsManualHeader(t *testing.T) {
	baseBody := []byte(`{"model":"gpt-5.4","input":"hi"}`)
	headers := http.Header{helps.DisableImageGenerationHeader: []string{"chat"}}
	body, degraded := maybeInjectImageGenerationTool(&config.Config{}, baseBody, "gpt-5.4", "/v1/responses", headers, nil)
	if degraded != false || string(body) != string(baseBody) {
		t.Fatalf("manual header should disable injection without degraded flag, body=%s degraded=%v", body, degraded)
	}
}

func TestMaybeInjectImageGenerationToolStripsDeclaredToolsForMarkedAuth(t *testing.T) {
	baseBody := []byte(`{"model":"gpt-5.4","input":"make an image","tool_choice":{"type":"image_generation"},"tools":[{"type":"image_generation","output_format":"png"},{"type":"function","name":"lookup"}]}`)

	marked := &cliproxyauth.Auth{ID: "a", Provider: "codex", ImageGenerationUnavailable: true}
	body, degraded := maybeInjectImageGenerationTool(&config.Config{}, baseBody, "gpt-5.4", "/v1/responses", nil, marked)
	if degraded != true {
		t.Fatalf("degraded = %v, want true", degraded)
	}
	if strings.Contains(string(body), "image_generation") {
		t.Fatalf("declared image tool should be stripped: %s", body)
	}
	if !strings.Contains(string(body), "lookup") {
		t.Fatalf("non-image tool should survive: %s", body)
	}
}

func TestErrImageGenerationUnavailableBody(t *testing.T) {
	err := errImageGenerationUnavailable()
	se, ok := err.(statusErr)
	if !ok {
		t.Fatalf("expected statusErr, got %T", err)
	}
	if se.StatusCode() != http.StatusForbidden {
		t.Fatalf("status = %d, want 403", se.StatusCode())
	}
	if !strings.Contains(se.Error(), "image_generation_not_enabled") {
		t.Fatalf("body = %s, want image_generation_not_enabled", se.Error())
	}
}

func TestSetImageDegradedHeader(t *testing.T) {
	if got := setImageDegradedHeader(nil, false); got != nil {
		t.Fatalf("degraded=false should leave headers untouched, got %v", got)
	}
	if got := setImageDegradedHeader(http.Header{}, false); len(got) != 0 {
		t.Fatalf("degraded=false should not add headers, got %v", got)
	}
	got := setImageDegradedHeader(nil, true)
	if got.Get(codexImageDegradedHeader) != "1" {
		t.Fatalf("degraded=true should set %s=1, got %v", codexImageDegradedHeader, got)
	}
	existing := http.Header{"X-Existing": []string{"v"}}
	got = setImageDegradedHeader(existing, true)
	if got.Get(codexImageDegradedHeader) != "1" || got.Get("X-Existing") != "v" {
		t.Fatalf("existing headers should be preserved, got %v", got)
	}
}
