package responses

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/config"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/providers"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/session"
	"github.com/jlcodes99/cockpit-tools/codex-proxy/internal/types"
	"github.com/gin-gonic/gin"
)

func TestHandleSuccess_PreservesPreviousResponseID(t *testing.T) {
	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)

	sessionManager := session.NewSessionManager(time.Hour, 100, 100000)
	provider := &providers.ResponsesProvider{SessionManager: sessionManager}
	envCfg := &config.EnvConfig{}

	sess, err := sessionManager.GetOrCreateSession("")
	if err != nil {
		t.Fatalf("GetOrCreateSession() err = %v", err)
	}
	if err := sessionManager.UpdateLastResponseID(sess.ID, "resp_prev"); err != nil {
		t.Fatalf("UpdateLastResponseID() err = %v", err)
	}
	sessionManager.RecordResponseMapping("resp_prev", sess.ID)

	resp := &http.Response{
		StatusCode: http.StatusOK,
		Header:     http.Header{"Content-Type": {"application/json"}},
		Body: io.NopCloser(strings.NewReader(`{
			"id":"resp_new",
			"model":"gpt-5",
			"status":"completed",
			"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}],
			"usage":{"input_tokens":12,"output_tokens":8,"total_tokens":20}
		}`)),
	}

	originalReq := &types.ResponsesRequest{
		PreviousResponseID: "resp_prev",
		Input:              "hello",
	}

	if _, err := handleSuccess(c, resp, provider, "responses", envCfg, sessionManager, time.Now(), originalReq, []byte(`{"model":"gpt-5","input":"hello"}`), false); err != nil {
		t.Fatalf("handleSuccess() err = %v", err)
	}

	body := w.Body.String()
	if !strings.Contains(body, `"previous_id":"resp_prev"`) {
		t.Fatalf("response body should preserve previous response id, got %s", body)
	}
	if strings.Contains(body, `"previous_id":"resp_new"`) {
		t.Fatalf("response body should not rewrite previous_id to current response id, got %s", body)
	}
}
